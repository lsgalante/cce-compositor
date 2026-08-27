// Edge bevel: a lit chamfer around the inside of a rounded rect.
//
// The rim is shaded as if the window edge were rolled off toward the surface
// plane, so the side facing the light picks up a highlight and the far side
// falls into shade, with the transition sweeping smoothly around the corner
// arcs. Everything is derived from the signed distance to the rounded-rect
// boundary, which makes the corners fall out for free — the gradient follows
// the curve instead of being mitred at 45 degrees.

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
// Rim width in px: how far in from the edge the chamfer reaches.
uniform float thickness;
// Unit vector pointing toward the light (screen space, y down).
uniform vec2 light_dir;
// Highlight and shade strengths, 0..1.
uniform float light_intensity;
uniform float shade_intensity;
// Shoulder: how much of the rim is the rounded roll-off vs the flat face.
// 0 = a hard flat chamfer, 1 = fully rounded shoulder.
uniform float shoulder;
// Focus treatment: > 0 wraps the highlight around all four sides in
// focus_color — the DE's focused-plate glint on the window edge. The azimuth
// mask against the light drops, so every edge shades as the lit one; the
// shade term goes with it (an all-around light casts no rim shadow).
uniform float focus;
uniform vec3 focus_color;

// Defined in corner_alpha.frag, which is concatenated after this source (the
// same way box_shadow.frag gets it). Using the shared routine rather than a
// private circular SDF is the whole point: at corner_shape > 2 the DE's
// corners are superellipses and the compositor hands us the span-WIDENED
// radius that shape needs. Read as a circle, that radius drew an arc twice
// the size of the real corner and the rim peeled ~15px off the edge at the
// diagonal.
float corner_dist(vec2 size, vec2 position,
        float radius_tl, float radius_tr, float radius_bl, float radius_br);

// The rounded rect is symmetric in both axes here (one radius on all four
// corners), so corner_dist's internal y flip cancels and offsetting the
// `position` uniform by d is just a translation of the sample point by -d.
float bevel_dist(vec2 offset) {
    return corner_dist(size, position + offset,
        corner_radius, corner_radius, corner_radius, corner_radius);
}

void main() {
    float dist = bevel_dist(vec2(0.0));

    // Outside the rect, or deeper in than the rim: nothing to draw. Discard
    // before the gradient so the four extra SDF evaluations only run on the
    // thin band that actually shades.
    float rim = max(thickness, 1.0);
    if (dist > 0.0 || dist < -rim) {
        discard;
    }

    // t: 0 at the outer edge, 1 where the rim meets the flat surface.
    float t = clamp(-dist / rim, 0.0, 1.0);

    // The surface normal of the chamfer. Its in-plane part points OUT of the
    // rect (the gradient of the SDF), and its steepness falls off across the
    // rim — steep at the edge, flat where it meets the face. Sampling at
    // -offset means the differences below are already in the same space the
    // light direction is authored in.
    vec2 grad = normalize(vec2(
        bevel_dist(vec2(-1.0, 0.0)) - bevel_dist(vec2(1.0, 0.0)),
        bevel_dist(vec2(0.0, -1.0)) - bevel_dist(vec2(0.0, 1.0))
    ) + vec2(1e-6));

    // Slope profile across the rim. Mixing linear and smoothstep gives the
    // "shoulder" control: a hard chamfer keeps a constant slope, a rounded
    // one eases off at both ends.
    float slope = mix(1.0 - t, 1.0 - smoothstep(0.0, 1.0, t), shoulder);

    // Lambert against the light, using only the in-plane direction (the
    // chamfer's tilt is what varies; the face itself is flat-on).
    float facing = dot(grad, normalize(light_dir + vec2(1e-6)));

    // Feather the very outer pixel so the rim doesn't alias against the
    // window's own rounded edge.
    float edge_aa = clamp(-dist, 0.0, 1.0);

    if (focus > 0.5) {
        // Focused window: the familiar lit-edge highlight, wrapped — every
        // edge shades as the one facing the light, in the accent color, no
        // shade side. Same slope profile, so band width and shoulder match
        // the unfocused rim exactly.
        float h = light_intensity * slope;
        gl_FragColor = vec4(focus_color * h, h) * v_color.a * edge_aa;
        return;
    }

    // Positive = lit side, negative = shaded side.
    float lit = max(facing, 0.0) * light_intensity;
    float shade = max(-facing, 0.0) * shade_intensity;

    float highlight = lit * slope;
    float shadow = shade * slope;

    // White highlight over dark shade, both premultiplied. v_color carries
    // the tint (and its alpha scales the whole effect).
    vec3 rgb = v_color.rgb * highlight;
    float alpha = highlight + shadow;

    gl_FragColor = vec4(rgb, alpha) * v_color.a * edge_aa;
}
