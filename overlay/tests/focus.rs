use nimbus_wayland::focus::{hyprland::parse_line, Event, FocusState, Rect};

#[test]
fn parses_focus_change() {
    assert_eq!(
        parse_line("activewindowv2>>5647ab3c"),
        Some(Event::Focused("5647ab3c".into()))
    );
}

/// Clicking the desktop leaves nothing focused. The ring must go away: showing
/// it on the previously focused window would be a lie, since that window will
/// not receive the next keystroke.
#[test]
fn empty_address_means_nothing_is_focused() {
    assert_eq!(parse_line("activewindowv2>>"), Some(Event::Unfocused));
    assert_eq!(parse_line("activewindow>>,"), Some(Event::Unfocused));
}

#[test]
fn parses_movement_and_takes_the_address_only() {
    assert_eq!(
        parse_line("movewindowv2>>5647ab3c,1,workspace-one"),
        Some(Event::Moving("5647ab3c".into()))
    );
    assert_eq!(
        parse_line("resizewindow>>5647ab3c"),
        Some(Event::Moving("5647ab3c".into()))
    );
}

#[test]
fn parses_monitor_events() {
    assert_eq!(parse_line("monitoradded>>DP-3"), Some(Event::OutputAdded("DP-3".into())));
    assert_eq!(parse_line("monitorremoved>>DP-3"), Some(Event::OutputRemoved("DP-3".into())));
}

#[test]
fn events_needing_a_geometry_reread_ask_for_a_rescan() {
    for line in ["workspace>>2", "focusedmon>>DP-1,3", "fullscreen>>1", "closewindow>>abc"] {
        assert_eq!(parse_line(line), Some(Event::Rescan), "{line}");
    }
}

#[test]
fn unknown_and_malformed_lines_are_ignored_not_fatal() {
    assert_eq!(parse_line("somethingnew>>data"), None);
    assert_eq!(parse_line("no separator here"), None);
    assert_eq!(parse_line(""), None);
}

fn state(address: &str) -> FocusState {
    FocusState {
        address: address.into(),
        output: "DP-1".into(),
        class: "foot".into(),
        rect: Rect { x: 0, y: 0, w: 800, h: 600 },
        fullscreen: false,
    }
}

/// The flare must fire on a change of window, and must not fire when the same
/// window merely moves, or dragging keeps it lit and it never settles.
#[test]
fn flare_fires_on_a_different_window_only() {
    let a = state("aaa");
    let mut b = state("bbb");
    assert!(a.is_different_window(None));
    assert!(b.is_different_window(Some(&a)));

    let mut moved = a.clone();
    moved.rect = Rect { x: 500, y: 400, w: 800, h: 600 };
    assert!(!moved.is_different_window(Some(&a)), "same window moving is not a focus change");

    b.rect = moved.rect;
    assert!(b.is_different_window(Some(&moved)), "geometry matching is irrelevant; identity decides");
}

#[test]
fn absurd_rects_are_rejected() {
    assert!(Rect { x: 0, y: 0, w: 1440, h: 900 }.is_sensible());
    assert!(!Rect { x: 0, y: 0, w: 10, h: 900 }.is_sensible());
    assert!(!Rect { x: 0, y: 0, w: 50_000, h: 900 }.is_sensible());
}
