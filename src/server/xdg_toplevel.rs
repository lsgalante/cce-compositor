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

        // Absorb a size-only ECHO: the scheduled size merely restates what the
        // client has already committed (its current geometry) and nothing else
        // changed. Sending it anyway hands a self-sizing client a stale size
        // one commit later — and when the client's content width flaps (the
        // cpu module's text crossing 10%), that stale echo re-triggers a
        // resize on both sides and the pair ping-pongs at frame rate (the
        // status-bar jitter: ~3300 alternating 95/104 configures in 5min).
        // Agree with reality instead and send nothing. A configure whose size
        // DIFFERS from the committed geometry — a real compositor-driven
        // resize — always goes through.
        {
            let echo_w = scheduled.width.or(sent.width);
            let echo_h = scheduled.height.or(sent.height);
            // Bounds are part of the echo, not a separate signal, when they
            // merely track the echoed size: the arrange schedules a status
            // window's bounds equal to its own box, so a self-resize ALWAYS
            // carries a matching bounds delta — requiring bounds equality
            // here would keep the absorb permanently disabled for exactly
            // the windows that loop. A bounds change that differs from the
            // echoed size (a real available-area change) still forces a
            // configure.
            let bounds_ok = (scheduled.bounds.width == sent.bounds.width
                && scheduled.bounds.height == sent.bounds.height)
                || (echo_w == Some(scheduled.bounds.width as u32)
                    && echo_h == Some(scheduled.bounds.height as u32));
            let non_size_equal = scheduled.activated == sent.activated
                && scheduled.ssd == sent.ssd
                && scheduled.tiled == sent.tiled
                && scheduled.capabilities == sent.capabilities
                && scheduled.maximized == sent.maximized
                && scheduled.inform_fullscreen == sent.inform_fullscreen
                && scheduled.resizing == sent.resizing;
            // Idle AND Committed: after any completed configure round-trip
            // the state machine RESTS in Committed (Acked → Committed on
            // commit; only the timeout path returns to Idle), so gating on
            // Idle alone leaves this absorb dead in steady state — the exact
            // moment the echo loop runs. Inflight/Acked stay excluded: a
            // real configure is mid-flight and the scheduled size may need
            // to supersede it.
            let size_is_echo = self.geometry.width > 0
                && self.geometry.height > 0
                && echo_w == Some(self.geometry.width as u32)
                && echo_h == Some(self.geometry.height as u32);
            // Timeout recovery states absorb too: a hot echo loop drives the
            // machine into TimedOut/TimedOutAcked, and an absorb that disarms
            // there switches itself off at exactly the moment it exists for
            // (observed live: a title-flapping window module sustained a
            // 372↔456 storm at state=TimedOutAcked). Only Inflight/Acked stay
            // excluded — a real configure is mid-flight there. Absorbing
            // leaves the timeout recovery untouched: a late ack or the next
            // commit still walks the state back to Idle.
            if size_is_echo
                && non_size_equal
                && bounds_ok
                && !matches!(
                    self.configure_state,
                    ConfigureState::Inflight(..) | ConfigureState::Acked
                )
            {
                let absorbed_bounds = scheduled.bounds;
                (*self.window).configure_sent.width = echo_w;
                (*self.window).configure_sent.height = echo_h;
                (*self.window).configure_sent.bounds = absorbed_bounds;
                (*self.window).configure_scheduled.width = None;
                (*self.window).configure_scheduled.height = None;
                return false;
            }
            if size_is_echo && log::log_enabled!(log::Level::Debug) {
                // The size restates committed geometry yet the absorb
                // declined — name the blocker (the state, or which non-size
                // field), so a live echo loop is diagnosable from the log.
                log::debug!(
                    "XdgToplevel::configure: echo NOT absorbed: state={:?} non_size_equal={} \
                     (bounds {}x{}/{}x{} act {}/{} ssd {}/{} tiled {:?}/{:?} caps {:?}/{:?} max {}/{} fs {}/{} rsz {}/{})",
                    self.configure_state, non_size_equal,
                    scheduled.bounds.width, scheduled.bounds.height, sent.bounds.width, sent.bounds.height,
                    scheduled.activated, sent.activated,
                    scheduled.ssd, sent.ssd,
                    scheduled.tiled, sent.tiled,
                    scheduled.capabilities, sent.capabilities,
                    scheduled.maximized, sent.maximized,
                    scheduled.inform_fullscreen, sent.inform_fullscreen,
                    scheduled.resizing, sent.resizing,
                );
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
            let mode = ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE;
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

        if log::log_enabled!(log::Level::Debug) {
            log::debug!(
                "XdgToplevel::configure: sending size {}x{} (scheduled={:?}, sent={:?}, geometry={:?}) to client '{}'",
                width, height, scheduled.width, sent.width, (self.geometry.width, self.geometry.height), (*self.window).get_title_string().unwrap_or_else(|| "None".to_string())
            );
        }

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
            (*window).set_closing();
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
        return;
    }

    let base = ffi::river_wlr_xdg_toplevel_get_base((*toplevel).wlr_toplevel);
    let mut new_geometry = std::mem::zeroed();
    ffi::river_wlr_xdg_surface_get_geometry(base, &mut new_geometry);
    (*toplevel).geometry = new_geometry;
    let is_status = (*(*toplevel).window).tiling_mode == crate::tiling::TilingMode::Status || 
                    (*(*toplevel).window).get_app_id_string().map_or(false, |id| id.starts_with("cce-status"));
    if is_status {
        (*(*toplevel).window).box_geom.width = new_geometry.width;
        (*(*toplevel).window).box_geom.height = new_geometry.height;
        (*(*(*toplevel).window).server).wm.dirty_windowing();
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

    let base = ffi::river_wlr_xdg_toplevel_get_base((*toplevel).wlr_toplevel);
    let mut new_geometry = std::mem::zeroed();
    ffi::river_wlr_xdg_surface_get_geometry(base, &mut new_geometry);
    (*toplevel).geometry = new_geometry;

    let is_status = (*(*toplevel).window).tiling_mode == crate::tiling::TilingMode::Status || 
                    (*(*toplevel).window).get_app_id_string().map_or(false, |id| id.starts_with("cce-status"));
    if is_status {
        (*(*toplevel).window).box_geom.width = new_geometry.width;
        (*(*toplevel).window).box_geom.height = new_geometry.height;
        (*(*(*toplevel).window).server).wm.dirty_windowing();
    }
}

unsafe extern "C" fn handle_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let toplevel = crate::container_of!(listener, XdgToplevel, commit);
    let window = (*toplevel).window;
    let base = ffi::river_wlr_xdg_toplevel_get_base((*toplevel).wlr_toplevel);
    let mut new_geometry = std::mem::zeroed();
    ffi::river_wlr_xdg_surface_get_geometry(base, &mut new_geometry);
    (*toplevel).geometry = new_geometry;

    let app_id = (*window).get_app_id_string().unwrap_or_default();
    let mut ignore_transparent = (*(*window).server).wm.layout.window_backdrop_blur_ignore_transparent;
    if app_id.starts_with("cce-status") {
        ignore_transparent = (*(*window).server).wm.layout.status_backdrop_blur_ignore_transparent;
    }
    let scale = (*window).scale;
    let actual_w = if (*window).rendering_sent.width > 0 { (*window).rendering_sent.width } else { (*toplevel).geometry.width as u32 };
    let actual_h = if (*window).rendering_sent.height > 0 { (*window).rendering_sent.height } else { (*toplevel).geometry.height as u32 };
    let geom_w = (actual_w as f64 * scale) as i32;
    let geom_h = (actual_h as f64 * scale) as i32;
    let is_status = (*window).tiling_mode == crate::tiling::TilingMode::Status || 
                    app_id.starts_with("cce-status");
    let is_cce_app = app_id.starts_with("cce-");
    // Must mirror Window::set_rendering_state's radius exactly: both paths drive the same
    // blur node, so if they disagree the corners flip between rounded and square depending
    // on which one ran last.
    let radius = if (*window).is_fullscreen() {
        0
    } else if (*window).rendering_requested.circular {
        let w = (*window).rendering_sent.width as i32;
        let h = (*window).rendering_sent.height as i32;
        w.min(h) / 2
    } else if (*window).wm_requested.ssd || is_cce_app {
        (*(*window).server).wm.layout.backplate_corner_radius
    } else {
        0
    };
    // Same span widening as Window::set_rendering_state (part of the mirror).
    let radius = if (*window).rendering_requested.circular {
        radius
    } else {
        crate::window::widen_corner_radius(radius, actual_w as i32, actual_h as i32)
    };
    let use_optimized = if is_status || radius > 0 {
        false
    } else {
        (*(*window).server).wm.layout.scenefx_optimized_blur
    };
    let blur_enabled = (*window).rendering_requested.blur && ((*window).wm_requested.ssd || is_cce_app || is_status);
    ffi::river_scene_node_enable_blur(
        (*window).tree as *mut ffi::wlr_scene_node,
        blur_enabled,
        use_optimized,
        ignore_transparent,
        0,
        0,
        geom_w,
        geom_h,
        // geom_w/h are already scaled to device px; the radius must match.
        (radius as f64 * scale) as i32,
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
        let mut new_geometry = std::mem::zeroed();
        ffi::river_wlr_xdg_surface_get_geometry(base, &mut new_geometry);
        (*toplevel).geometry = new_geometry;

        let is_status = (*window).tiling_mode == crate::tiling::TilingMode::Status || 
                        (*window).get_app_id_string().map_or(false, |id| id.starts_with("cce-status"));
        if is_status {
            (*window).box_geom.width = new_geometry.width;
            (*window).box_geom.height = new_geometry.height;
        }

        (*(*window).server).wm.dirty_windowing();
        return;
    }

    if (*window).state != crate::window::WindowState::Mapped && (*window).state != crate::window::WindowState::Ready {
        return;
    }

    // A self-sizing overlay (cce-cloud) repaints at a new size on its own, with no
    // configure round trip. The size-change branches below can't catch it: they
    // compare against (*toplevel).geometry, which already holds the new value by
    // the time they run, so size_changed is never true. Track the live geometry
    // here instead and move the border with it, in the same commit that puts the
    // new buffer on screen — waiting for the WM cycle (which round-trips out to
    // the external window-manager client) leaves the border a size behind.
    if (*window).tiling_mode == crate::tiling::TilingMode::Overlay {
        let mut live = std::mem::zeroed();
        ffi::river_wlr_xdg_surface_get_geometry(base, &mut live);
        if live.width > 0 && live.height > 0
            && (live.width != (*window).box_geom.width || live.height != (*window).box_geom.height)
        {
            (*window).box_geom.width = live.width;
            (*window).box_geom.height = live.height;
            // render_finish would otherwise reset box_geom from the render-start
            // snapshot (rendering_sent) and snap the border back to the old size.
            (*window).self_resized = true;
            (*window).draw_borders();
            (*window).set_dimensions(live.width as u32, live.height as u32);
            (*(*window).server).wm.dirty_windowing();
        }
    }

    match (*toplevel).configure_state {
        ConfigureState::Idle | ConfigureState::Committed | ConfigureState::TimedOut(..) => {
            // Nothing to do: client-initiated size/position changes CANNOT
            // be detected here. The top of handle_commit already refreshed
            // (*toplevel).geometry from this commit, so any comparison
            // against it never fires (the branch that used to live here was
            // dead for that reason). Self-sizing overlays are handled by the
            // live-geometry sync above; clients resizing through a configure
            // round trip land in the Acked arm.
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

            let is_status = (*window).tiling_mode == crate::tiling::TilingMode::Status ||
                            (*window).get_app_id_string().map_or(false, |id| id.starts_with("cce-status"));
            // Overlay included so its scheduled size tracks the client's own; the
            // border itself is handled by the live-geometry sync above.
            let is_overlay = (*window).tiling_mode == crate::tiling::TilingMode::Overlay;
            if matches!((*window).tiling_mode, crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Popup) || is_status || is_overlay {
                (*window).set_dimensions(new_geometry.width as u32, new_geometry.height as u32);
                if is_status {
                    (*window).box_geom.width = new_geometry.width;
                    (*window).box_geom.height = new_geometry.height;
                    (*(*window).server).wm.dirty_windowing();
                }
            }

            let (dec_w, dec_h) = (*window).get_decorations_size();
            if dec_w != (*window).last_decor_w || dec_h != (*window).last_decor_h {
                (*window).last_decor_w = dec_w;
                (*window).last_decor_h = dec_h;
                (*(*window).server).wm.dirty_windowing();
            }

            if let Some(sent_w) = (*window).configure_sent.width {
                if !(*window).wm_requested.ssd && dec_w > 0 && new_geometry.width as u32 == sent_w.saturating_sub(dec_w as u32) {
                    if !(*window).csd_buffer_size_bug {
                        (*window).csd_buffer_size_bug = true;
                        log::info!("Detected CSD buffer size bug for window '{}'. Activating workaround.", (*window).get_title_string().unwrap_or_default());
                        (*(*window).server).wm.dirty_windowing();
                    }
                }
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

    if let Some(edges) = (*window).resize_edges {
        let geometry = (*toplevel).geometry;
        let mut resize_active = false;
        let server = (*window).server;
        let seats_list = &mut (*server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let Some(ref op) = (*seat).op {
                if op.window_ptr == window {
                    if let crate::seat::PointerOpType::Resize { .. } = op.op_type {
                        resize_active = true;
                        break;
                    }
                }
            }
            curr_seat = (*curr_seat).next;
        }

        let mut new_vx = (*window).virtual_x;
        let mut new_vy = (*window).virtual_y;
        if edges.left {
            new_vx = (*window).resize_start_vx + ((*window).resize_start_w as f64 - geometry.width as f64);
        }
        if edges.top {
            new_vy = (*window).resize_start_vy + ((*window).resize_start_h as f64 - geometry.height as f64);
        }
        (*window).virtual_x = new_vx;
        (*window).virtual_y = new_vy;

        let scale = (*server).wm.desk_zoom;
        let pan_x = (*server).wm.desk_pan_x;
        let pan_y = (*server).wm.desk_pan_y;

        let mut out_x = 0;
        let mut out_y = 0;
        let outputs_list = &mut (*server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let wlr_box = (*output).sent.box_layout();
                out_x = wlr_box.x;
                out_y = wlr_box.y;
                break;
            }
            curr_out = (*curr_out).next;
        }

        let final_x = out_x + ((new_vx - pan_x) * scale) as i32;
        let final_y = out_y + ((new_vy - pan_y) * scale) as i32;

        (*window).rendering_requested.x = final_x;
        (*window).rendering_requested.y = final_y;
        (*window).box_geom.x = final_x;
        (*window).box_geom.y = final_y;

        // Keep the displayed buffer and the compensating position atomic.
        // Live buffer (no configure in flight): the commit is already on
        // screen, so move the scene tree in the same commit — waiting for
        // the next render pass lets a frame composite the new size at the
        // old position, jittering the anchored edges. Frozen buffer (saved
        // for an in-flight configure): moving the tree now would shift the
        // OLD-size buffer instead, so leave the position to the render pass
        // (which restores the new buffer and applies it together) and make
        // sure that pass runs promptly.
        if !(*window).surfaces.saved {
            ffi::river_scene_node_set_position_if_changed((*window).tree as *mut ffi::wlr_scene_node, final_x, final_y);
            ffi::river_scene_node_set_position_if_changed((*window).popup_tree as *mut ffi::wlr_scene_node, final_x, final_y);
            (*window).box_geom.width = geometry.width;
            (*window).box_geom.height = geometry.height;
            (*window).draw_borders();
        } else {
            (*server).wm.dirty_rendering();
        }

        if !resize_active {
            (*window).resize_edges = None;
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
        (*window).tiling_mode = crate::tiling::TilingMode::Maximized;
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
            && initial_mode != crate::tiling::TilingMode::Overlay
        {
            (*window).tiling_mode = crate::tiling::TilingMode::Floating;
            (*window).mode_locked = true;
        }

        (*seat).focus(crate::seat::Focus::Window(window));
        (*(*window).server).wm.stop_panning_animation();
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
            start_win_virtual_x: (*window).virtual_x,
            start_win_virtual_y: (*window).virtual_y,
            start_tiling_mode: (*window).tiling_mode,
            start_mode_locked: (*window).mode_locked,
            start_pan_x: (*(*window).server).wm.desk_pan_x,
            start_pan_y: (*(*window).server).wm.desk_pan_y,
            started_in_overview: (*(*window).server).wm.mode == crate::window_manager::WindowManagerMode::Overview,
        });
        cursor.op_start_pointer();
        cursor.set_xcursor(b"grab\0".as_ptr() as *const _);

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
        (*(*window).server).wm.stop_panning_animation();
        let cursor = &mut (*seat).cursor;
        let cursor_x = (*cursor.wlr_cursor).x;
        let cursor_y = (*cursor.wlr_cursor).y;

        let edges = crate::window::Edges::from_u32((*event).edges);
        (*seat).op = Some(crate::seat::SeatOp {
            sent_release: false,
            input: crate::seat::SeatOpInput::Pointer,
            start_x: cursor_x as i32,
            start_y: cursor_y as i32,
            x: cursor_x as i32,
            y: cursor_y as i32,
            window_ptr: window,
            op_type: crate::seat::PointerOpType::Resize {
                edges,
            },
            start_win_x: (*window).box_geom.x,
            start_win_y: (*window).box_geom.y,
            start_win_w: (*window).box_geom.width as u32,
            start_win_h: (*window).box_geom.height as u32,
            start_win_virtual_x: (*window).virtual_x,
            start_win_virtual_y: (*window).virtual_y,
            start_tiling_mode: (*window).tiling_mode,
            start_mode_locked: (*window).mode_locked,
            start_pan_x: (*(*window).server).wm.desk_pan_x,
            start_pan_y: (*(*window).server).wm.desk_pan_y,
            started_in_overview: (*(*window).server).wm.mode == crate::window_manager::WindowManagerMode::Overview,
        });
        cursor.op_start_pointer();
        let cursor_name = crate::cursor::get_resize_cursor_name(edges);
        cursor.set_xcursor(cursor_name.as_ptr() as *const _);

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
            ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_NO_PREFERENCE
        }
        ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE => {
            ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_PREFERS_CSD
        }
        ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE => {
            ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_PREFERS_SSD
        }
        _ => ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_NO_PREFERENCE,
    };
    (*window).set_decoration_hint(hint);

    if ffi::river_wlr_xdg_surface_get_initialized(base) {
        let mut mode = (*(*decoration).wlr_decoration).requested_mode;
        if mode == ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_NONE {
            let server = (*window).server;
            let rule_ssd = (*server).wm.get_rule_for_window(window).and_then(|r| r.ssd);
            if let Some(true) = rule_ssd {
                mode = ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE;
            } else {
                mode = ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE;
            }
        }
        ffi::wlr_xdg_toplevel_decoration_v1_set_mode((*decoration).wlr_decoration, mode);
        (*window).wm_requested.ssd = mode == ffi::wlr_xdg_toplevel_decoration_v1_mode_WLR_XDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE;
        (*(*window).server).wm.dirty_windowing();
    }
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


