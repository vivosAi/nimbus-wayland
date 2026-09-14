//! The settings, checked against the macOS defaults they are a port of.
//!
//! These are not arbitrary numbers. The whole point of the project is that the
//! two versions look and behave the same, so a default that drifts from
//! `Preferences.swift` is a bug, and these tests are what catches it.

use nimbus_wayland::config::{BandWidth, Config, IdleBehavior, MotionSpeed, State, Turbulence};

#[test]
fn defaults_match_the_macos_app() {
    let c = Config::default();
    assert!(c.enabled);
    assert_eq!(c.idle_intensity, 0.30);
    assert_eq!(c.flare_duration, 2.5);
    assert_eq!(c.band_width, BandWidth::Normal);
    assert_eq!(c.motion_speed, MotionSpeed::Normal);
    assert_eq!(c.turbulence, Turbulence::Normal);
    // 30, not 60. Cost is very nearly linear in frame rate, so this is the
    // setting that decides what the program costs.
    assert_eq!(c.frame_rate, 30);
    assert_eq!(c.palette_interval, 1800.0);
    assert!(c.hide_in_fullscreen);
    assert!(c.hide_while_dragging);
    assert_eq!(c.idle_behavior, IdleBehavior::Freeze);
    assert_eq!(c.idle_threshold, 0.0, "0 means never go idle");
    assert!(c.flare_on_return);
}

/// The three enumerated settings carry the macOS numbers exactly. Getting one
/// of these wrong makes the ring subtly unlike the Mac in a way that is very
/// hard to name and impossible to miss side by side.
#[test]
fn enumerated_settings_carry_the_macos_numbers() {
    assert_eq!(BandWidth::Thin.points(), (4.0, 12.0));
    assert_eq!(BandWidth::Normal.points(), (6.0, 18.0));
    assert_eq!(BandWidth::Thick.points(), (9.0, 28.0));

    assert_eq!(MotionSpeed::Calm.flow_speed(), 0.22);
    assert_eq!(MotionSpeed::Normal.flow_speed(), 0.45);
    assert_eq!(MotionSpeed::Lively.flow_speed(), 0.85);

    assert_eq!(Turbulence::Smooth.noise_scale(), 2.5);
    assert_eq!(Turbulence::Normal.noise_scale(), 4.0);
    assert_eq!(Turbulence::Churny.noise_scale(), 6.5);
}

/// `RingRenderer.swift` computes `max(bandOuter * scale * 0.6, 1.0)`. It is
/// derived rather than set independently so that a wider ring gets a
/// proportionally wider bloom and the two cannot drift apart.
#[test]
fn glow_falloff_is_derived_from_the_band_not_set_beside_it() {
    let mut c = Config::default();
    assert_eq!(c.glow_falloff(), 18.0 * 0.6);
    c.band_width = BandWidth::Thin;
    assert_eq!(c.glow_falloff(), 12.0 * 0.6);
    c.band_width = BandWidth::Thick;
    assert_eq!(c.glow_falloff(), 28.0 * 0.6);
    assert!(c.glow_falloff() >= 1.0, "never zero, or the bloom vanishes");
}

#[test]
fn settings_round_trip_through_json() {
    let (c, warnings) = Config::from_json(
        r#"{
            "enabled": false,
            "idle_intensity": 0.5,
            "band_width": "thick",
            "motion_speed": "calm",
            "turbulence": "churny",
            "frame_rate": 20,
            "palette_interval": 0,
            "exclusions": ["mpv", "steam_app_0"],
            "hide_in_fullscreen": false
        }"#,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!c.enabled);
    assert_eq!(c.idle_intensity, 0.5);
    assert_eq!(c.band_width, BandWidth::Thick);
    assert_eq!(c.motion_speed, MotionSpeed::Calm);
    assert_eq!(c.turbulence, Turbulence::Churny);
    assert_eq!(c.frame_rate, 20);
    assert_eq!(c.palette_interval, 0.0, "0 means never rotate");
    assert_eq!(c.exclusions, vec!["mpv", "steam_app_0"]);
    assert!(!c.hide_in_fullscreen);
}

