//! The two debounces, both of which exist because of behaviour observed on a
//! live Hyprland 0.56.2 socket rather than inferred from documentation.

use nimbus_wayland::focus::tracker::Tracker;
use nimbus_wayland::focus::{normalize_address, FocusState, Rect};

fn state(address: &str) -> FocusState {
    FocusState {
        address: address.into(),
        output: "DP-1".into(),
        class: "foot".into(),
        rect: Rect { x: 0, y: 0, w: 800, h: 600 },
        fullscreen: false,
    }
}

/// The event socket says `55593885e2e0`; `hyprctl -j clients` says
/// `0x55593885e2e0`. Geometry is joined across that gap, so without
/// normalization the match silently never succeeds and the ring never appears.
#[test]
fn the_two_address_spellings_agree_once_normalized() {
    assert_eq!(normalize_address("55593885e2e0"), "55593885e2e0");
    assert_eq!(normalize_address("0x55593885e2e0"), "55593885e2e0");
    assert_eq!(normalize_address("0x55593885E2E0"), "55593885e2e0");
    assert_eq!(normalize_address("  0x5647ab3c\n"), "5647ab3c");
    assert_eq!(normalize_address(""), "");
    assert_eq!(
        normalize_address("55593885e2e0"),
        normalize_address("0x55593885e2e0"),
        "the socket form and the clients form must compare equal"
    );
}

#[test]
fn a_window_with_focus_gets_a_ring() {
    let mut t = Tracker::default();
    assert!(t.visible(0.0).is_none(), "nothing focused yet");
    assert!(t.focus(state("aaa"), 0.0), "first window is a different window");
    assert_eq!(t.visible(0.0).map(|s| s.address.as_str()), Some("aaa"));
}

#[test]
fn flare_fires_only_on_a_change_to_a_different_window() {
    let mut t = Tracker::default();
    t.focus(state("aaa"), 0.0);

    let mut moved = state("aaa");
    moved.rect = Rect { x: 500, y: 400, w: 800, h: 600 };
    assert!(!t.focus(moved, 1.0), "the same window moving must not flare");

    assert!(t.focus(state("bbb"), 2.0), "a different window must flare");
}

/// Hyprland emits a transient empty `activewindowv2` in the middle of every
/// focus change. Acting on it directly blinks the ring off on every switch,
/// which is exactly the flicker this exists to rule out.
#[test]
fn the_transient_empty_focus_event_does_not_blink_the_ring() {
    let mut t = Tracker::default();
    t.focus(state("aaa"), 0.0);

    // The empty event arrives mid-switch...
    t.note_unfocus(1.000);
    assert!(
        t.visible(1.000).is_some(),
        "must not go dark the instant an empty event arrives"
    );
    assert!(t.visible(1.008).is_some(), "still lit one frame later at 120Hz");
    assert!(t.visible(1.016).is_some(), "still lit one frame later at 60Hz");

    // ...and the real window follows a frame or two behind.
    assert!(t.focus(state("bbb"), 1.020));
    assert_eq!(t.visible(1.020).map(|s| s.address.as_str()), Some("bbb"));
    assert!(
        !t.unfocus_confirmed(9.0),
        "the pending unfocus must be cancelled, not merely outrun"
    );
}

/// The same event, when it is real: clicking the desktop leaves nothing
/// focused, and the ring must go away. Showing it on the previously focused
/// window would be a lie, since that window will not receive the next keystroke.
#[test]
fn a_real_unfocus_does_put_the_ring_out() {
    let mut t = Tracker::default();
    t.focus(state("aaa"), 0.0);
    t.note_unfocus(1.0);

    assert!(t.visible(1.0).is_some(), "not yet");
    assert!(t.visible(1.0 + Tracker::default().unfocus_grace).is_none());
    assert!(t.visible(2.0).is_none(), "and it stays out");
}

#[test]
fn a_confirmed_unfocus_is_forgotten_so_the_next_window_flares() {
    let mut t = Tracker::default();
    t.focus(state("aaa"), 0.0);
    t.note_unfocus(1.0);
    t.retire_confirmed_unfocus(2.0);
    assert!(t.focused().is_none());
    assert!(
        t.focus(state("aaa"), 3.0),
        "returning to the same window after a real unfocus is still a change"
    );
}

/// Hide while dragging, restore when the window comes to rest.
/// Hyprland has no "drag finished" event, so rest is measured by a clock.
#[test]
fn the_ring_stands_down_for_a_drag_and_comes_back_at_rest() {
    let mut t = Tracker::default();
    t.focus(state("aaa"), 0.0);

    t.note_moving(1.0);
    assert!(t.visible(1.0).is_none(), "down as soon as movement starts");

    // A drag is a stream of events; each one must push the deadline out.
    for step in 1..=20 {
        let now = 1.0 + step as f64 * 0.05;
        t.note_moving(now);
        assert!(t.visible(now).is_none(), "still down at {now}");
    }

    let last = 1.0 + 20.0 * 0.05;
    assert!(t.visible(last + 0.1).is_none(), "not back before it has settled");
    assert!(t.visible(last + 0.3).is_some(), "back once the window is at rest");
    assert_eq!(
        t.visible(last + 0.3).map(|s| s.address.as_str()),
        Some("aaa"),
        "and around the same window, without a flare"
    );
}

/// Both decisions are made by a clock, so the frame loop has to be woken to
/// make them. Otherwise the ring hangs in the wrong state until some unrelated
/// event happens to arrive.
#[test]
fn pending_decisions_keep_the_frame_loop_awake() {
    let mut t = Tracker::default();
    t.focus(state("aaa"), 0.0);
    assert!(!t.has_pending_deadline(0.0), "nothing outstanding when settled");

    t.note_unfocus(1.0);
    assert!(t.has_pending_deadline(1.0), "an undecided unfocus is outstanding");
    assert!(!t.has_pending_deadline(2.0), "once decided it is not");

    let mut t2 = Tracker::default();
    t2.focus(state("aaa"), 0.0);
    t2.note_moving(1.0);
    assert!(t2.has_pending_deadline(1.0), "a drag is outstanding");
    assert!(!t2.has_pending_deadline(5.0), "a settled window is not");
}
