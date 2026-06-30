#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

varying vec4 v_color;
varying vec2 v_texcoord;

uniform vec2 size;
uniform vec2 position;
uniform float radius_top_left;
uniform float radius_top_right;
uniform float radius_bottom_left;
uniform float radius_bottom_right;
uniform float fade_inset;

uniform vec2 clip_size;
uniform vec2 clip_position;
uniform float clip_radius_top_left;
uniform float clip_radius_top_right;
uniform float clip_radius_bottom_left;
uniform float clip_radius_bottom_right;

float corner_alpha(vec2 size, vec2 position, bool is_cutout,
		float radius_tl, float radius_tr, float radius_bl, float radius_br);
float corner_dist(vec2 size, vec2 position,
		float radius_tl, float radius_tr, float radius_bl, float radius_br);

void main() {
	float quad_corner_alpha = corner_alpha(
		size - 1.0,
		position + 0.5,
		false,
		radius_top_left,
		radius_top_right,
		radius_bottom_left,
		radius_bottom_right
	);

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
		float r_tl = radius_top_left > 0.0 ? radius_top_left + fade_inset : 0.0;
		float r_tr = radius_top_right > 0.0 ? radius_top_right + fade_inset : 0.0;
		float r_bl = radius_bottom_left > 0.0 ? radius_bottom_left + fade_inset : 0.0;
		float r_br = radius_bottom_right > 0.0 ? radius_bottom_right + fade_inset : 0.0;
		float d = corner_dist(
			size - 1.0,
			position + 0.5,
			r_tl,
			r_tr,
			r_bl,
			r_br
		);
		float inside_dist = -d;
		float fade_factor = clamp(inside_dist / fade_inset, 0.0, 1.0);
		final_color *= fade_factor;
	}

	gl_FragColor = final_color * quad_corner_alpha * clip_corner_alpha;
}
