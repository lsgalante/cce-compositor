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
    pub active_tags: u32,
    pub global_layout: crate::tiling::TilingMode,
    pub tag_layouts: [crate::tiling::TilingMode; 4],
    pub has_tag_layout: [bool; 4],
    pub layout: crate::config::Layout,
    pub mode_rules: Vec<crate::config::ModeRule>,
    pub keybinds: Vec<crate::config::Keybind>,
    pub pointer_binds: Vec<crate::config::PointerBind>,
    pub gesture_binds: Vec<crate::config::GestureBind>,
    pub ipc_rx: Option<std::sync::mpsc::Receiver<crate::ipc_server::IpcRequest>>,
    pub ipc_timer: *mut ffi::wl_event_source,
    pub startup: Vec<crate::config::StartupConfig>,
    pub status_sender: Option<crate::status_server::StatusSender>,
    pub output_scale: f32,
    pub input_rules: Vec<crate::config::InputDeviceConfigRule>,
    pub input_config: crate::config::InputConfig,
    pub expose_active: bool,
    pub last_status_update: std::cell::RefCell<Option<crate::status_server::StatusUpdate>>,
}

impl WindowManager {
    pub unsafe fn init(&mut self) -> Result<(), ()> {
        // This is a stub for the 0-arg struct instantiation.
        // We will call the real initialization with the server parameter.
        ffi::wl_list_init(&mut self.sent.outputs);
        self.scheduled.output_config = std::ptr::null_mut();
        self.sent.output_config = std::ptr::null_mut();
        self.output_scale = 1.0;
        self.input_rules = Vec::new();
        self.input_config = crate::config::InputConfig::default();
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
        self.active_tags = 1;
        self.global_layout = crate::tiling::TilingMode::Cascade;
        self.tag_layouts = [crate::tiling::TilingMode::Cascade; 4];
        self.has_tag_layout = [false; 4];
        self.layout = crate::config::Layout::default();
        self.output_scale = 1.0;
        self.mode_rules = Vec::new();
        self.keybinds = Vec::new();
        self.pointer_binds = Vec::new();
        self.gesture_binds = Vec::new();
        self.ipc_rx = None;
        self.ipc_timer = std::ptr::null_mut();
        self.startup = Vec::new();
        self.status_sender = None;
        self.input_rules = Vec::new();
        self.input_config = crate::config::InputConfig::default();
        self.expose_active = false;
        self.last_status_update = std::cell::RefCell::new(None);

        ffi::wl_list_init(&mut self.sent.outputs);
        ffi::wl_list_init(&mut self.sent.seats);
        ffi::wl_list_init(&mut self.rendering_requested.list);

        let event_loop = ffi::wl_display_get_event_loop((*server).wl_server);
        self.timeout = ffi::wl_event_loop_add_timer(event_loop, Some(handle_timeout), self as *mut WindowManager as *mut _);
        if self.timeout.is_null() {
            return Err("Failed to create timer event source");
        }

        self.ipc_rx = None;
        self.ipc_timer = ffi::wl_event_loop_add_timer(event_loop, Some(handle_ipc_timer), self as *mut WindowManager as *mut _);
        if self.ipc_timer.is_null() {
            return Err("Failed to create IPC timer event source");
        }
        ffi::wl_event_source_timer_update(self.ipc_timer, 10);

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

    pub fn start_ipc(&mut self) {
        if self.ipc_rx.is_none() {
            let rx = crate::ipc_server::spawn_ipc_server();
            self.ipc_rx = Some(rx);
        }
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
        let bt = std::backtrace::Backtrace::force_capture();
        log::info!("dirty_windowing called from backtrace:\n{}", bt);
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

        self.arrange_views();

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

        self.keep_status_bar_on_top();

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
                    (*window).rendering_requested.circular.hash(&mut hasher);
                    (*window).rendering_requested.hidden.hash(&mut hasher);
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
                        if (*window).rendering_requested.hidden {
                            ffi::wlr_scene_node_reparent((*window).tree as *mut _, (*self.server).scene.hidden_tree);
                            ffi::wlr_scene_node_reparent((*window).popup_tree as *mut _, (*self.server).scene.hidden_tree);
                        } else {
                            ffi::wlr_scene_node_reparent((*window).popup_tree as *mut _, (*self.server).scene.layers.popups);
                            if rendered_fullscreen(window) {
                                ffi::wlr_scene_node_reparent((*window).tree as *mut _, (*self.server).scene.layers.fullscreen);
                                ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                                found_fullscreen = true;
                            } else if (*window).rendering_requested.circular {
                                ffi::wlr_scene_node_reparent((*window).tree as *mut _, (*self.server).scene.layers.top);
                                ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                            } else {
                                ffi::wlr_scene_node_reparent((*window).tree as *mut _, (*self.server).scene.layers.wm);
                                ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                            }
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

    pub unsafe fn get_rule_for_window(&self, win: *mut Window) -> Option<&crate::config::ModeRule> {
        let app_id = (*win).get_app_id_string();
        let title = (*win).get_title_string();

        for rule in &self.mode_rules {
            let match_app = rule.app_id_pattern == "*"
                || app_id.as_ref().map_or(false, |aid| aid.contains(&rule.app_id_pattern));
            let match_title = rule.title_pattern.as_ref().map_or(true, |tp| {
                title.as_ref().map_or(false, |t| t.contains(tp))
            });

            if match_app && match_title {
                return Some(rule);
            }
        }
        None
    }

    pub unsafe fn get_mode_for_window(&self, win: *mut Window) -> crate::tiling::TilingMode {
        let app_id = (*win).get_app_id_string();
        if app_id.as_deref() == Some("cce-status-interface") {
            return crate::tiling::TilingMode::Fullscreen;
        }
        if app_id.as_deref() == Some("cce-notification-daemon") || app_id.as_deref() == Some("clear-notification-daemon") {
            return crate::tiling::TilingMode::Popup;
        }

        if self.expose_active {
            return crate::tiling::TilingMode::Expose;
        }

        if (*win).mode_locked {
            return (*win).tiling_mode;
        }

        if (*win).has_parent {
            return crate::tiling::TilingMode::Floating;
        }

        if let Some(rule) = self.get_rule_for_window(win) {
            return rule.mode;
        }

        // Check tag layouts
        for tag_bit in 0..4 {
            let tag_mask = 1u32 << tag_bit;
            if ((*win).tags & tag_mask) != 0 && self.has_tag_layout[tag_bit] {
                return self.tag_layouts[tag_bit];
            }
        }

        self.global_layout
    }

    pub unsafe fn get_active_resize_dimensions(&self, win_ptr: *mut Window) -> Option<(u32, u32)> {
        let seats_list = &(*self.server).input_manager.seats as *const ffi::wl_list as *const WlList as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let Some(ref op) = (*seat).op {
                if op.window_ptr == win_ptr {
                    if let crate::seat::PointerOpType::Resize { edges } = op.op_type {
                        let dx = op.x - op.start_x;
                        let dy = op.y - op.start_y;
                        let mut new_w = op.start_win_w;
                        let mut new_h = op.start_win_h;

                        if edges.left {
                            new_w = std::cmp::max(50, op.start_win_w as i32 - dx) as u32;
                        } else if edges.right {
                            new_w = std::cmp::max(50, op.start_win_w as i32 + dx) as u32;
                        }

                        if edges.top {
                            new_h = std::cmp::max(50, op.start_win_h as i32 - dy) as u32;
                        } else if edges.bottom {
                            new_h = std::cmp::max(50, op.start_win_h as i32 + dy) as u32;
                        }
                        return Some((new_w, new_h));
                    }
                }
            }
            curr_seat = (*curr_seat).next;
        }
        None
    }

    pub unsafe fn arrange_views(&mut self) {
        log::info!("Monolithic arrange_views triggered. Windows: {}", self.windows.count());
        for (idx, &win_ptr) in self.windows.iter().enumerate() {
            if win_ptr.is_null() { continue; }
            let title = (*win_ptr).get_title_string().unwrap_or_else(|| "None".to_string());
            let aid = (*win_ptr).get_app_id_string().unwrap_or_else(|| "None".to_string());
            log::info!("  window #{}: title={:?}, app_id={:?}, state={:?}, closed={}", idx, title, aid, (*win_ptr).state, (*win_ptr).closed);
        }
        
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        
        let mut active_outputs: Vec<*mut crate::output::Output> = Vec::new();
        while curr_out != outputs_list {
            let next_out = (*curr_out).next;
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                active_outputs.push(output);
            }
            curr_out = next_out;
        }

        if active_outputs.is_empty() {
            return;
        }

        let mut focused_window: *mut Window = std::ptr::null_mut();
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let crate::seat::Focus::Window(w) = (*seat).focused {
                focused_window = w;
                break;
            }
            curr_seat = next_seat;
        }

        let mut stack_order: Vec<*mut Window> = Vec::new();
        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            if let crate::wm_node::WmNodeType::Window(window) = (*node).get() {
                stack_order.push(window);
            }
            curr = next;
        }

