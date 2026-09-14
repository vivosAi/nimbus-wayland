# Nimbus in the Omarchy bar

A Quickshell bar widget: the same menu the macOS app puts in the menu bar, in
the place Omarchy keeps such things.

```sh
cp -r omarchy/nimbus.ring ~/.config/omarchy/plugins/
omarchy bar put nimbus.ring --section right
omarchy restart shell
```

Click the mark in the bar to open it. Every setting is a row: click to step it
forward, right-click to step back, arrow keys and Enter work too. Right-clicking
the bar icon toggles the ring without opening anything.

The ten palettes sit in a grid under the Colour row, each tile showing that
palette's own two band colours with its glow beneath — the names are evocative
but nobody can recall what "Toxic" looks like, and the point of choosing a
colour is seeing it. Click one to use it; the name of whichever you are pointing
at appears on the Colour row. Right-click takes a palette out of the rotation
without switching to it, which is the macOS menu's separate "In rotation…"
submenu folded onto the same tile.

The bar mark is drawn, not a font glyph: it is a port of
`StatusItemIcon.swift` from the macOS project, a window outline inside a broken
ring, with every proportion unchanged.

It owns no state: every value is read back from `nimbus-wayland status` and
every change goes out as `nimbus-wayland set`, so the panel and the ring cannot
disagree about what is switched on. Changes are saved to
`~/.config/nimbus/config.json`, leaving any other keys in that file untouched.

If Nimbus is not running, the panel says so and tells you how to start it
rather than showing stale settings.

## Without the bar

Everything the panel does is a CLI subcommand:

```sh
nimbus-wayland status              # everything, as JSON
nimbus-wayland toggle              # ring on or off
nimbus-wayland set frame_rate 20   # live, and saved
nimbus-wayland next-color
nimbus-wayland --help
```
