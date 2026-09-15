//! The program: one layer-shell surface per output, a frame loop, and the
//! wiring between the compositor's idea of focus and the renderer's.
//!
//! Linux-only. Everything it decides has already been decided and tested
//! elsewhere — [`crate::focus::tracker`] owns *whether* there is a ring,
//! [`crate::style`] owns what colour and how bright, and
//! [`crate::render::uniforms`] owns the geometry. This file owns Wayland.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use calloop::{
    timer::{TimeoutAction, Timer},
    EventLoop, LoopHandle, RegistrationToken,
};
use calloop_wayland_source::WaylandSource;
use khronos_egl as egl;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_surface},
    Connection, Proxy, QueueHandle,
};
use wayland_egl::WlEglSurface;

use crate::config::{Config, IdleBehavior, State};
use crate::control;
use crate::focus::geometry::{self, Monitor};
use crate::focus::tracker::Tracker;
use crate::focus::{Event, FocusState};
use crate::render::gles::GlContext;
use crate::render::uniforms::{clamp_band, window_rect_in_surface, Uniforms};
use crate::style::{palettes, Animator, Rng, Rotator};

mod fractional;
mod idle;
pub mod server;
use fractional::FractionalScale;

/// How often to ask where the focused window is while a ring is on screen and
/// the window is at rest.
///
/// Hyprland has no event for an interactive move or resize — `movewindow`
/// means "sent to another workspace", and there is no `resizewindow` at all —
/// so a window that changes shape under a ring can only be noticed by asking.
/// Four times a second is one small socket read each, and nothing at all runs
/// when there is no ring to keep honest.
const POLL_AT_REST: Duration = Duration::from_millis(250);

/// How often to ask once a change has been seen. Fast enough that the ring
/// lands on the window the moment it stops, rather than a quarter-second later.
const POLL_WHILE_MOVING: Duration = Duration::from_millis(60);

/// How long after the last observed change to keep polling at the fast rate.
/// Comfortably longer than [`crate::focus::tracker::SETTLE_DELAY`], so the
/// settle itself is observed at the fast rate too.
const FAST_POLL_FOR: Duration = Duration::from_millis(500);

/// The ring's proportions all come from [`crate::config`] now, which carries
/// the macOS defaults verbatim. SPEC.md §8 is explicit that they ship unchanged
/// and get looked at before anyone reaches for a setting — that has now
/// happened, on a tiled desktop, and they were right.

/// Whether an output's surface is drawing this frame, and at what size.
struct OutputSurface {
    output: wl_output::WlOutput,
    /// The compositor's name for it, e.g. `DP-1`. Matched against the output
    /// name Hyprland reports for the focused window.
    name: String,
    layer: LayerSurface,
    wl_egl: Option<WlEglSurface>,
    egl_surface: Option<egl::Surface>,
    fractional: FractionalScale,
    /// Logical size, from the layer-shell configure.
    logical: (u32, u32),
    /// Logical position in compositor space, so a window's global coordinates
    /// can be made surface-local.
    origin: (i32, i32),
    /// True once a frame callback is outstanding, so we never queue two.
    frame_pending: bool,
    /// Whether this surface drew a ring last frame. Only the output holding the
    /// focused window draws; the rest must be cleared exactly once and then
    /// left alone, rather than re-cleared every frame forever.
    was_lit: bool,
}

impl OutputSurface {
    /// Physical pixel size of the buffer, which is the logical size scaled and
    /// rounded up. A fractional scale of 1.5 on a 1280-wide output gives 1920.
    fn physical(&self) -> (i32, i32) {
        let scale = self.fractional.scale();
        (
            (self.logical.0 as f32 * scale).ceil() as i32,
            (self.logical.1 as f32 * scale).ceil() as i32,
        )
    }
}

pub struct Nimbus {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    fractional_manager: fractional::Manager,
    idle_manager: idle::Manager,

    gl: GlContext,
    surfaces: Vec<OutputSurface>,

    config: Config,
    state: State,
    tracker: Tracker,
    animator: Animator,
    rotator: Rotator,
    clock: crate::render::uniforms::PhaseClock,
    rng: Rng,

