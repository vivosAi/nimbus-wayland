#!/usr/bin/env bash
# Nimbus for Wayland — one-line install.
#
#   curl -fsSL https://raw.githubusercontent.com/vivosAi/nimbus-wayland/main/packaging/install.sh | bash
#
# Leaves you with the ring running, set to start at every login, and the bar
# control in place if you use the Omarchy bar. That is the whole job the macOS
# .dmg does; there is nothing to double-click because there is no window.
#
# On Arch this defers to the AUR, because a package you can later upgrade and
# remove with your normal tools beats a binary this script dropped somewhere.

set -euo pipefail

REPO="vivosAi/nimbus-wayland"
BIN_NAME="nimbus-wayland"
PREFIX="${NIMBUS_PREFIX:-$HOME/.local}"

say()  { printf '\033[1m%s\033[0m\n' "$*"; }
note() { printf '  %s\n' "$*"; }
die()  { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Refuse early and clearly, rather than installing something that cannot work.
# ---------------------------------------------------------------------------

[ "$(uname -s)" = "Linux" ] || die "Nimbus for Wayland runs on Linux."

if [ -z "${WAYLAND_DISPLAY:-}" ]; then
  die "no Wayland session detected (WAYLAND_DISPLAY is unset).
  Nimbus draws on a Wayland compositor's overlay layer; run this from inside
  your desktop session."
fi

if [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
  say "Warning: this does not look like Hyprland."
  note "Nimbus finds the focused window through Hyprland's IPC socket."
  note "Other wlroots compositors need a focus backend adding first."
  note "Continuing anyway — it will install, but may find nothing to ring."
  echo
fi

# Only x86_64 has a prebuilt tarball so far.
case "$(uname -m)" in
  x86_64)  ARCH=x86_64 ;;
  *) die "no prebuilt binary for $(uname -m). Build from source:
  git clone https://github.com/$REPO && cd nimbus-wayland/overlay && cargo build --release" ;;
esac

# ---------------------------------------------------------------------------
# On Arch, hand over to the package manager.
# ---------------------------------------------------------------------------

if command -v pacman >/dev/null 2>&1 && [ "${NIMBUS_FORCE_BINARY:-0}" != "1" ]; then
  for helper in yay paru; do
    if command -v "$helper" >/dev/null 2>&1; then
      say "Arch detected — installing from the AUR with $helper."
      note "so that upgrades and removal work with your normal tools."
      echo
      "$helper" -S --needed nimbus-wayland-bin
      INSTALLED_BY_PACKAGE=1
      break
    fi
  done
fi

# ---------------------------------------------------------------------------
# Otherwise, fetch the binary.
# ---------------------------------------------------------------------------

if [ -z "${INSTALLED_BY_PACKAGE:-}" ]; then
  TAG="${NIMBUS_VERSION:-$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)}"
  [ -n "$TAG" ] || die "could not work out the latest release of $REPO."

  TARBALL="$BIN_NAME-${TAG#v}-$ARCH.tar.gz"
  URL="https://github.com/$REPO/releases/download/$TAG/$TARBALL"

  say "Installing Nimbus $TAG for $ARCH."
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT

  curl -fsSL "$URL" -o "$TMP/$TARBALL" \
    || die "could not download $URL"

  # Verify against the published checksums. A script piped into bash is exactly
  # the place where "it downloaded something" is not the same as "it downloaded
  # the right thing".
  if curl -fsSL "https://github.com/$REPO/releases/download/$TAG/SHA256SUMS" \
       -o "$TMP/SHA256SUMS" 2>/dev/null; then
    ( cd "$TMP" && grep " $TARBALL\$" SHA256SUMS | sha256sum -c - >/dev/null ) \
      || die "checksum mismatch for $TARBALL — refusing to install it."
    note "checksum verified"
  else
    note "no SHA256SUMS published for $TAG; skipping verification"
  fi

  tar -xzf "$TMP/$TARBALL" -C "$TMP"
  # The tarball unpacks into a versioned directory, as build-release.sh makes it.
  SRC="$TMP/${TARBALL%.tar.gz}"
  [ -f "$SRC/$BIN_NAME" ] || die "unexpected tarball layout: no $BIN_NAME in ${TARBALL%.tar.gz}/"

  install -Dm755 "$SRC/$BIN_NAME" "$PREFIX/bin/$BIN_NAME"
  install -Dm644 "$SRC/$BIN_NAME.service" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/systemd/user/$BIN_NAME.service"
  # The unit ships with an absolute /usr/bin path for the packaged install.
  sed -i "s|/usr/bin/$BIN_NAME|$PREFIX/bin/$BIN_NAME|" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/systemd/user/$BIN_NAME.service"

  if [ -d "$SRC/omarchy" ]; then
    mkdir -p "$PREFIX/share/nimbus-wayland"
    cp -r "$SRC/omarchy" "$PREFIX/share/nimbus-wayland/"
  fi
  [ -f "$SRC/config.example.json" ] \
    && install -Dm644 "$SRC/config.example.json" \
       "$PREFIX/share/nimbus-wayland/config.example.json"

  note "installed to $PREFIX/bin/$BIN_NAME"

  case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *) say "Note: $PREFIX/bin is not on your PATH."
       note "Add it in your shell's rc file, or use the full path." ;;
  esac
fi

# ---------------------------------------------------------------------------
# Start it, and keep it starting.
# ---------------------------------------------------------------------------

echo
say "Starting Nimbus."
# The binary knows how to do this: it installs a unit naming its own path if the
# install did not ship one, which is what happens for a --user install into
# ~/.local/bin where the packaged unit's /usr/bin path would point at nothing.
if "$PREFIX/bin/$BIN_NAME" autostart on 2>/dev/null || nimbus-wayland autostart on 2>/dev/null; then
  note "running, and will start at every login"
else
  note "could not set it to start at login; starting it for this session only"
  "$PREFIX/bin/$BIN_NAME" start 2>/dev/null || nimbus-wayland start 2>/dev/null || true
fi

# ---------------------------------------------------------------------------
# The bar control, which no package may install for you.
# ---------------------------------------------------------------------------

if command -v omarchy >/dev/null 2>&1; then
  echo
  say "Adding the bar control."
  if NIMBUS_SHARE="$PREFIX/share/nimbus-wayland" \
     "$PREFIX/bin/$BIN_NAME" install-bar 2>/dev/null \
     || nimbus-wayland install-bar 2>/dev/null; then
    :
  else
    note "could not add it automatically; run: nimbus-wayland install-bar"
  fi
fi

echo
say "Nimbus is installed."
# Queried, not asserted: whatever happened above, this reports what you actually
# have. An install that ends by telling you what to *do* leaves anyone who does
# not do it with no idea what state they are in.
"$PREFIX/bin/$BIN_NAME" setup 2>/dev/null || nimbus-wayland setup 2>/dev/null || true

cat <<'DONE'
  Click between windows and watch the ring follow.

  Every setting is also a command:

      nimbus-wayland color Aurora
      nimbus-wayland set idle_intensity 0.5
      nimbus-wayland --help

  To stop it:            nimbus-wayland quit
  To stop it starting:   nimbus-wayland autostart off

DONE
