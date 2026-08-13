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

// Signed distance to a rounded rect; negative inside.
float rounded_rect_sdf(vec2 p, vec2 half_size, float radius) {
    vec2 q = abs(p) - half_size + radius;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - radius;
}

void main() {
    vec2 half_size = size * 0.5;
    vec2 center = position + half_size;
    vec2 p = gl_FragCoord.xy - center;

    float dist = rounded_rect_sdf(p, half_size, corner_radius);

    // Outside the rect, or deeper in than the rim: nothing to draw.
    float rim = max(thickness, 1.0);
    if (dist > 0.0 || dist < -rim) {
        discard;
    }

    // t: 0 at the outer edge, 1 where the rim meets the flat surface.
    float t = clamp(-dist / rim, 0.0, 1.0);

    // The surface normal of the chamfer. Its in-plane part points OUT of the
    // rect (the gradient of the SDF), and its steepness falls off across the
    // rim — steep at the edge, flat where it meets the face.
    vec2 grad = normalize(vec2(
        rounded_rect_sdf(p + vec2(1.0, 0.0), half_size, corner_radius) -
        rounded_rect_sdf(p - vec2(1.0, 0.0), half_size, corner_radius),
        rounded_rect_sdf(p + vec2(0.0, 1.0), half_size, corner_radius) -
        rounded_rect_sdf(p - vec2(0.0, 1.0), half_size, corner_radius)
    ) + vec2(1e-6));

    // Slope profile across the rim. Mixing linear and smoothstep gives the
    // "shoulder" control: a hard chamfer keeps a constant slope, a rounded
    // one eases off at both ends.
    float slope = mix(1.0 - t, 1.0 - smoothstep(0.0, 1.0, t), shoulder);

    // Lambert against the light, using only the in-plane direction (the
    // chamfer's tilt is what varies; the face itself is flat-on).
    float facing = dot(grad, normalize(light_dir + vec2(1e-6)));

    // Positive = lit side, negative = shaded side.
    float lit = max(facing, 0.0) * light_intensity;
    float shade = max(-facing, 0.0) * shade_intensity;

    float highlight = lit * slope;
    float shadow = shade * slope;

    // White highlight over dark shade, both premultiplied. v_color carries
    // the tint (and its alpha scales the whole effect).
    vec3 rgb = v_color.rgb * highlight;
    float alpha = highlight + shadow;

    // Feather the very outer pixel so the rim doesn't alias against the
    // window's own rounded edge.
    float edge_aa = clamp(-dist, 0.0, 1.0);

    gl_FragColor = vec4(rgb, alpha) * v_color.a * edge_aa;
}
