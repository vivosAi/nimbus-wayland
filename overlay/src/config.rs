//! Settings, and the rotation state that outlives a restart.
//!
//! A port of `Preferences.swift`, which is where the macOS app keeps every
//! setting its menu exposes. The values are the macOS defaults, not fresh
//! inventions: the point of this project is that the two look and behave the
//! same, so a default that differs is a bug rather than a choice.
//!
//! Two files, deliberately separate:
//!
//! * **Config** is written by the user and only read by us. Nothing here
//!   rewrites it, so a hand-edited file with comments and ordering intact stays
//!   that way.
//! * **State** is written by us and is nobody's business to edit. It exists so
//!   that restarting resumes the palette rotation instead of resetting it, as
//!   the macOS version does by persisting the same three values.
//!
//! Platform-independent on purpose, so it is testable without a compositor.

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Enumerated settings
// ---------------------------------------------------------------------------

/// How far the band reaches either side of the window edge, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BandWidth {
    Thin,
    #[default]
    Normal,
    Thick,
}

impl BandWidth {
    /// `(inside the window edge, outside it)`.
    pub fn points(self) -> (f32, f32) {
        match self {
            BandWidth::Thin => (4.0, 12.0),
            BandWidth::Normal => (6.0, 18.0),
            BandWidth::Thick => (9.0, 28.0),
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "thin" => Some(BandWidth::Thin),
            "normal" => Some(BandWidth::Normal),
            "thick" => Some(BandWidth::Thick),
            _ => None,
        }
    }
}

/// How fast the light travels around the ring.
///
/// Every value is far below any rate that could read as flicker. That is a
/// hard floor for photosensitivity, not a matter of taste, so "faster" is not
/// an option offered beyond `Lively`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MotionSpeed {
    Calm,
    #[default]
    Normal,
    Lively,
}

impl MotionSpeed {
    pub fn flow_speed(self) -> f64 {
        match self {
            MotionSpeed::Calm => 0.22,
            MotionSpeed::Normal => 0.45,
            MotionSpeed::Lively => 0.85,
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "calm" => Some(MotionSpeed::Calm),
            "normal" => Some(MotionSpeed::Normal),
            "lively" => Some(MotionSpeed::Lively),
            _ => None,
        }
    }
}

/// How many distinct features there are around the ring: low values give broad
/// slow swells, high values fine churn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Turbulence {
    Smooth,
    #[default]
    Normal,
    Churny,
}

impl Turbulence {
    pub fn noise_scale(self) -> f32 {
        match self {
            Turbulence::Smooth => 2.5,
            Turbulence::Normal => 4.0,
            Turbulence::Churny => 6.5,
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "smooth" => Some(Turbulence::Smooth),
            "normal" => Some(Turbulence::Normal),
            "churny" => Some(Turbulence::Churny),
            _ => None,
        }
    }
}

/// What happens after `idle_threshold` seconds with no input.
///
/// The default is `Freeze`, not `FadeOut`. Pausing the render loop leaves the
/// last frame on screen, so the ring still marks the focused window when you
/// walk back to the machine and look at it before touching anything — which is
/// the case this program exists for. The GPU is idle either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IdleBehavior {
    /// Never stop. Costs the most power.
    AlwaysAnimate,
    /// Stop animating, keep the ring visible.
    #[default]
    Freeze,
    /// Stop animating and hide the ring entirely.
    FadeOut,
}

