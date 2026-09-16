// SPDX-License-Identifier: GPL-3.0-only

//! Idle timeouts: turn the displays off after `display_off` seconds without
//! input, and run the sleep command after `sleep` seconds. Both are 0 (off)
//! until `idle { }` in config.kdl sets them.
//!
//! Activity is whatever `Seat::handle_activity` already counts as activity
//! for the `ext-idle-notify` clients — pointer motion, buttons, axes,
//! gestures, tablet, and (since this module) keys. An `idle-inhibit`
//! inhibitor on a mapped surface (a video player) pauses both timers, the
//! same signal `wlr_idle_notifier_v1_set_inhibited` gets.
//!
//! "Display off" is the soft-disable the wlr-output-power-management
//! protocol already drives (`OutputStateValue::DisabledSoft` — the output
//! stays in the layout, nothing is re-arranged, and no frame events fire
//! while it is dark, so an idle desktop also stops rendering). Only outputs
//! this module darkened (`Output::idle_off`) are woken again: one a client
//! turned off with `wlopm` stays as that client left it.
//!
//! Resume: `systemctl suspend` returns as soon as the job is queued, so the
//! command's exit says nothing. Instead the wlroots session's `active`
//! signal — which fires when the seat comes back from suspend, and on a VT
//! switch back — is treated as activity, so a lid-open shows the screen
//! without waiting for a key.

use crate::ffi;
use crate::server::{Server, WlListener, wl_signal_add, wl_listener_remove};
use crate::output::{Output, OutputStateValue};

pub const DEFAULT_SLEEP_COMMAND: &str = "systemctl suspend";

/// The `idle { }` block, in seconds; 0 disables a timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleConfig {
    pub display_off_s: i64,
    pub sleep_s: i64,
    pub sleep_command: Option<String>,
}

impl Default for IdleConfig {
    fn default() -> Self {
        Self { display_off_s: 0, sleep_s: 0, sleep_command: None }
    }
}

/// Re-arming the timers on every pointer-motion event would be a pair of
/// `timerfd_settime` calls per event; once a second is plenty for timeouts
/// measured in minutes.
const REARM_MIN_MS: u64 = 1000;

pub struct IdleManager {
    pub server: *mut Server,
    display_timer: *mut ffi::wl_event_source,
    sleep_timer: *mut ffi::wl_event_source,
    /// Timeouts in ms; 0 = disabled.
    display_off_ms: i64,
    sleep_ms: i64,
    /// `None` means `DEFAULT_SLEEP_COMMAND`. (An `Option<String>` is
    /// null-niche safe under `Server::new`'s zeroed init; a bare `String`
    /// is not.)
    sleep_command: Option<String>,
    /// An idle-inhibitor is active: timers are held disarmed.
    inhibited: bool,
    /// The display timeout fired and outputs were darkened by us.
    displays_off: bool,
    /// The sleep command was spawned; cleared by the next activity.
    sleeping: bool,
    /// Monotonic ms of the last (re)arm and of the last activity.
    armed_at_ms: u64,
    last_activity_ms: u64,
    session_active: ffi::wl_listener,
    session_listening: bool,
}

