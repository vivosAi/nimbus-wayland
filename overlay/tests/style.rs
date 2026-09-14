use nimbus_wayland::style::*;

#[test]
fn palette_list_matches_the_macos_app() {
    let p = palettes();
    assert_eq!(p.len(), 10);
    assert_eq!(p[0].name, "Ember");
    let names: std::collections::HashSet<_> = p.iter().map(|x| x.name).collect();
    assert_eq!(names.len(), 10, "palette names must be unique");
}

/// A common mistake is a plain 2.2 power, which is wrong near black and shifts
/// every color.
#[test]
fn srgb_curve_endpoints_and_knee() {
    assert!((srgb_to_linear(0.0) - 0.0).abs() < 1e-6);
    assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
    // Below the knee the curve is a straight line, not a power.
    assert!((srgb_to_linear(0.04) - 0.04 / 12.92).abs() < 1e-6);
    // Mid grey: 0.5 sRGB is about 0.214 linear, distinctly not 0.5.
    assert!((srgb_to_linear(0.5) - 0.2140).abs() < 1e-3);
}

#[test]
fn channel_order_is_rgb_not_bgr() {
    let red = srgb_bytes(0xFF0000);
    assert_eq!(red, [1.0, 0.0, 0.0]);
    let blue = srgb_bytes(0x0000FF);
    assert_eq!(blue, [0.0, 0.0, 1.0]);
}

#[test]
fn idle_before_any_flare() {
    let a = Animator::default();
    assert!((a.intensity(100.0) - 0.30).abs() < 1e-5);
    assert!(!a.is_flaring(100.0));
}

#[test]
fn flare_peaks_full_then_settles_to_idle() {
    let mut a = Animator::default();
    a.flare(0.0);
    assert!((a.intensity(0.0) - 1.0).abs() < 1e-5);
    assert!((a.intensity(2.5) - 0.30).abs() < 1e-5);
    assert!((a.intensity(10.0) - 0.30).abs() < 1e-5);
}

#[test]
fn decay_is_monotonic_and_never_dips_below_idle() {
    let mut a = Animator::default();
    a.flare(0.0);
    let mut previous = 2.0;
    for step in 0..=50 {
        let v = a.intensity(step as f64 / 20.0);
        assert!(v <= previous + 1e-6, "intensity must never rise mid-decay");
        assert!(v >= 0.30 - 1e-6, "must never dim below the idle baseline");
        previous = v;
    }
}

/// Width and speed must relax on the same curve as brightness, or the ring
/// finishes brightening before it finishes shrinking and looks disjointed.
#[test]
fn width_and_speed_share_the_curve() {
    let mut a = Animator::default();
    a.flare(0.0);
    assert!((a.band_scale(0.0) - 1.6).abs() < 1e-5);
    assert!((a.speed_scale(0.0) - 1.8).abs() < 1e-5);
    assert!((a.band_scale(2.5) - 1.0).abs() < 1e-5);
    assert!((a.speed_scale(2.5) - 1.0).abs() < 1e-5);
}

#[test]
fn zero_flare_duration_is_safe() {
    let mut a = Animator::default();
    a.flare_duration = 0.0;
    a.flare(0.0);
    assert!(a.intensity(0.0).is_finite());
    assert!((a.intensity(0.0) - 0.30).abs() < 1e-5);
}

#[test]
fn rotation_never_picks_the_current_palette() {
    let mut rng = Rng::new(42);
    let mut r = Rotator::new(palettes(), 0);
    for step in 0..200 {
        let next = r.pick_next(&mut rng);
        assert_ne!(next, r.current_index());
        r.transition_to(next, step as f64 * 100.0);
    }
}

#[test]
fn rotation_never_repeats_within_the_recent_window() {
    let mut rng = Rng::new(7);
    let mut r = Rotator::new(palettes(), 0);
    let mut seen = vec![r.current_index()];
    for step in 0..200 {
        let next = r.pick_next(&mut rng);
        let window = &seen[seen.len().saturating_sub(RECENT_MEMORY)..];
        assert!(!window.contains(&next), "repeated within the recent window");
        r.transition_to(next, step as f64 * 100.0);
        seen.push(next);
    }
}

