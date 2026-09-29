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

float corner_dist(vec2 size, vec2 position,
		float radius_tl, float radius_tr, float radius_bl, float radius_br);
float corner_alpha(vec2 size, vec2 position, bool is_cutout,
		float radius_tl, float radius_tr, float radius_bl, float radius_br);

void main() {
	// The outline's signed distance, from the same function the clip below
	// and the buffer corner cut use, so an edge pixel sits at dist 0.
	//
	// This was inlined here with two differences that made the node draw
	// less than the scene thinks it covers. The y flip subtracted from
	// size.y where the rest of the math works in size - 1, so the distance
	// was one pixel off vertically: the box's top row got alpha 0 and the
	// row under it 0.5. And the smoothstep ran -0.5..0.5, which left every
	// straight edge pixel at half alpha. scene_node_opaque_region counts an
	// alpha-1 rounded rect as opaque outside its corner squares, so what
	// lay beneath those rows was culled and they blended over whatever the
	// buffer last held — a tint that halved on every repaint, or never
	// changed at all in the alpha-0 row.
	float dist = corner_dist(size - 1.0, position + 0.5,
		radius_top_left, radius_top_right,
		radius_bottom_left, radius_bottom_right);

	float result = smoothstep(0.0, 1.0, dist);
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
