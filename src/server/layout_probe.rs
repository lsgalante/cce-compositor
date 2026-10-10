//! The hand-written `#[repr(C)]` mirrors, checked against the C compiler.
//!
//! `layout_probe.c` reports, for every field a mirror declares, the offset
//! the real header gives it (and the size, where the mirror is the whole
//! struct). This test computes the same names from the Rust mirrors and
//! fails on any difference, or on a name only one side lists. The mirrors
//! exist because bindgen leaves some wlroots structs opaque and others are
//! blocklisted (build.rs) and written out in `ffi.rs`; without this, a field
//! a wlroots or scenefx update inserted ahead of `events` would have been
//! read as a signal in silence.

use std::collections::BTreeMap;
use std::ffi::{c_char, CStr};
use std::mem::{offset_of, size_of};

use crate::ffi;
use crate::output_manager::{WlrOutputManagerV1, WlrOutputPowerManagerV1};
use crate::server::{
    WlList, WlListener, WlrBackend, WlrCursorShapeManagerV1, WlrExtForeignToplevelImageCaptureSourceManagerV1,
    WlrRenderer, WlrXdgActivationV1, WlrXdgDecorationManagerV1, WlrXdgShell, WlrXwayland,
};

#[repr(C)]
struct Entry {
    name: *const c_char,
    value: usize,
}

extern "C" {
    static cce_layout_probe: [Entry; 0];
}

fn c_table() -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    // SAFETY: the C array ends with a { NULL, 0 } entry, and every name
    // before it is a string literal.
    unsafe {
        let mut p = std::ptr::addr_of!(cce_layout_probe) as *const Entry;
        while !(*p).name.is_null() {
            out.insert(CStr::from_ptr((*p).name).to_string_lossy().into_owned(), (*p).value);
            p = p.add(1);
        }
    }
    out
}

macro_rules! table {
    ($( $name:literal => $value:expr ),* $(,)?) => {{
        let mut t = BTreeMap::new();
        $( t.insert($name.to_string(), $value); )*
        t
    }};
}

