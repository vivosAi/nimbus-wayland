//! Palettes, the flare curve, and rotation.
//!
//! Deliberately free of platform dependencies so it compiles and tests
//! anywhere. Every bug worth catching in the macOS version lived in code
//! shaped like this, not in the window plumbing.

/// A ring color scheme, stored in **linear RGB**.
///
/// Authored as sRGB hex because that is readable, converted once because every
/// operation done to these values is an interpolation: mixing the two band
/// colors, and cross-fading one palette into the next. Interpolating in gamma
/// space gives muddy mid-tones, which on a two-color ring is very visible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub name: &'static str,
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub glow: [f32; 3],
}

pub const fn srgb_bytes(hex: u32) -> [f32; 3] {
    [
        ((hex >> 16) & 0xFF) as f32 / 255.0,
        ((hex >> 8) & 0xFF) as f32 / 255.0,
        (hex & 0xFF) as f32 / 255.0,
    ]
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear(hex: u32) -> [f32; 3] {
    let s = srgb_bytes(hex);
    [
        srgb_to_linear(s[0]),
        srgb_to_linear(s[1]),
        srgb_to_linear(s[2]),
    ]
}

impl Palette {
    fn new(name: &'static str, a: u32, b: u32, glow: u32) -> Self {
        Palette { name, a: linear(a), b: linear(b), glow: linear(glow) }
    }
}

/// The same ten the macOS app ships with. All high-chroma and bright, because
/// the ring has to win against arbitrary window content.
pub fn palettes() -> Vec<Palette> {
    vec![
        Palette::new("Ember", 0xFF3D00, 0xFFC400, 0xFF6D00),
        Palette::new("Plasma", 0x7C4DFF, 0x00E5FF, 0x536DFE),
        Palette::new("Toxic", 0x76FF03, 0x00E676, 0xB2FF59),
        Palette::new("Magma", 0xD50000, 0xFF6E40, 0xFF1744),
        Palette::new("Ice", 0x18FFFF, 0x82B1FF, 0x40C4FF),
        Palette::new("Neon Rose", 0xFF4081, 0xF50057, 0xFF80AB),
        Palette::new("Solar", 0xFFD600, 0xFFAB00, 0xFFEA00),
        Palette::new("Aurora", 0x00E676, 0x00B0FF, 0x1DE9B6),
        Palette::new("Ultraviolet", 0xE040FB, 0x651FFF, 0xAA00FF),
        Palette::new("Copper", 0xFF9100, 0xFFD180, 0xFF6D00),
    ]
}

// ---------------------------------------------------------------------------

/// The flare-and-settle curve.
///
/// The eye detects change far better than steady state, so a focus change gets
/// a bright pulse that relaxes into a calm baseline. Time is passed in rather
/// than read, so the curve can be tested without waiting for it.
pub struct Animator {
    pub idle_intensity: f32,
    pub flare_duration: f32,
    started_at: Option<f64>,
}

impl Default for Animator {
    fn default() -> Self {
        Animator { idle_intensity: 0.30, flare_duration: 2.5, started_at: None }
    }
}

impl Animator {
    /// Start a pulse. Fire this only on a change to a *different* window, never
    /// on a geometry update, or dragging keeps it lit and it never settles.
    pub fn flare(&mut self, now: f64) {
        self.started_at = Some(now);
    }

    /// 1 at the instant of a flare, decaying to 0. Everything else derives from
    /// this, so brightness, width and speed relax together as one thing.
    pub fn progress(&self, now: f64) -> f32 {
        let Some(start) = self.started_at else { return 0.0 };
        if self.flare_duration <= 0.0 {
            return 0.0;
        }
        let elapsed = (now - start) as f32;
        if elapsed < 0.0 {
            return 1.0;
        }
        if elapsed >= self.flare_duration {
            return 0.0;
        }
        // exp(-3) is about 0.05, so the pulse is 95% spent by the end.
        (-3.0 * elapsed / self.flare_duration).exp()
    }

    pub fn intensity(&self, now: f64) -> f32 {
        self.idle_intensity + (1.0 - self.idle_intensity) * self.progress(now)
    }

    pub fn band_scale(&self, now: f64) -> f32 {
        1.0 + 0.6 * self.progress(now)
    }

    pub fn speed_scale(&self, now: f64) -> f32 {
        1.0 + 0.8 * self.progress(now)
    }

    pub fn is_flaring(&self, now: f64) -> bool {
        self.progress(now) > 0.02
    }
}

// ---------------------------------------------------------------------------

/// Small deterministic generator, so palette selection can be tested. Not
/// cryptographic and does not need to be.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    pub fn next_usize(&mut self, bound: usize) -> usize {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        ((self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as usize) % bound.max(1)
    }
}

