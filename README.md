# Nimbus for Wayland

The Linux and Wayland version of Nimbus. The macOS one is at
[vivosAi/nimbus](https://github.com/vivosAi/nimbus).

Draws an animated ring of light around the window that has keyboard focus, so
you never type into the wrong one. You can always see where your pointer is;
you cannot see where your keyboard is, and on a large screen you end up typing
into the wrong window.

The same shader as the Mac, ported from the Metal original, drawn on a
`wlr-layer-shell` surface. Runs on Hyprland today.

https://github.com/user-attachments/assets/6b9e4b62-aeab-4b84-a1a5-659871354e7c

https://github.com/user-attachments/assets/db8752e8-a456-484a-8d6b-23c692101b00


## Install

```sh
curl -fsSL https://raw.githubusercontent.com/vivosAi/nimbus-wayland/main/packaging/install.sh | bash
```

That leaves you with the ring running, set to start at every login, and the
control panel in your bar. There is nothing to double-click, because there is
no window — Nimbus has no interface of its own, only the ring.

**AUR package: not published yet.** Arch Linux disabled all pushes to the AUR
in August 2026 while it works through an ongoing supply-chain attack — over
1,500 packages compromised platform-wide. That is not specific to this
project; nobody can publish a new AUR package right now. Once it lifts, `yay
-S nimbus-wayland-bin` will be the fast path here, and the installer above
will defer to it automatically the way it already does for every other
Arch package. Until then, the curl installer above works on Arch too, or
build straight from source:

```sh
git clone https://github.com/vivosAi/nimbus-wayland && cd nimbus-wayland/overlay
cargo build --release
```

A package installs files and nothing else — Arch never enables a service for
you — so turning it on is a separate step:

```sh
nimbus-wayland autostart on      # starts it now, and at every login
nimbus-wayland install-bar       # adds the control to the Omarchy bar
```

At any point, `nimbus-wayland setup` says what you actually have:

```
  Running now:      yes
  Starts at login:  yes
  Bar control:      yes
```

Every setting the panel offers is also a command:

```sh
nimbus-wayland start               # start it now
nimbus-wayland autostart on        # and at every session
nimbus-wayland color Aurora
nimbus-wayland set idle_intensity 0.5
nimbus-wayland --help
```

Settings live in `~/.config/nimbus/config.json`; a commented example is in
[config.example.json](config.example.json).

### From source

```sh
git clone https://github.com/vivosAi/nimbus-wayland
cd nimbus-wayland/overlay
cargo build --release
```

### Requirements

A Wayland compositor that offers `wlr-layer-shell` and OpenGL ES 3.0. In
practice that means **Hyprland 0.50 or newer**: Hyprland has required GLES 3.0
since that release, and so does this, so if the compositor runs then so does
Nimbus. Verified on 0.56.

Focus tracking speaks Hyprland's IPC. Sway and Niri offer the same layer-shell
protocol and need only a focus backend adding; the trait they would implement
is already there.

### Following a window that moves

Hyprland has no IPC event for a window being dragged or resized, so while a
ring is on screen Nimbus asks the compositor where the window is four times a
second — one small socket read each — and sixteen times a second once it sees
the window changing, so the ring lands the moment the window stops. Nothing
runs when there is no ring: nothing focused, fullscreen, excluded, switched off,
or you have walked away. With `hide_while_dragging` on, the ring stands down
while the window moves and returns a fifth of a second after it comes to rest.

## The control panel

Click the mark in the bar. Every setting is a row, and the ten palettes sit in
a grid showing their own colours rather than their names. Details in
[omarchy/README.md](omarchy/README.md).

## What the ring actually is

A signed distance field band straddling the window edge, with an outer bloom
and domain-warped turbulence, so the light moves *within* the ring rather than
the whole ring rotating. It flares brighter for a couple of seconds on a focus
change and settles back, and the palette rotates every half hour, because
anything constant fades from awareness.

The shader is a direct port of `Shaders.metal` from the macOS project — the one
the app ships, not the simplified one on its demo page. Same proportions, same
noise, same premultiplied output, same four-quad vertex stage that shades only
the band and never the window's interior.

Hyprland cannot express any of that in configuration. It needs a client drawing
its own surface, which is what [SPEC.md](SPEC.md) describes and what
[overlay/](overlay/) implements.

## Why not a Hyprland plugin

The plugin ABI is unstable and breaks on compositor releases, so a plugin needs
maintaining forever. A layer-shell client depends only on a stable protocol and
works unchanged on any compositor that speaks it, so Sway and Niri come nearly
free.

## Contributing

The spec is deliberately complete enough to implement from without having seen
the macOS version. If you want to build it, that is the document to read, and
sections 9 and 10 are the ones carrying hard-won detail rather than design
opinion.

## Licence

MIT, matching the macOS project.
