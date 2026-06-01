// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, WlList, wl_list_insert, wl_list_remove, wl_listener_remove};
use crate::slotmap::{SlotMap, Key};
use std::hash::{Hash, Hasher};

pub use crate::window::Window;
pub use crate::shell_surface::ShellSurface;

pub use crate::xwayland_override_redirect::XwaylandOverrideRedirect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowManagerState {
    Idle,
    Manage,
    InflightConfigures(u32),
    Render,
}

pub struct WindowManagerScheduled {
    pub dirty: bool,
    pub dirty_lazy: bool,
    pub output_config: *mut ffi::wlr_output_configuration_v1,
}

pub struct WindowManagerSent {
    pub session_locked: bool,
    pub outputs: ffi::wl_list,
    pub output_config: *mut ffi::wlr_output_configuration_v1,
    pub seats: ffi::wl_list,
}

pub struct WindowManagerRenderingScheduled {
    pub dirty: bool,
}

pub struct WindowManagerRenderingRequested {
    pub list: ffi::wl_list,
    pub order_hash: u64,
}

pub struct WindowManager {
    pub server: *mut Server,
    pub global: *mut ffi::wl_global,
    pub server_destroy: ffi::wl_listener,
    pub object: *mut ffi::wl_resource,
    pub state: WindowManagerState,
    pub windows: SlotMap<*mut Window>,
    pub scheduled: WindowManagerScheduled,
    pub sent: WindowManagerSent,
    pub rendering_scheduled: WindowManagerRenderingScheduled,
    pub rendering_requested: WindowManagerRenderingRequested,
    pub dirty_idle: *mut ffi::wl_event_source,
    pub timeout: *mut ffi::wl_event_source,
}

impl WindowManager {
    pub unsafe fn init(&mut self) -> Result<(), ()> {
        // This is a stub for the 0-arg struct instantiation.
        // We will call the real initialization with the server parameter.
        ffi::wl_list_init(&mut self.sent.outputs);
        self.scheduled.output_config = std::ptr::null_mut();
        self.sent.output_config = std::ptr::null_mut();
        Ok(())
    }

    pub unsafe fn init_with_server(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        self.global = std::ptr::null_mut();
        self.object = std::ptr::null_mut();
        self.state = WindowManagerState::Idle;
        self.windows = SlotMap::new();
        self.scheduled = WindowManagerScheduled {
            dirty: false,
            dirty_lazy: false,
            output_config: std::ptr::null_mut(),
        };
        self.sent = WindowManagerSent {
            session_locked: false,
            outputs: std::mem::zeroed(),
            output_config: std::ptr::null_mut(),
            seats: std::mem::zeroed(),
        };
        self.rendering_scheduled = WindowManagerRenderingScheduled {
            dirty: false,
        };
        self.rendering_requested = WindowManagerRenderingRequested {
            list: std::mem::zeroed(),
            order_hash: 0,
        };
        self.dirty_idle = std::ptr::null_mut();

        ffi::wl_list_init(&mut self.sent.outputs);
        ffi::wl_list_init(&mut self.sent.seats);
        ffi::wl_list_init(&mut self.rendering_requested.list);

        let event_loop = ffi::wl_display_get_event_loop((*server).wl_server);
        self.timeout = ffi::wl_event_loop_add_timer(event_loop, Some(handle_timeout), self as *mut WindowManager as *mut _);
        if self.timeout.is_null() {
            return Err("Failed to create timer event source");
        }

        self.global = ffi::wl_global_create(
            (*server).wl_server,
            &ffi::river_window_manager_v1_interface,
            4,
            self as *mut WindowManager as *mut _,
            Some(bind),
        );
        if self.global.is_null() {
            ffi::wl_event_source_remove(self.timeout);
            return Err("Failed to create river_window_manager_v1 global");
        }

        let server_destroy_ptr = &mut self.server_destroy as *mut ffi::wl_listener as *mut WlListener;
        (*server_destroy_ptr).notify = Some(handle_server_destroy);
        ffi::wl_display_add_destroy_listener((*server).wl_server, &mut self.server_destroy);

        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        if !self.global.is_null() {
            ffi::wl_global_destroy(self.global);
            self.global = std::ptr::null_mut();
        }
        if !self.timeout.is_null() {
            ffi::wl_event_source_remove(self.timeout);
            self.timeout = std::ptr::null_mut();
        }
        wl_listener_remove(&mut self.server_destroy);
    }

