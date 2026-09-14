//! The control socket: how anything outside the process changes a setting.
//!
//! The macOS app puts every setting behind a menu bar item. There is no menu
//! bar here, so the equivalent is a socket plus a handful of CLI subcommands,
//! and a bar widget drives them the way the Mac menu drives `UserDefaults`.
//!
//! Line-delimited JSON in both directions. That is not an elegant protocol, but
//! it is one a shell script, a QML `Process`, or `socat` can all speak without
//! a library, which matters far more for something whose entire job is to be
//! driven by other programs.
//!
//! The wire format lives here, away from the socket and the event loop, so it
//! can be tested without either.

use std::path::PathBuf;

use crate::config::Config;

/// What the caller wants.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Everything the panel needs to draw itself in one round trip.
    Status,
    /// Change settings, apply them live, and persist them.
    Set(serde_json::Map<String, serde_json::Value>),
    /// Rotate the palette now, rather than waiting for the timer.
    NextColor,
    /// Switch to a named palette, with the usual cross-fade.
    SetPalette(String),
    /// Re-read the config file from disk.
    Reload,
    Quit,
}

impl Request {
    pub fn parse(line: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("not valid JSON: {e}"))?;
        let command = value["command"]
            .as_str()
            .ok_or("no \"command\" field")?
            .to_string();

        match command.as_str() {
            "status" => Ok(Request::Status),
            "next_color" => Ok(Request::NextColor),
            "set_palette" => {
                let name = value["name"].as_str().ok_or("set_palette needs a \"name\"")?;
                if name.is_empty() {
                    return Err("set_palette with an empty name".into());
                }
                Ok(Request::SetPalette(name.to_string()))
            }
            "reload" => Ok(Request::Reload),
            "quit" => Ok(Request::Quit),
            "set" => {
                let settings = value
                    .get("settings")
                    .and_then(|s| s.as_object())
                    .ok_or("set needs a \"settings\" object")?
                    .clone();
                if settings.is_empty() {
                    return Err("set with no settings".into());
                }
                Ok(Request::Set(settings))
            }
            other => Err(format!("unknown command {other:?}")),
        }
    }

    pub fn to_json(&self) -> String {
        match self {
            Request::Status => r#"{"command":"status"}"#.into(),
            Request::NextColor => r#"{"command":"next_color"}"#.into(),
            Request::Reload => r#"{"command":"reload"}"#.into(),
            Request::Quit => r#"{"command":"quit"}"#.into(),
            Request::SetPalette(name) => {
                serde_json::json!({ "command": "set_palette", "name": name }).to_string()
            }
            Request::Set(map) => serde_json::json!({
                "command": "set",
                "settings": serde_json::Value::Object(map.clone()),
            })
            .to_string(),
        }
    }
}

/// Turn `set key value` from a command line into the typed value the config
/// expects.
///
/// A CLI hands us strings and nothing else, so `"true"` has to become a
/// boolean and `"30"` a number, or every setting would land as a string and be
/// rejected. Which one each key wants is knowable from the key alone.
pub fn coerce_setting(key: &str, raw: &str) -> Result<serde_json::Value, String> {
    match key {
        "enabled" | "hide_in_fullscreen" | "hide_while_dragging" | "flare_on_return" => {
            match raw {
                "true" | "on" | "yes" | "1" => Ok(serde_json::Value::Bool(true)),
                "false" | "off" | "no" | "0" => Ok(serde_json::Value::Bool(false)),
                _ => Err(format!("{key} takes true or false, not {raw:?}")),
            }
        }
        "band_width" | "motion_speed" | "turbulence" | "idle_behavior" => {
            Ok(serde_json::Value::String(raw.to_string()))
        }
        "frame_rate" | "debug_mode" => raw
            .parse::<u64>()
            .map(|n| serde_json::json!(n))
            .map_err(|_| format!("{key} takes a whole number, not {raw:?}")),
        "idle_intensity" | "flare_duration" | "palette_interval" | "idle_threshold" => raw
            .parse::<f64>()
            .map(|f| serde_json::json!(f))
            .map_err(|_| format!("{key} takes a number, not {raw:?}")),
        "exclusions" | "disabled_palettes" => Ok(serde_json::Value::Array(
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| serde_json::Value::String(s.to_string()))
                .collect(),
        )),
        other => Err(format!("unknown setting {other:?}")),
    }
}

