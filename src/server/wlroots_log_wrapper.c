// SPDX-FileCopyrightText: © 2021 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

#define _POSIX_C_SOURCE 199309L
#include <assert.h>
#include <stdarg.h>
#include <stdlib.h>
#include <stdio.h>

#include <wlr/util/log.h>

#define BUFFER_SIZE 1024

void river_wlroots_log_callback(enum wlr_log_importance importance, const char *ptr, size_t len);

static void callback(enum wlr_log_importance importance, const char *fmt, va_list args) {
	char buffer[BUFFER_SIZE];

	// Need to make a copy of the args in case our buffer isn't big
	// enough and we need to use them again.
	va_list args_copy;
	va_copy(args_copy, args);

	const int length = vsnprintf(buffer, BUFFER_SIZE, fmt, args);
	// Need to add one for the terminating 0 byte
	if (length + 1 <= BUFFER_SIZE) {
		// The formatted string fit within our buffer, pass it on to river
		river_wlroots_log_callback(importance, buffer, length);
	} else {
		// The formatted string did not fit in our buffer, we need
		// to allocate enough memory to hold it.
		char *allocated_buffer = malloc(length + 1);
		if (allocated_buffer != NULL) {
			const int length2 = vsnprintf(allocated_buffer, length + 1, fmt, args_copy);
			assert(length2 == length);
			river_wlroots_log_callback(importance, allocated_buffer, length);
			free(allocated_buffer);
		}
	}

	va_end(args_copy);
}

void river_init_wlroots_log(enum wlr_log_importance importance) {
	wlr_log_init(importance, callback);
}

#include <time.h>
#include <scenefx/types/wlr_scene.h>
#include <wlr/types/wlr_buffer.h>
#include <wlr/interfaces/wlr_buffer.h>
#include <drm_fourcc.h>
#include <wlr/types/wlr_output.h>
#include <wlr/util/region.h>
#include <wlr/types/wlr_compositor.h>
#include <wlr/types/wlr_input_device.h>
#include <wlr/types/wlr_keyboard.h>
#include <wlr/interfaces/wlr_keyboard.h>
#include <wlr/types/wlr_cursor.h>
#include <wlr/types/wlr_seat.h>
#include <wlr/types/wlr_pointer.h>
#include <wlr/types/wlr_tablet_v2.h>
#include <wlr/backend.h>
#include <wlr/types/wlr_xdg_shell.h>
#include <wlr/types/wlr_input_method_v2.h>
#include <wlr/types/wlr_data_device.h>

struct wlr_surface *river_scene_node_get_surface(struct wlr_scene_node *node) {
	if (node->type == WLR_SCENE_NODE_BUFFER) {
		struct wlr_scene_buffer *scene_buffer = wlr_scene_buffer_from_node(node);
		struct wlr_scene_surface *scene_surface = wlr_scene_surface_try_from_buffer(scene_buffer);
		if (scene_surface) {
			return scene_surface->surface;
		}
	}
	return NULL;
}

enum wlr_scene_node_type river_scene_node_get_type(struct wlr_scene_node *node) {
	return node->type;
}

struct wlr_scene_tree *river_scene_node_get_parent(struct wlr_scene_node *node) {
	return node->parent;
}

int river_scene_node_get_x(struct wlr_scene_node *node) {
	return node->x;
}

int river_scene_node_get_y(struct wlr_scene_node *node) {
	return node->y;
}

void *river_scene_node_get_data(struct wlr_scene_node *node) {
	return node->data;
}

void river_scene_node_set_data(struct wlr_scene_node *node, void *data) {
	node->data = data;
}

struct wl_signal *river_scene_node_get_destroy_signal(struct wlr_scene_node *node) {
	return &node->events.destroy;
}

static void save_surface_tree_iter(struct wlr_scene_buffer *buffer, int sx, int sy, void *user_data) {
	struct wlr_scene_tree *saved_tree = user_data;
	struct wlr_scene_tree *buffer_tree = wlr_scene_tree_create(saved_tree);
	if (!buffer_tree) {
		return;
	}
	wlr_scene_node_set_position(&buffer_tree->node, sx, sy);
	struct wlr_scene_buffer *scene_buffer = wlr_scene_buffer_create(buffer_tree, buffer->buffer);
	if (!scene_buffer) {
		wlr_scene_node_destroy(&buffer_tree->node);
		return;
	}
	wlr_scene_node_set_position(&scene_buffer->node, 0, 0);
	wlr_scene_buffer_set_dest_size(scene_buffer, buffer->dst_width, buffer->dst_height);
	wlr_scene_buffer_set_source_box(scene_buffer, &buffer->src_box);
	wlr_scene_buffer_set_transform(scene_buffer, buffer->transform);
}

void river_scene_tree_save_buffers(struct wlr_scene_tree *tree, struct wlr_scene_tree *saved_tree) {
	wlr_scene_node_for_each_buffer(&tree->node, save_surface_tree_iter, saved_tree);
}

void river_scene_tree_clear_children(struct wlr_scene_tree *tree) {
	struct wlr_scene_node *child, *tmp;
	wl_list_for_each_safe(child, tmp, &tree->children, link) {
		wlr_scene_node_destroy(child);
	}
}

struct wlr_scene_node *river_scene_node_from_children_link(struct wl_list *link) {
	return wl_container_of(link, (struct wlr_scene_node *)NULL, link);
}

struct wl_signal *river_wlr_output_get_destroy_signal(struct wlr_output *output) {
	return &output->events.destroy;
}

struct wl_signal *river_wlr_output_get_request_state_signal(struct wlr_output *output) {
	return &output->events.request_state;
}

struct wl_signal *river_wlr_output_get_frame_signal(struct wlr_output *output) {
	return &output->events.frame;
}