    pub unsafe fn ensure_windowing(&self) -> bool {
        match self.state {
            WindowManagerState::Manage => true,
            _ => {
                if !self.object.is_null() {
                    ffi::wl_resource_post_error(
                        self.object,
                        ffi::river_window_manager_v1_error_RIVER_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
                        b"invalid modification of window management state\0".as_ptr() as *const _,
                    );
                }
                false
            }
        }
    }

    pub unsafe fn ensure_rendering(&self) -> bool {
        match self.state {
            WindowManagerState::Manage | WindowManagerState::InflightConfigures(_) | WindowManagerState::Render => true,
            WindowManagerState::Idle => {
                if !self.object.is_null() {
                    ffi::wl_resource_post_error(
                        self.object,
                        ffi::river_window_manager_v1_error_RIVER_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
                        b"invalid modification of rendering state\0".as_ptr() as *const _,
                    );
                }
                false
            }
        }
    }

    pub unsafe fn dirty_windowing(&mut self) {
        self.scheduled.dirty = true;
        self.add_dirty_idle();
    }

    pub unsafe fn dirty_windowing_lazy(&mut self) {
        self.scheduled.dirty_lazy = true;
        self.add_dirty_idle();
    }

    pub unsafe fn clean_windowing(&mut self) {
        self.scheduled.dirty = false;
        self.remove_dirty_idle();
    }

    pub unsafe fn dirty_rendering(&mut self) {
        self.rendering_scheduled.dirty = true;
        self.add_dirty_idle();
    }

    pub unsafe fn clean_rendering(&mut self) {
        self.rendering_scheduled.dirty = false;
        self.remove_dirty_idle();
    }

    unsafe fn add_dirty_idle(&mut self) {
        if self.scheduled.dirty || self.scheduled.dirty_lazy || self.rendering_scheduled.dirty {
            if self.dirty_idle.is_null() {
                let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
                self.dirty_idle = ffi::wl_event_loop_add_idle(
                    event_loop,
                    Some(dirty_idle_callback),
                    self as *mut WindowManager as *mut _,
                );
            }
        }
    }

    unsafe fn remove_dirty_idle(&mut self) {
        if !self.scheduled.dirty && !self.rendering_scheduled.dirty {
            if !self.dirty_idle.is_null() {
                ffi::wl_event_source_remove(self.dirty_idle);
                self.dirty_idle = std::ptr::null_mut();
            }
        }
    }

    pub unsafe fn manage_start(&mut self) {
        assert!(matches!(self.state, WindowManagerState::Idle));
        assert!(self.scheduled.dirty);
        self.clean_windowing();
        self.state = WindowManagerState::Manage;

        log::debug!("manage sequence start");

        let session_locked = (*self.server).lock_manager.state == crate::lock_manager::LockState::Locked;
        if session_locked != self.sent.session_locked {
            if !self.object.is_null() {
                if session_locked {
                    ffi::wl_resource_post_event(self.object, ffi::RIVER_WINDOW_MANAGER_V1_SESSION_LOCKED);
                } else {
                    ffi::wl_resource_post_event(self.object, ffi::RIVER_WINDOW_MANAGER_V1_SESSION_UNLOCKED);
                }
            }
            self.sent.session_locked = session_locked;
        }

        (*self.server).om.auto_layout();

        let outputs = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs).next;
        while curr != outputs {
            let next = (*curr).next;
            let output = crate::container_of!(curr, crate::output::Output, link);
            (*output).manage_start();
            curr = next;
        }

        if !self.sent.output_config.is_null() {
            log::warn!("sent.output_config was not null in manage_start, destroying old configuration");
            ffi::wlr_output_configuration_v1_send_failed(self.sent.output_config);
            ffi::wlr_output_configuration_v1_destroy(self.sent.output_config);
            self.sent.output_config = std::ptr::null_mut();
        }
        self.sent.output_config = self.scheduled.output_config;
        self.scheduled.output_config = std::ptr::null_mut();

