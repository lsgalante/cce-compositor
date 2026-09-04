// Window resize-handle frame: a ring inside a rounded rect whose INNER
// edge is a wave of eight hills and eight valleys — a hill in the middle
// of each side and one on each corner, with a valley between every two.
// The corner hills are domes whose apex sits on the corner's 45° diagonal,
// pointing into the window, at the same height as the side hills.
//
// The ring is `band_min` thick at the valleys, which sit `R` (a quarter of
// the shorter side) in from every corner. Between the two valleys of a
// side, the band swells to `band` at the midpoint with a raised cosine.
// Between the two valleys that flank a corner, the ring's inner edge is a
// SUPERELLIPSE arc — a squircle corner of radius R − band_min, tangent to
// both valleys — whose exponent is solved so that its deepest point, on
// the diagonal, is exactly `band` in from the silhouette. That arc is the
// corner hill: it rises from each valley, peaks on the diagonal, and, being
// tangent at both ends, meets the side hills' flat valleys with no crease.
//
// Why a shader rather than rects: the swell is continuous along a side,
// and a scene rect can only approximate it with a clipped region whose
// corner radius is one scalar. That fillet is capped by the thickness
// change (tens of px) while a side is hundreds long, so it reads as a bump
// near the centre rather than a swell along the whole run.

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

varying vec4 v_color;
varying vec2 v_texcoord;

uniform vec2 position;
uniform vec2 size;
uniform float corner_radius;
// Thickness at a hill's apex, and at a valley.
uniform float band;
uniform float band_min;
// Retired by the wave profile and ignored: the valleys place the zone
// seams now, and the corner hill's size and height follow from `band`
// and the window's shorter side. Kept so the node API and its callers
// need not change.
uniform float corner_len;
uniform float gap;
uniform float bulge;
// Shape of each SIDE hill: the raised cosine's [0,1] height to this power.
// Below 1 broadens the hill (flatter top, tighter valleys); above 1
// sharpens it. Floored at 0.6 in the shader — below 0.5 the valleys would
// turn into cusps. The corner hills' shape is the superellipse's own.
uniform float swell_curve;
// Node-local rect (x, y, w, h) the ring must not draw over — the client is
// drawing an in-surface popover there and the menu must read as in FRONT of
// the chrome. w or h <= 0 disables.
uniform vec4 exclusion;
// Zone under the pointer (see ZONE_* below), or < 0 for none.
uniform float hovered;
uniform vec4 hover_color;

// Defined in corner_alpha.frag, concatenated after this source — the same
// shared routine bevel.frag uses, so the ring traces the same superellipse
// every other corner cut does.
float corner_dist(vec2 size, vec2 position,
        float radius_tl, float radius_tr, float radius_bl, float radius_br);

float ring_dist(vec2 offset) {
    return corner_dist(size, position + offset,
        corner_radius, corner_radius, corner_radius, corner_radius);
}

// Zone numbering MUST match the compositor's BorderElement::index():
// 0 Top, 1 Bottom, 2 Left, 3 Right, 4 TopLeft, 5 TopRight, 6 BottomLeft,
// 7 BottomRight.
const float ZONE_TOP = 0.0;
const float ZONE_BOTTOM = 1.0;
const float ZONE_LEFT = 2.0;
const float ZONE_RIGHT = 3.0;
const float ZONE_TL = 4.0;
const float ZONE_TR = 5.0;
const float ZONE_BL = 6.0;
const float ZONE_BR = 7.0;

const float PI = 3.14159265359;
const float LN2 = 0.69314718056;

// Height, 0..1, of a side's hill at distance `x` along it: a raised cosine
// peaking at the midpoint and reaching zero — with zero slope — at the two
// valleys, `R` in from either end. Zero beyond them.
float side_hill(float x, float len, float R) {
    float half_span = max(0.5 * len - R, 1e-3);
    float d = abs(x - 0.5 * len);
    if (d >= half_span) {
        return 0.0;
    }
    float h = 0.5 * (1.0 + cos(PI * d / half_span));
    return pow(h, max(swell_curve, 0.6));
}