struct wl_signal *river_wlr_output_get_present_signal(struct wlr_output *output) {
	return &output->events.present;
}

void *river_wlr_output_get_data(struct wlr_output *output) {
	return output->data;
}

void river_wlr_output_set_data(struct wlr_output *output, void *data) {
	output->data = data;
}

const char *river_wlr_output_get_name(struct wlr_output *output) {
	return output->name;
}

enum wlr_output_adaptive_sync_status river_wlr_output_get_adaptive_sync_status(struct wlr_output *output) {
	return output->adaptive_sync_status;
}

bool river_wlr_output_get_enabled(struct wlr_output *output) {
	return output->enabled;
}

struct wlr_output_mode *river_wlr_output_get_current_mode(struct wlr_output *output) {
	return output->current_mode;
}

int32_t river_wlr_output_get_width(struct wlr_output *output) {
	return output->width;
}

int32_t river_wlr_output_get_height(struct wlr_output *output) {
	return output->height;
}

int32_t river_wlr_output_get_refresh(struct wlr_output *output) {
	return output->refresh;
}

struct wl_global *river_wlr_output_get_global(struct wlr_output *output) {
	return output->global;
}

void *river_wlr_surface_get_data(struct wlr_surface *surface) {
	return surface->data;
}

void river_wlr_surface_set_data(struct wlr_surface *surface, void *data) {
	surface->data = data;
}

struct wl_signal *river_wlr_surface_get_commit_signal(struct wlr_surface *surface) {
	return &surface->events.commit;
}

enum wlr_input_device_type river_wlr_input_device_get_type(struct wlr_input_device *dev) {
	return dev->type;
}

const char *river_wlr_input_device_get_name(struct wlr_input_device *dev) {
	return dev->name;
}

struct wl_signal *river_wlr_input_device_get_destroy_signal(struct wlr_input_device *dev) {
	return &dev->events.destroy;
}

void *river_wlr_input_device_get_data(struct wlr_input_device *dev) {
	return dev->data;
}

void river_wlr_input_device_set_data(struct wlr_input_device *dev, void *data) {
	dev->data = data;
}

struct wl_signal *river_wlr_keyboard_get_key_signal(struct wlr_keyboard *kbd) {
	return &kbd->events.key;
}

struct wl_signal *river_wlr_keyboard_get_modifiers_signal(struct wlr_keyboard *kbd) {
	return &kbd->events.modifiers;
}

struct wl_signal *river_wlr_keyboard_get_keymap_signal(struct wlr_keyboard *kbd) {
	return &kbd->events.keymap;
}

struct xkb_keymap *river_wlr_keyboard_get_keymap(struct wlr_keyboard *kbd) {
	return kbd->keymap;
}

struct wlr_keyboard_modifiers *river_wlr_keyboard_get_modifiers(struct wlr_keyboard *kbd) {
	return &kbd->modifiers;
}

void *river_wlr_keyboard_get_data(struct wlr_keyboard *kbd) {
	return kbd->data;
}

void river_wlr_keyboard_set_data(struct wlr_keyboard *kbd, void *data) {
	kbd->data = data;
}

double river_wlr_cursor_get_x(struct wlr_cursor *cursor) {
	return cursor->x;
}

double river_wlr_cursor_get_y(struct wlr_cursor *cursor) {
	return cursor->y;
}

struct wl_signal *river_wlr_cursor_get_motion_signal(struct wlr_cursor *cursor) {
	return &cursor->events.motion;
}

struct wl_signal *river_wlr_cursor_get_motion_absolute_signal(struct wlr_cursor *cursor) {
	return &cursor->events.motion_absolute;
}

struct wl_signal *river_wlr_cursor_get_button_signal(struct wlr_cursor *cursor) {
	return &cursor->events.button;
}

struct wl_signal *river_wlr_cursor_get_axis_signal(struct wlr_cursor *cursor) {
	return &cursor->events.axis;
}

struct wl_signal *river_wlr_cursor_get_frame_signal(struct wlr_cursor *cursor) {
	return &cursor->events.frame;
}

struct wl_signal *river_wlr_cursor_get_swipe_begin_signal(struct wlr_cursor *cursor) {
	return &cursor->events.swipe_begin;
}

struct wl_signal *river_wlr_cursor_get_swipe_update_signal(struct wlr_cursor *cursor) {
	return &cursor->events.swipe_update;
}

struct wl_signal *river_wlr_cursor_get_swipe_end_signal(struct wlr_cursor *cursor) {
	return &cursor->events.swipe_end;
}

struct wl_signal *river_wlr_cursor_get_pinch_begin_signal(struct wlr_cursor *cursor) {
	return &cursor->events.pinch_begin;
}

struct wl_signal *river_wlr_cursor_get_pinch_update_signal(struct wlr_cursor *cursor) {
	return &cursor->events.pinch_update;
}

struct wl_signal *river_wlr_cursor_get_pinch_end_signal(struct wlr_cursor *cursor) {
	return &cursor->events.pinch_end;
}

struct wl_signal *river_wlr_cursor_get_hold_begin_signal(struct wlr_cursor *cursor) {
	return &cursor->events.hold_begin;
}

struct wl_signal *river_wlr_cursor_get_hold_end_signal(struct wlr_cursor *cursor) {
	return &cursor->events.hold_end;
}

struct wl_signal *river_wlr_cursor_get_touch_down_signal(struct wlr_cursor *cursor) {
	return &cursor->events.touch_down;
}

struct wl_signal *river_wlr_cursor_get_touch_motion_signal(struct wlr_cursor *cursor) {
	return &cursor->events.touch_motion;
}

