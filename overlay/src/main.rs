#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("nimbus-wayland runs on Linux under a Wayland compositor.");
    eprintln!("The logic crate builds anywhere; only the surface does not.");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    use nimbus_wayland::app::server;
    use nimbus_wayland::control::{coerce_setting, Request};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();

    // No arguments runs the overlay. Everything else talks to one that is
    // already running, the way the macOS menu talks to the app rather than
    // launching a second copy of it.
    let request = match words.as_slice() {
        [] => match nimbus_wayland::app::run() {
            Ok(()) => return,
            Err(e) => {
                eprintln!("nimbus: {e}");
                std::process::exit(1);
            }
        },
        ["-h" | "--help" | "help"] => {
            print!("{USAGE}");
            return;
        }
        ["-V" | "--version" | "version"] => {
            println!("nimbus-wayland {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        // These two do not talk to a running Nimbus at all; they move files
        // about, so they answer for themselves and return.
        ["install-bar"] => {
            match nimbus_wayland::install::install_bar() {
                Ok(message) => {
                    println!("{message}");
                    println!("{}", nimbus_wayland::install::config_hint());
                    return;
                }
                Err(e) => {
                    eprintln!("nimbus: {e}");
                    std::process::exit(1);
                }
            }
        }
        ["uninstall-bar"] => {
            match nimbus_wayland::install::uninstall_bar() {
                Ok(message) => {
                    println!("{message}");
                    return;
                }
                Err(e) => {
                    eprintln!("nimbus: {e}");
                    std::process::exit(1);
                }
            }
        }
        ["status"] => Request::Status,
        ["next-color" | "next-colour"] => Request::NextColor,
        ["color" | "colour", name] => Request::SetPalette((*name).to_string()),
        ["reload"] => Request::Reload,
        ["quit"] => Request::Quit,
        ["toggle"] => {
            // Read-modify-write, because "toggle" is a question about the
            // current value and the daemon is the only thing that knows it.
            match toggle_request() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("nimbus: {e}");
                    std::process::exit(1);
                }
            }
        }
        ["set", key, value] => match coerce_setting(key, value) {
            Ok(v) => {
                let mut map = serde_json::Map::new();
                map.insert((*key).to_string(), v);
                Request::Set(map)
            }
            Err(e) => {
                eprintln!("nimbus: {e}");
                std::process::exit(1);
            }
        },
        _ => {
            eprintln!("nimbus: do not understand {:?}\n", args.join(" "));
            eprint!("{USAGE}");
            std::process::exit(2);
        }
    };

    match server::ask(&request) {
        Ok(reply) => {
            println!("{reply}");
            // The reply is JSON and the caller may be a script, so failure has
            // to show up in the exit status too rather than only in the text.
            let ok = serde_json::from_str::<serde_json::Value>(&reply)
                .map(|v| v["ok"].as_bool().unwrap_or(false))
                .unwrap_or(false);
            if !ok {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("nimbus: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "linux")]
fn toggle_request() -> Result<nimbus_wayland::control::Request, String> {
    use nimbus_wayland::app::server;
    use nimbus_wayland::control::Request;

    let reply = server::ask(&Request::Status)?;
    let value: serde_json::Value =
        serde_json::from_str(&reply).map_err(|e| format!("unreadable status: {e}"))?;
    let enabled = value["settings"]["enabled"].as_bool().unwrap_or(true);

    let mut map = serde_json::Map::new();
    map.insert("enabled".into(), serde_json::Value::Bool(!enabled));
    Ok(Request::Set(map))
}

#[cfg(target_os = "linux")]
const USAGE: &str = "\
nimbus-wayland — an animated ring of light around the focused window

  nimbus-wayland                 run the overlay
  nimbus-wayland status          print everything as JSON
  nimbus-wayland toggle          switch the ring on or off
  nimbus-wayland set KEY VALUE   change one setting, live, and save it
  nimbus-wayland next-color      rotate the palette now
  nimbus-wayland color NAME      switch to a palette by name
  nimbus-wayland reload          re-read ~/.config/nimbus/config.json
  nimbus-wayland quit            stop the running overlay

  nimbus-wayland install-bar     add the control to the Omarchy bar
  nimbus-wayland uninstall-bar   take it back out

Settings, with their defaults:

  enabled              true            band_width     thin|normal|thick
  idle_intensity       0.30            motion_speed   calm|normal|lively
  flare_duration       2.5             turbulence     smooth|normal|churny
  frame_rate           30              debug_mode     0|1|2
  palette_interval     1800            exclusions     comma,separated,classes
  hide_in_fullscreen   true            disabled_palettes  comma,separated
  hide_while_dragging  true            idle_behavior  freeze|always_animate|fade_out
  flare_on_return      true            idle_threshold seconds, 0 = never

Examples:

  nimbus-wayland set idle_intensity 0.5      a louder ring at rest
  nimbus-wayland set frame_rate 20           cheaper on a slow machine
  nimbus-wayland set turbulence churny       finer structure in the light
  nimbus-wayland set exclusions mpv,vlc      never ring these
  nimbus-wayland color Aurora                pick a palette directly
  nimbus-wayland set idle_threshold 300      stand down after five minutes away
  nimbus-wayland set disabled_palettes Solar,Ice
                                             keep these out of the rotation

Every change takes effect on the next frame and is written to
~/.config/nimbus/config.json, leaving the rest of that file untouched.
";
