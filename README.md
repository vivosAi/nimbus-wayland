# Nimbus for Wayland

Draws an animated ring of light around the window that has keyboard focus, so
you never type into the wrong one.

A port of [Nimbus for macOS](https://github.com/vivosAi/nimbus), which is
shipped and working. This is not yet.

## Status

**Configuration only, working today.** Ten lines of Hyprland config plus a
script that rotates the color. See below.

**The overlay, partly written.** The logic is done and tested: palettes, the
flare curve, rotation, Hyprland event parsing, phase integration and coordinate
conversion. 29 tests, all passing.

The Wayland and GL layer is not written. It is gated to Linux in `Cargo.toml`,
so everything above it builds and tests on any machine, which is how the logic
got written without a Linux box to hand. Milestones are in
[SPEC.md](SPEC.md) §12.

## Try the config version first

It may be enough, and it takes five minutes to find out.

```sh
cp hyprland/nimbus.conf ~/.config/hypr/nimbus.conf
echo 'source = ~/.config/hypr/nimbus.conf' >> ~/.config/hypr/hyprland.conf
hyprctl reload
```

Focused windows get a border whose gradient rotates continuously. The motion is
the point: your eye stops seeing what never changes, so a static border works
for a week and is invisible by the end of the month.

To rotate the color as well, for the same reason on a longer timescale:

```sh
install -Dm755 hyprland/nimbus-rotate-color.sh ~/.local/bin/nimbus-rotate-color
echo 'exec-once = ~/.local/bin/nimbus-rotate-color --watch' >> ~/.config/hypr/hyprland.conf
```

Every 30 minutes it picks a new palette, never the current one and never one of
the last three. Ten palettes, the same ones the macOS app ships with.

## What the overlay would add

The config version is a rotating gradient. The macOS app draws a signed
distance field band with an outer bloom and domain-warped turbulence, so the
light moves *within* the ring rather than the whole ring rotating. It also
flares brighter for a couple of seconds on a focus change and settles back.

Hyprland cannot express that in config. It needs a client drawing its own
surface, which is what [SPEC.md](SPEC.md) describes.

## Why not a Hyprland plugin

The plugin ABI is unstable and breaks on compositor releases, so a plugin needs
maintaining forever. A layer-shell client depends only on a stable protocol and
works unchanged on other wlroots compositors, so Sway and Niri come nearly free.

## Contributing

The spec is deliberately complete enough to implement from without having seen
the macOS version. If you want to build it, that is the document to read, and
sections 9 and 10 are the ones carrying hard-won detail rather than design
opinion.

## Licence

MIT, matching the macOS project.
