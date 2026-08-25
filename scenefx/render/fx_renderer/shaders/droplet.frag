// Droplet backdrop refraction: the region behind a status-bar water drop,
// re-rendered through the drop's lens.
//
// The silhouette is the same shape the cce-ui client draws — a superellipse-
// rounded box (attach radius on the top corners, sheet radius on the bottom)
// smooth-intersected with a huge disc whose lowest point touches the box
// bottom (the continuous bottom arc) — so the refracting region and the
// drawn drop can never disagree. Inside it, the backdrop texture (scenefx's
// unblurred optimized-blur snapshot of everything beneath the bar layer) is
// sampled with two warps:
//
//  - a rim-weighted offset along the SDF gradient: the image bends around
//    the drop edge the way glass pulls the background with it;
//  - a faint inverted, minified sample about the drop center: the upside-
//    down image of the scene a real hanging drop shows in its belly.
//
// The client's translucent tint, gleam and text composite on top of this.

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

varying vec4 v_color;
varying vec2 v_texcoord;

uniform sampler2D tex;
uniform vec2 tex_size;
uniform vec2 position;   // box origin (same space corner shaders use)
uniform vec2 size;       // box size, px
uniform float attach_r;  // top-corner radius px
uniform float sheet_r;   // bottom-corner radius px
uniform float bow_rise;  // bottom-arc edge rise px (0 = flat bottom run)
uniform float blend_k;   // smooth-intersection blend px
uniform float curve;     // corner-shape exponent (2 = circular)
uniform float band_px;   // dome skirt width px (the refraction falloff scale)
uniform float refr;      // rim refraction strength px (sign flips direction)
uniform float ghost;     // inverted-lens ghost strength 0-1

// Signed distance to the drop silhouette at box-local p (y DOWN from the box
// top). The rounded-box part is the same construction as the client's
// rr_sdf_grad: quadrant-selected corner radius, Lp corners with a first-order
// |grad| correction above exponent 2.
float drop_dist(vec2 p) {
	vec2 half_ext = size * 0.5;
	vec2 c = p - half_ext;
	float r = (c.y > 0.0) ? sheet_r : attach_r;
	vec2 q = abs(c) - half_ext + vec2(r, r);
	float d;
	if (q.x > 0.0 && q.y > 0.0) {
		if (curve > 2.001 && r > 0.0) {
			vec2 u = q / r;
			float lp = max(pow(pow(u.x, curve) + pow(u.y, curve), 1.0 / curve), 1e-4);
			vec2 g = vec2(pow(u.x / lp, curve - 1.0), pow(u.y / lp, curve - 1.0));
			d = r * (lp - 1.0) / max(length(g), 1e-4);
		} else {
			d = length(q) - r;
		}
	} else {
		d = max(q.x, q.y) - r;
	}
	if (bow_rise > 0.25) {
		// Smooth-intersect with the bottom-arc disc (smax = -smin(-a,-b)).
		float hx = half_ext.x;
		float R = hx * hx / (2.0 * bow_rise);
		vec2 cc = vec2(hx, size.y - R);
		float dc = length(p - cc) - R;
		float k = max(blend_k, 1.0);
		float hm = clamp(0.5 + 0.5 * (d - dc) / k, 0.0, 1.0);
		d = mix(dc, d, hm) + k * hm * (1.0 - hm);
	}
	return d;
}

void main() {
	// Box-local coords with y down. NOTE: deliberately NOT the corner
	// shaders' extra y flip — that transform is unverifiable from the
	// symmetric shapes that use it (equal radii look the same flipped), and
	// with this drop's asymmetric silhouette it rendered the attach taper at
	// the BOTTOM (live bug: black flipped boxes behind the modules).
	// gl_FragCoord minus the box position is already top-down box-local
	// here.
	vec2 rel = gl_FragCoord.xy - position;

	float d = drop_dist(rel);
	float aa = clamp(-d + 0.5, 0.0, 1.0);
	if (aa <= 0.0) {
		discard;
	}

	// SDF gradient by central differences, in the same y-down local space.
	vec2 grad = normalize(vec2(
		drop_dist(rel + vec2(1.0, 0.0)) - drop_dist(rel - vec2(1.0, 0.0)),
		drop_dist(rel + vec2(0.0, 1.0)) - drop_dist(rel - vec2(0.0, 1.0))
	) + vec2(1e-6, 0.0));

	// Rim-weighted lens profile: 1 at the silhouette, 0 deep inside.
	float t = clamp(-d / max(band_px, 1.0), 0.0, 1.0);
	float lens = (1.0 - t) * (1.0 - t);

	// Refraction: offset the sample along the gradient. gl_FragCoord shares
	// the box-local frame's orientation (see `rel` above), so the offset
	// applies directly. Positive refr samples outward — the background bends
	// around the drop edge.
	vec2 off = grad * (refr * lens);
	vec2 uv = (gl_FragCoord.xy + off) / tex_size;
	vec4 col = texture2D(tex, clamp(uv, vec2(0.0), vec2(1.0)));

	// Inverted lens ghost: the belly shows a faint upside-down, minified
	// image of the scene about the drop center.
	if (ghost > 0.001) {
		vec2 center_fc = position + size * 0.5;
		vec2 ghost_uv = (center_fc + (center_fc - gl_FragCoord.xy) * 0.35) / tex_size;
		vec4 gcol = texture2D(tex, clamp(ghost_uv, vec2(0.0), vec2(1.0)));
		col.rgb = mix(col.rgb, gcol.rgb, ghost * t * t);
	}

	gl_FragColor = vec4(col.rgb, 1.0) * aa;
}
