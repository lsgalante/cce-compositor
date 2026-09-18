// Window resize handles: EIGHT DISCS inside the window's rounded
// silhouette — one at the midpoint of each side and one on each corner —
// each disc a zone of its own. `band` is the disc diameter. The side
// discs are tangent to their side; a corner disc sits on the corner's 45°
// diagonal, tangent to the rounded corner arc when that arc is wider than
// the disc and tucked into the two straight edges otherwise.
//
// The compositor's `window::handle_disc_layout` lays out the same discs
// from the same inputs (size, corner_radius, band) for the pointer hit
// test and the catcher rects; the two MUST stay in step, or a handle is
// drawn where it does not grab, or grabs where it is not drawn.
//
// Still a shader rather than eight scene rects: a rounded scene rect takes
// the renderer's global corner shape, a squircle, so a "rect with radius
// half its size" would draw as a squircle, not a circle.

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
// Disc diameter.
uniform float band;
// Retired by the disc handles and ignored; kept so the node API and its
// callers need not change.
uniform float band_min;
uniform float corner_len;
uniform float gap;
uniform float bulge;
uniform float swell_curve;
// Node-local rect (x, y, w, h) the handles must not draw over — the client
// is drawing an in-surface popover there and the menu must read as in
// FRONT of the chrome. w or h <= 0 disables.
uniform vec4 exclusion;
// Zone under the pointer (see ZONE_* below), or < 0 for none.
uniform float hovered;
uniform vec4 hover_color;

// Defined in corner_alpha.frag, concatenated after this source — the same
// shared routine bevel.frag uses, so the discs are clipped to the same
// superellipse silhouette every other corner cut traces.
float corner_dist(vec2 size, vec2 position,
        float radius_tl, float radius_tr, float radius_bl, float radius_br);

// Zone numbering MUST match the compositor's BorderElement::index():
// 0 Top, 1 Bottom, 2 Left, 3 Right, 4 TopLeft, 5 TopRight, 6 BottomLeft,
// 7 BottomRight.
const float SQRT2 = 1.41421356237;

void main() {
    // Rect-local position, TOP-DOWN: gl_FragCoord minus the box position is
    // already y-down box-local here (the pass renders under a FLIPPED_180
    // projection, so fragment row 0 is the top of the buffer — see the same
    // note in droplet.frag). corner_dist flips its own copy; this must not
    // be flipped too, or the zone labels swap vertically.
    vec2 p = gl_FragCoord.xy - position;

    // A client popover owns this rect; the handles yield to it wholesale.
    // The rect arrives top-left-origin (y down), the same space as p.
    if (exclusion.z > 0.0 && exclusion.w > 0.0
            && p.x >= exclusion.x && p.x < exclusion.x + exclusion.z
            && p.y >= exclusion.y && p.y < exclusion.y + exclusion.w) {
        discard;
    }

    float r = 0.5 * band;
    float t = corner_radius > r
        ? corner_radius - (corner_radius - r) / SQRT2
        : r;
    vec2 c[8];
    c[0] = vec2(0.5 * size.x, r);            // Top
    c[1] = vec2(0.5 * size.x, size.y - r);   // Bottom
    c[2] = vec2(r, 0.5 * size.y);            // Left
    c[3] = vec2(size.x - r, 0.5 * size.y);   // Right
    c[4] = vec2(t, t);                       // TopLeft
    c[5] = vec2(size.x - t, t);              // TopRight
    c[6] = vec2(t, size.y - t);              // BottomLeft
    c[7] = vec2(size.x - t, size.y - t);     // BottomRight

    // The nearest disc owns the fragment: signed distance to its rim,
    // negative inside.
    float d = 1e9;
    float zone = -1.0;
    for (int i = 0; i < 8; i++) {
        float di = length(p - c[i]) - r;
        if (di < d) {
            d = di;
            zone = float(i);
        }
    }
    // Feather the rim by a pixel, and clip to the window's own rounded
    // silhouette (a disc never crosses it, but the feather may).
    float sil = corner_dist(size, position,
        corner_radius, corner_radius, corner_radius, corner_radius);
    float aa = clamp(0.5 - d, 0.0, 1.0) * clamp(-sil, 0.0, 1.0);
    if (aa <= 0.0) {
        discard;
    }

    vec4 base = (hovered >= 0.0 && abs(hovered - zone) < 0.5) ? hover_color : v_color;
    // Premultiplied, like every other rect this renderer draws.
    gl_FragColor = base * aa;
}
