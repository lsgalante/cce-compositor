#ifndef SCENE_FX_RENDER_PASS_H
#define SCENE_FX_RENDER_PASS_H

#include <stdbool.h>
#include <wlr/render/pass.h>
#include <wlr/render/interface.h>
#include <wlr/render/swapchain.h>

#include "render/egl.h"
#include "types/fx/clipped_region.h"

struct fx_gles_render_pass {
	struct wlr_render_pass base;
	struct fx_framebuffer *buffer;
	float projection_matrix[9];
	struct wlr_egl_context prev_ctx;
	struct fx_render_timer *timer;
	struct wlr_drm_syncobj_timeline *signal_timeline;
	uint64_t signal_point;

	// The region where there's blur
	pixman_region32_t blur_padding_region;
	bool has_blur;
	// Contains output-specific framebuffers.
	// NULL when no advanced effects like blur is being used in the current pass.
	// Call `fx_render_pass_init_offscreen_buffers` to use advanced effects.
	struct fx_offscreen_buffers *fx_offscreen_buffers;
};

struct fx_gradient {
	float degree;
	/* The full area the gradient fit too, for borders use the window size */
	struct wlr_box range;
	/* The center of the gradient, {0.5, 0.5} for normal*/
	float origin[2];
	/* 1 = Linear, 2 = Conic */
	int linear;
	/* Whether or not to blend the colors */
	int blend;
	int count;
	float *colors;
};

struct fx_render_texture_options {
	struct wlr_render_texture_options base;
	const struct wlr_box *clip_box; // Used to clip csd. Ignored if NULL
	struct fx_corner_fradii corners;
	bool discard_transparent;
	struct clipped_fregion clipped_region;
};

struct fx_render_rect_options {
	struct wlr_render_rect_options base;
	struct clipped_fregion clipped_region;
};

struct fx_render_rect_grad_options {
	struct wlr_render_rect_options base;
	struct fx_gradient gradient;
};

struct fx_render_rounded_rect_options {
	struct wlr_render_rect_options base;
	struct fx_corner_fradii corners;
	struct clipped_fregion clipped_region;
	int fade_inset;
};

struct fx_render_rounded_rect_grad_options {
	struct wlr_render_rect_options base;
	struct fx_gradient gradient;
	struct fx_corner_fradii corners;
};

struct fx_render_box_shadow_options {
	struct wlr_box box;
	struct clipped_fregion clipped_region;
	/* Clip region, leave NULL to disable clipping */
	const pixman_region32_t *clip;

	float blur_sigma;
	int corner_radius;
	struct wlr_render_color color;
};

struct fx_render_bevel_options {
	struct wlr_box box;
	/* Clip region, leave NULL to disable clipping */
	const pixman_region32_t *clip;

	int corner_radius;
	/* Rim width in px: how far in from the edge the chamfer reaches. */
	float thickness;
	/* Direction TOWARD the light, screen space with y down. */
	float light_dir[2];
	float light_intensity;
	float shade_intensity;
	/* 0 = hard flat chamfer, 1 = fully rounded shoulder. */
	float shoulder;
	/* Focus treatment: > 0 wraps the rim highlight around all four sides
	 * in focus_color (the DE's focused-plate glint). */
	float focus;
	float focus_color[3];
	/* Falloff exponent of the focus glint across the rim; 1 = linear. */
	float focus_sharpness;
	/* Tint of the highlight; alpha scales the whole effect. */
	struct wlr_render_color color;
};

/* A resize-handle frame: a ring inside a rounded rect, cut into four corner
 * pieces and four edge bars, whose thickness swells from `band_min` at the
 * corners to `band` at each side's midpoint. */
struct fx_render_frame_options {
	struct wlr_box box;
	/* Clip region, leave NULL to disable clipping */
	const pixman_region32_t *clip;

