// Corner-shape exponent: 2 = circular arc, > 2 = superellipse "squircle"
// corners — kept in lockstep with the cce-ui clients' corner_shape so the
// compositor's corner cut lands exactly on the corners the clients draw.
uniform float corner_shape;

float get_dist(vec2 q, float radius) {
	vec2 p = max(q, 0.0);
	if (corner_shape > 2.001 && radius > 0.0 && p.x > 0.0 && p.y > 0.0) {
		// Superellipse (Lp-norm) corner, radius-normalized so the pow()
		// arguments stay near 1 (mediump-safe). The Lp gradient is not unit
		// length, so the distance carries a first-order |grad| correction —
		// exact on the boundary, where the AA smoothstep samples it.
		vec2 u = p / radius;
		float lp = max(pow(pow(u.x, corner_shape) + pow(u.y, corner_shape),
			1.0 / corner_shape), 1e-4);
		vec2 g = vec2(pow(u.x / lp, corner_shape - 1.0),
			pow(u.y / lp, corner_shape - 1.0));
		return min(max(q.x, q.y), 0.0)
			+ radius * (lp - 1.0) / max(length(g), 1e-4);
	}
	return min(max(q.x, q.y), 0.0) + length(p) - radius;
}

// Note: Returns 0.0 if outside, 1.0 if inside the bounds. The is_cutout parameter
// reverses this, 0.0 inside cutout and 1.0 for outside cutout.
float corner_alpha(vec2 size, vec2 position, bool is_cutout,
		float radius_tl, float radius_tr, float radius_bl, float radius_br) {
	if (radius_tl <= 0.0
			&& radius_tr <= 0.0
			&& radius_bl <= 0.0
			&& radius_br <= 0.0) {
		return 1.0;
	}

	vec2 relative_pos = (gl_FragCoord.xy - position);
	relative_pos.y = size.y - relative_pos.y;

	if (relative_pos.x < -0.5 || relative_pos.y < -0.5
			|| relative_pos.x > size.x + 0.5 || relative_pos.y > size.y + 0.5) {
		if (is_cutout) {
			return 1.0;
		}
		discard;
	}

	bool is_top_left = radius_tl > 0.0
		&& relative_pos.x <= radius_tl
		&& relative_pos.y <= radius_tl;
	bool is_top_right = radius_tr > 0.0
		&& relative_pos.x >= size.x - radius_tr
		&& relative_pos.y <= radius_tr;
	bool is_bottom_left = radius_bl > 0.0
		&& relative_pos.x <= radius_bl
		&& relative_pos.y >= size.y - radius_bl;
	bool is_bottom_right = radius_br > 0.0
		&& relative_pos.x >= size.x - radius_br
		&& relative_pos.y >= size.y - radius_br;
	if (!is_top_left && !is_top_right && !is_bottom_left && !is_bottom_right) {
		if (is_cutout) {
			discard;
		}
		return 1.0;
	}

	vec2 top_left = abs(relative_pos - size) - size + radius_tl;
	vec2 top_right = abs(relative_pos - vec2(0, size.y)) - size + radius_tr;
	vec2 bottom_left = abs(relative_pos - vec2(size.x, 0)) - size + radius_bl;
	vec2 bottom_right = abs(relative_pos) - size + radius_br;

	float dist = max(
		max(get_dist(top_left, radius_tl), get_dist(top_right, radius_tr)),
		max(get_dist(bottom_left, radius_bl), get_dist(bottom_right, radius_br))
	);

	float result = smoothstep(0.0, 1.0, dist);
	return is_cutout ? result : 1.0 - result;
}

float corner_dist(vec2 size, vec2 position,
		float radius_tl, float radius_tr, float radius_bl, float radius_br) {
	vec2 relative_pos = (gl_FragCoord.xy - position);
	relative_pos.y = size.y - relative_pos.y;

	vec2 top_left = abs(relative_pos - size) - size + radius_tl;
	vec2 top_right = abs(relative_pos - vec2(0, size.y)) - size + radius_tr;
	vec2 bottom_left = abs(relative_pos - vec2(size.x, 0)) - size + radius_bl;
	vec2 bottom_right = abs(relative_pos) - size + radius_br;

	float dist = max(
		max(get_dist(top_left, radius_tl), get_dist(top_right, radius_tr)),
		max(get_dist(bottom_left, radius_bl), get_dist(bottom_right, radius_br))
	);

	return dist;
}