    /// Kept so callbacks driven by something other than a Wayland event — the
    /// compositor socket, a pending deadline — can still ask for a frame.
    queue_handle: QueueHandle<Nimbus>,
    /// For arming the pacing timer from inside a frame callback.
    loop_handle: LoopHandle<'static, Nimbus>,
    /// The pacing timer, while one is outstanding, so a second early frame
    /// callback does not arm a second one.
    pace_timer: Option<RegistrationToken>,
    /// The geometry poll, while one is running. See [`POLL_AT_REST`].
    poll_timer: Option<RegistrationToken>,
    /// When the poll last saw the focused window change shape or place, which
    /// decides between the two poll rates.
    last_geometry_change: Option<Instant>,
    /// Hyprland's view of the outputs, for translating `clients[].monitor`.
    monitors: Vec<Monitor>,
    events: mpsc::Receiver<Event>,
    /// Requests from the control socket, answered on the main loop because only
    /// it knows the current settings.
    calls: mpsc::Receiver<server::Call>,
    started: Instant,
    last_rotation: Instant,
    /// When the last frame was drawn, so the frame rate can be capped. The
    /// macOS version leaves this to `preferredFramesPerSecond`; Wayland frame
    /// callbacks arrive at the refresh rate, so the cap is ours to apply.
    last_frame: Option<Instant>,
    /// True while the compositor says nobody has touched anything for
    /// `idle_threshold` seconds.
    idle: bool,
    exit: bool,
}

impl Nimbus {
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Ask the compositor what has focus now, and where it is.
    ///
    /// Called for events that change where a window is without saying where it
    /// went, which is most of them. It asks *what is focused*, not *where is
    /// the window I remember*. The difference showed up on a switch to an empty
    /// workspace: Hyprland sends the empty focus event and the workspace event
    /// one millisecond apart, and a rescan that looked the remembered window up
    /// in the client list found it, still on the other workspace with a good
    /// rect, and handed it back as a fresh focus. That cancelled the pending
    /// unfocus and left a ring around a window that was not on screen.
    fn rescan(&mut self) {
        if self.tracker.focused().is_none() {
            return;
        }
        match geometry::active_window(&self.monitors) {
            Ok(Some(state)) => self.place(state),
            // Nothing has focus. The grace period in the tracker still applies,
            // so the transient empty event mid-switch stays harmless.
            Ok(None) => {
                let now = self.now();
                self.tracker.note_unfocus(now);
            }
            Err(e) => eprintln!("nimbus: {e}"),
        }
    }

    /// Hand a placed window to the tracker, flaring if it is a different one.
    fn place(&mut self, state: FocusState) {
        let now = self.now();
        if self.tracker.focus(state, now) {
            // SPEC.md §10: the flare fires only on a change to a different
            // window, never on a geometry update, or dragging keeps it lit and
            // it never settles.
            self.animator.flare(now);
            if std::env::var_os("NIMBUS_DEBUG").is_some() {
                eprintln!("nimbus: flare at {now:.3}");
            }
        }
    }

    /// Turn a window address into a placed [`FocusState`], and tell the tracker.
    ///
    /// Asks for the focused window first, which is one small object, and falls
    /// back to the whole client list only when the compositor's idea of focus
    /// has already moved on. Hyprland 0.56 re-emits the focus event on every
    /// title change — a terminal with a spinner in its title fires it once a
    /// second — so the common case must be the cheap one.
    fn resolve(&mut self, address: &str) {
        let wanted = crate::focus::normalize_address(address);
        let placed = match geometry::active_window(&self.monitors) {
            Ok(Some(state)) if state.address == wanted => Ok(Some(state)),
            _ => geometry::locate(address, &self.monitors),
        };
        match placed {
            Ok(Some(state)) => self.place(state),
            // A window we cannot place is a window we cannot ring. This happens
            // briefly while a window is mapping.
            Ok(None) => {
                if std::env::var_os("NIMBUS_DEBUG").is_some() {
                    eprintln!("nimbus: could not place {address}");
                }
            }
            Err(e) => eprintln!("nimbus: {e}"),
        }
    }

    fn reload_monitors(&mut self) {
        match geometry::monitors() {
            Ok(m) => self.monitors = m,
            Err(e) => eprintln!("nimbus: {e}"),
        }
    }

    fn handle(&mut self, event: Event) {
        if std::env::var_os("NIMBUS_DEBUG").is_some() {
            eprintln!("nimbus: event {event:?}");
        }
        match event {
            Event::Focused(address) => self.resolve(&address),
            Event::Unfocused => {
                let now = self.now();
                self.tracker.note_unfocus(now);
            }
            Event::Moving(_) => {
                if self.config.hide_while_dragging {
                    let now = self.now();
                    self.tracker.note_moving(now);
                }
            }
            Event::Rescan => {
                self.reload_monitors();
                self.rescan();
            }
            Event::OutputAdded(_) | Event::OutputRemoved(_) => {
                // Wayland tells us about outputs through `OutputHandler`; this
                // is only Hyprland's id-to-name table needing a refresh.
                self.reload_monitors();
                self.rescan();
            }
        }
    }