/// One bad setting must cost that setting alone. An overlay that refuses to
/// start because a number was spelled wrong is worse than one that starts with
/// a default, and losing every *other* setting to one typo is worse still.
#[test]
fn a_bad_value_costs_only_its_own_setting() {
    let (c, warnings) = Config::from_json(
        r#"{
            "band_width": "enormous",
            "frame_rate": 5000,
            "idle_intensity": 12,
            "motion_speed": "lively"
        }"#,
    );
    assert_eq!(c.band_width, BandWidth::Normal, "fell back");
    assert_eq!(c.frame_rate, 30, "fell back");
    assert_eq!(c.idle_intensity, 0.30, "fell back");
    assert_eq!(c.motion_speed, MotionSpeed::Lively, "the good one survived");
    assert_eq!(warnings.len(), 3, "and each bad one is reported: {warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("band_width")));
}

#[test]
fn a_broken_file_still_starts_the_ring() {
    for text in ["", "not json at all", "[1, 2, 3]", "null"] {
        let (c, warnings) = Config::from_json(text);
        assert_eq!(c, Config::default(), "{text:?} must fall back cleanly");
        assert!(!warnings.is_empty(), "{text:?} should say why");
    }
}

/// A config written by a newer version must not make an older one shout on
/// every line it does not know about.
#[test]
fn unknown_keys_are_ignored_silently() {
    let (c, warnings) = Config::from_json(r#"{"some_future_setting": 42, "frame_rate": 60}"#);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(c.frame_rate, 60);
}

/// Without persisting the rotation, anyone who restarts often would see the
/// first palette far more than any other, which defeats its whole purpose.
#[test]
fn rotation_state_survives_a_restart() {
    let before = State {
        palette_index: 7,
        palette_recent: vec![2, 5, 7],
        palette_changed_at: 1_726_000_000.0,
    };
    let after = State::from_json(&before.to_json());
    assert_eq!(before, after);
}

#[test]
fn missing_or_broken_state_is_not_fatal() {
    assert_eq!(State::from_json("garbage"), State::default());
    assert_eq!(State::from_json("{}"), State::default());
}

/// The idle defaults are the macOS ones, and the important one is
/// `Freeze` rather than `FadeOut`: SPEC.md §10 is explicit that walking back to
/// the machine and looking at which window has focus, before touching
/// anything, is the case this program exists for. Hiding the ring while you are
/// away removes it exactly when it is most wanted.
#[test]
fn idle_defaults_keep_the_ring_on_screen() {
    let c = Config::default();
    assert_eq!(c.idle_behavior, IdleBehavior::Freeze);
    assert_eq!(c.idle_threshold, 0.0, "0 means never go idle at all");
    assert!(c.flare_on_return, "coming back is when you most need marking");
}

#[test]
fn idle_settings_parse_in_both_spellings() {
    // snake_case is ours; camelCase is what the macOS defaults are written in,
    // and someone copying a value across should not be punished for it.
    for (text, expected) in [
        (r#"{"idle_behavior":"freeze"}"#, IdleBehavior::Freeze),
        (r#"{"idle_behavior":"fade_out"}"#, IdleBehavior::FadeOut),
        (r#"{"idle_behavior":"fadeOut"}"#, IdleBehavior::FadeOut),
        (r#"{"idle_behavior":"always_animate"}"#, IdleBehavior::AlwaysAnimate),
        (r#"{"idle_behavior":"alwaysAnimate"}"#, IdleBehavior::AlwaysAnimate),
    ] {
        let (c, warnings) = Config::from_json(text);
        assert!(warnings.is_empty(), "{text}: {warnings:?}");
        assert_eq!(c.idle_behavior, expected, "{text}");
    }

    let (c, warnings) = Config::from_json(r#"{"idle_threshold": 300}"#);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(c.idle_threshold, 300.0);

    let (c, warnings) = Config::from_json(r#"{"idle_behavior":"vanish"}"#);
    assert_eq!(c.idle_behavior, IdleBehavior::Freeze, "unknown falls back");
    assert_eq!(warnings.len(), 1);
}