	int corner_radius;
	/* Thickness at a side's midpoint, and along the corner pieces. */
	float band;
	float band_min;
	/* How far a corner piece runs along each of its sides, and the gap
	 * separating it from the neighbouring bar. */
	float corner_len;
	float gap;
	/* Zone under the pointer (compositor BorderElement index), < 0 none. */
	float hovered;
	float hover_color[4];
	/* Shape of the swell; < 1 gains thickness early, > 1 late. */
	float swell_curve;
	/* Radius of the round pad on each corner, 0 to disable. */
	float bulge;
	/* Box-local rect the ring must not draw over; w/h <= 0 disables. */
	float exclusion[4];
	struct wlr_render_color color;
};

struct fx_render_droplet_options {
	struct wlr_box box;
	/* Clip region, leave NULL to disable clipping */
	const pixman_region32_t *clip;

	/* Silhouette, all px at output scale: top/bottom corner radii, the
	 * bottom-arc edge rise (0 = flat run), the smooth-intersection blend,
	 * and the corner-shape exponent (2 = circular). */
	float attach_r;
	float sheet_r;
	float bow_rise;
	float blend_k;
	float curve;
	/* Refraction: falloff band px, rim offset px (sign flips direction),
	 * inverted-lens ghost strength 0-1. */
	float band_px;
	float refr;
	float ghost;
};

struct fx_render_blur_pass_options {
	struct fx_render_texture_options tex_options;
	struct fx_framebuffer *current_buffer;
	struct blur_data *blur_data;
	bool use_optimized_blur;
	bool ignore_transparent;
	float blur_strength;
	struct fx_corner_fradii corners;
	struct clipped_fregion clipped_region;
};

struct fx_gles_render_pass *fx_get_render_pass(struct wlr_render_pass *render_pass);

/**
 * Initializes the render pass offscreen buffers required for advanced effects
 * like blur.
 */
bool fx_render_pass_init_offscreen_buffers(struct wlr_render_pass *render_pass,
		struct wlr_output *output);

/**
 * Render a fx texture.
 */
void fx_render_pass_add_texture(struct fx_gles_render_pass *render_pass,
	const struct fx_render_texture_options *options);

/**
 * Render a rectangle.
 */
void fx_render_pass_add_rect(struct fx_gles_render_pass *render_pass,
	const struct fx_render_rect_options *options);

/**
 * Render a rectangle with a gradient.
 */
void fx_render_pass_add_rect_grad(struct fx_gles_render_pass *render_pass,
	const struct fx_render_rect_grad_options *options);

/**
 * Render a rounded rectangle.
 */
void fx_render_pass_add_rounded_rect(struct fx_gles_render_pass *render_pass,
	const struct fx_render_rounded_rect_options *options);

/**
 * Render a rounded rectangle with a gradient.
 */
void fx_render_pass_add_rounded_rect_grad(struct fx_gles_render_pass *render_pass,
	const struct fx_render_rounded_rect_grad_options *options);

/**
 * Render a box shadow.
 */
void fx_render_pass_add_box_shadow(struct fx_gles_render_pass *pass,
		const struct fx_render_box_shadow_options *options);

/**
 * Render an edge bevel: a lit chamfer around the inside of a rounded rect.
 */
void fx_render_pass_add_frame(struct fx_gles_render_pass *pass,
	const struct fx_render_frame_options *options);

void fx_render_pass_add_bevel(struct fx_gles_render_pass *pass,
		const struct fx_render_bevel_options *options);

/**
 * Render a droplet backdrop: the unblurred below-layer snapshot re-rendered
 * through a water drop's lens (rim refraction + inverted ghost) inside the
 * drop's silhouette. No-op when the snapshot buffer does not exist.
 */
void fx_render_pass_add_droplet(struct fx_gles_render_pass *pass,
		const struct fx_render_droplet_options *options);

/**
 * Render blur.
 */
void fx_render_pass_add_blur(struct fx_gles_render_pass *pass,
		struct fx_render_blur_pass_options *fx_options);

/**
 * Render optimized blur.
 */
bool fx_render_pass_add_optimized_blur(struct fx_gles_render_pass *pass,
		struct fx_render_blur_pass_options *fx_options);

/**
 * Render from one buffer to another
 */
void fx_render_pass_read_to_buffer(struct fx_gles_render_pass *pass,
		pixman_region32_t *region, struct fx_framebuffer *dst_buffer,
		struct fx_framebuffer *src_buffer);

#endif