    /// Advance the animation and paint every surface that needs it.
    fn render(&mut self, qh: &QueueHandle<Self>) {
        if !self.frame_is_due() {
            self.arm_pace_timer();
            return;
        }
        self.last_frame = Some(Instant::now());
        let now = self.now();

        self.tracker.retire_confirmed_unfocus(now);

        // Rotation is off entirely at 0, which is the only way to say "leave
        // the colour alone" without disabling the ring.
        if self.config.palette_interval > 0.0
            && self.last_rotation.elapsed().as_secs_f64() >= self.config.palette_interval
        {
            let next = self.rotator.pick_next(&mut self.rng);
            self.rotator.transition_to(next, now);
            self.last_rotation = Instant::now();
            self.persist_rotation();
        }

        // Speed and width relax on the same curve as brightness, so the ring
        // settles as one thing rather than finishing brightening before it
        // finishes shrinking.
        let speed = self.config.motion_speed.flow_speed()
            * self.animator.speed_scale(now) as f64;
        self.clock.advance(now, speed);

        let visible = self.showing(now).cloned();
        for index in 0..self.surfaces.len() {
            let lit = visible
                .as_ref()
                .is_some_and(|s| s.output == self.surfaces[index].name);
            self.paint(index, lit.then(|| visible.clone().unwrap()), now, qh);
        }
    }

    /// Paint one output's surface: the ring if the focused window is on it,
    /// otherwise nothing at all.
    fn paint(
        &mut self,
        index: usize,
        state: Option<FocusState>,
        now: f64,
        qh: &QueueHandle<Self>,
    ) {
        let (Some(egl_surface), (width, height)) = (
            self.surfaces[index].egl_surface,
            self.surfaces[index].physical(),
        ) else {
            return;
        };
        if width <= 0 || height <= 0 {
            return;
        }

        // An output with no ring needs clearing once, when the ring leaves it.
        // Clearing every frame forever would keep the compositor recompositing
        // for a surface that stays empty.
        if state.is_none() && !self.surfaces[index].was_lit {
            return;
        }

        if self.gl.make_current(egl_surface, width, height).is_err() {
            return;
        }

        match &state {
            None => self.gl.clear(),
            Some(focus) => {
                let scale = self.surfaces[index].fractional.scale();
                let logical_height = self.surfaces[index].logical.1 as i32;
                let origin = self.surfaces[index].origin;

                let (band_inner, band_outer) = self.config.band_width.points();
                let (inner, outer) = clamp_band(band_inner, band_outer, focus.rect);

                let mut u = Uniforms {
                    resolution: [width as f32, height as f32],
                    window_rect: window_rect_in_surface(
                        focus.rect,
                        origin,
                        logical_height,
                        scale,
                    ),
                    ..Default::default()
                };
                let flare = self.animator.band_scale(now);
                u.set_corner_radius(Config::CORNER_RADIUS * scale)
                    .set_band_inner(inner * flare * scale)
                    .set_band_outer(outer * flare * scale)
                    .set_flow_phase(self.clock.flow as f32)
                    .set_warp_phase(self.clock.warp as f32)
                    .set_intensity(self.animator.intensity(now))
                    .set_debug_mode(self.config.debug_mode as f32)
                    .set_noise_scale(self.config.turbulence.noise_scale())
                    .set_glow_falloff(self.config.glow_falloff() * scale)
                    .set_colors(self.rotator.colors(now));

                if std::env::var_os("NIMBUS_DEBUG").is_some() {
                    eprintln!(
                        "nimbus: frame t={now:.3} intensity={:.3} flow={:.3} \
                         noise={:.2} glow={:.2} corner={:.1} band=({:.1},{:.1}) rect={:?}",
                        u.intensity(), u.flow_phase(), u.noise_scale(), u.glow_falloff(),
                        u.corner_radius(), u.band_inner(), u.band_outer(), u.window_rect
                    );
                }
                self.gl.draw(&u);
            }
        }

        let surface = self.surfaces[index].layer.wl_surface().clone();
        // Ask for the next frame *before* swapping, so the callback is attached
        // to the commit the swap performs.
        if !self.surfaces[index].frame_pending {
            surface.frame(qh, surface.clone());
            self.surfaces[index].frame_pending = true;
        }
        let _ = self.gl.swap_buffers(egl_surface);

        self.surfaces[index].was_lit = state.is_some();
    }

    /// Re-size the buffer to the surface's current physical size.
    ///
    /// Called both on a layer-shell configure and on a fractional scale change,
    /// because a scale change alters the physical size without altering the
    /// logical one and so never produces a configure of its own.
    fn resize_surface(&mut self, index: usize) {
        let (pw, ph) = self.surfaces[index].physical();
        if pw <= 0 || ph <= 0 {
            return;
        }
        self.surfaces[index]
            .fractional
            .set_logical_size(self.surfaces[index].logical);
        if let Some(wl_egl) = self.surfaces[index].wl_egl.as_ref() {
            wl_egl.resize(pw, ph, 0, 0);
        }
    }

