// Window resize handles: EIGHT DISCS inside the window's rounded
// silhouette — one on each side and one on each corner — each disc a zone
// of its own, plus (`buttons` > 0) three BUTTON discs in the top row, left
// of the top-right corner disc: minimize, maximize, float/tile toggle.
// `band` is the disc diameter. Every disc sits the same distance in from
// the edges it touches, so the three discs along an edge are inline: a
// corner disc sits on the corner's 45° diagonal, tangent to the rounded
// corner arc when that arc is wider than the disc and tucked into the two
// straight edges otherwise, and the side discs take that same inset.
//
// The compositor's `window::handle_disc_layout` lays out the same discs
// from the same inputs (size, corner_radius, band, buttons) for the
// pointer hit test and the catcher rects; the two MUST stay in step, or a
// handle is drawn where it does not grab, or grabs where it is not drawn.
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
// Window buttons: 0 none, 1 for a Floating window, 2 for a Tiled one (the
// toggle's glyph shows the mode a click goes to).
uniform float buttons;

// Defined in corner_alpha.frag, concatenated after this source — the same
// shared routine bevel.frag uses, so the discs are clipped to the same
// superellipse silhouette every other corner cut traces.
float corner_dist(vec2 size, vec2 position,
        float radius_tl, float radius_tr, float radius_bl, float radius_br);

// Zone numbering MUST match the compositor's BorderElement::index():
// 0 Top, 1 Bottom, 2 Left, 3 Right, 4 TopLeft, 5 TopRight, 6 BottomLeft,
// 7 BottomRight, 8 Minimize, 9 Maximize, 10 ToggleTile.
const float SQRT2 = 1.41421356237;
// Centre-to-centre spacing of the top row's discs, in diameters
// (`window::HANDLE_BUTTON_STEP`).
const float STEP = 1.25;

float box_sdf(vec2 q, vec2 half_size) {
    vec2 d = abs(q) - half_size;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

// Coverage of a button's glyph at `q` (px from the disc centre, y down)
// for a disc of radius `r`.
float glyph(float zone, vec2 q, float r) {
    float stroke = max(0.11 * r, 1.0);
    float sd;
    if (zone < 8.5) {
        // Minimize: a bar across the lower middle.
        sd = box_sdf(q - vec2(0.0, 0.18 * r), vec2(0.45 * r, 0.5 * stroke));
    } else if (zone < 9.5) {
        // Maximize: a square outline.
        sd = abs(box_sdf(q, vec2(0.4 * r))) - 0.5 * stroke;
    } else if (buttons < 1.5) {
        // Floating -> tile: a 2x2 grid of cells.
        vec2 cell = vec2(0.2 * r);
        vec2 g = abs(q) - vec2(0.22 * r);
        sd = box_sdf(g, cell * 0.85);
    } else {
        // Tiled -> float: two overlapping window outlines.
        float a = abs(box_sdf(q - vec2(0.12 * r, -0.12 * r), vec2(0.28 * r))) - 0.5 * stroke;
        float b = abs(box_sdf(q + vec2(0.12 * r, -0.12 * r), vec2(0.28 * r))) - 0.5 * stroke;
        // The back outline is hidden under the front one.
        if (box_sdf(q - vec2(0.12 * r, -0.12 * r), vec2(0.28 * r)) < 0.0) {
            b = 1e9;
        }
        sd = min(a, b);
    }
    return clamp(0.5 - sd, 0.0, 1.0);
}

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
    float s = STEP * band;
    // The buttons only when the top row has room for all six discs; the
    // Top disc then leaves the midpoint only if it would crowd them.
    bool with_buttons = buttons > 0.5 && size.x >= 2.0 * t + 5.0 * s;
    float top_x = with_buttons ? min(0.5 * size.x, size.x - t - 4.0 * s) : 0.5 * size.x;
    vec2 c[11];
    c[0] = vec2(top_x, t);                   // Top
    c[1] = vec2(0.5 * size.x, size.y - t);   // Bottom
    c[2] = vec2(t, 0.5 * size.y);            // Left
    c[3] = vec2(size.x - t, 0.5 * size.y);   // Right
    c[4] = vec2(t, t);                       // TopLeft
    c[5] = vec2(size.x - t, t);              // TopRight
    c[6] = vec2(t, size.y - t);              // BottomLeft
    c[7] = vec2(size.x - t, size.y - t);     // BottomRight
    c[8] = vec2(size.x - t - 3.0 * s, t);    // Minimize
    c[9] = vec2(size.x - t - 2.0 * s, t);    // Maximize
    c[10] = vec2(size.x - t - s, t);         // ToggleTile
    int n = with_buttons ? 11 : 8;

    // The nearest disc owns the fragment: signed distance to its rim,
    // negative inside.
    float d = 1e9;
    float zone = -1.0;
    for (int i = 0; i < 11; i++) {
        if (i >= n) {
            break;
        }
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
    // A button's glyph is the disc darkened, so it reads on any disc color.
    if (zone > 7.5) {
        int zi = int(zone + 0.5);
        vec2 cz = zi == 8 ? c[8] : (zi == 9 ? c[9] : c[10]);
        float g = glyph(zone, p - cz, r);
        base = vec4(base.rgb * (1.0 - 0.65 * g), base.a);
    }
    // Premultiplied, like every other rect this renderer draws.
    gl_FragColor = base * aa;
}
