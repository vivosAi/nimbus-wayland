#version 300 es
// Ring fragment stage, GLSL ES 3.00.
//
// A direct port of `ring_fragment` in the macOS project's
// Sources/Nimbus/Render/Shaders.metal — the shader the app actually ships,
// which draws the ring and nothing else and outputs premultiplied alpha.
//
// It is deliberately NOT ported from docs/ring.js. That file is the web demo:
// to show a ring in a browser it has to invent a desktop to put the ring
// around, so it also draws two mock monitors, four mock windows and their
// shadows, and composites everything itself into an opaque frame. Porting from
// it drags all of that into an overlay that must be transparent, and loses the
// `discard`s and the parameterised fBm along the way.
//
// Blend state on the GL side must match the macOS pipeline:
//     glBlendFuncSeparate(GL_ONE, GL_ONE_MINUS_SRC_ALPHA,
//                         GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
//
// Two rules carried over from the macOS version, both of which were real bugs on
// macOS before they were comments: send an accumulated phase rather than a
// timestamp, and clamp the per-frame delta.

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

in vec2 vPixel;
out vec4 fragColor;

// ---------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------

float hash21(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

float valueNoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 w = f * f * (3.0 - 2.0 * f);            // smoothstep interpolation
    float a = hash21(i);
    float b = hash21(i + vec2(1.0, 0.0));
    float c = hash21(i + vec2(0.0, 1.0));
    float d = hash21(i + vec2(1.0, 1.0));
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

// fBm normalized to roughly 0…1. Octaves are per-call: the turbulence layers
// only need two, and paying for three everywhere is wasted on an iGPU.
//
// GLSL ES 1.00 required constant loop bounds, which is why the WebGL demo had
// to hand-unroll this into separate fbm2/fbm3 functions. ES 3.00 does not, so
// this is the Metal original one-for-one rather than two copies to keep in
// step.
float fbm(vec2 p, int octaves) {
    float sum = 0.0;
    float amp = 0.5;
    float norm = 0.0;
    for (int i = 0; i < octaves; ++i) {
        sum += amp * valueNoise(p);
        norm += amp;
        p *= 2.0;
        amp *= 0.5;
    }
    return sum / max(norm, 1e-5);
}

// Signed distance to a rounded rectangle. Negative inside the window.
float sdRoundBox(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

// ---------------------------------------------------------------------------
// Fragment
// ---------------------------------------------------------------------------

void main() {
    // Debug mode 1+: paint every rasterized fragment opaque blue. Combined with
    // mode 2 this separates "the surface is not compositing" from "the ring
    // geometry or shading produces nothing".
    if (pad0.x >= 1.0) {
        fragColor = vec4(0.0, 0.35, 1.0, 1.0);
        return;
    }

    float cornerRadius = params0.x;
    float bandInner    = params0.y;
    float bandOuter    = params0.z;
    // Phases, not times. The host integrates speed over elapsed time and sends
    // the accumulated angle, because multiplying an absolute timestamp by a
    // *changing* speed makes the phase leap by hundreds of radians the moment
    // the speed changes — which is what a flare does. Integrating keeps the
    // motion continuous through every speed change.
    float flowPhase    = params0.w;
    float intensity    = params1.x;
    float warpPhase    = params1.y;
    float noiseScale   = params1.z;
    float glowFalloff  = params1.w;

    vec2 halfSize = windowRect.zw * 0.5;
    vec2 center   = windowRect.xy + halfSize;
    vec2 p        = vPixel - center;

    float d = sdRoundBox(p, halfSize,
                         min(cornerRadius, min(halfSize.x, halfSize.y)));

    // Reject before touching any noise. Everything below costs real ALU.
    if (d > bandOuter * 1.5 + glowFalloff * 5.0 || d < -bandInner * 1.5) {
        discard;
    }

    // Position around the perimeter. Dividing by the half-extents before atan
    // maps the rectangle onto a circle, so the motion travels at an even rate
    // on a wide window instead of bunching up at the short edges. One extra
    // divide versus a plain atan, and it is the difference between a 1920x200
    // terminal looking right and looking lopsided.
    float theta = atan(p.y / max(halfSize.y, 1.0),
                       p.x / max(halfSize.x, 1.0));

    // Sampling on a unit circle rather than on a 0…1 coordinate means the noise
    // has no seam where the perimeter wraps. Every term below is either a
    // function of this point or constant in theta, so seamlessness survives.
    vec2 ring = vec2(cos(theta), sin(theta));

    // Domain warping: noise used to displace the lookup of more noise. This is
    // what separates fire from a moving highlight — it produces curling,
    // folding structure instead of a rigid pattern sliding past. Drifting the
    // warp on its own clock also means the ring keeps changing everywhere at
    // once, not only where the rotation currently is.
    vec2 q = ring * noiseScale + vec2(0.0, warpPhase);
    vec2 warp = vec2(fbm(q, 2), fbm(q + vec2(5.2, 1.3), 2));

    // Large, slow tongues traveling one way...
    float tongues = fbm(ring * noiseScale * 1.5
                        + warp * 1.15
                        + vec2(flowPhase, -flowPhase * 0.55), 3);
    // ...and finer, faster detail traveling the other, so the eye never
    // resolves it into a single repeating loop.
    float detail = fbm(ring * noiseScale * 3.5
                       - vec2(flowPhase * 1.6, warpPhase * 1.65), 2);

    float n = clamp(0.68 * tongues + 0.42 * detail, 0.0, 1.0);
    // Widen the dynamic range. Without this the whole ring sits in a narrow
    // band of brightness and reads as static.
    n = smoothstep(0.12, 0.88, n);

    // The band's outer edge and the bloom's reach both move with the noise.
    // A ring whose *shape* changes reads as alive; one whose brightness alone
    // changes reads as a light with a fault.
    float outerLocal = bandOuter * (0.45 + 1.05 * n);
    float glowLocal  = glowFalloff * (0.55 + 0.85 * n);

    // A soft strip straddling the window edge. Both edges are feathered by at
    // least 1.5px (enforced host-side) so there is no aliasing on the ring.
    float band = smoothstep(outerLocal, 0.0, d) * smoothstep(-bandInner, 0.0, d);
    float glow = exp(-max(d, 0.0) / max(glowLocal, 0.5));

    if (band <= 0.0005 && glow <= 0.0025) {
        discard;
    }

    // 0 at the inner edge of the band, 1 at the outer: lets the color cool as
    // it reaches away from the window, the way a flame does.
    float across = clamp((d + bandInner) / max(bandInner + outerLocal, 1.0),
                         0.0, 1.0);

    vec3 core       = mix(colorB.rgb, colorA.rgb, n);
    vec3 bandColour = mix(core, colorGlow.rgb, across * 0.55);

    // Keep a floor under the band so the ring is never fully dark anywhere
    // along its length — dark gaps read as a broken ring rather than as motion.
    float bandAlpha = band * (0.30 + 0.70 * n);
    float glowAlpha = glow * 0.32;

    float alpha = clamp((bandAlpha + glowAlpha) * intensity, 0.0, 1.0);

    // Premultiplied output, matching the pipeline's blend state. Letting the
    // color exceed the alpha makes the bloom read as light being added rather
    // than as a gray film over whatever is behind it.
    vec3 premultiplied = bandColour * (bandAlpha * intensity)
                       + colorGlow.rgb * (glowAlpha * intensity);

    fragColor = vec4(premultiplied, alpha);
}
