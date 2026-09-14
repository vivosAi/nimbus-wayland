//! What has keyboard focus, and where it is.

pub mod hyprland;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// Frames this absurd are always a compositor or client misreporting,
    /// never a real window.
    pub fn is_sensible(&self) -> bool {
        self.w >= 40 && self.h >= 40 && self.w <= 20_000 && self.h <= 20_000
    }
}

/// A snapshot of what currently has keyboard focus.
#[derive(Clone, Debug, PartialEq)]
pub struct FocusState {
    /// The compositor's handle for the window. Identity, not geometry: this is
    /// what decides whether a change is a *different* window, and so whether to
    /// flare.
    pub address: String,
    /// Which output it is on. Decides which surface draws.
    pub output: String,
    pub rect: Rect,
    pub fullscreen: bool,
}

impl FocusState {
    /// A focus change worth flaring for, as opposed to the same window moving.
    ///
    /// Unlike the macOS version, which had no reliable window identity and had
    /// to infer this from geometry, the compositor gives us an address.
    pub fn is_different_window(&self, other: Option<&FocusState>) -> bool {
        match other {
            None => true,
            Some(prev) => prev.address != self.address,
        }
    }
}

/// What the compositor tells us. Kept minimal: anything not acted on is noise.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Focus moved to this window address. Geometry is fetched separately,
    /// because Hyprland's focus event does not carry it.
    Focused(String),
    /// Nothing has focus. The desktop, for instance. Draw nothing: showing the
    /// ring on the previously focused window would be a lie, since that window
    /// will not receive the next keystroke.
    Unfocused,
    /// A window is being moved or resized. Stand down until it comes to rest.
    Moving(String),
    /// The focused window finished moving.
    Settled,
    OutputAdded(String),
    OutputRemoved(String),
    /// Workspace or monitor focus changed, so geometry should be re-read.
    Rescan,
}
