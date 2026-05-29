#ifndef WRAPPER_H
#define WRAPPER_H

// Wayland Server headers
#include <wayland-server.h>

// Wlroots base headers
#include <wlr/backend.h>
#include <wlr/backend/wayland.h>
#include <wlr/backend/x11.h>
#include <wlr/backend/session.h>
#include <wlr/types/wlr_keyboard.h>
#include <wlr/interfaces/wlr_keyboard.h>
#include <wlr/render/allocator.h>
#include <wlr/render/wlr_renderer.h>
#include <wlr/render/pixman.h>

// Wlroots types
// #include <wlr/types/wlr_fixes.h>
#include <wlr/types/wlr_compositor.h>
#include <wlr/types/wlr_subcompositor.h>
#include <wlr/types/wlr_data_device.h>
#include <wlr/types/wlr_primary_selection.h>
#include <wlr/types/wlr_primary_selection_v1.h>
#include <wlr/types/wlr_screencopy_v1.h>
#include <wlr/types/wlr_export_dmabuf_v1.h>
#include <wlr/types/wlr_xdg_shell.h>
#include <wlr/types/wlr_xdg_decoration_v1.h>
#include <wlr/types/wlr_xdg_activation_v1.h>
#include <wlr/types/wlr_xdg_foreign_registry.h>
#include <wlr/types/wlr_xdg_foreign_v2.h>
#include <wlr/types/wlr_layer_shell_v1.h>
#include <wlr/types/wlr_presentation_time.h>
#include <wlr/types/wlr_relative_pointer_v1.h>
#include <wlr/types/wlr_pointer_constraints_v1.h>
#include <wlr/types/wlr_tablet_tool.h>
#include <wlr/types/wlr_tablet_v2.h>
#include <wlr/types/wlr_pointer_gestures_v1.h>
#include <wlr/types/wlr_idle_inhibit_v1.h>
#include <wlr/types/wlr_idle_notify_v1.h>
#include <wlr/types/wlr_virtual_pointer_v1.h>
#include <wlr/types/wlr_virtual_keyboard_v1.h>
#include <wlr/types/wlr_input_method_v2.h>
#include <wlr/types/wlr_text_input_v3.h>
#include <wlr/types/wlr_ext_foreign_toplevel_list_v1.h>
#include <wlr/types/wlr_foreign_toplevel_management_v1.h>
#include <wlr/types/wlr_ext_data_control_v1.h>
#include <wlr/types/wlr_data_control_v1.h>
#include <wlr/types/wlr_output_power_management_v1.h>
#include <wlr/types/wlr_gamma_control_v1.h>
#include <wlr/types/wlr_output.h>
#include <wlr/types/wlr_output_layout.h>
#include <wlr/types/wlr_cursor.h>
#include <wlr/types/wlr_touch.h>
#include <wlr/types/wlr_seat.h>
#include <wlr/types/wlr_xcursor_manager.h>
#include <wlr/types/wlr_xdg_output_v1.h>
#include <wlr/types/wlr_output_management_v1.h>
#include <wlr/types/wlr_output_swapchain_manager.h>
#include <wlr/types/wlr_fractional_scale_v1.h>
#include <wlr/types/wlr_cursor_shape_v1.h>
#include <wlr/types/wlr_tearing_control_v1.h>
#include <wlr/types/wlr_viewporter.h>
#include <wlr/types/wlr_ext_image_copy_capture_v1.h>
#include <wlr/types/wlr_ext_image_capture_source_v1.h>
#include <wlr/types/wlr_session_lock_v1.h>
#include <wlr/types/wlr_security_context_v1.h>
#include <wlr/types/wlr_shm.h>
#include <wlr/types/wlr_single_pixel_buffer_v1.h>
#include <wlr/types/wlr_alpha_modifier_v1.h>
#include <wlr/types/wlr_color_management_v1.h>
// #include <wlr/types/wlr_color_representation_v1.h>
#include <wlr/types/wlr_linux_dmabuf_v1.h>
#include <wlr/types/wlr_linux_drm_syncobj_v1.h>
#include <scenefx/types/wlr_scene.h>
#include <scenefx/render/fx_renderer/fx_renderer.h>

#include <wlr/util/addon.h>
#include <wlr/xwayland.h>

// Wlroots utils
#include <wlr/util/log.h>
#include <wlr/util/edges.h>
#include <wlr/util/box.h>
#include <wlr/util/region.h>

