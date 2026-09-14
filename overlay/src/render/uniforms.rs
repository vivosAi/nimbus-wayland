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

/// The constant block handed to the shader. Field order matches
/// `shaders/ring.frag`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Uniforms {
    pub resolution: [f32; 2],
    /// The window's rect **in surface-local pixels**, y up.
    pub window_rect: [f32; 4],
    pub corner_radius: f32,
    pub band_inner: f32,
    pub band_outer: f32,
    pub flow_phase: f32,
    pub warp_phase: f32,
    pub intensity: f32,
    pub noise_scale: f32,
    pub glow_falloff: f32,
    pub color_a: [f32; 3],
    pub color_b: [f32; 3],
    pub color_glow: [f32; 3],
}

/// Band widths must not swallow a small window whole.
pub fn clamp_band(inner: f32, outer: f32, rect: Rect) -> (f32, f32) {
    let limit = (rect.w.min(rect.h) as f32) / 4.0;
    (inner.min(limit), outer.min(limit))
}

/// Convert a window rect in compositor coordinates into surface-local pixels,
/// y up, scaled for the output.
///
/// The macOS version's worst bug was a wrong scale factor: every frame rendered
/// and presented correctly and composited to nothing, with no error anywhere.
/// Keep this in one place and test it.
pub fn window_rect_in_surface(
    window: Rect,
    surface_origin: (i32, i32),
    surface_height: i32,
    scale: f32,
) -> [f32; 4] {
    let x = (window.x - surface_origin.0) as f32;
    // Compositor coordinates run y down; the shader works y up.
    let y = (surface_height - (window.y - surface_origin.1) - window.h) as f32;
    [x * scale, y * scale, window.w as f32 * scale, window.h as f32 * scale]
}
