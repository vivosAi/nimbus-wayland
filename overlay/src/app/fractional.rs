//! Fractional scaling, handled from the start rather than retrofitted.
//!
//! SPEC.md §13 puts this first among the open questions, and for a good
//! reason: a wrong scale factor was the worst bug in the macOS version, because
//! every frame rendered correctly, presented correctly, and composited to
//! nothing, with no error anywhere to say so.
//!
//! Two protocols, used together or not at all:
//!
//! * `wp_fractional_scale_v1` reports the scale the compositor actually wants,
//!   in 120ths. 1.25 arrives as 150.
//! * `wp_viewporter` is what makes it usable. The buffer is rendered at
//!   `ceil(logical × scale)` physical pixels with `buffer_scale` left at 1, and
//!   the viewport's destination is set to the logical size. Without it, Wayland
//!   can only express integer buffer scales and a 1.25 output gets a buffer of
//!   the wrong size — silently.

use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};

use super::Nimbus;

/// The compositor reports scale in 120ths of the logical size.
const SCALE_DENOMINATOR: f32 = 120.0;

/// Per-surface scale state.
#[derive(Debug, Default)]
pub struct FractionalScale {
    object: Option<WpFractionalScaleV1>,
    viewport: Option<WpViewport>,
    /// Numerator in 120ths, once the compositor has told us.
    numerator: Option<u32>,
    /// `wl_surface.set_buffer_scale`, used only where the fractional protocol
    /// is absent. The two must never be combined.
    integer_fallback: i32,
}

impl FractionalScale {
    /// The scale to render at. 1.0 until the compositor says otherwise, which
    /// is correct for every ordinary display.
    pub fn scale(&self) -> f32 {
        match self.numerator {
            Some(n) if n > 0 => n as f32 / SCALE_DENOMINATOR,
            _ => self.integer_fallback.max(1) as f32,
        }
    }

    pub fn set_integer_fallback(&mut self, factor: i32) {
        // Ignored where the fractional protocol is in use: the compositor is
        // already telling us something more precise.
        if self.object.is_none() {
            self.integer_fallback = factor.max(1);
        }
    }

    /// Tell the compositor what logical size the buffer stands for. Only
    /// meaningful with a viewport; without one the buffer size and buffer scale
    /// carry that information instead.
    pub fn set_logical_size(&self, logical: (u32, u32)) {
        if let Some(viewport) = &self.viewport {
            if logical.0 > 0 && logical.1 > 0 {
                viewport.set_destination(logical.0 as i32, logical.1 as i32);
            }
        }
    }

    pub fn destroy(&mut self) {
        if let Some(o) = self.object.take() {
            o.destroy();
        }
        if let Some(v) = self.viewport.take() {
            v.destroy();
        }
    }
}

impl Default for Manager {
    fn default() -> Self {
        Manager { manager: None, viewporter: None }
    }
}

/// The two globals, if this compositor has them.
#[derive(Debug)]
pub struct Manager {
    manager: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
}

impl Manager {
    /// Both globals are optional. A compositor without them is not an error —
    /// it simply has no fractional scaling, and the integer path is correct.
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<Nimbus>) -> Self {
        let manager = globals.bind::<WpFractionalScaleManagerV1, _, _>(qh, 1..=1, ()).ok();
        let viewporter = globals.bind::<WpViewporter, _, _>(qh, 1..=1, ()).ok();
        if manager.is_some() != viewporter.is_some() {
            eprintln!(
                "nimbus: this compositor offers only one of wp_fractional_scale_v1 \
                 and wp_viewporter; falling back to integer scaling"
            );
        }
        Manager { manager, viewporter }
    }

    /// Start watching a surface's scale. Returns state that reports 1.0 until
    /// the compositor says otherwise.
    pub fn track(&self, qh: &QueueHandle<Nimbus>, surface: &WlSurface) -> FractionalScale {
        // Neither protocol is useful without the other: the scale is unusable
        // without a viewport to express it, and a viewport with no fractional
        // scale has nothing to express.
        let (Some(manager), Some(viewporter)) = (&self.manager, &self.viewporter) else {
            return FractionalScale { integer_fallback: 1, ..Default::default() };
        };

        let object = manager.get_fractional_scale(surface, qh, surface.clone());
        let viewport = viewporter.get_viewport(surface, qh, ());
        // With a viewport carrying the logical size, the buffer is its own
        // physical size at scale 1.
        surface.set_buffer_scale(1);

        FractionalScale {
            object: Some(object),
            viewport: Some(viewport),
            numerator: None,
            integer_fallback: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Neither manager has any events; the per-surface object has exactly one.
// ---------------------------------------------------------------------------

impl Dispatch<WpFractionalScaleManagerV1, ()> for Nimbus {
    fn event(
        _: &mut Self,
        _: &WpFractionalScaleManagerV1,
        _: <WpFractionalScaleManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpViewporter, ()> for Nimbus {
    fn event(
        _: &mut Self,
        _: &WpViewporter,
        _: <WpViewporter as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpViewport, ()> for Nimbus {
    fn event(
        _: &mut Self,
        _: &WpViewport,
        _: <WpViewport as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpFractionalScaleV1, WlSurface> for Nimbus {
    fn event(
        state: &mut Self,
        _: &WpFractionalScaleV1,
        event: <WpFractionalScaleV1 as Proxy>::Event,
        surface: &WlSurface,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else {
            return;
        };

        let Some(index) = state
            .surfaces
            .iter()
            .position(|s| {
                use smithay_client_toolkit::shell::WaylandSurface;
                s.layer.wl_surface() == surface
            })
        else {
            return;
        };

        if state.surfaces[index].fractional.numerator == Some(scale) {
            return;
        }
        state.surfaces[index].fractional.numerator = Some(scale);

        // The buffer is now the wrong physical size for this scale. Resize it
        // and repaint, rather than waiting for a configure that may never come:
        // a scale change on its own does not resize the surface logically.
        state.resize_surface(index);
        let now = state.now();
        let visible = state.showing(now).cloned();
        let lit = visible
            .as_ref()
            .is_some_and(|s| s.output == state.surfaces[index].name);
        state.paint(index, lit.then(|| visible.unwrap()), now, qh);
    }
}