        for &win_ptr in self.windows.iter() {
            (*win_ptr).manage_start();
        }

        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            (*seat).manage_start();
            curr = next;
        }

        if !self.object.is_null() {
            ffi::wl_resource_post_event(self.object, ffi::RIVER_WINDOW_MANAGER_V1_MANAGE_START);
            self.start_timeout_timer(3000);
        } else {
            self.manage_finish();
        }
    }

    pub unsafe fn manage_finish(&mut self) {
        assert!(matches!(self.state, WindowManagerState::Manage));
        self.cancel_timeout_timer();

        log::debug!("manage sequence finish");

        let seats = &mut self.sent.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link_sent);
            (*seat).manage_finish();
            curr = next;
        }

        self.state = WindowManagerState::InflightConfigures(0);

        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            match (*node).get() {
                crate::wm_node::WmNodeType::Window(window) => {
                    if (*window).manage_finish() {
                        if let WindowManagerState::InflightConfigures(ref mut count) = self.state {
                            *count += 1;
                        }
                    }
                }
                _ => {}
            }
            curr = next;
        }

        if let WindowManagerState::InflightConfigures(count) = self.state {
            log::debug!("sent {} tracked configure(s)", count);
            if count > 0 {
                self.start_timeout_timer(100);
            } else {
                self.render_start();
            }
        }
    }

    unsafe fn start_timeout_timer(&mut self, ms: u32) {
        if !self.timeout.is_null() {
            ffi::wl_event_source_timer_update(self.timeout, ms as i32);
        }
    }

    unsafe fn cancel_timeout_timer(&mut self) {
        if !self.timeout.is_null() {
            ffi::wl_event_source_timer_update(self.timeout, 0);
        }
    }

    pub unsafe fn notify_configured(&mut self) {
        if let WindowManagerState::InflightConfigures(ref mut count) = self.state {
            *count -= 1;
            if *count == 0 {
                self.cancel_timeout_timer();
                self.render_start();
            }
        }
    }

    pub unsafe fn render_start(&mut self) {
        assert!(matches!(self.state, WindowManagerState::InflightConfigures(0)) ||
                (matches!(self.state, WindowManagerState::Idle) && self.rendering_scheduled.dirty));
        self.state = WindowManagerState::Render;
        self.clean_rendering();

        log::debug!("render sequence start");

        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            match (*node).get() {
                crate::wm_node::WmNodeType::Window(window) => {
                    (*window).render_start();
                }
                _ => {}
            }
            curr = next;
        }

        if !self.object.is_null() {
            ffi::wl_resource_post_event(self.object, ffi::RIVER_WINDOW_MANAGER_V1_RENDER_START);
            self.start_timeout_timer(3000);
        } else {
            self.render_finish();
        }
    }

    pub unsafe fn render_finish(&mut self) {
        assert!(matches!(self.state, WindowManagerState::Render));
        self.state = WindowManagerState::Idle;
        self.cancel_timeout_timer();

        log::debug!("render sequence finish");

        for &window in self.windows.iter() {
            if !matches!((*window).state, crate::window::WindowState::Closing) {
                (*window).surfaces.drop_saved();
            }
            if matches!((*window).state, crate::window::WindowState::Init) {
                ffi::wlr_scene_node_reparent((*window).tree as *mut ffi::wlr_scene_node, (*self.server).scene.hidden_tree);
            }
            if let crate::window::WindowImpl::Destroying = (*window).impl_type {
                Window::destroy(window);
            }
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            match (*node).get() {
                crate::wm_node::WmNodeType::Window(window) => {
                    (*window).ref_key.hash(&mut hasher);
                    rendered_fullscreen(window).hash(&mut hasher);
                }
                crate::wm_node::WmNodeType::ShellSurface(shell_surface) => {
                    (shell_surface as usize).hash(&mut hasher);
                }
            }
            curr = next;
        }
        let new_order_hash = hasher.finish();
        let reorder = self.rendering_requested.order_hash != new_order_hash;
        self.rendering_requested.order_hash = new_order_hash;

        let mut found_fullscreen = false;
        curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            match (*node).get() {
                crate::wm_node::WmNodeType::Window(window) => {
                    (*window).render_finish();
                    if reorder {
                        ffi::wlr_scene_node_reparent((*window).popup_tree as *mut _, (*self.server).scene.layers.popups);
                        if rendered_fullscreen(window) {
                            ffi::wlr_scene_node_reparent((*window).tree as *mut _, (*self.server).scene.layers.fullscreen);
                            ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                            found_fullscreen = true;
                        } else {
                            ffi::wlr_scene_node_reparent((*window).tree as *mut _, (*self.server).scene.layers.wm);
                            ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                        }
                    }
                }
                crate::wm_node::WmNodeType::ShellSurface(shell_surface) => {
                    (*shell_surface).render_finish();
                    if reorder {
                        ffi::wlr_scene_node_reparent((*shell_surface).popup_tree as *mut _, (*self.server).scene.layers.popups);
                        if found_fullscreen {
                            ffi::wlr_scene_node_reparent((*shell_surface).tree as *mut _, (*self.server).scene.layers.fullscreen);
                        } else {
                            ffi::wlr_scene_node_reparent((*shell_surface).tree as *mut _, (*self.server).scene.layers.wm);
                        }
                        ffi::wlr_scene_node_raise_to_top((*shell_surface).tree as *mut _);
                    }
                }
            }
            curr = next;
        }

        (*self.server).om.commit_output_state(self.server);

        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            (*seat).cursor.update_hovered();
            curr = next;
        }

        (*self.server).idle_inhibit_manager.check_active();

        log::debug!("finished committing transaction");

        if self.scheduled.dirty || self.scheduled.dirty_lazy || self.rendering_scheduled.dirty {
            self.add_dirty_idle();
        }
    }
}