    /// Whether enough time has passed to draw another frame.
    ///
    /// Wayland frame callbacks arrive at the display's refresh rate, so the cap
    /// has to be applied here — on macOS `preferredFramesPerSecond` does it.
    /// This is the setting that decides what the program costs: cost is very
    /// nearly linear in frame rate and almost independent of what the shader
    /// does.
    fn frame_is_due(&self) -> bool {
        let Some(last) = self.last_frame else { return true };
        // A small tolerance, or a frame that arrives a hair early is dropped and
        // the effective rate halves.
        last.elapsed() + Duration::from_micros(500) >= self.frame_budget()
    }

    fn frame_budget(&self) -> Duration {
        Duration::from_secs_f64(1.0 / self.config.frame_rate.max(1) as f64)
    }

    /// Come back when the rest of the frame budget has elapsed.
    ///
    /// Dropping an early frame callback also drops the request for the next
    /// one, because callbacks are only re-armed by a paint. Without this,
    /// nothing woke the loop again until the dispatch timeout, and the ring ran
    /// at about 10 fps whatever `frame_rate` said: measured at 11.7 fps with
    /// the setting at 60 on a 75 Hz display, one frame every 115 ms.
    ///
    /// A timer for the remainder rather than re-arming the callback, so the
    /// loop wakes once per frame it intends to draw and not once per refresh.
    fn arm_pace_timer(&mut self) {
        if self.pace_timer.is_some() {
            return;
        }
        let Some(last) = self.last_frame else { return };
        let remaining = self.frame_budget().saturating_sub(last.elapsed());
        let token = self.loop_handle.insert_source(
            Timer::from_duration(remaining),
            |_, _, state: &mut Nimbus| {
                state.pace_timer = None;
                let now = state.now();
                if state.wants_frames(now) {
                    let qh = state.queue_handle.clone();
                    state.render(&qh);
                }
                TimeoutAction::Drop
            },
        );
        self.pace_timer = token.ok();
    }

    /// Whether anything still needs drawing, or the loop can go quiet.
    ///
    /// SPEC.md §10: idle stops the animation but must not remove the ring.
    /// Walking back to the machine and looking at which window has focus,
    /// before touching anything, is the case this exists for — so going quiet
    /// must mean "stop advancing the pattern", never "stop showing the ring".
    fn wants_frames(&self, now: f64) -> bool {
        // Idle stops the animation. It does not stop the ring: no more frames
        // means the last one stays on screen, which costs nothing and leaves
        // the mark exactly where it was.
        if self.idle && self.config.idle_behavior != IdleBehavior::AlwaysAnimate {
            return false;
        }
        self.showing(now).is_some() || self.tracker.has_pending_deadline(now)
    }

    /// The window to ring right now, after every reason not to.
    ///
    /// The tracker answers "does anything have focus, and is it at rest". This
    /// adds the reasons the *user* has given for not wanting a ring, which the
    /// tracker has no business knowing about.
    fn showing(&self, now: f64) -> Option<&FocusState> {
        let state = self.tracker.visible(now)?;
        self.user_allows(state).then_some(state)
    }

    /// The user's reasons for not wanting a ring around this window, as
    /// distinct from the tracker's clock-based ones (unfocus grace, a drag in
    /// progress). Separate because the geometry poll needs the first set
    /// without the second: a window hidden mid-drag still has to be watched, or
    /// nothing would notice when it comes to rest.
    fn user_allows(&self, state: &FocusState) -> bool {
        if !self.config.enabled {
            return false;
        }
        // The only idle behaviour that takes the ring away. `Freeze` keeps it,
        // which is the point: SPEC.md §10 is explicit that walking back and
        // looking at which window has focus, before touching anything, is the
        // case this program exists for.
        if self.idle && self.config.idle_behavior == IdleBehavior::FadeOut {
            return false;
        }
        // A full-screen window has no neighbours to be confused with, and a
        // ring over a film or a game is exactly where it is least wanted.
        if self.config.hide_in_fullscreen && state.fullscreen {
            return false;
        }
        if self.config.exclusions.iter().any(|c| c == &state.class) {
            return false;
        }
        true
    }

    /// Whether the focused window's geometry is worth asking about.
    ///
    /// Not while idle: nobody is there to move anything, and a window that
    /// changes on its own will say so with an event. Not when the user has
    /// ruled the ring out: a fullscreen window cannot be resized, and leaving
    /// fullscreen is an event.
    fn wants_polling(&self) -> bool {
        !self.idle && self.tracker.focused().is_some_and(|s| self.user_allows(s))
    }

    /// Start the geometry poll if it should be running and is not.
    ///
    /// Cheap to call anywhere the answer might have changed: a focus event, a
    /// setting, coming back from idle.
    fn ensure_poll(&mut self) {
        if self.poll_timer.is_some() || !self.wants_polling() {
            return;
        }
        let token = self.loop_handle.insert_source(
            Timer::from_duration(POLL_AT_REST),
            |_, _, state: &mut Nimbus| state.poll_geometry(),
        );
        self.poll_timer = token.ok();
    }

