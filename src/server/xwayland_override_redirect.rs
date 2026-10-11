// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server};
use crate::xwayland_window::XwaylandWindow;

#[repr(C)]
pub struct XwaylandOverrideRedirect {
    pub server: *mut Server,
    pub xsurface: *mut ffi::wlr_xwayland_surface,
    pub surface_tree: crate::scene_handle::SceneTree,

    pub request_configure: crate::listener::Listener,
    pub destroy: crate::listener::Listener,
    pub set_override_redirect: crate::listener::Listener,
    pub associate: crate::listener::Listener,
    pub dissociate: crate::listener::Listener,

    pub map: crate::listener::Listener,
    pub unmap: crate::listener::Listener,

    pub set_geometry: crate::listener::Listener,
    /// Re-applies the 1/scale dest size after every commit (the scene's
    /// commit handler resets it) — see `Window`'s commit handler for why a
    /// per-frame pass alone leaves a hit-testing gap.
    pub commit: crate::listener::Listener,
}

impl XwaylandOverrideRedirect {
    pub unsafe fn create(
        xsurface: *mut ffi::wlr_xwayland_surface,
        server: *mut Server,
    ) -> Result<(), &'static str> {
        let title_ptr = (*xsurface).title;
        let class_ptr = (*xsurface).class;
        log::debug!(
            "new xwayland override redirect: title='{:?}', class='{:?}'",
            if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") },
            if class_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(class_ptr).to_str().unwrap_or("") }
        );

        let override_redirect = Box::new(XwaylandOverrideRedirect {
            server,
            xsurface,
            surface_tree: crate::scene_handle::SceneTree::none(),
            request_configure: std::mem::zeroed(),
            destroy: std::mem::zeroed(),
            set_override_redirect: std::mem::zeroed(),
            associate: std::mem::zeroed(),
            dissociate: std::mem::zeroed(),
            map: std::mem::zeroed(),
            unmap: std::mem::zeroed(),
            set_geometry: std::mem::zeroed(),
            commit: std::mem::zeroed(),
        });

        let raw = Box::into_raw(override_redirect);
        (*server).wm.override_redirects.push(raw);

        (*raw).request_configure.connect(&mut (*xsurface).events.request_configure, handle_request_configure);
        (*raw).destroy.connect(&mut (*xsurface).events.destroy, handle_destroy);
        (*raw).set_override_redirect.connect(&mut (*xsurface).events.set_override_redirect, handle_set_override_redirect);
        (*raw).associate.connect(&mut (*xsurface).events.associate, handle_associate);
        (*raw).dissociate.connect(&mut (*xsurface).events.dissociate, handle_dissociate);

        if !(*xsurface).surface.is_null() {
            handle_associate_impl(raw);
            if ffi::river_wlr_surface_is_mapped((*xsurface).surface) {
                handle_map_impl(raw);
            }
        }

        Ok(())
    }

    /// Put the surface tree where X11 says the window is, in logical
    /// pixels (`xwayland_window::x11_scale`).
    pub unsafe fn place(&mut self) {
        if self.surface_tree.is_null() {
            return;
        }
        let s = crate::xwayland_window::x11_scale_for(self.server, self.xsurface);
        ffi::wlr_scene_node_set_position(
            self.surface_tree.node(),
            crate::xwayland_window::from_x11((*self.xsurface).x as i32, s),
            crate::xwayland_window::from_x11((*self.xsurface).y as i32, s),
        );
    }

    /// Draw the physical-pixel X11 buffer at 1/scale. Runs from the
    /// per-frame pass (output.rs) because the scene's commit listener
    /// resets a committed buffer's dest size; every setter is change-checked.
    pub unsafe fn apply_x11_scale(&mut self) {
        if self.surface_tree.is_null() {
            return;
        }
        let s = crate::xwayland_window::x11_scale_for(self.server, self.xsurface) as f64;
        if s == 1.0 {
            return;
        }
        unsafe extern "C" fn iter(
            buffer: *mut ffi::wlr_scene_buffer,
            _sx: i32,
            _sy: i32,
            user_data: *mut std::ffi::c_void,
        ) {
            let inv = *(user_data as *const f64);
            let node = buffer as *mut ffi::wlr_scene_node;
            let surface = ffi::river_scene_node_get_surface(node);
            if surface.is_null() {
                return;
            }
            let w = ffi::river_wlr_surface_get_width(surface);
            let h = ffi::river_wlr_surface_get_height(surface);
            ffi::river_scene_buffer_set_dest_size_if_changed(
                buffer,
                (w as f64 * inv).round() as i32,
                (h as f64 * inv).round() as i32,
            );
            ffi::river_scene_buffer_set_scaled_opaque_region(buffer, surface, inv);
        }
        let inv = 1.0 / s;
        ffi::wlr_scene_node_for_each_buffer(
            self.surface_tree.node(),
            Some(iter),
            &inv as *const f64 as *mut std::ffi::c_void,
        );
    }

    /// Give the keyboard to this popup when it asks for it, but never take
    /// it from another client: only when the seat holds nothing, or holds
    /// a window or popup of the popup's own process.
    ///
    /// wlroots' `override_redirect_wants_focus` only rules out the window
    /// types a popup is expected to declare, and Wine declares none of
    /// them: its tooltips and menus alike are `_NET_WM_WINDOW_TYPE_DIALOG`
    /// with `WM_TAKE_FOCUS`, and their Win32 styles (`_WINE_HWND_STYLE`)
    /// match too. So a tray icon's tooltip — which Wine's `explorer.exe`
    /// shows on every click the XEmbed bridge forwards — took the keyboard
    /// from whatever the user was typing in until 2026-09-27. An X server's
    /// own window manager never focuses an override-redirect window at
    /// all; the cost of holding back here is keyboard navigation in a menu
    /// whose app has no focused window, e.g. a tray menu, which the pointer
    /// still drives.
    pub unsafe fn focus_if_desired(&self) {
        if (*self.server).lock_manager.state != crate::lock_manager::LockState::Unlocked {
            return;
        }
        if !ffi::wlr_xwayland_surface_override_redirect_wants_focus(self.xsurface)
            || ffi::wlr_xwayland_surface_icccm_input_model(self.xsurface)
                == ffi::wlr_xwayland_icccm_input_model_WLR_ICCCM_INPUT_MODEL_NONE
        {
            return;
        }
        let seat = (*self.server).input_manager.default_seat;
        if seat.is_null() {
            return;
        }
        let pid = (*self.xsurface).pid;
        match (*seat).focused {
            crate::seat::Focus::Window(window) if !window.is_null() => {
                if let crate::window::WindowImpl::Xwayland(xwindow) = (*window).impl_type {
                    if !xwindow.is_null() && (*(*xwindow).xsurface).pid == pid {
                        (*seat).keyboard_enter_or_leave((*self.xsurface).surface);
                        return;
                    }
                }
                log::debug!("override redirect (pid {pid}) mapped unfocused: another client holds the keyboard");
                return;
            }
            crate::seat::Focus::None => {}
            crate::seat::Focus::OverrideRedirect(or) if !or.is_null() && (*(*or).xsurface).pid == pid => {}
            _ => {
                log::debug!("override redirect (pid {pid}) mapped unfocused: another client holds the keyboard");
                return;
            }
        }
        (*seat).focus(crate::seat::Focus::OverrideRedirect(self as *const _ as *mut _));
    }
}