// Deprecated sent_outputs that is part of structural layout compatibility
pub struct WindowManagerScheduledCompat {
    pub output_config: *mut ffi::wlr_output_configuration_v1,
}
pub struct WindowManagerSentCompat {
    pub outputs: ffi::wl_list,
    pub output_config: *mut ffi::wlr_output_configuration_v1,
}
impl WindowManager {
    // Add legacy fields so structural offsets are preserved if layout-based code is compiled
    pub fn sent_outputs_compat(&self) {}
}

unsafe fn rendered_fullscreen(window: *mut Window) -> bool {
    !(*window).wm_requested.fullscreen.is_null() && !(*window).rendering_requested.hidden
}

unsafe extern "C" fn dirty_idle_callback(data: *mut std::ffi::c_void) {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    (*wm).dirty_idle = std::ptr::null_mut();
    match (*wm).state {
        WindowManagerState::Idle => {
            if (*wm).rendering_scheduled.dirty {
                (*wm).render_start();
            } else if (*wm).scheduled.dirty || (*wm).scheduled.dirty_lazy {
                (*wm).scheduled.dirty = true;
                (*wm).scheduled.dirty_lazy = false;
                (*wm).manage_start();
            }
        }
        _ => {}
    }
}

unsafe extern "C" fn handle_timeout(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }

    match (*wm).state {
        WindowManagerState::InflightConfigures(count) => {
            log::error!("timeout occurred, some imperfect frames may be shown");
            assert!(count > 0);
            (*wm).state = WindowManagerState::InflightConfigures(0);
            (*wm).render_start();
        }
        WindowManagerState::Manage | WindowManagerState::Render => {
            if !(*wm).object.is_null() {
                log::error!("window manager unresponsive for more than 3 seconds, disconnecting");
                ffi::wl_resource_post_error(
                    (*wm).object,
                    ffi::river_window_manager_v1_error_RIVER_WINDOW_MANAGER_V1_ERROR_UNRESPONSIVE,
                    b"unresponsive for more than 3 seconds\0".as_ptr() as *const _,
                );
                let client = ffi::wl_resource_get_client((*wm).object);
                ffi::wl_client_destroy(client);
            }
        }
        WindowManagerState::Idle => {}
    }
    0
}

unsafe extern "C" fn handle_server_destroy(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let wm = crate::container_of!(listener, WindowManager, server_destroy);
    (*wm).deinit();
}

// WM request handlers
unsafe extern "C" fn wm_stop(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if !wm.is_null() {
        (*wm).object = std::ptr::null_mut();
        ffi::wl_resource_post_event(resource, ffi::RIVER_WINDOW_MANAGER_V1_FINISHED);
        ffi::wl_resource_set_implementation(
            resource,
            &INERT_WM_INTERFACE as *const _ as *const _,
            std::ptr::null_mut(),
            None,
        );
    }
}

unsafe extern "C" fn wm_destroy(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn wm_manage_finish(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    if !matches!((*wm).state, WindowManagerState::Manage) {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_window_manager_v1_error_RIVER_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
            b"manage_finish request does not match manage_start\0".as_ptr() as *const _,
        );
        return;
    }
    (*wm).manage_finish();
}

unsafe extern "C" fn wm_manage_dirty(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    (*wm).scheduled.dirty_lazy = true;
    (*wm).add_dirty_idle();
}