    /// One poll: where is the focused window now, and has it moved?
    ///
    /// SPEC.md §7 rules out a re-sync timer, and this is not one. It does not
    /// compensate for events the compositor dropped; it compensates for events
    /// the compositor does not have. A focus change seen here is left to the
    /// event socket, which will report it properly and flare.
    fn poll_geometry(&mut self) -> TimeoutAction {
        if !self.wants_polling() {
            self.poll_timer = None;
            return TimeoutAction::Drop;
        }
        let Some(previous) = self.tracker.focused().cloned() else {
            self.poll_timer = None;
            return TimeoutAction::Drop;
        };

        match geometry::active_window(&self.monitors) {
            Ok(Some(current)) if current.address == previous.address => {
                let changed = current.rect != previous.rect
                    || current.output != previous.output
                    || current.fullscreen != previous.fullscreen;
                if changed {
                    let now = self.now();
                    if self.config.hide_while_dragging {
                        // Movement is the *absence* of rest; the tracker
                        // measures rest with its clock, and each change seen
                        // here pushes that clock forward.
                        self.tracker.note_moving(now);
                    }
                    // Same address, so this never flares: SPEC.md §10.
                    self.tracker.focus(current, now);
                    self.last_geometry_change = Some(Instant::now());
                    self.request_frame();
                }
            }
            // Nothing has focus. Usually the event socket has already said so;
            // if it has not, or said so and was overruled, this puts the ring
            // out within a poll interval. The tracker's grace period debounces
            // it exactly as it does the event.
            Ok(None) => {
                let now = self.now();
                self.tracker.note_unfocus(now);
            }
            // A different window: the event socket owns that change and will
            // deliver it with the flare it deserves.
            Ok(Some(_)) => {}
            Err(e) => {
                // The request socket failing means the compositor is going
                // away, at which point there is nothing to poll.
                eprintln!("nimbus: {e}");
                self.poll_timer = None;
                return TimeoutAction::Drop;
            }
        }

        let recently_moving = self
            .last_geometry_change
            .is_some_and(|t| t.elapsed() < FAST_POLL_FOR);
        TimeoutAction::ToDuration(if recently_moving { POLL_WHILE_MOVING } else { POLL_AT_REST })
    }
}

impl Nimbus {
    /// Answer one control-socket request.
    ///
    /// Settings take effect on the next frame with no restart, which is the
    /// whole reason this exists: the macOS menu changes things while you watch,
    /// and a port that needed a restart to change a colour would not be the
    /// same product.
    fn serve_call(&mut self, call: server::Call) {
        let answer = match call.request {
            control::Request::Status => self.status(),
            control::Request::Quit => {
                self.exit = true;
                serde_json::json!({ "ok": true }).to_string()
            }
            control::Request::SetPalette(name) => {
                let now = self.now();
                if self.rotator.select_by_name(&name, now) {
                    // Choosing by hand restarts the timer, so the next
                    // automatic change is a full interval away rather than
                    // arriving seconds after a deliberate choice.
                    self.last_rotation = Instant::now();
                    self.persist_rotation();
                    self.request_frame();
                    self.status()
                } else {
                    control::error_json(&format!("no palette called {name:?}"))
                }
            }
            control::Request::NextColor => {
                let now = self.now();
                let next = self.rotator.pick_next(&mut self.rng);
                self.rotator.transition_to(next, now);
                self.last_rotation = Instant::now();
                self.persist_rotation();
                self.request_frame();
                self.status()
            }
            control::Request::Reload => {
                let (config, warnings) = Config::load();
                self.adopt(config);
                if warnings.is_empty() {
                    self.status()
                } else {
                    serde_json::json!({ "ok": true, "warnings": warnings }).to_string()
                }
            }
            control::Request::Set(settings) => self.apply_settings(settings),
        };
        let _ = call.reply.send(answer);
    }

    fn status(&self) -> String {
        let views: Vec<control::PaletteView> = self
            .rotator
            .palettes()
            .iter()
            .enumerate()
            .map(|(i, p)| control::PaletteView {
                name: p.name,
                hex: p.hex_strings(),
                in_rotation: !self.rotator.is_disabled(i),
            })
            .collect();
        control::status_json(
            &self.config,
            &views,
            self.rotator.current_index(),
            self.showing(self.now()).is_some(),
        )
    }

    /// The compositor says you have walked away, or come back.
    fn set_idle(&mut self, idle: bool) {
        if self.idle == idle {
            return;
        }
        self.idle = idle;

        if !idle && self.config.flare_on_return {
            // Returning to the machine is exactly when you are most likely to
            // type into the wrong window, so it earns a full flare rather than
            // a silent resume.
            let now = self.now();
            self.animator.flare(now);
        }

        // Hiding and un-hiding both need a repaint; freezing does not, because
        // the last frame is already the right picture and leaving it alone is
        // the entire point.
        if self.config.idle_behavior == IdleBehavior::FadeOut || !idle {
            for s in &mut self.surfaces {
                s.was_lit = true;
            }
        }
        self.request_frame();
        self.ensure_poll();
    }

