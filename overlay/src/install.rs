//! Putting the bar control where the shell can find it.
//!
//! A package installs into system directories and deliberately never writes to
//! anyone's home: one install serves every user on the machine, and `~` is not
//! the package manager's to touch. But an Omarchy bar plugin has to live in
//! `~/.config/omarchy/plugins/`.
//!
//! So the package ships the plugin to `/usr/share/nimbus-wayland/omarchy/` and
//! this copies it into place when the user asks. Explicitly, rather than on
//! first run: reaching into someone's desktop configuration uninvited is the
//! behaviour people rightly resent, and this audience most of all.

use std::path::{Path, PathBuf};

const PLUGIN_ID: &str = "nimbus.ring";
const FILES: [&str; 3] = ["manifest.json", "Panel.qml", "NimbusMark.qml"];

/// Where the plugin's source files are, wherever this build happens to be
/// running from.
///
/// Checked in order so that a packaged install, a `cargo run` from the repo and
/// a hand-placed binary all work without the user knowing which they have.
fn source_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // An override, mostly so this is testable and so a distribution that puts
    // things elsewhere is not stuck.
    if let Some(dir) = std::env::var_os("NIMBUS_SHARE") {
        candidates.push(PathBuf::from(dir).join("omarchy").join(PLUGIN_ID));
    }

    candidates.push(
        PathBuf::from("/usr/share/nimbus-wayland/omarchy").join(PLUGIN_ID),
    );
    candidates.push(
        PathBuf::from("/usr/local/share/nimbus-wayland/omarchy").join(PLUGIN_ID),
    );

    // Relative to the binary, for a tarball unpacked anywhere.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin) = exe.parent() {
            candidates.push(
                bin.join("../share/nimbus-wayland/omarchy").join(PLUGIN_ID),
            );
            // And straight out of a cloned repo: target/release/../../../omarchy
            candidates.push(bin.join("../../../omarchy").join(PLUGIN_ID));
        }
    }

    candidates
        .into_iter()
        .find(|c| c.join("manifest.json").is_file())
}

fn plugin_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("omarchy").join("plugins").join(PLUGIN_ID))
}

/// Is this an Omarchy desktop at all?
fn omarchy_present() -> bool {
    which("omarchy").is_some()
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// Copy the bar control into the user's config and add it to the bar.
pub fn install_bar() -> Result<String, String> {
    if !omarchy_present() {
        return Err(
            "this does not look like an Omarchy desktop — the bar control is \
             an Omarchy shell plugin.\n\
             Nimbus itself works fine without it; everything the panel does is \
             also a command (see `nimbus-wayland --help`)."
                .into(),
        );
    }

    let source = source_dir().ok_or(
        "cannot find the bar control's files.\n\
         They should be at /usr/share/nimbus-wayland/omarchy/nimbus.ring/ \
         — set NIMBUS_SHARE if they are somewhere else.",
    )?;
    let target = plugin_dir().ok_or("HOME is unset, so there is no config directory")?;

    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    for name in FILES {
        let from = source.join(name);
        let to = target.join(name);
        std::fs::copy(&from, &to)
            .map_err(|e| format!("copying {} to {}: {e}", from.display(), to.display()))?;
    }

    // Adding it to the bar is a separate step and a separate failure: the files
    // being in place is worth reporting even if the layout command is not
    // available.
    let placed = std::process::Command::new("omarchy")
        .args(["bar", "put", PLUGIN_ID, "--section", "right"])
        .output();

    let mut message = format!("Bar control installed to {}", target.display());
    match placed {
        Ok(out) if out.status.success() => {
            message.push_str("\nAdded to the bar. Click the ring to open it.");
        }
        _ => {
            message.push_str(
                "\nCould not add it to the bar automatically. Add it with:\n  \
                 omarchy bar put nimbus.ring --section right",
            );
        }
    }
    Ok(message)
}

/// Take it back out again. An install that cannot be undone is one people are
/// right to hesitate over.
pub fn uninstall_bar() -> Result<String, String> {
    let target = plugin_dir().ok_or("HOME is unset, so there is no config directory")?;

    let _ = std::process::Command::new("omarchy")
        .args(["bar", "drop", PLUGIN_ID])
        .output();

    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        Ok(format!("Bar control removed from {}", target.display()))
    } else {
        Ok("The bar control was not installed.".into())
    }
}

