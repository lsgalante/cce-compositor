//! `ccectl restart-app [--force] [app_id|id]`: close every window of the
//! app's process, wait for it to exit, and start it again from the
//! `/proc` snapshot taken first (`crate::app_restart`). The reply is held
//! until the new process is started or the restart gives up, so a script can
//! chain on it.
//!
//! The new process's windows come back as the old ones stood: each closed
//! window's state is kept as a [`RestartEntry`] and `Window::try_restore`
//! takes it ahead of anything in `state.json`.

use super::*;
use crate::app_restart::{PendingRestart, Snapshot, GRACE_MS, KILL_AFTER_TERM_MS, POLL_MS};

/// How long a closed window's state waits for the restarted app to map its
/// window again: the grace and `--force` signals, then the app's own start.
const RESTART_ENTRY_TTL_NS: u64 = 60_000_000_000;

/// A window `restart-app` closed, as it stood: the saved-state record a
/// login would restore from, plus what that record leaves out. It is
/// handed back exactly — no recall into view, no cascade, no geometric
/// promotion, the mode lock as it was — and only to a window of the same
/// app_id and program, once.
pub(crate) struct RestartEntry {
    pub state: SavedWindowState,
    pub mode_locked: bool,
    /// The process that held the window, so a restart that is called off
    /// can take its entries back.
    pub pid: i32,
    pub expires_ns: u64,
}

impl WindowManager {
    pub(crate) fn restart_app_command(&mut self, parts: &[&str]) -> String {
        crate::wm_scope!(mut);
        let mut args = &parts[1..];
        let force = args.first() == Some(&"--force");
        if force {
            args = &args[1..];
        }
        // The effective focused window, not the seat's: with no name this is
        // what the launcher's System page ("Restart Focused App") runs, and
        // while that popup is open it holds the seat focus itself — the seat's
        // window would restart cce-cloud, not the app behind it.
        // `focused_window` skips overlay UI exactly as `close` does.
        let target = if args.is_empty() {
            self.focused_window()
        } else {
            self.find_window_id(&args.join(" "))
        };
        let Some(win) = target.and_then(|id| self.windows.get(id)) else {
            return "error: window not found\n".to_string();
        };
        let pid = win.unreliable_pid();
        if self.app_restarts.iter().any(|r| r.snap.pid == pid) {
            return format!("error: process {} is already restarting\n", pid);
        }
        let snap = match Snapshot::take(pid) {
            Ok(s) => s,
            Err(e) => return format!("error: {}\n", e),
        };
        let now = crate::util::timestamp_ns();
        let mut pending = match PendingRestart::new(snap, None, force, now) {
            Ok(p) => p,
            Err(e) => return format!("error: {}\n", e),
        };

        // Every window of the process, not just the one named: an app with
        // two windows open does not exit when one closes.
        let ids: Vec<crate::window::WindowId> = self
            .windows
            .values()
            .filter(|w| !w.closed && w.unreliable_pid() == pid)
            .map(|w| w.id())
            .collect();
        self.keep_restart_entries(pid, &ids, now);
        for &id in &ids {
            if let Some(w) = self.windows.get_mut(id) {
                w.restart_closing = true;
                w.close();
            }
        }
        log::info!(
            "[ipc] restart-app: {} (pid {}{}) — closed {} window(s), waiting for it to exit",
            pending.snap.name,
            pid,
            pending.snap.unit.as_deref().map(|u| format!(", unit {}", u)).unwrap_or_default(),
            ids.len()
        );

        // Answer once the outcome is known; without a channel (a caller
        // that is not the socket) there is nobody to answer.
        pending.reply = self.pending_ipc_reply.take();
        self.app_restarts.push(pending);
        self.arm_app_restart_timer();
        String::new()
    }

    /// Record each of `ids` (windows of `pid`, about to close) as a
    /// [`RestartEntry`]. The focused one goes first, so a relaunch that
    /// comes back with fewer windows, or retitled, gives the main one its
    /// spot. Skipped like `save_state` skips them, and for the same reason
    /// `try_restore` never asks for them: transients, Utility, shy helpers,
    /// satellites, and the desktop's own surfaces.
    fn keep_restart_entries(&mut self, pid: i32, ids: &[crate::window::WindowId], now: u64) {
        crate::wm_scope!(mut);
        self.restart_entries.retain(|e| e.expires_ns > now && e.pid != pid);
        let argv = proc_args(pid);
        let cmdline = argv.join(" ");
        let focused = self.focused_window();
        let mut ordered: Vec<_> = ids.to_vec();
        ordered.sort_by_key(|&id| Some(id) != focused);
        for id in ordered {
            let Some(w) = self.windows.get(id) else { continue };
            if w.tiling_mode == crate::tiling::TilingMode::Utility
                || w.get_parent().is_some()
                || w.is_shy()
                || w.satellite
                || w.is_status_bar()
                || w.is_wallpaper()
                || w.is_grid()
            {
                continue;
            }
            let Some(app_id) = w.get_app_id_string().filter(|a| !a.is_empty()) else { continue };
            let (virtual_x, virtual_y, fullscreen_at) = saved_spot(w);
            let state = SavedWindowState {
                app_id: app_id.clone(),
                title: w.get_title_string().unwrap_or_default(),
                tiling_mode: w.tiling_mode,
                minimized: w.minimized,
                virtual_x,
                virtual_y,
                scale: w.scale,
                width: w.box_geom.width as u32,
                height: w.box_geom.height as u32,
                cmdline: if cmdline.is_empty() { app_id } else { cmdline.clone() },
                focused: Some(id) == focused,
                argv: (!argv.is_empty()).then(|| argv.clone()),
                fullscreen_at,
                stack: None,
            };
            log::info!(
                "[ipc] restart-app: keeping {:?} ({}) {:?} {}x{} at ({:.0}, {:.0}){}",
                state.title,
                state.app_id,
                state.tiling_mode,
                state.width,
                state.height,
                state.virtual_x,
                state.virtual_y,
                if state.focused { ", focused" } else { "" }
            );
            self.restart_entries.push(RestartEntry {
                state,
                mode_locked: w.mode_locked,
                pid,
                expires_ns: now + RESTART_ENTRY_TTL_NS,
            });
        }
    }

