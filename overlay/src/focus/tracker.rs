//! Turns the compositor's raw event stream into the one question the renderer
//! asks: *is there a ring right now, and around what?*
//!
//! Both delays below exist because of things Hyprland actually does, observed
//! on 0.56.2 rather than read off the wiki. Neither is a re-sync timer of the
//! kind that is ruled out: nothing here polls, and nothing here compensates
//! for a dropped event. They debounce events that genuinely arrive.

use super::FocusState;

/// How long to wait before believing that nothing has focus.
///
/// Hyprland emits a transient *empty* focus event in the middle of every focus
/// change, before the new window's:
///
/// ```text
/// activewindow>>,
/// activewindowv2>>                 ← this one, for about one frame
/// activewindow>>foot,nimbus
/// activewindowv2>>55593885e2e0
/// ```
///
/// Drawing nothing when nothing has focus is right —
/// but obeyed literally it hides the ring for a frame on *every* switch, which
/// is the flicker M2 is supposed to rule out. So an empty event only counts
/// once it has survived this long without a window arriving.
///
/// Long enough to cover the gap at any refresh rate, short enough that clicking
/// the desktop still reads as immediate.
pub const UNFOCUS_GRACE: f64 = 0.10;

/// How long after the last move or resize event a window counts as at rest.
///
/// The ring hides while a window is being dragged and
/// restore it when the window comes to rest. Hyprland has no event for the
/// second half — there is no "drag finished" — so `Event::Settled` has no
/// producer and never will. Rest is the *absence* of movement, which can only
/// be measured by a clock.
pub const SETTLE_DELAY: f64 = 0.20;

/// Holds what the compositor last said, and decides what that means now.
#[derive(Debug)]
pub struct Tracker {
    focused: Option<FocusState>,
    /// When an empty focus event arrived, if we have not yet decided whether it
    /// was real or the transient one.
    unfocus_pending_since: Option<f64>,
    /// When movement was last seen. Each event pushes it forward, so the ring
    /// stays down for the whole drag rather than flickering through it.
    moving_since: Option<f64>,
    pub unfocus_grace: f64,
    pub settle_delay: f64,
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker {
            focused: None,
            unfocus_pending_since: None,
            moving_since: None,
            unfocus_grace: UNFOCUS_GRACE,
            settle_delay: SETTLE_DELAY,
        }
    }
}

impl Tracker {
    /// A window has focus, with geometry already resolved.
    ///
    /// Returns whether this warrants a flare: true only on a change to a
    /// *different* window. A window that merely moved must not flare, or
    /// dragging keeps it lit and it never settles.
    pub fn focus(&mut self, state: FocusState, _now: f64) -> bool {
        let flare = state.is_different_window(self.focused.as_ref());
        // Whatever the compositor was saying a moment ago is now overtaken.
        self.unfocus_pending_since = None;
        if flare {
            // A different window cannot be the one that was being dragged.
            self.moving_since = None;
        }
        self.focused = Some(state);
        flare
    }

    /// An empty focus event arrived. Start the grace period rather than acting
    /// on it: it is far more often the transient one than a real unfocus.
    pub fn note_unfocus(&mut self, now: f64) {
        if self.unfocus_pending_since.is_none() {
            self.unfocus_pending_since = Some(now);
        }
    }

    /// A window is being moved or resized.
    pub fn note_moving(&mut self, now: f64) {
        self.moving_since = Some(now);
    }

    /// The last thing the compositor said had focus, whether or not the ring is
    /// currently drawn around it.
    pub fn focused(&self) -> Option<&FocusState> {
        self.focused.as_ref()
    }

    /// True once an empty focus event has outlived the grace period, meaning
    /// focus really has left every window.
    pub fn unfocus_confirmed(&self, now: f64) -> bool {
        match self.unfocus_pending_since {
            Some(since) => now - since >= self.unfocus_grace,
            None => false,
        }
    }

    /// True while a window is still being dragged or resized.
    pub fn is_moving(&self, now: f64) -> bool {
        match self.moving_since {
            Some(since) => now - since < self.settle_delay,
            None => false,
        }
    }

    /// **The question the renderer asks.** What to draw a ring around this
    /// frame, or `None` for no ring at all.
    pub fn visible(&self, now: f64) -> Option<&FocusState> {
        if self.unfocus_confirmed(now) || self.is_moving(now) {
            return None;
        }
        self.focused.as_ref()
    }

    /// Drop the remembered window once an unfocus is confirmed, so the next
    /// window to take focus flares as the different window it is.
    ///
    /// Call this from the frame loop after `visible`. Kept separate because
    /// `visible` takes `&self`: deciding and forgetting are different jobs.
    pub fn retire_confirmed_unfocus(&mut self, now: f64) {
        if self.unfocus_confirmed(now) {
            self.focused = None;
            self.unfocus_pending_since = None;
        }
    }

    /// Whether the frame loop still needs to be woken to resolve a pending
    /// decision, even though nothing has changed on screen.
    ///
    /// Without this the ring would stay up until some unrelated event happened
    /// to arrive, because both decisions are made by a clock rather than by an
    /// event.
    pub fn has_pending_deadline(&self, now: f64) -> bool {
        (self.unfocus_pending_since.is_some() && !self.unfocus_confirmed(now))
            || self.is_moving(now)
    }
}
