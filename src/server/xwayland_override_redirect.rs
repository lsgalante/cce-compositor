// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, wl_signal_add};
use crate::xwayland_window::XwaylandWindow;

#[repr(C)]
pub struct XwaylandOverrideRedirect {
    pub server: *mut Server,
    pub xsurface: *mut ffi::wlr_xwayland_surface,
    pub surface_tree: *mut ffi::wlr_scene_tree,

    pub request_configure: ffi::wl_listener,
    pub destroy: ffi::wl_listener,
    pub set_override_redirect: ffi::wl_listener,
    pub associate: ffi::wl_listener,
    pub dissociate: ffi::wl_listener,

    pub map: ffi::wl_listener,
    pub unmap: ffi::wl_listener,

    pub set_geometry: ffi::wl_listener,
}

unsafe fn connect_listener(
    signal: *mut ffi::wl_signal,
    listener: *mut ffi::wl_listener,
    callback: unsafe extern "C" fn(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void),
) {
    let wl_lis = listener as *mut WlListener;
    (*wl_lis).notify = Some(callback);
    wl_signal_add(signal, listener);
}

unsafe fn wl_listener_remove_safe(listener: *mut ffi::wl_listener) {
    let prev = (*listener).link.prev;
    let next = (*listener).link.next;
    if !prev.is_null() && !next.is_null() && prev != listener as *mut ffi::wl_list && next != listener as *mut ffi::wl_list {
        ffi::wl_list_remove(&mut (*listener).link);
        (*listener).link.prev = std::ptr::null_mut();
        (*listener).link.next = std::ptr::null_mut();
    }
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
            surface_tree: std::ptr::null_mut(),
            request_configure: std::mem::zeroed(),
            destroy: std::mem::zeroed(),
            set_override_redirect: std::mem::zeroed(),
            associate: std::mem::zeroed(),
            dissociate: std::mem::zeroed(),
            map: std::mem::zeroed(),
            unmap: std::mem::zeroed(),
            set_geometry: std::mem::zeroed(),
        });

        let raw = Box::into_raw(override_redirect);

        connect_listener(&mut (*xsurface).events.request_configure, &mut (*raw).request_configure, handle_request_configure);
        connect_listener(&mut (*xsurface).events.destroy, &mut (*raw).destroy, handle_destroy);
        connect_listener(&mut (*xsurface).events.set_override_redirect, &mut (*raw).set_override_redirect, handle_set_override_redirect);
        connect_listener(&mut (*xsurface).events.associate, &mut (*raw).associate, handle_associate);
        connect_listener(&mut (*xsurface).events.dissociate, &mut (*raw).dissociate, handle_dissociate);

        if !(*xsurface).surface.is_null() {
            handle_associate_impl(raw);
            if ffi::river_wlr_surface_is_mapped((*xsurface).surface) {
                handle_map_impl(raw);
            }
        }

        Ok(())
    }

    pub unsafe fn focus_if_desired(&self) {
        if (*self.server).lock_manager.state != crate::lock_manager::LockState::Unlocked {
            return;
        }

        if ffi::wlr_xwayland_surface_override_redirect_wants_focus(self.xsurface)
            && ffi::wlr_xwayland_surface_icccm_input_model(self.xsurface)
                != ffi::wlr_xwayland_icccm_input_model_WLR_ICCCM_INPUT_MODEL_NONE
        {
            let seat = (*self.server).input_manager.default_seat;
            if !seat.is_null() {
                if let crate::seat::Focus::Window(window) = (*seat).focused {
                    if !window.is_null() && matches!((*window).impl_type, crate::window::WindowImpl::Xwayland(_)) {
                        let parent_xwindow = (*window).impl_type;
                        if let crate::window::WindowImpl::Xwayland(xwindow) = parent_xwindow {
                            if !xwindow.is_null() && (*(*xwindow).xsurface).pid == (*self.xsurface).pid {
                                (*seat).keyboard_enter_or_leave((*self.xsurface).surface);
                                return;
                            }
                        }
                    }
                }
                (*seat).focus(crate::seat::Focus::OverrideRedirect(self as *const _ as *mut _));
            }
        }
    }
}

unsafe extern "C" fn handle_request_configure(_listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let event = data as *mut ffi::wlr_xwayland_surface_configure_event;
    ffi::wlr_xwayland_surface_configure((*event).surface, (*event).x, (*event).y, (*event).width, (*event).height);
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, destroy);

    wl_listener_remove_safe(&mut (*or).request_configure);
    wl_listener_remove_safe(&mut (*or).destroy);
    wl_listener_remove_safe(&mut (*or).associate);
    wl_listener_remove_safe(&mut (*or).dissociate);
    wl_listener_remove_safe(&mut (*or).set_override_redirect);

    let _ = Box::from_raw(or);
}