// System helper libraries
#include <xkbcommon/xkbcommon.h>
#include <libinput.h>
#include <libevdev/libevdev.h>
#include <pixman.h>

// Custom and upstream protocols generated via wayland-scanner
#include "river-window-management-v1-protocol.h"
#include "river-xkb-bindings-v1-protocol.h"
#include "river-layer-shell-v1-protocol.h"
#include "river-input-management-v1-protocol.h"
#include "river-libinput-config-v1-protocol.h"
#include "river-xkb-config-v1-protocol.h"
#include "virtual-keyboard-unstable-v1-protocol.h"
#include <wlr/backend/libinput.h>

// Custom FFI helpers defined in wlroots_log_wrapper.c
struct wlr_surface *river_scene_node_get_surface(struct wlr_scene_node *node);
enum wlr_scene_node_type river_scene_node_get_type(struct wlr_scene_node *node);
struct wlr_scene_tree *river_scene_node_get_parent(struct wlr_scene_node *node);
int river_scene_node_get_x(struct wlr_scene_node *node);
int river_scene_node_get_y(struct wlr_scene_node *node);
void *river_scene_node_get_data(struct wlr_scene_node *node);
void river_scene_node_set_data(struct wlr_scene_node *node, void *data);
struct wl_signal *river_scene_node_get_destroy_signal(struct wlr_scene_node *node);
void river_scene_tree_save_buffers(struct wlr_scene_tree *tree, struct wlr_scene_tree *saved_tree);
void river_scene_tree_clear_children(struct wlr_scene_tree *tree);
struct wlr_scene_node *river_scene_node_from_children_link(struct wl_list *link);

struct wl_signal *river_wlr_output_get_destroy_signal(struct wlr_output *output);
struct wl_signal *river_wlr_output_get_request_state_signal(struct wlr_output *output);
struct wl_signal *river_wlr_output_get_frame_signal(struct wlr_output *output);
struct wl_signal *river_wlr_output_get_present_signal(struct wlr_output *output);
void *river_wlr_output_get_data(struct wlr_output *output);
void river_wlr_output_set_data(struct wlr_output *output, void *data);
const char *river_wlr_output_get_name(struct wlr_output *output);
enum wlr_output_adaptive_sync_status river_wlr_output_get_adaptive_sync_status(struct wlr_output *output);

bool river_wlr_output_get_enabled(struct wlr_output *output);
struct wlr_output_mode *river_wlr_output_get_current_mode(struct wlr_output *output);
int32_t river_wlr_output_get_width(struct wlr_output *output);
int32_t river_wlr_output_get_height(struct wlr_output *output);
int32_t river_wlr_output_get_refresh(struct wlr_output *output);
struct wl_global *river_wlr_output_get_global(struct wlr_output *output);

void *river_wlr_surface_get_data(struct wlr_surface *surface);
void river_wlr_surface_set_data(struct wlr_surface *surface, void *data);
struct wl_signal *river_wlr_surface_get_commit_signal(struct wlr_surface *surface);
struct wl_signal *river_wlr_surface_get_map_signal(struct wlr_surface *surface);
struct wl_signal *river_wlr_surface_get_unmap_signal(struct wlr_surface *surface);
struct wl_resource *river_wlr_surface_get_resource(struct wlr_surface *surface);
bool river_wlr_surface_is_mapped(struct wlr_surface *surface);

enum wlr_input_device_type river_wlr_input_device_get_type(struct wlr_input_device *dev);
const char *river_wlr_input_device_get_name(struct wlr_input_device *dev);
struct wl_signal *river_wlr_input_device_get_destroy_signal(struct wlr_input_device *dev);
void *river_wlr_input_device_get_data(struct wlr_input_device *dev);
void river_wlr_input_device_set_data(struct wlr_input_device *dev, void *data);

struct wl_signal *river_wlr_keyboard_get_key_signal(struct wlr_keyboard *kbd);
struct wl_signal *river_wlr_keyboard_get_modifiers_signal(struct wlr_keyboard *kbd);
struct wl_signal *river_wlr_keyboard_get_keymap_signal(struct wlr_keyboard *kbd);
struct xkb_keymap *river_wlr_keyboard_get_keymap(struct wlr_keyboard *kbd);
struct wlr_keyboard_modifiers *river_wlr_keyboard_get_modifiers(struct wlr_keyboard *kbd);
void *river_wlr_keyboard_get_data(struct wlr_keyboard *kbd);
void river_wlr_keyboard_set_data(struct wlr_keyboard *kbd, void *data);