/// Merge changed settings into the config file, leaving everything else exactly
/// as the user left it.
///
/// Read-modify-write rather than serializing the whole `Config`: the file may
/// carry keys we do not know, and it is hand-edited. Rewriting it wholesale
/// would silently delete a future version's settings and reflow the user's
/// formatting. `serde_json`'s `preserve_order` keeps their key order intact.
pub fn merge_into_config_file(
    path: &PathBuf,
    settings: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let mut document = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default(),
        Err(_) => serde_json::Map::new(),
    };

    for (key, value) in settings {
        document.insert(key.clone(), value.clone());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(document))
        .map_err(|e| e.to_string())?;

    // Write beside the target and rename, so an interrupted write cannot leave
    // a half-written config that fails to parse on next start.
    let temporary = path.with_extension("json.new");
    std::fs::write(&temporary, text + "\n").map_err(|e| format!("{}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// The whole state of the program, as one JSON object.
///
/// Everything the panel needs in one round trip: it redraws from this, so a
/// second call to learn the palette names would mean the two could disagree.
/// One palette, as the panel needs to draw it.
pub struct PaletteView {
    pub name: &'static str,
    /// `#rrggbb` for the two band colours and the glow.
    pub hex: [String; 3],
    /// False when the user has taken it out of the rotation.
    pub in_rotation: bool,
}

pub fn status_json(
    config: &Config,
    palettes: &[PaletteView],
    current_palette: usize,
    showing: bool,
) -> String {
    serde_json::json!({
        "ok": true,
        "running": true,
        // Whether a ring is on screen this instant, as opposed to whether the
        // program is switched on. The panel shows both.
        "showing": showing,
        "palettes": palettes.iter().map(|p| serde_json::json!({
            "name": p.name,
            "a": p.hex[0],
            "b": p.hex[1],
            "glow": p.hex[2],
            "in_rotation": p.in_rotation,
        })).collect::<Vec<_>>(),
        "palette_index": current_palette,
        "palette_name": palettes.get(current_palette).map(|p| p.name).unwrap_or(""),
        "settings": {
            "enabled": config.enabled,
            "idle_intensity": tidy(config.idle_intensity),
            "flare_duration": tidy(config.flare_duration),
            "band_width": match config.band_width {
                crate::config::BandWidth::Thin => "thin",
                crate::config::BandWidth::Normal => "normal",
                crate::config::BandWidth::Thick => "thick",
            },
            "motion_speed": match config.motion_speed {
                crate::config::MotionSpeed::Calm => "calm",
                crate::config::MotionSpeed::Normal => "normal",
                crate::config::MotionSpeed::Lively => "lively",
            },
            "turbulence": match config.turbulence {
                crate::config::Turbulence::Smooth => "smooth",
                crate::config::Turbulence::Normal => "normal",
                crate::config::Turbulence::Churny => "churny",
            },
            "frame_rate": config.frame_rate,
            "palette_interval": config.palette_interval,
            "exclusions": config.exclusions,
            "disabled_palettes": config.disabled_palettes,
            "hide_in_fullscreen": config.hide_in_fullscreen,
            "hide_while_dragging": config.hide_while_dragging,
            "idle_behavior": match config.idle_behavior {
                crate::config::IdleBehavior::AlwaysAnimate => "always_animate",
                crate::config::IdleBehavior::Freeze => "freeze",
                crate::config::IdleBehavior::FadeOut => "fade_out",
            },
            "idle_threshold": config.idle_threshold,
            "flare_on_return": config.flare_on_return,
            "debug_mode": config.debug_mode,
        }
    })
    .to_string()
}

/// `f32` widened to `f64` produces things like `0.30000001192092896`, which is
/// the same number but reads as a bug to anyone looking at the JSON and is
/// awkward for a panel to display. Round to the precision these settings
/// actually have.
fn tidy(value: f32) -> f64 {
    (value as f64 * 1000.0).round() / 1000.0
}

pub fn error_json(message: &str) -> String {
    serde_json::json!({ "ok": false, "error": message }).to_string()
}

/// Where the socket lives.
///
/// `XDG_RUNTIME_DIR` rather than a config or state directory: the socket is
/// meaningful only while the process is running, and that directory is cleared
/// at logout, so a stale socket cannot outlive the session that made it.
pub fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| PathBuf::from(d).join("nimbus").join("control.sock"))
}
