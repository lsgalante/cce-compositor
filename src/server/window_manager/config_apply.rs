//! Applying the config: input rules and devices, key repeat, startup programs,
//! re-mapping saved entries and retiling when the grid changes, and
//! `reload_config`. Split out of window_manager.rs on 2026-10-10.

use super::*;

impl WindowManager {
    pub unsafe fn apply_input_rules(&mut self) {
        crate::wm_scope!(mut);
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
        crate::wm_scope!(mut);
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
        self.apply_key_repeat();
    }

    /// Push `input_config`'s repeat rate/delay to every hardware keyboard and
    /// return how many there are. A keyboard regroups on a change (groups are
    /// keyed by repeat info), so one already matching is left alone.
    pub unsafe fn apply_key_repeat(&mut self) -> usize {
        crate::wm_scope!(mut);
        if self.server.is_null() {
            return 0;
        }
        let (rate, delay) = self.input_config.repeat_info();
        let mut count = 0;
        let devices_head = &mut (*self.server).input_manager.devices as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*devices_head).next;
        while curr != devices_head {
            let next = (*curr).next;
            let device = crate::container_of!(curr, crate::input_device::InputDevice, link);
            if !(*device).virtual_device
                && ffi::river_wlr_input_device_get_type((*device).wlr_device) == ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD
            {
                let keyboard = (*device).destroy_data as *mut crate::keyboard::Keyboard;
                if !keyboard.is_null() {
                    count += 1;
                    if (*keyboard).config.repeat_rate != rate || (*keyboard).config.repeat_delay != delay {
                        (*keyboard).set_repeat_info(rate, delay);
                    }
                }
            }
            curr = next;
        }
        count
    }

    pub unsafe fn spawn_startup_program(&mut self, prog: crate::config::StartupConfig) {
        crate::wm_scope!(mut);
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

    /// Re-tile the SAVED Tiled entries (restore queue + last-window
    /// states) from `old` grid params onto the current grid — the
    /// stateful sibling of `retile_for_grid_change`, which can only reach
    /// mapped windows. Without this, a window closed under one grid and
    /// reopened under another restores misaligned pixels, touches extra
    /// cells, and the tiled snap grows it by a cell.
    pub fn remap_saved_entries(&mut self, old: &crate::policy::snap::SnapParams) {
        crate::wm_scope!(mut);
        let new = self.layout.snap_params();
        for entry in self
            .restore_queue
            .iter_mut()
            .chain(self.last_window_states.iter_mut())
        {
            if entry.tiling_mode != crate::tiling::TilingMode::Tiled {
                continue;
            }
            if entry.width == 0 || entry.height == 0 {
                continue;
            }
            let (nx, ny, nw, nh) = crate::policy::cells::remap_block(
                entry.virtual_x,
                entry.virtual_y,
                entry.width as f64,
                entry.height as f64,
                old,
                &new,
            );
            entry.virtual_x = nx;
            entry.virtual_y = ny;
            entry.width = nw.round() as u32;
            entry.height = nh.round() as u32;
        }
    }

    /// Re-tile every Tiled window after a grid-geometry change (cell
    /// sizes, gap, or fade inset): each window keeps its BLOCK of squares
    /// (`cells::remap_block`), so it resizes with the grid instead of
    /// keeping its old pixel box and later spanning whatever new cells that
    /// box happens to touch. A window under an active seat op is left
    /// alone — the op owns its geometry until release. No-op when the
    /// geometry is unchanged.
    pub unsafe fn retile_for_grid_change(&mut self, old: crate::policy::snap::SnapParams) {
        crate::wm_scope!(mut);
        let new = self.layout.snap_params();
        if old.cell_w == new.cell_w
            && old.cell_h == new.cell_h
            && old.gap_width == new.gap_width
            && old.cell_inset == new.cell_inset
        {
            return;
        }
        let op_win = self
            .first_seat()
            .and_then(|s| (*s).op.as_ref().map(|op| op.window_ptr))
            .unwrap_or(std::ptr::null_mut());
        let wins: Vec<*mut crate::window::Window> = self.windows.iter().copied().collect();
        for w in wins {
            if w.is_null() || (*w).closed || w == op_win {
                continue;
            }
            if !matches!((*w).state, crate::window::WindowState::Mapped) {
                continue;
            }
            if self.get_mode_for_window(w) != crate::tiling::TilingMode::Tiled {
                continue;
            }
            let (nx, ny, nw, nh) = crate::policy::cells::remap_block(
                (*w).virtual_x,
                (*w).virtual_y,
                (*w).box_geom.width as f64,
                (*w).box_geom.height as f64,
                &old,
                &new,
            );
            (*w).virtual_x = nx;
            (*w).virtual_y = ny;
            (*w).box_geom.width = nw.round() as i32;
            (*w).box_geom.height = nh.round() as i32;
            // The saved floating spot follows like move-window: a later
            // Tiled -> Floating exit restores at the remapped square rather
            // than yanking the window back across the resized grid.
            (*w).saved_floating_virtual_x = nx;
            (*w).saved_floating_virtual_y = ny;
        }
    }

    pub unsafe fn reload_config(&mut self) -> Result<(), String> {
        crate::wm_scope!(mut);
        if let Some(path) = crate::config::default_config_path() {
            let old_sp = self.layout.snap_params();
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

                    self.retile_for_grid_change(old_sp);
                    // The saved entries hold geometry from before the
                    // reload too — closed windows must reopen on their
                    // squares, not their stale pixels.
                    self.remap_saved_entries(&old_sp);
                    // The grid client reads the same desktop keys and only
                    // repaints when handed a patch: hand it one.
                    self.invalidate_grid_patches();
                    crate::shared::pending().dirty_windowing();

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