double river_wlr_cursor_get_x(struct wlr_cursor *cursor);
double river_wlr_cursor_get_y(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_motion_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_motion_absolute_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_button_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_axis_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_frame_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_swipe_begin_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_swipe_update_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_swipe_end_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_pinch_begin_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_pinch_update_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_pinch_end_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_hold_begin_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_hold_end_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_touch_down_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_touch_motion_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_touch_up_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_touch_cancel_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_touch_frame_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_tablet_tool_axis_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_tablet_tool_proximity_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_tablet_tool_tip_signal(struct wlr_cursor *cursor);
struct wl_signal *river_wlr_cursor_get_tablet_tool_button_signal(struct wlr_cursor *cursor);

const char *river_wlr_seat_get_name(struct wlr_seat *seat);
void *river_wlr_seat_get_data(struct wlr_seat *seat);
void river_wlr_seat_set_data(struct wlr_seat *seat, void *data);
struct wl_signal *river_wlr_seat_get_request_set_cursor_signal(struct wlr_seat *seat);
struct wl_signal *river_wlr_seat_get_request_set_selection_signal(struct wlr_seat *seat);
struct wl_signal *river_wlr_seat_get_request_start_drag_signal(struct wlr_seat *seat);
struct wl_signal *river_wlr_seat_get_start_drag_signal(struct wlr_seat *seat);
struct wl_signal *river_wlr_seat_get_request_set_primary_selection_signal(struct wlr_seat *seat);
struct wlr_seat_client *river_wlr_seat_get_pointer_focused_client(struct wlr_seat *seat);
struct wl_client *river_wlr_seat_client_get_client(struct wlr_seat_client *client);
struct wlr_keyboard *river_wlr_seat_get_keyboard(struct wlr_seat *seat);
struct wl_global *river_wlr_seat_get_global(struct wlr_seat *seat);
struct wl_signal *river_wlr_backend_get_new_input_signal(struct wlr_backend *backend);

const struct wlr_surface_role *river_wlr_surface_get_role(struct wlr_surface *surface);
struct wl_resource *river_wlr_surface_get_role_resource(struct wlr_surface *surface);
void river_wlr_surface_set_role_object(struct wlr_surface *surface, struct wl_resource *role_resource);

struct wlr_surface *river_wlr_xdg_surface_get_surface(struct wlr_xdg_surface *xdg_surface);
struct wl_signal *river_wlr_xdg_surface_get_ack_configure_signal(struct wlr_xdg_surface *xdg_surface);
struct wl_signal *river_wlr_xdg_surface_get_new_popup_signal(struct wlr_xdg_surface *xdg_surface);
void river_wlr_xdg_surface_get_geometry(struct wlr_xdg_surface *xdg_surface, struct wlr_box *box);
bool river_wlr_xdg_surface_get_initial_commit(struct wlr_xdg_surface *xdg_surface);
bool river_wlr_xdg_surface_get_initialized(struct wlr_xdg_surface *xdg_surface);

struct wlr_xdg_surface *river_wlr_xdg_toplevel_get_base(struct wlr_xdg_toplevel *toplevel);
struct wl_signal *river_wlr_xdg_toplevel_get_destroy_signal(struct wlr_xdg_toplevel *toplevel);
struct wl_signal *river_wlr_xdg_toplevel_get_request_show_window_menu_signal(struct wlr_xdg_toplevel *toplevel);
struct wl_signal *river_wlr_xdg_toplevel_get_request_fullscreen_signal(struct wlr_xdg_toplevel *toplevel);
struct wl_signal *river_wlr_xdg_toplevel_get_request_maximize_signal(struct wlr_xdg_toplevel *toplevel);
struct wl_signal *river_wlr_xdg_toplevel_get_request_minimize_signal(struct wlr_xdg_toplevel *toplevel);
struct wl_signal *river_wlr_xdg_toplevel_get_request_move_signal(struct wlr_xdg_toplevel *toplevel);
struct wl_signal *river_wlr_xdg_toplevel_get_request_resize_signal(struct wlr_xdg_toplevel *toplevel);
struct wl_signal *river_wlr_xdg_toplevel_get_set_parent_signal(struct wlr_xdg_toplevel *toplevel);
struct wl_signal *river_wlr_xdg_toplevel_get_set_title_signal(struct wlr_xdg_toplevel *toplevel);
struct wl_signal *river_wlr_xdg_toplevel_get_set_app_id_signal(struct wlr_xdg_toplevel *toplevel);

const char *river_wlr_xdg_toplevel_get_title(struct wlr_xdg_toplevel *toplevel);
const char *river_wlr_xdg_toplevel_get_app_id(struct wlr_xdg_toplevel *toplevel);
struct wlr_xdg_toplevel *river_wlr_xdg_toplevel_get_parent(struct wlr_xdg_toplevel *toplevel);
bool river_wlr_xdg_toplevel_get_requested_fullscreen(struct wlr_xdg_toplevel *toplevel);
struct wlr_output *river_wlr_xdg_toplevel_get_requested_fullscreen_output(struct wlr_xdg_toplevel *toplevel);
bool river_wlr_xdg_toplevel_get_requested_maximized(struct wlr_xdg_toplevel *toplevel);
void river_wlr_xdg_toplevel_get_requested_min_max_size(struct wlr_xdg_toplevel *toplevel, int *min_w, int *min_h, int *max_w, int *max_h);

struct wlr_xdg_surface *river_wlr_xdg_popup_get_base(struct wlr_xdg_popup *popup);
struct wl_signal *river_wlr_xdg_popup_get_destroy_signal(struct wlr_xdg_popup *popup);
struct wl_signal *river_wlr_xdg_popup_get_reposition_signal(struct wlr_xdg_popup *popup);
void river_wlr_xdg_popup_get_anchor_rect(struct wlr_xdg_popup *popup, struct wlr_box *box);
void *river_wlr_xdg_surface_get_data(struct wlr_xdg_surface *xdg_surface);
void river_wlr_xdg_surface_set_data(struct wlr_xdg_surface *xdg_surface, void *data);
struct wl_list *river_wlr_xdg_surface_get_popups(struct wlr_xdg_surface *xdg_surface);
struct wlr_scene_tree *river_wlr_scene_tree_get_parent(struct wlr_scene_tree *tree);
struct wl_list *river_scene_tree_get_children(struct wlr_scene_tree *tree);

struct wl_signal *river_wlr_tablet_v2_tablet_tool_get_set_cursor_signal(struct wlr_tablet_v2_tablet_tool *tool);
struct wlr_surface *river_wlr_tablet_v2_tablet_tool_get_focused_surface(struct wlr_tablet_v2_tablet_tool *tool);
uint32_t river_wlr_tablet_v2_tablet_tool_get_proximity_serial(struct wlr_tablet_v2_tablet_tool *tool);
bool river_wlr_tablet_v2_tablet_tool_get_is_down(struct wlr_tablet_v2_tablet_tool *tool);
size_t river_wlr_tablet_v2_tablet_tool_get_num_buttons(struct wlr_tablet_v2_tablet_tool *tool);
struct wlr_tablet_tool *river_wlr_tablet_v2_tablet_tool_get_wlr_tool(struct wlr_tablet_v2_tablet_tool *tool);

void river_wlr_seat_touch_cancel_all(struct wlr_seat *wlr_seat);
struct wlr_surface *river_wlr_seat_get_keyboard_focused_surface(struct wlr_seat *seat);

int river_wlr_surface_get_width(struct wlr_surface *surface);
int river_wlr_surface_get_height(struct wlr_surface *surface);
struct wlr_keyboard *river_wlr_input_method_keyboard_grab_v2_get_keyboard(struct wlr_input_method_keyboard_grab_v2 *grab);
struct wl_signal *river_wlr_input_method_keyboard_grab_v2_get_destroy_signal(struct wlr_input_method_keyboard_grab_v2 *grab);

struct wlr_drag_icon *river_wlr_drag_get_icon(struct wlr_drag *drag);
struct wlr_seat *river_wlr_drag_get_seat(struct wlr_drag *drag);
enum wlr_drag_grab_type river_wlr_drag_get_grab_type(struct wlr_drag *drag);
int32_t river_wlr_drag_get_touch_id(struct wlr_drag *drag);
struct wlr_data_source *river_wlr_drag_get_source(struct wlr_drag *drag);
struct wl_signal *river_wlr_drag_get_destroy_signal(struct wlr_drag *drag);
struct wlr_seat_client *river_wlr_drag_get_seat_client(struct wlr_drag *drag);

void river_wlr_keyboard_init(struct wlr_keyboard *keyboard, void (*led_update)(struct wlr_keyboard *keyboard, uint32_t leds), const char *name);

void river_scene_node_enable_blur(struct wlr_scene_node *node, bool enabled);

#endif // WRAPPER_H