    /// Draw now, rather than at whatever time the next frame callback lands.
    ///
    /// A colour chosen by hand has to appear immediately; waiting up to a frame
    /// interval reads as the click not having registered.
    fn request_frame(&mut self) {
        self.last_frame = None;
        let qh = self.queue_handle.clone();
        self.render(&qh);
    }

    /// Validate against the real parser, apply live, then persist.
    ///
    /// Round-tripping through `Config::from_json` rather than setting fields
    /// directly means the socket cannot accept a value the config file would
    /// reject. One validator, one set of rules.
    fn apply_settings(&mut self, settings: serde_json::Map<String, serde_json::Value>) -> String {
        let mut merged = self.config_as_json();
        for (key, value) in &settings {
            merged.insert(key.clone(), value.clone());
        }
        let (config, warnings) =
            Config::from_json(&serde_json::Value::Object(merged).to_string());
        if !warnings.is_empty() {
            return control::error_json(&warnings.join("; "));
        }

        self.adopt(config);

        if let Some(path) = crate::config::config_path() {
            if let Err(e) = control::merge_into_config_file(&path, &settings) {
                // The change is already live; failing to write it down is worth
                // saying but not worth undoing.
                return serde_json::json!({
                    "ok": true,
                    "warning": format!("applied, but not saved: {e}"),
                })
                .to_string();
            }
        }
        self.status()
    }

    /// Take a new config and push the parts of it that live elsewhere.
    fn adopt(&mut self, config: Config) {
        self.animator.idle_intensity = config.idle_intensity;
        self.animator.flare_duration = config.flare_duration;
        self.rotator.set_disabled(&config.disabled_palettes);
        if config.idle_threshold != self.config.idle_threshold {
            let qh = self.queue_handle.clone();
            self.idle_manager.watch(&qh, config.idle_threshold);
            // A threshold of 0 means the compositor will never tell us we have
            // resumed, so a program left idle would stay frozen forever.
            if config.idle_threshold <= 0.0 {
                self.idle = false;
            }
        }
        self.config = config;
        // Every surface has to be repainted, including the ones that should now
        // be empty, so a change that hides the ring takes effect immediately
        // rather than leaving the last frame on screen.
        for s in &mut self.surfaces {
            s.was_lit = true;
        }
        self.last_frame = None;
        let qh = self.queue_handle.clone();
        self.render(&qh);
        self.ensure_poll();
    }

    fn config_as_json(&self) -> serde_json::Map<String, serde_json::Value> {
        let text = self.status();
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["settings"].as_object().cloned())
            .unwrap_or_default()
    }

    fn persist_rotation(&mut self) {
        self.state.palette_index = self.rotator.current_index();
        self.state.palette_recent = self.rotator.recent().to_vec();
        self.state.palette_changed_at = unix_now();
        self.state.save();
    }
}

/// Seconds since the epoch, for the persisted rotation clock. `Instant` cannot
/// be written to a file; this can.
fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Wayland plumbing
// ---------------------------------------------------------------------------

impl CompositorHandler for Nimbus {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        // Integer scale is the fallback for compositors without
        // `wp_fractional_scale_v1`. Where that protocol is present it wins, and
        // this is ignored — mixing the two is how you end up rendering at the
        // wrong size, which SPEC.md §13 calls the worst bug in the macOS
        // version because it fails silently.
        for s in &mut self.surfaces {
            if s.layer.wl_surface() == surface {
                s.fractional.set_integer_fallback(new_factor);
                s.layer.wl_surface().set_buffer_scale(new_factor);
            }
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        for s in &mut self.surfaces {
            if s.layer.wl_surface() == surface {
                s.frame_pending = false;
            }
        }
        let now = self.now();
        if self.wants_frames(now) {
            self.render(qh);
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Nimbus {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.add_output(qh, output);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            if let Some(s) = self.surfaces.iter_mut().find(|s| s.output == output) {
                s.origin = info.logical_position.unwrap_or(info.location);
                if let Some(name) = info.name {
                    s.name = name;
                }
            }
        }
        self.reload_monitors();
    }

    /// SPEC.md §6: a monitor unplugged while holding the ring must not leave a
    /// surface behind.
    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // Destroy the scale and viewport objects with the surface. Leaving
        // them behind leaks a protocol object per hotplug, and SPEC.md §6 is
        // explicit that unplugging a monitor while it holds the ring must not
        // leave a surface behind.
        self.surfaces.retain_mut(|s| {
            if s.output == output {
                s.fractional.destroy();
                false
            } else {
                true
            }
        });
        self.reload_monitors();
    }
}

