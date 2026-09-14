//! Hyprland's event socket.
//!
//! Events arrive newline-delimited on
//! `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket2.sock`
//! in the form `EVENT>>DATA`.
//!
//! Parsing is separated from reading so it can be tested without a compositor,
//! which is most of what is worth testing here.

use super::{normalize_address, Event};

/// Parse one line. Returns `None` for events we do not act on, which is most
/// of them.
///
/// Event names move between Hyprland releases. If focus tracking stops working
/// after an update, this function is the first place to look. Verified against
/// a live socket on 0.56.2.
///
/// Every address is passed through [`normalize_address`], because the socket
/// and `hyprctl -j clients` disagree about the `0x` prefix and geometry has to
/// be joined across that gap.
pub fn parse_line(line: &str) -> Option<Event> {
    let (name, data) = line.split_once(">>")?;
    // The first comma-separated field, verbatim. Output names are case
    // sensitive — `DP-3` must survive as `DP-3`, because it is matched against
    // what the Wayland output advertises.
    let first = |s: &str| s.split(',').next().unwrap_or("").trim().to_string();
    // The same field when it is a window handle, which needs the 0x prefix and
    // case flattened out. Only ever applied to addresses.
    let first_address = |s: &str| normalize_address(s.split(',').next().unwrap_or(""));

    match name {
        // v2 carries the address; the v1 form carries class and title instead.
        //
        // An empty payload here is *usually not* a real unfocus: Hyprland emits
        // one transiently in the middle of every focus change. It is handed on
        // as-is and debounced in `tracker`, which owns that decision.
        "activewindowv2" => {
            let addr = normalize_address(data);
            if addr.is_empty() {
                Some(Event::Unfocused)
            } else {
                Some(Event::Focused(addr))
            }
        }
        // Emitted with an empty payload when focus leaves every window.
        "activewindow" if data.trim().is_empty() || data.trim() == "," => Some(Event::Unfocused),
        "closewindow" => Some(Event::Rescan),
        "movewindowv2" | "movewindow" => Some(Event::Moving(first_address(data))),
        "resizewindow" => Some(Event::Moving(first_address(data))),
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

/// Read the event socket forever, handing every parsed event to `sink`.
///
/// Runs on its own thread: the Wayland event loop owns the main thread, and
/// blocking on a compositor socket there would stall rendering. The sink is
/// expected to wake the Wayland loop.
///
/// Returns only on error or EOF, which means Hyprland has gone away — at which
/// point there is no desktop left to draw on.
#[cfg(target_os = "linux")]
pub fn listen<F: FnMut(Event)>(mut sink: F) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    let path = socket_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not running under Hyprland: HYPRLAND_INSTANCE_SIGNATURE is unset",
        )
    })?;
    let stream = UnixStream::connect(path)?;
    let reader = BufReader::new(stream);

    for line in reader.lines() {
        let line = line?;
        // Hyprland emits plenty we do not act on. Unknown names are not an
        // error: event names move between releases, and an overlay that exits
        // because the compositor said something new is worse than one that
        // ignores it.
        if let Some(event) = parse_line(&line) {
            sink(event);
        }
    }
    Ok(())
}