impl IdleBehavior {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "always_animate" | "alwaysAnimate" => Some(IdleBehavior::AlwaysAnimate),
            "freeze" => Some(IdleBehavior::Freeze),
            "fade_out" | "fadeOut" => Some(IdleBehavior::FadeOut),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------

/// Everything the menu on macOS exposes, with the same defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub enabled: bool,
    /// Baseline brightness once the flare has settled. The menu's
    /// Subtle/Normal/Loud are 0.18 / 0.30 / 0.50.
    pub idle_intensity: f32,
    /// Seconds for the focus-change flare to relax to `idle_intensity`.
    pub flare_duration: f32,
    pub band_width: BandWidth,
    pub motion_speed: MotionSpeed,
    pub turbulence: Turbulence,
    /// Frames per second while animating. The macOS default is 30, not 60:
    /// cost is very nearly linear in frame rate and almost independent of what
    /// the shader does, so this is the one setting that decides what the
    /// program costs.
    pub frame_rate: u32,
    /// Seconds between palette changes; 0 means never.
    pub palette_interval: f64,
    /// Palettes the rotation may not choose, by name.
    pub disabled_palettes: Vec<String>,
    /// Window classes that never get a ring. On macOS these are bundle IDs.
    pub exclusions: Vec<String>,
    pub hide_in_fullscreen: bool,
    pub hide_while_dragging: bool,
    pub idle_behavior: IdleBehavior,
    /// Seconds of no input before `idle_behavior` applies. 0 means never go
    /// idle: the ring is wanted at all times, and a sleeping display stops
    /// rendering anyway because nobody can see it.
    pub idle_threshold: f64,
    /// Fire a full flare on the first input after an idle stretch. Returning to
    /// the machine is exactly when you are most likely to type into the wrong
    /// window.
    pub flare_on_return: bool,
    /// Renderer diagnostic; see `Uniforms::debug_mode`.
    pub debug_mode: u8,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            idle_intensity: 0.30,
            flare_duration: 2.5,
            band_width: BandWidth::Normal,
            motion_speed: MotionSpeed::Normal,
            turbulence: Turbulence::Normal,
            frame_rate: 30,
            palette_interval: 1800.0,
            disabled_palettes: Vec::new(),
            exclusions: Vec::new(),
            hide_in_fullscreen: true,
            hide_while_dragging: true,
            idle_behavior: IdleBehavior::Freeze,
            idle_threshold: 0.0,
            flare_on_return: true,
            debug_mode: 0,
        }
    }
}

impl Config {
    /// Corner radius in logical pixels, matching `RingRenderer.cornerRadiusPoints`.
    pub const CORNER_RADIUS: f32 = 11.0;

    /// Pixel falloff of the outer bloom.
    ///
    /// Derived from the band's outer width rather than set independently, so a
    /// wider ring gets a proportionally wider bloom and the two never drift out
    /// of proportion. The macOS renderer computes exactly this.
    pub fn glow_falloff(&self) -> f32 {
        let (_, outer) = self.band_width.points();
        (outer * 0.6).max(1.0)
    }

