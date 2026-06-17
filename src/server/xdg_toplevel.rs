// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, WlList, wl_listener_remove, wl_signal_add};
use crate::window::Window;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigureState {
    Idle,
    Inflight(u32),
    Acked,
    Committed,
    TimedOut(u32),
    TimedOutAcked,
}

pub struct XdgToplevel {
    pub window: *mut Window,
    pub wlr_toplevel: *mut ffi::wlr_xdg_toplevel,
    pub decoration: *mut XdgDecoration,
    pub geometry: ffi::wlr_box,
    pub configure_state: ConfigureState,

    pub destroy: ffi::wl_listener,
    pub ack_configure: ffi::wl_listener,
    pub map: ffi::wl_listener,
    pub unmap: ffi::wl_listener,
    pub commit: ffi::wl_listener,
    pub new_popup: ffi::wl_listener,
    pub request_show_window_menu: ffi::wl_listener,
    pub request_fullscreen: ffi::wl_listener,
    pub request_maximize: ffi::wl_listener,
    pub request_minimize: ffi::wl_listener,
    pub request_move: ffi::wl_listener,
    pub request_resize: ffi::wl_listener,
    pub set_parent: ffi::wl_listener,
    pub set_title: ffi::wl_listener,
    pub set_app_id: ffi::wl_listener,
}

pub struct XdgDecoration {
    pub wlr_decoration: *mut ffi::wlr_xdg_toplevel_decoration_v1,
    pub destroy: ffi::wl_listener,
    pub request_mode: ffi::wl_listener,
}

impl XdgToplevel {
    pub unsafe fn create(
        wlr_toplevel: *mut ffi::wlr_xdg_toplevel,
        server: *mut Server,
    ) -> Result<(), &'static str> {
        log::debug!("new xdg_toplevel");

        let window = Window::create(crate::window::WindowImpl::Toplevel(std::ptr::null_mut()), server)?;

        let toplevel = Box::new(XdgToplevel {
            window,
            wlr_toplevel,
            decoration: std::ptr::null_mut(),
            geometry: std::mem::zeroed(),
            configure_state: ConfigureState::Idle,

            destroy: std::mem::zeroed(),
            ack_configure: std::mem::zeroed(),
            map: std::mem::zeroed(),
            unmap: std::mem::zeroed(),
            commit: std::mem::zeroed(),
            new_popup: std::mem::zeroed(),
            request_show_window_menu: std::mem::zeroed(),
            request_fullscreen: std::mem::zeroed(),
            request_maximize: std::mem::zeroed(),
            request_minimize: std::mem::zeroed(),
            request_move: std::mem::zeroed(),
            request_resize: std::mem::zeroed(),
            set_parent: std::mem::zeroed(),
            set_title: std::mem::zeroed(),
            set_app_id: std::mem::zeroed(),
        });

        let raw = Box::into_raw(toplevel);
        (*window).set_impl(crate::window::WindowImpl::Toplevel(raw));

        let base = ffi::river_wlr_xdg_toplevel_get_base(wlr_toplevel);
        let surface = ffi::river_wlr_xdg_surface_get_surface(base);

        let unmap_listener = &mut (*raw).unmap as *mut ffi::wl_listener as *mut WlListener;
        (*unmap_listener).notify = Some(handle_unmap);
        wl_signal_add(ffi::river_wlr_surface_get_unmap_signal(surface), &mut (*raw).unmap);

        let surfaces_tree = (*window).surfaces.tree;
        let capture_tree = &mut (*(*window).capture_scene).tree as *mut ffi::wlr_scene_tree;

        let scene_xdg = ffi::wlr_scene_xdg_surface_create(surfaces_tree, base);
        if scene_xdg.is_null() {
            let _ = Box::from_raw(raw);
            return Err("wlr_scene_xdg_surface_create failed");
        }
        let capture_xdg = ffi::wlr_scene_xdg_surface_create(capture_tree, base);
        if capture_xdg.is_null() {
            // Already added unmap listener, but let's at least try to free raw
            let _ = Box::from_raw(raw);
            return Err("wlr_scene_xdg_surface_create for capture tree failed");
        }