/// Where the settings file lives, whether or not it exists yet.
///
/// Printed after installing, because someone who does not use the Omarchy bar
/// still needs to know where to change things.
pub fn config_hint() -> String {
    let path = crate::config::config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "~/.config/nimbus/config.json".into());
    let exists = Path::new(&path).exists();
    if exists {
        format!("Settings: {path}")
    } else {
        format!("Settings: {path} (not created yet — defaults are in use)")
    }
}

// ---------------------------------------------------------------------------
// Running, and keeping running
// ---------------------------------------------------------------------------

const UNIT: &str = "nimbus-wayland.service";

/// Where we would write a unit of our own, if the install did not ship one.
///
/// `~/.config/systemd/user` rather than the package's `/usr/lib/systemd/user`:
/// it is writable, it takes precedence, and it can name the binary's real path,
/// which a packaged unit cannot when the binary was installed by hand.
fn user_unit_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("systemd").join("user").join(UNIT))
}

fn systemctl(args: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
}

/// Does systemd know about a unit for us, from any source?
fn unit_known() -> bool {
    systemctl(&["cat", UNIT])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Write a unit naming *this* binary.
///
/// Generated rather than copied from the package's, because the packaged one
/// hard-codes `/usr/bin/nimbus-wayland` and an install into `~/.local/bin` --
/// which is what the one-line installer does without root -- would point at
/// nothing.
fn write_user_unit() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("cannot find my own path: {e}"))?;
    let path = user_unit_path().ok_or("HOME is unset")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let unit = format!(
        "[Unit]\n\
         Description=Draws an animated ring of light around the window that has keyboard focus\n\
         Documentation=https://github.com/vivosAi/nimbus-wayland\n\
         PartOf=graphical-session.target\n\
         Requires=graphical-session.target\n\
         After=graphical-session.target\n\
         ConditionEnvironment=WAYLAND_DISPLAY\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={}\n\
         Slice=session.slice\n\
         Restart=on-failure\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n",
        exe.display()
    );
    std::fs::write(&path, unit).map_err(|e| format!("{}: {e}", path.display()))?;
    let _ = systemctl(&["daemon-reload"]);
    Ok(path)
}

/// Is Nimbus set to start with the session?
pub fn autostart_enabled() -> bool {
    systemctl(&["is-enabled", UNIT])
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
        .unwrap_or(false)
}

/// Start it now, whether or not it is set to start at login.
///
/// Through systemd where a unit exists, so the two agree about what is running;
/// detached otherwise, so this still works on a machine with no unit at all.
pub fn start() -> Result<String, String> {
    if crate::control::socket_path().is_some_and(|p| {
        std::os::unix::net::UnixStream::connect(p).is_ok()
    }) {
        return Ok("Nimbus is already running.".into());
    }

    if unit_known() {
        let out = systemctl(&["start", UNIT]).map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok("Nimbus started.".into());
        }
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }

    let exe = std::env::current_exe().map_err(|e| format!("cannot find my own path: {e}"))?;
    // setsid so it outlives whatever launched it -- a bar widget's Process, a
    // terminal that is about to close.
    std::process::Command::new("setsid")
        .arg("-f")
        .arg(&exe)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    Ok("Nimbus started.".into())
}

/// Turn starting-with-the-session on or off.
pub fn set_autostart(on: bool) -> Result<String, String> {
    if on {
        if !unit_known() {
            write_user_unit()?;
        }
        let out = systemctl(&["enable", "--now", UNIT]).map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok("Nimbus will start with every session.".into())
    } else {
        let out = systemctl(&["disable", UNIT]).map_err(|e| e.to_string())?;
        if !out.status.success() && unit_known() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok("Nimbus will no longer start with the session. It is still running; \
            stop it with: nimbus-wayland quit"
            .into())
    }
}
