// Window resize-handle frame: a ring inside a rounded rect whose thickness
// swells from the corners toward the middle of each side, like the moulding
// of a picture frame.
//
// Two elements make the frame. The BAND is thinnest at the SEAMS — the gaps
// between corner and edge pieces — and swells away from them in both
// directions: an edge bar rises to `band` at its side's midpoint, a corner
// arm rises back toward the apex, where it flows into the round BULGE that
// sits on each corner (a disc smooth-unioned onto the ring, its inner
// boundary bowing inward like a bead). So each gap separates two thin tips,
// and every piece thickens toward its own middle. `corner_len` places the
// seams and the corner zones.
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
// Shape of the swell along a side. Below 1 the ring gains its thickness
// early and then creeps toward the peak; above 1 does the reverse.
uniform float swell_curve;
// Radius of the round pad on each corner, 0 to disable.
uniform float bulge;
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
    // Outside the rect, or deeper in than anything here reaches: the band at
    // its thickest, or a corner bulge (its centre already sits corner_radius
    // in from the silhouette).
    if (dist > 0.0 || dist < -(max(band, bulge + corner_radius) + 1.0)) {
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

    // The corner run, clamped against ITS OWN side (see the zone comment
    // below); the band profile and the zone cut share it so the thickness
    // minimum lands exactly on the seam.
    float cl = min(corner_len, 0.45 * len);

    // The band: band_min at the seams, swelling away from them both ways —
    // an edge bar to `band` at the side's midpoint, a corner arm back toward
    // the apex, where the pad's union takes over. v is 0 at a seam and 1 at
    // a piece's peak; swell_curve pulls the gain early (below 1) or late.
    float half_side = max(0.5 * len, 1e-3);
    float v;
    if (u <= cl) {
        v = 1.0 - u / max(cl, 1e-3);
    } else {
        float run = max(half_side - cl - gap, 1e-3);
        v = clamp((u - cl - gap) / run, 0.0, 1.0);
    }
    float s = pow(smoothstep(0.0, 1.0, v), max(swell_curve, 0.01));
    float thickness = mix(band_min, band, s);
    float f_ring = thickness - depth;

    // The corner bulge: a disc centred on the corner ARC's centre, so it sits
    // flush behind the silhouette and pokes inward by its radius. Positive
    // inside, like f_ring.
    bool left = dl <= dr;
    bool top = dt <= db;
    vec2 arc_c = vec2(left ? corner_radius : size.x - corner_radius,
                      top ? corner_radius : size.y - corner_radius);
    float f_disc = bulge - length(p - arc_c);

    // Smooth union of band and bulge: the pad flows into the moulding with a
    // fillet instead of a notch. Blend radius scales with the pad.
    float k = max(0.35 * bulge, 1.0);
    float h = clamp(0.5 + 0.5 * (f_disc - f_ring) / k, 0.0, 1.0);
    float f = mix(f_ring, f_disc, h) + k * h * (1.0 - h);

    // Zones: anything on the pad is its corner's, then the band cut as
    // before. The gap only severs the BAND — a groove across the pad would
    // read as damage, so it skips fragments the disc owns.
    // The corner run clamps per side because two corner zones on one side
    // must never meet — past that, the side's midpoint would resize
    // diagonally.
    float zone;
    if (u <= cl || f_disc > 0.0) {
        zone = top ? (left ? ZONE_TL : ZONE_TR) : (left ? ZONE_BL : ZONE_BR);
        if (u > cl && u <= cl + gap && f_disc <= 0.0) {
            discard;
        }
    } else if (u <= cl + gap) {
        discard;
    } else if (vertical) {
        zone = (dl <= dr) ? ZONE_LEFT : ZONE_RIGHT;
    } else {
        zone = (dt <= db) ? ZONE_TOP : ZONE_BOTTOM;
    }

    if (f <= 0.0) {
        discard;
    }

    // Feather both flanks by a pixel: the silhouette side against the
    // window's own rounded edge, the inner side along the unioned boundary.
    float aa = clamp(depth, 0.0, 1.0) * clamp(f, 0.0, 1.0);

    vec4 base = (hovered >= 0.0 && abs(hovered - zone) < 0.5) ? hover_color : v_color;
    // Premultiplied, like every other rect this renderer draws.
    gl_FragColor = base * aa;
}