        ffi::river_wlr_xdg_surface_set_data(base, raw as *mut _);
        ffi::river_wlr_surface_set_data(surface, (*window).tree as *mut ffi::wlr_scene_node as *mut _);

        let destroy_listener = &mut (*raw).destroy as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_listener).notify = Some(handle_destroy);
        wl_signal_add(ffi::river_wlr_xdg_toplevel_get_destroy_signal(wlr_toplevel), &mut (*raw).destroy);

        let ack_listener = &mut (*raw).ack_configure as *mut ffi::wl_listener as *mut WlListener;
        (*ack_listener).notify = Some(handle_ack_configure);
        wl_signal_add(ffi::river_wlr_xdg_surface_get_ack_configure_signal(base), &mut (*raw).ack_configure);

        let map_listener = &mut (*raw).map as *mut ffi::wl_listener as *mut WlListener;
        (*map_listener).notify = Some(handle_map);
        wl_signal_add(ffi::river_wlr_surface_get_map_signal(surface), &mut (*raw).map);

        let commit_listener = &mut (*raw).commit as *mut ffi::wl_listener as *mut WlListener;
        (*commit_listener).notify = Some(handle_commit);
        wl_signal_add(ffi::river_wlr_surface_get_commit_signal(surface), &mut (*raw).commit);

        let popup_listener = &mut (*raw).new_popup as *mut ffi::wl_listener as *mut WlListener;
        (*popup_listener).notify = Some(handle_new_popup);
        wl_signal_add(ffi::river_wlr_xdg_surface_get_new_popup_signal(base), &mut (*raw).new_popup);

        let menu_listener = &mut (*raw).request_show_window_menu as *mut ffi::wl_listener as *mut WlListener;
        (*menu_listener).notify = Some(handle_request_show_window_menu);
        wl_signal_add(ffi::river_wlr_xdg_toplevel_get_request_show_window_menu_signal(wlr_toplevel), &mut (*raw).request_show_window_menu);

        let fs_listener = &mut (*raw).request_fullscreen as *mut ffi::wl_listener as *mut WlListener;
        (*fs_listener).notify = Some(handle_request_fullscreen);
        wl_signal_add(ffi::river_wlr_xdg_toplevel_get_request_fullscreen_signal(wlr_toplevel), &mut (*raw).request_fullscreen);

        let max_listener = &mut (*raw).request_maximize as *mut ffi::wl_listener as *mut WlListener;
        (*max_listener).notify = Some(handle_request_maximize);
        wl_signal_add(ffi::river_wlr_xdg_toplevel_get_request_maximize_signal(wlr_toplevel), &mut (*raw).request_maximize);

        let min_listener = &mut (*raw).request_minimize as *mut ffi::wl_listener as *mut WlListener;
        (*min_listener).notify = Some(handle_request_minimize);
        wl_signal_add(ffi::river_wlr_xdg_toplevel_get_request_minimize_signal(wlr_toplevel), &mut (*raw).request_minimize);

        let move_listener = &mut (*raw).request_move as *mut ffi::wl_listener as *mut WlListener;
        (*move_listener).notify = Some(handle_request_move);
        wl_signal_add(ffi::river_wlr_xdg_toplevel_get_request_move_signal(wlr_toplevel), &mut (*raw).request_move);

        let resize_listener = &mut (*raw).request_resize as *mut ffi::wl_listener as *mut WlListener;
        (*resize_listener).notify = Some(handle_request_resize);
        wl_signal_add(ffi::river_wlr_xdg_toplevel_get_request_resize_signal(wlr_toplevel), &mut (*raw).request_resize);

        let parent_listener = &mut (*raw).set_parent as *mut ffi::wl_listener as *mut WlListener;
        (*parent_listener).notify = Some(handle_set_parent);
        wl_signal_add(ffi::river_wlr_xdg_toplevel_get_set_parent_signal(wlr_toplevel), &mut (*raw).set_parent);

        let title_listener = &mut (*raw).set_title as *mut ffi::wl_listener as *mut WlListener;
        (*title_listener).notify = Some(handle_set_title);
        wl_signal_add(ffi::river_wlr_xdg_toplevel_get_set_title_signal(wlr_toplevel), &mut (*raw).set_title);

        let app_listener = &mut (*raw).set_app_id as *mut ffi::wl_listener as *mut WlListener;
        (*app_listener).notify = Some(handle_set_app_id);
        wl_signal_add(ffi::river_wlr_xdg_toplevel_get_set_app_id_signal(wlr_toplevel), &mut (*raw).set_app_id);

        Ok(())
    }

    pub unsafe fn destroy_popups(&self) {
        let base = ffi::river_wlr_xdg_toplevel_get_base(self.wlr_toplevel);
        let list_head = ffi::river_wlr_xdg_surface_get_popups(base) as *mut WlList;
        let mut curr = (*list_head).next;
        while curr != list_head {
            let next = (*curr).next;
            let popup = crate::container_of!(curr, ffi::wlr_xdg_popup, link);
            ffi::wl_resource_destroy((*popup).resource);
            curr = next;
        }
    }

    pub unsafe fn configure(&mut self) -> bool {
        match self.configure_state {
            ConfigureState::Idle
            | ConfigureState::Inflight(..)
            | ConfigureState::Acked
            | ConfigureState::Committed
            | ConfigureState::TimedOut(..)
            | ConfigureState::TimedOutAcked => {}
        }

        let scheduled = &(*self.window).configure_scheduled;
        let sent = &(*self.window).configure_sent;

        if !self.needs_configure() {
            match self.configure_state {
                ConfigureState::Idle => return false,
                ConfigureState::TimedOut(serial) => {
                    self.configure_state = ConfigureState::Inflight(serial);
                    return true;
                }
                ConfigureState::TimedOutAcked => {
                    self.configure_state = ConfigureState::Acked;
                    return true;
                }
                ConfigureState::Inflight(..) | ConfigureState::Acked | ConfigureState::Committed => {
                    return false;
                }
            }
        }

        ffi::wlr_xdg_toplevel_set_activated(self.wlr_toplevel, scheduled.activated);
        ffi::wlr_xdg_toplevel_set_tiled(
            self.wlr_toplevel,
            scheduled.tiled,
        );
        ffi::wlr_xdg_toplevel_set_wm_capabilities(
            self.wlr_toplevel,
            scheduled.capabilities,
        );
        ffi::wlr_xdg_toplevel_set_maximized(self.wlr_toplevel, scheduled.maximized);
        ffi::wlr_xdg_toplevel_set_fullscreen(self.wlr_toplevel, scheduled.inform_fullscreen);
        ffi::wlr_xdg_toplevel_set_resizing(self.wlr_toplevel, scheduled.resizing);
        
        if !self.decoration.is_null() {
            let mode = if scheduled.ssd {
                if (*(*self.decoration).wlr_decoration).requested_mode == ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE {
                    ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE
                } else {
                    ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE
                }
            } else {
                ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE
            };
            ffi::wlr_xdg_toplevel_decoration_v1_set_mode((*self.decoration).wlr_decoration, mode);
        }

        if scheduled.bounds.width != sent.bounds.width || scheduled.bounds.height != sent.bounds.height {
            ffi::wlr_xdg_toplevel_set_bounds(self.wlr_toplevel, scheduled.bounds.width as i32, scheduled.bounds.height as i32);
        }

        let width = if let Some(w) = scheduled.width {
            w
        } else if let Some(w) = (*self.window).configure_sent.width {
            w
        } else {
            self.geometry.width as u32
        };

        let height = if let Some(h) = scheduled.height {
            h
        } else if let Some(h) = (*self.window).configure_sent.height {
            h
        } else {
            self.geometry.height as u32
        };

        log::info!(
            "XdgToplevel::configure: sending size {}x{} (scheduled={:?}, sent={:?}, geometry={:?}) to client '{}'",
            width, height, scheduled.width, sent.width, (self.geometry.width, self.geometry.height), (*self.window).get_title_string().unwrap_or_else(|| "None".to_string())
        );

        let configure_serial = ffi::wlr_xdg_toplevel_set_size(self.wlr_toplevel, width as i32, height as i32);

        (*self.window).configure_sent = (*self.window).configure_scheduled.clone();
        (*self.window).configure_sent.width = Some(width);
        (*self.window).configure_sent.height = Some(height);
        (*self.window).configure_scheduled.width = None;
        (*self.window).configure_scheduled.height = None;

        if width != 0 && height != 0 &&
           width == self.geometry.width as u32 && height == self.geometry.height as u32 &&
           matches!(self.configure_state, ConfigureState::Idle) {
            return false;
        }

        self.configure_state = ConfigureState::Inflight(configure_serial);
        true
    }

    pub unsafe fn needs_configure(&self) -> bool {
        let scheduled = &(*self.window).configure_scheduled;
        let sent = &(*self.window).configure_sent;

        if scheduled.width.is_some() && scheduled.width != sent.width {
            return true;
        }
        if scheduled.height.is_some() && scheduled.height != sent.height {
            return true;
        }
        if scheduled.bounds.width != sent.bounds.width || scheduled.bounds.height != sent.bounds.height {
            return true;
        }
        if scheduled.activated != sent.activated {
            return true;
        }
        if scheduled.ssd != sent.ssd {
            return true;
        }
        if scheduled.tiled != sent.tiled {
            return true;
        }
        if scheduled.capabilities != sent.capabilities {
            return true;
        }
        if scheduled.maximized != sent.maximized {
            return true;
        }
        if scheduled.inform_fullscreen != sent.inform_fullscreen {
            return true;
        }
        if scheduled.resizing != sent.resizing {
            return true;
        }

        false
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, destroy);

    if !(*toplevel).decoration.is_null() {
        XdgDecoration::deinit((*toplevel).decoration);
    }

    wl_listener_remove(&mut (*toplevel).destroy);
    wl_listener_remove(&mut (*toplevel).ack_configure);
    wl_listener_remove(&mut (*toplevel).map);
    wl_listener_remove(&mut (*toplevel).unmap);
    wl_listener_remove(&mut (*toplevel).commit);
    wl_listener_remove(&mut (*toplevel).new_popup);
    wl_listener_remove(&mut (*toplevel).request_show_window_menu);
    wl_listener_remove(&mut (*toplevel).request_fullscreen);
    wl_listener_remove(&mut (*toplevel).request_maximize);
    wl_listener_remove(&mut (*toplevel).request_minimize);
    wl_listener_remove(&mut (*toplevel).request_move);
    wl_listener_remove(&mut (*toplevel).request_resize);
    wl_listener_remove(&mut (*toplevel).set_parent);
    wl_listener_remove(&mut (*toplevel).set_title);
    wl_listener_remove(&mut (*toplevel).set_app_id);

    let base = ffi::river_wlr_xdg_toplevel_get_base((*toplevel).wlr_toplevel);
    ffi::river_wlr_xdg_surface_set_data(base, std::ptr::null_mut());
    let surface = ffi::river_wlr_xdg_surface_get_surface(base);
    ffi::river_wlr_surface_set_data(surface, std::ptr::null_mut());

    let window = (*toplevel).window;
    (*window).impl_destroying();
    match (*window).state {
        crate::window::WindowState::Init | crate::window::WindowState::Closing => {}
        crate::window::WindowState::Ready | crate::window::WindowState::Initialized | crate::window::WindowState::Mapped => {
            (*window).state = crate::window::WindowState::Closing;
            (*(*window).server).wm.dirty_windowing();
        }
    }

    let _ = Box::from_raw(toplevel);
}