void main() {
    float dist = ring_dist(vec2(0.0));
    // Rect-local position, TOP-DOWN: gl_FragCoord minus the box position is
    // already y-down box-local here (the pass renders under a FLIPPED_180
    // projection, so fragment row 0 is the top of the buffer — see the same
    // note in droplet.frag). Everything below that names a side reads y as
    // "distance from the top", so this must NOT be flipped the way
    // corner_dist flips its own copy: the SDF is symmetric under that flip
    // (one radius for all four corners), but the zone labels are not.
    vec2 p = gl_FragCoord.xy - position;

    // Distances to the four sides, and which of each pair is nearer.
    float dl = p.x;
    float dr = size.x - p.x;
    float dt = p.y;
    float db = size.y - p.y;
    bool left = dl <= dr;
    bool top = dt <= db;
    float dv = left ? dl : dr;
    float dh = top ? dt : db;

    // The valleys sit R in from every corner; nothing reaches deeper than
    // that, so anything past it is interior.
    float R = 0.25 * min(size.x, size.y);
    if (dist > 0.0 || dist < -(R + 2.0)) {
        discard;
    }
    float depth = -dist; // 0 at the silhouette, growing inward

    // A client popover owns this rect; the ring yields to it wholesale. The
    // rect arrives top-left-origin (y down), the same space as p.
    if (exclusion.z > 0.0 && exclusion.w > 0.0
            && p.x >= exclusion.x && p.x < exclusion.x + exclusion.z
            && p.y >= exclusion.y && p.y < exclusion.y + exclusion.w) {
        discard;
    }

    float A = max(band - band_min, 0.0);
    // The inner superellipse's radius, and the exponent that puts its
    // diagonal point — R - Rp * 2^(-1/n) in from each side — at `band`.
    // Clamped to a circle at the low end (a window too small for the
    // solve gets a taller corner rather than a concave one).
    float Rp = max(R - band_min, 1e-3);
    float ratio = clamp(1.0 - A / Rp, 0.05, 0.9999); // 2^(-1/n)
    float n = clamp(-LN2 / log(ratio), 2.0, 12.0);

    // Signed distance to the interior, positive in the ring: in a corner
    // square, the superellipse (its implicit function scaled by its
    // gradient, exact where it matters — at the edge); elsewhere, how far
    // short of the nearer inner edges the fragment falls.
    float f;
    if (dv < R && dh < R) {
        vec2 c = max(vec2(R - dv, R - dh) / Rp, 0.0);
        vec2 cn = pow(c, vec2(n));
        float F = cn.x + cn.y - 1.0;
        vec2 grad = n * pow(c, vec2(n - 1.0)) / Rp;
        f = F / max(length(grad), 1e-4);
    } else {
        float t_h = band_min + A * side_hill(p.x, size.x, R);
        float t_v = band_min + A * side_hill(p.y, size.y, R);
        f = max(t_h - dh, t_v - dv);
    }
    if (f <= 0.0) {
        discard;
    }

    // Zones: the side the fragment is nearer to owns it, so the split runs
    // along the diagonals; along that side, everything within R of an end
    // — the corner hill, valley to valley — is the corner zone.
    bool vertical = dv < dh;
    float from_end = vertical ? dh : dv;
    float zone;
    if (from_end < R) {
        zone = top ? (left ? ZONE_TL : ZONE_TR) : (left ? ZONE_BL : ZONE_BR);
    } else if (vertical) {
        zone = left ? ZONE_LEFT : ZONE_RIGHT;
    } else {
        zone = top ? ZONE_TOP : ZONE_BOTTOM;
    }

    // Feather both flanks by a pixel: the silhouette side against the
    // window's own rounded edge, the inner side along the wave.
    float aa = clamp(depth, 0.0, 1.0) * clamp(f, 0.0, 1.0);

    vec4 base = (hovered >= 0.0 && abs(hovered - zone) < 0.5) ? hover_color : v_color;
    // Premultiplied, like every other rect this renderer draws.
    gl_FragColor = base * aa;
}
