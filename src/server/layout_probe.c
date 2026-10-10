// The real layout of every C struct the compositor mirrors by hand.
//
// Some wlroots structs reach Rust only through hand-written #[repr(C)]
// mirrors: bindgen leaves wlr_renderer, wlr_backend and
// wlr_xdg_activation_v1 opaque, and wl_listener, wlr_addon,
// wlr_input_device and the pixman types are blocklisted in build.rs and
// written out in ffi.rs. Nothing checked those mirrors against the headers
// (bindgen's own layout tests are off, and would not cover them), so a
// field wlroots inserted ahead of `events` would have been read as a signal
// without a word. This table is the compiler's answer for each mirrored
// field; `layout_probe.rs` compares it with the mirrors in a test. A field
// renamed or removed in a header fails right here, at build time.

#define _POSIX_C_SOURCE 200809L
#include <stddef.h>

#include <pixman.h>
#include <wayland-server-core.h>
#include <wlr/backend.h>
#include <wlr/render/wlr_renderer.h>
#include <wlr/types/wlr_cursor_shape_v1.h>
#include <wlr/types/wlr_ext_image_capture_source_v1.h>
#include <wlr/types/wlr_input_device.h>
#include <wlr/types/wlr_output_management_v1.h>
#include <wlr/types/wlr_output_power_management_v1.h>
#include <wlr/types/wlr_xdg_activation_v1.h>
#include <wlr/types/wlr_xdg_decoration_v1.h>
#include <wlr/types/wlr_xdg_shell.h>
#include <wlr/util/addon.h>
#include <wlr/xwayland.h>

struct cce_layout_entry {
	const char *name;
	size_t value;
};

#define SZ(t) { #t ".size", sizeof(struct t) }
#define OF(t, f) { #t "." #f, offsetof(struct t, f) }
// A field wlroots keeps under `struct { ... } WLR_PRIVATE;`: outside
// wlroots' own build that member is literally named WLR_PRIVATE, and the
// mirrors flatten it, which keeps the same layout.
#define OFP(t, f) { #t "." #f, offsetof(struct t, WLR_PRIVATE.f) }

const struct cce_layout_entry cce_layout_probe[] = {
	SZ(wl_list),
	OF(wl_list, next),
	SZ(wl_listener),
	OF(wl_listener, link),
	OF(wl_listener, notify),
	OF(wl_interface, name),

	SZ(pixman_box32),
	OF(pixman_box32, y2),
	SZ(pixman_region32),
	OF(pixman_region32, extents),
	OF(pixman_region32, data),
	SZ(pixman_rectangle32),
	OF(pixman_rectangle32, width),
	OF(pixman_rectangle32, height),

	SZ(wlr_input_device),
	OF(wlr_input_device, type),
	OF(wlr_input_device, name),
	OF(wlr_input_device, events.destroy),
	OF(wlr_input_device, data),
	SZ(wlr_addon),
	OF(wlr_addon, impl),
	OFP(wlr_addon, owner),
	OFP(wlr_addon, link),

	OF(wlr_renderer, render_buffer_caps),
	OF(wlr_renderer, events.destroy),
	OF(wlr_renderer, events.lost),
	OF(wlr_renderer, features.output_color_transform),
	OF(wlr_renderer, features.timeline),

	OF(wlr_backend, impl),
	OF(wlr_backend, buffer_caps),
	OF(wlr_backend, features.timeline),
	OF(wlr_backend, events.destroy),
	OF(wlr_backend, events.new_input),
	OF(wlr_backend, events.new_output),

	OF(wlr_xdg_shell, global),
	OF(wlr_xdg_shell, version),
	OF(wlr_xdg_shell, clients),
	OF(wlr_xdg_shell, popup_grabs),
	OF(wlr_xdg_shell, ping_timeout),
	OF(wlr_xdg_shell, events.new_surface),
	OF(wlr_xdg_shell, events.new_toplevel),
	OF(wlr_xdg_shell, events.new_popup),
	OF(wlr_xdg_shell, events.destroy),

	OF(wlr_xdg_decoration_manager_v1, global),
	OF(wlr_xdg_decoration_manager_v1, decorations),
	OF(wlr_xdg_decoration_manager_v1, events.new_toplevel_decoration),
	OF(wlr_xdg_decoration_manager_v1, events.destroy),

	OF(wlr_xdg_activation_v1, global),
	OF(wlr_xdg_activation_v1, token_timeout_msec),
	OF(wlr_xdg_activation_v1, tokens),
	OF(wlr_xdg_activation_v1, events.destroy),
	OF(wlr_xdg_activation_v1, events.request_activate),
	OF(wlr_xdg_activation_v1, events.new_token),

	OF(wlr_cursor_shape_manager_v1, global),
	OF(wlr_cursor_shape_manager_v1, events.request_set_shape),
	OF(wlr_cursor_shape_manager_v1, events.destroy),

	OF(wlr_ext_foreign_toplevel_image_capture_source_manager_v1, global),
	OF(wlr_ext_foreign_toplevel_image_capture_source_manager_v1, events.destroy),
	OF(wlr_ext_foreign_toplevel_image_capture_source_manager_v1, events.new_request),

	OF(wlr_xwayland, server),
	OF(wlr_xwayland, own_server),
	OF(wlr_xwayland, xwm),
	OF(wlr_xwayland, shell_v1),
	OF(wlr_xwayland, display_name),
	OF(wlr_xwayland, wl_display),
	OF(wlr_xwayland, compositor),
	OF(wlr_xwayland, seat),
	OF(wlr_xwayland, events.destroy),
	OF(wlr_xwayland, events.ready),
	OF(wlr_xwayland, events.new_surface),
	OF(wlr_xwayland, events.remove_startup_info),

	OF(wlr_output_manager_v1, display),
	OF(wlr_output_manager_v1, global),
	OF(wlr_output_manager_v1, resources),
	OF(wlr_output_manager_v1, heads),
	OF(wlr_output_manager_v1, serial),
	OF(wlr_output_manager_v1, current_configuration_dirty),
	OF(wlr_output_manager_v1, events.apply),
	OF(wlr_output_manager_v1, events.test),
	OF(wlr_output_manager_v1, events.destroy),

	OF(wlr_output_power_manager_v1, global),
	OF(wlr_output_power_manager_v1, output_powers),
	OF(wlr_output_power_manager_v1, events.set_mode),
	OF(wlr_output_power_manager_v1, events.destroy),

	{ NULL, 0 },
};
