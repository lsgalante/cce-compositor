// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, WlList, wl_listener_remove};
use crate::slotmap::SlotMap;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowManagerMode {
    Normal,
    Overview,
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

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SavedWindowState {
    pub app_id: String,
    pub title: String,
    pub tiling_mode: crate::tiling::TilingMode,
    pub minimized: bool,
    pub virtual_x: f64,
    pub virtual_y: f64,
    pub scale: f64,
    pub width: u32,
    pub height: u32,
    pub cmdline: String,
    #[serde(default)]
    pub focused: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SavedState {
    pub desk_pan_x: f64,
    pub desk_pan_y: f64,
    pub desk_zoom: f64,
    pub global_layout: crate::tiling::TilingMode,
    pub windows: Vec<SavedWindowState>,
}

pub struct WindowManager {
    pub server: *mut Server,
    pub global: *mut ffi::wl_global,
    pub server_destroy: ffi::wl_listener,
    pub object: *mut ffi::wl_resource,
    pub state: WindowManagerState,
    pub windows: SlotMap<*mut Window>,
    pub focus_history: Vec<*mut Window>,
    pub scheduled: WindowManagerScheduled,
    pub sent: WindowManagerSent,
    pub rendering_scheduled: WindowManagerRenderingScheduled,
    pub rendering_requested: WindowManagerRenderingRequested,
    pub dirty_idle: *mut ffi::wl_event_source,
    pub timeout: *mut ffi::wl_event_source,
    pub desk_pan_x: f64,
    pub desk_pan_y: f64,
    pub desk_zoom: f64,
    pub mode: WindowManagerMode,
    pub global_layout: crate::tiling::TilingMode,
    pub layout: crate::config::Layout,
    pub mode_rules: Vec<crate::config::ModeRule>,
    pub keybinds: Vec<crate::config::Keybind>,
    pub pointer_binds: Vec<crate::config::PointerBind>,
    pub gesture_binds: Vec<crate::config::GestureBind>,
    pub ipc_rx: Option<std::sync::mpsc::Receiver<crate::ipc_server::IpcRequest>>,
    pub ipc_timer: *mut ffi::wl_event_source,
    pub startup: Vec<crate::config::StartupConfig>,
    pub startup_pids: Vec<(crate::config::StartupConfig, nix::unistd::Pid)>,
    pub status_sender: Option<crate::status_server::StatusSender>,
    pub output_scale: f32,
    pub display: std::collections::HashMap<String, f64>,
    pub input_rules: Vec<crate::config::InputDeviceConfigRule>,
    pub input_config: crate::config::InputConfig,
    pub last_status_update: std::cell::RefCell<Option<crate::status_server::StatusUpdate>>,
    pub restore_queue: Vec<SavedWindowState>,
    pub shutting_down: bool,
    pub target_desk_pan_x: Option<f64>,
    pub target_desk_pan_y: Option<f64>,
    pub animation_timer: *mut ffi::wl_event_source,
    pub has_restored_focused_window: bool,
    pub restored_focused_window_mapped: bool,
}

impl WindowManager {
    pub unsafe fn init(&mut self) -> Result<(), ()> {
        // This is a stub for the 0-arg struct instantiation.
        // We will call the real initialization with the server parameter.
        ffi::wl_list_init(&mut self.sent.outputs);
        self.scheduled.output_config = std::ptr::null_mut();
        self.sent.output_config = std::ptr::null_mut();
        self.output_scale = 1.0;
        self.display = std::collections::HashMap::new();
        self.input_rules = Vec::new();
        self.input_config = crate::config::InputConfig::default();
        self.mode = WindowManagerMode::Normal;
        Ok(())
    }

    pub unsafe fn init_with_server(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        self.global = std::ptr::null_mut();
        self.object = std::ptr::null_mut();
        self.state = WindowManagerState::Idle;
        self.windows = SlotMap::new();
        self.focus_history = Vec::new();
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
        self.desk_pan_x = 0.0;
        self.desk_pan_y = 0.0;
        self.target_desk_pan_x = None;
        self.target_desk_pan_y = None;
        self.animation_timer = std::ptr::null_mut();
        self.desk_zoom = 1.0;
        self.mode = WindowManagerMode::Normal;
        self.global_layout = crate::tiling::TilingMode::Cascade;
        self.restore_queue = Vec::new();
        self.shutting_down = false;
        self.layout = crate::config::Layout::default();
        self.output_scale = 1.0;
        self.display = std::collections::HashMap::new();
        self.has_restored_focused_window = false;
        self.restored_focused_window_mapped = false;
        self.mode_rules = Vec::new();
        self.keybinds = Vec::new();
        self.pointer_binds = Vec::new();
        self.gesture_binds = Vec::new();
        self.ipc_rx = None;
        self.ipc_timer = std::ptr::null_mut();
        self.startup = Vec::new();
        self.startup_pids = Vec::new();
        self.status_sender = None;
        self.input_rules = Vec::new();
        self.input_config = crate::config::InputConfig::default();
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
            &ffi::zcce_window_manager_v1_interface,
            4,
            self as *mut WindowManager as *mut _,
            Some(bind),
        );
        if self.global.is_null() {
            ffi::wl_event_source_remove(self.timeout);
            return Err("Failed to create zcce_window_manager_v1 global");
        }

        let server_destroy_ptr = &mut self.server_destroy as *mut ffi::wl_listener as *mut WlListener;
        (*server_destroy_ptr).notify = Some(handle_server_destroy);
        ffi::wl_display_add_destroy_listener((*server).wl_server, &mut self.server_destroy);

        Ok(())
    }

    pub unsafe fn load_state(&mut self, path: &str) {
        log::info!("Loading state from {}", path);
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(state) = serde_json::from_str::<SavedState>(&content) {
                self.desk_pan_x = state.desk_pan_x;
                self.desk_pan_y = state.desk_pan_y;
                self.desk_zoom = state.desk_zoom;
                self.mode = if (state.desk_zoom - 1.0).abs() > 0.001 { WindowManagerMode::Overview } else { WindowManagerMode::Normal };
                self.global_layout = state.global_layout;
                self.restore_queue = state.windows;
                self.has_restored_focused_window = self.restore_queue.iter().any(|w| w.focused);
                self.restored_focused_window_mapped = false;
                log::info!(
                    "State loaded successfully. {} windows in restore queue, has_restored_focused_window={}.",
                    self.restore_queue.len(),
                    self.has_restored_focused_window
                );
            } else {
                log::error!("Failed to parse state JSON from {}", path);
            }
        } else {
            log::info!("State file not found or unreadable at {}, starting with empty state.", path);
        }
    }

    pub unsafe fn save_state(&self) {
        if self.shutting_down {
            return;
        }
        let Some(path_str) = crate::config::default_state_path() else {
            log::error!("Could not resolve state file path");
            return;
        };
        log::debug!("Saving state to {}", path_str);
        
        let focused_win = self.focused_window();
        let mut saved_wins = Vec::new();
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                continue;
            }
            if (*w).is_status_bar() {
                continue;
            }
            
            let app_id = (*w).get_app_id_string().unwrap_or_default();
            let title = (*w).get_title_string().unwrap_or_default();
            
            let pid = (*w).unreliable_pid();
            let mut cmdline = if pid > 0 {
                let proc_cmdline = std::fs::read(format!("/proc/{}/cmdline", pid)).unwrap_or_default();
                if !proc_cmdline.is_empty() {
                    let mut args: Vec<String> = proc_cmdline
                        .split(|&b| b == 0)
                        .map(|arg| String::from_utf8_lossy(arg).into_owned())
                        .collect();
                    if args.last().map_or(false, |s| s.is_empty()) {
                        args.pop();
                    }
                    if !args.is_empty() && args[0].starts_with("/tmp/.mount_") {
                        if let Ok(environ_bytes) = std::fs::read(format!("/proc/{}/environ", pid)) {
                            let appimage_opt = environ_bytes
                                .split(|&b| b == 0)
                                .find(|env_var| env_var.starts_with(b"APPIMAGE="))
                                .map(|env_var| {
                                    let val_bytes = &env_var[b"APPIMAGE=".len()..];
                                    String::from_utf8_lossy(val_bytes).into_owned()
                                });
                            if let Some(appimage_path) = appimage_opt {
                                args[0] = appimage_path;
                            }
                        }
                    }
                    args.join(" ")
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            if cmdline.is_empty() {
                cmdline = app_id.clone();
            }

            let is_focused = w == focused_win;

            saved_wins.push(SavedWindowState {
                app_id,
                title,
                tiling_mode: (*w).tiling_mode,
                minimized: (*w).minimized,
                virtual_x: (*w).virtual_x,
                virtual_y: (*w).virtual_y,
                scale: (*w).scale,
                width: (*w).box_geom.width as u32,
                height: (*w).box_geom.height as u32,
                cmdline,
                focused: is_focused,
            });
        }
        
        let state = SavedState {
            desk_pan_x: self.desk_pan_x,
            desk_pan_y: self.desk_pan_y,
            desk_zoom: self.desk_zoom,
            global_layout: self.global_layout,
            windows: saved_wins,
        };
        
        if let Ok(json_str) = serde_json::to_string_pretty(&state) {
            let path = std::path::Path::new(&path_str);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(path, json_str) {
                log::error!("Failed to write state file: {}", e);
            }
        }
    }

    pub unsafe fn match_and_remove_restore_state(&mut self, app_id: &str, title: &str) -> Option<SavedWindowState> {
        if app_id.is_empty() {
            return None;
        }
        // First pass: Exact match (app_id AND title)
        if let Some(pos) = self.restore_queue.iter().position(|w| w.app_id == app_id && w.title == title) {
            return Some(self.restore_queue.remove(pos));
        }
        // Second pass: Fuzzy title match (e.g. prefix match, asterisk stripping)
        if let Some(pos) = self.restore_queue.iter().position(|w| {
            if w.app_id != app_id {
                return false;
            }
            let t1 = title.trim_end_matches('*');
            let t2 = w.title.trim_end_matches('*');
            t1 == t2 || t1.starts_with(t2) || t2.starts_with(t1)
        }) {
            return Some(self.restore_queue.remove(pos));
        }
        // Third pass: app_id only match
        if let Some(pos) = self.restore_queue.iter().position(|w| w.app_id == app_id) {
            return Some(self.restore_queue.remove(pos));
        }
        None
    }

    pub unsafe fn spawn_restored_windows(&mut self) {
        log::info!("Spawning restored windows. Total: {}", self.restore_queue.len());
        let restored = self.restore_queue.clone();
        std::thread::spawn(move || {
            for (i, w) in restored.into_iter().enumerate() {
                if !w.cmdline.is_empty() {
                    let delay = 1000 + i as u64 * 500;
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                    log::info!("Deferred spawning restored window command: {}", w.cmdline);
                    let cmd = w.cmdline;
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
        });
    }

    pub fn start_ipc(&mut self, display_socket: Option<String>) {
        if self.ipc_rx.is_none() {
            let rx = crate::ipc_server::spawn_ipc_server(display_socket);
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
        if !self.animation_timer.is_null() {
            ffi::wl_event_source_remove(self.animation_timer);
            self.animation_timer = std::ptr::null_mut();
        }
        wl_listener_remove(&mut self.server_destroy);
    }

    pub unsafe fn stop_panning_animation(&mut self) {
        self.target_desk_pan_x = None;
        self.target_desk_pan_y = None;
    }

    pub unsafe fn ensure_windowing(&self) -> bool {
        match self.state {
            WindowManagerState::Manage => true,
            _ => {
                if !self.object.is_null() {
                    ffi::wl_resource_post_error(
                        self.object,
                        ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
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
                        ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
                        b"invalid modification of rendering state\0".as_ptr() as *const _,
                    );
                }
                false
            }
        }
    }

    pub unsafe fn dirty_windowing(&mut self) {
        if log::log_enabled!(log::Level::Debug) {
            let bt = std::backtrace::Backtrace::force_capture();
            log::debug!("dirty_windowing called from backtrace:\n{}", bt);
        }
        self.scheduled.dirty = true;
        self.add_dirty_idle();
    }
 
    pub unsafe fn dirty_windowing_lazy(&mut self) {
        self.scheduled.dirty_lazy = true;
        self.add_dirty_idle();
    }
 
    pub unsafe fn clean_windowing(&mut self) {
        self.scheduled.dirty = false;
        self.scheduled.dirty_lazy = false;
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
        if !self.scheduled.dirty && !self.scheduled.dirty_lazy && !self.rendering_scheduled.dirty {
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
                    ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_MANAGER_V1_SESSION_LOCKED);
                } else {
                    ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_MANAGER_V1_SESSION_UNLOCKED);
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
            ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_MANAGER_V1_MANAGE_START);
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
                        if !(*window).wm_requested.resizing {
                            if let WindowManagerState::InflightConfigures(ref mut count) = self.state {
                                *count += 1;
                            }
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
            ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_MANAGER_V1_RENDER_START);
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
        self.layout.overlay_behavior.hash(&mut hasher);
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
                    (*window).tiling_mode.hash(&mut hasher);
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
                            } else if (*window).tiling_mode == crate::tiling::TilingMode::Popup {
                                ffi::wlr_scene_node_reparent((*window).tree as *mut _, (*self.server).scene.layers.popups);
                                ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                            } else if (*window).rendering_requested.circular {
                                ffi::wlr_scene_node_reparent((*window).tree as *mut _, (*self.server).scene.layers.top);
                                ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                            } else if (*window).tiling_mode == crate::tiling::TilingMode::Overlay && self.layout.overlay_behavior == "above" {
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
        self.save_state();
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
        if (*win).is_status_bar() {
            return crate::tiling::TilingMode::Status;
        }
        let app_id = (*win).get_app_id_string();
        if app_id.as_deref() == Some("cce-notifier") || app_id.as_deref() == Some("cce-notification-daemon") || app_id.as_deref() == Some("clear-notification-daemon") {
            return crate::tiling::TilingMode::Popup;
        }
        if app_id.as_deref() == Some("cce-cloud") {
            return crate::tiling::TilingMode::Overlay;
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

        crate::tiling::TilingMode::Floating
    }

    pub unsafe fn get_active_resize_dimensions(&self, win_ptr: *mut Window) -> Option<(u32, u32)> {
        let seats_list = &(*self.server).input_manager.seats as *const ffi::wl_list as *const WlList as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let Some(ref op) = (*seat).op {
                if op.window_ptr == win_ptr {
                    if let crate::seat::PointerOpType::Resize { edges } = op.op_type {
                        let scale = self.desk_zoom;
                        let dx = op.x - op.start_x;
                        let dy = op.y - op.start_y;
                        let virtual_dx = dx as f64 / scale;
                        let virtual_dy = dy as f64 / scale;
                        let mut new_w = op.start_win_w;
                        let mut new_h = op.start_win_h;

                        if edges.left {
                            new_w = std::cmp::max(50, (op.start_win_w as f64 - virtual_dx) as i32) as u32;
                        } else if edges.right {
                            new_w = std::cmp::max(50, (op.start_win_w as f64 + virtual_dx) as i32) as u32;
                        }

                        if edges.top {
                            new_h = std::cmp::max(50, (op.start_win_h as f64 - virtual_dy) as i32) as u32;
                        } else if edges.bottom {
                            new_h = std::cmp::max(50, (op.start_win_h as f64 + virtual_dy) as i32) as u32;
                        }
                        return Some((new_w, new_h));
                    }
                }
            }
            curr_seat = (*curr_seat).next;
        }
        None
    }

    pub unsafe fn is_window_being_moved(&self, win_ptr: *mut Window) -> bool {
        let seats_list = &(*self.server).input_manager.seats as *const ffi::wl_list as *const WlList as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let Some(ref op) = (*seat).op {
                if op.window_ptr == win_ptr {
                    if let crate::seat::PointerOpType::Move = op.op_type {
                        return true;
                    }
                }
            }
            curr_seat = (*curr_seat).next;
        }
        false
    }


fn get_closest_tag(x: f64, y: f64) -> i32 {
    let centers = [(0.0, 0.0), (2000.0, 0.0), (0.0, 2000.0), (2000.0, 2000.0)];
    let mut min_dist = f64::MAX;
    let mut best_tag = 1;
    for (i, &(cx, cy)) in centers.iter().enumerate() {
        let dx = x - cx;
        let dy = y - cy;
        let dist = dx * dx + dy * dy;
        if dist < min_dist {
            min_dist = dist;
            best_tag = (i + 1) as i32;
        }
    }
    best_tag
}

    pub unsafe fn arrange_views(&mut self) {
        log::debug!("Monolithic arrange_views triggered. Windows: {}", self.windows.count());
        if log::log_enabled!(log::Level::Debug) {
            for (idx, &win_ptr) in self.windows.iter().enumerate() {
                if win_ptr.is_null() { continue; }
                let title = (*win_ptr).get_title_string().unwrap_or_else(|| "None".to_string());
                let aid = (*win_ptr).get_app_id_string().unwrap_or_else(|| "None".to_string());
                log::debug!("  window #{}: title={:?}, app_id={:?}, state={:?}, closed={}", idx, title, aid, (*win_ptr).state, (*win_ptr).closed);
            }
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

            let viewport_w = wlr_box.width as f64;
            let viewport_h = wlr_box.height as f64;
            let camera_center_x = self.desk_pan_x + (viewport_w / 2.0) / self.desk_zoom;
            let camera_center_y = self.desk_pan_y + (viewport_h / 2.0) / self.desk_zoom;
            let _active_tag = Self::get_closest_tag(camera_center_x, camera_center_y);

            let mut overlay_windows: Vec<*mut Window> = Vec::new();
            let mut normal_windows: Vec<*mut Window> = Vec::new();

            for &win_ptr in self.windows.iter() {
                if (*win_ptr).closed {
                    continue;
                }

                let app_id = (*win_ptr).get_app_id_string();
                let is_status_bar = app_id.as_deref().map_or(false, |id| id.starts_with("cce-status"));
                
                if is_status_bar {
                    (*win_ptr).tiling_mode = crate::tiling::TilingMode::Status;
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
                    (*win_ptr).scale = 1.0;
                    ffi::wlr_scene_node_set_enabled((*win_ptr).tree as *mut ffi::wlr_scene_node, true);
                    (*win_ptr).rendering_requested.hidden = false;
                    (*win_ptr).rendering_requested.blur = self.layout.window_blur;
                    continue;
                }

                let visible = !(*win_ptr).minimized
                    && !matches!((*win_ptr).state, crate::window::WindowState::Closing | crate::window::WindowState::Init);
                if !visible {
                    ffi::wlr_scene_node_set_enabled((*win_ptr).tree as *mut ffi::wlr_scene_node, false);
                    (*win_ptr).rendering_requested.hidden = true;
                    continue;
                }

                ffi::wlr_scene_node_set_enabled((*win_ptr).tree as *mut ffi::wlr_scene_node, true);
                (*win_ptr).rendering_requested.hidden = false;

                let mode = self.get_mode_for_window(win_ptr);
                (*win_ptr).tiling_mode = mode;

                if !(*win_ptr).mode_locked {
                    if let Some(rule) = self.get_rule_for_window(win_ptr) {
                        if let Some(rule_ssd) = rule.ssd {
                            (*win_ptr).wm_requested.ssd = rule_ssd;
                        }
                    }
                }

                let is_moving = self.is_window_being_moved(win_ptr);
                if mode == crate::tiling::TilingMode::Overlay && !is_moving {
                    overlay_windows.push(win_ptr);
                } else {
                    normal_windows.push(win_ptr);
                }
            }

            let bw = 0;

            let g = self.layout.overlay_border_gap;
            let dec_h = std::cmp::max(bw, 16);
            for (sp_idx, &win_ptr) in overlay_windows.iter().enumerate() {
                if sp_idx == 0 {
                    let mut sp_x = (*win_ptr).box_geom.x;
                    let mut sp_y = (*win_ptr).box_geom.y;
                    let mut sp_w = (*win_ptr).box_geom.width as i32;
                    let mut sp_h = (*win_ptr).box_geom.height as i32;

                    if sp_w == 0 || sp_h == 0 {
                        sp_w = if (*win_ptr).wm_scheduled.dimensions_hint.min_width > 32 {
                            std::cmp::max(self.layout.overlay_width, (*win_ptr).wm_scheduled.dimensions_hint.min_width as i32)
                        } else {
                            self.layout.overlay_width
                        };
                        sp_h = (usable_h - (dec_h + bw) - 2 * g).max(1);

                        sp_x = if self.layout.overlay_position == "right" {
                            usable_x + usable_w - sp_w - g + bw
                        } else {
                            usable_x + g + bw
                        };
                        sp_y = usable_y + dec_h + g;

                        (*win_ptr).box_geom.x = sp_x;
                        (*win_ptr).box_geom.y = sp_y;
                        (*win_ptr).box_geom.width = sp_w;
                        (*win_ptr).box_geom.height = sp_h;
                    }

                    (*win_ptr).rendering_requested.x = sp_x;
                    (*win_ptr).rendering_requested.y = sp_y;
                    (*win_ptr).scale = 1.0;

                    let vx = self.desk_pan_x + (sp_x - phys_x) as f64 / self.desk_zoom;
                    let vy = self.desk_pan_y + (sp_y - phys_y) as f64 / self.desk_zoom;
                    (*win_ptr).virtual_x = vx;
                    (*win_ptr).virtual_y = vy;
                    
                    let mut sp_target_w = sp_w;
                    let mut sp_target_h = sp_h;
                    if !(*win_ptr).wm_requested.ssd {
                        let (dec_w, dec_h) = (*win_ptr).get_decorations_size();
                        sp_target_w = (sp_w - dec_w).max(1);
                        sp_target_h = (sp_h - dec_h).max(1);
                    }
                    (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                        width: sp_target_w as u32,
                        height: sp_target_h as u32,
                    });
                    (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                        width: sp_target_w as u32,
                        height: sp_target_h as u32,
                    };
                    (*win_ptr).wm_requested.tiled = 1 | 2 | 4 | 8;

                    let is_focused = win_ptr == focused_window;
                    let r = self.layout.border_r;
                    let g_color = self.layout.border_g;
                    let b = self.layout.border_b;
                    let a = self.layout.border_a;

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
                        if !self.layout.window_opacity { 1.0f32 } else { 0.85f32 }
                    };
                } else {
                    normal_windows.push(win_ptr);
                }
            }

            // Manage entering/exiting Maximized state for normal windows
            for &win_ptr in &normal_windows {
                let mode = (*win_ptr).tiling_mode;
                if mode == crate::tiling::TilingMode::Maximized && !(*win_ptr).was_maximized {
                    // Entering Maximized mode
                    let mut w = (*win_ptr).box_geom.width;
                    let mut h = (*win_ptr).box_geom.height;
                    if w <= 0 {
                        w = if (*win_ptr).wm_scheduled.dimensions_hint.min_width > 32 {
                            (*win_ptr).wm_scheduled.dimensions_hint.min_width as i32
                        } else {
                            800
                        };
                    }
                    if h <= 0 {
                        h = if (*win_ptr).wm_scheduled.dimensions_hint.min_height > 32 {
                            (*win_ptr).wm_scheduled.dimensions_hint.min_height as i32
                        } else {
                            600
                        };
                    }
                    (*win_ptr).saved_maximized_width = w;
                    (*win_ptr).saved_maximized_height = h;
                    (*win_ptr).saved_maximized_virtual_x = (*win_ptr).virtual_x;
                    (*win_ptr).saved_maximized_virtual_y = (*win_ptr).virtual_y;
                    (*win_ptr).was_maximized = true;
                    log::info!("[Maximized] Saved window {:?} geometry: {}x{} at ({}, {})", 
                        (*win_ptr).get_title_string().as_deref().unwrap_or(""), 
                        (*win_ptr).saved_maximized_width, (*win_ptr).saved_maximized_height, 
                        (*win_ptr).saved_maximized_virtual_x, (*win_ptr).saved_maximized_virtual_y
                    );
                } else if mode != crate::tiling::TilingMode::Maximized && (*win_ptr).was_maximized {
                    // Exiting Maximized mode
                    if (*win_ptr).saved_maximized_width > 0 && (*win_ptr).saved_maximized_height > 0 {
                        (*win_ptr).box_geom.width = (*win_ptr).saved_maximized_width;
                        (*win_ptr).box_geom.height = (*win_ptr).saved_maximized_height;
                        (*win_ptr).virtual_x = (*win_ptr).saved_maximized_virtual_x;
                        (*win_ptr).virtual_y = (*win_ptr).saved_maximized_virtual_y;
                        (*win_ptr).was_maximized = false;
                        
                        (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                            width: (*win_ptr).saved_maximized_width as u32,
                            height: (*win_ptr).saved_maximized_height as u32,
                        });
                        (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                            width: (*win_ptr).saved_maximized_width as u32,
                            height: (*win_ptr).saved_maximized_height as u32,
                        };
                        log::info!("[Maximized] Restored window {:?} geometry: {}x{} at ({}, {})", 
                            (*win_ptr).get_title_string().as_deref().unwrap_or(""), 
                            (*win_ptr).saved_maximized_width, (*win_ptr).saved_maximized_height, 
                            (*win_ptr).saved_maximized_virtual_x, (*win_ptr).saved_maximized_virtual_y
                        );
                    }
                }
            }

            // Arrange normal windows on the virtual surface
            for &win_ptr in &normal_windows {
                let mode = (*win_ptr).tiling_mode;
                let is_focused = win_ptr == focused_window;

                if mode == crate::tiling::TilingMode::Popup {
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
                    let fy = usable_y + self.layout.gap_top;

                    (*win_ptr).rendering_requested.x = fx;
                    (*win_ptr).rendering_requested.y = fy;
                    (*win_ptr).scale = 1.0;
                    (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                        width: fw as u32,
                        height: fh as u32,
                    });
                    (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                        width: fw as u32,
                        height: fh as u32,
                    };
                } else if mode == crate::tiling::TilingMode::Fullscreen {
                    (*win_ptr).rendering_requested.x = phys_x;
                    (*win_ptr).rendering_requested.y = phys_y;
                    (*win_ptr).scale = 1.0;
                    (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                        width: phys_w as u32,
                        height: phys_h as u32,
                    });
                    (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                        width: phys_w as u32,
                        height: phys_h as u32,
                    };
                    (*win_ptr).wm_requested.tiled = 1 | 2 | 4 | 8;
                } else if mode == crate::tiling::TilingMode::Maximized {
                    // Maximized mode: Resizes to fully fill all cells of the desktop grid it is fully/partially inside of.
                    let scale = self.layout.desktop_grid_scale;
                    
                    // Use saved maximized geometry for cell calculation
                    let x1 = (*win_ptr).saved_maximized_virtual_x;
                    let y1 = (*win_ptr).saved_maximized_virtual_y;
                    let w = (*win_ptr).saved_maximized_width as f64;
                    let h = (*win_ptr).saved_maximized_height as f64;
                    let x2 = x1 + w;
                    let y2 = y1 + h;
                    
                    let col_min = (x1 / scale).floor() as i32;
                    let col_max = ((x2 / scale).ceil() as i32 - 1).max(col_min);
                    let row_min = (y1 / scale).floor() as i32;
                    let row_max = ((y2 / scale).ceil() as i32 - 1).max(row_min);
                    
                    let snapped_x1 = col_min as f64 * scale;
                    let snapped_x2 = (col_max + 1) as f64 * scale;
                    let snapped_y1 = row_min as f64 * scale;
                    let snapped_y2 = (row_max + 1) as f64 * scale;
                    
                    let fw = snapped_x2 - snapped_x1;
                    let fh = snapped_y2 - snapped_y1;
                    
                    // Update current virtual position for rendering
                    (*win_ptr).virtual_x = snapped_x1;
                    (*win_ptr).virtual_y = snapped_y1;
                    
                    let final_x = phys_x + (((*win_ptr).virtual_x - self.desk_pan_x) * self.desk_zoom) as i32;
                    let final_y = phys_y + (((*win_ptr).virtual_y - self.desk_pan_y) * self.desk_zoom) as i32;

                    (*win_ptr).rendering_requested.x = final_x;
                    (*win_ptr).rendering_requested.y = final_y;
                    (*win_ptr).scale = self.desk_zoom;

                    (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                        width: fw as u32,
                        height: fh as u32,
                    });
                    (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                        width: fw as u32,
                        height: fh as u32,
                    };
                } else {
                    // Regular pannable window on the virtual surface
                    let fw = if let Some(resize_size) = self.get_active_resize_dimensions(win_ptr) {
                        resize_size.0 as i32
                    } else if (*win_ptr).box_geom.width > 0 {
                        (*win_ptr).box_geom.width as i32
                    } else if (*win_ptr).wm_scheduled.dimensions_hint.min_width > 32 {
                        (*win_ptr).wm_scheduled.dimensions_hint.min_width as i32
                    } else {
                        800
                    };
                    let fh = if let Some(resize_size) = self.get_active_resize_dimensions(win_ptr) {
                        resize_size.1 as i32
                    } else if (*win_ptr).box_geom.height > 0 {
                        (*win_ptr).box_geom.height as i32
                    } else if (*win_ptr).wm_scheduled.dimensions_hint.min_height > 32 {
                        (*win_ptr).wm_scheduled.dimensions_hint.min_height as i32
                    } else {
                        600
                    };

                    let final_x = phys_x + (((*win_ptr).virtual_x - self.desk_pan_x) * self.desk_zoom) as i32;
                    let final_y = phys_y + (((*win_ptr).virtual_y - self.desk_pan_y) * self.desk_zoom) as i32;

                    (*win_ptr).rendering_requested.x = final_x;
                    (*win_ptr).rendering_requested.y = final_y;
                    (*win_ptr).scale = self.desk_zoom;

                    (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions {
                        width: fw as u32,
                        height: fh as u32,
                    });
                    (*win_ptr).wm_requested.bounds = crate::window::Dimensions {
                        width: fw as u32,
                        height: fh as u32,
                    };
                }

                // Apply borders, opacity, and blur to normal windows
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
                    if !self.layout.window_opacity { 1.0f32 } else { 0.90f32 }
                };
            }
        }
        // If the focused window is no longer visible, refocus
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            let mut focused_visible = false;
            match (*seat).focused {
                crate::seat::Focus::Window(w) => {
                    if !w.is_null() && !(*w).closed && !(*w).minimized && matches!((*w).state, crate::window::WindowState::Mapped) {
                        focused_visible = true;
                    }
                }
                crate::seat::Focus::None => {
                    focused_visible = true;
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

    pub unsafe fn window_is_valid(&self, win: *mut Window) -> bool {
        if win.is_null() {
            return false;
        }
        self.windows.iter().any(|&w| w == win)
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
        let curr_seat = (*seats_list).next;
        if curr_seat != seats_list {
            Some(crate::container_of!(curr_seat, crate::seat::Seat, link))
        } else {
            None
        }
    }

    pub unsafe fn record_focus(&mut self, window: *mut Window) {
        if window.is_null() {
            return;
        }
        self.focus_history.retain(|&w| w != window);
        self.focus_history.insert(0, window);
    }

    pub unsafe fn remove_from_history(&mut self, window: *mut Window) {
        self.focus_history.retain(|&w| w != window);
    }

    pub unsafe fn focus_next_visible_window(&mut self, seat: *mut crate::seat::Seat) {
        let mut next_focus: *mut Window = std::ptr::null_mut();
        for &w in self.focus_history.iter() {
            if !w.is_null() && !(*w).closed && !(*w).minimized && matches!((*w).state, crate::window::WindowState::Mapped) {
                let app_id = (*w).get_app_id_string();
                let is_status_bar = app_id.as_deref().map_or(false, |id| id.starts_with("cce-status"));
                if !is_status_bar {
                    next_focus = w;
                    break;
                }
            }
        }
        if next_focus.is_null() {
            for &w in self.windows.iter() {
                if !w.is_null() && !(*w).closed && !(*w).minimized && matches!((*w).state, crate::window::WindowState::Mapped) {
                    let app_id = (*w).get_app_id_string();
                    let is_status_bar = app_id.as_deref().map_or(false, |id| id.starts_with("cce-status"));
                    if !is_status_bar {
                        next_focus = w;
                    }
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
                if app_id.starts_with("cce-status") {
                    status_bar_windows.push(win_ptr);
                }
            }
        }
        for win_ptr in status_bar_windows {
            let node_link = &mut (*win_ptr).node.link as *mut ffi::wl_list as *mut WlList;
            let list_head = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
            if !node_link.is_null() && !list_head.is_null() && (*node_link).next != list_head {
                if !(*node_link).prev.is_null() && !(*node_link).next.is_null() {
                    crate::server::wl_list_remove(node_link);
                }
                let prev_node = (*list_head).prev;
                if !prev_node.is_null() && (*prev_node).next == list_head {
                    crate::server::wl_list_insert(prev_node, node_link);
                }
            }
        }
    }

    pub unsafe fn raise_window(&mut self, window: *mut Window) {
        if window.is_null() {
            return;
        }
        let node_link = &mut (*window).node.link as *mut ffi::wl_list as *mut WlList;
        let list_head = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        if !node_link.is_null() && !list_head.is_null() && (*node_link).next != list_head {
            if !(*node_link).prev.is_null() && !(*node_link).next.is_null() {
                crate::server::wl_list_remove(node_link);
            }
            let prev_node = (*list_head).prev;
            if !prev_node.is_null() && (*prev_node).next == list_head {
                crate::server::wl_list_insert(prev_node, node_link);
            }
        }
        self.keep_status_bar_on_top();
    }

    pub unsafe fn execute_action(&mut self, action: &crate::config::Action, command: Option<&str>) {
        use crate::config::Action;
        self.stop_panning_animation();
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
                        if !w.is_null() && !(*w).closed && matches!((*w).state, crate::window::WindowState::Mapped) {
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
                            if !window.is_null() && !(*window).closed && !(*window).minimized {
                                let is_status_bar = (*window).get_app_id_string()
                                    .map_or(false, |aid| aid.starts_with("cce-status"));
                                if !is_status_bar {
                                    visible_windows.push(window);
                                }
                            }
                        }
                        curr = next;
                    }

                    visible_windows.sort_by_key(|&w| unsafe { (*w).ref_key.index });

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
                match self.reload_config() {
                    Ok(()) => {
                        let _ = std::process::Command::new("notify-send")
                            .arg("cce")
                            .arg("Configuration reloaded successfully")
                            .spawn();
                    }
                    Err(e) => {
                        let _ = std::process::Command::new("notify-send")
                            .arg("cce")
                            .arg(format!("Failed to reload config:\n{}", e))
                            .spawn();
                    }
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
            Action::LayoutNext => {}
            Action::ModeNext => {
                let cycle = [
                    crate::tiling::TilingMode::Floating,
                    crate::tiling::TilingMode::Fullscreen,
                ];
                if let Some(seat) = self.first_seat() {
                    if let crate::seat::Focus::Window(fw) = (*seat).focused {
                        let current_mode = (*fw).tiling_mode;
                        let next = cycle
                            .iter()
                            .position(|m| *m == current_mode)
                            .map(|i| cycle[(i + 1) % cycle.len()])
                            .unwrap_or(crate::tiling::TilingMode::Floating);
                        (*fw).tiling_mode = next;
                        (*fw).mode_locked = true;
                        self.dirty_windowing();
                    }
                }
            }
            Action::ModeNextShared => {
                let cycle = [
                    crate::tiling::TilingMode::Floating,
                    crate::tiling::TilingMode::Fullscreen,
                ];
                if let Some(seat) = self.first_seat() {
                    if let crate::seat::Focus::Window(fw) = (*seat).focused {
                        let current_mode = (*fw).tiling_mode;
                        let next = cycle
                            .iter()
                            .position(|m| *m == current_mode)
                            .map(|i| cycle[(i + 1) % cycle.len()])
                            .unwrap_or(crate::tiling::TilingMode::Floating);
                        
                        for &w in self.windows.iter() {
                            if !w.is_null() && !(*w).closed && !matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) && (*w).tiling_mode == current_mode {
                                (*w).tiling_mode = next;
                                (*w).mode_locked = true;
                            }
                        }
                        self.dirty_windowing();
                    }
                }
            }
            Action::View1 | Action::View2 | Action::View3 | Action::View4 => {
                let (target_x, target_y) = match action {
                    Action::View1 => (0.0, 0.0),
                    Action::View2 => (2000.0, 0.0),
                    Action::View3 => (0.0, 2000.0),
                    Action::View4 => (2000.0, 2000.0),
                    _ => (0.0, 0.0),
                };
                
                let (mut viewport_w, mut viewport_h) = (1920.0, 1080.0);
                let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                let mut curr_out = (*outputs_list).next;
                while curr_out != outputs_list {
                    let output = crate::container_of!(curr_out, crate::output::Output, link);
                    if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                        let wlr_box = (*output).sent.box_layout();
                        viewport_w = wlr_box.width as f64;
                        viewport_h = wlr_box.height as f64;
                        break;
                    }
                    curr_out = (*curr_out).next;
                }
                
                self.desk_pan_x = target_x - (viewport_w / 2.0) / self.desk_zoom;
                self.desk_pan_y = target_y - (viewport_h / 2.0) / self.desk_zoom;
                self.dirty_windowing();
            }
            Action::SetViewport1 | Action::SetViewport2 | Action::SetViewport3 | Action::SetViewport4 => {
                let (target_x, target_y) = match action {
                    Action::SetViewport1 => (0.0, 0.0),
                    Action::SetViewport2 => (2000.0, 0.0),
                    Action::SetViewport3 => (0.0, 2000.0),
                    Action::SetViewport4 => (2000.0, 2000.0),
                    _ => (0.0, 0.0),
                };
                if let Some(seat) = self.first_seat() {
                    if let crate::seat::Focus::Window(fw) = (*seat).focused {
                        let w = if (*fw).box_geom.width > 0 { (*fw).box_geom.width as f64 } else { 800.0 };
                        let h = if (*fw).box_geom.height > 0 { (*fw).box_geom.height as f64 } else { 600.0 };
                        (*fw).virtual_x = target_x - w / 2.0;
                        (*fw).virtual_y = target_y - h / 2.0;
                        self.dirty_windowing();
                    }
                }
            }
            Action::ZoomIn | Action::ZoomOut | Action::ZoomReset => {
                let (mut viewport_w, mut viewport_h) = (1920.0, 1080.0);
                let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                let mut curr_out = (*outputs_list).next;
                while curr_out != outputs_list {
                    let output = crate::container_of!(curr_out, crate::output::Output, link);
                    if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                        let wlr_box = (*output).sent.box_layout();
                        viewport_w = wlr_box.width as f64;
                        viewport_h = wlr_box.height as f64;
                        break;
                    }
                    curr_out = (*curr_out).next;
                }
                
                let cx = self.desk_pan_x + (viewport_w / 2.0) / self.desk_zoom;
                let cy = self.desk_pan_y + (viewport_h / 2.0) / self.desk_zoom;
                
                let new_zoom = match action {
                    Action::ZoomIn => (self.desk_zoom * 1.1).min(10.0),
                    Action::ZoomOut => (self.desk_zoom / 1.1).max(0.1),
                    Action::ZoomReset => 1.0,
                    _ => self.desk_zoom,
                };
                
                self.desk_pan_x = cx - (viewport_w / 2.0) / new_zoom;
                self.desk_pan_y = cy - (viewport_h / 2.0) / new_zoom;
                self.desk_zoom = new_zoom;
                self.mode = if (new_zoom - 1.0).abs() > 0.001 { WindowManagerMode::Overview } else { WindowManagerMode::Normal };
                self.dirty_windowing();
            }
            Action::PanLeft | Action::PanRight | Action::PanUp | Action::PanDown => {
                let step = 100.0 / self.desk_zoom;
                match action {
                    Action::PanLeft => self.desk_pan_x -= step,
                    Action::PanRight => self.desk_pan_x += step,
                    Action::PanUp => self.desk_pan_y -= step,
                    Action::PanDown => self.desk_pan_y += step,
                    _ => {}
                }
                self.dirty_windowing();
            }
            Action::OverlayLeft => {
                self.layout.overlay_position = "left".to_string();
                self.dirty_windowing();
            }
            Action::OverlayRight => {
                self.layout.overlay_position = "right".to_string();
                self.dirty_windowing();
            }
            Action::Expose => {
                if self.mode == WindowManagerMode::Overview {
                    let mut viewport_w = 1920.0;
                    let mut viewport_h = 1080.0;
                    let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                    let mut curr_out = (*outputs_list).next;
                    while curr_out != outputs_list {
                        let output = crate::container_of!(curr_out, crate::output::Output, link);
                        if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                            let wlr_box = (*output).sent.box_layout();
                            viewport_w = wlr_box.width as f64;
                            viewport_h = wlr_box.height as f64;
                            break;
                        }
                        curr_out = (*curr_out).next;
                    }

                    if let Some(seat) = self.first_seat() {
                        if let crate::seat::Focus::Window(fw) = (*seat).focused {
                            if self.window_is_valid(fw) {
                                let win_w = if (*fw).box_geom.width > 0 { (*fw).box_geom.width as f64 } else { 800.0 };
                                let win_h = if (*fw).box_geom.height > 0 { (*fw).box_geom.height as f64 } else { 600.0 };
                                let center_x = (*fw).virtual_x + win_w / 2.0;
                                let center_y = (*fw).virtual_y + win_h / 2.0;
                                self.desk_zoom = 1.0;
                                self.mode = WindowManagerMode::Normal;
                                self.desk_pan_x = center_x - viewport_w / 2.0;
                                self.desk_pan_y = center_y - viewport_h / 2.0;
                                self.stop_panning_animation();
                                self.dirty_windowing();
                                return;
                            }
                        }
                    }
                    self.desk_zoom = 1.0;
                    self.mode = WindowManagerMode::Normal;
                    self.desk_pan_x = 0.0;
                    self.desk_pan_y = 0.0;
                    self.stop_panning_animation();
                    self.dirty_windowing();
                } else {
                    let mut min_vx = f64::MAX;
                    let mut max_vx = f64::MIN;
                    let mut min_vy = f64::MAX;
                    let mut max_vy = f64::MIN;
                    let mut has_visible_windows = false;

                    for &win_ptr in self.windows.iter() {
                        if win_ptr.is_null() || (*win_ptr).closed || (*win_ptr).minimized {
                            continue;
                        }

                        let app_id = (*win_ptr).get_app_id_string();
                        let is_status_bar = app_id.as_deref().map_or(false, |id| id.starts_with("cce-status"));
                        if is_status_bar {
                            continue;
                        }

                        let visible = !matches!((*win_ptr).state, crate::window::WindowState::Closing | crate::window::WindowState::Init);
                        if !visible {
                            continue;
                        }

                        let mode = self.get_mode_for_window(win_ptr);
                        if mode == crate::tiling::TilingMode::Popup || mode == crate::tiling::TilingMode::Overlay {
                            continue;
                        }

                        let win_w = if (*win_ptr).box_geom.width > 0 { (*win_ptr).box_geom.width as f64 } else { 800.0 };
                        let win_h = if (*win_ptr).box_geom.height > 0 { (*win_ptr).box_geom.height as f64 } else { 600.0 };

                        let vx = (*win_ptr).virtual_x;
                        let vy = (*win_ptr).virtual_y;

                        if vx < min_vx { min_vx = vx; }
                        if vx + win_w > max_vx { max_vx = vx + win_w; }
                        if vy < min_vy { min_vy = vy; }
                        if vy + win_h > max_vy { max_vy = vy + win_h; }
                        has_visible_windows = true;
                    }

                    if has_visible_windows {
                        let mut viewport_w = 1920.0;
                        let mut viewport_h = 1080.0;
                        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                        let mut curr_out = (*outputs_list).next;
                        while curr_out != outputs_list {
                            let output = crate::container_of!(curr_out, crate::output::Output, link);
                            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                                let wlr_box = (*output).sent.box_layout();
                                viewport_w = wlr_box.width as f64;
                                viewport_h = wlr_box.height as f64;
                                break;
                            }
                            curr_out = (*curr_out).next;
                        }

                        let box_w = max_vx - min_vx;
                        let box_h = max_vy - min_vy;

                        let margin = 100.0;
                        let avail_w = (viewport_w - 2.0 * margin).max(200.0);
                        let avail_h = (viewport_h - 2.0 * margin).max(200.0);

                        let zoom_x = avail_w / box_w.max(1.0);
                        let zoom_y = avail_h / box_h.max(1.0);
                        let new_zoom = zoom_x.min(zoom_y).min(1.0).max(0.05);

                        let center_x = min_vx + box_w / 2.0;
                        let center_y = min_vy + box_h / 2.0;

                        self.desk_zoom = new_zoom;
                        self.mode = WindowManagerMode::Overview;
                        self.desk_pan_x = center_x - (viewport_w / 2.0) / new_zoom;
                        self.desk_pan_y = center_y - (viewport_h / 2.0) / new_zoom;
                        self.dirty_windowing();
                    }
                }
            }
            _ => {}
        }
    }

    pub unsafe fn process_ipc_command(&mut self, cmd: &str) -> String {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            self.stop_panning_animation();
            return "error: empty command\n".to_string();
        }
        
        let action = parts[0];
        if action != "focus-window" {
            self.stop_panning_animation();
        }
        match action {
            "view" => {
                if parts.len() < 2 { return "error: missing tag\n".to_string(); }
                if let Ok(tag) = parts[1].parse::<i32>() {
                    if tag >= 1 && tag <= 4 {
                        let act = match tag {
                            1 => crate::config::Action::View1,
                            2 => crate::config::Action::View2,
                            3 => crate::config::Action::View3,
                            4 => crate::config::Action::View4,
                            _ => crate::config::Action::None,
                        };
                        self.execute_action(&act, None);
                        return "ok\n".to_string();
                    }
                }
                "error: invalid tag\n".to_string()
            }
            "set-viewport" | "set-tag" => {
                if parts.len() < 2 { return "error: missing viewport index\n".to_string(); }
                if let Ok(tag) = parts[1].parse::<i32>() {
                    if tag >= 1 && tag <= 4 {
                        let act = match tag {
                            1 => crate::config::Action::SetViewport1,
                            2 => crate::config::Action::SetViewport2,
                            3 => crate::config::Action::SetViewport3,
                            4 => crate::config::Action::SetViewport4,
                            _ => crate::config::Action::None,
                        };
                        self.execute_action(&act, None);
                        return "ok\n".to_string();
                    }
                }
                "error: invalid viewport index\n".to_string()
            }
            "pan-by" => {
                if parts.len() < 3 { return "error: missing dx or dy\n".to_string(); }
                if let (Ok(dx), Ok(dy)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    self.desk_pan_x += dx;
                    self.desk_pan_y += dy;
                    self.dirty_windowing();
                    return "ok\n".to_string();
                }
                "error: invalid dx or dy\n".to_string()
            }
            "pan-to" => {
                if parts.len() < 3 { return "error: missing x or y\n".to_string(); }
                if let (Ok(x), Ok(y)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    self.desk_pan_x = x;
                    self.desk_pan_y = y;
                    self.dirty_windowing();
                    return "ok\n".to_string();
                }
                "error: invalid x or y\n".to_string()
            }
            "zoom-in" => {
                self.execute_action(&crate::config::Action::ZoomIn, None);
                "ok\n".to_string()
            }
            "zoom-out" => {
                self.execute_action(&crate::config::Action::ZoomOut, None);
                "ok\n".to_string()
            }
            "set-zoom" => {
                if parts.len() < 2 { return "error: missing zoom factor\n".to_string(); }
                if let Ok(factor) = parts[1].parse::<f64>() {
                    let new_zoom = factor.clamp(0.1, 10.0);
                    let (mut viewport_w, mut viewport_h) = (1920.0, 1080.0);
                    let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                    let mut curr_out = (*outputs_list).next;
                    while curr_out != outputs_list {
                        let output = crate::container_of!(curr_out, crate::output::Output, link);
                        if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                            let wlr_box = (*output).sent.box_layout();
                            viewport_w = wlr_box.width as f64;
                            viewport_h = wlr_box.height as f64;
                            break;
                        }
                        curr_out = (*curr_out).next;
                    }
                    let cx = self.desk_pan_x + (viewport_w / 2.0) / self.desk_zoom;
                    let cy = self.desk_pan_y + (viewport_h / 2.0) / self.desk_zoom;
                    self.desk_pan_x = cx - (viewport_w / 2.0) / new_zoom;
                    self.desk_pan_y = cy - (viewport_h / 2.0) / new_zoom;
                    self.desk_zoom = new_zoom;
                    self.mode = if (new_zoom - 1.0).abs() > 0.001 { WindowManagerMode::Overview } else { WindowManagerMode::Normal };
                    self.dirty_windowing();
                    return "ok\n".to_string();
                }
                "error: invalid zoom factor\n".to_string()
            }
            "set-coords" => {
                if parts.len() < 3 { return "error: missing x or y\n".to_string(); }
                if let (Ok(x), Ok(y)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    if let Some(seat) = self.first_seat() {
                        if let crate::seat::Focus::Window(fw) = (*seat).focused {
                            (*fw).virtual_x = x;
                            (*fw).virtual_y = y;
                            self.dirty_windowing();
                            return "ok\n".to_string();
                        }
                    }
                    return "error: no focused window\n".to_string();
                }
                "error: invalid x or y\n".to_string()
            }
            "set-coords-of" => {
                if parts.len() < 4 { return "error: missing app_id, x, or y\n".to_string(); }
                let app_id_query = parts[1];
                if let (Ok(x), Ok(y)) = (parts[2].parse::<f64>(), parts[3].parse::<f64>()) {
                    let mut found = false;
                    for &w in self.windows.iter() {
                        if !w.is_null() && !(*w).closed && !(*w).minimized && matches!((*w).state, crate::window::WindowState::Mapped) {
                            if let Some(aid) = (*w).get_app_id_string() {
                                if aid.to_lowercase() == app_id_query.to_lowercase() {
                                    (*w).virtual_x = x;
                                    (*w).virtual_y = y;
                                    found = true;
                                }
                            }
                        }
                    }
                    if found {
                        self.dirty_windowing();
                        return "ok\n".to_string();
                    } else {
                        return "error: window not found\n".to_string();
                    }
                }
                "error: invalid x or y\n".to_string()
            }
            "close" => {
                self.execute_action(&crate::config::Action::Close, None);
                "ok\n".to_string()
            }
            "expose" => {
                self.execute_action(&crate::config::Action::Expose, None);
                "ok\n".to_string()
            }
            "wm-mode" => {
                if parts.len() < 2 {
                    return format!("{:?}\n", self.mode).to_lowercase();
                }
                let target = parts[1].to_lowercase();
                if target == "normal" {
                    if self.mode == WindowManagerMode::Overview {
                        self.execute_action(&crate::config::Action::Expose, None);
                    }
                    return "ok\n".to_string();
                } else if target == "overview" {
                    if self.mode == WindowManagerMode::Normal {
                        self.execute_action(&crate::config::Action::Expose, None);
                    }
                    return "ok\n".to_string();
                }
                "error: invalid mode, specify 'normal' or 'overview'\n".to_string()
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
                if parts.len() < 2 { return "error: missing app_id/id\n".to_string(); }
                let query = parts[1..].join(" ").to_lowercase();
                if let Some(seat) = self.first_seat() {
                    let mut best_target: *mut Window = std::ptr::null_mut();
                    let mut best_score = 0;

                    // Try to match by numerical window index first
                    if let Ok(id) = query.parse::<u32>() {
                        for &w in self.windows.iter() {
                            if !w.is_null() && !(*w).closed && matches!((*w).state, crate::window::WindowState::Mapped) && (*w).ref_key.index == id {
                                best_target = w;
                                break;
                            }
                        }
                    }

                    // Fallback to matching by app_id
                    if best_target.is_null() {
                        for &w in self.windows.iter() {
                            if !w.is_null() && !(*w).closed && matches!((*w).state, crate::window::WindowState::Mapped) {
                                let aid = (*w).get_app_id_string();
                                
                                let mut score = 0;
                                if let Some(ref aid_str) = aid {
                                    let aid_lower = aid_str.to_lowercase();
                                    if aid_lower == query {
                                        score = score.max(100);
                                    } else if aid_lower.contains(&query) {
                                        score = score.max(50);
                                    }
                                }

                                if score > best_score {
                                    best_score = score;
                                    best_target = w;
                                }
                            }
                        }
                    }

                    if !best_target.is_null() {
                        if (*best_target).minimized {
                            (*best_target).minimized = false;
                        }
                        (*seat).focus(crate::seat::Focus::Window(best_target));
                        self.raise_window(best_target);
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
                unsafe {
                    match self.reload_config() {
                        Ok(()) => "ok\n".to_string(),
                        Err(e) => format!("error: failed to reload config: {}\n", e),
                    }
                }
            }
            "retile" => {
                self.dirty_windowing();
                "ok\n".to_string()
            }
            "windows" => {
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

                let mut out = String::new();
                for &w in self.windows.iter() {
                    if !w.is_null() && !(*w).closed && !matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                        let app_id = (*w).get_app_id_string().unwrap_or_default();
                        let title = (*w).get_title_string().unwrap_or_default();
                        out.push_str(&format!(
                            "window id={} app_id={} title=\"{}\" mode={} x={} y={} w={} h={} vx={:.1} vy={:.1} minimized={} has_parent={} focused={} ssd={}\n",
                            (*w).ref_key.index,
                            app_id,
                            title,
                            (*w).tiling_mode.as_str(),
                            (*w).box_geom.x,
                            (*w).box_geom.y,
                            (*w).box_geom.width,
                            (*w).box_geom.height,
                            (*w).virtual_x,
                            (*w).virtual_y,
                            (*w).minimized,
                            (*w).has_parent,
                            w == focused_window,
                            (*w).wm_requested.ssd,
                        ));
                    }
                }
                out
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
                    "desktop_background" => {
                        self.layout.desktop_background = val.to_string();
                        let parsed_color = crate::config::parse_hex_color(val);
                        self.layout.background_r = ((parsed_color >> 16) & 0xFF) * 0x01010101;
                        self.layout.background_g = ((parsed_color >> 8) & 0xFF) * 0x01010101;
                        self.layout.background_b = (parsed_color & 0xFF) * 0x01010101;
                        unsafe {
                            let outputs_head = &mut (*self.server).om.outputs as *mut crate::ffi::wl_list as *mut crate::server::WlList;
                            let mut curr = (*outputs_head).next;
                            while curr != outputs_head {
                                let next = (*curr).next;
                                let output = &mut *crate::container_of!(curr, crate::output::Output, link);
                                output.update_background_color();
                                curr = next;
                            }
                        }
                    }
                    "desktop_grid_color" => {
                        self.layout.desktop_grid_color = crate::config::parse_hex_color_rgba(val);
                    }
                    "desktop_grid_scale" => {
                        if let Ok(v) = val.parse::<f64>() {
                            self.layout.desktop_grid_scale = v;
                        }
                    }
                    "desktop_line_width" => {
                        if let Ok(v) = val.parse::<i32>() {
                            self.layout.desktop_line_width = v;
                        }
                    }
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
                    "side_panel_width" | "pinned_width" | "overlay_width" => { if let Ok(v) = val.parse::<i32>() { self.layout.overlay_width = v; } }
                    "side_panel_behavior" | "pinned_behavior" | "overlay_behavior" => { self.layout.overlay_behavior = val.to_string(); }
                    "side_panel_position" | "pinned_position" | "overlay_position" => { self.layout.overlay_position = val.to_string(); }
                    "side_panel_border_gap" | "pinned_border_gap" | "overlay_border_gap" => { if let Ok(v) = val.parse::<i32>() { self.layout.overlay_border_gap = v; } }
                    _ => return format!("error: unknown layout key: {}\n", key),
                }
                self.dirty_windowing();
                "ok\n".to_string()
            }
            "viewport-layout" => {
                "ok\n".to_string()
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
                            let name_ptr = ffi::river_wlr_input_device_get_name((*device).wlr_device);
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
            "pointer-move-to" => {
                if parts.len() < 3 { return "error: usage: pointer-move-to <x> <y>\n".to_string(); }
                if let (Ok(x), Ok(y)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
                    let mut curr_seat = (*seats_list).next;
                    while curr_seat != seats_list {
                        let next_seat = (*curr_seat).next;
                        let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
                        let cursor = &mut (*seat).cursor;
                        ffi::wlr_cursor_warp_absolute(cursor.wlr_cursor, std::ptr::null_mut(), x, y);
                        cursor.update_hovered();
                        cursor.passthrough(crate::util::msec_timestamp());
                        curr_seat = next_seat;
                    }
                    "ok\n".to_string()
                } else {
                    "error: invalid x or y\n".to_string()
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
            let name_ptr = ffi::river_wlr_input_device_get_name((*device).wlr_device);
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

    pub unsafe fn spawn_startup_program(&mut self, prog: crate::config::StartupConfig) {
        log::info!("spawning TOML startup program: {}", prog.exec);
        let cmd = prog.exec.clone();
        match nix::unistd::fork() {
            Ok(nix::unistd::ForkResult::Child) => {
                crate::process::cleanup_child();

                if !self.server.is_null() && !(*self.server).xwayland.is_null() {
                    let xwayland_cast = (*self.server).xwayland as *mut crate::server::WlrXwayland;
                    if !(*xwayland_cast).display_name.is_null() {
                        let display_name = std::ffi::CStr::from_ptr((*xwayland_cast).display_name)
                            .to_string_lossy()
                            .into_owned();
                        std::env::set_var("DISPLAY", display_name);
                    }
                }

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
            Ok(nix::unistd::ForkResult::Parent { child }) => {
                self.startup_pids.push((prog, child));
            }
            Err(e) => {
                log::error!("failed to fork child for startup program: {}", e);
            }
        }
    }

    pub unsafe fn reload_config(&mut self) -> Result<(), String> {
        if let Some(path) = crate::config::default_config_path() {
            let old_pids = std::mem::take(&mut self.startup_pids);
            match crate::config::parse_config(&path, self) {
                Ok(()) => {
                    // Update scales of existing outputs from the newly loaded config
                    let om_outputs = &mut (*self.server).om.outputs as *mut ffi::wl_list;
                    let mut link = (*om_outputs).next;
                    while link != om_outputs {
                        let output = &mut *crate::container_of!(link, crate::output::Output, link);
                        let wlr_output = output.wlr_output;
                        if !wlr_output.is_null() {
                            let name_raw = ffi::river_wlr_output_get_name(wlr_output);
                            let name = std::ffi::CStr::from_ptr(name_raw).to_string_lossy();
                            let scale_key = format!("scale_{}", name);
                            let output_scale = self.display.get(&scale_key)
                                .map(|&s| s as f32)
                                .unwrap_or(self.output_scale);
                            if output.scheduled.scale != output_scale {
                                output.scheduled.scale = output_scale;
                            }
                        }
                        link = (*link).next;
                    }

                    self.dirty_windowing();

                    // Process old PIDs
                    for (old_prog, old_pid) in old_pids {
                        // If it is still in new startup and once == true, keep it running
                        let still_exists_and_once = self.startup.iter().any(|p| p.exec == old_prog.exec && p.once && !p.restart);
                        if still_exists_and_once {
                            self.startup_pids.push((old_prog, old_pid));
                        } else {
                            log::info!("Terminating old startup program pid {} ({})", old_pid, old_prog.exec);
                            let _ = nix::sys::signal::kill(old_pid, nix::sys::signal::Signal::SIGTERM);
                        }
                    }

                    // Spawn new/restarted programs
                    let current_startup = self.startup.clone();
                    for prog in current_startup {
                        if prog.once {
                            // Only spawn if not already running
                            let running = self.startup_pids.iter().any(|(p, _)| p.exec == prog.exec);
                            if !running {
                                self.spawn_startup_program(prog);
                            }
                        } else {
                            // once == false: spawn a new instance
                            self.spawn_startup_program(prog);
                        }
                    }

                    Ok(())
                }
                Err(e) => {
                    self.startup_pids = old_pids;
                    log::error!("failed to reload config: {}", e);
                    Err(e)
                }
            }
        } else {
            log::error!("no config file found to reload");
            Err("No config file found".to_string())
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

unsafe fn rendered_fullscreen(window: *mut Window) -> bool {
    (*window).is_fullscreen() && !(*window).rendering_requested.hidden
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
                    ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_UNRESPONSIVE,
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
unsafe extern "C" fn wm_stop(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if !wm.is_null() {
        (*wm).object = std::ptr::null_mut();
        ffi::wl_resource_post_event(resource, ffi::ZCCE_WINDOW_MANAGER_V1_FINISHED);
        ffi::wl_resource_set_implementation(
            resource,
            &INERT_WM_INTERFACE as *const _ as *const _,
            std::ptr::null_mut(),
            None,
        );
    }
}

unsafe extern "C" fn wm_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn wm_manage_finish(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    if !matches!((*wm).state, WindowManagerState::Manage) {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
            b"manage_finish request does not match manage_start\0".as_ptr() as *const _,
        );
        return;
    }
    (*wm).manage_finish();
}

unsafe extern "C" fn wm_manage_dirty(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    (*wm).scheduled.dirty_lazy = true;
    (*wm).add_dirty_idle();
}

unsafe extern "C" fn wm_render_finish(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    if !matches!((*wm).state, WindowManagerState::Render) {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
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

unsafe extern "C" fn wm_exit_session(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    log::info!("window manager requested to exit session");
    ffi::wl_display_terminate((*(*wm).server).wl_server);
}

static WM_INTERFACE: ffi::zcce_window_manager_v1_interface = ffi::zcce_window_manager_v1_interface {
    stop: Some(wm_stop),
    destroy: Some(wm_destroy),
    manage_finish: Some(wm_manage_finish),
    manage_dirty: Some(wm_manage_dirty),
    render_finish: Some(wm_render_finish),
    get_shell_surface: Some(wm_get_shell_surface),
    exit_session: Some(wm_exit_session),
    get_cce_toplevel: Some(crate::cce_window_management::cce_wm_get_cce_toplevel),
};

static INERT_WM_INTERFACE: ffi::zcce_window_manager_v1_interface = ffi::zcce_window_manager_v1_interface {
    stop: None,
    destroy: Some(wm_destroy),
    manage_finish: None,
    manage_dirty: None,
    render_finish: None,
    get_shell_surface: None,
    exit_session: None,
    get_cce_toplevel: None,
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
    log::info!("Client binding zcce_window_manager_v1: PID={}, cmdline='{}'", pid, cmdline);

    let resource = ffi::wl_resource_create(client, &ffi::zcce_window_manager_v1_interface, version as i32, id);
    if resource.is_null() {
        ffi::wl_client_post_no_memory(client);
        log::error!("out of memory binding zcce_window_manager_v1");
        return;
    }

    // We do not set (*wm).object = resource, so the built-in window manager remains active.
    // We just set the implementation to WM_INTERFACE so the client can call get_cce_toplevel.
    ffi::wl_resource_set_implementation(
        resource,
        &WM_INTERFACE as *const _ as *const _,
        wm as *mut _,
        Some(handle_destroy_wm_resource),
    );
}

unsafe extern "C" fn handle_destroy_wm_resource(resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    if (*wm).object != resource {
        return;
    }
    log::debug!("active zcce_window_manager_v1 destroyed");
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

pub(crate) unsafe extern "C" fn handle_panning_animation_tick(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    
    let mut done = true;
    let factor = 0.15;
    
    if let Some(target_x) = (*wm).target_desk_pan_x {
        let dx = target_x - (*wm).desk_pan_x;
        if dx.abs() > 0.5 {
            (*wm).desk_pan_x += dx * factor;
            done = false;
        } else {
            (*wm).desk_pan_x = target_x;
            (*wm).target_desk_pan_x = None;
        }
    }
    
    if let Some(target_y) = (*wm).target_desk_pan_y {
        let dy = target_y - (*wm).desk_pan_y;
        if dy.abs() > 0.5 {
            (*wm).desk_pan_y += dy * factor;
            done = false;
        } else {
            (*wm).desk_pan_y = target_y;
            (*wm).target_desk_pan_y = None;
        }
    }
    
    (*wm).dirty_windowing();
    
    if !done {
        if !(*wm).animation_timer.is_null() {
            ffi::wl_event_source_timer_update((*wm).animation_timer, 16);
        }
    }
    0
}