    /// The entry a window of `app_id` running `program` takes back after a
    /// restart, removed: the same title, then a resembling one, then the
    /// first of the app's left (titled or not — these are its own windows,
    /// not another app's under a shared class).
    pub(crate) fn take_restart_entry(&mut self, app_id: &str, title: &str, program: Option<&str>) -> Option<RestartEntry> {
        crate::wm_scope!(mut);
        let now = crate::util::timestamp_ns();
        self.restart_entries.retain(|e| e.expires_ns > now);
        if app_id.is_empty() {
            return None;
        }
        let ours = |e: &RestartEntry| e.state.app_id == app_id && same_program(&e.state, program);
        let pos = self
            .restart_entries
            .iter()
            .position(|e| ours(e) && titles_match(title, &e.state.title))
            .or_else(|| self.restart_entries.iter().position(|e| ours(e) && titles_resemble(title, &e.state.title)))
            .or_else(|| self.restart_entries.iter().position(ours))?;
        Some(self.restart_entries.remove(pos))
    }

    fn arm_app_restart_timer(&mut self) {
        crate::wm_scope!(mut);
        if !self.app_restart_timer.is_set() {
            // SAFETY: `handle_app_restart_tick` is an event-loop entry point that ignores its data.
            self.app_restart_timer = unsafe { crate::event_source::Timer::on_shared_loop(handle_app_restart_tick, 0) };
        }
        if !self.app_restart_timer.is_set() {
            // Without a timer nothing would ever start them again: say so
            // now. The windows are already closing.
            for mut r in self.app_restarts.drain(..) {
                r.answer(format!("error: no event-loop timer; {} was closed but will not be restarted\n", r.snap.name));
            }
            return;
        }
        self.app_restart_timer.arm(POLL_MS);
    }

    /// Start each app whose process has exited; give up on (or, `--force`,
    /// signal) one that outstays [`GRACE_MS`].
    fn poll_app_restarts(&mut self) {
        crate::wm_scope!(mut);
        let now = crate::util::timestamp_ns();
        let ms = |since: u64| now.saturating_sub(since) / 1_000_000;
        let mut kept = Vec::new();
        for mut r in std::mem::take(&mut self.app_restarts) {
            let name = r.snap.name.clone();
            let old = r.snap.pid;
            if r.exited() {
                let line = match r.snap.relaunch() {
                    Ok(new) if r.snap.unit.is_some() => {
                        log::info!("[ipc] restart-app: {} exited; starting its unit (systemctl pid {})", name, new);
                        format!("ok restarted {} (unit {})\n", name, r.snap.unit.as_deref().unwrap_or_default())
                    }
                    Ok(new) => {
                        log::info!("[ipc] restart-app: {} exited; started pid {}", name, new);
                        format!("ok restarted {} (pid {} -> {})\n", name, old, new)
                    }
                    Err(e) => {
                        log::error!("[ipc] restart-app: {} exited but did not start again: {}", name, e);
                        format!("error: {} exited but did not start again: {}\n", name, e)
                    }
                };
                r.answer(line);
                continue;
            }
            match r.termed_ns {
                None if ms(r.started_ns) >= GRACE_MS => {
                    if !r.force {
                        log::info!("[ipc] restart-app: {} (pid {}) still running after {} ms; restart cancelled", name, old, GRACE_MS);
                        // Its windows are staying (a save prompt kept them):
                        // their entries must not land a new window on them.
                        self.restart_entries.retain(|e| e.pid != old);
                        r.answer(format!(
                            "error: {} (pid {}) is still running {} s after its windows were asked to close \
                             (a save prompt, or it stays resident); not restarted — `restart-app --force` signals it\n",
                            name,
                            old,
                            GRACE_MS / 1000
                        ));
                        continue;
                    }
                    log::info!("[ipc] restart-app: {} (pid {}) still running; SIGTERM", name, old);
                    r.signal(libc::SIGTERM);
                    r.termed_ns = Some(now);
                }
                Some(t) if !r.killed && ms(t) >= KILL_AFTER_TERM_MS => {
                    log::info!("[ipc] restart-app: {} (pid {}) ignored SIGTERM; SIGKILL", name, old);
                    r.signal(libc::SIGKILL);
                    r.killed = true;
                }
                _ => {}
            }
            kept.push(r);
        }
        self.app_restarts = kept;
        if !self.app_restarts.is_empty() {
            self.arm_app_restart_timer();
        }
    }
}

unsafe extern "C" fn handle_app_restart_tick(_data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = crate::reentry::wm(crate::shared::server_ptr());
    if !wm.is_null() {
        (*wm).poll_app_restarts();
    }
    0
}