impl LayerShellHandler for Nimbus {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.surfaces.retain_mut(|s| {
            if &s.layer == layer {
                s.fractional.destroy();
                false
            } else {
                true
            }
        });
        if self.surfaces.is_empty() {
            self.exit = true;
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(index) = self.surfaces.iter().position(|s| &s.layer == layer) else {
            return;
        };

        let (w, h) = configure.new_size;
        if w == 0 || h == 0 {
            return;
        }
        self.surfaces[index].logical = (w, h);
        let (pw, ph) = self.surfaces[index].physical();

        self.surfaces[index].fractional.set_logical_size((w, h));
        match self.surfaces[index].wl_egl.as_ref() {
            // Already mapped: a configure after the first is a resize.
            Some(_) => self.resize_surface(index),
            None => {
                let surface_id = layer.wl_surface().id();
                let Ok(wl_egl) = WlEglSurface::new(surface_id, pw, ph) else {
                    eprintln!("nimbus: could not create a wl_egl_window");
                    return;
                };
                let egl_surface = unsafe {
                    self.gl.egl().create_window_surface(
                        self.gl.display(),
                        self.gl.config(),
                        wl_egl.ptr() as egl::NativeWindowType,
                        None,
                    )
                };
                match egl_surface {
                    Ok(s) => {
                        self.surfaces[index].egl_surface = Some(s);
                        self.surfaces[index].wl_egl = Some(wl_egl);
                    }
                    Err(e) => {
                        eprintln!("nimbus: eglCreateWindowSurface failed: {e}");
                        return;
                    }
                }
            }
        }

        // A surface with no ring on it still has to be committed once, or the
        // compositor never maps it and the first focus change has nothing to
        // draw into.
        self.surfaces[index].was_lit = true;
        let now = self.now();
        let visible = self.showing(now).cloned();
        let lit = visible
            .as_ref()
            .is_some_and(|s| s.output == self.surfaces[index].name);
        self.paint(index, lit.then(|| visible.unwrap()), now, qh);
    }
}

impl ProvidesRegistryState for Nimbus {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_compositor!(Nimbus);
delegate_output!(Nimbus);
delegate_layer!(Nimbus);
delegate_registry!(Nimbus);

impl Nimbus {
    fn add_output(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let info = self.output_state.info(&output);
        let name = info
            .as_ref()
            .and_then(|i| i.name.clone())
            .unwrap_or_default();
        let origin = info
            .as_ref()
            .map(|i| i.logical_position.unwrap_or(i.location))
            .unwrap_or((0, 0));

        let surface = self.compositor.create_surface(qh);

        // Click-through. SPEC.md §2: on macOS this needs `ignoresMouseEvents`;
        // here it is an empty input region, and without it the overlay would
        // swallow every click on the desktop.
        if let Ok(region) = smithay_client_toolkit::compositor::Region::new(&self.compositor) {
            surface.set_input_region(Some(region.wl_region()));
            region.wl_region().destroy();
        }

        let fractional = self.fractional_manager.track(qh, &surface);

        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("nimbus"),
            Some(&output),
        );
        // Anchored on all four edges so the surface is the whole output: the
        // ring can be anywhere on it, and a window can sit against any edge.
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();

        self.surfaces.push(OutputSurface {
            output,
            name,
            layer,
            wl_egl: None,
            egl_surface: None,
            fractional,
            logical: (0, 0),
            origin,
            frame_pending: false,
            was_lit: false,
        });
    }
}