/// Rotates the color scheme and cross-fades between schemes.
///
/// Rotation exists to stop the ring becoming invisible through familiarity, so
/// selection avoids repeating recent palettes, and transitions are always faded:
/// a hard cut reads as a glitch rather than as a change.
pub struct Rotator {
    available: Vec<Palette>,
    current: usize,
    previous: Option<[[f32; 3]; 3]>,
    fade_start: Option<f64>,
    recent: Vec<usize>,
    pub fade_duration: f64,
}

/// How many recent palettes may not be reused. With ten to choose from,
/// avoiding the last three keeps the sequence from feeling like it has
/// favourites.
pub const RECENT_MEMORY: usize = 3;

impl Rotator {
    pub fn new(available: Vec<Palette>, start: usize) -> Self {
        let current = if available.is_empty() { 0 } else { start.min(available.len() - 1) };
        Rotator {
            available,
            current,
            previous: None,
            fade_start: None,
            recent: Vec::new(),
            fade_duration: 3.0,
        }
    }

    pub fn current(&self) -> Palette {
        self.available[self.current]
    }

    pub fn current_index(&self) -> usize {
        self.current
    }

    /// Never the current palette, and never one of the last few. Relaxes the
    /// recency rule rather than deadlocking when few are available.
    pub fn pick_next(&self, rng: &mut Rng) -> usize {
        if self.available.len() <= 1 {
            return self.current;
        }
        let mut candidates: Vec<usize> = (0..self.available.len())
            .filter(|i| *i != self.current && !self.recent.contains(i))
            .collect();
        if candidates.is_empty() {
            candidates = (0..self.available.len()).filter(|i| *i != self.current).collect();
        }
        candidates[rng.next_usize(candidates.len())]
    }

    pub fn transition_to(&mut self, index: usize, now: f64) {
        if index == self.current || index >= self.available.len() {
            return;
        }
        // Start the next fade from what is actually on screen, not from a color
        // nobody is looking at.
        self.previous = Some(self.colors(now));
        self.current = index;
        self.fade_start = Some(now);
        self.recent.push(index);
        if self.recent.len() > RECENT_MEMORY {
            let excess = self.recent.len() - RECENT_MEMORY;
            self.recent.drain(0..excess);
        }
    }

    /// 0 while a fade is running, 1 once it has finished.
    pub fn fade_progress(&self, now: f64) -> f64 {
        let Some(start) = self.fade_start else { return 1.0 };
        let elapsed = now - start;
        if elapsed >= self.fade_duration {
            1.0
        } else {
            (elapsed / self.fade_duration).max(0.0)
        }
    }

    /// The colors to upload this frame: a, b, glow, interpolated in linear RGB.
    pub fn colors(&self, now: f64) -> [[f32; 3]; 3] {
        let target = self.current();
        let to = [target.a, target.b, target.glow];
        let t = self.fade_progress(now) as f32;
        let (Some(from), true) = (self.previous, t < 1.0) else { return to };
        // Smoothstep, so the fade has no visible start or stop edge.
        let e = t * t * (3.0 - 2.0 * t);
        let mut out = to;
        for i in 0..3 {
            for c in 0..3 {
                out[i][c] = from[i][c] + (to[i][c] - from[i][c]) * e;
            }
        }
        out
    }
}