unsafe extern "C" fn handle_request_configure(_listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let event = data as *mut ffi::wlr_xwayland_surface_configure_event;
    ffi::wlr_xwayland_surface_configure((*event).surface, (*event).x, (*event).y, (*event).width, (*event).height);
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, destroy);
    (*(*or).server).wm.override_redirects.retain(|&p| p != or);

    (*or).request_configure.disconnect();
    (*or).destroy.disconnect();
    (*or).associate.disconnect();
    (*or).dissociate.disconnect();
    (*or).set_override_redirect.disconnect();

    let _ = Box::from_raw(or);
}

unsafe extern "C" fn handle_associate(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, associate);
    handle_associate_impl(or);
}

unsafe fn handle_associate_impl(or: *mut XwaylandOverrideRedirect) {
    let surface = (*(*or).xsurface).surface;
    if !surface.is_null() {
        (*or).map.connect(ffi::river_wlr_surface_get_map_signal(surface), handle_map);
        (*or).unmap.connect(ffi::river_wlr_surface_get_unmap_signal(surface), handle_unmap);
    }
}

unsafe extern "C" fn handle_dissociate(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, dissociate);
    (*or).map.disconnect();
    (*or).unmap.disconnect();
}

unsafe extern "C" fn handle_map(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, map);
    handle_map_impl(or);
}

/// WM_CLASS of the XEmbed tray bridge's container windows
/// (`cce-status-interface`'s `cce-xembed-tray`).
const XEMBED_TRAY_CLASS: &str = "cce-xembed-tray";

/// Whether this override-redirect window is one of the tray bridge's
/// containers. Each holds a legacy X11 tray icon the bridge adopted and
/// republishes as a StatusNotifierItem; X needs it mapped for the icon to
/// draw at all, but the icon is shown in the status bar, so the window
/// itself must never be. The bridge also gives it an empty input region,
/// so X never routes the pointer into it either.
unsafe fn is_xembed_tray_container(xsurface: *mut ffi::wlr_xwayland_surface) -> bool {
    let class = (*xsurface).class;
    !class.is_null() && std::ffi::CStr::from_ptr(class).to_bytes() == XEMBED_TRAY_CLASS.as_bytes()
}

