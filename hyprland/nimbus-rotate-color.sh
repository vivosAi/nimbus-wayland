#!/usr/bin/env bash
# Rotate the focused-window border color.
#
# Anything constant fades from awareness, so a single color eventually stops
# being seen no matter how bright it is. Rotating defeats that.
#
#   nimbus-rotate-color.sh          rotate once
#   nimbus-rotate-color.sh --watch  rotate every 30 minutes
#
# Run it from a systemd timer, or with --watch from exec-once in hyprland.conf.

set -euo pipefail

STATE="${XDG_STATE_HOME:-$HOME/.local/state}/nimbus"
RECENT="$STATE/recent"
INTERVAL="${NIMBUS_INTERVAL:-1800}"
ANGLE="${NIMBUS_ANGLE:-45deg}"

# Name, then the three colors: two for the gradient and one for the shadow.
# The same ten the macOS app ships with.
PALETTES=(
  "Ember|FF3D00|FFC400|FF6D00"
  "Plasma|7C4DFF|00E5FF|536DFE"
  "Toxic|76FF03|00E676|B2FF59"
  "Magma|D50000|FF6E40|FF1744"
  "Ice|18FFFF|82B1FF|40C4FF"
  "NeonRose|FF4081|F50057|FF80AB"
  "Solar|FFD600|FFAB00|FFEA00"
  "Aurora|00E676|00B0FF|1DE9B6"
  "Ultraviolet|E040FB|651FFF|AA00FF"
  "Copper|FF9100|FFD180|FF6D00"
)

# How many recent palettes may not be reused. With ten to choose from, avoiding
# the last three keeps the sequence from feeling like it has favourites.
MEMORY=3

pick() {
  mkdir -p "$STATE"
  touch "$RECENT"
  local recent candidates
  recent="$(tail -n "$MEMORY" "$RECENT" 2>/dev/null || true)"

  candidates=()
  for entry in "${PALETTES[@]}"; do
    local name="${entry%%|*}"
    grep -qx "$name" <<<"$recent" || candidates+=("$entry")
  done
  # If the recency rule excludes everything, relax it rather than getting stuck.
  [ ${#candidates[@]} -eq 0 ] && candidates=("${PALETTES[@]}")

  local chosen="${candidates[RANDOM % ${#candidates[@]}]}"
  local name="${chosen%%|*}"
  echo "$name" >> "$RECENT"
  # Keep the file from growing without bound.
  tail -n "$MEMORY" "$RECENT" > "$RECENT.tmp" && mv "$RECENT.tmp" "$RECENT"
  echo "$chosen"
}

apply() {
  local entry="$1"
  IFS='|' read -r name a b glow <<<"$entry"
  hyprctl keyword general:col.active_border \
    "rgba(${a}ee) rgba(${b}ee) rgba(${glow}ee) $ANGLE" >/dev/null
  hyprctl keyword decoration:shadow:color "rgba(${glow}55)" >/dev/null 2>&1 || true
  echo "nimbus: $name"
}

if [ "${1:-}" = "--watch" ]; then
  while true; do
    apply "$(pick)"
    sleep "$INTERVAL"
  done
else
  apply "$(pick)"
fi