struct wl_signal *river_wlr_cursor_get_touch_up_signal(struct wlr_cursor *cursor) {
	return &cursor->events.touch_up;
}

struct wl_signal *river_wlr_cursor_get_touch_cancel_signal(struct wlr_cursor *cursor) {
	return &cursor->events.touch_cancel;
}

struct wl_signal *river_wlr_cursor_get_touch_frame_signal(struct wlr_cursor *cursor) {
	return &cursor->events.touch_frame;
}

struct wl_signal *river_wlr_cursor_get_tablet_tool_axis_signal(struct wlr_cursor *cursor) {
	return &cursor->events.tablet_tool_axis;
}

struct wl_signal *river_wlr_cursor_get_tablet_tool_proximity_signal(struct wlr_cursor *cursor) {
	return &cursor->events.tablet_tool_proximity;
}

struct wl_signal *river_wlr_cursor_get_tablet_tool_tip_signal(struct wlr_cursor *cursor) {
	return &cursor->events.tablet_tool_tip;
}

struct wl_signal *river_wlr_cursor_get_tablet_tool_button_signal(struct wlr_cursor *cursor) {
	return &cursor->events.tablet_tool_button;
}

const char *river_wlr_seat_get_name(struct wlr_seat *seat) {
	return seat->name;
}

void *river_wlr_seat_get_data(struct wlr_seat *seat) {
	return seat->data;
}

void river_wlr_seat_set_data(struct wlr_seat *seat, void *data) {
	seat->data = data;
}

struct wl_signal *river_wlr_seat_get_request_set_cursor_signal(struct wlr_seat *seat) {
	return &seat->events.request_set_cursor;
}

struct wl_signal *river_wlr_seat_get_request_set_selection_signal(struct wlr_seat *seat) {
	return &seat->events.request_set_selection;
}

struct wl_signal *river_wlr_seat_get_request_start_drag_signal(struct wlr_seat *seat) {
	return &seat->events.request_start_drag;
}

struct wl_signal *river_wlr_seat_get_start_drag_signal(struct wlr_seat *seat) {
	return &seat->events.start_drag;
}

struct wl_signal *river_wlr_seat_get_request_set_primary_selection_signal(struct wlr_seat *seat) {
	return &seat->events.request_set_primary_selection;
}

struct wlr_seat_client *river_wlr_seat_get_pointer_focused_client(struct wlr_seat *seat) {
	return seat->pointer_state.focused_client;
}

struct wlr_surface *river_wlr_seat_get_pointer_focused_surface(struct wlr_seat *seat) {
	return seat->pointer_state.focused_surface;
}

/* Damage the whole output and schedule a frame — the public-field replica of
 * scenefx's internal scene_output_damage_whole(). Viewport zoom re-lays-out
 * the entire screen, but per-node damage under-reports at the seams (stale
 * slivers of the previous zoom level survive), so camera motion forces a
 * full repaint. */
void river_scene_output_damage_whole(struct wlr_scene_output *scene_output) {
	struct wlr_output *output = scene_output->output;
	pixman_region32_t damage;
	pixman_region32_init_rect(&damage, 0, 0, output->width, output->height);
	wlr_output_schedule_frame(output);
	wlr_damage_ring_add(&scene_output->damage_ring, &damage);
	/* pending_commit_damage lives in the WLR_PRIVATE member — reaching in is
	 * the same deal as the pointer_state accesses elsewhere in this file. */
	pixman_region32_union(&scene_output->WLR_PRIVATE.pending_commit_damage,
		&scene_output->WLR_PRIVATE.pending_commit_damage, &damage);
	pixman_region32_fini(&damage);
}

struct wl_client *river_wlr_seat_client_get_client(struct wlr_seat_client *client) {
	return client->client;
}

struct wlr_keyboard *river_wlr_seat_get_keyboard(struct wlr_seat *seat) {
	return seat->keyboard_state.keyboard;
}

struct wl_global *river_wlr_seat_get_global(struct wlr_seat *seat) {
	return seat->global;
}

struct wl_signal *river_wlr_backend_get_new_input_signal(struct wlr_backend *backend) {
	return &backend->events.new_input;
}

const struct wlr_surface_role *river_wlr_surface_get_role(struct wlr_surface *surface) {
	return surface->role;
}

struct wl_resource *river_wlr_surface_get_role_resource(struct wlr_surface *surface) {
	return surface->role_resource;
}

void river_wlr_surface_set_role_object(struct wlr_surface *surface, struct wl_resource *role_resource) {
	surface->role_resource = role_resource;
}

struct wlr_surface *river_wlr_xdg_surface_get_surface(struct wlr_xdg_surface *xdg_surface) {
	return xdg_surface->surface;
}

struct wl_signal *river_wlr_xdg_surface_get_ack_configure_signal(struct wlr_xdg_surface *xdg_surface) {
	return &xdg_surface->events.ack_configure;
}

struct wl_signal *river_wlr_xdg_surface_get_new_popup_signal(struct wlr_xdg_surface *xdg_surface) {
	return &xdg_surface->events.new_popup;
}

void river_wlr_xdg_surface_get_geometry(struct wlr_xdg_surface *xdg_surface, struct wlr_box *box) {
	*box = xdg_surface->geometry;
}

bool river_wlr_xdg_surface_get_initial_commit(struct wlr_xdg_surface *xdg_surface) {
	return xdg_surface->initial_commit;
}

bool river_wlr_xdg_surface_get_initialized(struct wlr_xdg_surface *xdg_surface) {
	return xdg_surface->initialized;
}

struct wlr_xdg_surface *river_wlr_xdg_toplevel_get_base(struct wlr_xdg_toplevel *toplevel) {
	return toplevel->base;
}