unsafe fn handle_map_impl(or: *mut XwaylandOverrideRedirect) {
    // No scene node at all: nothing to draw, hit-test or focus. Unmap
    // copes with the missing tree and the unconnected listeners.
    if is_xembed_tray_container((*or).xsurface) {
        log::debug!("xembed tray container mapped; not shown");
        return;
    }
    let surface = (*(*or).xsurface).surface;
    let override_redirect_tree = (*(*or).server).scene.layers.override_redirect.raw();

    let surface_tree = ffi::wlr_scene_subsurface_tree_create(override_redirect_tree, surface);
    if surface_tree.is_null() {
        log::error!("out of memory creating subsurface tree for override redirect");
        let surface_resource = ffi::river_wlr_surface_get_resource(surface);
        let client = ffi::wl_resource_get_client(surface_resource);
        ffi::wl_client_post_no_memory(client);
        return;
    }
    (*or).surface_tree = crate::scene_handle::SceneTree::adopt(surface_tree);

    crate::scene_node_data::SceneNodeData::attach(
        surface_tree as *mut ffi::wlr_scene_node,
        crate::scene_node_data::SceneNodeDataVal::OverrideRedirect(or as *mut _),
    );

    ffi::river_wlr_surface_set_data(surface, surface_tree as *mut ffi::wlr_scene_node as *mut _);

    (*or).place();
    (*or).apply_x11_scale();

    (*or).set_geometry.connect(&mut (*(*or).xsurface).events.set_geometry, handle_set_geometry);
    // After the scene's subsurface tree, so this runs after its reset.
    (*or).commit.connect(ffi::river_wlr_surface_get_commit_signal(surface), handle_commit);

    (*or).focus_if_desired();
}

unsafe extern "C" fn handle_unmap(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, unmap);

    (*or).set_geometry.disconnect();
    (*or).commit.disconnect();

    let surface = (*(*or).xsurface).surface;
    if !surface.is_null() {
        ffi::river_wlr_surface_set_data(surface, std::ptr::null_mut());
    }

    (*or).surface_tree.destroy();

    let default_seat = (*(*or).server).input_manager.default_seat;
    if !default_seat.is_null() {
        if let crate::seat::Focus::Window(window) = (*default_seat).focused {
            if !window.is_null() && matches!((*window).impl_type, crate::window::WindowImpl::Xwayland(_)) {
                let parent_xwindow = (*window).impl_type;
                if let crate::window::WindowImpl::Xwayland(xwindow) = parent_xwindow {
                    if !xwindow.is_null()
                        && (*(*xwindow).xsurface).pid == (*(*or).xsurface).pid
                        && ffi::river_wlr_seat_get_keyboard_focused_surface((*default_seat).wlr_seat) == surface
                    {
                        (*default_seat).keyboard_enter_or_leave((*window).root_surface());
                    }
                }
            }
        }
    }

    (*(*or).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, commit);
    (*or).apply_x11_scale();
}

unsafe extern "C" fn handle_set_geometry(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, set_geometry);
    (*or).place();
}

unsafe extern "C" fn handle_set_override_redirect(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, set_override_redirect);
    let xsurface = (*or).xsurface;
    log::debug!("xwayland surface unset override redirect");
    assert!(!(*xsurface).override_redirect);

    let surface = (*xsurface).surface;
    if !surface.is_null() {
        if ffi::river_wlr_surface_is_mapped(surface) {
            // handle unmap inline
            (*or).set_geometry.disconnect();
    (*or).commit.disconnect();
            ffi::river_wlr_surface_set_data(surface, std::ptr::null_mut());
            (*or).surface_tree.destroy();
        }
        (*or).map.disconnect();
        (*or).unmap.disconnect();
    }

    let server = (*or).server;

    // Destroy this OR instance. Drop it from the list first, as
    // `handle_destroy` does: the per-frame `apply_x11_scale` pass walks
    // `override_redirects`, and a freed entry left there crashed the
    // compositor on its next frame (2026-09-26, a Wine tray icon handed
    // back by the XEmbed bridge and remapped as a managed window;
    // `verify/clients` `or-flip` reproduces it).
    (*server).wm.override_redirects.retain(|&p| p != or);
    (*or).request_configure.disconnect();
    (*or).destroy.disconnect();
    (*or).associate.disconnect();
    (*or).dissociate.disconnect();
    (*or).set_override_redirect.disconnect();
    let _ = Box::from_raw(or);

    if let Err(e) = XwaylandWindow::create(xsurface, server) {
        log::error!("Failed to transition OR to XwaylandWindow: {}", e);
    }
}
