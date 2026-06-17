// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, wl_signal_add, wl_listener_remove};
use crate::window::{Window, WindowImpl, WindowState};
use crate::xwayland_override_redirect::XwaylandOverrideRedirect;

#[repr(C)]
pub struct XwaylandWindow {
    pub window: *mut Window,
    pub xsurface: *mut ffi::wlr_xwayland_surface,
    pub surface_tree: *mut ffi::wlr_scene_tree,

    pub destroy: ffi::wl_listener,
    pub request_configure: ffi::wl_listener,
    pub set_override_redirect: ffi::wl_listener,
    pub associate: ffi::wl_listener,
    pub dissociate: ffi::wl_listener,
    pub set_size_hints: ffi::wl_listener,
    pub set_title: ffi::wl_listener,
    pub set_class: ffi::wl_listener,
    pub set_parent: ffi::wl_listener,
    pub set_decorations: ffi::wl_listener,
    pub request_maximize: ffi::wl_listener,
    pub request_fullscreen: ffi::wl_listener,
    pub request_minimize: ffi::wl_listener,

    pub map: ffi::wl_listener,
    pub unmap: ffi::wl_listener,
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

impl XwaylandWindow {
    pub unsafe fn create(
        xsurface: *mut ffi::wlr_xwayland_surface,
        server: *mut Server,
    ) -> Result<(), &'static str> {
        let title_ptr = (*xsurface).title;
        let class_ptr = (*xsurface).class;
        log::debug!(
            "new xwayland window: title='{:?}', class='{:?}'",
            if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") },
            if class_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(class_ptr).to_str().unwrap_or("") }
        );

        let window = Window::create(WindowImpl::Xwayland(std::ptr::null_mut()), server)?;

        let xwindow = Box::new(XwaylandWindow {
            window,
            xsurface,
            surface_tree: std::ptr::null_mut(),
            destroy: std::mem::zeroed(),
            request_configure: std::mem::zeroed(),
            set_override_redirect: std::mem::zeroed(),
            associate: std::mem::zeroed(),
            dissociate: std::mem::zeroed(),
            set_size_hints: std::mem::zeroed(),
            set_title: std::mem::zeroed(),
            set_class: std::mem::zeroed(),
            set_parent: std::mem::zeroed(),
            set_decorations: std::mem::zeroed(),
            request_maximize: std::mem::zeroed(),
            request_fullscreen: std::mem::zeroed(),
            request_minimize: std::mem::zeroed(),
            map: std::mem::zeroed(),
            unmap: std::mem::zeroed(),
        });

        let raw = Box::into_raw(xwindow);
        (*window).set_impl(WindowImpl::Xwayland(raw));

        (*xsurface).data = raw as *mut std::ffi::c_void;

        connect_listener(&mut (*xsurface).events.destroy, &mut (*raw).destroy, handle_destroy);
        connect_listener(&mut (*xsurface).events.associate, &mut (*raw).associate, handle_associate);
        connect_listener(&mut (*xsurface).events.dissociate, &mut (*raw).dissociate, handle_dissociate);
        connect_listener(&mut (*xsurface).events.request_configure, &mut (*raw).request_configure, handle_request_configure);
        connect_listener(&mut (*xsurface).events.set_override_redirect, &mut (*raw).set_override_redirect, handle_set_override_redirect);
        // connect_listener(&mut (*xsurface).events.set_size_hints, &mut (*raw).set_size_hints, handle_set_size_hints);
        connect_listener(&mut (*xsurface).events.set_title, &mut (*raw).set_title, handle_set_title);
        connect_listener(&mut (*xsurface).events.set_class, &mut (*raw).set_class, handle_set_class);
        connect_listener(&mut (*xsurface).events.set_parent, &mut (*raw).set_parent, handle_set_parent);
        connect_listener(&mut (*xsurface).events.set_decorations, &mut (*raw).set_decorations, handle_set_decorations);
        connect_listener(&mut (*xsurface).events.request_maximize, &mut (*raw).request_maximize, handle_request_maximize);
        connect_listener(&mut (*xsurface).events.request_fullscreen, &mut (*raw).request_fullscreen, handle_request_fullscreen);
        connect_listener(&mut (*xsurface).events.request_minimize, &mut (*raw).request_minimize, handle_request_minimize);

        if !(*xsurface).surface.is_null() {
            handle_associate_impl(raw);
            if ffi::river_wlr_surface_is_mapped((*xsurface).surface) {
                handle_map_impl(raw);
            }
        }