struct wl_signal *river_wlr_xdg_toplevel_get_destroy_signal(struct wlr_xdg_toplevel *toplevel) {
	return &toplevel->events.destroy;
}

struct wl_signal *river_wlr_xdg_toplevel_get_request_show_window_menu_signal(struct wlr_xdg_toplevel *toplevel) {
	return &toplevel->events.request_show_window_menu;
}

struct wl_signal *river_wlr_xdg_toplevel_get_request_fullscreen_signal(struct wlr_xdg_toplevel *toplevel) {
	return &toplevel->events.request_fullscreen;
}

struct wl_signal *river_wlr_xdg_toplevel_get_request_maximize_signal(struct wlr_xdg_toplevel *toplevel) {
	return &toplevel->events.request_maximize;
}

struct wl_signal *river_wlr_xdg_toplevel_get_request_minimize_signal(struct wlr_xdg_toplevel *toplevel) {
	return &toplevel->events.request_minimize;
}

struct wl_signal *river_wlr_xdg_toplevel_get_request_move_signal(struct wlr_xdg_toplevel *toplevel) {
	return &toplevel->events.request_move;
}

struct wl_signal *river_wlr_xdg_toplevel_get_request_resize_signal(struct wlr_xdg_toplevel *toplevel) {
	return &toplevel->events.request_resize;
}

struct wl_signal *river_wlr_xdg_toplevel_get_set_parent_signal(struct wlr_xdg_toplevel *toplevel) {
	return &toplevel->events.set_parent;
}

struct wl_signal *river_wlr_xdg_toplevel_get_set_title_signal(struct wlr_xdg_toplevel *toplevel) {
	return &toplevel->events.set_title;
}

struct wl_signal *river_wlr_xdg_toplevel_get_set_app_id_signal(struct wlr_xdg_toplevel *toplevel) {
	return &toplevel->events.set_app_id;
}

const char *river_wlr_xdg_toplevel_get_title(struct wlr_xdg_toplevel *toplevel) {
	return toplevel->title;
}

const char *river_wlr_xdg_toplevel_get_app_id(struct wlr_xdg_toplevel *toplevel) {
	return toplevel->app_id;
}

struct wlr_xdg_toplevel *river_wlr_xdg_toplevel_get_parent(struct wlr_xdg_toplevel *toplevel) {
	return toplevel->parent;
}

bool river_wlr_xdg_toplevel_get_requested_fullscreen(struct wlr_xdg_toplevel *toplevel) {
	return toplevel->requested.fullscreen;
}

struct wlr_output *river_wlr_xdg_toplevel_get_requested_fullscreen_output(struct wlr_xdg_toplevel *toplevel) {
	return toplevel->requested.fullscreen_output;
}

bool river_wlr_xdg_toplevel_get_requested_maximized(struct wlr_xdg_toplevel *toplevel) {
	return toplevel->requested.maximized;
}

void river_wlr_xdg_toplevel_get_requested_min_max_size(struct wlr_xdg_toplevel *toplevel, int *min_w, int *min_h, int *max_w, int *max_h) {
	*min_w = toplevel->current.min_width;
	*min_h = toplevel->current.min_height;
	*max_w = toplevel->current.max_width;
	*max_h = toplevel->current.max_height;
}

struct wlr_xdg_surface *river_wlr_xdg_popup_get_base(struct wlr_xdg_popup *popup) {
	return popup->base;
}

struct wl_signal *river_wlr_xdg_popup_get_destroy_signal(struct wlr_xdg_popup *popup) {
	return &popup->events.destroy;
}

struct wl_signal *river_wlr_xdg_popup_get_reposition_signal(struct wlr_xdg_popup *popup) {
	return &popup->events.reposition;
}

void river_wlr_xdg_popup_get_anchor_rect(struct wlr_xdg_popup *popup, struct wlr_box *box) {
	*box = popup->scheduled.rules.anchor_rect;
}

void *river_wlr_xdg_surface_get_data(struct wlr_xdg_surface *xdg_surface) {
	return xdg_surface->data;
}

void river_wlr_xdg_surface_set_data(struct wlr_xdg_surface *xdg_surface, void *data) {
	xdg_surface->data = data;
}

struct wl_list *river_wlr_xdg_surface_get_popups(struct wlr_xdg_surface *xdg_surface) {
	return &xdg_surface->popups;
}

struct wlr_scene_tree *river_wlr_scene_tree_get_parent(struct wlr_scene_tree *tree) {
	return tree->node.parent;
}

struct wl_list *river_scene_tree_get_children(struct wlr_scene_tree *tree) {
	return &tree->children;
}

struct wl_signal *river_wlr_surface_get_map_signal(struct wlr_surface *surface) {
	return &surface->events.map;
}

struct wl_signal *river_wlr_surface_get_unmap_signal(struct wlr_surface *surface) {
	return &surface->events.unmap;
}

struct wl_resource *river_wlr_surface_get_resource(struct wlr_surface *surface) {
	return surface->resource;
}

bool river_wlr_surface_is_mapped(struct wlr_surface *surface) {
	return surface->mapped;
}

struct wl_signal *river_wlr_tablet_v2_tablet_tool_get_set_cursor_signal(struct wlr_tablet_v2_tablet_tool *tool) {
	return &tool->events.set_cursor;
}

struct wlr_surface *river_wlr_tablet_v2_tablet_tool_get_focused_surface(struct wlr_tablet_v2_tablet_tool *tool) {
	return tool->focused_surface;
}

uint32_t river_wlr_tablet_v2_tablet_tool_get_proximity_serial(struct wlr_tablet_v2_tablet_tool *tool) {
	return tool->proximity_serial;
}