unsafe extern "C" fn handle_associate(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, associate);
    handle_associate_impl(or);
}

unsafe fn handle_associate_impl(or: *mut XwaylandOverrideRedirect) {
    let surface = (*(*or).xsurface).surface;
    if !surface.is_null() {
        connect_listener(
            ffi::river_wlr_surface_get_map_signal(surface),
            &mut (*or).map,
            handle_map,
        );
        connect_listener(
            ffi::river_wlr_surface_get_unmap_signal(surface),
            &mut (*or).unmap,
            handle_unmap,
        );
    }
}

unsafe extern "C" fn handle_dissociate(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, dissociate);
    wl_listener_remove_safe(&mut (*or).map);
    wl_listener_remove_safe(&mut (*or).unmap);
}

unsafe extern "C" fn handle_map(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, map);
    handle_map_impl(or);
}

unsafe fn handle_map_impl(or: *mut XwaylandOverrideRedirect) {
    let surface = (*(*or).xsurface).surface;
    let override_redirect_tree = (*(*or).server).scene.layers.override_redirect;

    let surface_tree = ffi::wlr_scene_subsurface_tree_create(override_redirect_tree, surface);
    if surface_tree.is_null() {
        log::error!("out of memory creating subsurface tree for override redirect");
        let surface_resource = ffi::river_wlr_surface_get_resource(surface);
        let client = ffi::wl_resource_get_client(surface_resource);
        ffi::wl_client_post_no_memory(client);
        return;
    }
    (*or).surface_tree = surface_tree;

    crate::scene_node_data::SceneNodeData::attach(
        surface_tree as *mut ffi::wlr_scene_node,
        crate::scene_node_data::SceneNodeDataVal::OverrideRedirect(or as *mut _),
    );

    ffi::river_wlr_surface_set_data(surface, surface_tree as *mut ffi::wlr_scene_node as *mut _);

    ffi::wlr_scene_node_set_position(
        surface_tree as *mut ffi::wlr_scene_node,
        (*(*or).xsurface).x as i32,
        (*(*or).xsurface).y as i32,
    );

    connect_listener(&mut (*(*or).xsurface).events.set_geometry, &mut (*or).set_geometry, handle_set_geometry);

    (*or).focus_if_desired();
}

unsafe extern "C" fn handle_unmap(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, unmap);

    wl_listener_remove_safe(&mut (*or).set_geometry);

    let surface = (*(*or).xsurface).surface;
    if !surface.is_null() {
        ffi::river_wlr_surface_set_data(surface, std::ptr::null_mut());
    }

    if !(*or).surface_tree.is_null() {
        ffi::wlr_scene_node_destroy((*or).surface_tree as *mut ffi::wlr_scene_node);
        (*or).surface_tree = std::ptr::null_mut();
    }

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

unsafe extern "C" fn handle_set_geometry(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let or = crate::container_of!(listener, XwaylandOverrideRedirect, set_geometry);
    if !(*or).surface_tree.is_null() {
        ffi::wlr_scene_node_set_position(
            (*or).surface_tree as *mut ffi::wlr_scene_node,
            (*(*or).xsurface).x as i32,
            (*(*or).xsurface).y as i32,
        );
    }
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
            wl_listener_remove_safe(&mut (*or).set_geometry);
            ffi::river_wlr_surface_set_data(surface, std::ptr::null_mut());
            if !(*or).surface_tree.is_null() {
                ffi::wlr_scene_node_destroy((*or).surface_tree as *mut ffi::wlr_scene_node);
                (*or).surface_tree = std::ptr::null_mut();
            }
        }
        wl_listener_remove_safe(&mut (*or).map);
        wl_listener_remove_safe(&mut (*or).unmap);
    }

    let server = (*or).server;

    // Destroy this OR instance
    wl_listener_remove_safe(&mut (*or).request_configure);
    wl_listener_remove_safe(&mut (*or).destroy);
    wl_listener_remove_safe(&mut (*or).associate);
    wl_listener_remove_safe(&mut (*or).dissociate);
    wl_listener_remove_safe(&mut (*or).set_override_redirect);
    let _ = Box::from_raw(or);

    if let Err(e) = XwaylandWindow::create(xsurface, server) {
        log::error!("Failed to transition OR to XwaylandWindow: {}", e);
    }
}
