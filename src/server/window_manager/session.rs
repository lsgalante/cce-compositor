//! The session's windows across restarts: loading and saving `state.json`, the
//! restore placeholders shown until a saved window returns, matching a new window to
//! its saved entry, relaunching saved programs, and the clean exit that saves on
//! logout. Split out of window_manager.rs on 2026-10-10.

use super::*;

impl WindowManager {
    pub unsafe fn load_state(&mut self, path: &str) {
        log::info!("Loading state from {}", path);
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(state) = serde_json::from_str::<SavedState>(&content) {
                self.desk_pan_x = state.desk_pan_x;
                self.desk_pan_y = state.desk_pan_y;
                self.desk_zoom = state.desk_zoom;
                self.set_mode(if (state.desk_zoom - 1.0).abs() > 0.001 { WindowManagerMode::Overview } else { WindowManagerMode::Normal });
                self.restore_queue = state.windows;
                self.last_window_states = state.last_window_states;
                // Geometry in the file was measured under the grid it
                // records; if this session's grid differs, put every Tiled
                // entry back on ITS SQUARES now, before placeholders and
                // restores consume the pixels. A pre-field file (grid: None)
                // has nothing to remap from and loads as-is.
                if let Some(g) = state.grid {
                    let current = self.layout.snap_params();
                    if !g.matches(&current) {
                        self.remap_saved_entries(&g.to_params(&current));
                    }
                }
                self.has_restored_focused_window = self.restore_queue.iter().any(|w| w.focused);
                self.restored_focused_window_mapped = false;
                log::info!(
                    "State loaded successfully. {} windows in restore queue, has_restored_focused_window={}.",
                    self.restore_queue.len(),
                    self.has_restored_focused_window
                );
                self.create_restore_placeholders();
            } else {
                log::error!("Failed to parse state JSON from {}", path);
            }
        } else {
            log::info!("State file not found or unreadable at {}, starting with empty state.", path);
        }
    }

    /// Dim frames at every restored window's saved geometry, shown from
    /// login until the real window maps (or a timeout sweeps the leftovers):
    /// the desk isn't a void while slow programs load, and the saved camera
    /// has something to be pointed at. Purely visual — placeholders are
    /// scene rects, not windows; focus logic never sees them.
    pub unsafe fn create_restore_placeholders(&mut self) {
        let parent = (*self.server).scene.layers.wm.raw();
        if parent.is_null() {
            return;
        }
        for entry in &self.restore_queue {
            if entry.width == 0 || entry.height == 0 {
                continue;
            }
            // Nothing is launched for this entry, so no window is coming to
            // replace the plate: it would stand there until the sweep, a
            // minute of frame over empty desk. The entry stays queued, so
            // the app still lands on its saved spot if the user starts it.
            if restore_command(entry).is_none() {
                continue;
            }
            let mut color = if entry.focused {
                self.layout.border_color_focused
            } else {
                self.layout.border_color
            };
            // Dim: scale all channels (premultiplied convention).
            for c in color.iter_mut() {
                *c *= 0.25;
            }
            let rect = ffi::wlr_scene_rect_create(parent, entry.width as i32, entry.height as i32, color.as_ptr());
            if rect.is_null() {
                continue;
            }
            self.restore_placeholders.push(RestorePlaceholder {
                rect: crate::scene_handle::SceneRect::adopt(rect),
                app_id: entry.app_id.clone(),
                title: entry.title.clone(),
                vx: entry.virtual_x,
                vy: entry.virtual_y,
                w: entry.width,
                h: entry.height,
            });
        }
        log::info!(
            "[Restore] {} placeholder(s) for {} queued window(s)",
            self.restore_placeholders.len(),
            self.restore_queue.len()
        );
        if self.restore_placeholders.is_empty() {
            return;
        }
        self.update_restore_placeholders();
        // Sweep leftovers whose programs never came back.
        let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
        self.restore_placeholder_timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_restore_placeholder_timeout),
            self as *mut WindowManager as *mut _,
        );
        if !self.restore_placeholder_timer.is_null() {
            ffi::wl_event_source_timer_update(self.restore_placeholder_timer, 60_000);
        }
    }

    /// The restore placeholder under a layout-space point, as its virtual
    /// rect `(vx, vy, w, h)`. Topmost (latest-created) wins on overlap.
    pub unsafe fn placeholder_at(&self, lx: f64, ly: f64) -> Option<(f64, f64, f64, f64)> {
        if self.restore_placeholders.is_empty() {
            return None;
        }
        let (mut out_x, mut out_y) = (0.0, 0.0);
        let outputs_list = &(*self.server).om.outputs as *const ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let wlr_box = (*output).sent.box_layout();
                out_x = wlr_box.x as f64;
                out_y = wlr_box.y as f64;
                break;
            }
            curr_out = (*curr_out).next;
        }
        let zoom = self.desk_zoom;
        for p in self.restore_placeholders.iter().rev() {
            let x = out_x + (p.vx - self.desk_pan_x) * zoom;
            let y = out_y + (p.vy - self.desk_pan_y) * zoom;
            let w = p.w as f64 * zoom;
            let h = p.h as f64 * zoom;
            if lx >= x && lx < x + w && ly >= y && ly < y + h {
                return Some((p.vx, p.vy, p.w as f64, p.h as f64));
            }
        }
        None
    }

    /// Keep placeholders tracking the camera, same transform as windows.
    pub unsafe fn update_restore_placeholders(&mut self) {
        if self.restore_placeholders.is_empty() {
            return;
        }
        let (mut out_x, mut out_y) = (0, 0);
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
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
        let zoom = self.desk_zoom;
        for p in &self.restore_placeholders {
            let x = out_x + ((p.vx - self.desk_pan_x) * zoom).round() as i32;
            let y = out_y + ((p.vy - self.desk_pan_y) * zoom).round() as i32;
            let (w, h) = ((p.w as f64 * zoom) as i32, (p.h as f64 * zoom) as i32);
            ffi::river_scene_node_set_position_if_changed(p.rect.node(), x, y);
            ffi::river_scene_rect_set_size_if_changed(p.rect.raw(), w, h);
            // Span-widened like the window the placeholder stands in for
            // and the grid cell it sits on; the raw radius read visibly
            // squarer than both at corner_shape > 2.
            let radius = crate::window::widen_corner_radius(
                (self.layout.root_plate_corner_radius as f64 * zoom) as i32, w, h,
            );
            ffi::river_scene_rect_set_corner_radius(p.rect.raw(), radius);
        }
    }

    /// Drop the placeholder claimed by a matched restore entry.
    pub(crate) unsafe fn remove_placeholder_for(&mut self, entry: &SavedWindowState) {
        if let Some(pos) = self
            .restore_placeholders
            .iter()
            .position(|p| p.app_id == entry.app_id && p.title == entry.title)
        {
            // Dropping the placeholder destroys its rect.
            self.restore_placeholders.remove(pos);
        }
        if self.restore_placeholders.is_empty() && !self.restore_placeholder_timer.is_null() {
            ffi::wl_event_source_remove(self.restore_placeholder_timer);
            self.restore_placeholder_timer = std::ptr::null_mut();
        }
    }

    pub unsafe fn clear_restore_placeholders(&mut self) {
        self.restore_placeholders.clear();
        if !self.restore_placeholder_timer.is_null() {
            ffi::wl_event_source_remove(self.restore_placeholder_timer);
            self.restore_placeholder_timer = std::ptr::null_mut();
        }
    }

    pub unsafe fn save_state(&mut self) {
        if self.shutting_down {
            return;
        }
        // A direct save supersedes a scheduled one.
        self.save_state_pending = false;
        if !self.save_state_timer.is_null() {
            ffi::wl_event_source_timer_update(self.save_state_timer, 0);
        }
        let Some(path_str) = crate::config::default_state_path() else {
            log::error!("Could not resolve state file path");
            return;
        };
        let focused_win = self.focused_window();
        let mut saved_wins = Vec::new();
        let mut last_states = self.last_window_states.clone();
        // (app_id, title, program) of each live shy window, whose own
        // entries are scrubbed after the loop — see the skip below.
        let mut shy: Vec<(String, String, String)> = Vec::new();
        let mut live_pids: Vec<i32> = Vec::new();

        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                continue;
            }
            // The grid layer is owned by its systemd unit and anchored by
            // live patches — saving/restoring it would spawn a duplicate
            // and dictate a stale geometry.
            if (*w).is_status_bar() || (*w).is_wallpaper() || (*w).is_grid() {
                continue;
            }
            // A Utility window owns its geometry entirely; saving it would
            // let a later session restore a size over the client's request —
            // the exact bug the mode deletes. (The clean-exit loops below do
            // NOT skip it: it is a real window and must still be closed.)
            if (*w).tiling_mode == crate::tiling::TilingMode::Utility {
                continue;
            }
            // A transient (dialog) belongs to its parent's process: saved, it
            // would carry that process's cmdline and a session restore would
            // spawn the whole app a second time just to place a dialog that
            // no longer exists. `try_restore` skips transients to match.
            if !(*w).get_parent().is_null() {
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
            live_pids.push(pid);
            let mut args = match self.proc_args_cache.get(&pid) {
                Some(args) if pid > 0 => args.clone(),
                _ => {
                    let args = proc_args(pid);
                    if pid > 0 && !args.is_empty() {
                        self.proc_args_cache.insert(pid, args.clone());
                    }
                    args
                }
            };
            // A shy helper (`Window::is_shy`) is never restored — its app
            // places it — so saving it can only do harm: it takes the app's
            // one slot in `last_window_states`, holding a geometry nothing
            // should borrow, and puts a helper process in the session list.
            // Wine's fallback tray window did both, saved at the 1214x689 it
            // had wrongly borrowed from Ubisoft Connect, which left it the
            // only `steam_proton` entry. Skip it, and scrub what it saved
            // before this rule (or before its no-activate state arrived,
            // which can be after map) so a poisoned state file heals.
            // A satellite (`mode_rule over_sibling`) is skipped and scrubbed
            // the same way: it is placed over its sibling, never restored,
            // and saved it would take the app's one `last_window_states`
            // slot — the main window then reopens at a settings window's
            // size.
            if (*w).is_shy() || (*w).satellite {
                if let Some(program) = args.first() {
                    shy.push((app_id.clone(), title.clone(), program.clone()));
                }
                continue;
            }
            // foot only tracks its launch dir, not the shell's current
            // dir, so restore the child shell's cwd via
            // --working-directory. Strip any pre-existing one first so
            // the flag doesn't accumulate across save/restore cycles.
            if app_id == "foot" && !args.is_empty() {
                if let Some(cwd) = foot_shell_cwd(pid) {
                    let mut i = 1;
                    while i < args.len() {
                        if args[i] == "--working-directory" || args[i] == "-D" {
                            args.drain(i..(i + 2).min(args.len()));
                        } else if args[i].starts_with("--working-directory=")
                            || args[i].starts_with("-D")
                        {
                            args.remove(i);
                        } else {
                            i += 1;
                        }
                    }
                    // A plain argv entry: `restore_command` quotes it.
                    args.insert(1.min(args.len()), format!("--working-directory={}", cwd));
                }
            }
            let mut cmdline = args.join(" ");
            if cmdline.is_empty() {
                cmdline = app_id.clone();
            }

            let is_focused = w == focused_win;

            let win_state = SavedWindowState {
                app_id: app_id.clone(),
                title: title.clone(),
                tiling_mode: (*w).tiling_mode,
                minimized: (*w).minimized,
                // Fullscreen, virtual_x/y is the desk spot the window
                // covers (`place_fullscreen_windows`), not where it lives.
                virtual_x: if (*w).was_fullscreen { (*w).saved_virtual_x } else { (*w).virtual_x },
                virtual_y: if (*w).was_fullscreen { (*w).saved_virtual_y } else { (*w).virtual_y },
                scale: (*w).scale,
                width: (*w).box_geom.width as u32,
                height: (*w).box_geom.height as u32,
                cmdline,
                focused: is_focused,
                argv: (!args.is_empty()).then(|| args.clone()),
                fullscreen_at: if (*w).was_fullscreen {
                    Some(((*w).virtual_x, (*w).virtual_y))
                } else {
                    (*w).last_fullscreen_at.or((*w).restore_fullscreen_at)
                },
            };

            saved_wins.push(win_state.clone());

            // `last_window_states` holds one entry per app_id, and an
            // untitled one can never be borrowed (`borrowable`) — writing
            // it would only evict the app's titled entry, as Wine's tray
            // window evicted Ubisoft Connect's.
            if title.is_empty() {
                continue;
            }
            // One slot per app_id AND program: every Proton program is
            // `steam_proton`, so keyed on the app_id alone Trackmania and
            // the Ubisoft Connect it launches from took turns evicting each
            // other, and the game's entry was gone by the time it was next
            // launched — Connect is still up after the game closes.
            if let Some(pos) = last_states.iter().position(|s| last_state_slot(s, &app_id, args.first().map(String::as_str))) {
                last_states[pos] = win_state;
            } else {
                last_states.push(win_state);
            }
        }
        // Scrub entries persisted before the cce-cloud exclusion above, and
        // untitled ones written before the rule above (never borrowable).
        last_states.retain(|s| s.app_id != "cce-cloud" && !s.title.is_empty());
        let saved_by_shy = |s: &SavedWindowState| {
            shy.iter().any(|(app_id, title, program)| {
                s.app_id == *app_id && s.title == *title && saved_by_program(s, program)
            })
        };
        last_states.retain(|s| !saved_by_shy(s));
        self.exit_orphans.retain(|s| !saved_by_shy(s));
        self.last_window_states = last_states;

        // A cancelled logout already closed some windows; keep their entries
        // until an exit succeeds so they are restored next login. An orphan
        // is dropped once a live window is the same app — relaunched. The
        // identity is app_id AND cmdline: an X11 class is as coarse as
        // "python3", and two unrelated apps must not stand in for each other.
        self.exit_orphans.retain(|o| !saved_wins.iter().any(|s| same_app(s, o)));
        saved_wins.extend(self.exit_orphans.iter().cloned());
        self.last_saved_windows = saved_wins.clone();

        let state = SavedState {
            desk_pan_x: self.desk_pan_x,
            desk_pan_y: self.desk_pan_y,
            desk_zoom: self.desk_zoom,
            windows: saved_wins,
            last_window_states: self.last_window_states.clone(),
            // The grid these geometries were measured under, so a later
            // session under a different grid can keep each Tiled entry on
            // its squares (remap_saved_entries) instead of re-deriving the
            // span from stale pixels.
            grid: Some(crate::policy::state::SavedGrid::from_params(&self.layout.snap_params())),
        };
        
        self.proc_args_cache.retain(|pid, _| live_pids.contains(pid));

        // Unchanged is skipped, compared compactly: the pretty form costs
        // more and is only wanted for the file.
        let Ok(compact) = serde_json::to_string(&state) else { return };
        if self.last_saved_state_json.as_deref() == Some(compact.as_str()) {
            return;
        }
        let Ok(json_str) = serde_json::to_string_pretty(&state) else { return };
        log::debug!("Saving state to {}", path_str);
        let path = std::path::Path::new(&path_str);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Whole or not at all: a crash mid-write used to leave a truncated
        // state.json, which the next login could not restore from.
        let tmp = path.with_extension("json.tmp");
        match std::fs::write(&tmp, &json_str).and_then(|()| std::fs::rename(&tmp, path)) {
            // Only remember it once it is actually on disk, so a failed
            // write is retried on the next transaction rather than latched.
            Ok(()) => self.last_saved_state_json = Some(compact),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
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

        // Windows that are still here when this fires are being held by
        // something — nearly always an app asking whether to save — and the
        // timeout CANCELS the logout rather than forcing the display down
        // over the prompt. Long enough for the user to answer the prompt in
        // place; a quick answer completes the logout with no cancel at all.
        ffi::wl_event_source_timer_update(self.clean_exit_timer, 10_000);
    }

    /// Abandon a clean exit whose windows did not all close in time.
    ///
    /// What stalls a logout is nearly always an app refusing its close
    /// request to ask about unsaved work — Houdini with a dirty scene. Until
    /// 2026-09-08 the timeout forced the display down at that point, which
    /// is exactly the loss the prompt exists to prevent. So the logout is
    /// cancelled instead, the user is told what held it up, and they log
    /// out again once the prompt is answered; `ccectl exit force` skips the
    /// wait for a client that is hung rather than asking.
    ///
    /// The windows the exit already closed would otherwise vanish from the
    /// next state snapshot (`save_state` runs every transaction), and with
    /// them from the next login: they are remembered as `exit_orphans`.
    pub unsafe fn cancel_clean_exit(&mut self) {
        if !self.clean_exit_in_progress {
            return;
        }
        self.clean_exit_in_progress = false;
        self.shutting_down = false;
        if !self.clean_exit_timer.is_null() {
            ffi::wl_event_source_timer_update(self.clean_exit_timer, 0);
        }

        let mut remaining: Vec<String> = Vec::new();
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                continue;
            }
            if (*w).is_status_bar() || (*w).is_wallpaper() {
                continue;
            }
            // A later self-initiated close must not read as a crash.
            (*w).close_requested = false;
            let app_id = (*w).get_app_id_string().unwrap_or_default();
            if (*w).get_parent().is_null() && !app_id.is_empty() {
                let title = (*w).get_title_string().unwrap_or_default();
                remaining.push(if title.is_empty() { app_id.clone() } else { title });
            }
        }
        // The snapshot written as the exit began lists every window it went
        // on to close; a fresh one lists what survived. The difference is
        // what must be remembered.
        let before = std::mem::take(&mut self.last_saved_windows);
        self.exit_orphans.clear();
        self.save_state();
        let survivors = std::mem::take(&mut self.last_saved_windows);
        self.exit_orphans = before
            .into_iter()
            .filter(|b| !survivors.iter().any(|l| same_app(l, b)))
            .collect();
        // Write the merged snapshot now rather than on the next transaction.
        self.save_state();

        // A restart-compositor that stalled must not leave its flag behind
        // for the next plain logout to act on.
        let user = std::env::var("USER").unwrap_or_else(|_| format!("uid{}", libc::getuid()));
        let _ = std::fs::remove_file(cce_core::ipc::ctl::restart_flag(&user));

        let held_by = remaining.join(", ");
        log::info!(
            "Clean exit cancelled: {} window(s) still open ({}); {} closed window(s) remembered for the next login",
            remaining.len(), held_by, self.exit_orphans.len()
        );
        let _ = std::process::Command::new("notify-send")
            .arg("cce")
            .arg(format!(
                "Logout cancelled — still open: {}\nAnswer any save prompt, then log out again.",
                held_by
            ))
            .spawn();
        self.dirty_windowing();
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

    /// `program` is the asking window's argv[0] (`proc_args`), `None` when
    /// unknown; it gates only the app_id-only pass (see `same_program`).
    pub unsafe fn match_and_remove_restore_state(
        &mut self,
        app_id: &str,
        title: &str,
        program: Option<&str>,
    ) -> Option<SavedWindowState> {
        if app_id.is_empty() {
            return None;
        }
        // First pass: Exact match (app_id AND title). An empty title is no
        // identity (`titles_match`); it falls through to the app_id-only pass.
        if let Some(pos) = self.restore_queue.iter().position(|w| w.app_id == app_id && titles_match(title, &w.title)) {
            let entry = self.restore_queue.remove(pos);
            self.remove_placeholder_for(&entry);
            return Some(entry);
        }
        // Second pass: Fuzzy title match (e.g. prefix match, asterisk stripping)
        if let Some(pos) = self.restore_queue.iter().position(|w| w.app_id == app_id && titles_resemble(title, &w.title)) {
            let entry = self.restore_queue.remove(pos);
            self.remove_placeholder_for(&entry);
            return Some(entry);
        }
        // Third pass: app_id only match, from the same program
        if let Some(pos) = self.restore_queue.iter().position(|w| w.app_id == app_id && borrowable(w, program)) {
            let entry = self.restore_queue.remove(pos);
            self.remove_placeholder_for(&entry);
            return Some(entry);
        }
        log_program_veto(self.restore_queue.iter(), app_id, title, program);
        None
    }

    /// Whether a saved entry describes THIS window — same app_id and a
    /// title the first two matcher passes would accept — as opposed to one
    /// the app_id-only pass would merely lend it.
    pub fn has_titled_saved_entry(&self, app_id: &str, title: &str) -> bool {
        self.restore_queue
            .iter()
            .chain(self.last_window_states.iter())
            .any(|w| w.app_id == app_id && (titles_match(title, &w.title) || titles_resemble(title, &w.title)))
    }

    /// `program` as for `match_and_remove_restore_state`.
    pub unsafe fn match_last_window_state(
        &self,
        app_id: &str,
        title: &str,
        program: Option<&str>,
    ) -> Option<SavedWindowState> {
        if app_id.is_empty() {
            return None;
        }
        // First pass: Exact match (app_id AND title), as above
        if let Some(w) = self.last_window_states.iter().find(|w| w.app_id == app_id && titles_match(title, &w.title)) {
            return Some(w.clone());
        }
        // Second pass: Fuzzy title match
        if let Some(w) = self.last_window_states.iter().find(|w| w.app_id == app_id && titles_resemble(title, &w.title)) {
            return Some(w.clone());
        }
        // Third pass: app_id only match, from the same program
        if let Some(w) = self.last_window_states.iter().find(|w| w.app_id == app_id && borrowable(w, program)) {
            return Some(w.clone());
        }
        log_program_veto(self.last_window_states.iter(), app_id, title, program);
        None
    }

    pub unsafe fn spawn_restored_windows(&mut self) {
        log::info!("Spawning restored windows. Total: {}", self.restore_queue.len());
        let restored = self.restore_queue.clone();
        std::thread::spawn(move || {
            // Clients that reach for the Secret Service on startup start last,
            // behind the keyring barrier: launched into a still-locked keyring
            // they either fail outright or quietly fall back to plaintext
            // credential storage. gnome-keyring now comes up already unlocked
            // in ~1.3s, so the wait is short — but it is not zero, and losing
            // that race downgrades an app's storage without saying so.
            // Everything else starts immediately.
            let (secret_gated, immediate): (Vec<_>, Vec<_>) = restored
                .into_iter()
                .partition(|w| Self::needs_secret_service(&w.cmdline));

            let mut spawned_any = false;
            for w in &immediate {
                Self::spawn_restored_one(w, &mut spawned_any);
            }
            if !secret_gated.is_empty() {
                Self::wait_for_secret_service(secret_gated.len());
                for w in &secret_gated {
                    Self::spawn_restored_one(w, &mut spawned_any);
                }
            }
        });
    }

    /// True when this restored command will talk to `org.freedesktop.secrets`
    /// as it starts. Electron names its backend on the command line, and that
    /// is the population that blocks on a locked keyring (`basic` is Electron's
    /// plaintext fallback — it never touches the Secret Service).
    pub(crate) fn needs_secret_service(cmdline: &str) -> bool {
        match cmdline.split("--password-store=").nth(1) {
            Some(rest) => !matches!(rest.split_whitespace().next(), None | Some("basic")),
            None => false,
        }
    }

    /// Block until the Secret Service reports its default collection unlocked.
    ///
    /// Bounded twice over, because a login must never wedge here: if nothing
    /// owns the bus name shortly after startup this machine has no keyring to
    /// wait for, and if the collection simply never unlocks the gated apps
    /// still get launched (degraded, exactly as they were before this barrier
    /// existed) rather than being dropped.
    pub(crate) fn wait_for_secret_service(gated: usize) {
        const NAME_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
        const UNLOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
        const POLL: std::time::Duration = std::time::Duration::from_millis(500);

        let busctl = |args: &[&str]| -> Option<String> {
            let out = std::process::Command::new("busctl").args(args).output().ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        };

        let started = std::time::Instant::now();
        let mut have_name = false;
        while started.elapsed() < NAME_GRACE {
            if busctl(&[
                "--user", "call", "org.freedesktop.DBus", "/org/freedesktop/DBus",
                "org.freedesktop.DBus", "NameHasOwner", "s", "org.freedesktop.secrets",
            ])
            .as_deref()
                == Some("b true")
            {
                have_name = true;
                break;
            }
            std::thread::sleep(POLL);
        }
        if !have_name {
            log::info!(
                "No Secret Service on the bus after {NAME_GRACE:?}; starting {gated} keyring client(s) without waiting"
            );
            return;
        }

        let started = std::time::Instant::now();
        while started.elapsed() < UNLOCK_TIMEOUT {
            match busctl(&[
                "--user", "get-property", "org.freedesktop.secrets",
                "/org/freedesktop/secrets/aliases/default",
                "org.freedesktop.Secret.Collection", "Locked",
            ])
            .as_deref()
            {
                Some("b false") => {
                    log::info!(
                        "Keyring unlocked after {:?}; starting {gated} gated client(s)",
                        started.elapsed()
                    );
                    return;
                }
                _ => std::thread::sleep(POLL),
            }
        }
        log::warn!(
            "Keyring still locked after {UNLOCK_TIMEOUT:?}; starting {gated} gated client(s) anyway"
        );
    }

    pub(crate) fn spawn_restored_one(w: &SavedWindowState, spawned_any: &mut bool) {
        let Some(cmd) = restore_command(w) else {
            if !relaunchable(&w.cmdline) {
                if !w.cmdline.trim().is_empty() {
                    log::info!(
                        "Skipping unrestorable Windows-path command for {:?}: {}",
                        w.app_id,
                        w.cmdline.trim()
                    );
                }
            } else {
                log::warn!(
                    "Not relaunching {:?}: saved without its argv and its command line has \
                     shell characters, which the shell would run; start it once by hand \
                     and the next save records it safely: {}",
                    w.app_id,
                    w.cmdline.trim()
                );
            }
            return;
        };
        // Small stagger so N clients don't all hit Vulkan device
        // init at the same instant; restore matching and focus
        // restoration are map-order independent.
        if *spawned_any {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        *spawned_any = true;
        log::info!("Deferred spawning restored window command: {}", cmd);
        match unsafe { nix::unistd::fork() } {
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

/// Working directory of the shell running inside a foot window.
///
/// foot's own process cwd never follows `cd` — it stays at its launch dir for
/// the window's whole life. The live directory the user is actually in lives in
/// foot's child (the shell it spawned for that window). We read the first child
/// and return its `/proc/<pid>/cwd`. Returns `None` if it can't be read.
pub(crate) fn foot_shell_cwd(foot_pid: i32) -> Option<String> {
    let children =
        std::fs::read_to_string(format!("/proc/{0}/task/{0}/children", foot_pid)).ok()?;
    let child: i32 = children.split_whitespace().next()?.parse().ok()?;
    let cwd = std::fs::read_link(format!("/proc/{}/cwd", child)).ok()?;
    Some(cwd.to_string_lossy().into_owned())
}

/// The bare program name a window should be restored by, when the binary it
/// is running is NOT what its name resolves to on `PATH`.
///
/// An `exec` wrapper leaves no trace in `/proc`: `~/.local/bin/inkscape`
/// (`exec /usr/bin/inkscape "$@"`, the GDK_SCALE fix) shows up in the
/// window's cmdline as `/usr/bin/inkscape`, and a restore that replays that
/// path starts the program without its wrapper. Desktop entries and the
/// launcher run bare names, so the first `PATH` hit for the name IS how the
/// user's environment launches the program. When that hit is a different
/// file from the one running, record the name and let the restore's
/// `sh -c` resolve it the same way. Returns `None` for a relative argv[0],
/// a binary no longer on disk, or a name whose first `PATH` hit is the very
/// same file (through any symlink) — there the absolute path is already the
/// truth, and keeping it means a later `PATH` change cannot redirect it.
pub(crate) fn path_shadowed_name(argv0: &str, path_var: &str) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    if !argv0.starts_with('/') {
        return None;
    }
    let exe = std::path::Path::new(argv0);
    let name = exe.file_name()?.to_str()?;
    let real = std::fs::canonicalize(exe).ok()?;
    for dir in path_var.split(':').filter(|d| !d.is_empty()) {
        let candidate = std::path::Path::new(dir).join(name);
        let Ok(meta) = std::fs::metadata(&candidate) else { continue };
        if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
            continue;
        }
        // First executable hit decides, as `sh` would decide it.
        let candidate_real = std::fs::canonicalize(&candidate).unwrap_or(candidate);
        return if candidate_real == real { None } else { Some(name.to_string()) };
    }
    None
}

/// A process's argv the way `save_state` records it: `/proc/<pid>/cmdline`
/// split on NUL, an AppImage's throwaway `/tmp/.mount_*` path swapped for
/// the `APPIMAGE` it was launched from, and argv[0] reduced to its bare name
/// when `PATH` resolves that name to a different file (`path_shadowed_name`).
/// Empty when the pid is unknown or the process is gone.
pub(crate) fn proc_args(pid: i32) -> Vec<String> {
    if pid <= 0 {
        return Vec::new();
    }
    let proc_cmdline = std::fs::read(format!("/proc/{}/cmdline", pid)).unwrap_or_default();
    if proc_cmdline.is_empty() {
        return Vec::new();
    }
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
    // A wrapper that exec'd the real binary is invisible here;
    // restore by the bare name when PATH says the name is a
    // different file (see path_shadowed_name).
    if !args.is_empty() {
        if let Some(name) =
            path_shadowed_name(&args[0], &std::env::var("PATH").unwrap_or_default())
        {
            args[0] = name;
        }
    }
    args
}

/// Whether `saved` was recorded from a run of `program` — the argv[0]
/// `proc_args` gives for the window that wants to borrow it.
///
/// The app_id-only pass of the state matchers exists for a relaunched main
/// window whose title changed, and an app_id alone cannot tell that apart
/// from a different program that happens to share the class. Every Proton
/// window is `steam_proton`: Wine's fallback system-tray window (owned by
/// the prefix's `explorer.exe /desktop`, untitled, a few icons wide) took
/// Ubisoft Connect's 1214x689 and sat on the desk as a big white window, and
/// Trackmania took the launcher's size the same way. Only a known program
/// vetoes: an unknown pid, or an entry `save_state` could only label with
/// its app_id, matches as before.
pub(crate) fn same_program(saved: &SavedWindowState, program: Option<&str>) -> bool {
    let Some(program) = program.filter(|p| !p.is_empty()) else {
        return true;
    };
    if saved.cmdline.is_empty() || saved.cmdline == saved.app_id {
        return true;
    }
    saved_by_program(saved, program)
}

/// Whether `saved`'s recorded cmdline positively names `program` as its
/// argv[0]. Unlike `same_program`, an entry that recorded no real cmdline
/// is not a match: this one decides what to delete, not what to refuse.
pub(crate) fn saved_by_program(saved: &SavedWindowState, program: &str) -> bool {
    // The saved cmdline is argv joined with spaces, and argv[0] may hold
    // spaces itself (`C:\Program Files (x86)\...`), so compare by prefix
    // rather than by splitting.
    !program.is_empty()
        && saved
            .cmdline
            .strip_prefix(program)
            .map_or(false, |rest| rest.is_empty() || rest.starts_with(' '))
}

/// Whether the app_id-only pass may hand `saved` to a window of `program`:
/// the same program saved it, and it has a title.
///
/// An untitled entry is never borrowed. Untitled on both sides no longer
/// matches in the title passes (`titles_match`), so this pass was the one
/// way left to reach one — and an untitled window is almost always a
/// helper: Wine's tray window was saved at the 1214x689 it had wrongly
/// borrowed, and the same program kept handing it back to itself every
/// session. The cost is an app whose windows never set a title: its
/// relaunch, and its session restore, open where the app puts them.
pub(crate) fn borrowable(saved: &SavedWindowState, program: Option<&str>) -> bool {
    !saved.title.is_empty() && same_program(saved, program)
}

/// Whether `saved` is the `last_window_states` slot a window of `app_id`
/// running `program` writes to: one per app_id and program. An entry or a
/// window with no program to tell by shares the app_id's slot, as every
/// entry did before programs counted (`same_program`'s leniency).
pub(crate) fn last_state_slot(saved: &SavedWindowState, app_id: &str, program: Option<&str>) -> bool {
    saved.app_id == app_id && same_program(saved, program)
}

pub(crate) fn relaunchable(cmdline: &str) -> bool {
    !cmdline.trim().is_empty() && !is_windows_path(cmdline)
}

/// One argument, quoted for `sh -c` so the shell hands it back unchanged:
/// left bare when it is only characters the shell never reinterprets, else
/// single-quoted (with `'` written `'\''`).
pub(crate) fn shell_quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg.bytes().all(|b| b.is_ascii_alphanumeric() || b"@%+=:,./_-".contains(&b));
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// A legacy `cmdline` (a state file from before `argv`) the shell reads
/// exactly as written: words of plain characters and spaces, nothing it
/// expands, substitutes, redirects or chains.
pub(crate) fn plain_cmdline(cmdline: &str) -> bool {
    cmdline.bytes().all(|b| b.is_ascii_alphanumeric() || b" @%+=:,./_-".contains(&b))
}

/// The `sh -c` command that relaunches a saved window, or None when it must
/// not be relaunched.
///
/// Until 2026-10-02 this was `cmdline` itself — argv joined with spaces and
/// run through `/bin/sh -c` at the next login — so an argument's own shell
/// characters were executed: a viewer left open on `~/Downloads/x$(cmd).pdf`
/// ran `cmd`, and a URL with `&` was split into two commands. Now the saved
/// `argv` is quoted argument by argument. An entry saved before `argv`
/// existed is relaunched only when its cmdline is plain; one with shell
/// characters is skipped (its geometry still applies when the app is
/// started by hand, and the next save records its argv).
///
/// Chromium and every Electron app rewrite their own `/proc/<pid>/cmdline`
/// to one space-joined string, so their argv is saved as a single element
/// holding the whole command line. Quoted as one word, that names no file:
/// Claude Desktop and Chrome stopped coming back at login the day argv
/// quoting landed (2026-10-02, `sh: ... No such file or directory`). A
/// lone argv[0] with whitespace that is not an existing file is therefore
/// read as a joined command line, under the same plain-only rule as a
/// legacy entry.
pub(crate) fn restore_command(w: &SavedWindowState) -> Option<String> {
    if !relaunchable(&w.cmdline) {
        return None;
    }
    match w.argv.as_deref() {
        Some([joined]) if is_rewritten_cmdline(joined) => {
            plain_cmdline(joined).then(|| joined.clone())
        }
        Some(argv) if !argv.is_empty() => {
            Some(argv.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" "))
        }
        _ if plain_cmdline(&w.cmdline) => Some(w.cmdline.clone()),
        _ => None,
    }
}