    /// Parse whatever of a config file we understand.
    ///
    /// Unknown keys are ignored and malformed values fall back to the default
    /// for that key alone. A typo in one setting must not cost the user every
    /// other setting, and must never stop the ring appearing: an overlay that
    /// refuses to start because a number was spelled wrong is worse than one
    /// that starts with a default.
    pub fn from_json(text: &str) -> (Self, Vec<String>) {
        let mut config = Config::default();
        let mut warnings = Vec::new();

        let value: serde_json::Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                warnings.push(format!("config is not valid JSON ({e}); using defaults"));
                return (config, warnings);
            }
        };
        let Some(map) = value.as_object() else {
            warnings.push("config must be a JSON object; using defaults".into());
            return (config, warnings);
        };

        let warn_bad = |key: &str, expected: &str, warnings: &mut Vec<String>| {
            warnings.push(format!("{key}: expected {expected}; keeping the default"));
        };

        for (key, v) in map {
            match key.as_str() {
                "enabled" => match v.as_bool() {
                    Some(b) => config.enabled = b,
                    None => warn_bad(key, "true or false", &mut warnings),
                },
                "idle_intensity" => match v.as_f64() {
                    // Above 1.0 the ring is simply saturated, which is not a
                    // crash but is never what anyone meant.
                    Some(f) if (0.0..=1.0).contains(&f) => config.idle_intensity = f as f32,
                    _ => warn_bad(key, "a number from 0.0 to 1.0", &mut warnings),
                },
                "flare_duration" => match v.as_f64() {
                    Some(f) if f >= 0.0 => config.flare_duration = f as f32,
                    _ => warn_bad(key, "a number of seconds, 0 or more", &mut warnings),
                },
                "band_width" => match v.as_str().and_then(BandWidth::parse) {
                    Some(b) => config.band_width = b,
                    None => warn_bad(key, "thin, normal or thick", &mut warnings),
                },
                "motion_speed" => match v.as_str().and_then(MotionSpeed::parse) {
                    Some(m) => config.motion_speed = m,
                    None => warn_bad(key, "calm, normal or lively", &mut warnings),
                },
                "turbulence" => match v.as_str().and_then(Turbulence::parse) {
                    Some(t) => config.turbulence = t,
                    None => warn_bad(key, "smooth, normal or churny", &mut warnings),
                },
                "frame_rate" => match v.as_u64() {
                    // 1 fps is a legitimate choice on a very slow machine; 240
                    // is past any display this runs on.
                    Some(n) if (1..=240).contains(&n) => config.frame_rate = n as u32,
                    _ => warn_bad(key, "a whole number from 1 to 240", &mut warnings),
                },
                "palette_interval" => match v.as_f64() {
                    Some(f) if f >= 0.0 => config.palette_interval = f,
                    _ => warn_bad(key, "seconds, or 0 to never rotate", &mut warnings),
                },
                "disabled_palettes" => match string_list(v) {
                    Some(l) => config.disabled_palettes = l,
                    None => warn_bad(key, "a list of palette names", &mut warnings),
                },
                "exclusions" => match string_list(v) {
                    Some(l) => config.exclusions = l,
                    None => warn_bad(key, "a list of window classes", &mut warnings),
                },
                "hide_in_fullscreen" => match v.as_bool() {
                    Some(b) => config.hide_in_fullscreen = b,
                    None => warn_bad(key, "true or false", &mut warnings),
                },
                "hide_while_dragging" => match v.as_bool() {
                    Some(b) => config.hide_while_dragging = b,
                    None => warn_bad(key, "true or false", &mut warnings),
                },
                "idle_behavior" => match v.as_str().and_then(IdleBehavior::parse) {
                    Some(i) => config.idle_behavior = i,
                    None => warn_bad(
                        key,
                        "always_animate, freeze or fade_out",
                        &mut warnings,
                    ),
                },
                "idle_threshold" => match v.as_f64() {
                    Some(f) if f >= 0.0 => config.idle_threshold = f,
                    _ => warn_bad(key, "seconds, or 0 to never go idle", &mut warnings),
                },
                "flare_on_return" => match v.as_bool() {
                    Some(b) => config.flare_on_return = b,
                    None => warn_bad(key, "true or false", &mut warnings),
                },
                "debug_mode" => match v.as_u64() {
                    Some(n) if n <= 2 => config.debug_mode = n as u8,
                    _ => warn_bad(key, "0, 1 or 2", &mut warnings),
                },
                // Forward compatibility: a config written by a newer version
                // must not make an older one shout on every line it adds.
                _ => {}
            }
        }
        (config, warnings)
    }

    /// Read the config file, or the defaults if there is not one.
    pub fn load() -> (Self, Vec<String>) {
        let Some(path) = config_path() else {
            return (Config::default(), Vec::new());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let (config, mut warnings) = Config::from_json(&text);
                for w in &mut warnings {
                    *w = format!("{}: {w}", path.display());
                }
                (config, warnings)
            }
            // No file is the normal case, not an error.
            Err(_) => (Config::default(), Vec::new()),
        }
    }
}

fn string_list(v: &serde_json::Value) -> Option<Vec<String>> {
    let array = v.as_array()?;
    array
        .iter()
        .map(|i| i.as_str().map(str::to_string))
        .collect()
}

// ---------------------------------------------------------------------------

/// What the program remembers between runs.
///
/// Only the palette rotation, and only so that restarting does not reset it.
/// Without this, anyone who restarts often would see the first palette far more
/// than any other, which defeats the rotation's entire purpose.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub palette_index: usize,
    pub palette_recent: Vec<usize>,
    /// Unix seconds when the palette last changed.
    pub palette_changed_at: f64,
}

impl State {
    pub fn from_json(text: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return State::default();
        };
        State {
            palette_index: value["palette_index"].as_u64().unwrap_or(0) as usize,
            palette_recent: value["palette_recent"]
                .as_array()
                .map(|a| a.iter().filter_map(|i| i.as_u64()).map(|i| i as usize).collect())
                .unwrap_or_default(),
            palette_changed_at: value["palette_changed_at"].as_f64().unwrap_or(0.0),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::json!({
            "palette_index": self.palette_index,
            "palette_recent": self.palette_recent,
            "palette_changed_at": self.palette_changed_at,
        })
        .to_string()
    }

    pub fn load() -> Self {
        state_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| State::from_json(&t))
            .unwrap_or_default()
    }

    /// Best effort. Losing the rotation position costs nothing worth reporting,
    /// and a read-only state directory must not stop the ring being drawn.
    pub fn save(&self) {
        let Some(path) = state_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, self.to_json());
    }
}

// ---------------------------------------------------------------------------

fn xdg_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)),
    }
}

pub fn config_path() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config").map(|d| d.join("nimbus").join("config.json"))
}

pub fn state_path() -> Option<PathBuf> {
    xdg_dir("XDG_STATE_HOME", ".local/state").map(|d| d.join("nimbus").join("state.json"))
}
