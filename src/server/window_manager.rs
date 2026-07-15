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

pub use crate::policy::state::{SavedState, SavedWindowState};

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
    pub status_hide_mode: bool,
    pub adjust_position_mode: bool,
    /// xkb modifier mask currently held via injected `key-down` (see the ipc handler):
    /// OR'd over the device state on every synthetic modifiers notify so clients see
    /// ctrl/shift/alt/super combos from injection like they would from hardware.
    pub injected_key_mods: u32,
    pub restore_queue: Vec<SavedWindowState>,
    pub last_window_states: Vec<SavedWindowState>,
    pub shutting_down: bool,
    pub target_desk_pan_x: Option<f64>,
    pub target_desk_pan_y: Option<f64>,
    pub animation_timer: *mut ffi::wl_event_source,
    pub has_restored_focused_window: bool,
    pub restored_focused_window_mapped: bool,
    pub last_viewport_zoom: f64,
    pub last_viewport_pan_x: f64,
    pub last_viewport_pan_y: f64,
    pub viewport_is_active: bool,
    pub clean_exit_in_progress: bool,
    pub clean_exit_timer: *mut ffi::wl_event_source,
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
        self.last_window_states = Vec::new();
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
        self.status_hide_mode = false;
        self.adjust_position_mode = false;
        self.injected_key_mods = 0;
        let _ = std::fs::remove_file("/tmp/cce-status-interface-adjust-mode");

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
            ffi::wl_event_source_remove(self.timeout);
            return Err("Failed to create IPC timer event source");
        }
        ffi::wl_event_source_timer_update(self.ipc_timer, 10);

        self.clean_exit_timer = ffi::wl_event_loop_add_timer(event_loop, Some(handle_clean_exit_timeout), self as *mut WindowManager as *mut _);
        if self.clean_exit_timer.is_null() {
            ffi::wl_event_source_remove(self.timeout);
            ffi::wl_event_source_remove(self.ipc_timer);
            return Err("Failed to create clean exit timer event source");
        }
        self.clean_exit_in_progress = false;

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
                self.last_window_states = state.last_window_states;
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

    pub unsafe fn save_state(&mut self) {
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
        let mut last_states = self.last_window_states.clone();

        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                continue;
            }
            if (*w).is_status_bar() || (*w).is_wallpaper() {
                continue;
            }

            let app_id = (*w).get_app_id_string().unwrap_or_default();
            if app_id.is_empty() {
                continue;
            }
            // cce-cloud surfaces are transient popups owned by the daemon, so
            // their cmdline is `cce-cloud --daemon` — restoring one would spawn
            // a duplicate daemon that steals the socket from cce-cloud.service.
            if app_id == "cce-cloud" {
                continue;
            }
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

            let win_state = SavedWindowState {
                app_id: app_id.clone(),
                title: title.clone(),
                tiling_mode: (*w).tiling_mode,
                minimized: (*w).minimized,
                virtual_x: (*w).virtual_x,
                virtual_y: (*w).virtual_y,
                scale: (*w).scale,
                width: (*w).box_geom.width as u32,
                height: (*w).box_geom.height as u32,
                cmdline,
                focused: is_focused,
            };

            saved_wins.push(win_state.clone());

            if let Some(pos) = last_states.iter().position(|s| s.app_id == app_id) {
                last_states[pos] = win_state;
            } else {
                last_states.push(win_state);
            }
        }
        // Scrub entries persisted before the cce-cloud exclusion above.
        last_states.retain(|s| s.app_id != "cce-cloud");
        self.last_window_states = last_states;

        let state = SavedState {
            desk_pan_x: self.desk_pan_x,
            desk_pan_y: self.desk_pan_y,
            desk_zoom: self.desk_zoom,
            global_layout: self.global_layout,
            windows: saved_wins,
            last_window_states: self.last_window_states.clone(),
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

    pub unsafe fn start_clean_exit(&mut self) {
        if self.clean_exit_in_progress {
            return;
        }
        log::info!("Starting clean exit process...");

        // Save state before closing windows and setting exit flags
        self.save_state();

        self.clean_exit_in_progress = true;
        self.shutting_down = true;

        // Get list of windows we need to wait for to close cleanly
        let mut normal_windows = Vec::new();
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                continue;
            }
            if (*w).is_status_bar() || (*w).is_wallpaper() {
                continue;
            }
            normal_windows.push(w);
        }

        if normal_windows.is_empty() {
            log::info!("No active windows to close. Exiting immediately.");
            ffi::wl_display_terminate((*self.server).wl_server);
            return;
        }

        log::info!("Sending close request to {} windows...", normal_windows.len());
        for &w in &normal_windows {
            log::info!("Closing window: {:?}", (*w).get_title_string());
            (*w).close();
        }

        // Set a clean exit timer fallback to 2.0 seconds (2000 ms)
        ffi::wl_event_source_timer_update(self.clean_exit_timer, 2000);
    }

    pub unsafe fn check_clean_exit_progress(&mut self) {
        if !self.clean_exit_in_progress {
            return;
        }

        let mut normal_windows_count = 0;
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                continue;
            }
            if (*w).is_status_bar() || (*w).is_wallpaper() {
                continue;
            }
            normal_windows_count += 1;
        }

        if normal_windows_count == 0 {
            log::info!("All windows closed cleanly. Exiting display server.");
            if !self.clean_exit_timer.is_null() {
                ffi::wl_event_source_remove(self.clean_exit_timer);
                self.clean_exit_timer = std::ptr::null_mut();
            }
            ffi::wl_display_terminate((*self.server).wl_server);
        } else {
            log::info!("Clean exit: waiting for {} remaining windows to close...", normal_windows_count);
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

    pub unsafe fn match_last_window_state(&self, app_id: &str, title: &str) -> Option<SavedWindowState> {
        if app_id.is_empty() {
            return None;
        }
        // First pass: Exact match (app_id AND title)
        if let Some(w) = self.last_window_states.iter().find(|w| w.app_id == app_id && w.title == title) {
            return Some(w.clone());
        }
        // Second pass: Fuzzy title match
        if let Some(w) = self.last_window_states.iter().find(|w| {
            if w.app_id != app_id {
                return false;
            }
            let t1 = title.trim_end_matches('*');
            let t2 = w.title.trim_end_matches('*');
            t1 == t2 || t1.starts_with(t2) || t2.starts_with(t1)
        }) {
            return Some(w.clone());
        }
        // Third pass: app_id only match
        if let Some(w) = self.last_window_states.iter().find(|w| w.app_id == app_id) {
            return Some(w.clone());
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
        if !self.clean_exit_timer.is_null() {
            ffi::wl_event_source_remove(self.clean_exit_timer);
            self.clean_exit_timer = std::ptr::null_mut();
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

        let has_wallpaper = self.windows.iter().any(|&w| !w.is_null() && !(*w).closed && (*w).is_wallpaper());
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let next_out = (*curr_out).next;
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                if !(*output).background_rect.is_null() {
                    ffi::wlr_scene_node_set_enabled((*output).background_rect as *mut ffi::wlr_scene_node, !has_wallpaper);
                }
            }
            curr_out = next_out;
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
        log::info!("[render_finish_loop] START - render_list={:p}", render_list);
        let mut debug_curr = (*render_list).next;
        let mut debug_idx = 0;
        while debug_curr != render_list {
            let node = crate::container_of!(debug_curr, crate::wm_node::WmNode, link);
            if let crate::wm_node::WmNodeType::Window(window) = (*node).get() {
                let app_id = (*window).get_app_id_string().unwrap_or_default();
                log::info!("[render_finish_loop] List[{}] = app_id={} state={:?}", debug_idx, app_id, (*window).state);
            }
            debug_curr = (*debug_curr).next;
            debug_idx += 1;
        }

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
                            let layer = if (*window).get_app_id_string().as_deref() == Some("cce-wallpaper") {
                                (*self.server).scene.layers.background
                            } else if rendered_fullscreen(window) {
                                (*self.server).scene.layers.fullscreen
                            } else if (*window).tiling_mode == crate::tiling::TilingMode::Popup {
                                (*self.server).scene.layers.popups
                            } else if (*window).tiling_mode == crate::tiling::TilingMode::Status {
                                (*self.server).scene.layers.top
                            } else if (*window).rendering_requested.circular {
                                (*self.server).scene.layers.top
                            } else if (*window).tiling_mode == crate::tiling::TilingMode::Overlay && self.layout.overlay_behavior == "above" {
                                (*self.server).scene.layers.top
                            } else {
                                (*self.server).scene.layers.wm
                            };

                            ffi::wlr_scene_node_reparent((*window).tree as *mut _, layer);
                            if (*window).get_app_id_string().as_deref() == Some("cce-wallpaper") {
                                ffi::wlr_scene_node_lower_to_bottom((*window).tree as *mut _);
                            } else {
                                ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                            }
                            if rendered_fullscreen(window) {
                                found_fullscreen = true;
                            }

                            ffi::wlr_scene_node_reparent((*window).popup_tree as *mut _, layer);
                            ffi::wlr_scene_node_place_above((*window).popup_tree as *mut _, (*window).tree as *mut _);
                        }
                    }
                }
                crate::wm_node::WmNodeType::ShellSurface(shell_surface) => {
                    (*shell_surface).render_finish();
                    if reorder {
                        let layer = if found_fullscreen {
                            (*self.server).scene.layers.fullscreen
                        } else {
                            (*self.server).scene.layers.wm
                        };

                        ffi::wlr_scene_node_reparent((*shell_surface).tree as *mut _, layer);
                        ffi::wlr_scene_node_raise_to_top((*shell_surface).tree as *mut _);

                        ffi::wlr_scene_node_reparent((*shell_surface).popup_tree as *mut _, layer);
                        ffi::wlr_scene_node_place_above((*shell_surface).popup_tree as *mut _, (*shell_surface).tree as *mut _);
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
        if app_id.as_deref().map_or(false, |id| id.starts_with("cce-cloud")) {
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

        // Snapshot outputs and windows into plain data, compute the whole
        // frame's plan in policy code, then apply it to the scene graph.
        let mut output_snaps: Vec<crate::policy::arrange::OutputSnapshot> = Vec::new();
        for &output in &active_outputs {
            let wlr_box = (*output).sent.box_layout();
            let non_ex = (*output).layer_shell.scheduled.non_exclusive_area;
            output_snaps.push(crate::policy::arrange::OutputSnapshot {
                layout_box: crate::policy::api::Rect {
                    x: wlr_box.x,
                    y: wlr_box.y,
                    width: wlr_box.width,
                    height: wlr_box.height,
                },
                non_exclusive: crate::policy::api::Rect {
                    x: non_ex.x,
                    y: non_ex.y,
                    width: non_ex.width,
                    height: non_ex.height,
                },
            });
        }

        let mut win_ptrs: Vec<*mut Window> = Vec::new();
        let mut window_snaps: Vec<crate::policy::arrange::WindowSnapshot> = Vec::new();
        for &win_ptr in self.windows.iter() {
            if win_ptr.is_null() || (*win_ptr).closed {
                continue;
            }
            let rule_ssd = if !(*win_ptr).mode_locked {
                self.get_rule_for_window(win_ptr).and_then(|rule| rule.ssd)
            } else {
                None
            };
            window_snaps.push(crate::policy::arrange::WindowSnapshot {
                app_id: (*win_ptr).get_app_id_string(),
                title: (*win_ptr).get_title_string(),
                role: (*win_ptr).role(),
                minimized: (*win_ptr).minimized,
                closing_or_init: matches!((*win_ptr).state, crate::window::WindowState::Closing | crate::window::WindowState::Init),
                mode: self.get_mode_for_window(win_ptr),
                rule_ssd,
                being_moved: self.is_window_being_moved(win_ptr),
                status_edge: (*win_ptr).status_edge,
                is_focused: win_ptr == focused_window,
                box_geom: crate::policy::api::Rect {
                    x: (*win_ptr).box_geom.x,
                    y: (*win_ptr).box_geom.y,
                    width: (*win_ptr).box_geom.width,
                    height: (*win_ptr).box_geom.height,
                },
                min_size: (
                    (*win_ptr).wm_scheduled.dimensions_hint.min_width as i32,
                    (*win_ptr).wm_scheduled.dimensions_hint.min_height as i32,
                ),
                virtual_pos: ((*win_ptr).virtual_x, (*win_ptr).virtual_y),
                active_resize: self.get_active_resize_dimensions(win_ptr),
                ssd: (*win_ptr).wm_requested.ssd,
                decorations_size: (*win_ptr).measure_decorations(),
                was_maximized: (*win_ptr).was_maximized,
                saved_maximized_size: ((*win_ptr).saved_maximized_width, (*win_ptr).saved_maximized_height),
                saved_maximized_virtual: ((*win_ptr).saved_maximized_virtual_x, (*win_ptr).saved_maximized_virtual_y),
            });
            win_ptrs.push(win_ptr);
        }

        let params = crate::policy::arrange::ArrangeParams {
            bar_height: self.layout.bar_height,
            status_hide_mode: self.status_hide_mode,
            hide_mode_preview: self.layout.status_module_hide_mode_preview as i32,
            status_blur: self.layout.status_background_blur > 0.001,
            window_blur: self.layout.window_blur,
            opacity_enabled: self.layout.window_opacity,
            decoration: crate::policy::api::DecorationSpec {
                border_width: self.layout.border_width,
                border_color: crate::policy::api::Rgba(self.layout.border_color),
                corner_radius: self.layout.border_corner_radius,
            },
            border_color_focused: crate::policy::api::Rgba(self.layout.border_color_focused),
            overlay: crate::policy::arrange::OverlayParams {
                overlay_width: self.layout.overlay_width,
                border_gap: self.layout.overlay_border_gap,
                border_width: self.layout.border_width,
                position_right: self.layout.overlay_position == "right",
                cloud_position_default: self.layout.cloud_position_default,
            },
            normal: crate::policy::arrange::NormalParams {
                gap_right: self.layout.gap_right,
                gap_top: self.layout.gap_top,
                border_width: self.layout.border_width,
                cloud_position_default: self.layout.cloud_position_default,
                desktop_grid_scale: self.layout.desktop_grid_scale,
                desktop_gap_width: self.layout.desktop_gap_width as f64,
                desktop_cell_inset: self.layout.desktop_cell_fade_inset as f64,
            },
            pan_x: self.desk_pan_x,
            pan_y: self.desk_pan_y,
            zoom: self.desk_zoom,
        };

        let plan = crate::policy::arrange::arrange(&window_snaps, &output_snaps, &params);

        for &output in &active_outputs {
            if !(*output).background_rect.is_null() {
                ffi::wlr_scene_node_set_enabled((*output).background_rect as *mut ffi::wlr_scene_node, plan.background_rect_enabled);
            }
        }

        for (&win_ptr, wp) in win_ptrs.iter().zip(plan.windows.iter()) {
            if let Some(enabled) = wp.scene_enabled {
                ffi::wlr_scene_node_set_enabled((*win_ptr).tree as *mut ffi::wlr_scene_node, enabled);
            }
            if let Some(hidden) = wp.hidden {
                (*win_ptr).rendering_requested.hidden = hidden;
            }
            if let Some(mode) = wp.tiling_mode {
                (*win_ptr).tiling_mode = mode;
            }
            if let Some(tiled) = wp.tiled {
                (*win_ptr).wm_requested.tiled = tiled;
            }
            if let Some(ssd) = wp.ssd {
                (*win_ptr).wm_requested.ssd = ssd;
            }
            if let Some(scale) = wp.scale {
                (*win_ptr).scale = scale;
            }
            if let Some((x, y)) = wp.pos {
                (*win_ptr).rendering_requested.x = x;
                (*win_ptr).rendering_requested.y = y;
            }
            if let Some(bg) = wp.box_geom {
                (*win_ptr).box_geom.x = bg.x;
                (*win_ptr).box_geom.y = bg.y;
                (*win_ptr).box_geom.width = bg.width;
                (*win_ptr).box_geom.height = bg.height;
            }
            if let Some((vx, vy)) = wp.virtual_pos {
                (*win_ptr).virtual_x = vx;
                (*win_ptr).virtual_y = vy;
            }
            if let Some((width, height)) = wp.size {
                (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions { width, height });
                (*win_ptr).wm_requested.bounds = crate::window::Dimensions { width, height };
            }
            if let Some(dec) = wp.decoration {
                (*win_ptr).rendering_requested.border = crate::window::Border {
                    edges: crate::window::Edges { top: true, bottom: true, left: true, right: true },
                    width: dec.border_width.max(0) as u32,
                    color: dec.border_color.0,
                    hover_color: self.layout.border_color_hover,
                    corner_radius: dec.corner_radius.max(0),
                };
            }
            if let Some(blur) = wp.blur {
                (*win_ptr).rendering_requested.blur = blur;
            }
            if let Some(opacity) = wp.opacity {
                (*win_ptr).rendering_requested.opacity = opacity;
            }
            if let Some(((width, height), (vx, vy))) = wp.saved_maximized {
                (*win_ptr).saved_maximized_width = width;
                (*win_ptr).saved_maximized_height = height;
                (*win_ptr).saved_maximized_virtual_x = vx;
                (*win_ptr).saved_maximized_virtual_y = vy;
            }
            if let Some(was_maximized) = wp.was_maximized {
                (*win_ptr).was_maximized = was_maximized;
            }
        }

        // Force configure for all status bar windows so they receive the new geometry immediately
        for &win_ptr in self.windows.iter() {
            if !win_ptr.is_null() && !(*win_ptr).closed && (*win_ptr).is_status_bar() {
                if (*win_ptr).wm_requested.dimensions.is_some() {
                    (*win_ptr).manage_finish();
                }
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

    pub unsafe fn update_viewport_local(&mut self) {
        let zoom_changed = self.desk_zoom != self.last_viewport_zoom;
        let pan_changed = self.desk_pan_x != self.last_viewport_pan_x || self.desk_pan_y != self.last_viewport_pan_y;
        let active = zoom_changed || pan_changed;

        self.last_viewport_zoom = self.desk_zoom;
        self.last_viewport_pan_x = self.desk_pan_x;
        self.last_viewport_pan_y = self.desk_pan_y;

        let was_active = self.viewport_is_active;
        self.viewport_is_active = active;

        self.arrange_views();
        // Clear rendering dirty flag so we don't trigger the idle callback's IPC handshake
        self.rendering_scheduled.dirty = false;
        self.remove_dirty_idle();

        if active {
            for &window in self.windows.iter() {
                if !window.is_null() {
                    (*window).render_viewport_update();
                }
            }
        } else if was_active {
            for &window in self.windows.iter() {
                if !window.is_null() {
                    (*window).render_finish();
                }
            }
        } else {
            for &window in self.windows.iter() {
                if !window.is_null() {
                    (*window).render_viewport_update();
                }
            }
        }

        // Commit outputs or schedule frame updates
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs_list).next;
        while curr != outputs_list {
            let next = (*curr).next;
            let output = crate::container_of!(curr, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                ffi::wlr_output_schedule_frame((*output).wlr_output);
            }
            curr = next;
        }

        // Re-evaluate cursor focus/hover since windows have moved relative to pointers
        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats).next;
        while curr_seat != seats {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            (*seat).cursor.update_hovered();
            curr_seat = next_seat;
        }
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
                let is_wallpaper = app_id.as_deref() == Some("cce-wallpaper");
                if !is_status_bar && !is_wallpaper {
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
                    let is_wallpaper = app_id.as_deref() == Some("cce-wallpaper");
                    if !is_status_bar && !is_wallpaper {
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

    /// True for windows that should appear in the window switcher: mapped,
    /// non-closed, and not one of the desktop-shell surfaces (status bar,
    /// wallpaper, or the switcher's own cce-cloud overlay).
    unsafe fn is_switchable_window(&self, w: *mut Window) -> bool {
        if w.is_null() || (*w).closed {
            return false;
        }
        if !matches!((*w).state, crate::window::WindowState::Mapped) {
            return false;
        }
        match (*w).get_app_id_string().as_deref() {
            Some(id) if id.starts_with("cce-status") => false,
            Some("cce-wallpaper") | Some("cce-cloud") => false,
            _ => true,
        }
    }

    /// Open the alt-tab window switcher: spawn a `cce-cloud --switcher` overlay,
    /// feed it the currently switchable windows in most-recently-used order, and
    /// hand off to a background thread that focuses whatever the user commits to.
    ///
    /// cce-cloud is a `Layer::Overlay` surface with exclusive keyboard focus, so
    /// once it is up it drives the interaction itself (Tab cycles, Super release
    /// commits, Escape cancels) and prints the chosen entry to stdout. We only
    /// build the list and map the committed entry back to a window id.
    pub unsafe fn launch_window_switcher(&mut self) {
        // Most-recently-used order (focus_history is MRU-front). The focused
        // window lands first, so cce-cloud auto-selects index 1 — the previously
        // focused window — which is the classic alt-tab default.
        let mut ordered: Vec<*mut Window> = Vec::new();
        for &w in self.focus_history.iter() {
            if self.is_switchable_window(w) && !ordered.contains(&w) {
                ordered.push(w);
            }
        }
        for &w in self.windows.iter() {
            if self.is_switchable_window(w) && !ordered.contains(&w) {
                ordered.push(w);
            }
        }
        if ordered.is_empty() {
            return;
        }

        // cce-cloud echoes the committed entry back verbatim, so keep the mapping
        // from display string to window id to resolve the selection.
        let mut items: Vec<(String, String)> = Vec::new();
        let mut input = String::new();
        for &w in &ordered {
            let id = (*w).ref_key.index.to_string();
            let app_id = (*w).get_app_id_string().unwrap_or_default();
            let title = (*w).get_title_string().unwrap_or_default();
            let display = if title.is_empty() {
                app_id.clone()
            } else {
                format!("{} ({})", title, app_id)
            };
            input.push_str(&display);
            input.push('\n');
            items.push((id, display));
        }

        // Position near the top-centre of the enabled output. Passing explicit
        // -x/-y keeps cce-cloud a layer-shell overlay (omitting both would make
        // it an XDG toplevel, which would not grab keyboard the same way).
        let (mut vp_w, mut origin_x, mut origin_y) = (1920.0_f64, 0i32, 0i32);
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let wlr_box = (*output).sent.box_layout();
                vp_w = wlr_box.width as f64;
                origin_x = wlr_box.x;
                origin_y = wlr_box.y;
                break;
            }
            curr_out = (*curr_out).next;
        }
        // cce-cloud's default logical width is 600; centre it horizontally.
        let x_pos = origin_x + (((vp_w - 600.0) / 2.0).max(0.0)) as i32;
        let y_pos = origin_y + 80;

        let display_env = std::env::var("WAYLAND_DISPLAY").ok();
        std::thread::spawn(move || {
            run_window_switcher(input, items, x_pos, y_pos, display_env);
        });
    }

    pub unsafe fn keep_status_bar_on_top(&mut self) {
        let mut status_bar_windows = Vec::new();
        for &win_ptr in self.windows.iter() {
            if win_ptr.is_null() || (*win_ptr).closed {
                continue;
            }
            if let Some(app_id) = (*win_ptr).get_app_id_string() {
                if app_id.starts_with("cce-status") {
                    if matches!((*win_ptr).state, crate::window::WindowState::Mapped) {
                        status_bar_windows.push(win_ptr);
                    }
                }
            }
        }
        for win_ptr in status_bar_windows {
            let node_link = &mut (*win_ptr).node.link as *mut ffi::wl_list as *mut WlList;
            let list_head = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
            if !node_link.is_null() && !list_head.is_null() {
                if (*node_link).next != list_head {
                    if (*win_ptr).is_linked() {
                        crate::server::wl_list_remove_and_reinit(node_link);
                    }
                    let last = (*list_head).prev;
                    if !last.is_null() {
                        crate::server::wl_list_insert(last, node_link);
                    }
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
        if !node_link.is_null() && !list_head.is_null() {
            if (*node_link).next != list_head {
                if (*window).is_linked() {
                    crate::server::wl_list_remove_and_reinit(node_link);
                }
                let last = (*list_head).prev;
                if !last.is_null() {
                    crate::server::wl_list_insert(last, node_link);
                }
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
            Action::WindowSwitcher => {
                self.launch_window_switcher();
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
                self.start_clean_exit();
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
                if matches!(self.state, WindowManagerState::Idle) {
                    self.update_viewport_local();
                } else {
                    self.dirty_windowing();
                }
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
                    let mut phys_x = 0.0;
                    let mut phys_y = 0.0;
                    let mut output_found = false;

                    if let Some(seat) = self.first_seat() {
                        let lx = (*seat).cursor.x();
                        let ly = (*seat).cursor.y();
                        let wlr_output = (*self.server).om.output_at(lx, ly);
                        if !wlr_output.is_null() {
                            let mut output_box = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
                            ffi::wlr_output_layout_get_box((*self.server).om.output_layout, wlr_output, &mut output_box);
                            viewport_w = output_box.width as f64;
                            viewport_h = output_box.height as f64;
                            phys_x = output_box.x as f64;
                            phys_y = output_box.y as f64;
                            output_found = true;
                        }
                    }

                    if !output_found {
                        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                        let mut curr_out = (*outputs_list).next;
                        while curr_out != outputs_list {
                            let output = crate::container_of!(curr_out, crate::output::Output, link);
                            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                                let wlr_box = (*output).sent.box_layout();
                                viewport_w = wlr_box.width as f64;
                                viewport_h = wlr_box.height as f64;
                                phys_x = wlr_box.x as f64;
                                phys_y = wlr_box.y as f64;
                                break;
                            }
                            curr_out = (*curr_out).next;
                        }
                    }

                    if let Some(seat) = self.first_seat() {
                        let lx = (*seat).cursor.x();
                        let ly = (*seat).cursor.y();

                        let mut hovered_win: *mut crate::window::Window = std::ptr::null_mut();
                        if let Some(result) = (*self.server).scene.at(lx, ly) {
                            if let crate::scene_node_data::SceneNodeDataVal::Window(window) = result.data {
                                hovered_win = window;
                            }
                        }

                        if !hovered_win.is_null() && !(*hovered_win).is_status_bar() && !(*hovered_win).is_wallpaper() {
                            (*seat).focus(crate::seat::Focus::Window(hovered_win));
                            if !(*seat).object.is_null() && !(*hovered_win).object.is_null() {
                                ffi::wl_resource_post_event((*seat).object, 4, (*hovered_win).object);
                            }

                            let win_w = if (*hovered_win).box_geom.width > 0 { (*hovered_win).box_geom.width as f64 } else { 800.0 };
                            let win_h = if (*hovered_win).box_geom.height > 0 { (*hovered_win).box_geom.height as f64 } else { 600.0 };
                            let center_x = (*hovered_win).virtual_x + win_w / 2.0;
                            let center_y = (*hovered_win).virtual_y + win_h / 2.0;
                            self.desk_zoom = 1.0;
                            self.mode = WindowManagerMode::Normal;
                            self.desk_pan_x = center_x - viewport_w / 2.0;
                            self.desk_pan_y = center_y - viewport_h / 2.0;
                            self.stop_panning_animation();
                            if matches!(self.state, WindowManagerState::Idle) {
                                self.update_viewport_local();
                            } else {
                                self.dirty_windowing();
                            }
                            return;
                        } else {
                            let vx = self.desk_pan_x + (lx - phys_x) / self.desk_zoom;
                            let vy = self.desk_pan_y + (ly - phys_y) / self.desk_zoom;
                            self.desk_zoom = 1.0;
                            self.mode = WindowManagerMode::Normal;
                            self.desk_pan_x = vx - viewport_w / 2.0;
                            self.desk_pan_y = vy - viewport_h / 2.0;
                            self.stop_panning_animation();
                            if matches!(self.state, WindowManagerState::Idle) {
                                self.update_viewport_local();
                            } else {
                                self.dirty_windowing();
                            }
                            return;
                        }
                    }

                    self.desk_zoom = 1.0;
                    self.mode = WindowManagerMode::Normal;
                    self.desk_pan_x = 0.0;
                    self.desk_pan_y = 0.0;
                    self.stop_panning_animation();
                    if matches!(self.state, WindowManagerState::Idle) {
                        self.update_viewport_local();
                    } else {
                        self.dirty_windowing();
                    }
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
                        let is_wallpaper = app_id.as_deref() == Some("cce-wallpaper");
                        if is_status_bar || is_wallpaper {
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
                        if matches!(self.state, WindowManagerState::Idle) {
                            self.update_viewport_local();
                        } else {
                            self.dirty_windowing();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Resolve a window by a query string: an exact numeric window id first,
    /// otherwise a case-insensitive app_id match (exact beats substring).
    /// Returns null if nothing mapped matches. Shared by focus-window /
    /// center-window.
    pub unsafe fn find_window_by_query(&self, query: &str) -> *mut Window {
        let query = query.to_lowercase();

        if let Ok(id) = query.parse::<u32>() {
            for &w in self.windows.iter() {
                if !w.is_null() && !(*w).closed
                    && matches!((*w).state, crate::window::WindowState::Mapped)
                    && (*w).ref_key.index == id
                {
                    return w;
                }
            }
        }

        let mut best_target: *mut Window = std::ptr::null_mut();
        let mut best_score = 0;
        for &w in self.windows.iter() {
            if !w.is_null() && !(*w).closed
                && matches!((*w).state, crate::window::WindowState::Mapped)
            {
                if let Some(aid) = (*w).get_app_id_string() {
                    let aid_lower = aid.to_lowercase();
                    let score = if aid_lower == query {
                        100
                    } else if aid_lower.contains(&query) {
                        50
                    } else {
                        0
                    };
                    if score > best_score {
                        best_score = score;
                        best_target = w;
                    }
                }
            }
        }
        best_target
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
            "status-hide-mode" => {
                let enable = if parts.len() >= 2 {
                    match parts[1] {
                        "true" | "on" | "enable" | "1" => true,
                        "false" | "off" | "disable" | "0" => false,
                        _ => !self.status_hide_mode,
                    }
                } else {
                    !self.status_hide_mode
                };
                self.status_hide_mode = enable;
                self.dirty_windowing();
                return format!("ok {}\n", enable);
            }
            "adjust-position-mode" => {
                if parts.get(1).copied() == Some("query") {
                    return format!("ok {}\n", self.adjust_position_mode);
                }
                let enable = if parts.len() >= 2 {
                    match parts[1] {
                        "true" | "on" | "enable" | "1" => true,
                        "false" | "off" | "disable" | "0" => false,
                        _ => !self.adjust_position_mode,
                    }
                } else {
                    !self.adjust_position_mode
                };
                self.adjust_position_mode = enable;
                if enable {
                    let _ = std::fs::File::create("/tmp/cce-status-interface-adjust-mode");
                } else {
                    let _ = std::fs::remove_file("/tmp/cce-status-interface-adjust-mode");
                }
                self.dirty_windowing();
                return format!("ok {}\n", enable);
            }
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
                    if matches!(self.state, WindowManagerState::Idle) {
                        self.update_viewport_local();
                    } else {
                        self.dirty_windowing();
                    }
                    return "ok\n".to_string();
                }
                "error: invalid dx or dy\n".to_string()
            }
            "pan-to" => {
                if parts.len() < 3 { return "error: missing x or y\n".to_string(); }
                if let (Ok(x), Ok(y)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    self.desk_pan_x = x;
                    self.desk_pan_y = y;
                    if matches!(self.state, WindowManagerState::Idle) {
                        self.update_viewport_local();
                    } else {
                        self.dirty_windowing();
                    }
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
            "zoom-reset" => {
                self.execute_action(&crate::config::Action::ZoomReset, None);
                "ok\n".to_string()
            }
            "pan-left" => {
                self.execute_action(&crate::config::Action::PanLeft, None);
                "ok\n".to_string()
            }
            "pan-right" => {
                self.execute_action(&crate::config::Action::PanRight, None);
                "ok\n".to_string()
            }
            "pan-up" => {
                self.execute_action(&crate::config::Action::PanUp, None);
                "ok\n".to_string()
            }
            "pan-down" => {
                self.execute_action(&crate::config::Action::PanDown, None);
                "ok\n".to_string()
            }
            "overlay-left" => {
                self.execute_action(&crate::config::Action::OverlayLeft, None);
                "ok\n".to_string()
            }
            "overlay-right" => {
                self.execute_action(&crate::config::Action::OverlayRight, None);
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
            "focus-prev" => {
                self.execute_action(&crate::config::Action::FocusPrev, None);
                "ok\n".to_string()
            }
            "window-switcher" => {
                self.execute_action(&crate::config::Action::WindowSwitcher, None);
                "ok\n".to_string()
            }
            "fullscreen" => {
                self.execute_action(&crate::config::Action::Fullscreen, None);
                "ok\n".to_string()
            }
            "mode-next" => {
                self.execute_action(&crate::config::Action::ModeNext, None);
                "ok\n".to_string()
            }
            "mode-next-shared" => {
                self.execute_action(&crate::config::Action::ModeNextShared, None);
                "ok\n".to_string()
            }
            "focus-window" => {
                if parts.len() < 2 { return "error: missing app_id/id\n".to_string(); }
                if let Some(seat) = self.first_seat() {
                    let best_target = self.find_window_by_query(&parts[1..].join(" "));
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
            "center-window" | "bring-window" => {
                // Pan the desktop so the target window (given app_id/id, or the
                // focused window if omitted) is centered in the output, then focus
                // and raise it. Replies with the window's resulting on-screen box so
                // the caller can screenshot it directly (grim -g "X,Y WxH").
                let target: *mut Window = if parts.len() >= 2 {
                    self.find_window_by_query(&parts[1..].join(" "))
                } else if let Some(seat) = self.first_seat() {
                    match (*seat).focused {
                        crate::seat::Focus::Window(w) => w,
                        _ => std::ptr::null_mut(),
                    }
                } else {
                    std::ptr::null_mut()
                };

                if target.is_null() {
                    return "error: window not found\n".to_string();
                }
                if (*target).minimized {
                    (*target).minimized = false;
                }

                // Enabled output's origin + size (fall back to 1920x1080 @ 0,0).
                let (mut vp_w, mut vp_h) = (1920.0_f64, 1080.0_f64);
                let (mut phys_x, mut phys_y) = (0i32, 0i32);
                let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                let mut curr_out = (*outputs_list).next;
                while curr_out != outputs_list {
                    let output = crate::container_of!(curr_out, crate::output::Output, link);
                    if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                        let wlr_box = (*output).sent.box_layout();
                        vp_w = wlr_box.width as f64;
                        vp_h = wlr_box.height as f64;
                        phys_x = wlr_box.x;
                        phys_y = wlr_box.y;
                        break;
                    }
                    curr_out = (*curr_out).next;
                }

                let w = if (*target).box_geom.width > 0 { (*target).box_geom.width as f64 } else { 800.0 };
                let h = if (*target).box_geom.height > 0 { (*target).box_geom.height as f64 } else { 600.0 };

                // Center the window's virtual center in the viewport. Sets desk_pan
                // directly (no animation) so the reply geometry is immediately valid.
                let center_x = (*target).virtual_x + w / 2.0;
                let center_y = (*target).virtual_y + h / 2.0;
                self.desk_pan_x = center_x - (vp_w / 2.0) / self.desk_zoom;
                self.desk_pan_y = center_y - (vp_h / 2.0) / self.desk_zoom;

                if let Some(seat) = self.first_seat() {
                    (*seat).focus(crate::seat::Focus::Window(target));
                }
                self.raise_window(target);
                self.dirty_windowing();

                // screen = output_origin + (virtual - desk_pan) * zoom
                let screen_x = phys_x as f64 + ((*target).virtual_x - self.desk_pan_x) * self.desk_zoom;
                let screen_y = phys_y as f64 + ((*target).virtual_y - self.desk_pan_y) * self.desk_zoom;
                format!(
                    "ok x={} y={} w={} h={}\n",
                    screen_x.round() as i32,
                    screen_y.round() as i32,
                    (w * self.desk_zoom).round() as i32,
                    (h * self.desk_zoom).round() as i32,
                )
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
                // `windows --json` emits one JSON object per line; titles and
                // app_ids are then properly escaped, unlike the text format.
                let as_json = parts.get(1).copied() == Some("--json");
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
                        if as_json {
                            out.push_str(&serde_json::json!({
                                "id": (*w).ref_key.index,
                                "app_id": app_id,
                                "title": title,
                                "mode": (*w).tiling_mode.as_str(),
                                "x": (*w).box_geom.x,
                                "y": (*w).box_geom.y,
                                "w": (*w).box_geom.width,
                                "h": (*w).box_geom.height,
                                "vx": (*w).virtual_x,
                                "vy": (*w).virtual_y,
                                "minimized": (*w).minimized,
                                "has_parent": (*w).has_parent,
                                "focused": w == focused_window,
                                "ssd": (*w).wm_requested.ssd,
                            }).to_string());
                            out.push('\n');
                        } else {
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
                    "desktop_gap_color" => {
                        self.layout.desktop_gap_color = val.to_string();
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
                    "desktop_cell_color" => {
                        self.layout.desktop_cell_color = crate::config::parse_hex_color_rgba(val);
                    }
                    "desktop_grid_scale" => {
                        if let Ok(v) = val.parse::<f64>() {
                            self.layout.desktop_grid_scale = v;
                        }
                    }
                    "desktop_gap_width" => {
                        if let Ok(v) = val.parse::<i32>() {
                            self.layout.desktop_gap_width = v;
                        }
                    }
                    "desktop_cell_corner_radius" => {
                        if let Ok(v) = val.parse::<i32>() {
                            self.layout.desktop_cell_corner_radius = v;
                        }
                    }
                    "desktop_cell_fade_inset" => {
                        if let Ok(v) = val.parse::<i64>() {
                            self.layout.desktop_cell_fade_inset = v;
                        }
                    }
                    "desktop_grid_fade_mode" => {
                        self.layout.desktop_grid_fade_mode = val.to_string();
                    }
                    "desktop_enable_solid_color" => {
                        if let Ok(v) = val.parse::<bool>() {
                            self.layout.desktop_enable_solid_color = v;
                        }
                    }
                    "desktop_solid_color" => {
                        self.layout.desktop_solid_color = crate::config::parse_hex_color_rgba(val);
                    }
                    "gap" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap = v; } }
                    "gap_top" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_top = v; } }
                    "gap_left" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_left = v; } }
                    "gap_right" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_right = v; } }
                    "gap_bottom" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_bottom = v; } }
                    "offset" | "cascade_offset" => { if let Ok(v) = val.parse::<i32>() { self.layout.cascade_offset = v; } }
                    "grid_gap" => { if let Ok(v) = val.parse::<i32>() { self.layout.grid_gap = v; } }
                    "transition_duration" => { if let Ok(v) = val.parse::<i32>() { self.layout.transition_duration = v; } }
                    "bar_height" => { if let Ok(v) = val.parse::<i32>() { self.layout.bar_height = v; } }

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
            // ── Synthetic pointer input (`ccectl pointer-*`): full wlrctl replacement
            // plus held buttons. Injection runs through the real cursor handlers
            // (`Cursor::inject_*`), so grabs/ops/focus behave exactly as with hardware.
            "pointer-move-to" => {
                if parts.len() < 3 { return "error: usage: pointer-move-to <x> <y>\n".to_string(); }
                if let (Ok(x), Ok(y)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    self.for_each_cursor(|cursor| cursor.inject_motion_to(x, y));
                    "ok\n".to_string()
                } else {
                    "error: invalid x or y\n".to_string()
                }
            }
            "pointer-move-by" => {
                if parts.len() < 3 { return "error: usage: pointer-move-by <dx> <dy>\n".to_string(); }
                if let (Ok(dx), Ok(dy)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    self.for_each_cursor(|cursor| cursor.inject_motion_by(dx, dy));
                    "ok\n".to_string()
                } else {
                    "error: invalid dx or dy\n".to_string()
                }
            }
            "pointer-press" | "pointer-release" | "pointer-click" => {
                let button = match Self::parse_pointer_button(parts.get(1).copied()) {
                    Some(b) => b,
                    None => return "error: unknown button (left|right|middle|back|forward or an evdev code)\n".to_string(),
                };
                match action {
                    "pointer-press" => self.for_each_cursor(|cursor| cursor.inject_button(button, true)),
                    "pointer-release" => self.for_each_cursor(|cursor| cursor.inject_button(button, false)),
                    _ => self.for_each_cursor(|cursor| {
                        cursor.inject_button(button, true);
                        cursor.inject_button(button, false);
                    }),
                }
                "ok\n".to_string()
            }
            "pointer-scroll" => {
                if parts.len() < 2 { return "error: usage: pointer-scroll <dy> [dx]\n".to_string(); }
                let dy = parts[1].parse::<f64>();
                let dx = parts.get(2).map(|v| v.parse::<f64>()).unwrap_or(Ok(0.0));
                if let (Ok(dy), Ok(dx)) = (dy, dx) {
                    self.for_each_cursor(|cursor| cursor.inject_scroll(dy, dx));
                    "ok\n".to_string()
                } else {
                    "error: invalid dy or dx\n".to_string()
                }
            }
            "pointer-location" => {
                let mut reply = "error: no seat\n".to_string();
                let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
                let curr_seat = (*seats_list).next;
                if curr_seat != seats_list {
                    let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
                    let cursor = &(*seat).cursor;
                    reply = format!("x={} y={}\n", cursor.x(), cursor.y());
                }
                reply
            }
            // One-direction key events (held modifiers/keys); `keycode` is the evdev
            // code. Like `keypress`, this notifies the focused client directly and does
            // not run compositor keybindings. Modifier keycodes additionally update an
            // injected xkb mask and push a modifiers event, so the focused client's xkb
            // state tracks ctrl/shift/alt/super combos exactly as it would from
            // hardware (`wlr_seat_keyboard_notify_key` alone never changes modifier
            // state — that lives on the keyboard device, which injection bypasses).
            "key-down" | "key-up" => {
                if parts.len() < 2 { return "error: usage: key-down|key-up <keycode>\n".to_string(); }
                if let Ok(keycode) = parts[1].parse::<u32>() {
                    let pressed = action == "key-down";
                    let state = if pressed {
                        ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED
                    } else {
                        ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED
                    };
                    // evdev → real xkb modifier name (left/right pairs).
                    let mod_name: Option<&[u8]> = match keycode {
                        42 | 54 => Some(b"Shift\0"),
                        29 | 97 => Some(b"Control\0"),
                        56 | 100 => Some(b"Mod1\0"),
                        125 | 126 => Some(b"Mod4\0"),
                        _ => None,
                    };
                    let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
                    let mut curr_seat = (*seats_list).next;
                    while curr_seat != seats_list {
                        let next_seat = (*curr_seat).next;
                        let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
                        ffi::wlr_seat_keyboard_notify_key((*seat).wlr_seat, crate::util::msec_timestamp(), keycode, state);
                        if let Some(name) = mod_name {
                            let kb = ffi::river_wlr_seat_get_keyboard((*seat).wlr_seat);
                            if !kb.is_null() && !(*kb).keymap.is_null() {
                                let idx = ffi::xkb_keymap_mod_get_index((*kb).keymap, name.as_ptr() as *const _);
                                if idx != ffi::XKB_MOD_INVALID {
                                    let mask = 1u32 << idx;
                                    if pressed {
                                        self.injected_key_mods |= mask;
                                    } else {
                                        self.injected_key_mods &= !mask;
                                    }
                                    // Injected mask OR'd over the device's live state, so a
                                    // real keyboard keeps working mid-injection.
                                    let mut mods = (*kb).modifiers;
                                    mods.depressed |= self.injected_key_mods;
                                    ffi::wlr_seat_keyboard_notify_modifiers((*seat).wlr_seat, &mut mods);
                                }
                            }
                        }
                        curr_seat = next_seat;
                    }
                    "ok\n".to_string()
                } else {
                    "error: invalid keycode\n".to_string()
                }
            }
            "keypress" | "key-press" => {
                if parts.len() < 2 { return "error: usage: keypress <keycode>\n".to_string(); }
                if let Ok(keycode) = parts[1].parse::<u32>() {
                    let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
                    let mut curr_seat = (*seats_list).next;
                    while curr_seat != seats_list {
                        let next_seat = (*curr_seat).next;
                        let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
                        let time = crate::util::msec_timestamp();
                        ffi::wlr_seat_keyboard_notify_key((*seat).wlr_seat, time, keycode, ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED);
                        ffi::wlr_seat_keyboard_notify_key((*seat).wlr_seat, time + 1, keycode, ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED);
                        curr_seat = next_seat;
                    }
                    "ok\n".to_string()
                } else {
                    "error: invalid keycode\n".to_string()
                }
            }
            _ => format!("error: unknown command: {}\n", action),
        }
    }

    /// Run `f` on every seat's cursor (the synthetic-input commands act on all seats,
    /// like the pre-existing pointer-move-to loop did).
    unsafe fn for_each_cursor(&mut self, mut f: impl FnMut(&mut crate::cursor::Cursor)) {
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            f(&mut (*seat).cursor);
            curr_seat = next_seat;
        }
    }

    /// Button-name/evdev-code parsing for the pointer commands; a missing argument
    /// means the left button, like wlrctl.
    fn parse_pointer_button(arg: Option<&str>) -> Option<u32> {
        match arg {
            None | Some("left") => Some(0x110),
            Some("right") => Some(0x111),
            Some("middle") => Some(0x112),
            Some("back") | Some("side") => Some(0x113),
            Some("forward") | Some("extra") => Some(0x114),
            Some(other) => other.parse::<u32>().ok(),
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
        log::info!("spawning startup program: {}", prog.exec);
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

/// Prefer the installed `~/.local/bin/cce-cloud`, falling back to PATH lookup.
fn cce_cloud_cmd() -> String {
    if let Ok(home) = std::env::var("HOME") {
        let path = format!("{}/.local/bin/cce-cloud", home);
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }
    "cce-cloud".to_string()
}

/// Body of the window switcher, run on a detached thread. Spawns cce-cloud in
/// switcher mode, pipes it the item list, waits for the committed selection on
/// stdout, and asks the compositor to focus it via the control socket (so the
/// actual focus change happens on the main thread through the IPC dispatcher).
fn run_window_switcher(
    input: String,
    items: Vec<(String, String)>,
    x_pos: i32,
    y_pos: i32,
    display_env: Option<String>,
) {
    use std::io::{Read, Write};

    let mut child = match std::process::Command::new(cce_cloud_cmd())
        .args([
            "--switcher",
            "-p",
            "Windows:",
            "-x",
            &x_pos.to_string(),
            "-y",
            &y_pos.to_string(),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("window switcher: failed to spawn cce-cloud: {}", e);
            return;
        }
    };

    // Write the item list, then drop stdin so cce-cloud sees EOF on the feed.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
        let _ = stdin.flush();
    }

    let mut selected = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut selected);
    }
    let _ = child.wait();

    let selected = selected.trim();
    if selected.is_empty() {
        return; // cancelled (Escape) or empty selection
    }

    let id = match items.iter().find(|(_, display)| display == selected) {
        Some((id, _)) => id.clone(),
        None => return,
    };

    let sock = match display_env {
        Some(d) => format!("/tmp/cce-{}.sock", d),
        None => "/tmp/cce.sock".to_string(),
    };
    if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&sock) {
        let _ = stream.write_all(format!("focus-window {}\n", id).as_bytes());
        let _ = stream.flush();
        let mut resp = String::new();
        let _ = stream.read_to_string(&mut resp);
    }
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

unsafe extern "C" fn handle_clean_exit_timeout(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    log::info!("Clean exit timeout reached. Forcing display termination.");
    ffi::wl_display_terminate((*(*wm).server).wl_server);
    0
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
    
    if matches!((*wm).state, WindowManagerState::Idle) {
        (*wm).update_viewport_local();
    } else {
        (*wm).dirty_windowing();
    }
    
    if !done {
        if !(*wm).animation_timer.is_null() {
            ffi::wl_event_source_timer_update((*wm).animation_timer, 16);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(invalid_value)]
    fn test_last_window_state_matching() {
        let mut wm = unsafe { std::mem::MaybeUninit::<WindowManager>::zeroed().assume_init() };
        unsafe {
            std::ptr::write(&mut wm.last_window_states, Vec::new());
        }
        
        wm.last_window_states.push(SavedWindowState {
            app_id: "test-app".to_string(),
            title: "My App Window".to_string(),
            tiling_mode: crate::tiling::TilingMode::Floating,
            minimized: false,
            virtual_x: 100.0,
            virtual_y: 200.0,
            scale: 1.0,
            width: 800,
            height: 600,
            cmdline: "test-app".to_string(),
            focused: false,
        });

        unsafe {
            // Test exact match
            let matched = wm.match_last_window_state("test-app", "My App Window");
            assert!(matched.is_some());
            let m = matched.unwrap();
            assert_eq!(m.app_id, "test-app");
            assert_eq!(m.virtual_x, 100.0);
            assert_eq!(m.virtual_y, 200.0);

            // Test fuzzy title match
            let matched_fuzzy = wm.match_last_window_state("test-app", "My App Window*");
            assert!(matched_fuzzy.is_some());

            // Test app_id only match
            let matched_appid = wm.match_last_window_state("test-app", "Different Title");
            assert!(matched_appid.is_some());
            assert_eq!(matched_appid.unwrap().width, 800);

            // Test no match
            let no_match = wm.match_last_window_state("other-app", "My App Window");
            assert!(no_match.is_none());
        }

        std::mem::forget(wm);
    }
}