/// Whether a one-element argv is a process's rewritten, space-joined
/// command line rather than a program whose path contains a space.
pub(crate) fn is_rewritten_cmdline(arg0: &str) -> bool {
    arg0.contains(char::is_whitespace) && !std::path::Path::new(arg0).exists()
}

/// Whether `s` starts with a Windows drive path (`C:\` or `C:/`) — the
/// argv[0] every Wine/Proton process rewrites its command line to, which
/// makes it the one reliable sign that a window belongs to Wine: its
/// WM_CLASS is `steam_proton`, `steam_app_N` or the exe's own name.
pub(crate) fn is_windows_path(s: &str) -> bool {
    let b = s.trim().as_bytes();
    b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\')
}

/// Whether a window mapping as (`app_id`, `program`) is the reconnect of
/// one that vanished as (`gone_app_id`, `gone_program`): the same app_id
/// AND the same program. An app_id alone is too coarse — every Proton
/// program is `steam_proton`, so a game launched from Ubisoft Connect,
/// mapping a second after one of the launcher's own windows closed, was
/// held unfocused as the launcher "reconnecting", and the user's
/// fullscreen key went to whatever still had focus (2026-09-26). A
/// program unknown on either side (the process already gone) falls back
/// to the app_id alone, as before.
pub(crate) fn is_reconnect(gone_app_id: &str, gone_program: Option<&str>, app_id: &str, program: Option<&str>) -> bool {
    gone_app_id == app_id
        && match (gone_program, program) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
}