        Ok(())
    }

    pub unsafe fn get_scale(xwindow: *mut XwaylandWindow) -> f32 {
        let window = (*xwindow).window;
        if window.is_null() {
            return 1.0;
        }
        let server = (*window).server;
        if server.is_null() {
            return 1.0;
        }
        let wlr_output = (*server).om.max_overlap_output(&(*window).box_geom);
        if !wlr_output.is_null() {
            let output = ffi::river_wlr_output_get_data(wlr_output) as *mut crate::output::Output;
            if !output.is_null() {
                return (*output).current.scale;
            }
        }
        // Fallback: first output in layout
        let mut link = (*server).om.outputs.next;
        if link != &mut (*server).om.outputs as *mut ffi::wl_list {
            let output = crate::container_of!(link, crate::output::Output, link);
            return (*output).current.scale;
        }
        1.0
    }

    pub unsafe fn configure(&mut self) -> bool {
        let window = self.window;
        let scheduled = &mut (*window).configure_scheduled;
        let sent = &mut (*window).configure_sent;

        let scale = Self::get_scale(self);

        if scheduled.width == Some(0) {
            scheduled.width = Some((*self.xsurface).width as u32);
        }
        if scheduled.height == Some(0) {
            scheduled.height = Some((*self.xsurface).height as u32);
        }

        let mut phys_width = if let Some(w) = scheduled.width {
            w as u16
        } else {
            (*self.xsurface).width
        };

        let mut phys_height = if let Some(h) = scheduled.height {
            h as u16
        } else {
            (*self.xsurface).height
        };

        let mut phys_x = ((*window).box_geom.x as f32 * scale).round() as i16;
        let mut phys_y = ((*window).box_geom.y as f32 * scale).round() as i16;

        let class_ptr = (*self.xsurface).class;
        let class = if class_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(class_ptr).to_str().unwrap_or("") };
        let title_ptr = (*self.xsurface).title;
        let title = if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") };
        let is_wine = class.contains("steam_proton") || class.contains("wine") || class.contains("upc.exe") || title.contains("Ubisoft");
        let has_parent = !(*self.xsurface).parent.is_null();

        if is_wine && !has_parent {
            if scheduled.width.is_some() {
                phys_width += 32;
            }
            if scheduled.height.is_some() {
                phys_height += 32;
            }
            phys_x -= (16.0 * scale).round() as i16;
            phys_y -= (16.0 * scale).round() as i16;
        }

        if phys_x != (*self.xsurface).x
            || phys_y != (*self.xsurface).y
            || phys_width != (*self.xsurface).width
            || phys_height != (*self.xsurface).height
        {
            ffi::wlr_xwayland_surface_configure(
                self.xsurface,
                phys_x,
                phys_y,
                phys_width,
                phys_height,
            );
        }

        if scheduled.activated != sent.activated {
            self.set_activated(scheduled.activated);
        }
        if scheduled.maximized != sent.maximized {
            ffi::wlr_xwayland_surface_set_maximized(self.xsurface, scheduled.maximized, scheduled.maximized);
        }
        if scheduled.inform_fullscreen != sent.inform_fullscreen {
            ffi::wlr_xwayland_surface_set_fullscreen(self.xsurface, scheduled.inform_fullscreen);
        }

        let mut width = scheduled.width.unwrap_or((*self.xsurface).width as u32);
        let mut height = scheduled.height.unwrap_or((*self.xsurface).height as u32);

        if is_wine && !has_parent {
            if scheduled.width.is_none() {
                width = width.saturating_sub(32);
            }
            if scheduled.height.is_none() {
                height = height.saturating_sub(32);
            }
        }

        (*window).configure_sent = (*window).configure_scheduled.clone();
        (*window).configure_sent.width = Some(width);
        (*window).configure_sent.height = Some(height);
        (*window).configure_scheduled.width = None;
        (*window).configure_scheduled.height = None;

        false
    }

    pub unsafe fn set_activated(&self, activated: bool) {
        if activated && (*self.xsurface).minimized {
            ffi::wlr_xwayland_surface_set_minimized(self.xsurface, false);
        }
        ffi::wlr_xwayland_surface_activate(self.xsurface, activated);
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, destroy);
    handle_destroy_impl(xwindow);
}

