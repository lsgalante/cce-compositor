// Window resize-handle frame: a ring inside a rounded rect whose thickness
// swells from the corners toward the middle of each side, like the moulding
// of a picture frame.
//
// The ring is cut into eight zones — four corner pieces and four edge bars —
// separated by `gap`, but the THICKNESS is one continuous profile per side,
// independent of that cut: `band_min` at the very corner easing to `band` at
// the side's midpoint. The corner pieces are part of the same curve rather
// than a flat run, so the swell reads as moulding carried right around the
// frame instead of a bar that steps up between two constant corners.
// `corner_len` therefore places the gaps; it no longer bounds the swell.
//
// Why a shader rather than rects: the swell is continuous along a side, and
// a scene rect can only approximate it with a clipped region whose corner
// radius is one scalar. That fillet is capped by the thickness change (tens
// of px) while a side is hundreds long, so it reads as a bump near the
// centre rather than a swell along the whole run.

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
// Thickness at a side's midpoint, and at the corner pieces.
uniform float band;
uniform float band_min;
// How far a corner piece runs along each of its sides, and the gap that
// separates it from the neighbouring edge bar.
uniform float corner_len;
uniform float gap;
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

void main() {
    float dist = ring_dist(vec2(0.0));
    // Outside the rect, or deeper in than the thickest the ring ever gets.
    if (dist > 0.0 || dist < -band) {
        discard;
    }
    float depth = -dist; // 0 at the silhouette, growing inward

    // Rect-local position, in the SAME y-flipped space corner_dist works in
    // (see corner_alpha.frag: it flips y after subtracting `position`).
    // Without the flip the zone logic is upside down relative to the SDF.
    vec2 p = gl_FragCoord.xy - position;
    p.y = size.y - p.y;

    // Which side owns this fragment: whichever edge it sits nearer. The
    // comparison is on distance to the edge, so the split runs along the
    // diagonals and every corner is shared consistently by its two sides.
    float dl = p.x;
    float dr = size.x - p.x;
    float dt = p.y;
    float db = size.y - p.y;
    bool vertical = min(dl, dr) < min(dt, db);

    // `along` is the coordinate down the owning side, `len` its full length.
    float along = vertical ? p.y : p.x;
    float len = vertical ? size.y : size.x;
    // Distance from the nearer end of that side.
    float u = min(along, len - along);

    // Thickness: one curve over the whole half-side, corner pieces included.
    // Spreading it across the full run rather than only the bar is what makes
    // the swell gradual; smoothstep flattens the curve at both ends, so it
    // leaves no crease where the two halves meet at the midpoint and none
    // where it starts at the corner.
    float half_side = max(0.5 * len, 1e-3);
    float t = clamp(u / half_side, 0.0, 1.0);
    float thickness = mix(band_min, band, smoothstep(0.0, 1.0, t));

    // The cut into zones is separate from the profile: it decides where the
    // gaps fall and which zone the pointer is over, nothing about thickness.
    float zone;
    if (u <= corner_len) {
        bool left = dl <= dr;
        bool top = dt <= db;
        zone = top ? (left ? ZONE_TL : ZONE_TR) : (left ? ZONE_BL : ZONE_BR);
    } else if (u <= corner_len + gap) {
        // The gap between a corner piece and its neighbouring bar.
        discard;
    } else if (vertical) {
        zone = (dl <= dr) ? ZONE_LEFT : ZONE_RIGHT;
    } else {
        zone = (dt <= db) ? ZONE_TOP : ZONE_BOTTOM;
    }

    if (depth > thickness) {
        discard;
    }

    // Feather both flanks by a pixel: the silhouette side against the
    // window's own rounded edge, the inner side against its content.
    float aa = clamp(depth, 0.0, 1.0) * clamp(thickness - depth, 0.0, 1.0);

    vec4 base = (hovered >= 0.0 && abs(hovered - zone) < 0.5) ? hover_color : v_color;
    // Premultiplied, like every other rect this renderer draws.
    gl_FragColor = base * aa;
}