/// The state matchers' exact title pass. Two empty titles do not match: an
/// untitled window has no identity beyond its app_id, so "" == "" was an
/// app_id-only match that skipped `same_program` — Wine's untitled tray
/// window and any other untitled helper of a `steam_proton` app all
/// claimed one another's entries through it.
pub(crate) fn titles_match(a: &str, b: &str) -> bool {
    !a.is_empty() && a == b
}

/// The state matchers' fuzzy title pass: equal once a trailing `*` (an
/// editor's unsaved marker) is stripped, or one a prefix of the other.
/// An empty title resembles nothing — as a prefix it would resemble every
/// title, which made this pass an app_id-only match in disguise and let a
/// title-less window skip `same_program`.
pub(crate) fn titles_resemble(a: &str, b: &str) -> bool {
    let t1 = a.trim_end_matches('*');
    let t2 = b.trim_end_matches('*');
    if t1.is_empty() || t2.is_empty() {
        return false;
    }
    t1 == t2 || t1.starts_with(t2) || t2.starts_with(t1)
}

/// Say so when an app_id had saved entries but `same_program` refused them
/// all — otherwise a window that restores nothing looks like one that had
/// nothing saved. Debug, because `try_restore` runs again on every title
/// change of a window that has not restored.
pub(crate) fn log_program_veto<'a>(
    entries: impl Iterator<Item = &'a SavedWindowState>,
    app_id: &str,
    title: &str,
    program: Option<&str>,
) {
    let others: Vec<&str> = entries
        .filter(|w| w.app_id == app_id && !w.title.is_empty())
        .map(|w| w.cmdline.as_str())
        .collect();
    if !others.is_empty() {
        log::debug!(
            "Not borrowing saved state for app_id={} title={:?}: program {:?} saved none (saved by {:?})",
            app_id, title, program.unwrap_or(""), others
        );
    }
}

/// Two saved entries describe the same app: same app_id and the same
/// command line (an X11 class alone is as coarse as "python3").
pub(crate) fn same_app(a: &SavedWindowState, b: &SavedWindowState) -> bool {
    a.app_id == b.app_id && a.cmdline == b.cmdline
}
