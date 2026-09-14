//! What has keyboard focus, and where it is.

pub mod geometry;
pub mod hyprland;
pub mod tracker;

/// Canonical form of a compositor window handle.
///
/// Hyprland reports the same window two different ways, which is not documented
/// anywhere and was found by watching a live socket on 0.56.2:
///
/// ```text
/// activewindowv2>>55593885e2e0          the event socket, bare hex
/// "address": "0x55593885e2e0"           hyprctl -j clients, 0x-prefixed
/// ```
///
/// Geometry has to be joined from `clients` onto an address that arrived from
/// the socket, so comparing the two raw strings never matches — and it fails
/// silently, as a ring that simply never appears. Normalize on the way in and
/// the question cannot come up again.
pub fn normalize_address(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed)
        .to_ascii_lowercase()
}

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
    /// The window's class, which is what an exclusion names. On macOS this is
    /// the bundle ID; the role is the same — an escape hatch for applications
    /// that misreport their geometry, rather than special cases in code.
    pub class: String,
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
    ///
    /// There is deliberately no `Settled` counterpart. Hyprland has no "drag
    /// finished" event, so rest is the absence of movement and only a clock can
    /// measure it — see [`tracker::SETTLE_DELAY`].
    Moving(String),
    OutputAdded(String),
    OutputRemoved(String),
    /// Workspace or monitor focus changed, so geometry should be re-read.
    Rescan,
}