        for &output in &active_outputs {
            let wlr_box = (*output).sent.box_layout();
            let phys_x = wlr_box.x;
            let phys_y = wlr_box.y;
            let phys_w = wlr_box.width;
            let phys_h = wlr_box.height;

            let mut usable_x = phys_x;
            let mut usable_y = phys_y;
            let mut usable_w = phys_w;
            let mut usable_h = phys_h;

            let non_ex = (*output).layer_shell.scheduled.non_exclusive_area;
            if non_ex.width > 0 && non_ex.height > 0 {
                usable_x = phys_x + non_ex.x;
                usable_y = phys_y + non_ex.y;
                usable_w = non_ex.width;
                usable_h = non_ex.height;
            }

            let mut tiled_windows: Vec<*mut Window> = Vec::new();
            let mut floating_windows: Vec<*mut Window> = Vec::new();
            let mut side_panel_windows: Vec<*mut Window> = Vec::new();

            for &win_ptr in self.windows.iter() {
                if (*win_ptr).closed {
                    continue;
                }

                let app_id = (*win_ptr).get_app_id_string();
                let is_status_bar = app_id.as_deref() == Some("cce-status-interface");
                let visible = is_status_bar || ((*win_ptr).tags & self.active_tags) != 0;
                if !visible {
                    ffi::wlr_scene_node_set_enabled((*win_ptr).tree as *mut ffi::wlr_scene_node, false);
                    (*win_ptr).rendering_requested.hidden = true;
                    continue;
                }

                ffi::wlr_scene_node_set_enabled((*win_ptr).tree as *mut ffi::wlr_scene_node, true);
                (*win_ptr).rendering_requested.hidden = false;

                let mode = self.get_mode_for_window(win_ptr);
                if !self.expose_active {
                    (*win_ptr).tiling_mode = mode;
                }

                // Apply ModeRule SSD configuration if defined and not locked
                if !(*win_ptr).mode_locked {
                    if let Some(rule) = self.get_rule_for_window(win_ptr) {
                        if let Some(rule_ssd) = rule.ssd {
                            (*win_ptr).wm_requested.ssd = rule_ssd;
                        }
                    }
                }

                let app_id = (*win_ptr).get_app_id_string();

                if app_id.as_deref() == Some("cce-status-interface") {
                    let wlr_box = (*output).sent.box_layout();
                    let bar_h = self.layout.bar_height as u32;
                    (*win_ptr).rendering_requested.x = wlr_box.x;
                    (*win_ptr).rendering_requested.y = wlr_box.y;
                    (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                        width: wlr_box.width as u32,
                        height: bar_h,
                    });
                    (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                        width: wlr_box.width as u32,
                        height: bar_h,
                    };
                    (*win_ptr).wm_requested.tiled = 0;
                    (*win_ptr).wm_requested.ssd = false;
                    continue;
                }

                if mode == crate::tiling::TilingMode::Floating || mode == crate::tiling::TilingMode::Popup {
                    floating_windows.push(win_ptr);
                } else if mode == crate::tiling::TilingMode::SidePanel {
                    side_panel_windows.push(win_ptr);
                } else {
                    tiled_windows.push(win_ptr);
                }
            }

            let side_panel_win = side_panel_windows.first().copied();
            let mut side_panel_w = 0;
            let mut shift_x = 0;
            if let Some(sp_win) = side_panel_win {
                let hint_min_w = (*sp_win).wm_scheduled.dimensions_hint.min_width as i32;
                side_panel_w = if hint_min_w > 32 {
                    std::cmp::max(self.layout.side_panel_width, hint_min_w)
                } else {
                    self.layout.side_panel_width
                };
                if self.layout.side_panel_behavior != "above" {
                    shift_x = side_panel_w + self.layout.side_panel_border_gap;
                }
            }

            let tiled_usable_w = (usable_w - shift_x).max(1);
            let tiled_usable_x = if self.layout.side_panel_position == "right" || self.layout.side_panel_behavior == "above" {
                usable_x
            } else {
                usable_x + shift_x
            };

            let n_tiled = tiled_windows.len() as i32;
            let current_layout = if n_tiled > 0 {
                self.get_mode_for_window(tiled_windows[0])
            } else {
                self.global_layout
            };

            log::info!("arrange_views: n_tiled = {}, current_layout = {:?}, expose_active = {}", n_tiled, current_layout, self.expose_active);

            if current_layout == crate::tiling::TilingMode::Cascade {
                tiled_windows.sort_by_key(|&w| stack_order.iter().position(|&x| x == w).unwrap_or(usize::MAX));
            }