unsafe extern "C" fn handle_unmap(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, unmap);
    (*(*toplevel).window).unmap();
}

unsafe extern "C" fn handle_map(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, map);
    if let Err(e) = (*(*toplevel).window).map() {
        log::error!("Window map failed: {}", e);
        let client = ffi::wl_resource_get_client((*(*toplevel).wlr_toplevel).resource);
        ffi::wl_client_post_no_memory(client);
    }
}

unsafe extern "C" fn handle_new_popup(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, new_popup);
    let wlr_xdg_popup = data as *mut ffi::wlr_xdg_popup;

    let window = (*toplevel).window;
    let capture_node = &mut (*(*window).capture_scene).tree as *mut ffi::wlr_scene_tree;
    if let Err(e) = crate::xdg_popup::XdgPopup::create(
        wlr_xdg_popup,
        (*window).popup_tree,
        capture_node,
    ) {
        log::error!("Failed to create popup: {}", e);
        ffi::wl_resource_post_no_memory((*wlr_xdg_popup).resource);
    }
}

unsafe extern "C" fn handle_ack_configure(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let toplevel = crate::container_of!(listener, XdgToplevel, ack_configure);
    let acked_configure = data as *mut ffi::wlr_xdg_surface_configure;
    let serial = (*acked_configure).serial;

    match (*toplevel).configure_state {
        ConfigureState::Inflight(s) => {
            if serial == s {
                (*toplevel).configure_state = ConfigureState::Acked;
            }
        }
        ConfigureState::TimedOut(s) => {
            if serial == s {
                (*toplevel).configure_state = ConfigureState::TimedOutAcked;
            }
        }
        _ => {}
    }
}

