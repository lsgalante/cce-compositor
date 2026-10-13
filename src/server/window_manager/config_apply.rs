//! Applying the config: input rules and devices, key repeat, startup programs,
//! re-mapping saved entries and retiling when the grid changes, and
//! `reload_config`. Split out of window_manager.rs on 2026-10-10.

use super::*;

impl WindowManager {
    /// # Safety
    /// Every device registered in `input` has a live `wlr_device`.
    pub unsafe fn apply_input_rules(&mut self, input: &mut crate::input_manager::InputManager) {
        crate::wm_scope!(mut);
        for device in input.devices() {
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
        }
    }

    /// # Safety
    /// As [`apply_key_repeat`](Self::apply_key_repeat); a device's libinput
    /// handle is live as long as its `wlr_device`.
    pub unsafe fn apply_input_config(&mut self, input: &mut crate::input_manager::InputManager) {
        crate::wm_scope!(mut);
        for device in input.devices() {
            if let Some(ref mut libinput) = (*device).libinput {
                libinput.apply_config(&crate::shared::layout().input_config);
            }
        }
        self.apply_key_repeat(input);
    }

    /// Push `input_config`'s repeat rate/delay to every hardware keyboard and
    /// return how many there are. A keyboard regroups on a change (groups are
    /// keyed by repeat info), so one already matching is left alone.
    ///
    /// # Safety
    /// Every device registered in `input` has a live `wlr_device`, and a
    /// hardware keyboard's `destroy_data` is null or its live `Keyboard`.
    pub unsafe fn apply_key_repeat(&mut self, input: &mut crate::input_manager::InputManager) -> usize {
        crate::wm_scope!(mut);
        let (rate, delay) = crate::shared::layout().input_config.repeat_info();
        let mut count = 0;
        for device in input.devices() {
            if !(*device).virtual_device
                && ffi::river_wlr_input_device_get_type((*device).wlr_device) == ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD
            {
                let keyboard = (*device).destroy_data as *mut crate::keyboard::Keyboard;
                if !keyboard.is_null() {
                    count += 1;
                    if (*keyboard).config.repeat_rate != rate || (*keyboard).config.repeat_delay != delay {
                        let seat = (*device).sid.and_then(|sid| self.seats.ptr(sid)).unwrap_or(std::ptr::null_mut());
                        (*keyboard).set_repeat_info(seat, rate, delay);
                    }
                }
            }
        }
        count
    }

    pub fn spawn_startup_program(&mut self, prog: crate::config::StartupConfig) {
        crate::wm_scope!(mut);
        log::info!("spawning startup program: {}", prog.exec);
        let display = crate::shared::handles().xwayland.display_name();
        let env: &[(&str, &str)] = if display.is_empty() { &[] } else { &[("DISPLAY", &display)] };
        match crate::process::spawn_sh(&prog.exec, env) {
            Ok(child) => self.startup_pids.push((prog, child)),
            Err(e) => log::error!("failed to fork child for startup program: {}", e),
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
        let new = crate::shared::layout().snap_params();
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
    pub fn retile_for_grid_change(&mut self, old: crate::policy::snap::SnapParams) {
        crate::wm_scope!(mut);
        let new = crate::shared::layout().snap_params();
        if old.cell_w == new.cell_w
            && old.cell_h == new.cell_h
            && old.gap_width == new.gap_width
            && old.cell_inset == new.cell_inset
        {
            return;
        }
        let op_win = self.seats.values().next().and_then(|s| s.op.as_ref().and_then(|op| op.window));
        for id in self.windows.ids() {
            if Some(id) == op_win {
                continue;
            }
            let Some(w) = self.windows.get(id) else { continue };
            if w.closed || !matches!(w.state, crate::window::WindowState::Mapped) {
                continue;
            }
            if self.get_mode_for_window(w) != crate::tiling::TilingMode::Tiled {
                continue;
            }
            let (nx, ny, nw, nh) = crate::policy::cells::remap_block(
                w.virtual_x,
                w.virtual_y,
                w.box_geom.width as f64,
                w.box_geom.height as f64,
                &old,
                &new,
            );
            let Some(w) = self.windows.get_mut(id) else { continue };
            w.virtual_x = nx;
            w.virtual_y = ny;
            w.box_geom.width = nw.round() as i32;
            w.box_geom.height = nh.round() as i32;
            // The saved floating spot follows like move-window: a later
            // Tiled -> Floating exit restores at the remapped square rather
            // than yanking the window back across the resized grid.
            w.saved_floating_virtual_x = nx;
            w.saved_floating_virtual_y = ny;
        }
    }

    pub fn reload_config(&mut self, sys: &mut crate::ctx::Sys, input: &mut crate::input_manager::InputManager) -> Result<(), String> {
        crate::wm_scope!(mut);
        if let Some(path) = crate::config::default_config_path() {
            let old_sp = crate::shared::layout().snap_params();
            let old_pids = std::mem::take(&mut self.startup_pids);
            match crate::config::parse_config(&path, self, Some((sys, input))) {
                Ok(()) => {
                    // Update scales of existing outputs from the newly loaded config
                    for output in self.outputs.values_mut() {
                        let wlr_output = output.wlr_output().ptr();
                        if !wlr_output.is_null() {
                            let name = output.wlr_output().name();
                            let scale_key = format!("scale_{}", name);
                            let output_scale = self.display.get(&scale_key)
                                .map(|&s| s as f32)
                                .unwrap_or(self.output_scale);
                            if output.scheduled.scale != output_scale {
                                output.scheduled.scale = output_scale;
                            }
                        }
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