unsafe extern "C" fn wm_render_finish(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    if !matches!((*wm).state, WindowManagerState::Render) {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_window_manager_v1_error_RIVER_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
            b"render_finish request does not match render_start\0".as_ptr() as *const _,
        );
        return;
    }
    (*wm).render_finish();
}

unsafe extern "C" fn wm_get_shell_surface(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    surface_resource: *mut ffi::wl_resource,
) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    let surface = ffi::wlr_surface_from_resource(surface_resource);
    let version = ffi::wl_resource_get_version(resource) as u32;
    if let Err(e) = crate::shell_surface::ShellSurface::create(client, version, id, surface, (*wm).server) {
        log::error!("Failed to create shell surface: {}", e);
        ffi::wl_client_post_no_memory(client);
    }
}

unsafe extern "C" fn wm_exit_session(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    log::info!("window manager requested to exit session");
    ffi::wl_display_terminate((*(*wm).server).wl_server);
}

static WM_INTERFACE: ffi::river_window_manager_v1_interface = ffi::river_window_manager_v1_interface {
    stop: Some(wm_stop),
    destroy: Some(wm_destroy),
    manage_finish: Some(wm_manage_finish),
    manage_dirty: Some(wm_manage_dirty),
    render_finish: Some(wm_render_finish),
    get_shell_surface: Some(wm_get_shell_surface),
    exit_session: Some(wm_exit_session),
};

static INERT_WM_INTERFACE: ffi::river_window_manager_v1_interface = ffi::river_window_manager_v1_interface {
    stop: None,
    destroy: Some(wm_destroy),
    manage_finish: None,
    manage_dirty: None,
    render_finish: None,
    get_shell_surface: None,
    exit_session: None,
};

unsafe extern "C" fn bind(
    client: *mut ffi::wl_client,
    data: *mut std::ffi::c_void,
    version: u32,
    id: u32,
) {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return;
    }

    let resource = ffi::wl_resource_create(client, &ffi::river_window_manager_v1_interface, version as i32, id);
    if resource.is_null() {
        ffi::wl_client_post_no_memory(client);
        log::error!("out of memory binding river_window_manager_v1");
        return;
    }

    if !(*wm).object.is_null() {
        ffi::wl_resource_post_event(resource, ffi::RIVER_WINDOW_MANAGER_V1_UNAVAILABLE);
        ffi::wl_resource_set_implementation(
            resource,
            &INERT_WM_INTERFACE as *const _ as *const _,
            std::ptr::null_mut(),
            None,
        );
        return;
    }

    (*wm).object = resource;
    ffi::wl_resource_set_implementation(
        resource,
        &WM_INTERFACE as *const _ as *const _,
        wm as *mut _,
        Some(handle_destroy_wm_resource),
    );
    (*wm).dirty_windowing();
}

unsafe extern "C" fn handle_destroy_wm_resource(resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    if (*wm).object != resource {
        return;
    }
    log::debug!("active river_window_manager_v1 destroyed");
    (*wm).object = std::ptr::null_mut();

    let server = (*wm).server;

    // Iterate over outputs and make inert
    let outputs_list = &mut (*server).om.outputs as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*outputs_list).next;
    while curr != outputs_list {
        let next = (*curr).next;
        let output = crate::container_of!(curr, crate::output::Output, link);
        (*output).make_inert();
        curr = next;
    }

    // Iterate over seats and make inert
    let seats_list = &mut (*server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
    curr = (*seats_list).next;
    while curr != seats_list {
        let next = (*curr).next;
        let seat = crate::container_of!(curr, crate::seat::Seat, link);
        (*seat).make_inert();
        
        let bindings_head = &mut (*seat).xkb_bindings as *mut ffi::wl_list as *mut WlList;
        let mut curr_b = (*bindings_head).next;
        while curr_b != bindings_head {
            let next_b = (*curr_b).next;
            let binding = crate::container_of!(curr_b, crate::xkb_bindings::XkbBinding, link);
            (*binding).wm_scheduled.state_changes.clear();
            curr_b = next_b;
        }
        
        curr = next;
    }

    // Iterate over windows and make inert
    for &window in (*wm).windows.iter() {
        (*window).make_inert();
    }

    match (*wm).state {
        WindowManagerState::Idle | WindowManagerState::InflightConfigures(_) => {}
        WindowManagerState::Manage => (*wm).manage_finish(),
        WindowManagerState::Render => (*wm).render_finish(),
    }
}