unsafe extern "C" fn handle_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, commit);
    let window = (*toplevel).window;
    let base = ffi::river_wlr_xdg_toplevel_get_base((*toplevel).wlr_toplevel);

    ffi::river_scene_node_enable_blur(
        (*window).surfaces.tree as *mut ffi::wlr_scene_node,
        (*window).rendering_requested.blur,
    );

    let capture_node = &mut (*(*window).capture_scene).tree as *mut ffi::wlr_scene_tree as *mut ffi::wlr_scene_node;
    let mut geom = std::mem::zeroed();
    ffi::river_wlr_xdg_surface_get_geometry(base, &mut geom);
    ffi::wlr_scene_subsurface_tree_set_clip(capture_node, &geom);

    let mut min_w = 0;
    let mut min_h = 0;
    let mut max_w = 0;
    let mut max_h = 0;
    ffi::river_wlr_xdg_toplevel_get_requested_min_max_size((*toplevel).wlr_toplevel, &mut min_w, &mut min_h, &mut max_w, &mut max_h);

    (*window).set_dimensions_hint(crate::window::DimensionsHint {
        min_width: min_w as u32,
        min_height: min_h as u32,
        max_width: max_w as u32,
        max_height: max_h as u32,
    });

    if ffi::river_wlr_xdg_surface_get_initial_commit(base) {
        assert!((*window).state != crate::window::WindowState::Ready);
        (*window).state = crate::window::WindowState::Ready;
        (*(*window).server).wm.dirty_windowing();
        return;
    }

    if (*window).state != crate::window::WindowState::Mapped {
        return;
    }

    match (*toplevel).configure_state {
        ConfigureState::Idle | ConfigureState::Committed | ConfigureState::TimedOut(..) => {
            let old_geometry = (*toplevel).geometry;
            let mut new_geometry = std::mem::zeroed();
            ffi::river_wlr_xdg_surface_get_geometry(base, &mut new_geometry);
            (*toplevel).geometry = new_geometry;

            let size_changed = new_geometry.width != old_geometry.width || new_geometry.height != old_geometry.height;

            if size_changed {
                log::debug!(
                    "client initiated size change: {}x{} -> {}x{}",
                    old_geometry.width, old_geometry.height, new_geometry.width, new_geometry.height
                );
                if matches!((*window).tiling_mode, crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Popup) {
                    (*window).set_dimensions(new_geometry.width as u32, new_geometry.height as u32);
                    (*window).configure_sent.width = Some(new_geometry.width as u32);
                    (*window).configure_sent.height = Some(new_geometry.height as u32);
                } else {
                    (*window).render_finish();
                }
            } else if old_geometry.x != new_geometry.x || old_geometry.y != new_geometry.y {
                (*window).render_finish();
            }
        }
        ConfigureState::Inflight(..) => {
            (*window).send_frame_done();
        }
        ConfigureState::Acked | ConfigureState::TimedOutAcked => {
            let mut new_geometry = std::mem::zeroed();
            ffi::river_wlr_xdg_surface_get_geometry(base, &mut new_geometry);
            (*toplevel).geometry = new_geometry;

            (*window).rendering_scheduled.width = new_geometry.width as u32;
            (*window).rendering_scheduled.height = new_geometry.height as u32;

            let (dec_w, dec_h) = (*window).get_decorations_size();
            if dec_w != (*window).last_decor_w || dec_h != (*window).last_decor_h {
                (*window).last_decor_w = dec_w;
                (*window).last_decor_h = dec_h;
                (*(*window).server).wm.dirty_windowing();
            }

            match (*toplevel).configure_state {
                ConfigureState::Acked => {
                    (*toplevel).configure_state = ConfigureState::Committed;
                    (*(*window).server).wm.notify_configured();
                }
                ConfigureState::TimedOutAcked => {
                    (*toplevel).configure_state = ConfigureState::Idle;
                    (*(*window).server).wm.dirty_rendering();
                }
                _ => unreachable!(),
            }
        }
    }
}