bool river_wlr_tablet_v2_tablet_tool_get_is_down(struct wlr_tablet_v2_tablet_tool *tool) {
	return tool->is_down;
}

size_t river_wlr_tablet_v2_tablet_tool_get_num_buttons(struct wlr_tablet_v2_tablet_tool *tool) {
	return tool->num_buttons;
}

struct wlr_tablet_tool *river_wlr_tablet_v2_tablet_tool_get_wlr_tool(struct wlr_tablet_v2_tablet_tool *tool) {
	return tool->wlr_tool;
}

void river_wlr_seat_touch_cancel_all(struct wlr_seat *wlr_seat) {
	struct wlr_touch_point *point, *tmp;
	wl_list_for_each_safe(point, tmp, &wlr_seat->touch_state.touch_points, link) {
		wlr_seat_touch_notify_cancel(wlr_seat, point->client);
	}
}

struct wlr_surface *river_wlr_seat_get_keyboard_focused_surface(struct wlr_seat *seat) {
	return seat->keyboard_state.focused_surface;
}

int river_wlr_surface_get_width(struct wlr_surface *surface) {
	return surface->current.width;
}

int river_wlr_surface_get_height(struct wlr_surface *surface) {
	return surface->current.height;
}

// Commit sequence of a surface's current state. Cheap "has this drawn
// anything new?" key for the status-bar backdrop sampler, which is trying
// hard NOT to read back a texture it has already read.
uint32_t river_wlr_surface_current_seq(struct wlr_surface *surface) {
	return surface->current.seq;
}

void river_wlr_surface_get_buffer_size(struct wlr_surface *surface, int *width, int *height) {
	*width = surface->current.buffer_width;
	*height = surface->current.buffer_height;
}

struct wlr_keyboard *river_wlr_input_method_keyboard_grab_v2_get_keyboard(struct wlr_input_method_keyboard_grab_v2 *grab) {
	return grab->keyboard;
}

struct wl_signal *river_wlr_input_method_keyboard_grab_v2_get_destroy_signal(struct wlr_input_method_keyboard_grab_v2 *grab) {
	return &grab->events.destroy;
}

struct wlr_drag_icon *river_wlr_drag_get_icon(struct wlr_drag *drag) {
	return drag->icon;
}

struct wlr_seat *river_wlr_drag_get_seat(struct wlr_drag *drag) {
	return drag->seat;
}

enum wlr_drag_grab_type river_wlr_drag_get_grab_type(struct wlr_drag *drag) {
	return drag->grab_type;
}

int32_t river_wlr_drag_get_touch_id(struct wlr_drag *drag) {
	return drag->touch_id;
}

struct wlr_data_source *river_wlr_drag_get_source(struct wlr_drag *drag) {
	return drag->source;
}

struct wl_signal *river_wlr_drag_get_destroy_signal(struct wlr_drag *drag) {
	return &drag->events.destroy;
}

struct wlr_seat_client *river_wlr_drag_get_seat_client(struct wlr_drag *drag) {
	return drag->seat_client;
}

void river_wlr_keyboard_init(struct wlr_keyboard *keyboard,
		void (*led_update)(struct wlr_keyboard *keyboard, uint32_t leds),
		const char *name) {
	static struct wlr_keyboard_impl impl;
	static bool impl_initialized = false;
	if (!impl_initialized) {
		impl.name = name;
		impl.led_update = led_update;
		impl_initialized = true;
	}
	wlr_keyboard_init(keyboard, &impl, name);
}

static struct wlr_scene_node *find_blur_node(struct wlr_scene_tree *tree) {
	struct wlr_scene_node *child;
	wl_list_for_each(child, &tree->children, link) {
		if (child->type == WLR_SCENE_NODE_BLUR) {
			return child;
		}
	}
	return NULL;
}

static struct wlr_scene_node *find_optimized_blur_node(struct wlr_scene_tree *tree) {
	struct wlr_scene_node *child;
	wl_list_for_each(child, &tree->children, link) {
		if (child->type == WLR_SCENE_NODE_OPTIMIZED_BLUR) {
			return child;
		}
	}
	return NULL;
}

static void get_size_iterator(struct wlr_scene_buffer *buffer, int sx, int sy, void *user_data) {
	(void)sx;
	(void)sy;
	int *size = (int *)user_data;
	if (buffer->dst_width > size[0]) size[0] = buffer->dst_width;
	if (buffer->dst_height > size[1]) size[1] = buffer->dst_height;
}

static void find_buffer_iterator(struct wlr_scene_buffer *buffer, int sx, int sy, void *user_data) {
	(void)sx;
	(void)sy;
	struct wlr_scene_buffer **result = (struct wlr_scene_buffer **)user_data;
	if (*result == NULL) {
		*result = buffer;
	}
}

