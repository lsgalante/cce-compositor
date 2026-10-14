//! `ccectl restart-app [--force] [app_id|id]`: close every window of the
//! app's process, wait for it to exit, and start it again from the
//! `/proc` snapshot taken first (`crate::app_restart`). The reply is held
//! until the new process is started or the restart gives up, so a script can
//! chain on it.

use super::*;
use crate::app_restart::{PendingRestart, Snapshot, GRACE_MS, KILL_AFTER_TERM_MS, POLL_MS};

impl WindowManager {
    pub(crate) fn restart_app_command(&mut self, parts: &[&str]) -> String {
        crate::wm_scope!(mut);
        let mut args = &parts[1..];
        let force = args.first() == Some(&"--force");
        if force {
            args = &args[1..];
        }
        let target = if args.is_empty() {
            self.seat_focused_window_id()
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
        for &id in &ids {
            if let Some(w) = self.windows.get_mut(id) {
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
