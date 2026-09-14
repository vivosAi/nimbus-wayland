//! What the fragment shader is told, and the clock that drives it.

use crate::focus::Rect;

/// Accumulated phase, not elapsed time.
///
/// **Never compute `time * speed`.** A flare changes the speed, and multiplying
/// an absolute timestamp by a changing multiplier moves the pattern by hundreds
/// of radians instantly, so it appears to spin up and settle rather than simply
/// moving faster. This was a real bug in the macOS version before it was a
/// comment here.
#[derive(Default, Debug)]
pub struct PhaseClock {
    pub flow: f64,
    pub warp: f64,
    last: Option<f64>,
}

impl PhaseClock {
    /// Advance by however long has passed, at the current speed.
    ///
    /// The delta is clamped because rendering pauses while the ring is hidden.
    /// Without it, the first frame back advances the pattern by however long it
    /// was away and the ring visibly jumps.
    pub fn advance(&mut self, now: f64, speed: f64) {
        let dt = match self.last {
            Some(last) => (now - last).clamp(0.0, 0.1),
            None => 0.0,
        };
        self.last = Some(now);
        self.flow += dt * speed;
        self.warp += dt * speed * 0.4;
    }
}

// ---------------------------------------------------------------------------

/// The GPU-side constant block. **This layout must match `shaders/ring.frag`
/// and `shaders/ring.vert` byte for byte**, and both of those are ports of
/// `Shaders.metal` in the macOS project, so all four agree by construction.
///
/// The macOS version calls a mismatch here "the single nastiest bug available
/// in this codebase", because it produces a garbled ring with no compile error
/// anywhere. The defence is the same one it uses: store *only* 16-byte-aligned
/// vectors, plus one explicitly padded `[f32; 2]`, and pack the loose scalars
/// into `params0`/`params1` rather than letting them sit between the vectors.
/// Readable names are exposed as accessors.
///
/// These offsets are simultaneously Metal's default struct layout and GLSL's
/// `std140`, which is why the same 112 bytes can be uploaded unchanged.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Uniforms {
    /// Drawable size in **physical** pixels. Offset 0.
    pub resolution: [f32; 2],
    /// Offset 8. `.0` carries the debug mode; `.1` is padding and must stay 0.
    pub pad0: [f32; 2],
    /// The window's rect in surface-local **physical** pixels, y up. Offset 16.
    pub window_rect: [f32; 4],
    /// Linear RGB in `.xyz`; `.w` unused. Offset 32.
    pub color_a: [f32; 4],
    /// Offset 48.
    pub color_b: [f32; 4],
    /// Offset 64.
    pub color_glow: [f32; 4],
    /// `cornerRadius, bandInner, bandOuter, flowPhase`. Offset 80.
    pub params0: [f32; 4],
    /// `intensity, warpPhase, noiseScale, glowFalloff`. Offset 96.
    pub params1: [f32; 4],
}

/// Byte size of the uniform block, matching `Uniforms.expectedStride` on macOS.
pub const UNIFORMS_SIZE: usize = 112;

// The macOS renderer asserts its stride at startup. Rust can do it at compile
// time, so a layout mistake never reaches a running compositor.
const _: () = assert!(core::mem::size_of::<Uniforms>() == UNIFORMS_SIZE);
const _: () = assert!(core::mem::align_of::<Uniforms>() == 4);

/// Generates the readable accessors over the packed scalars, so no call site
/// ever writes `params1[3]` and means `glowFalloff`.
macro_rules! packed_scalar {
    ($get:ident, $set:ident, $field:ident[$i:literal], $doc:expr) => {
        #[doc = $doc]
        pub fn $get(&self) -> f32 {
            self.$field[$i]
        }
        #[doc = $doc]
        pub fn $set(&mut self, value: f32) -> &mut Self {
            self.$field[$i] = value;
            self
        }
    };
}