            let gap = self.layout.gap;
            let gap_top = self.layout.gap_top;
            let gap_left = self.layout.gap_left;
            let gap_right = self.layout.gap_right;
            let gap_bottom = self.layout.gap_bottom;
            let bw = match current_layout {
                crate::tiling::TilingMode::Cascade => self.layout.cascade_border_width,
                crate::tiling::TilingMode::Grid | crate::tiling::TilingMode::Expose => self.layout.grid_border_width,
                crate::tiling::TilingMode::Fullscreen => self.layout.fullscreen_border_width,
                _ => self.layout.border_width,
            };
            let cascade_offset = self.layout.cascade_offset;
            let bar_height = self.layout.bar_height;

            for (idx, &win_ptr) in tiled_windows.iter().enumerate() {
                let win_bw = if !(*win_ptr).wm_requested.ssd {
                    0
                } else {
                    bw
                };
                let win_dec_h = if !(*win_ptr).wm_requested.ssd {
                    0
                } else {
                    std::cmp::max(win_bw, 16)
                };

                let (x, y, w, h) = match current_layout {
                    crate::tiling::TilingMode::Cascade => {
                        crate::tiling::tile_cascade(
                            tiled_usable_w, usable_h, gap, gap_top, gap_left, gap_right, gap_bottom,
                            win_bw, win_dec_h, cascade_offset, bar_height, n_tiled, idx as i32
                        )
                    }
                    crate::tiling::TilingMode::Grid => {
                        crate::tiling::tile_grid(
                            tiled_usable_w, usable_h, gap, gap_top, gap_left, gap_right, gap_bottom,
                            win_bw, win_dec_h, bar_height, n_tiled, idx as i32
                        )
                    }
                    crate::tiling::TilingMode::Expose => {
                        crate::tiling::tile_expose(
                            tiled_usable_w, usable_h, gap, gap_top, gap_left, gap_right, gap_bottom,
                            win_bw, win_dec_h, bar_height, n_tiled, idx as i32
                        )
                    }
                    crate::tiling::TilingMode::Fullscreen => {
                        crate::tiling::tile_fullscreen(
                            tiled_usable_w, usable_h, gap_top, gap_left, gap_right, gap_bottom,
                            win_bw, bar_height
                        )
                    }
                    _ => {
                        ((*win_ptr).box_geom.x, (*win_ptr).box_geom.y, (*win_ptr).box_geom.width as i32, (*win_ptr).box_geom.height as i32)
                    }
                };

                let mut final_x = tiled_usable_x + x;
                let mut final_y = usable_y + y;

                if self.expose_active {
                    let orig_w = (*win_ptr).box_geom.width;
                    let orig_h = (*win_ptr).box_geom.height;
                    let scale = if orig_w > 0 && orig_h > 0 {
                        let scale_x = w as f64 / orig_w as f64;
                        let scale_y = h as f64 / orig_h as f64;
                        scale_x.min(scale_y).min(1.0)
                    } else {
                        1.0
                    };
                    (*win_ptr).scale = scale;

                    let visual_w = orig_w as f64 * scale;
                    let visual_h = orig_h as f64 * scale;
                    let offset_x = (w as f64 - visual_w) / 2.0;
                    let offset_y = (h as f64 - visual_h) / 2.0;

                    final_x += offset_x as i32;
                    final_y += offset_y as i32;
                } else {
                    (*win_ptr).scale = 1.0;
                }

                log::info!("arrange_views: tiled window index {}, title = {:?}, app_id = {:?}, geom_box = (x={}, y={}, w={}, h={}), target_box = (x={}, y={}, w={}, h={}), scale = {}",
                    idx,
                    (*win_ptr).get_title_string(),
                    (*win_ptr).get_app_id_string(),
                    (*win_ptr).box_geom.x, (*win_ptr).box_geom.y, (*win_ptr).box_geom.width, (*win_ptr).box_geom.height,
                    final_x, final_y, w, h, (*win_ptr).scale
                );

                (*win_ptr).rendering_requested.x = final_x;
                (*win_ptr).rendering_requested.y = final_y;

                if !self.expose_active {
                    (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                        width: w as u32,
                        height: h as u32,
                    });
                    (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                        width: w as u32,
                        height: h as u32,
                    };
                    (*win_ptr).wm_requested.tiled = 1 | 2 | 4 | 8;
                }

                let is_focused = win_ptr == focused_window;
                let (r, g_val, b, mut a) = if is_focused {
                    (
                        self.layout.border_r,
                        self.layout.border_g,
                        self.layout.border_b,
                        self.layout.border_a,
                    )
                } else {
                    let mut tiled_stack = tiled_windows.clone();
                    tiled_stack.sort_by_key(|&w| stack_order.iter().position(|&x| x == w).unwrap_or(usize::MAX));
                    let pos = tiled_stack.iter().position(|&w| w == win_ptr).unwrap_or(0);
                    let depth = n_tiled - 1 - pos as i32;
                    let mut factor = 1.0_f64;
                    for _ in 0..depth {
                        factor *= 0.70; // UNFOCUSED_DEPTH_FACTOR
                    }
                    let r = blend_channel(self.layout.background_r, self.layout.border_r, factor);
                    let g = blend_channel(self.layout.background_g, self.layout.border_g, factor);
                    let b = blend_channel(self.layout.background_b, self.layout.border_b, factor);
                    let a = blend_channel(self.layout.background_a, self.layout.border_a, factor);
                    (r, g, b, a)
                };

                (*win_ptr).rendering_requested.border = crate::window::Border {
                    edges: crate::window::Edges { top: true, bottom: true, left: true, right: true },
                    width: bw as u32,
                    r,
                    g: g_val,
                    b,
                    a,
                };