fn now_ms() -> u64 {
    let ts = crate::util::timestamp();
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

impl IdleManager {
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        let event_loop = ffi::wl_display_get_event_loop((*server).wl_server);
        self.display_timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_display_timeout),
            self as *mut IdleManager as *mut _,
        );
        if self.display_timer.is_null() {
            return Err("Failed to create idle display timer");
        }
        self.sleep_timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_sleep_timeout),
            self as *mut IdleManager as *mut _,
        );
        if self.sleep_timer.is_null() {
            ffi::wl_event_source_remove(self.display_timer);
            self.display_timer = std::ptr::null_mut();
            return Err("Failed to create idle sleep timer");
        }
        self.display_off_ms = 0;
        self.sleep_ms = 0;
        self.sleep_command = None;
        self.inhibited = false;
        self.displays_off = false;
        self.sleeping = false;
        self.armed_at_ms = 0;
        self.last_activity_ms = now_ms();

        // Headless and nested backends have no session; only DRM does.
        let session = (*server).session;
        if !session.is_null() {
            let listener = &mut self.session_active as *mut ffi::wl_listener as *mut WlListener;
            (*listener).notify = Some(handle_session_active);
            wl_signal_add(ffi::river_wlr_session_get_active_signal(session), &mut self.session_active);
            self.session_listening = true;
        }
        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        if self.session_listening {
            wl_listener_remove(&mut self.session_active);
            self.session_listening = false;
        }
        if !self.display_timer.is_null() {
            ffi::wl_event_source_remove(self.display_timer);
            self.display_timer = std::ptr::null_mut();
        }
        if !self.sleep_timer.is_null() {
            ffi::wl_event_source_remove(self.sleep_timer);
            self.sleep_timer = std::ptr::null_mut();
        }
    }

    /// Apply an `idle { }` block (config load and `ccectl reload`).
    pub unsafe fn configure(&mut self, cfg: &IdleConfig) {
        self.display_off_ms = cfg.display_off_s.max(0) * 1000;
        self.sleep_ms = cfg.sleep_s.max(0) * 1000;
        self.sleep_command = cfg.sleep_command.clone();
        log::info!(
            "idle timeouts: display_off={}s sleep={}s command={:?}",
            cfg.display_off_s.max(0),
            cfg.sleep_s.max(0),
            self.sleep_command()
        );
        self.rearm(true);
    }

    pub fn sleep_command(&self) -> &str {
        self.sleep_command.as_deref().unwrap_or(DEFAULT_SLEEP_COMMAND)
    }

    /// Input arrived (or the session came back). Wakes darkened outputs
    /// and restarts both countdowns.
    pub unsafe fn on_activity(&mut self) {
        let now = now_ms();
        self.last_activity_ms = now;
        let changed = self.displays_off || self.sleeping;
        if self.displays_off {
            self.set_displays(true);
        }
        self.sleeping = false;
        self.rearm(changed);
    }

    /// From `IdleInhibitManager::check_active`: an inhibitor appeared or
    /// the last one went away.
    pub unsafe fn set_inhibited(&mut self, inhibited: bool) {
        if self.inhibited == inhibited {
            return;
        }
        self.inhibited = inhibited;
        log::debug!("idle: inhibited={}", inhibited);
        self.rearm(true);
    }

    /// Arm (or disarm, when inhibited or unconfigured) both timers from
    /// now. Throttled unless `force`: pointer motion calls this per event.
    unsafe fn rearm(&mut self, force: bool) {
        let now = now_ms();
        if !force && now.saturating_sub(self.armed_at_ms) < REARM_MIN_MS {
            return;
        }
        self.armed_at_ms = now;
        let active = !self.inhibited;
        let display_ms = if active { self.display_off_ms } else { 0 };
        let sleep_ms = if active { self.sleep_ms } else { 0 };
        if !self.display_timer.is_null() {
            ffi::wl_event_source_timer_update(self.display_timer, display_ms.min(i32::MAX as i64) as i32);
        }
        if !self.sleep_timer.is_null() {
            ffi::wl_event_source_timer_update(self.sleep_timer, sleep_ms.min(i32::MAX as i64) as i32);
        }
    }

    /// Darken (`on == false`) every enabled output, or wake the ones this
    /// module darkened. Goes through the same scheduled-state path as the
    /// output-power protocol; the next transaction commits it.
    pub unsafe fn set_displays(&mut self, on: bool) {
        let server = &mut *self.server;
        let head = &mut server.om.outputs as *mut ffi::wl_list;
        let mut link = (*head).next;
        let mut touched = 0;
        while link != head {
            let output = &mut *crate::container_of!(link, Output, link);
            link = (*link).next;
            if output.wlr_output.is_null() {
                continue;
            }
            if on {
                if output.idle_off {
                    output.idle_off = false;
                    if output.scheduled.state == OutputStateValue::DisabledSoft {
                        output.scheduled.state = OutputStateValue::Enabled;
                        touched += 1;
                    }
                }
            } else if output.scheduled.state == OutputStateValue::Enabled {
                output.scheduled.state = OutputStateValue::DisabledSoft;
                output.idle_off = true;
                touched += 1;
            }
        }
        self.displays_off = !on;
        log::info!("idle: displays {} ({} output(s))", if on { "on" } else { "off" }, touched);
        if touched > 0 {
            server.wm.dirty_windowing();
        }
    }

    /// Run the sleep command (`sh -c`), detached; the server's SIGCHLD
    /// handler reaps it.
    pub unsafe fn sleep_now(&mut self) {
        let cmd = self.sleep_command().to_string();
        log::info!("idle: sleeping via `{}`", cmd);
        self.sleeping = true;
        match nix::unistd::fork() {
            Ok(nix::unistd::ForkResult::Child) => {
                crate::process::cleanup_child();
                let sh = std::ffi::CString::new("/bin/sh").unwrap();
                let dash_c = std::ffi::CString::new("-c").unwrap();
                let cmd_c = std::ffi::CString::new(cmd).unwrap_or_else(|_| std::ffi::CString::new("true").unwrap());
                let args = [sh.as_c_str(), dash_c.as_c_str(), cmd_c.as_c_str()];
                let _ = nix::unistd::execv(&sh, &args);
                std::process::exit(1);
            }
            Ok(nix::unistd::ForkResult::Parent { .. }) => {}
            Err(e) => {
                log::error!("idle: failed to fork for sleep command: {}", e);
                self.sleeping = false;
            }
        }
    }

    /// `ccectl idle` report.
    pub fn status(&self) -> String {
        let idle_s = now_ms().saturating_sub(self.last_activity_ms) / 1000;
        format!(
            "display_off={}s sleep={}s command={:?} idle={}s inhibited={} displays_off={} sleeping={}\n",
            self.display_off_ms / 1000,
            self.sleep_ms / 1000,
            self.sleep_command(),
            idle_s,
            self.inhibited,
            self.displays_off,
            self.sleeping
        )
    }

    /// `ccectl idle …` — see `cce_ctl.rs` for the surface.
    pub unsafe fn ipc(&mut self, args: &[&str]) -> String {
        match args {
            [] | ["status"] => self.status(),
            ["wake"] => {
                self.on_activity();
                "ok\n".to_string()
            }
            ["display", "off"] => {
                self.set_displays(false);
                "ok\n".to_string()
            }
            ["display", "on"] => {
                self.set_displays(true);
                "ok\n".to_string()
            }
            ["sleep"] => {
                self.sleep_now();
                "ok\n".to_string()
            }
            ["timeouts", display, sleep] => {
                match (display.parse::<i64>(), sleep.parse::<i64>()) {
                    (Ok(d), Ok(s)) if d >= 0 && s >= 0 => {
                        let cfg = IdleConfig { display_off_s: d, sleep_s: s, sleep_command: self.sleep_command.clone() };
                        self.configure(&cfg);
                        "ok\n".to_string()
                    }
                    _ => "error: idle timeouts <display_off_s> <sleep_s> (non-negative seconds, 0 = off)\n".to_string(),
                }
            }
            _ => "error: usage: idle [status|wake|display on|display off|sleep|timeouts <display_off_s> <sleep_s>]\n".to_string(),
        }
    }
}

unsafe extern "C" fn handle_display_timeout(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let idle = &mut *(data as *mut IdleManager);
    if !idle.inhibited && !idle.displays_off {
        log::info!("idle: display timeout reached");
        idle.set_displays(false);
    }
    0
}

unsafe extern "C" fn handle_sleep_timeout(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let idle = &mut *(data as *mut IdleManager);
    if !idle.inhibited && !idle.sleeping {
        log::info!("idle: sleep timeout reached");
        idle.sleep_now();
    }
    0
}

unsafe extern "C" fn handle_session_active(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let idle = &mut *crate::container_of!(listener, IdleManager, session_active);
    let session = (*idle.server).session;
    if session.is_null() || !ffi::river_wlr_session_get_active(session) {
        return;
    }
    log::info!("idle: session active (resume / VT switch), waking");
    idle.on_activity();
}