unsafe extern "C" fn handle_request_show_window_menu(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let toplevel = crate::container_of!(listener, XdgToplevel, request_show_window_menu);
    let event = data as *mut ffi::wlr_xdg_toplevel_show_window_menu_event;
    let window = (*toplevel).window;

    (*window).wm_scheduled.show_window_menu_requested = Some(crate::window::ShowWindowMenuRequest {
        x: (*event).x - (*toplevel).geometry.x,
        y: (*event).y - (*toplevel).geometry.y,
    });
    (*(*window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_request_fullscreen(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, request_fullscreen);
    let window = (*toplevel).window;

    if ffi::river_wlr_xdg_toplevel_get_requested_fullscreen((*toplevel).wlr_toplevel) {
        let wlr_output = ffi::river_wlr_xdg_toplevel_get_requested_fullscreen_output((*toplevel).wlr_toplevel);
        if !wlr_output.is_null() {
            let output = ffi::river_wlr_output_get_data(wlr_output) as *mut crate::output::Output;
            (*window).wm_scheduled.fullscreen_requested = crate::window::FullscreenRequest::Fullscreen(output);
        } else {
            (*window).wm_scheduled.fullscreen_requested = crate::window::FullscreenRequest::Fullscreen(std::ptr::null_mut());
        }
    } else {
        (*window).wm_scheduled.fullscreen_requested = crate::window::FullscreenRequest::Exit;
    }
    (*(*window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_request_maximize(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, request_maximize);
    let window = (*toplevel).window;

    if ffi::river_wlr_xdg_toplevel_get_requested_maximized((*toplevel).wlr_toplevel) {
        (*window).tiling_mode = crate::tiling::TilingMode::Cascade;
        (*window).mode_locked = true;
        (*window).wm_scheduled.maximize_requested = crate::window::MaximizeRequest::Maximize;
    } else {
        (*window).tiling_mode = crate::tiling::TilingMode::Floating;
        (*window).mode_locked = true;
        (*window).wm_scheduled.maximize_requested = crate::window::MaximizeRequest::Unmaximize;
    }
    (*(*window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_request_minimize(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, request_minimize);
    let window = (*toplevel).window;

    (*window).wm_scheduled.minimize_requested = true;
    (*(*window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_request_move(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let toplevel = crate::container_of!(listener, XdgToplevel, request_move);
    let event = data as *mut ffi::wlr_xdg_toplevel_move_event;
    let window = (*toplevel).window;
    let seat = ffi::river_wlr_seat_get_data((*(*event).seat).seat) as *mut crate::seat::Seat;

    if ffi::wlr_seat_validate_pointer_grab_serial((*seat).wlr_seat, std::ptr::null_mut(), (*event).serial) {
        let initial_mode = (*window).tiling_mode;
        if initial_mode != crate::tiling::TilingMode::Floating
            && initial_mode != crate::tiling::TilingMode::Popup
            && initial_mode != crate::tiling::TilingMode::Fullscreen
        {
            (*window).tiling_mode = crate::tiling::TilingMode::Floating;
            (*window).mode_locked = true;
        }

        (*seat).focus(crate::seat::Focus::Window(window));
        let cursor = &mut (*seat).cursor;
        let cursor_x = (*cursor.wlr_cursor).x;
        let cursor_y = (*cursor.wlr_cursor).y;

        (*seat).op = Some(crate::seat::SeatOp {
            sent_release: false,
            input: crate::seat::SeatOpInput::Pointer,
            start_x: cursor_x as i32,
            start_y: cursor_y as i32,
            x: cursor_x as i32,
            y: cursor_y as i32,
            window_ptr: window,
            op_type: crate::seat::PointerOpType::Move,
            start_win_x: (*window).box_geom.x,
            start_win_y: (*window).box_geom.y,
            start_win_w: (*window).box_geom.width as u32,
            start_win_h: (*window).box_geom.height as u32,
        });
        cursor.op_start_pointer();

        (*window).wm_scheduled.pointer_move_requested = seat;
        (*(*window).server).wm.dirty_windowing();
    }
}

unsafe extern "C" fn handle_request_resize(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let toplevel = crate::container_of!(listener, XdgToplevel, request_resize);
    let event = data as *mut ffi::wlr_xdg_toplevel_resize_event;
    let window = (*toplevel).window;
    let seat = ffi::river_wlr_seat_get_data((*(*event).seat).seat) as *mut crate::seat::Seat;

    if ffi::wlr_seat_validate_pointer_grab_serial((*seat).wlr_seat, std::ptr::null_mut(), (*event).serial) {
        let initial_mode = (*window).tiling_mode;
        if initial_mode != crate::tiling::TilingMode::Floating
            && initial_mode != crate::tiling::TilingMode::Popup
            && initial_mode != crate::tiling::TilingMode::Fullscreen
        {
            (*window).tiling_mode = crate::tiling::TilingMode::Floating;
            (*window).mode_locked = true;
        }

        (*seat).focus(crate::seat::Focus::Window(window));
        let cursor = &mut (*seat).cursor;
        let cursor_x = (*cursor.wlr_cursor).x;
        let cursor_y = (*cursor.wlr_cursor).y;

        (*seat).op = Some(crate::seat::SeatOp {
            sent_release: false,
            input: crate::seat::SeatOpInput::Pointer,
            start_x: cursor_x as i32,
            start_y: cursor_y as i32,
            x: cursor_x as i32,
            y: cursor_y as i32,
            window_ptr: window,
            op_type: crate::seat::PointerOpType::Resize {
                edges: crate::window::Edges::from_u32((*event).edges),
            },
            start_win_x: (*window).box_geom.x,
            start_win_y: (*window).box_geom.y,
            start_win_w: (*window).box_geom.width as u32,
            start_win_h: (*window).box_geom.height as u32,
        });
        cursor.op_start_pointer();

        (*window).wm_scheduled.pointer_resize_requested = Some(crate::window::PointerResizeRequest {
            seat,
            edges: (*event).edges,
        });
        (*(*window).server).wm.dirty_windowing();
    }
}

unsafe extern "C" fn handle_set_parent(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, set_parent);
    let window = (*toplevel).window;
    (*(*window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_set_title(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, set_title);
    let window = (*toplevel).window;
    (*window).notify_title();
}

unsafe extern "C" fn handle_set_app_id(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, set_app_id);
    let window = (*toplevel).window;
    (*window).notify_app_id();
}

unsafe extern "C" fn handle_decoration_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let decoration = crate::container_of!(listener, XdgDecoration, destroy);
    XdgDecoration::deinit(decoration);
}

unsafe extern "C" fn handle_decoration_request_mode(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let decoration = crate::container_of!(listener, XdgDecoration, request_mode);
    
    let base = ffi::river_wlr_xdg_toplevel_get_base((*(*decoration).wlr_decoration).toplevel);
    let toplevel = ffi::river_wlr_xdg_surface_get_data(base) as *mut XdgToplevel;
    let window = (*toplevel).window;

    let hint = match (*(*decoration).wlr_decoration).requested_mode {
        ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_NONE => {
            ffi::river_window_v1_decoration_hint_RIVER_WINDOW_V1_DECORATION_HINT_NO_PREFERENCE
        }
        ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE => {
            ffi::river_window_v1_decoration_hint_RIVER_WINDOW_V1_DECORATION_HINT_PREFERS_CSD
        }
        ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE => {
            ffi::river_window_v1_decoration_hint_RIVER_WINDOW_V1_DECORATION_HINT_PREFERS_SSD
        }
        _ => ffi::river_window_v1_decoration_hint_RIVER_WINDOW_V1_DECORATION_HINT_NO_PREFERENCE,
    };
    (*window).set_decoration_hint(hint);
}

impl XdgDecoration {
    pub unsafe fn init(wlr_decoration: *mut ffi::wlr_xdg_toplevel_decoration_v1) -> *mut Self {
        let base = ffi::river_wlr_xdg_toplevel_get_base((*wlr_decoration).toplevel);
        let toplevel = ffi::river_wlr_xdg_surface_get_data(base) as *mut XdgToplevel;

        let decoration = Box::into_raw(Box::new(XdgDecoration {
            wlr_decoration,
            destroy: std::mem::zeroed(),
            request_mode: std::mem::zeroed(),
        }));

        (*toplevel).decoration = decoration;

        let destroy_ptr = &mut (*decoration).destroy as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_ptr).notify = Some(handle_decoration_destroy);
        wl_signal_add(&mut (*wlr_decoration).events.destroy, &mut (*decoration).destroy);

        let req_mode_ptr = &mut (*decoration).request_mode as *mut ffi::wl_listener as *mut WlListener;
        (*req_mode_ptr).notify = Some(handle_decoration_request_mode);
        wl_signal_add(&mut (*wlr_decoration).events.request_mode, &mut (*decoration).request_mode);

        if ffi::river_wlr_xdg_surface_get_initialized(base) {
            handle_decoration_request_mode(&mut (*decoration).request_mode, std::ptr::null_mut());
        }

        decoration
    }

    pub unsafe fn deinit(decoration: *mut XdgDecoration) {
        let base = ffi::river_wlr_xdg_toplevel_get_base((*(*decoration).wlr_decoration).toplevel);
        let toplevel = ffi::river_wlr_xdg_surface_get_data(base) as *mut XdgToplevel;

        wl_listener_remove(&mut (*decoration).destroy);
        wl_listener_remove(&mut (*decoration).request_mode);

        assert!(!(*toplevel).decoration.is_null());
        (*toplevel).decoration = std::ptr::null_mut();

        let _ = Box::from_raw(decoration);
    }
}