/// With almost everything unavailable the recency rule cannot be satisfied. It
/// must relax rather than deadlocking on one color forever.
#[test]
fn relaxes_recency_when_few_palettes_available() {
    let mut rng = Rng::new(99);
    let two: Vec<_> = palettes().into_iter().take(2).collect();
    let mut r = Rotator::new(two, 0);
    for step in 0..20 {
        let next = r.pick_next(&mut rng);
        assert_ne!(next, r.current_index(), "must keep alternating between the two");
        r.transition_to(next, step as f64);
    }
}

#[test]
fn single_palette_does_not_panic() {
    let mut rng = Rng::new(1);
    let one: Vec<_> = palettes().into_iter().take(1).collect();
    let r = Rotator::new(one, 0);
    assert_eq!(r.pick_next(&mut rng), 0);
}

#[test]
fn cross_fade_starts_at_the_old_color_and_ends_at_the_new() {
    let mut r = Rotator::new(palettes(), 0);
    let from = r.current().a;
    r.transition_to(4, 1000.0);
    let at_start = r.colors(1000.0);
    assert!((at_start[0][0] - from[0]).abs() < 1e-4);
    let at_end = r.colors(1000.0 + r.fade_duration);
    assert!((at_end[0][0] - r.current().a[0]).abs() < 1e-4);
}

#[test]
fn mid_fade_lies_between_the_two_and_is_never_a_cut() {
    let mut r = Rotator::new(palettes(), 0);
    let from = r.current().a[2];
    r.transition_to(4, 0.0);
    let to = r.current().a[2];
    let mid = r.colors(r.fade_duration / 2.0)[0][2];
    assert!(mid >= from.min(to) - 1e-4 && mid <= from.max(to) + 1e-4);
}

/// The config has modelled `disabled_palettes` from the start; until the
/// rotator was told about them the setting did nothing at all.
#[test]
fn the_timer_never_picks_a_palette_you_switched_off() {
    let mut r = Rotator::new(palettes(), 0);
    r.set_disabled(&["Toxic".into(), "magma".into()]);

    let toxic = palettes().iter().position(|p| p.name == "Toxic").unwrap();
    let magma = palettes().iter().position(|p| p.name == "Magma").unwrap();
    assert!(r.is_disabled(toxic));
    assert!(r.is_disabled(magma), "matching is case-insensitive");

    let mut rng = Rng::new(7);
    for step in 0..200 {
        let next = r.pick_next(&mut rng);
        assert_ne!(next, toxic, "picked a disabled palette at step {step}");
        assert_ne!(next, magma, "picked a disabled palette at step {step}");
        r.transition_to(next, step as f64);
    }
}

/// Recency is the rule that gives way when choices run short. Being switched
/// off is not — that is the user's instruction, not a preference of ours.
#[test]
fn recency_relaxes_before_a_disabled_palette_is_used() {
    let all = palettes();
    let keep: Vec<String> = all.iter().skip(2).map(|p| p.name.to_string()).collect();
    let mut r = Rotator::new(all, 0);
    r.set_disabled(&keep); // everything except the first two

    let mut rng = Rng::new(3);
    for step in 0..50 {
        let next = r.pick_next(&mut rng);
        assert!(next <= 1, "step {step} escaped the two that are left");
        r.transition_to(next, step as f64);
    }
}

/// With everything but the current palette switched off there is nothing to
/// move to, and staying put beats overriding the user.
#[test]
fn all_but_one_disabled_stays_put_rather_than_disobeying() {
    let all = palettes();
    let others: Vec<String> = all.iter().skip(1).map(|p| p.name.to_string()).collect();
    let mut r = Rotator::new(all, 0);
    r.set_disabled(&others);
    let mut rng = Rng::new(11);
    assert_eq!(r.pick_next(&mut rng), 0);
}

#[test]
fn a_palette_can_be_chosen_by_name() {
    let mut r = Rotator::new(palettes(), 0);
    assert!(r.select_by_name("aurora", 1.0), "names are case-insensitive");
    assert_eq!(r.current().name, "Aurora");
    assert!(!r.select_by_name("Chartreuse", 2.0), "an unknown name is refused");
    assert_eq!(r.current().name, "Aurora", "and changes nothing");
}

/// The swatch a panel draws must be the colour the ring actually uses, so the
/// authored hex is kept rather than converted back out of linear space.
#[test]
fn palettes_report_the_hex_they_were_authored_in() {
    let ember = palettes()[0];
    assert_eq!(ember.name, "Ember");
    assert_eq!(
        ember.hex_strings(),
        ["#ff3d00".to_string(), "#ffc400".to_string(), "#ff6d00".to_string()]
    );
}