impl Uniforms {
    packed_scalar!(corner_radius, set_corner_radius, params0[0],
        "Corner radius in pixels. The shader clamps it to the window's half-extent.");
    packed_scalar!(band_inner, set_band_inner, params0[1],
        "How far the band reaches inside the window edge, in pixels.");
    packed_scalar!(band_outer, set_band_outer, params0[2],
        "How far it reaches outside, in pixels.");
    packed_scalar!(flow_phase, set_flow_phase, params0[3],
        "Accumulated angle the noise has traveled around the ring, in radians. \
         A phase rather than a timestamp: a speed change must not move the \
         pattern, only change how fast it advances from here.");
    packed_scalar!(intensity, set_intensity, params1[0],
        "0…1, from the animator.");
    packed_scalar!(warp_phase, set_warp_phase, params1[1],
        "The turbulence's own accumulated phase, on a slower clock than the flow.");
    packed_scalar!(noise_scale, set_noise_scale, params1[2],
        "Spatial frequency of the turbulence around the perimeter.");
    packed_scalar!(glow_falloff, set_glow_falloff, params1[3],
        "Pixel falloff of the outer bloom.");

    /// 0 = normal. 1 = paint rasterized fragments blue. 2 = also replace the
    /// ring geometry with a full-viewport quad.
    ///
    /// A diagnostic, not a feature, and the one worth keeping: it separates
    /// "the surface is not compositing" from "the ring geometry produces
    /// nothing". SPEC.md §13 calls a wrong scale factor the worst bug in the
    /// macOS version precisely because it fails silently in the first way.
    pub fn debug_mode(&self) -> f32 {
        self.pad0[0]
    }
    pub fn set_debug_mode(&mut self, value: f32) -> &mut Self {
        self.pad0[0] = value;
        self
    }

    /// Linear RGB triples from [`crate::style::Rotator::colors`], widened to
    /// the `vec4`s the block stores.
    pub fn set_colors(&mut self, colors: [[f32; 3]; 3]) -> &mut Self {
        self.color_a = [colors[0][0], colors[0][1], colors[0][2], 1.0];
        self.color_b = [colors[1][0], colors[1][1], colors[1][2], 1.0];
        self.color_glow = [colors[2][0], colors[2][1], colors[2][2], 1.0];
        self
    }

    /// The block as bytes, ready for `glBufferSubData`.
    pub fn as_bytes(&self) -> &[u8] {
        // Safe: `#[repr(C)]` over `f32` arrays has no padding and no niches, so
        // every one of the 112 bytes is initialized.
        unsafe {
            core::slice::from_raw_parts(
                (self as *const Uniforms).cast::<u8>(),
                UNIFORMS_SIZE,
            )
        }
    }
}

// ---------------------------------------------------------------------------

/// Band widths must not swallow a small window whole.
///
/// The vertex stage relies on this: it builds the band's inner bounds from
/// `bandInner`, and without the clamp those bounds invert on a window narrower
/// than twice the band.
pub fn clamp_band(inner: f32, outer: f32, rect: Rect) -> (f32, f32) {
    let limit = (rect.w.min(rect.h) as f32) / 4.0;
    (inner.min(limit), outer.min(limit))
}

/// Convert a window rect in compositor coordinates into surface-local physical
/// pixels, y up.
///
/// `surface_origin` and `surface_height_logical` are in the compositor's own
/// **logical** coordinate space, the same one `window` is in — the flip happens
/// before `scale` is applied. Passing a buffer height here instead is wrong by
/// exactly the scale factor, and on a 1x output the two are equal, so the
/// mistake survives every test on ordinary hardware and appears only on a
/// scaled display.
///
/// The macOS version's worst bug was a wrong scale factor: every frame rendered
/// and presented correctly and composited to nothing, with no error anywhere.
/// Keep this in one place and test it.
pub fn window_rect_in_surface(
    window: Rect,
    surface_origin: (i32, i32),
    surface_height_logical: i32,
    scale: f32,
) -> [f32; 4] {
    let x = (window.x - surface_origin.0) as f32;
    // Compositor coordinates run y down; the shader works y up.
    let y = (surface_height_logical - (window.y - surface_origin.1) - window.h) as f32;
    [x * scale, y * scale, window.w as f32 * scale, window.h as f32 * scale]
}
