# Nimbus for Wayland — implementation spec

A port of [Nimbus](https://github.com/vivosAi/nimbus) to Wayland compositors,
primarily Hyprland. Draws an animated ring of light around the window that has
keyboard focus.

**Target reader:** a coding agent or a contributor picking this up cold.

---

## 1. Problem

The same one the macOS version solves. You can always see where your pointer is.
You cannot see where your keyboard is. On a large display, or several, the
compositor's focus indicator is easy to miss, so you type into the wrong window.

The fix is an unmissable, continuously animated marker around the focused window.

## 2. What is different from macOS, and why it matters

Most of the macOS app exists to work around the platform. Almost none of that
work is needed here.

| Problem on macOS | On Wayland |
|---|---|
| Finding the focused window needs the Accessibility API, a permission prompt, per-process observers and a messaging timeout | The compositor knows. Subscribe to its IPC socket. |
| Window geometry needs AX reads that can hang, plus coordinate conversion between two origin systems | The IPC event carries position and size in one coordinate space |
| Drawing requires a borderless always-on-top NSWindow positioned by hand | `wlr-layer-shell` puts a surface on the overlay layer |
| Click-through needs `ignoresMouseEvents` | Set an empty input region |
| Notifications can be dropped, so a re-sync timer is needed | Events are ordered and reliable on a socket |

**The consequence: this should be a smaller and more reliable program than the
macOS one.** If it is turning out larger, something has gone wrong.

What is genuinely harder here: there is no single window that can move between
monitors, so output handling needs more care. See §6.

## 3. Two levels of solution

Ship both. They serve different users and the first costs almost nothing.

**Level 1, configuration only.** Hyprland already draws an animated gradient
border on the focused window. Roughly ten lines of config, plus a small script
to rotate the palette on a timer. This may be enough, and a user should find
that out before installing anything.

**Level 2, the overlay.** A layer-shell surface rendering the real shader:
signed distance field band, outer bloom, domain-warped turbulence. This is what
the rest of this document specifies.

## 4. Platform and technology

| Choice | Decision | Why |
|---|---|---|
| Protocol | `wlr-layer-shell-unstable-v1` | The only way to put a surface above normal windows on wlroots compositors |
| Language | Rust | Mature Wayland client crates, no manual memory handling around GL contexts |
| Wayland client | `smithay-client-toolkit` | Handles layer-shell, outputs and seats without hand-rolling protocol code |
| Rendering | OpenGL ES 3 via EGL, or `wgpu` | The shader is already GLSL. `wgpu` is pleasanter but pulls in a large dependency for one fragment shader |
| Focus source | Compositor IPC | Hyprland's socket2 to start; abstract behind a trait so Sway and others can be added |

**Do not write a Hyprland plugin.** The plugin ABI is unstable and breaks on
compositor releases, so it would need maintenance forever. A layer-shell client
depends only on a stable protocol and works on other wlroots compositors
unchanged.

**Do not use `decoration:screen_shader`.** It applies a fragment shader to the
whole composited output but has no way to receive the focused window's geometry
as a uniform. Making it work would mean regenerating the shader file and
reloading it on every focus change and every window move.

## 5. Architecture

```
nimbus-wayland/
├── hyprland/            Level 1: config and the palette rotation script
├── shaders/ring.frag    GLSL, copied from the macOS project's demo page
└── overlay/
    ├── src/
    │   ├── main.rs
    │   ├── focus/       compositor IPC, emits FocusState
    │   │   ├── mod.rs        the trait every backend implements
    │   │   └── hyprland.rs   socket2 listener
    │   ├── surface.rs   layer-shell surface, one per output
    │   ├── render.rs    EGL context, shader, uniforms
    │   └── style.rs     palettes, the flare curve, rotation
    └── Cargo.toml
```

Data flow, one direction:

```
FocusBackend --FocusState--> OutputRouter --> Surface(output N) --> Renderer
```

`FocusState` carries what the renderer needs and nothing else:

```rust
struct FocusState {
    address: String,     // compositor's window handle, for identity
    output: String,      // which monitor it is on
    rect: Rect,          // position and size, compositor coordinates
    fullscreen: bool,
}
```

## 6. Output handling

**A Wayland surface belongs to one output.** There is no surface spanning two
monitors, so a single overlay that follows focus across screens, as on macOS, is
not possible.

- Create one layer-shell surface per output at startup, on the `overlay` layer.
- Only the surface on the output holding the focused window renders. Every other
  one stays transparent with its rendering paused, costing nothing.
- Handle outputs appearing and disappearing. A monitor unplugged while holding
  the ring must not leave a surface behind.
- **A window straddling two outputs** is rare when tiling and possible when
  floating. Either render on both surfaces so the ring reads as continuous
  across the bezel, or pick the output holding most of the window and accept
  clipping at the edge. The first looks better and needs the two surfaces to
  agree on phase, so they must share one clock.

Only one window has keyboard focus at a time, so **exactly one ring is ever
visible**, regardless of how many surfaces exist.

## 7. Focus tracking

Hyprland exposes a socket at
`$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket2.sock`, which emits
newline-delimited events. The ones that matter:

- `activewindowv2` — focus changed, carries the window address
- `openwindow`, `closewindow`
- `movewindow`, `resizewindow` (check current event names against the wiki)
- `monitoradded`, `monitorremoved`
- `workspace`, `focusedmon`

Geometry is not in every event, so query it with `hyprctl -j clients` or the
writable socket, and match on the window address.

**Put the backend behind a trait from the start**, even with only one
implementation. Sway's IPC is different in wire format but identical in shape,
and the abstraction is nearly free if it exists on day one and expensive to
retrofit.

Do not add a re-sync timer unless a dropped event is actually observed. The
macOS version needs one because AX notifications go missing. A socket is
ordered and reliable, so polling would be solving a problem that does not exist
here.

## 8. Where the band sits is a setting, not a deduction

The mechanism is unaffected by tiling: the compositor reports a rect whether the
window is tiled or floating.

**The appearance needs a choice.** The macOS ring straddles the window edge,
roughly 6px inside and 18px outside, which looks right when windows float and
overlap. In a tiled layout, neighbouring windows sit a few pixels apart, so that
glow spills onto the window next door. With `gaps_in = 0` it lands on top of it.

Expose it:

```
ring_placement = straddle   # default. 6px inside, 18px outside. The macOS look.
ring_placement = inside     # entirely within the window edge. For tight gaps.
```

**Do not derive this from whether the window is floating.** Two reasons, and the
first matters more:

- **The ring would change shape as you toggle floating.** The whole value of
  this is a marker you read without looking at it. Two shapes means learning
  two markers, and a shape that changes under you is worse than either.
- **Tiled does not mean cramped.** Gaps are configurable. Someone on
  `gaps_in = 20` has room to spare and would resent losing the glow; someone on
  `gaps_in = 0` needs it inside. Whether a window is tiled says nothing about
  how much space is around it, so it is the wrong thing to branch on.

Default to `straddle`, so Wayland looks like the screenshots and the demo page
rather than being a quietly different product. Users with tight gaps switch
once and never think about it again.

`inside` costs a few pixels of window content, which is the trade and should be
said plainly in the README rather than discovered.

## 9. Rendering

The shader exists. `docs/ring.js` in the macOS repository contains a working
GLSL port of the Metal original: the rounded-box SDF, value noise, three-octave
fBm, the domain warp, the band mask, the outer bloom and the premultiplied
output. Copy the fragment shader out of it.

Two things from the macOS version that must be preserved, because both were bugs
that cost real time there:

**Integrate speed into a phase. Never compute `time * speed`.** A flare changes
the speed, and multiplying a large timestamp by a changing multiplier makes the
pattern leap and spin instead of simply moving faster. Accumulate
`phase += dt * speed` per frame and send the phase.

**Clamp the per-frame delta.** Rendering pauses when the ring is hidden. Without
a clamp the first frame back advances the pattern by however long it was away.

Blend state is premultiplied alpha: source factor one, destination factor one
minus source alpha.

**Frame cost dominates.** On the macOS version, CPU was almost exactly linear in
frames per second and near enough independent of what the shader did. Expect the
same. Run at the full rate only while a flare is decaying and halve it once the
ring settles.

## 10. Behavior to copy verbatim

These were all learned the hard way and are not worth rediscovering.

- **The flare fires only on a change to a different window**, not on every
  geometry update. Otherwise dragging or resizing keeps it lit and it never
  settles. Curve: `intensity = idle + (1 - idle) * exp(-3t / duration)`, with
  `duration` 2.5s and `idle` 0.30. Band width and motion speed decay on the same
  curve so the ring relaxes as one thing.
- **Colors rotate to defeat habituation.** Anything constant fades from
  awareness. A static marker works for a week and is invisible by the end of the
  month. Rotate every 30 minutes by default, cross-fading over 3 seconds in
  linear RGB. Never the current palette, never one of the last three.
- **Hide the ring while a window is being dragged or resized**, and restore it
  when the window comes to rest. While dragging you already know which window is
  active, and chasing it looks worse than standing down.
- **No ring when nothing has focus.** Showing it on the previously focused
  window would be a lie: that window will not receive your keystrokes.
- **Idle stops the animation but must not remove the ring.** Walking back to the
  machine and looking at which window has focus, before touching anything, is
  the case this exists for.

## 11. Non-goals

- No window management. Never move, resize or close anything.
- No network access of any kind.
- No X11. A different mechanism entirely, and not worth the complexity.
- No compositor plugin. See §4.

## 12. Milestones

Verify each by hand before starting the next.

**M0.** Layer-shell surface on every output, filled a flat color, click-through
confirmed. *Verify:* clicks pass through everywhere, including where the surface
overlaps a window.

**M1.** Hyprland IPC listener logging focus changes with address, output and
rect. No rendering. *Verify:* logged values match reality when switching
windows, switching workspaces, moving a window between monitors, and toggling
floating.

**M2.** A flat rectangle drawn around the focused window, on the right output,
following it. *Verify:* correct across every M1 scenario, no flicker, and only
one ring with two monitors.

**M3.** The real shader. *Verify:* correct alpha compositing, no dark halo,
sharp on a scaled output, motion smooth and slow.

**M4.** Flare, palette rotation, hide-while-dragging, tiled versus floating band
widths.

**M5.** Config file, packaging, and the level 1 Hyprland config in the same
repository.

## 13. Open questions

- **Fractional scaling.** A wrong scale factor was the worst bug in the macOS
  version: the layer rendered every frame correctly and composited to nothing,
  with no error anywhere. Wayland's `wp_fractional_scale_v1` needs handling
  properly from the start rather than being retrofitted.
- **Which GL path.** `wgpu` is pleasant but heavy for one fragment shader. Raw
  EGL with GLES3 is smaller and more fiddly. Decide at M0, not M3.
- **Does level 1 make level 2 unnecessary?** Genuinely unknown until the config
  version has been used for a few weeks. That is the point of shipping it first.