// ---------------------------------------------------------------------------

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Refuse to be the second copy. Two overlays draw two rings on the same
    // window, at twice the cost, and only one of them can hold the control
    // socket -- so the panel would be talking to whichever won the race.
    //
    // Checked before any Wayland work, because the failure should be a clear
    // sentence rather than a second surface appearing.
    if let Some(path) = control::socket_path() {
        if std::os::unix::net::UnixStream::connect(&path).is_ok() {
            return Err(
                "nimbus is already running.\n\
                 Use `nimbus-wayland quit` to stop it, or `nimbus-wayland status` \
                 to see what it is doing."
                    .into(),
            );
        }
    }

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<Nimbus>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|e| format!("wl_compositor is missing: {e}"))?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| {
        format!(
            "this compositor does not support wlr-layer-shell, which is the \
             only way to put a surface above normal windows: {e}"
        )
    })?;
    let fractional_manager = fractional::Manager::bind(&globals, &qh);
    let idle_manager = idle::Manager::bind(&globals, &qh);

    let gl = unsafe { GlContext::new(conn.backend().display_ptr() as *mut _) }?;
    gl.set_swap_interval(0);

    let (config, warnings) = Config::load();
    // Warn, never refuse. An overlay that will not start because one setting
    // was spelled wrong is worse than one that starts with a default.
    for w in &warnings {
        eprintln!("nimbus: {w}");
    }
    let state = State::load();

    let mut animator = Animator::default();
    animator.idle_intensity = config.idle_intensity;
    animator.flare_duration = config.flare_duration;

    let mut rotator = Rotator::new(palettes(), state.palette_index);
    rotator.restore(state.palette_index, &state.palette_recent);
    rotator.set_disabled(&config.disabled_palettes);

    // Resume the rotation clock where it left off rather than restarting the
    // full interval, so frequent restarts cannot stall the rotation forever.
    let elapsed = (unix_now() - state.palette_changed_at).max(0.0);
    let last_rotation = Instant::now()
        .checked_sub(Duration::from_secs_f64(elapsed.min(config.palette_interval.max(0.0))))
        .unwrap_or_else(Instant::now);

    let (tx, events) = mpsc::channel();
    let (call_tx, calls) = mpsc::channel();

    // Created before the state so the state can hold its handle: the pacing
    // timer is armed from inside a Wayland callback, which only sees `Nimbus`.
    let mut event_loop: EventLoop<'static, Nimbus> = EventLoop::try_new()?;
    let handle: LoopHandle<'static, Nimbus> = event_loop.handle();

    let mut nimbus = Nimbus {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        fractional_manager,
        idle_manager,
        gl,
        surfaces: Vec::new(),
        config,
        state,
        tracker: Tracker::default(),
        animator,
        rotator,
        clock: Default::default(),
        rng: Rng::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x2545_F491),
        ),
        queue_handle: qh.clone(),
        loop_handle: handle.clone(),
        pace_timer: None,
        poll_timer: None,
        last_geometry_change: None,
        monitors: geometry::monitors().unwrap_or_default(),
        events,
        calls,
        started: Instant::now(),
        last_rotation,
        last_frame: None,
        idle: false,
        exit: false,
    };

    // Ask the compositor to tell us when nobody is there. Off by default:
    // `idle_threshold` is 0, because the ring is wanted at all times and a
    // sleeping display stops rendering anyway — nobody can see it.
    if nimbus.config.idle_threshold > 0.0 && !nimbus.idle_manager.is_available() {
        eprintln!(
            "nimbus: this compositor has no ext_idle_notify_v1, so idle_threshold does nothing"
        );
    }
    let threshold = nimbus.config.idle_threshold;
    nimbus.idle_manager.watch(&qh, threshold);

    // Whatever already has focus, so the ring is there on startup rather than
    // waiting for the user to switch windows to find out the program works.
    if let Ok(reply) = geometry::request("j/activewindow") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&reply) {
            if let Some(address) = value["address"].as_str() {
                nimbus.resolve(address);
            }
        }
    }
    nimbus.ensure_poll();

    WaylandSource::new(conn.clone(), event_queue).insert(handle.clone())?;

    // The compositor socket runs on its own thread; the Wayland loop owns the
    // main one. A pipe wakes the loop so a focus change is acted on
    // immediately, rather than at whatever time the next frame happens to be.
    let (ping, ping_source) = calloop::ping::make_ping()?;
    let control_ping = ping.clone();
    std::thread::spawn(move || {
        let result = crate::focus::hyprland::listen(|event| {
            if tx.send(event).is_ok() {
                ping.ping();
            }
        });
        if let Err(e) = result {
            eprintln!("nimbus: the compositor event socket closed: {e}");
        }
    });

    // The control socket. Not fatal if it cannot be bound: the ring is the
    // product, and a machine with no XDG_RUNTIME_DIR should still get one.
    match server::bind() {
        Ok((listener, path)) => {
            std::thread::spawn(move || {
                server::serve(listener, |call| {
                    if call_tx.send(call).is_ok() {
                        control_ping.ping();
                    }
                });
            });
            if std::env::var_os("NIMBUS_DEBUG").is_some() {
                eprintln!("nimbus: listening on {}", path.display());
            }
        }
        Err(e) => eprintln!("nimbus: no control socket ({e}); settings stay as configured"),
    }

    handle.insert_source(ping_source, |_, _, state: &mut Nimbus| {
        let qh = state.queue_handle.clone();
        while let Ok(event) = state.events.try_recv() {
            state.handle(event);
        }
        while let Ok(call) = state.calls.try_recv() {
            state.serve_call(call);
        }
        let now = state.now();
        if state.wants_frames(now) {
            state.render(&qh);
        }
        state.ensure_poll();
    })?;

    while !nimbus.exit {
        event_loop.dispatch(Duration::from_millis(100), &mut nimbus)?;
        // Both of the tracker's decisions are made by a clock rather than by an
        // event, so the loop has to come back and make them even if nothing
        // else happens.
        let now = nimbus.now();
        if nimbus.wants_frames(now) {
            nimbus.render(&qh);
        }
        nimbus.ensure_poll();
    }
    Ok(())
}