// `corner_radius` is in the same (scaled, device) pixels as width/height. It is applied
// here rather than through river_scene_node_set_corner_radius so that a blur node can
// never exist without it: that helper looks the blur up by scanning a tree's direct
// children, so aiming it at the wrong tree silently no-ops, and it was never called at
// all on the viewport-update path. Note the optimized blur node cannot be rounded --
// wlr_scene_optimized_blur has no radius field -- so callers that need rounded corners
// must pass optimized = false.
void river_scene_node_enable_blur(struct wlr_scene_node *node, bool enabled, bool optimized, bool ignore_transparent, int x, int y, int width, int height, int corner_radius) {
	if (node->type != WLR_SCENE_NODE_TREE) {
		return;
	}
	struct wlr_scene_tree *tree = wlr_scene_tree_from_node(node);
	struct wlr_scene_node *opt_blur_node = find_optimized_blur_node(tree);
	struct wlr_scene_node *std_blur_node = find_blur_node(tree);

	if (!enabled) {
		if (opt_blur_node) {
			wlr_scene_node_destroy(opt_blur_node);
		}
		if (std_blur_node) {
			wlr_scene_node_destroy(std_blur_node);
		}
		return;
	}

	if (width <= 0 || height <= 0) {
		int size[2] = {0, 0};
		wlr_scene_node_for_each_buffer(node, get_size_iterator, size);
		width = size[0];
		height = size[1];
		x = 0;
		y = 0;
	}

	if (width <= 0 || height <= 0) {
		if (opt_blur_node) {
			wlr_scene_node_destroy(opt_blur_node);
		}
		if (std_blur_node) {
			wlr_scene_node_destroy(std_blur_node);
		}
		return;
	}

	if (optimized) {
		if (!opt_blur_node) {
			struct wlr_scene_optimized_blur *opt_blur = wlr_scene_optimized_blur_create(tree, width, height);
			if (opt_blur) {
				opt_blur_node = &opt_blur->node;
			}
		} else {
			wlr_scene_optimized_blur_set_size((struct wlr_scene_optimized_blur *)opt_blur_node, width, height);
		}
		if (opt_blur_node) {
			wlr_scene_node_set_position(opt_blur_node, x, y);
		}
	} else {
		if (opt_blur_node) {
			wlr_scene_node_destroy(opt_blur_node);
			opt_blur_node = NULL;
		}
	}

	if (!std_blur_node) {
		struct wlr_scene_blur *std_blur = wlr_scene_blur_create(tree, width, height);
		if (std_blur) {
			std_blur_node = &std_blur->node;
			wlr_scene_blur_set_should_only_blur_bottom_layer(std_blur, optimized);
		}
	} else {
		wlr_scene_blur_set_should_only_blur_bottom_layer((struct wlr_scene_blur *)std_blur_node, optimized);
		wlr_scene_blur_set_size((struct wlr_scene_blur *)std_blur_node, width, height);
	}

	// Set on both the create and the reuse path: a node reused across a resize keeps its
	// radius, but one recreated after a blur toggle would otherwise come back square.
	if (std_blur_node) {
		wlr_scene_blur_set_corner_radius((struct wlr_scene_blur *)std_blur_node, corner_radius);
	}

	if (std_blur_node) {
		wlr_scene_node_set_position(std_blur_node, x, y);
		struct wlr_scene_buffer *source_buffer = NULL;
		if (ignore_transparent) {
			wlr_scene_node_for_each_buffer(node, find_buffer_iterator, &source_buffer);
		}
		wlr_scene_blur_set_transparency_mask_source((struct wlr_scene_blur *)std_blur_node, source_buffer);
	}

	// Ensure correct stack order (from back to front): opt_blur_node -> std_blur_node -> window content.
	// Only reorder when out of order: the lower_to_bottom pair is not idempotent
	// (on an already-ordered tree each call swaps the two nodes, and every swap
	// damages the node's whole window-sized region — this runs per commit and
	// per render_finish, so the no-op path must not touch the scene graph).
	struct wlr_scene_node *want_bottom = opt_blur_node ? opt_blur_node : std_blur_node;
	struct wlr_scene_node *want_second = opt_blur_node ? std_blur_node : NULL;
	bool ordered = want_bottom != NULL
		&& tree->children.next == &want_bottom->link
		&& (want_second == NULL || want_bottom->link.next == &want_second->link);
	if (!ordered) {
		if (std_blur_node) {
			wlr_scene_node_lower_to_bottom(std_blur_node);
		}
		if (opt_blur_node) {
			wlr_scene_node_lower_to_bottom(opt_blur_node);
		}
	}
}

static void set_opacity_iterator(struct wlr_scene_buffer *buffer, int sx, int sy, void *user_data) {
	(void)sx;
	(void)sy;
	float opacity = *(float *)user_data;
	if (buffer->opacity != opacity) {
		wlr_scene_buffer_set_opacity(buffer, opacity);
	}
}

void river_scene_node_set_opacity(struct wlr_scene_node *node, float opacity) {
	wlr_scene_node_for_each_buffer(node, set_opacity_iterator, &opacity);
	if (node->type == WLR_SCENE_NODE_TREE) {
		struct wlr_scene_tree *tree = wlr_scene_tree_from_node(node);
		struct wlr_scene_node *blur_node = find_blur_node(tree);
		if (blur_node && blur_node->type == WLR_SCENE_NODE_BLUR) {
			wlr_scene_blur_set_alpha((struct wlr_scene_blur *)blur_node, opacity);
		}
	}
}

static void set_corner_radius_iterator(struct wlr_scene_buffer *buffer, int sx, int sy, void *user_data) {
	(void)sx;
	(void)sy;
	int radius = *(int *)user_data;
	wlr_scene_buffer_set_corner_radius(buffer, radius);
}

void river_scene_node_set_corner_radius(struct wlr_scene_node *node, int radius) {
	wlr_scene_node_for_each_buffer(node, set_corner_radius_iterator, &radius);
	if (node->type == WLR_SCENE_NODE_TREE) {
		struct wlr_scene_tree *tree = wlr_scene_tree_from_node(node);
		struct wlr_scene_node *blur_node = find_blur_node(tree);
		if (blur_node && blur_node->type == WLR_SCENE_NODE_BLUR) {
			wlr_scene_blur_set_corner_radius((struct wlr_scene_blur *)blur_node, radius);
		}
	}
}

void river_scene_buffer_set_dest_size_if_changed(struct wlr_scene_buffer *scene_buffer, int width, int height) {
	if (scene_buffer->dst_width != width || scene_buffer->dst_height != height) {
		wlr_scene_buffer_set_dest_size(scene_buffer, width, height);
	}
}

