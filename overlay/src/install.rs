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