unsafe fn handle_destroy_impl(xwindow: *mut XwaylandWindow) {
    wl_listener_remove_safe(&mut (*xwindow).destroy);
    wl_listener_remove_safe(&mut (*xwindow).associate);
    wl_listener_remove_safe(&mut (*xwindow).dissociate);
    wl_listener_remove_safe(&mut (*xwindow).request_configure);
    wl_listener_remove_safe(&mut (*xwindow).set_override_redirect);
    wl_listener_remove_safe(&mut (*xwindow).set_size_hints);
    wl_listener_remove_safe(&mut (*xwindow).set_title);
    wl_listener_remove_safe(&mut (*xwindow).set_class);
    wl_listener_remove_safe(&mut (*xwindow).set_parent);
    wl_listener_remove_safe(&mut (*xwindow).set_decorations);
    wl_listener_remove_safe(&mut (*xwindow).request_maximize);
    wl_listener_remove_safe(&mut (*xwindow).request_fullscreen);
    wl_listener_remove_safe(&mut (*xwindow).request_minimize);

    (*(*xwindow).xsurface).data = std::ptr::null_mut();

    let window = (*xwindow).window;
    (*window).impl_destroying();

    let _ = Box::from_raw(xwindow);
}

unsafe extern "C" fn handle_associate(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, associate);
    handle_associate_impl(xwindow);
}

unsafe fn handle_associate_impl(xwindow: *mut XwaylandWindow) {
    let surface = (*(*xwindow).xsurface).surface;
    if !surface.is_null() {
        connect_listener(
            ffi::river_wlr_surface_get_map_signal(surface),
            &mut (*xwindow).map,
            handle_map,
        );
        connect_listener(
            ffi::river_wlr_surface_get_unmap_signal(surface),
            &mut (*xwindow).unmap,
            handle_unmap,
        );
    }
}

unsafe extern "C" fn handle_dissociate(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, dissociate);
    handle_dissociate_impl(xwindow);
}

unsafe fn handle_dissociate_impl(xwindow: *mut XwaylandWindow) {
    wl_listener_remove_safe(&mut (*xwindow).map);
    wl_listener_remove_safe(&mut (*xwindow).unmap);
}

unsafe extern "C" fn handle_map(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, map);
    handle_map_impl(xwindow);
}