void river_scene_node_set_position_if_changed(struct wlr_scene_node *node, int x, int y) {
	if (node->x != x || node->y != y) {
		wlr_scene_node_set_position(node, x, y);
	}
}

void river_scene_rect_set_size_if_changed(struct wlr_scene_rect *rect, int width, int height) {
	if (rect->width != width || rect->height != height) {
		wlr_scene_rect_set_size(rect, width, height);
	}
}

void river_scene_rect_set_corner_radius(struct wlr_scene_rect *rect, int radius) {
	wlr_scene_rect_set_corner_radius(rect, radius);
}

int river_scene_buffer_get_width(struct wlr_scene_buffer *scene_buffer) {
	if (scene_buffer->buffer) {
		return scene_buffer->buffer->width;
	}
	return scene_buffer->dst_width;
}

int river_scene_buffer_get_height(struct wlr_scene_buffer *scene_buffer) {
	if (scene_buffer->buffer) {
		return scene_buffer->buffer->height;
	}
	return scene_buffer->dst_height;
}

/* Scale a surface's opaque region to match a custom buffer dest size and
 * apply it to the scene buffer. The scene keeps opaque regions in SURFACE
 * coordinates and scenefx's occlusion culling only intersects them with the
 * node box — so a zoomed-down window's unscaled opaque rect still covered
 * (almost) the whole scaled node, INCLUDING its translucent CSD shadow
 * margins. Culling then skipped repainting behind the shadow ring: stale
 * pixels showed through the translucent shadow around focused windows when
 * zoomed (worst at the bottom, where Chromium's ring is tallest). */
/* The subsurface clip on a surface buffer (wlr_scene_subsurface_tree_set_clip,
 * applied by the window's apply_surface_clip as the xdg geometry): the part of
 * the surface this buffer shows, in surface coordinates. wlroots crops the
 * buffer's source box to it and sizes/positions the node from it on every
 * commit, so every pass that rewrites a buffer's dest size or position must
 * work from this extent rather than the surface's full size — or it stretches
 * the cropped source back out to the whole surface. False (and an empty box)
 * when the buffer is not a surface's or nothing is clipped. */
bool river_scene_buffer_get_surface_clip(struct wlr_scene_buffer *scene_buffer,
		struct wlr_box *out) {
	*out = (struct wlr_box){0};
	struct wlr_scene_surface *scene_surface = wlr_scene_surface_try_from_buffer(scene_buffer);
	if (!scene_surface) {
		return false;
	}
	*out = scene_surface->WLR_PRIVATE.clip;
	return !wlr_box_empty(out);
}

void river_scene_buffer_set_scaled_opaque_region(struct wlr_scene_buffer *scene_buffer,
		struct wlr_surface *surface, double scale) {
	pixman_region32_t scaled;
	pixman_region32_init(&scaled);
	pixman_region32_copy(&scaled, &surface->opaque_region);
	/* A clipped buffer shows only `clip` of the surface, at its own origin:
	 * bring the region into buffer space before scaling, or the part of it
	 * that lies in the cropped-away margin claims pixels the node never
	 * paints. */
	struct wlr_box clip;
	if (river_scene_buffer_get_surface_clip(scene_buffer, &clip)) {
		pixman_region32_translate(&scaled, -clip.x, -clip.y);
		pixman_region32_intersect_rect(&scaled, &scaled, 0, 0, clip.width, clip.height);
	}
	wlr_region_scale(&scaled, &scaled, (float)scale);
	wlr_scene_buffer_set_opaque_region(scene_buffer, &scaled);
	pixman_region32_fini(&scaled);
}

int river_scene_buffer_get_dest_width(struct wlr_scene_buffer *scene_buffer) {
	return scene_buffer->dst_width;
}

int river_scene_buffer_get_dest_height(struct wlr_scene_buffer *scene_buffer) {
	return scene_buffer->dst_height;
}

bool river_scene_node_get_enabled(struct wlr_scene_node *node) {
	return node->enabled;
}

/* Overview-delay debugging: dump the true scene-side state of every buffer
 * under a node — enabled flags, absolute position, dest size, whether a
 * wlr_buffer/texture is attached, the primary output, and the computed
 * visibility region. The visibility extents are the ground truth for "will
 * this buffer be drawn": an empty region means the scene considers it
 * invisible regardless of what the window-manager state says. */
static void river_ovdbg_buffer_iter(struct wlr_scene_buffer *buffer,
		int sx, int sy, void *data) {
	const char *tag = data;
	struct wlr_scene_node *node = &buffer->node;
	int lx = 0, ly = 0;
	bool coords_en = wlr_scene_node_coords(node, &lx, &ly);
	pixman_box32_t *ext = pixman_region32_extents(&node->WLR_PRIVATE.visible);
	struct timespec ts;
	clock_gettime(CLOCK_REALTIME, &ts);
	fprintf(stderr,
		"[ovdbg] t=%ld.%03ld %s buf=%p en=%d coords_en=%d abs=(%d,%d) "
		"dst=%dx%d bufwh=%dx%d wlrbuf=%p tex=%p primary=%p vis=(%d,%d %dx%d)\n",
		(long)ts.tv_sec, ts.tv_nsec / 1000000, tag, (void *)buffer,
		node->enabled, coords_en, lx, ly,
		buffer->dst_width, buffer->dst_height,
		buffer->WLR_PRIVATE.buffer_width, buffer->WLR_PRIVATE.buffer_height,
		(void *)buffer->buffer, (void *)buffer->WLR_PRIVATE.texture,
		(void *)buffer->primary_output,
		ext->x1, ext->y1, ext->x2 - ext->x1, ext->y2 - ext->y1);
}