                (*win_ptr).rendering_requested.blur = self.layout.window_blur;
                (*win_ptr).rendering_requested.opacity = if is_focused { 1.0f32 } else {
                    if self.layout.transparency_opacity >= 1.0 { 1.0f32 } else { 0.85f32 }
                };
            }

            let g = self.layout.side_panel_border_gap;
            let dec_h = std::cmp::max(bw, 16);
            for (sp_idx, &win_ptr) in side_panel_windows.iter().enumerate() {
                if sp_idx == 0 {
                    let sp_x = if self.layout.side_panel_position == "right" {
                        usable_x + usable_w - side_panel_w - g + bw
                    } else {
                        usable_x + g + bw
                    };
                    let sp_y = bar_height + usable_y + dec_h + g;
                    let sp_h = (usable_h - bar_height - (dec_h + bw) - 2 * g).max(1);
                    let sp_w = (side_panel_w - bw * 2).max(1);

                    (*win_ptr).rendering_requested.x = sp_x;
                    (*win_ptr).rendering_requested.y = sp_y;
                    (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                        width: sp_w as u32,
                        height: sp_h as u32,
                    });
                    (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                        width: sp_w as u32,
                        height: sp_h as u32,
                    };
                    (*win_ptr).wm_requested.tiled = 1 | 2 | 4 | 8;

                    let opacity_factor = if self.layout.transparency_opacity >= 1.0 { 1.0f32 } else { self.layout.side_panel_border_opacity as f32 / 100.0 };
                    let is_focused = win_ptr == focused_window;
                    let r = self.layout.border_r;
                    let g_color = self.layout.border_g;
                    let b = self.layout.border_b;
                    let a = (self.layout.border_a as f32 * opacity_factor) as u32;

                    (*win_ptr).rendering_requested.border = crate::window::Border {
                        edges: crate::window::Edges { top: true, bottom: true, left: true, right: true },
                        width: bw as u32,
                        r,
                        g: g_color,
                        b,
                        a,
                    };
                    (*win_ptr).rendering_requested.blur = self.layout.window_blur;
                    (*win_ptr).rendering_requested.opacity = if is_focused { 1.0f32 } else {
                        if self.layout.transparency_opacity >= 1.0 { 1.0f32 } else { 0.85f32 * opacity_factor }
                    };
                } else {
                    floating_windows.push(win_ptr);
                }
            }

            let mut idx_floating = 0;
            for &win_ptr in &floating_windows {
                let is_focused = win_ptr == focused_window;
                let r = self.layout.border_r;
                let g_val = self.layout.border_g;
                let b = self.layout.border_b;
                let a = self.layout.border_a;

                (*win_ptr).rendering_requested.border = crate::window::Border {
                    edges: crate::window::Edges { top: true, bottom: true, left: true, right: true },
                    width: bw as u32,
                    r,
                    g: g_val,
                    b,
                    a,
                };
                (*win_ptr).rendering_requested.blur = self.layout.window_blur;
                (*win_ptr).rendering_requested.opacity = if is_focused { 1.0f32 } else {
                    if self.layout.transparency_opacity >= 1.0 { 1.0f32 } else { 0.90f32 }
                };

                if (*win_ptr).tiling_mode == crate::tiling::TilingMode::Popup {
                    let hint_min_w = (*win_ptr).wm_scheduled.dimensions_hint.min_width as i32;
                    let hint_min_h = (*win_ptr).wm_scheduled.dimensions_hint.min_height as i32;
                    let fw = if (*win_ptr).box_geom.width > 0 {
                        (*win_ptr).box_geom.width as i32
                    } else if hint_min_w > 32 {
                        hint_min_w
                    } else {
                        360
                    };
                    let fh = if (*win_ptr).box_geom.height > 0 {
                        (*win_ptr).box_geom.height as i32
                    } else if hint_min_h > 32 {
                        hint_min_h
                    } else {
                        100
                    };
                    let fx = usable_x + usable_w - fw - self.layout.gap_right;
                    let fy = usable_y + bar_height + self.layout.gap_top;

                    (*win_ptr).rendering_requested.x = fx;
                    (*win_ptr).rendering_requested.y = fy;
                    (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                        width: fw as u32,
                        height: fh as u32,
                    });
                    (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                        width: fw as u32,
                        height: fh as u32,
                    };
                } else {
                    let fbw = self.layout.floating_border_width;
                    let mut fw = if let Some(resize_size) = self.get_active_resize_dimensions(win_ptr) {
                        resize_size.0 as i32
                    } else if (*win_ptr).box_geom.width > 0 {
                        (*win_ptr).box_geom.width as i32
                    } else if (*win_ptr).wm_scheduled.dimensions_hint.min_width > 32 {
                        (*win_ptr).wm_scheduled.dimensions_hint.min_width as i32
                    } else {
                        usable_w * 2 / 3
                    };
                    let mut fh = if let Some(resize_size) = self.get_active_resize_dimensions(win_ptr) {
                        resize_size.1 as i32
                    } else if (*win_ptr).box_geom.height > 0 {
                        (*win_ptr).box_geom.height as i32
                    } else if (*win_ptr).wm_scheduled.dimensions_hint.min_height > 32 {
                        (*win_ptr).wm_scheduled.dimensions_hint.min_height as i32
                    } else {
                        usable_h * 2 / 3
                    };

                    let max_w = usable_w - gap_left - gap_right;
                    let max_h = usable_h - bar_height - gap_top - gap_bottom;
                    fw = fw.clamp(1, max_w.max(1));
                    fh = fh.clamp(1, max_h.max(1));

                    let mut fx = if (*win_ptr).rendering_requested.x != 0 || (*win_ptr).rendering_requested.y != 0 {
                        (*win_ptr).rendering_requested.x
                    } else {
                        usable_x + gap_left + fbw + cascade_offset * idx_floating
                    };
                    let mut fy = if (*win_ptr).rendering_requested.x != 0 || (*win_ptr).rendering_requested.y != 0 {
                        (*win_ptr).rendering_requested.y
                    } else {
                        usable_y + gap_left + fbw + bar_height + gap_top + cascade_offset * idx_floating
                    };

                    let min_x = usable_x + gap_left;
                    let max_x = usable_x + usable_w - gap_right - fw;
                    let min_y = usable_y + bar_height + gap_top;
                    let max_y = usable_y + usable_h - gap_bottom - fh;

                    fx = fx.clamp(min_x, max_x.max(min_x));
                    fy = fy.clamp(min_y, max_y.max(min_y));

                    (*win_ptr).rendering_requested.x = fx;
                    (*win_ptr).rendering_requested.y = fy;
                    (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                        width: fw as u32,
                        height: fh as u32,
                    });
                    (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                        width: fw as u32,
                        height: fh as u32,
                    };

                    idx_floating += 1;
                }
            }
        }
        // If the focused window is no longer visible on the active tags, refocus
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            let mut focused_visible = false;
            match (*seat).focused {
                crate::seat::Focus::Window(w) => {
                    if !w.is_null() && !(*w).closed && !(*w).minimized && ((*w).tags & self.active_tags) != 0 {
                        focused_visible = true;
                    }
                }
                crate::seat::Focus::None => {
                    focused_visible = false;
                }
                _ => {
                    focused_visible = true;
                }
            }
            if !focused_visible {
                self.focus_next_visible_window(seat);
            }
            curr_seat = next_seat;
        }

        self.update_status();
        self.rendering_scheduled.dirty = true;
    }

    pub unsafe fn focused_window(&self) -> *mut crate::window::Window {
        let seats_list = &(*self.server).input_manager.seats as *const ffi::wl_list as *const WlList as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let crate::seat::Focus::Window(w) = (*seat).focused {
                return w;
            }
            curr_seat = (*curr_seat).next;
        }
        std::ptr::null_mut()
    }

    pub unsafe fn focused_layer_surface(&self) -> *mut ffi::wlr_surface {
        let seats_list = &(*self.server).input_manager.seats as *const ffi::wl_list as *const WlList as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let crate::seat::Focus::LayerSurface(s) = (*seat).focused {
                return s;
            }
            curr_seat = (*curr_seat).next;
        }
        std::ptr::null_mut()
    }


    pub unsafe fn update_status(&self) {
        if let Some(ref sender) = self.status_sender {
            let update = crate::status_server::build_status_update(self);
            let mut last = self.last_status_update.borrow_mut();
            if last.as_ref() != Some(&update) {
                sender.send(update.clone());
                *last = Some(update);
            }
        }
    }

    pub unsafe fn first_seat(&self) -> Option<*mut crate::seat::Seat> {
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        if curr_seat != seats_list {
            Some(crate::container_of!(curr_seat, crate::seat::Seat, link))
        } else {
            None
        }
    }

    pub unsafe fn focus_next_visible_window(&mut self, seat: *mut crate::seat::Seat) {
        let mut next_focus: *mut Window = std::ptr::null_mut();
        for &w in self.windows.iter() {
            if !w.is_null() && !(*w).closed && !(*w).minimized && ((*w).tags & self.active_tags) != 0 {
                let app_id = (*w).get_app_id_string();
                let is_status_bar = app_id.as_deref() == Some("cce-status-interface");
                if !is_status_bar {
                    next_focus = w;
                }
            }
        }
        if !next_focus.is_null() {
            (*seat).focus(crate::seat::Focus::Window(next_focus));
        } else {
            (*seat).focus(crate::seat::Focus::None);
        }
    }

    pub unsafe fn keep_status_bar_on_top(&mut self) {
        let mut status_bar_windows = Vec::new();
        for &win_ptr in self.windows.iter() {
            if win_ptr.is_null() || (*win_ptr).closed {
                continue;
            }
            if !matches!((*win_ptr).state, crate::window::WindowState::Mapped) {
                continue;
            }
            if let Some(app_id) = (*win_ptr).get_app_id_string() {
                if app_id == "cce-status-interface" {
                    let link = &(*win_ptr).node.link;
                    if !link.prev.is_null() && !link.next.is_null() {
                        status_bar_windows.push(win_ptr);
                    }
                }
            }
        }
        for win_ptr in status_bar_windows {
            let node_link = &mut (*win_ptr).node.link as *mut ffi::wl_list as *mut WlList;
            let list_head = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
            if (*node_link).next != list_head {
                crate::server::wl_list_remove(node_link);
                crate::server::wl_list_insert((*list_head).prev, node_link);
            }
        }
    }

    pub unsafe fn raise_window(&mut self, window: *mut Window) {
        let node_link = &mut (*window).node.link as *mut ffi::wl_list as *mut WlList;
        let list_head = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        if (*node_link).next != list_head {
            crate::server::wl_list_remove(node_link);
            crate::server::wl_list_insert((*list_head).prev, node_link);
        }
        self.keep_status_bar_on_top();
    }

    pub unsafe fn execute_action(&mut self, action: &crate::config::Action, command: Option<&str>) {
        use crate::config::Action;
        match action {
            Action::None => {}
            Action::Spawn => {
                if let Some(cmd) = command {
                    log::info!("executing spawn: {}, WAYLAND_DISPLAY: {:?}", cmd, std::env::var("WAYLAND_DISPLAY"));
                    match nix::unistd::fork() {
                        Ok(nix::unistd::ForkResult::Child) => {
                            crate::process::cleanup_child();
                            let env: Vec<std::ffi::CString> = std::env::vars()
                                .map(|(k, v)| std::ffi::CString::new(format!("{}={}", k, v)).unwrap())
                                .collect();
                            let env_ptrs: Vec<&std::ffi::CStr> = env.iter().map(|s| s.as_c_str()).collect();
                            let sh_c = std::ffi::CString::new("/bin/sh").unwrap();
                            let c_c = std::ffi::CString::new("-c").unwrap();
                            let cmd_c = std::ffi::CString::new(cmd).unwrap();
                            let args = [sh_c.as_c_str(), c_c.as_c_str(), cmd_c.as_c_str()];
                            let _ = nix::unistd::execve(&sh_c, &args, &env_ptrs);
                            std::process::exit(1);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            log::error!("failed to fork child process: {}", e);
                        }
                    }
                }
            }
            Action::Toggle => {
                if let Some(cmd) = command {
                    let prog_name = crate::config::extract_program_name(cmd);
                    let mut matched_win: *mut Window = std::ptr::null_mut();
                    for &w in self.windows.iter() {
                        if !w.is_null() && !(*w).closed {
                            let aid = (*w).get_app_id_string();
                            let title = (*w).get_title_string();

                            let mut match_aid = false;
                            if let Some(ref aid_str) = aid {
                                let aid_lower = aid_str.to_lowercase();
                                let prog_lower = prog_name.to_lowercase();
                                if aid_lower == prog_lower || aid_lower.contains(&prog_lower) || prog_lower.contains(&aid_lower) {
                                    match_aid = true;
                                }
                            } else if let Some(ref title_str) = title {
                                let title_lower = title_str.to_lowercase();
                                let prog_lower = prog_name.to_lowercase();
                                if title_lower.contains(&prog_lower) {
                                    match_aid = true;
                                }
                            }
                            if match_aid {
                                matched_win = w;
                                break;
                            }
                        }
                    }

                    if !matched_win.is_null() {
                        let aid = (*matched_win).get_app_id_string().unwrap_or_default();
                        log::info!("toggle: closing window {:?}", aid);
                        (*matched_win).close();
                        if let Some(seat) = self.first_seat() {
                            if let crate::seat::Focus::Window(fw) = (*seat).focused {
                                if fw == matched_win {
                                    self.focus_next_visible_window(seat);
                                }
                            }
                        }
                        self.dirty_windowing();
                    } else {
                        log::info!("toggle: spawning {}", cmd);
                        self.execute_action(&Action::Spawn, Some(cmd));
                    }
                }
            }
            Action::Close => {
                if let Some(seat) = self.first_seat() {
                    if let crate::seat::Focus::Window(fw) = (*seat).focused {
                        log::info!("close: closing focused window");
                        (*fw).close();
                        self.focus_next_visible_window(seat);
                        self.dirty_windowing();
                    }
                }
            }
            Action::Minimize => {
                if let Some(seat) = self.first_seat() {
                    if let crate::seat::Focus::Window(fw) = (*seat).focused {
                        log::info!("minimize: minimizing focused window");
                        (*fw).minimized = true;
                        self.focus_next_visible_window(seat);
                        self.dirty_windowing();
                    }
                }
            }
            Action::FocusNext | Action::FocusPrev => {
                if let Some(seat) = self.first_seat() {
                    let focused_win = if let crate::seat::Focus::Window(fw) = (*seat).focused {
                        fw
                    } else {
                        std::ptr::null_mut()
                    };

                    let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
                    let mut curr = (*render_list).next;
                    let mut visible_windows = Vec::new();
                    while curr != render_list {
                        let next = (*curr).next;
                        let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
                        if let crate::wm_node::WmNodeType::Window(window) = (*node).get() {
                            if !window.is_null() && !(*window).closed && !(*window).minimized && ((*window).tags & self.active_tags) != 0 {
                                let is_status_bar = (*window).get_app_id_string()
                                    .map_or(false, |aid| aid == "cce-status-interface");
                                if !is_status_bar {
                                    visible_windows.push(window);
                                }
                            }
                        }
                        curr = next;
                    }

                    let n = visible_windows.len();
                    if n > 0 {
                        let current_idx = visible_windows.iter().position(|&w| w == focused_win);
                        let target_idx = match current_idx {
                            Some(idx) => {
                                if *action == Action::FocusNext {
                                    (idx + 1) % n
                                } else {
                                    (idx + n - 1) % n
                                }
                            }
                            None => {
                                if *action == Action::FocusNext {
                                    0
                                } else {
                                    n - 1
                                }
                            }
                        };
                        let target_win = visible_windows[target_idx];
                        (*seat).focus(crate::seat::Focus::Window(target_win));
                        self.raise_window(target_win);
                        self.dirty_windowing();
                    }
                }
            }
            Action::Reload => {
                log::info!("monolithic execute_action: Reload requested");
                if let Some(path) = crate::config::default_config_path() {
                    match crate::config::parse_config(&path, self) {
                        Ok(()) => {
                            self.dirty_windowing();
                            let _ = std::process::Command::new("notify-send")
                                .arg("cce")
                                .arg("Configuration reloaded successfully")
                                .spawn();
                        }
                        Err(e) => {
                            log::error!("failed to reload config: {}", e);
                            let _ = std::process::Command::new("notify-send")
                                .arg("cce")
                                .arg(format!("Failed to reload config:\n{}", e))
                                .spawn();
                        }
                    }
                } else {
                    log::error!("no config file found to reload");
                    let _ = std::process::Command::new("notify-send")
                        .arg("cce")
                        .arg("No config file found to reload")
                        .spawn();
                }
            }
            Action::Exit => {
                log::info!("monolithic execute_action: Exit requested");
                ffi::wl_display_terminate((*self.server).wl_server);
            }
            Action::Fullscreen => {
                if let Some(seat) = self.first_seat() {
                    if let crate::seat::Focus::Window(fw) = (*seat).focused {
                        let is_fullscreen = (*fw).tiling_mode == crate::tiling::TilingMode::Fullscreen;
                        if is_fullscreen {
                            let target_mode = self.get_mode_for_window(fw);
                            (*fw).tiling_mode = if target_mode == crate::tiling::TilingMode::Fullscreen {
                                crate::tiling::TilingMode::Cascade
                            } else {
                                target_mode
                            };
                            (*fw).mode_locked = false;
                        } else {
                            (*fw).tiling_mode = crate::tiling::TilingMode::Fullscreen;
                            (*fw).mode_locked = true;
                        }
                        self.dirty_windowing();
                    }
                }
            }
            Action::LayoutNext => {
                let cycle = [
                    crate::tiling::TilingMode::Cascade,
                    crate::tiling::TilingMode::Grid,
                    crate::tiling::TilingMode::Fullscreen,
                ];
                let active_tags = self.active_tags;
                let first_tag_bit = (0..4).find(|b| (active_tags & (1u32 << b)) != 0);
                let current = if let Some(bit) = first_tag_bit {
                    if self.has_tag_layout[bit] {
                        self.tag_layouts[bit]
                    } else {
                        self.global_layout
                    }
                } else {
                    self.global_layout
                };
                
                let next = cycle
                    .iter()
                    .position(|m| *m == current)
                    .map(|i| cycle[(i + 1) % cycle.len()])
                    .unwrap_or(crate::tiling::TilingMode::Cascade);
                    
                for tag_bit in 0..4 {
                    if (active_tags & (1u32 << tag_bit)) != 0 {
                        self.tag_layouts[tag_bit] = next;
                        self.has_tag_layout[tag_bit] = true;
                    }
                }
                log::info!("layout-next: cycled layout to {:?}", next);
                self.dirty_windowing();
            }
            Action::ModeNext => {
                let cycle = [
                    crate::tiling::TilingMode::Cascade,
                    crate::tiling::TilingMode::Grid,
                    crate::tiling::TilingMode::Fullscreen,
                    crate::tiling::TilingMode::Floating,
                ];
                if let Some(seat) = self.first_seat() {
                    if let crate::seat::Focus::Window(fw) = (*seat).focused {
                        let current_mode = (*fw).tiling_mode;
                        let next = cycle
                            .iter()
                            .position(|m| *m == current_mode)
                            .map(|i| cycle[(i + 1) % cycle.len()])
                            .unwrap_or(crate::tiling::TilingMode::Cascade);
                        (*fw).tiling_mode = next;
                        (*fw).mode_locked = true;
                        self.dirty_windowing();
                    }
                }
            }
            Action::ModeNextShared => {
                let cycle = [
                    crate::tiling::TilingMode::Cascade,
                    crate::tiling::TilingMode::Grid,
                    crate::tiling::TilingMode::Fullscreen,
                    crate::tiling::TilingMode::Floating,
                ];
                if let Some(seat) = self.first_seat() {
                    if let crate::seat::Focus::Window(fw) = (*seat).focused {
                        let current_mode = (*fw).tiling_mode;
                        let next = cycle
                            .iter()
                            .position(|m| *m == current_mode)
                            .map(|i| cycle[(i + 1) % cycle.len()])
                            .unwrap_or(crate::tiling::TilingMode::Cascade);
                        
                        let active_tags = self.active_tags;
                        for &w in self.windows.iter() {
                            if !w.is_null() && !(*w).closed && ((*w).tags & active_tags) != 0 && (*w).tiling_mode == current_mode {
                                (*w).tiling_mode = next;
                                (*w).mode_locked = true;
                            }
                        }
                        self.dirty_windowing();
                    }
                }
            }
             Action::View1 | Action::View2 | Action::View3 | Action::View4 => {
                let tag = match action {
                    Action::View1 => 1,
                    Action::View2 => 2,
                    Action::View3 => 3,
                    Action::View4 => 4,
                    _ => 1,
                };
                self.active_tags = 1 << (tag - 1);
                self.dirty_windowing();
            }
            Action::Toggle1 | Action::Toggle2 | Action::Toggle3 | Action::Toggle4 => {
                let tag = match action {
                    Action::Toggle1 => 1,
                    Action::Toggle2 => 2,
                    Action::Toggle3 => 3,
                    Action::Toggle4 => 4,
                    _ => 1,
                };
                self.active_tags ^= 1 << (tag - 1);
                if self.active_tags == 0 {
                    self.active_tags = 1;
                }
                self.dirty_windowing();
            }
            Action::SetTag1 | Action::SetTag2 | Action::SetTag3 | Action::SetTag4 => {
                let tag = match action {
                    Action::SetTag1 => 1,
                    Action::SetTag2 => 2,
                    Action::SetTag3 => 3,
                    Action::SetTag4 => 4,
                    _ => 1,
                };
                if let Some(seat) = self.first_seat() {
                    if let crate::seat::Focus::Window(fw) = (*seat).focused {
                        (*fw).tags = 1 << (tag - 1);
                        if ((*fw).tags & self.active_tags) == 0 {
                            self.focus_next_visible_window(seat);
                        }
                        self.dirty_windowing();
                    }
                }
            }
            Action::SidePanelLeft => {
                self.layout.side_panel_position = "left".to_string();
                self.dirty_windowing();
            }
            Action::SidePanelRight => {
                self.layout.side_panel_position = "right".to_string();
                self.dirty_windowing();
            }
            Action::Expose => {
                self.expose_active = !self.expose_active;
                log::info!("Expose mode toggled: {}", self.expose_active);
                self.dirty_windowing();
            }
            _ => {}
        }
    }

    pub unsafe fn process_ipc_command(&mut self, cmd: &str) -> String {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return "error: empty command\n".to_string();
        }
        
        let action = parts[0];
        match action {
            "view" => {
                if parts.len() < 2 { return "error: missing tag\n".to_string(); }
                if let Ok(tag) = parts[1].parse::<i32>() {
                    if tag >= 1 && tag <= 4 {
                        self.execute_action(&crate::config::Action::View1, Some(parts[1]));
                        return "ok\n".to_string();
                    }
                }
                "error: invalid tag\n".to_string()
            }
            "toggle" => {
                if parts.len() < 2 { return "error: missing tag\n".to_string(); }
                if let Ok(tag) = parts[1].parse::<i32>() {
                    if tag >= 1 && tag <= 4 {
                        let act = match tag {
                            1 => crate::config::Action::Toggle1,
                            2 => crate::config::Action::Toggle2,
                            3 => crate::config::Action::Toggle3,
                            4 => crate::config::Action::Toggle4,
                            _ => crate::config::Action::None,
                        };
                        self.execute_action(&act, None);
                        return "ok\n".to_string();
                    }
                }
                "error: invalid tag\n".to_string()
            }
            "set-tag" => {
                if parts.len() < 2 { return "error: missing tag\n".to_string(); }
                if let Ok(tag) = parts[1].parse::<i32>() {
                    if tag >= 1 && tag <= 4 {
                        let act = match tag {
                            1 => crate::config::Action::SetTag1,
                            2 => crate::config::Action::SetTag2,
                            3 => crate::config::Action::SetTag3,
                            4 => crate::config::Action::SetTag4,
                            _ => crate::config::Action::None,
                        };
                        self.execute_action(&act, None);
                        return "ok\n".to_string();
                    }
                }
                "error: invalid tag\n".to_string()
            }
            "close" => {
                self.execute_action(&crate::config::Action::Close, None);
                "ok\n".to_string()
            }
            "expose" => {
                self.execute_action(&crate::config::Action::Expose, None);
                "ok\n".to_string()
            }
            "minimize" => {
                self.execute_action(&crate::config::Action::Minimize, None);
                "ok\n".to_string()
            }
            "focus-next" => {
                self.execute_action(&crate::config::Action::FocusNext, None);
                "ok\n".to_string()
            }
            "focus-window" => {
                if parts.len() < 2 { return "error: missing app_id or title\n".to_string(); }
                let query = parts[1..].join(" ").to_lowercase();
                if let Some(seat) = self.first_seat() {
                    let mut target: *mut Window = std::ptr::null_mut();
                    for &w in self.windows.iter() {
                        if !w.is_null() && !(*w).closed && !(*w).minimized && ((*w).tags & self.active_tags) != 0 {
                            let aid = (*w).get_app_id_string();
                            let title = (*w).get_title_string();

                            let mut match_found = false;
                            if let Some(ref aid_str) = aid {
                                if aid_str.to_lowercase().contains(&query) { match_found = true; }
                            }
                            if let Some(ref title_str) = title {
                                if title_str.to_lowercase().contains(&query) { match_found = true; }
                            }
                            if match_found {
                                target = w;
                                break;
                            }
                        }
                    }
                    if !target.is_null() {
                        (*seat).focus(crate::seat::Focus::Window(target));
                        self.raise_window(target);
                        self.dirty_windowing();
                        "ok\n".to_string()
                    } else {
                        "error: window not found\n".to_string()
                    }
                } else {
                    "error: no seat found\n".to_string()
                }
            }
            "exit" => {
                self.execute_action(&crate::config::Action::Exit, None);
                "ok\n".to_string()
            }
            "reload" => {
                if let Some(path) = crate::config::default_config_path() {
                    match crate::config::parse_config(&path, self) {
                        Ok(()) => {
                            self.dirty_windowing();
                            "ok\n".to_string()
                        }
                        Err(e) => format!("error: failed to reload config: {}\n", e),
                    }
                } else {
                    "error: no config file found\n".to_string()
                }
            }
            "retile" => {
                self.dirty_windowing();
                "ok\n".to_string()
            }
            "spawn" => {
                if parts.len() < 2 { return "error: missing command\n".to_string(); }
                let cmd = parts[1..].join(" ");
                self.execute_action(&crate::config::Action::Spawn, Some(&cmd));
                "ok\n".to_string()
            }
            "layout" => {
                if parts.len() < 3 { return "error: missing layout key or value\n".to_string(); }
                let key = parts[1];
                let val = parts[2];
                match key {
                    "gap" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap = v; } }
                    "gap_top" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_top = v; } }
                    "gap_left" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_left = v; } }
                    "gap_right" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_right = v; } }
                    "gap_bottom" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_bottom = v; } }
                    "offset" | "cascade_offset" => { if let Ok(v) = val.parse::<i32>() { self.layout.cascade_offset = v; } }
                    "grid_gap" => { if let Ok(v) = val.parse::<i32>() { self.layout.grid_gap = v; } }
                    "bar_height" => { if let Ok(v) = val.parse::<i32>() { self.layout.bar_height = v; } }
                    "border_width" => { if let Ok(v) = val.parse::<i32>() { self.layout.border_width = v; } }
                    "fullscreen_border_width" => { if let Ok(v) = val.parse::<i32>() { self.layout.fullscreen_border_width = v; } }
                    "cascade_border_width" => { if let Ok(v) = val.parse::<i32>() { self.layout.cascade_border_width = v; } }
                    "grid_border_width" => { if let Ok(v) = val.parse::<i32>() { self.layout.grid_border_width = v; } }
                    "floating_border_width" => { if let Ok(v) = val.parse::<i32>() { self.layout.floating_border_width = v; } }
                    "transition_duration" => { if let Ok(v) = val.parse::<i32>() { self.layout.transition_duration = v; } }
                    "border_color" => {
                        let border_color_val = crate::config::parse_hex_color(val);
                        self.layout.border_r = ((border_color_val >> 16) & 0xFF) * 0x01010101;
                        self.layout.border_g = ((border_color_val >> 8) & 0xFF) * 0x01010101;
                        self.layout.border_b = (border_color_val & 0xFF) * 0x01010101;
                    }
                    "side_panel_width" => { if let Ok(v) = val.parse::<i32>() { self.layout.side_panel_width = v; } }
                    "side_panel_behavior" => { self.layout.side_panel_behavior = val.to_string(); }
                    "side_panel_position" => { self.layout.side_panel_position = val.to_string(); }
                    "side_panel_border_gap" => { if let Ok(v) = val.parse::<i32>() { self.layout.side_panel_border_gap = v; } }
                    "side_panel_border_opacity" => { if let Ok(v) = val.parse::<i32>() { self.layout.side_panel_border_opacity = v; } }
                    _ => return format!("error: unknown layout key: {}\n", key),
                }
                self.dirty_windowing();
                "ok\n".to_string()
            }
            "tag-layout" => {
                if parts.len() < 3 { return "error: missing tag or layout mode\n".to_string(); }
                if let Ok(tag) = parts[1].parse::<usize>() {
                    if tag >= 1 && tag <= 4 {
                        let mode = crate::config::parse_tiling_mode(parts[2]);
                        self.tag_layouts[tag - 1] = mode;
                        self.has_tag_layout[tag - 1] = true;
                        self.dirty_windowing();
                        return "ok\n".to_string();
                    }
                }
                "error: invalid tag\n".to_string()
            }
            "mode" => {
                if parts.len() < 3 { return "error: missing mode or app_id\n".to_string(); }
                let mode = crate::config::parse_tiling_mode(parts[1]);
                let app_id = parts[2].to_string();
                let title = if parts.len() >= 4 { Some(parts[3..].join(" ")) } else { None };
                self.mode_rules.push(crate::config::ModeRule {
                    mode,
                    app_id_pattern: app_id,
                    title_pattern: title,
                    single_instance: false,
                    tag: -1,
                    circular: false,
                    ssd: None,
                });
                self.dirty_windowing();
                "ok\n".to_string()
            }
            "input" => {
                if parts.len() < 4 { return "error: usage: input <device_name|*> scroll-factor <value>\n".to_string(); }
                let device_name = parts[1];
                let key = parts[2];
                let val = parts[3];
                if key == "scroll-factor" {
                    if let Ok(factor) = val.parse::<f64>() {
                        if factor < 0.0 {
                            return "error: scroll factor cannot be negative\n".to_string();
                        }
                        let mut found = false;
                        let devices_head = &mut (*self.server).input_manager.devices as *mut ffi::wl_list as *mut WlList;
                        let mut curr = (*devices_head).next;
                        while curr != devices_head {
                            let next = (*curr).next;
                            let device = crate::container_of!(curr, crate::input_device::InputDevice, link);
                            let name_ptr = (*(*device).wlr_device).name;
                            if !name_ptr.is_null() {
                                let name = std::ffi::CStr::from_ptr(name_ptr).to_string_lossy();
                                if device_name == "*" || name.contains(device_name) {
                                    (*device).config.scroll_factor = factor;
                                    found = true;
                                }
                            }
                            curr = next;
                        }
                        if found {
                            "ok\n".to_string()
                        } else {
                            "error: no matching device found\n".to_string()
                        }
                    } else {
                        "error: invalid scroll-factor value\n".to_string()
                    }
                } else {
                    format!("error: unknown input command: {}\n", key)
                }
            }
            _ => format!("error: unknown command: {}\n", action),
        }
    }

    pub unsafe fn apply_input_rules(&mut self) {
        if self.server.is_null() {
            return;
        }
        let devices_head = &mut (*self.server).input_manager.devices as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*devices_head).next;
        while curr != devices_head {
            let next = (*curr).next;
            let device = crate::container_of!(curr, crate::input_device::InputDevice, link);
            let name_ptr = (*(*device).wlr_device).name;
            if !name_ptr.is_null() {
                let name = std::ffi::CStr::from_ptr(name_ptr).to_string_lossy();
                for rule in &self.input_rules {
                    if rule.name == "*" || name.contains(&rule.name) {
                        if let Some(factor) = rule.scroll_factor {
                            (*device).config.scroll_factor = factor;
                        }
                    }
                }
            }
            curr = next;
        }
    }

    pub unsafe fn apply_input_config(&mut self) {
        if self.server.is_null() {
            return;
        }
        let devices_head = &mut (*self.server).input_manager.devices as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*devices_head).next;
        while curr != devices_head {
            let next = (*curr).next;
            let device = crate::container_of!(curr, crate::input_device::InputDevice, link);
            if let Some(ref mut libinput) = (*device).libinput {
                libinput.apply_config(&self.input_config);
            }
            curr = next;
        }
    }
}