unsafe fn handle_map_impl(xwindow: *mut XwaylandWindow) {
    let surfaces_tree = (*(*xwindow).window).surfaces.tree;
    let surface = (*(*xwindow).xsurface).surface;
    let surface_tree = ffi::wlr_scene_subsurface_tree_create(surfaces_tree, surface);
    if surface_tree.is_null() {
        log::error!("out of memory creating subsurface tree");
        let surface_resource = ffi::river_wlr_surface_get_resource(surface);
        let client = ffi::wl_resource_get_client(surface_resource);
        ffi::wl_client_post_no_memory(client);
        return;
    }
    (*xwindow).surface_tree = surface_tree;

    let class_ptr = (*(*xwindow).xsurface).class;
    let class = if class_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(class_ptr).to_str().unwrap_or("") };
    let title_ptr = (*(*xwindow).xsurface).title;
    let title = if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") };
    let is_wine = class.contains("steam_proton") || class.contains("wine") || class.contains("upc.exe") || title.contains("Ubisoft");
    let has_parent = !(*(*xwindow).xsurface).parent.is_null();

    if is_wine && !has_parent {
        ffi::wlr_scene_node_set_position(surface_tree as *mut ffi::wlr_scene_node, -16, -16);
    }

    ffi::river_wlr_surface_set_data(surface, &mut (*(*xwindow).window).node as *mut crate::wm_node::WmNode as *mut _);

    let capture_tree = &mut (*(*(*xwindow).window).capture_scene).tree as *mut ffi::wlr_scene_tree;
    let capture_surface = ffi::wlr_scene_surface_create(capture_tree, surface);
    if capture_surface.is_null() {
        log::error!("out of memory creating capture surface");
        let surface_resource = ffi::river_wlr_surface_get_resource(surface);
        let client = ffi::wl_resource_get_client(surface_resource);
        ffi::wl_client_post_no_memory(client);
        return;
    }

    if (*(*xwindow).xsurface).fullscreen {
        (*(*xwindow).window).wm_scheduled.fullscreen_requested = crate::window::FullscreenRequest::Fullscreen(std::ptr::null_mut());
    }

    (*(*xwindow).window).state = WindowState::Initialized;
    if let Err(e) = (*(*xwindow).window).map() {
        log::error!("out of memory mapping window: {}", e);
        let surface_resource = ffi::river_wlr_surface_get_resource(surface);
        let client = ffi::wl_resource_get_client(surface_resource);
        ffi::wl_client_post_no_memory(client);
    }
    (*(*(*xwindow).window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_unmap(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, unmap);
    handle_unmap_impl(xwindow);
}

unsafe fn handle_unmap_impl(xwindow: *mut XwaylandWindow) {
    let surface = (*(*xwindow).xsurface).surface;
    if !surface.is_null() {
        ffi::river_wlr_surface_set_data(surface, std::ptr::null_mut());
    }
    (*(*xwindow).window).unmap();
    if !(*xwindow).surface_tree.is_null() {
        ffi::wlr_scene_node_destroy((*xwindow).surface_tree as *mut ffi::wlr_scene_node);
        (*xwindow).surface_tree = std::ptr::null_mut();
    }
}

unsafe extern "C" fn handle_request_configure(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, request_configure);
    let event = data as *mut ffi::wlr_xwayland_surface_configure_event;

    let surface = (*(*xwindow).xsurface).surface;
    if surface.is_null() || !ffi::river_wlr_surface_is_mapped(surface) {
        ffi::wlr_xwayland_surface_configure((*xwindow).xsurface, (*event).x, (*event).y, (*event).width, (*event).height);
        return;
    }

    let scale = XwaylandWindow::get_scale(xwindow);

    let class_ptr = (*(*xwindow).xsurface).class;
    let class = if class_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(class_ptr).to_str().unwrap_or("") };
    let title_ptr = (*(*xwindow).xsurface).title;
    let title = if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") };
    let is_wine = class.contains("steam_proton") || class.contains("wine") || class.contains("upc.exe") || title.contains("Ubisoft");

    let has_parent = !(*(*xwindow).xsurface).parent.is_null();
    log::info!(
        "XWayland configure request: title='{}' class='{}' has_parent={} is_wine={} event=({}, {}, {}, {}) xsurface=({}, {}, {}, {}) scale={}",
        title,
        class,
        has_parent,
        is_wine,
        (*event).x, (*event).y, (*event).width, (*event).height,
        (*(*xwindow).xsurface).x, (*(*xwindow).xsurface).y, (*(*xwindow).xsurface).width, (*(*xwindow).xsurface).height,
        scale
    );

    if has_parent {
        ffi::wlr_xwayland_surface_configure(
            (*xwindow).xsurface,
            (*event).x,
            (*event).y,
            (*event).width,
            (*event).height,
        );
        let log_x = ((*event).x as f32 / scale).round() as i32;
        let log_y = ((*event).y as f32 / scale).round() as i32;
        let log_width = (*event).width as u32;
        let log_height = (*event).height as u32;
        
        let window = (*xwindow).window;
        (*window).box_geom.x = log_x;
        (*window).box_geom.y = log_y;
        (*window).box_geom.width = log_width as i32;
        (*window).box_geom.height = log_height as i32;
        (*window).rendering_requested.x = log_x;
        (*window).rendering_requested.y = log_y;
        (*window).rendering_sent.width = log_width;
        (*window).rendering_sent.height = log_height;
        (*window).set_dimensions(log_width, log_height);
        return;
    }

    let window = (*xwindow).window;
    let is_tiled = unsafe { (*window).wm_requested.tiled != 0 };

    let (phys_width, phys_height) = if is_tiled {
        let log_w = (*window).configure_sent.width.unwrap_or((*window).box_geom.width as u32);
        let log_h = (*window).configure_sent.height.unwrap_or((*window).box_geom.height as u32);
        if log_w > 0 && log_h > 0 {
            let mut w = log_w;
            let mut h = log_h;
            if is_wine && !has_parent {
                w += 32;
                h += 32;
            }
            (w as u16, h as u16)
        } else {
            ((*event).width, (*event).height)
        }
    } else {
        ((*event).width, (*event).height)
    };

    let mut phys_x = ((*window).box_geom.x as f32 * scale).round() as i16;
    let mut phys_y = ((*window).box_geom.y as f32 * scale).round() as i16;

    if is_wine && !has_parent {
        phys_x -= (16.0 * scale).round() as i16;
        phys_y -= (16.0 * scale).round() as i16;
    }

    ffi::wlr_xwayland_surface_configure(
        (*xwindow).xsurface,
        phys_x,
        phys_y,
        phys_width,
        phys_height,
    );
    let mut log_width = phys_width as u32;
    let mut log_height = phys_height as u32;
    if is_wine && !has_parent {
        log_width = log_width.saturating_sub(32);
        log_height = log_height.saturating_sub(32);
    }
    (*window).set_dimensions(log_width, log_height);
}

