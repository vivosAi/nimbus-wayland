//! Where the focused window is, and on which output.
//!
//! SPEC.md §5's table claims the compositor's event carries position and size.
//! It does not — §7 concedes as much — so geometry is a separate question asked
//! over Hyprland's *writable* socket.
//!
//! That socket rather than `hyprctl`: a live 0.56.2 emits `fullscreen` events
//! around ordinary focus changes, and each one asks for a re-read. Forking a
//! process per re-read would put a process spawn on the focus path for nothing.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use super::{normalize_address, FocusState, Rect};

/// Path to the request socket, or `None` if we are not running under Hyprland.
///
/// Note the name: `.socket.sock` is the one you write requests to, and
/// `.socket2.sock` is the one that streams events. They are different sockets
/// and the two-character difference is easy to miss.
pub fn request_socket_path() -> Option<PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    Some(
        std::path::Path::new(&runtime)
            .join("hypr")
            .join(signature)
            .join(".socket.sock"),
    )
}

/// Ask Hyprland something and read the whole reply. `j/` asks for JSON.
pub fn request(command: &str) -> std::io::Result<String> {
    let path = request_socket_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not running under Hyprland: HYPRLAND_INSTANCE_SIGNATURE is unset",
        )
    })?;
    let mut stream = UnixStream::connect(path)?;
    stream.write_all(command.as_bytes())?;
    stream.flush()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

/// An output, as the compositor sees it.
///
/// `id` exists only to translate `clients[].monitor`, which is an integer and
/// not the name everything else uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub position: (i32, i32),
    pub size: (i32, i32),
    pub scale: f32,
}

/// Parse `j/monitors`.
pub fn parse_monitors(json: &str) -> Result<Vec<Monitor>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("monitors: {e}"))?;
    let array = value.as_array().ok_or("monitors: expected an array")?;
    let mut out = Vec::with_capacity(array.len());
    for m in array {
        out.push(Monitor {
            id: m["id"].as_i64().ok_or("monitors: no id")?,
            name: m["name"].as_str().ok_or("monitors: no name")?.to_string(),
            position: (
                m["x"].as_i64().unwrap_or(0) as i32,
                m["y"].as_i64().unwrap_or(0) as i32,
            ),
            size: (
                m["width"].as_i64().unwrap_or(0) as i32,
                m["height"].as_i64().unwrap_or(0) as i32,
            ),
            scale: m["scale"].as_f64().unwrap_or(1.0) as f32,
        });
    }
    Ok(out)
}

/// Find one window in `j/clients` and turn it into a [`FocusState`].
///
/// `address` may be in either of Hyprland's two spellings; both sides are
/// normalized before comparison. See [`normalize_address`].
pub fn find_client(
    json: &str,
    address: &str,
    monitors: &[Monitor],
) -> Result<Option<FocusState>, String> {
    let wanted = normalize_address(address);
    if wanted.is_empty() {
        return Ok(None);
    }

    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("clients: {e}"))?;
    let array = value.as_array().ok_or("clients: expected an array")?;

    for c in array {
        let Some(raw) = c["address"].as_str() else { continue };
        if normalize_address(raw) != wanted {
            continue;
        }

        // A window that is mapped but not yet given a size reports a zero rect.
        // Treat it as not found rather than drawing a ring around nothing; the
        // next event will carry a real one.
        let at = &c["at"];
        let size = &c["size"];
        let rect = Rect {
            x: at[0].as_i64().unwrap_or(0) as i32,
            y: at[1].as_i64().unwrap_or(0) as i32,
            w: size[0].as_i64().unwrap_or(0) as i32,
            h: size[1].as_i64().unwrap_or(0) as i32,
        };
        if !rect.is_sensible() {
            return Ok(None);
        }

        // `monitor` is an integer id. Every other part of the program, and
        // Wayland itself, identifies an output by name.
        let monitor_id = c["monitor"].as_i64().unwrap_or(-1);
        let output = monitors
            .iter()
            .find(|m| m.id == monitor_id)
            .map(|m| m.name.clone())
            .unwrap_or_default();

        // `fullscreen` is a mode, not a flag: 0 none, 1 maximized, 2 fullscreen.
        let fullscreen = c["fullscreen"].as_i64().unwrap_or(0) != 0;

        let class = c["class"].as_str().unwrap_or_default().to_string();
        return Ok(Some(FocusState { address: wanted, output, class, rect, fullscreen }));
    }
    Ok(None)
}

/// Read the current monitor list from the compositor.
pub fn monitors() -> Result<Vec<Monitor>, String> {
    let json = request("j/monitors").map_err(|e| format!("j/monitors: {e}"))?;
    parse_monitors(&json)
}

/// Read one window's geometry from the compositor.
pub fn locate(address: &str, monitors: &[Monitor]) -> Result<Option<FocusState>, String> {
    let json = request("j/clients").map_err(|e| format!("j/clients: {e}"))?;
    find_client(&json, address, monitors)
}
