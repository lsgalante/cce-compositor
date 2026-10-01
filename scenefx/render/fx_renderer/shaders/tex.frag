#define SOURCE %d
#define EFFECTS %d

#define SOURCE_TEXTURE_RGBA 1
#define SOURCE_TEXTURE_RGBX 2
#define SOURCE_TEXTURE_EXTERNAL 3

#if !defined(SOURCE) || !defined(EFFECTS)
#error "Missing shader preamble"
#endif

#if SOURCE == SOURCE_TEXTURE_EXTERNAL
#extension GL_OES_EGL_image_external : require
#endif

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

varying vec2 v_texcoord;

#if SOURCE == SOURCE_TEXTURE_EXTERNAL
uniform samplerExternalOES tex;
#elif SOURCE == SOURCE_TEXTURE_RGBA || SOURCE == SOURCE_TEXTURE_RGBX
uniform sampler2D tex;
#endif

uniform float alpha;

#if EFFECTS
uniform vec2 size;
uniform vec2 position;
uniform float radius_top_left;
uniform float radius_top_right;
uniform float radius_bottom_left;
uniform float radius_bottom_right;

uniform vec2 clip_size;
uniform vec2 clip_position;
uniform float clip_radius_top_left;
uniform float clip_radius_top_right;
uniform float clip_radius_bottom_left;
uniform float clip_radius_bottom_right;
#endif

uniform bool discard_transparent;

// Backdrop compression (see compress_backdrop below). Ceiling and knee are
// linear luminances; a ceiling of 0 turns it off, which is every texture
// draw but a status segment's blurred backdrop.
uniform float compress_ceil;
uniform float compress_knee;
uniform bool compress_invert;

vec4 sample_texture() {
#if SOURCE == SOURCE_TEXTURE_RGBA || SOURCE == SOURCE_TEXTURE_EXTERNAL
	return texture2D(tex, v_texcoord);
#elif SOURCE == SOURCE_TEXTURE_RGBX
	return vec4(texture2D(tex, v_texcoord).rgb, 1.0);
#endif
}

// Pull the backdrop's luminance under a ceiling, so text of a known color
// keeps its contrast over every pixel of it. Below the knee nothing moves;
// above it the luminance approaches the ceiling asymptotically (slope 1 at
// the knee, so there is no visible seam), and the color is scaled rather
// than desaturated, so a bright backdrop keeps its hue and only loses
// brightness. Dark text compresses the INVERTED image, which lifts the
// shadows toward white instead. Keep in step with droplet.frag.
vec3 compress_backdrop(vec3 c) {
	vec3 lin = pow(max(c, vec3(0.0)), vec3(2.2));
	if (compress_invert) {
		lin = vec3(1.0) - lin;
	}
	float y = dot(lin, vec3(0.2126, 0.7152, 0.0722));
	if (y > compress_knee) {
		float span = max(compress_ceil - compress_knee, 1e-4);
		float y2 = compress_knee + span * (1.0 - exp(-(y - compress_knee) / span));
		lin *= y2 / y;
	}
	if (compress_invert) {
		lin = vec3(1.0) - lin;
	}
	return pow(clamp(lin, 0.0, 1.0), vec3(1.0 / 2.2));
}

vec4 sample_compressed() {
	vec4 c = sample_texture();
	if (compress_ceil > 0.0 && c.a > 0.0) {
		c.rgb = compress_backdrop(c.rgb / c.a) * c.a;
	}
	return c;
}

#if EFFECTS
float corner_alpha(vec2 size, vec2 position, bool is_cutout,
		float radius_tl, float radius_tr, float radius_bl, float radius_br);
#endif

void main() {
#if EFFECTS
	float quad_corner_alpha = corner_alpha(
		size - 0.5,
		position + 0.25,
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

	gl_FragColor = sample_compressed() * alpha * quad_corner_alpha * clip_corner_alpha;
#else
	gl_FragColor = sample_compressed() * alpha;
#endif

	if (discard_transparent && gl_FragColor.a == 0.0) {
		discard;
	}
}
