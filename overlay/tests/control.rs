//! The control protocol: what the bar panel and the CLI both speak.

use nimbus_wayland::config::{BandWidth, Config};
use nimbus_wayland::control::{
    coerce_setting, merge_into_config_file, status_json, PaletteView, Request,
};

fn views(names: &[&'static str]) -> Vec<PaletteView> {
    names
        .iter()
        .map(|n| PaletteView {
            name: n,
            hex: ["#ff3d00".into(), "#ffc400".into(), "#ff6d00".into()],
            in_rotation: true,
        })
        .collect()
}

#[test]
fn requests_round_trip() {
    for request in [
        Request::Status,
        Request::NextColor,
        Request::SetPalette("Aurora".into()),
        Request::Reload,
        Request::Quit,
    ] {
        assert_eq!(Request::parse(&request.to_json()), Ok(request.clone()), "{request:?}");
    }

    let mut map = serde_json::Map::new();
    map.insert("frame_rate".into(), serde_json::json!(15));
    let set = Request::Set(map);
    assert_eq!(Request::parse(&set.to_json()), Ok(set));
}

/// The socket is reachable by anything on the machine that can open it, so it
/// has to treat every line as hostile input rather than as a well-formed
/// request from our own CLI.
#[test]
fn malformed_requests_are_refused_not_guessed_at() {
    for line in [
        "",
        "hello",
        "{}",
        r#"{"command":42}"#,
        r#"{"command":"launch_missiles"}"#,
        r#"{"command":"set"}"#,
        r#"{"command":"set","settings":{}}"#,
        r#"{"command":"set_palette"}"#,
        r#"{"command":"set_palette","name":""}"#,
    ] {
        assert!(Request::parse(line).is_err(), "{line:?} should be refused");
    }
}

/// A command line hands over strings and nothing else, so every value has to
/// be turned into the type its setting expects or they would all be rejected.
#[test]
fn command_line_strings_become_the_right_types() {
    assert_eq!(coerce_setting("enabled", "true"), Ok(serde_json::json!(true)));
    assert_eq!(coerce_setting("enabled", "off"), Ok(serde_json::json!(false)));
    assert_eq!(coerce_setting("frame_rate", "20"), Ok(serde_json::json!(20)));
    assert_eq!(coerce_setting("idle_intensity", "0.5"), Ok(serde_json::json!(0.5)));
    assert_eq!(
        coerce_setting("band_width", "thick"),
        Ok(serde_json::json!("thick"))
    );
    assert_eq!(
        coerce_setting("exclusions", "mpv, vlc"),
        Ok(serde_json::json!(["mpv", "vlc"]))
    );
    assert_eq!(
        coerce_setting("exclusions", ""),
        Ok(serde_json::json!([])),
        "clearing the list must be expressible"
    );

    assert!(coerce_setting("frame_rate", "fast").is_err());
    assert!(coerce_setting("enabled", "maybe").is_err());
    assert!(coerce_setting("no_such_setting", "1").is_err());
}

/// The panel redraws entirely from one status call, so everything it shows has
/// to be in there — a second call to learn the palette names would let the two
/// disagree.
#[test]
fn status_carries_everything_the_panel_draws() {
    let config = Config::default();
    let text = status_json(&config, &views(&["Ember", "Plasma", "Toxic"]), 1, true);
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["showing"], true);
    assert_eq!(v["palette_name"], "Plasma");
    assert_eq!(v["palettes"].as_array().unwrap().len(), 3);
    // The panel draws a swatch per palette, so the colours have to come with
    // the names — a second call to fetch them could disagree with the first.
    assert_eq!(v["palettes"][0]["name"], "Ember");
    assert_eq!(v["palettes"][0]["a"], "#ff3d00");
    assert_eq!(v["palettes"][0]["in_rotation"], true);
    assert_eq!(v["settings"]["frame_rate"], 30);
    assert_eq!(v["settings"]["band_width"], "normal");

    // f32 widened to f64 gives 0.30000001192092896, which is the same number
    // but reads as a bug and is awkward to display.
    assert_eq!(v["settings"]["idle_intensity"], 0.3);
}

/// Every setting the status reports must be one the config parser accepts, or
/// the panel could read a value back and fail to write it again.
#[test]
fn status_settings_feed_straight_back_into_the_parser() {
    let mut config = Config::default();
    config.band_width = BandWidth::Thick;
    config.frame_rate = 15;
    config.exclusions = vec!["mpv".into()];

    let text = status_json(&config, &views(&["Ember"]), 0, false);
    let settings = serde_json::from_str::<serde_json::Value>(&text).unwrap()["settings"].clone();

    let (round_tripped, warnings) = Config::from_json(&settings.to_string());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(round_tripped.band_width, BandWidth::Thick);
    assert_eq!(round_tripped.frame_rate, 15);
    assert_eq!(round_tripped.exclusions, vec!["mpv".to_string()]);
}

/// The config file is hand-edited and may carry keys a future version added.
/// Writing one setting must not delete the rest of it.
#[test]
fn writing_one_setting_leaves_the_rest_of_the_file_alone() {
    let dir = std::env::temp_dir().join(format!("nimbus-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");

    std::fs::write(
        &path,
        r#"{"// a comment key": "kept", "frame_rate": 60, "some_future_setting": [1,2]}"#,
    )
    .unwrap();

    let mut change = serde_json::Map::new();
    change.insert("frame_rate".into(), serde_json::json!(20));
    merge_into_config_file(&path, &change).unwrap();

    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(after["frame_rate"], 20, "the change landed");
    assert_eq!(after["// a comment key"], "kept", "comments survive");
    assert_eq!(
        after["some_future_setting"],
        serde_json::json!([1, 2]),
        "a setting we do not understand is not deleted"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn writing_to_a_file_that_does_not_exist_yet_creates_it() {
    let dir = std::env::temp_dir().join(format!("nimbus-test-new-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let path = dir.join("nested").join("config.json");

    let mut change = serde_json::Map::new();
    change.insert("enabled".into(), serde_json::json!(false));
    merge_into_config_file(&path, &change).unwrap();

    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(after["enabled"], false);

    std::fs::remove_dir_all(&dir).ok();
}
