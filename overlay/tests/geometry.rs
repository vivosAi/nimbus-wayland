use nimbus_wayland::focus::geometry::{
    find_client, parse_active_window, parse_client, parse_monitors, Monitor,
};
use nimbus_wayland::focus::Rect;

fn monitors() -> Vec<Monitor> {
    parse_monitors(
        r#"[
            {"id": 0, "name": "DP-1", "x": 0, "y": 0, "width": 2560, "height": 1440, "scale": 1.0},
            {"id": 1, "name": "HDMI-A-1", "x": 2560, "y": 0, "width": 1920, "height": 1080, "scale": 1.25}
        ]"#,
    )
    .unwrap()
}

/// Trimmed from a real `hyprctl -j activewindow` on 0.56.2: the fields we read,
/// plus a few we do not, in the order Hyprland prints them.
const ACTIVE: &str = r#"{
    "address": "0x55a5ad510e90",
    "mapped": true,
    "at": [12, 38],
    "size": [596, 750],
    "workspace": {"id": 1, "name": "1"},
    "floating": false,
    "monitor": 1,
    "class": "foot",
    "title": "nimbus",
    "fullscreen": 0,
    "fullscreenClient": 0
}"#;

#[test]
fn the_focused_window_reply_becomes_a_placed_focus_state() {
    let state = parse_active_window(ACTIVE, &monitors()).unwrap().unwrap();
    assert_eq!(state.address, "55a5ad510e90", "0x stripped, so it joins with the event socket");
    assert_eq!(state.output, "HDMI-A-1", "monitor id translated to the name Wayland uses");
    assert_eq!(state.class, "foot");
    assert_eq!(state.rect, Rect { x: 12, y: 38, w: 596, h: 750 });
    assert!(!state.fullscreen);
}

/// Nothing focused is `{}`, not an error and not a window.
#[test]
fn nothing_focused_is_an_empty_object() {
    assert_eq!(parse_active_window("{}", &monitors()).unwrap(), None);
    assert!(parse_active_window("[]", &monitors()).is_err(), "the wrong shape is worth saying");
    assert!(parse_active_window("nonsense", &monitors()).is_err());
}

/// The poll and the event path must agree about a window, or the poll would
/// report a "change" on every tick.
#[test]
fn the_client_list_and_the_focused_window_describe_a_window_identically() {
    let list = format!("[{{\"address\": \"0xdeadbeef\", \"at\": [0, 0], \"size\": [100, 100], \"monitor\": 0, \"class\": \"x\", \"fullscreen\": 2}}, {ACTIVE}]");
    let from_list = find_client(&list, "55a5ad510e90", &monitors()).unwrap().unwrap();
    let from_active = parse_active_window(ACTIVE, &monitors()).unwrap().unwrap();
    assert_eq!(from_list, from_active);

    let other = find_client(&list, "0xDEADBEEF", &monitors()).unwrap().unwrap();
    assert!(other.fullscreen, "fullscreen is a mode: any non-zero value counts");
    assert_eq!(other.output, "DP-1");
}

/// A window that is mapped but not yet sized reports a zero rect. That is not
/// a window to ring; the next event will carry a real one.
#[test]
fn an_unsized_or_absurd_window_is_not_placed() {
    let not_yet_sized: serde_json::Value = serde_json::from_str(
        r#"{"address": "0x1", "at": [0, 0], "size": [0, 0], "monitor": 0, "class": "x", "fullscreen": 0}"#,
    )
    .unwrap();
    assert_eq!(parse_client(&not_yet_sized, &monitors()), None);

    let no_address: serde_json::Value =
        serde_json::from_str(r#"{"at": [0, 0], "size": [800, 600]}"#).unwrap();
    assert_eq!(parse_client(&no_address, &monitors()), None);
}

/// An unknown monitor id must not fail the whole placement: the ring would
/// simply draw on no output, which the next `Rescan` corrects.
#[test]
fn an_unknown_monitor_yields_an_empty_output_name() {
    let json = ACTIVE.replace("\"monitor\": 1", "\"monitor\": 7");
    let state = parse_active_window(&json, &monitors()).unwrap().unwrap();
    assert_eq!(state.output, "");
}
