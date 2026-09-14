use nimbus_wayland::focus::Rect;
use nimbus_wayland::render::uniforms::*;

/// The macOS version shipped a bug where a flare made the pattern spin, because
/// position was computed as time * speed. Integrating means a speed change
/// alters the rate from here, never the position.
#[test]
fn changing_speed_does_not_move_the_pattern() {
    let mut clock = PhaseClock::default();
    clock.advance(1000.0, 1.0);
    clock.advance(1000.05, 1.0);
    let before = clock.flow;

    // A flare arrives: speed jumps. The phase must continue, not leap.
    clock.advance(1000.05, 5.0);
    assert!((clock.flow - before).abs() < 1e-9, "a speed change alone must not move the phase");

    clock.advance(1000.10, 5.0);
    assert!(clock.flow > before, "it should then advance faster");
}

#[test]
fn phase_only_ever_moves_forward() {
    let mut clock = PhaseClock::default();
    let mut last = 0.0;
    for i in 0..100 {
        clock.advance(1000.0 + i as f64 * 0.016, 0.45);
        assert!(clock.flow >= last);
        last = clock.flow;
    }
}

/// Rendering pauses while the ring is hidden. Without a clamp, the first frame
/// back advances the pattern by however long it was away and the ring jumps.
#[test]
fn a_long_pause_does_not_jump_the_pattern() {
    let mut clock = PhaseClock::default();
    clock.advance(0.0, 1.0);
    clock.advance(0.016, 1.0);
    let before = clock.flow;
    clock.advance(600.0, 1.0); // ten minutes hidden
    let jumped = clock.flow - before;
    assert!(jumped <= 0.1 + 1e-9, "advanced by {jumped}, should be clamped to 0.1");
}

#[test]
fn band_is_clamped_on_windows_smaller_than_it() {
    let tiny = Rect { x: 0, y: 0, w: 60, h: 60 };
    let (inner, outer) = clamp_band(6.0, 18.0, tiny);
    assert_eq!(inner, 6.0, "6 already fits inside 60/4");
    assert_eq!(outer, 15.0, "18 must clamp to min(w,h)/4");

    let normal = Rect { x: 0, y: 0, w: 1440, h: 900 };
    assert_eq!(clamp_band(6.0, 18.0, normal), (6.0, 18.0));
}

/// Compositor coordinates run y down and the shader works y up. Getting this
/// backwards is the single most common source of bugs in this kind of program.
#[test]
fn rect_conversion_flips_y_and_applies_scale() {
    let window = Rect { x: 100, y: 50, w: 800, h: 600 };
    let r = window_rect_in_surface(window, (0, 0), 1080, 1.0);
    assert_eq!(r[0], 100.0);
    // 1080 - 50 - 600 = 430 measured from the bottom.
    assert_eq!(r[1], 430.0);
    assert_eq!(r[2], 800.0);
    assert_eq!(r[3], 600.0);
}

#[test]
fn rect_conversion_is_relative_to_the_surface_origin() {
    // A window on a second monitor whose origin is at x = 1920.
    let window = Rect { x: 2000, y: 100, w: 800, h: 600 };
    let r = window_rect_in_surface(window, (1920, 0), 1080, 1.0);
    assert_eq!(r[0], 80.0, "x must be surface-local, not global");
}

#[test]
fn scale_multiplies_every_component() {
    let window = Rect { x: 100, y: 50, w: 800, h: 600 };
    let one = window_rect_in_surface(window, (0, 0), 1080, 1.0);
    let two = window_rect_in_surface(window, (0, 0), 1080, 2.0);
    for i in 0..4 {
        assert_eq!(two[i], one[i] * 2.0, "component {i} must scale");
    }
}