fn rust_table() -> BTreeMap<String, usize> {
    table! {
        "wl_list.size" => size_of::<ffi::wl_list>(),
        "wl_list.next" => offset_of!(ffi::wl_list, next),
        "wl_listener.size" => size_of::<ffi::wl_listener>(),
        "wl_listener.link" => offset_of!(ffi::wl_listener, link),
        "wl_listener.notify" => offset_of!(ffi::wl_listener, notify),
        "wl_interface.name" => 0,

        "pixman_box32.size" => size_of::<ffi::pixman_box32>(),
        "pixman_box32.y2" => offset_of!(ffi::pixman_box32, y2),
        "pixman_region32.size" => size_of::<ffi::pixman_region32>(),
        "pixman_region32.extents" => offset_of!(ffi::pixman_region32, extents),
        "pixman_region32.data" => offset_of!(ffi::pixman_region32, data),
        "pixman_rectangle32.size" => size_of::<ffi::pixman_rectangle32>(),
        "pixman_rectangle32.width" => offset_of!(ffi::pixman_rectangle32, width),
        "pixman_rectangle32.height" => offset_of!(ffi::pixman_rectangle32, height),

        "wlr_input_device.size" => size_of::<ffi::wlr_input_device>(),
        "wlr_input_device.type" => offset_of!(ffi::wlr_input_device, type_),
        "wlr_input_device.name" => offset_of!(ffi::wlr_input_device, name),
        "wlr_input_device.events.destroy" => offset_of!(ffi::wlr_input_device, events.destroy),
        "wlr_input_device.data" => offset_of!(ffi::wlr_input_device, data),
        "wlr_addon.size" => size_of::<ffi::wlr_addon>(),
        "wlr_addon.impl" => offset_of!(ffi::wlr_addon, impl_),
        "wlr_addon.owner" => offset_of!(ffi::wlr_addon, owner),
        "wlr_addon.link" => offset_of!(ffi::wlr_addon, link),

        "wlr_renderer.render_buffer_caps" => offset_of!(WlrRenderer, render_buffer_caps),
        "wlr_renderer.events.destroy" => offset_of!(WlrRenderer, events.destroy),
        "wlr_renderer.events.lost" => offset_of!(WlrRenderer, events.lost),
        "wlr_renderer.features.output_color_transform" => offset_of!(WlrRenderer, features.output_color_transform),
        "wlr_renderer.features.timeline" => offset_of!(WlrRenderer, features.timeline),

        "wlr_backend.impl" => offset_of!(WlrBackend, impl_),
        "wlr_backend.buffer_caps" => offset_of!(WlrBackend, buffer_caps),
        "wlr_backend.features.timeline" => offset_of!(WlrBackend, features.timeline),
        "wlr_backend.events.destroy" => offset_of!(WlrBackend, events.destroy),
        "wlr_backend.events.new_input" => offset_of!(WlrBackend, events.new_input),
        "wlr_backend.events.new_output" => offset_of!(WlrBackend, events.new_output),

        "wlr_xdg_shell.global" => offset_of!(WlrXdgShell, global),
        "wlr_xdg_shell.version" => offset_of!(WlrXdgShell, version),
        "wlr_xdg_shell.clients" => offset_of!(WlrXdgShell, clients),
        "wlr_xdg_shell.popup_grabs" => offset_of!(WlrXdgShell, popup_grabs),
        "wlr_xdg_shell.ping_timeout" => offset_of!(WlrXdgShell, ping_timeout),
        "wlr_xdg_shell.events.new_surface" => offset_of!(WlrXdgShell, events.new_surface),
        "wlr_xdg_shell.events.new_toplevel" => offset_of!(WlrXdgShell, events.new_toplevel),
        "wlr_xdg_shell.events.new_popup" => offset_of!(WlrXdgShell, events.new_popup),
        "wlr_xdg_shell.events.destroy" => offset_of!(WlrXdgShell, events.destroy),

        "wlr_xdg_decoration_manager_v1.global" => offset_of!(WlrXdgDecorationManagerV1, global),
        "wlr_xdg_decoration_manager_v1.decorations" => offset_of!(WlrXdgDecorationManagerV1, decorations),
        "wlr_xdg_decoration_manager_v1.events.new_toplevel_decoration" =>
            offset_of!(WlrXdgDecorationManagerV1, events.new_toplevel_decoration),
        "wlr_xdg_decoration_manager_v1.events.destroy" => offset_of!(WlrXdgDecorationManagerV1, events.destroy),

        "wlr_xdg_activation_v1.global" => offset_of!(WlrXdgActivationV1, global),
        "wlr_xdg_activation_v1.token_timeout_msec" => offset_of!(WlrXdgActivationV1, token_timeout_msec),
        "wlr_xdg_activation_v1.tokens" => offset_of!(WlrXdgActivationV1, tokens),
        "wlr_xdg_activation_v1.events.destroy" => offset_of!(WlrXdgActivationV1, events.destroy),
        "wlr_xdg_activation_v1.events.request_activate" => offset_of!(WlrXdgActivationV1, events.request_activate),
        "wlr_xdg_activation_v1.events.new_token" => offset_of!(WlrXdgActivationV1, events.new_token),

        "wlr_cursor_shape_manager_v1.global" => offset_of!(WlrCursorShapeManagerV1, global),
        "wlr_cursor_shape_manager_v1.events.request_set_shape" =>
            offset_of!(WlrCursorShapeManagerV1, events.request_set_shape),
        "wlr_cursor_shape_manager_v1.events.destroy" => offset_of!(WlrCursorShapeManagerV1, events.destroy),

        "wlr_ext_foreign_toplevel_image_capture_source_manager_v1.global" =>
            offset_of!(WlrExtForeignToplevelImageCaptureSourceManagerV1, global),
        "wlr_ext_foreign_toplevel_image_capture_source_manager_v1.events.destroy" =>
            offset_of!(WlrExtForeignToplevelImageCaptureSourceManagerV1, events.destroy),
        "wlr_ext_foreign_toplevel_image_capture_source_manager_v1.events.new_request" =>
            offset_of!(WlrExtForeignToplevelImageCaptureSourceManagerV1, events.new_request),

        "wlr_xwayland.server" => offset_of!(WlrXwayland, server),
        "wlr_xwayland.own_server" => offset_of!(WlrXwayland, own_server),
        "wlr_xwayland.xwm" => offset_of!(WlrXwayland, xwm),
        "wlr_xwayland.shell_v1" => offset_of!(WlrXwayland, shell_v1),
        "wlr_xwayland.display_name" => offset_of!(WlrXwayland, display_name),
        "wlr_xwayland.wl_display" => offset_of!(WlrXwayland, wl_display),
        "wlr_xwayland.compositor" => offset_of!(WlrXwayland, compositor),
        "wlr_xwayland.seat" => offset_of!(WlrXwayland, seat),
        "wlr_xwayland.events.destroy" => offset_of!(WlrXwayland, events.destroy),
        "wlr_xwayland.events.ready" => offset_of!(WlrXwayland, events.ready),
        "wlr_xwayland.events.new_surface" => offset_of!(WlrXwayland, events.new_surface),
        "wlr_xwayland.events.remove_startup_info" => offset_of!(WlrXwayland, events.remove_startup_info),

        "wlr_output_manager_v1.display" => offset_of!(WlrOutputManagerV1, display),
        "wlr_output_manager_v1.global" => offset_of!(WlrOutputManagerV1, global),
        "wlr_output_manager_v1.resources" => offset_of!(WlrOutputManagerV1, resources),
        "wlr_output_manager_v1.heads" => offset_of!(WlrOutputManagerV1, heads),
        "wlr_output_manager_v1.serial" => offset_of!(WlrOutputManagerV1, serial),
        "wlr_output_manager_v1.current_configuration_dirty" =>
            offset_of!(WlrOutputManagerV1, current_configuration_dirty),
        "wlr_output_manager_v1.events.apply" => offset_of!(WlrOutputManagerV1, events.apply),
        "wlr_output_manager_v1.events.test" => offset_of!(WlrOutputManagerV1, events.test),
        "wlr_output_manager_v1.events.destroy" => offset_of!(WlrOutputManagerV1, events.destroy),

        "wlr_output_power_manager_v1.global" => offset_of!(WlrOutputPowerManagerV1, global),
        "wlr_output_power_manager_v1.output_powers" => offset_of!(WlrOutputPowerManagerV1, output_powers),
        "wlr_output_power_manager_v1.events.set_mode" => offset_of!(WlrOutputPowerManagerV1, events.set_mode),
        "wlr_output_power_manager_v1.events.destroy" => offset_of!(WlrOutputPowerManagerV1, events.destroy),
    }
}

#[test]
fn every_hand_written_mirror_matches_the_c_layout() {
    let c = c_table();
    let rust = rust_table();
    let mut wrong = Vec::new();
    for name in c.keys().chain(rust.keys()).collect::<std::collections::BTreeSet<_>>() {
        match (c.get(name), rust.get(name)) {
            (Some(a), Some(b)) if a == b => {}
            (a, b) => wrong.push(format!("{name}: C {a:?}, Rust {b:?}")),
        }
    }
    assert!(wrong.is_empty(), "mirrors out of step with the headers:\n  {}", wrong.join("\n  "));
}

#[test]
fn the_crate_local_list_mirrors_match_the_ffi_ones() {
    assert_eq!(size_of::<WlList>(), size_of::<ffi::wl_list>());
    assert_eq!(size_of::<WlListener>(), size_of::<ffi::wl_listener>());
    assert_eq!(offset_of!(WlListener, notify), offset_of!(ffi::wl_listener, notify));
}