unsafe extern "C" fn handle_ipc_timer(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    
    if let Some(ref rx) = (*wm).ipc_rx {
        while let Ok(req) = rx.try_recv() {
            let reply = (*wm).process_ipc_command(&req.command);
            let _ = req.reply_tx.send(reply);
        }
    }
    
    if !(*wm).ipc_timer.is_null() {
        ffi::wl_event_source_timer_update((*wm).ipc_timer, 10);
    }
    
    0
}

fn blend_channel(bg_channel: u32, fg_channel: u32, factor: f64) -> u32 {
    let bg = (bg_channel & 0xFF) as u8 as f64;
    let fg = (fg_channel & 0xFF) as u8 as f64;
    let val = (bg + (fg - bg) * factor) as u8;
    val as u32 * 0x01010101
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
    
    if matches!((*wm).state, WindowManagerState::Idle) {
        if (*wm).scheduled.dirty || (*wm).scheduled.dirty_lazy {
            (*wm).scheduled.dirty = true;
            (*wm).scheduled.dirty_lazy = false;
            (*wm).manage_start();
        } else if (*wm).rendering_scheduled.dirty {
            (*wm).render_start();
        }
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

    let mut pid = 0;
    let mut uid = 0;
    let mut gid = 0;
    ffi::wl_client_get_credentials(client, &mut pid, &mut uid, &mut gid);
    let cmdline = std::fs::read_to_string(format!("/proc/{}/cmdline", pid))
        .unwrap_or_default()
        .replace('\0', " ");
    log::info!("Client binding river_window_manager_v1: PID={}, cmdline='{}'", pid, cmdline);

    let resource = ffi::wl_resource_create(client, &ffi::river_window_manager_v1_interface, version as i32, id);
    if resource.is_null() {
        ffi::wl_client_post_no_memory(client);
        log::error!("out of memory binding river_window_manager_v1");
        return;
    }

    if !(*wm).object.is_null() {
        log::warn!("river_window_manager_v1 already bound, rejecting new client PID={}", pid);
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
