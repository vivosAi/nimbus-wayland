// Ring fragment shader, GLSL ES 1.00.
//
// Lifted from the macOS project's demo page (docs/ring.js), which is itself a
// port of Sources/Nimbus/Render/Shaders.metal. Three copies of this exist and
// they are expected to drift; none of them is generated from the others.
//
// Uniforms are documented in SPEC.md section 9. Two rules matter: send an
// accumulated phase rather than a timestamp, and clamp the per-frame delta.
precision highp float;

uniform vec2  uResolution;
uniform vec4  uWin0;         // x, y, w, h  (px, y up)
uniform vec4  uWin1;
uniform vec4  uWin2;
uniform vec4  uWin3;
uniform vec4  uScreen0;      // the two displays
uniform vec4  uScreen1;
uniform float uFocusIndex;   // 0..3
uniform float uCornerRadius;
uniform float uBandInner;
uniform float uBandOuter;
uniform float uFlowPhase;    // integrated, never time * speed
uniform float uWarpPhase;
uniform float uIntensity;
uniform float uNoiseScale;
uniform float uGlowFalloff;
uniform vec3  uColorA;
uniform vec3  uColorB;
uniform vec3  uColorGlow;

float sdRoundBox(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

float hash21(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

float valueNoise(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 w = f * f * (3.0 - 2.0 * f);
    float a = hash21(i);
    float b = hash21(i + vec2(1.0, 0.0));
    float c = hash21(i + vec2(0.0, 1.0));
    float d = hash21(i + vec2(1.0, 1.0));
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

// Fixed octave counts rather than a loop bound, so this compiles on WebGL 1
// where loop bounds must be constant.
float fbm2(vec2 p) {
    return (0.5 * valueNoise(p) + 0.25 * valueNoise(p * 2.0)) / 0.75;
}
float fbm3(vec2 p) {
    return (0.5 * valueNoise(p) + 0.25 * valueNoise(p * 2.0)
          + 0.125 * valueNoise(p * 4.0)) / 0.875;
}

// The space the monitors sit in.
vec3 ambient(vec2 uv) {
    float v = 1.0 - 0.7 * length(uv - vec2(0.5, 0.5));
    return mix(vec3(0.012, 0.014, 0.022), vec3(0.028, 0.031, 0.044), v);
}

// Indexed with if/else rather than a uniform array: GLSL ES 1.00 restricts
// dynamic indexing, and a few branches are clearer than fighting it.
vec4 rectFor(float i) {
    if (i < 0.5) return uWin0;
    if (i < 1.5) return uWin1;
    if (i < 2.5) return uWin2;
    return uWin3;
}

// A display: the desktop surface plus a thin bezel edge, so two of them read as
// two monitors rather than as one wide canvas. Multi-monitor is where losing
// track of focus actually hurts, so the demo should look like it.
vec4 screenPanel(vec2 p, vec4 rect) {
    vec2 halfSize = rect.zw * 0.5;
    vec2 c = rect.xy + halfSize;
    float d = sdRoundBox(p - c, halfSize, 6.0);
    if (d > 3.0) return vec4(0.0);

    vec2 uv = (p - rect.xy) / rect.zw;
    float v = 1.0 - 0.5 * length(uv - vec2(0.5, 0.58));
    vec3 col = mix(vec3(0.047, 0.054, 0.078), vec3(0.078, 0.086, 0.121), v);
    // Bezel: a bright hairline at the very edge.
    col = mix(col, vec3(0.20, 0.22, 0.28), smoothstep(-2.0, -0.2, d));
    return vec4(col, 1.0 - smoothstep(-0.5, 1.0, d));
}

// Real macOS windows cast a soft shadow, and its absence is most of why a
// mock-up looks flat. The focused window gets a deeper one, as it does on a
// real desktop.
float windowShadow(vec2 p, vec4 rect, float focused) {
    vec2 halfSize = rect.zw * 0.5;
    vec2 c = rect.xy + halfSize - vec2(0.0, rect.w * 0.015);
    float d = sdRoundBox(p - c, halfSize, uCornerRadius + 3.0);
    float spread = rect.w * (0.045 + 0.035 * focused);
    return exp(-max(d, 0.0) / spread) * (0.45 + 0.25 * focused);
}

// A window: body, title bar, traffic lights.
vec4 windowLayer(vec2 p, vec4 rect, float focused) {
    vec2 halfSize = rect.zw * 0.5;
    vec2 c = rect.xy + halfSize;
    float d = sdRoundBox(p - c, halfSize, uCornerRadius);
    if (d > 1.0) return vec4(0.0);

    float inside = 1.0 - smoothstep(-1.0, 1.0, d);

    float barH = rect.w * 0.10;
    float inBar = step(rect.y + rect.w - barH, p.y) * step(p.y, rect.y + rect.w);
    vec3 body = mix(vec3(0.043, 0.051, 0.078), vec3(0.094, 0.102, 0.133), inBar);

    // Unfocused windows sit back a little, the way macOS dims them — which is
    // the very cue that is too subtle to rely on, and the reason this app exists.
    body *= mix(0.82, 1.0, focused);

    for (int i = 0; i < 3; i++) {
        vec2 dotp = vec2(rect.x + barH * (0.9 + float(i) * 0.85),
                         rect.y + rect.w - barH * 0.5);
        float dd = length(p - dotp) - barH * 0.16;
        body = mix(body, vec3(0.45), (1.0 - smoothstep(-0.6, 0.6, dd)) * 0.7);
    }

    // Text-like lines, so it reads as a window with content in it.
    for (int i = 0; i < 5; i++) {
        float ly = rect.y + rect.w - barH * 2.2 - float(i) * rect.w * 0.085;
        float lw = rect.z * (0.62 - mod(float(i) * 0.17, 0.34));
        float inLine = step(rect.x + rect.z * 0.08, p.x) * step(p.x, rect.x + rect.z * 0.08 + lw)
                     * step(ly - rect.w * 0.018, p.y) * step(p.y, ly + rect.w * 0.018);
        body = mix(body, vec3(0.22, 0.24, 0.30), inLine * 0.9);
    }

    return vec4(body, inside);
}

// The ring itself — the part ported from Shaders.metal.
vec4 ring(vec2 p, vec4 rect) {
    vec2 halfSize = rect.zw * 0.5;
    vec2 c = rect.xy + halfSize;
    vec2 q = p - c;

    float d = sdRoundBox(q, halfSize, uCornerRadius);
    if (d > uBandOuter * 1.5 + uGlowFalloff * 5.0 || d < -uBandInner * 1.5) {
        return vec4(0.0);
    }

    float theta = atan(q.y / max(halfSize.y, 1.0), q.x / max(halfSize.x, 1.0));
    vec2 r = vec2(cos(theta), sin(theta));

    vec2 wq = r * uNoiseScale + vec2(0.0, uWarpPhase);
    vec2 warp = vec2(fbm2(wq), fbm2(wq + vec2(5.2, 1.3)));

    float tongues = fbm3(r * uNoiseScale * 1.5 + warp * 1.15
                         + vec2(uFlowPhase, -uFlowPhase * 0.55));
    float detail = fbm2(r * uNoiseScale * 3.5
                        - vec2(uFlowPhase * 1.6, uWarpPhase * 1.65));

    float n = clamp(0.68 * tongues + 0.42 * detail, 0.0, 1.0);
    n = smoothstep(0.12, 0.88, n);

    float outerLocal = uBandOuter * (0.45 + 1.05 * n);
    float glowLocal  = uGlowFalloff * (0.55 + 0.85 * n);

    float band = smoothstep(outerLocal, 0.0, d) * smoothstep(-uBandInner, 0.0, d);
    float glow = exp(-max(d, 0.0) / max(glowLocal, 0.5));

    float across = clamp((d + uBandInner) / max(uBandInner + outerLocal, 1.0), 0.0, 1.0);
    vec3 core = mix(uColorB, uColorA, n);
    vec3 color = mix(core, uColorGlow, across * 0.55);

    float bandAlpha = band * (0.30 + 0.70 * n);
    float glowAlpha = glow * 0.32;
    float alpha = clamp((bandAlpha + glowAlpha) * uIntensity, 0.0, 1.0);

    vec3 premul = color * (bandAlpha * uIntensity) + uColorGlow * (glowAlpha * uIntensity);
    return vec4(premul, alpha);
}

void main() {
    vec2 p = gl_FragCoord.xy;
    vec3 col = ambient(p / uResolution);

    vec4 s0 = screenPanel(p, uScreen0);
    col = mix(col, s0.rgb, s0.a);
    vec4 s1 = screenPanel(p, uScreen1);
    col = mix(col, s1.rgb, s1.a);

    // Index order is stacking order, back to front. Each window is preceded by
    // its own shadow so it falls on what is behind it, not on itself.
    for (int i = 0; i < 4; i++) {
        float fi = float(i);
        if (abs(fi - uFocusIndex) < 0.5) continue;
        vec4 r = rectFor(fi);
        col *= 1.0 - windowShadow(p, r, 0.0);
        vec4 w = windowLayer(p, r, 0.0);
        col = mix(col, w.rgb, w.a);
    }

    vec4 fr = rectFor(uFocusIndex);
    col *= 1.0 - windowShadow(p, fr, 1.0);
    vec4 focused = windowLayer(p, fr, 1.0);
    col = mix(col, focused.rgb, focused.a);

    vec4 r = ring(p, rectFor(uFocusIndex));
    col = col * (1.0 - r.a) + r.rgb;

    gl_FragColor = vec4(col, 1.0);
}