unsafe extern "C" fn handle_set_override_redirect(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_override_redirect);
    let xsurface = (*xwindow).xsurface;
    log::info!("xwayland surface set override redirect: val={}", (*xsurface).override_redirect);
    assert!((*xsurface).override_redirect);

    let surface = (*xsurface).surface;
    if !surface.is_null() {
        if ffi::river_wlr_surface_is_mapped(surface) {
            handle_unmap_impl(xwindow);
        }
        handle_dissociate_impl(xwindow);
    }
    let server = (*(*xwindow).window).server;
    handle_destroy_impl(xwindow);

    if let Err(e) = XwaylandOverrideRedirect::create(xsurface, server) {
        log::error!("Failed to create XwaylandOverrideRedirect: {}", e);
    }
}

unsafe extern "C" fn handle_set_size_hints(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_size_hints);
    let size_hints = (*(*xwindow).xsurface).size_hints;
    if !size_hints.is_null() {
        let min_width = std::cmp::max(0, (*size_hints).min_width) as u32;
        let min_height = std::cmp::max(0, (*size_hints).min_height) as u32;
        let max_width = if (*size_hints).max_width <= 0 {
            0
        } else {
            std::cmp::max(min_width, (*size_hints).max_width as u32)
        };
        let max_height = if (*size_hints).max_height <= 0 {
            0
        } else {
            std::cmp::max(min_height, (*size_hints).max_height as u32)
        };
        let hint = crate::window::DimensionsHint {
            min_width,
            max_width,
            min_height,
            max_height,
        };
        (*(*xwindow).window).set_dimensions_hint(hint);
    }
}

unsafe extern "C" fn handle_set_title(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_title);
    (*(*xwindow).window).notify_title();
}

unsafe extern "C" fn handle_set_class(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_class);
    (*(*xwindow).window).notify_app_id();
}

unsafe extern "C" fn handle_set_parent(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_parent);
    (*(*(*xwindow).window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_set_decorations(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_decorations);
    let prefers_csd = ((*(*xwindow).xsurface).decorations
        & (ffi::wlr_xwayland_surface_decorations_WLR_XWAYLAND_SURFACE_DECORATIONS_NO_BORDER
            | ffi::wlr_xwayland_surface_decorations_WLR_XWAYLAND_SURFACE_DECORATIONS_NO_TITLE) as u32)
        != 0;

    let hint = if prefers_csd {
        ffi::river_window_v1_decoration_hint_RIVER_WINDOW_V1_DECORATION_HINT_PREFERS_CSD
    } else {
        ffi::river_window_v1_decoration_hint_RIVER_WINDOW_V1_DECORATION_HINT_PREFERS_SSD
    };
    (*(*xwindow).window).set_decoration_hint(hint);
}

unsafe extern "C" fn handle_request_maximize(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, request_maximize);
    let maximized = (*(*xwindow).xsurface).maximized_vert || (*(*xwindow).xsurface).maximized_horz;
    let window = (*xwindow).window;
    if maximized {
        (*window).tiling_mode = crate::tiling::TilingMode::Cascade;
        (*window).mode_locked = true;
    } else {
        (*window).tiling_mode = crate::tiling::TilingMode::Floating;
        (*window).mode_locked = true;
    }
    (*window).wm_scheduled.maximize_requested = if maximized {
        crate::window::MaximizeRequest::Maximize
    } else {
        crate::window::MaximizeRequest::Unmaximize
    };
    (*(*window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_request_fullscreen(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, request_fullscreen);
    let fullscreen = (*(*xwindow).xsurface).fullscreen;
    (*(*xwindow).window).wm_scheduled.fullscreen_requested = if fullscreen {
        crate::window::FullscreenRequest::Fullscreen(std::ptr::null_mut())
    } else {
        crate::window::FullscreenRequest::Exit
    };
    (*(*(*xwindow).window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_request_minimize(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, request_minimize);
    let event = data as *mut ffi::wlr_xwayland_minimize_event;
    ffi::wlr_xwayland_surface_set_minimized((*xwindow).xsurface, (*event).minimize);
    (*(*xwindow).window).wm_scheduled.minimize_requested = true;
    (*(*(*xwindow).window).server).wm.dirty_windowing();
}
