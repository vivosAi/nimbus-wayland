//! Hyprland's event socket.
//!
//! Events arrive newline-delimited on
//! `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket2.sock`
//! in the form `EVENT>>DATA`.
//!
//! Parsing is separated from reading so it can be tested without a compositor,
//! which is most of what is worth testing here.

use super::Event;

/// Parse one line. Returns `None` for events we do not act on, which is most
/// of them.
///
/// Event names move between Hyprland releases. If focus tracking stops working
/// after an update, this function is the first place to look.
pub fn parse_line(line: &str) -> Option<Event> {
    let (name, data) = line.split_once(">>")?;
    let first = |s: &str| s.split(',').next().unwrap_or("").to_string();

    match name {
        // v2 carries the address; the v1 form carries class and title instead.
        "activewindowv2" => {
            let addr = data.trim();
            if addr.is_empty() {
                Some(Event::Unfocused)
            } else {
                Some(Event::Focused(addr.to_string()))
            }
        }
        // Emitted with an empty payload when focus leaves every window.
        "activewindow" if data.trim().is_empty() || data.trim() == "," => Some(Event::Unfocused),
        "closewindow" => Some(Event::Rescan),
        "movewindowv2" | "movewindow" => Some(Event::Moving(first(data))),
        "resizewindow" => Some(Event::Moving(first(data))),
        "monitoradded" | "monitoraddedv2" => Some(Event::OutputAdded(first(data))),
        "monitorremoved" => Some(Event::OutputRemoved(first(data))),
        "workspace" | "workspacev2" | "focusedmon" | "fullscreen" | "changefloatingmode" => {
            Some(Event::Rescan)
        }
        _ => None,
    }
}

/// Path to the event socket, or `None` if we are not running under Hyprland.
pub fn socket_path() -> Option<std::path::PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    Some(
        std::path::Path::new(&runtime)
            .join("hypr")
            .join(signature)
            .join(".socket2.sock"),
    )
}
