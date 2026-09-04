#ifndef _FX_OFFSCREEN_BUFFERS_H
#define _FX_OFFSCREEN_BUFFERS_H

#include <wlr/types/wlr_output.h>
#include <wlr/util/addon.h>

/**
 * Used to add effect framebuffers per output instead of every output sharing
 * them.
 */
struct fx_offscreen_buffers {
	struct wl_list link; // fx_renderer.offscreen_buffers
	struct wlr_addon addon;

	// Contains the blurred background for tiled windows. Larger than the
	// output by cache_margin_* on every side: a bake is stored at its
	// node's ANCHOR (where the node was when it baked, plus the margin), and
	// sampled shifted by the node's travel since, so a node that moves with
	// its backdrop keeps reading its own bake; the margin gives room to bake
	// the strips a window hanging off the output exposes as it travels,
	// without re-anchoring the whole bake.
	struct fx_framebuffer *optimized_blur_buffer;
	int cache_margin_x, cache_margin_y;
	// Contains the non-blurred background for tiled windows. Used for blurring
	// optimized surfaces with an alpha. Just as inefficient as the regular blur.
	struct fx_framebuffer *optimized_no_blur_buffer;
	// Contains the original pixels to draw over the areas where artifact are visible
	struct fx_framebuffer *blur_saved_pixels_buffer;
	// Blur swaps between the two effects buffers every time it scales the image
	// Buffer used for effects
	struct fx_framebuffer *effects_buffer;
	// Swap buffer used for effects
	struct fx_framebuffer *effects_buffer_swapped;
};

void fx_offscreen_buffers_destroy(struct fx_offscreen_buffers *fbos);
struct fx_offscreen_buffers *fx_offscreen_buffers_try_get(struct wlr_output *output);

#endif
