precision highp float;

varying vec4 v_color;
varying vec2 v_texcoord;

uniform vec2 size;
uniform vec2 position;
uniform float radius_top_left;
uniform float radius_top_right;
uniform float radius_bottom_left;
uniform float radius_bottom_right;
uniform float fade_inset;
uniform int fade_mode;

uniform vec2 clip_size;
uniform vec2 clip_position;
uniform float clip_radius_top_left;
uniform float clip_radius_top_right;
uniform float clip_radius_bottom_left;
uniform float clip_radius_bottom_right;

float get_dist(vec2 q, float radius);
float corner_alpha(vec2 size, vec2 position, bool is_cutout,
		float radius_tl, float radius_tr, float radius_bl, float radius_br);

void main() {
	vec2 relative_pos = (gl_FragCoord.xy - (position + 0.5));
	relative_pos.y = size.y - relative_pos.y;

	// Bounding box check
	/* if (relative_pos.x < -0.5 || relative_pos.y < -0.5
			|| relative_pos.x > size.x - 0.5 || relative_pos.y > size.y - 0.5) {
		discard;
	} */

	float r_tl = radius_top_left;
	float r_tr = radius_top_right;
	float r_bl = radius_bottom_left;
	float r_br = radius_bottom_right;

	// Calculate corner distance
	vec2 top_left = abs(relative_pos - (size - 1.0)) - (size - 1.0) + r_tl;
	vec2 top_right = abs(relative_pos - vec2(0.0, size.y - 1.0)) - (size - 1.0) + r_tr;
	vec2 bottom_left = abs(relative_pos - vec2(size.x - 1.0, 0.0)) - (size - 1.0) + r_bl;
	vec2 bottom_right = abs(relative_pos) - (size - 1.0) + r_br;

	float dist = max(
		max(get_dist(top_left, r_tl), get_dist(top_right, r_tr)),
		max(get_dist(bottom_left, r_bl), get_dist(bottom_right, r_br))
	);

	float result = smoothstep(-0.5, 0.5, dist);
	float quad_corner_alpha = 1.0 - result;

	// Clipping
	float clip_corner_alpha = corner_alpha(
		clip_size - 1.0,
		clip_position + 0.5,
		true,
		clip_radius_top_left,
		clip_radius_top_right,
		clip_radius_bottom_left,
		clip_radius_bottom_right
	);

	vec4 final_color = v_color;
	if (fade_inset > 0.0) {
		// Fade by distance to the ROUNDED outline — the same SDF that cuts
		// the corners — not the straight edges. The old per-axis product
		// formed a square-cornered onset at each corner that visually
		// swamped the corner arcs whenever fade and radius were combined.
		float edge_dist = -dist;

		// Clamp to [0, fade_inset] and normalize
		float factor = clamp(edge_dist / fade_inset, 0.0, 1.0);

		if (fade_mode == 1) {
			factor = smoothstep(0.0, 1.0, factor);
		} else if (fade_mode == 2) {
			factor = factor * factor;
		} else if (fade_mode == 3) {
			factor = 0.5 - 0.5 * cos(factor * 3.14159265);
		} else if (fade_mode == 4) {
			float x = 1.0 - factor;
			factor = clamp((exp(-3.0 * x * x) - 0.049) / 0.951, 0.0, 1.0);
		}

		final_color *= pow(factor, 2.2);
	}

	gl_FragColor = final_color * quad_corner_alpha * clip_corner_alpha;
}
