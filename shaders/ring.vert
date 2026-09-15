#version 300 es
// Ring vertex stage, GLSL ES 3.00.
//
// A direct port of `ring_vertex` in the macOS project's
// Sources/Nimbus/Render/Shaders.metal. Not derived from docs/ring.js: that is
// the web demo, which draws a whole mock desktop into a full-canvas quad and
// therefore has no equivalent of this stage at all.
//
// Draw 24 vertices, no vertex buffer, no attributes:
//
//     glDrawArrays(GL_TRIANGLES, 0, 24);
//
// Four quads (top, bottom, left, right) tile the band with no overlap and no
// seam, so the window's interior is never rasterized. On macOS this was an
// optimization for integrated graphics. Here it matters more: the layer-shell
// surface spans the whole output, so a full-screen quad would shade every pixel
// of the display to light a ring around one window.

precision highp float;

layout(std140) uniform Uniforms {
    vec2 resolution;    //  0  drawable size in physical pixels
    vec2 pad0;          //  8  .x carries the debug mode (0 = off)
    vec4 windowRect;    // 16  x, y, w, h in pixels, bottom-left origin
    vec4 colorA;        // 32  linear RGB in .xyz
    vec4 colorB;        // 48
    vec4 colorGlow;     // 64
    vec4 params0;       // 80  cornerRadius, bandInner, bandOuter, flowPhase
    vec4 params1;       // 96  intensity, warpPhase, noiseScale, glowFalloff
};                      // 112 — std140 reproduces the Metal layout exactly

// Our own pixel space: bottom-left origin, matching the coordinates the host
// converts window rects into.
out vec2 vPixel;

const vec2 kOffsets[6] = vec2[6](
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0)
);

void main() {
    // Debug mode 2: one full-viewport quad, to test the surface and its
    // compositing in isolation from any of the ring geometry. This is the
    // diagnostic for what was the worst bug in the macOS
    // version — every frame rendering correctly and compositing to nothing,
    // with no error anywhere. A wrong Wayland scale factor fails the same way.
    if (pad0.x >= 2.0) {
        vec2 o = kOffsets[min(gl_VertexID, 5)];
        gl_Position = vec4(o.x * 2.0 - 1.0, o.y * 2.0 - 1.0, 0.0, 1.0);
        vPixel = o * resolution;
        // Collapse the remaining vertices so they rasterize nothing.
        if (gl_VertexID >= 6) {
            gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
        }
        return;
    }

    float bandInner   = params0.y;
    float bandOuter   = params0.z;
    float glowFalloff = params1.w;

    // Past ~5 falloff lengths the bloom is under 1% and invisible; beyond that
    // we would be shading pixels that contribute nothing. The 1.5x allows for
    // the band's outer edge moving outward (the flame tongues) without the
    // strip clipping them into a straight line.
    float outerExtent = bandOuter * 1.5 + glowFalloff * 5.0;

    vec2 wpos  = windowRect.xy;
    vec2 wsize = windowRect.zw;

    // Outer bounds, clipped to the drawable.
    float ox0 = max(wpos.x - outerExtent, 0.0);
    float oy0 = max(wpos.y - outerExtent, 0.0);
    float ox1 = min(wpos.x + wsize.x + outerExtent, resolution.x);
    float oy1 = min(wpos.y + wsize.y + outerExtent, resolution.y);

    // Inner bounds: everything strictly inside this is fully transparent.
    // Band widths are clamped host-side to min(w,h)/4, so this cannot invert.
    float ix0 = clamp(wpos.x + bandInner,           ox0, ox1);
    float iy0 = clamp(wpos.y + bandInner,           oy0, oy1);
    float ix1 = clamp(wpos.x + wsize.x - bandInner, ox0, ox1);
    float iy1 = clamp(wpos.y + wsize.y - bandInner, oy0, oy1);

    int quad   = gl_VertexID / 6;
    int corner = gl_VertexID % 6;

    vec2 lo, hi;
    if (quad == 0) {          // top strip, full width
        lo = vec2(ox0, iy1); hi = vec2(ox1, oy1);
    } else if (quad == 1) {   // bottom strip, full width
        lo = vec2(ox0, oy0); hi = vec2(ox1, iy0);
    } else if (quad == 2) {   // left strip, between the two
        lo = vec2(ox0, iy0); hi = vec2(ix0, iy1);
    } else {                  // right strip
        lo = vec2(ix1, iy0); hi = vec2(ox1, iy1);
    }

    vec2 pixel = mix(lo, hi, kOffsets[corner]);

    // Bottom-left-origin pixels to NDC, which has y up.
    gl_Position = vec4(pixel.x / resolution.x * 2.0 - 1.0,
                       pixel.y / resolution.y * 2.0 - 1.0,
                       0.0, 1.0);
    vPixel = pixel;
}