void river_scene_ovdbg_dump(struct wlr_scene_node *node, const char *tag) {
	wlr_scene_node_for_each_buffer(node, river_ovdbg_buffer_iter, (void *)tag);
}

/* Overview-delay/shadow debugging: report a window's drop-shadow node state so
 * it can be compared against the window's current zoom scale. Everything here
 * is device px, as update_shadow writes it. */
void river_scene_shadow_dbg(struct wlr_scene_shadow *shadow, const char *tag) {
	if (!shadow) {
		fprintf(stderr, "[ovdbg] %s shadow=NULL\n", tag);
		return;
	}
	struct wlr_scene_node *node = &shadow->node;
	fprintf(stderr,
		"[ovdbg] %s shadow en=%d pos=(%d,%d) size=%dx%d sigma=%.1f radius=%d "
		"clip=(%d,%d %dx%d)\n",
		tag, node->enabled, node->x, node->y, shadow->width, shadow->height,
		shadow->blur_sigma, shadow->corner_radius,
		shadow->clipped_region.area.x, shadow->clipped_region.area.y,
		shadow->clipped_region.area.width, shadow->clipped_region.area.height);
}

/* ------------------------------------------------------------------
 * CPU-backed buffer: lets the compositor hand the renderer pixels it
 * rasterized itself (the desktop-grid square labels). wlroots has no public
 * constructor for this, so implement the minimal wlr_buffer: the renderer
 * reaches the pixels through data-ptr access and uploads them like any shm
 * buffer. The data is copied in, so the caller's Rust Vec can be dropped.
 * ------------------------------------------------------------------ */
struct cce_data_buffer {
	struct wlr_buffer base;
	void *data;
	uint32_t format;
	size_t stride;
};

static void cce_data_buffer_destroy(struct wlr_buffer *wlr_buffer) {
	struct cce_data_buffer *buf = (struct cce_data_buffer *)wlr_buffer;
	free(buf->data);
	free(buf);
}

static bool cce_data_buffer_begin_data_ptr_access(struct wlr_buffer *wlr_buffer,
		uint32_t flags, void **data, uint32_t *format, size_t *stride) {
	struct cce_data_buffer *buf = (struct cce_data_buffer *)wlr_buffer;
	if (flags & WLR_BUFFER_DATA_PTR_ACCESS_WRITE) {
		return false; /* immutable once built */
	}
	*data = buf->data;
	*format = buf->format;
	*stride = buf->stride;
	return true;
}

static void cce_data_buffer_end_data_ptr_access(struct wlr_buffer *wlr_buffer) {
	/* nothing to unmap */
}

static const struct wlr_buffer_impl cce_data_buffer_impl = {
	.destroy = cce_data_buffer_destroy,
	.begin_data_ptr_access = cce_data_buffer_begin_data_ptr_access,
	.end_data_ptr_access = cce_data_buffer_end_data_ptr_access,
};

/* Copy `data` (ARGB8888, premultiplied, `stride` bytes per row) into a new
 * buffer. Returns NULL on allocation failure. The buffer starts with one
 * reference, as wlr_buffer_init leaves it: pass it to a scene buffer and then
 * drop this reference with wlr_buffer_drop(). */
struct wlr_buffer *river_data_buffer_create(int width, int height,
		size_t stride, const void *data) {
	if (width <= 0 || height <= 0 || stride == 0) {
		return NULL;
	}
	struct cce_data_buffer *buf = calloc(1, sizeof(*buf));
	if (!buf) {
		return NULL;
	}
	size_t size = stride * (size_t)height;
	buf->data = malloc(size);
	if (!buf->data) {
		free(buf);
		return NULL;
	}
	memcpy(buf->data, data, size);
	buf->format = DRM_FORMAT_ARGB8888;
	buf->stride = stride;
	wlr_buffer_init(&buf->base, &cce_data_buffer_impl, width, height);
	return &buf->base;
}

/* Mark every optimized-blur node in the scene dirty, forcing a re-bake of the
 * shared blurred-backdrop cache on the next frame. The cache is otherwise
 * invalidated only by blur-parameter setters and window blur RESIZES
 * (optimized_blur_set_size marks dirty; a same-size re-enable does not), so
 * backdrop CONTENT that changes without either — the grid client latching a
 * new patch after the viewport has settled, or the client<->fallback grid
 * swap — leaves every translucent window showing a stale bake. */
static void mark_optimized_blur_dirty_rec(struct wlr_scene_node *node) {
	if (node->type == WLR_SCENE_NODE_OPTIMIZED_BLUR) {
		wlr_scene_optimized_blur_mark_dirty(wlr_scene_optimized_blur_from_node(node));
		return;
	}
	if (node->type == WLR_SCENE_NODE_TREE) {
		struct wlr_scene_tree *tree = wlr_scene_tree_from_node(node);
		struct wlr_scene_node *child;
		wl_list_for_each(child, &tree->children, link) {
			mark_optimized_blur_dirty_rec(child);
		}
	}
}

void river_scene_mark_optimized_blur_dirty(struct wlr_scene *scene) {
	mark_optimized_blur_dirty_rec(&scene->tree.node);
}

/* See wlr_scene.blur_frozen: suspend the moved-node-below blur
 * invalidation for the duration of a camera pan. Thawing marks every
 * optimized blur dirty once so the settled frame re-bakes against the
 * final backdrop. */
/* See wlr_scene.blur_frozen. Blurs sample the shared cache where their own
 * bake lives, frozen or not, and bake the strips they newly expose as they
 * travel, so a pan needs no re-bake at either end: the thaw is just the
 * flag. */
void river_scene_set_blur_frozen(struct wlr_scene *scene, bool frozen) {
	scene->blur_frozen = frozen;
}
