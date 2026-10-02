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
//! The timeouts come from `idle { }` in config.kdl, but the System
//! Interface's Power plan can override either per power mode: its applier
//! writes [`PLAN_DISPLAY_OFF_FILE`] / [`PLAN_SLEEP_FILE`] under [`PLAN_DIR`] as root on plug,
//! unplug and boot (seconds, 0 = never), and this module polls both once a
//! second, so battery can darken the display sooner than the desk does
//! without a reload. A missing file means the config's value.
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

/// How long the sleep waits for the session to finish locking before it
/// goes ahead anyway. The desktop is hidden from the moment the lock starts
/// (`LockManager::lock_now`), so a lock still settling at that point is a
/// blank screen, not an open one; staying awake forever on a wedged output
/// would be the worse failure.
const LOCK_BEFORE_SLEEP_MS: i32 = 3000;

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

/// The Power plan's per-mode timeouts, written by `cce-power-apply`
/// (`cce_settings::power_plan::IDLE_DISPLAY_OFF_PATH` / `IDLE_SLEEP_PATH`;
/// the paths are repeated here because that crate is an app, not a
/// dependency). Seconds, 0 = never; absent = use the config.
pub const PLAN_DIR: &str = "/run/cce";
pub const PLAN_DISPLAY_OFF_FILE: &str = "idle_display_off";
pub const PLAN_SLEEP_FILE: &str = "idle_sleep";

/// Where one plan file lives. `CCE_IDLE_PLAN_DIR` moves the directory for
/// one process, for testing: /run/cce is root's, and a shadow session must
/// not read the live machine's plan files either.
fn plan_path(file: &str) -> String {
    let dir = std::env::var("CCE_IDLE_PLAN_DIR").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| PLAN_DIR.to_string());
    format!("{}/{}", dir, file)
}

/// How often the plan files are stat'ed. A mode change is a plug or an
/// unplug, so a second is instant to a person and two stats a second is
/// nothing.
const PLAN_POLL_MS: i32 = 1000;

/// One plan file's contents as a timeout: seconds on a line, nothing else.
pub fn parse_plan_secs(text: &str) -> Option<i64> {
    text.trim().parse::<u32>().ok().map(i64::from)
}

/// The timeout in force: the plan's when it has one, else the config's.
fn effective_ms(cfg_ms: i64, plan_ms: Option<i64>) -> i64 {
    plan_ms.unwrap_or(cfg_ms)
}

/// A change stamp for one plan file: its mtime in ns plus one, or 0 when it
/// is absent, so appearing, vanishing and rewriting all read as a change.
fn plan_stamp(path: &str) -> u128 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() + 1)
        .unwrap_or(0)
}

fn read_plan_ms(path: &str) -> Option<i64> {
    parse_plan_secs(&std::fs::read_to_string(path).ok()?).map(|s| s * 1000)
}

pub struct IdleManager {
    pub server: *mut Server,
    display_timer: *mut ffi::wl_event_source,
    sleep_timer: *mut ffi::wl_event_source,
    /// The timeouts in force, in ms; 0 = disabled. The plan's when it has
    /// one, else the config's (`effective_ms`).
    display_off_ms: i64,
    sleep_ms: i64,
    /// The `idle { }` block's own values, kept apart so a plan override can
    /// be lifted again when its file goes away.
    cfg_display_off_ms: i64,
    cfg_sleep_ms: i64,
    /// The Power plan's per-mode override for each, from the files under
    /// /run/cce; None while a file is absent or unparsable.
    plan_display_off_ms: Option<i64>,
    plan_sleep_ms: Option<i64>,
    /// Polls the plan files; see `PLAN_POLL_MS`.
    plan_timer: *mut ffi::wl_event_source,
    /// `plan_stamp` of each file at the last poll: the files are only
    /// re-read when one changes.
    plan_stamps: [u128; 2],
    /// `None` means `DEFAULT_SLEEP_COMMAND`. (An `Option<String>` is
    /// null-niche safe under `Server::new`'s zeroed init; a bare `String`
    /// is not.)
    sleep_command: Option<String>,
    /// An idle-inhibitor is active: timers are held disarmed.
    inhibited: bool,
    /// Who holds one, by app id (a window) or surface kind, deduplicated,
    /// in creation order; empty when nobody does. `Option` for the same
    /// reason as `sleep_command`: null-niche safe under the zeroed init.
    inhibitors: Option<Vec<String>>,
    /// The display timeout fired and outputs were darkened by us.
    displays_off: bool,
    /// The sleep command was spawned; cleared by the next activity.
    sleeping: bool,
    /// A sleep is waiting for the session lock to complete (`on_locked`),
    /// or for `lock_fallback_timer`, whichever comes first.
    sleep_after_lock: bool,
    lock_fallback_timer: *mut ffi::wl_event_source,
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
        self.lock_fallback_timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_lock_fallback),
            self as *mut IdleManager as *mut _,
        );
        if self.lock_fallback_timer.is_null() {
            ffi::wl_event_source_remove(self.display_timer);
            ffi::wl_event_source_remove(self.sleep_timer);
            self.display_timer = std::ptr::null_mut();
            self.sleep_timer = std::ptr::null_mut();
            return Err("Failed to create idle lock-fallback timer");
        }
        self.plan_timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_plan_poll),
            self as *mut IdleManager as *mut _,
        );
        if self.plan_timer.is_null() {
            ffi::wl_event_source_remove(self.display_timer);
            ffi::wl_event_source_remove(self.sleep_timer);
            self.display_timer = std::ptr::null_mut();
            self.sleep_timer = std::ptr::null_mut();
            return Err("Failed to create idle plan-poll timer");
        }
        self.display_off_ms = 0;
        self.sleep_ms = 0;
        self.cfg_display_off_ms = 0;
        self.cfg_sleep_ms = 0;
        self.plan_display_off_ms = None;
        self.plan_sleep_ms = None;
        self.plan_stamps = [0, 0];
        ffi::wl_event_source_timer_update(self.plan_timer, PLAN_POLL_MS);
        self.sleep_command = None;
        self.inhibited = false;
        self.inhibitors = None;
        self.displays_off = false;
        self.sleeping = false;
        self.sleep_after_lock = false;
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
        if !self.plan_timer.is_null() {
            ffi::wl_event_source_remove(self.plan_timer);
            self.plan_timer = std::ptr::null_mut();
        }
        if !self.lock_fallback_timer.is_null() {
            ffi::wl_event_source_remove(self.lock_fallback_timer);
            self.lock_fallback_timer = std::ptr::null_mut();
        }
    }

    /// Apply an `idle { }` block (config load and `ccectl reload`). A plan
    /// override in force stays in force: the config is the base it lifts to.
    pub unsafe fn configure(&mut self, cfg: &IdleConfig) {
        self.cfg_display_off_ms = cfg.display_off_s.max(0) * 1000;
        self.cfg_sleep_ms = cfg.sleep_s.max(0) * 1000;
        self.sleep_command = cfg.sleep_command.clone();
        self.refresh_effective();
        log::info!(
            "idle timeouts: display_off={}s sleep={}s command={:?}{}",
            self.display_off_ms / 1000,
            self.sleep_ms / 1000,
            self.sleep_command(),
            self.plan_note()
        );
        self.rearm(true);
    }

    /// Recompute the timeouts in force from the config and the plan.
    fn refresh_effective(&mut self) {
        self.display_off_ms = effective_ms(self.cfg_display_off_ms, self.plan_display_off_ms);
        self.sleep_ms = effective_ms(self.cfg_sleep_ms, self.plan_sleep_ms);
    }

    /// True when the Power plan overrides at least one timeout.
    fn plan_active(&self) -> bool {
        self.plan_display_off_ms.is_some() || self.plan_sleep_ms.is_some()
    }

    /// For log lines: which values the plan is imposing, or nothing.
    fn plan_note(&self) -> String {
        if !self.plan_active() {
            return String::new();
        }
        let show = |v: Option<i64>| v.map(|ms| format!("{}s", ms / 1000)).unwrap_or_else(|| "config".to_string());
        format!(
            " (power plan: display_off={} sleep={}, config {}s/{}s)",
            show(self.plan_display_off_ms),
            show(self.plan_sleep_ms),
            self.cfg_display_off_ms / 1000,
            self.cfg_sleep_ms / 1000
        )
    }

    /// From `plan_timer`: re-read the plan files when either changed, and
    /// put the new timeouts in force from now.
    pub unsafe fn poll_plan(&mut self) {
        let (off_path, sleep_path) = (plan_path(PLAN_DISPLAY_OFF_FILE), plan_path(PLAN_SLEEP_FILE));
        let stamps = [plan_stamp(&off_path), plan_stamp(&sleep_path)];
        if stamps == self.plan_stamps {
            return;
        }
        self.plan_stamps = stamps;
        self.plan_display_off_ms = read_plan_ms(&off_path);
        self.plan_sleep_ms = read_plan_ms(&sleep_path);
        self.refresh_effective();
        log::info!(
            "idle timeouts: display_off={}s sleep={}s{}",
            self.display_off_ms / 1000,
            self.sleep_ms / 1000,
            if self.plan_active() { self.plan_note() } else { " (power plan lifted, config values)".to_string() }
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
        // Someone is here: a sleep still waiting for its lock is called off.
        // The lock itself stands.
        if self.sleep_after_lock {
            self.sleep_after_lock = false;
            ffi::wl_event_source_timer_update(self.lock_fallback_timer, 0);
            log::info!("idle: activity while locking; the sleep is off, the lock stays");
        }
        self.rearm(changed);
    }

    /// From `IdleInhibitManager::check_active`: the set of clients holding
    /// an inhibitor changed. The names are what `ccectl idle status` and the
    /// log report, so a display that never darkens can be traced to the app
    /// keeping it on rather than to a bare `inhibited=true`.
    pub unsafe fn set_inhibitors(&mut self, names: Vec<String>) {
        if self.inhibitors.as_deref().unwrap_or(&[]) == names.as_slice() {
            return;
        }
        let inhibited = !names.is_empty();
        if inhibited {
            log::info!("idle: inhibited by {}", names.join(", "));
        } else {
            log::info!("idle: no inhibitors left");
        }
        self.inhibitors = Some(names);
        let flipped = self.inhibited != inhibited;
        self.inhibited = inhibited;
        // A change of holder while still inhibited leaves the timers as they
        // are: disarmed. Only the flag flipping rearms.
        if flipped {
            self.rearm(true);
        }
    }

    /// The inhibitor names as the status line prints them.
    fn inhibited_by(&self) -> String {
        self.inhibitors.as_deref().unwrap_or(&[]).join(",")
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

    /// Lock the session, then sleep once it is locked.
    ///
    /// Every compositor-initiated sleep goes through here (the idle timeout,
    /// `ccectl idle sleep`): a sleep used to run with the session unlocked,
    /// so the desktop was there for whoever woke the machine. A lid close is
    /// logind's sleep, not ours, and is covered by `sleep_lock`.
    pub unsafe fn lock_then_sleep(&mut self) {
        if self.sleeping || self.sleep_after_lock {
            return;
        }
        let lock = &mut (*self.server).lock_manager;
        lock.lock_now();
        if lock.state == crate::lock_manager::LockState::Locked {
            self.sleep_now();
            return;
        }
        log::info!("idle: locking before sleep");
        self.sleep_after_lock = true;
        ffi::wl_event_source_timer_update(self.lock_fallback_timer, LOCK_BEFORE_SLEEP_MS);
    }

    /// The session finished locking (`LockManager::send_locked`).
    pub unsafe fn on_locked(&mut self) {
        if self.sleep_after_lock {
            self.sleep_after_lock = false;
            ffi::wl_event_source_timer_update(self.lock_fallback_timer, 0);
            self.sleep_now();
        }
    }

    /// `ccectl idle` report.
    pub fn status(&self) -> String {
        let idle_s = now_ms().saturating_sub(self.last_activity_ms) / 1000;
        format!(
            "display_off={}s sleep={}s command={:?} idle={}s inhibited={} inhibited_by={:?} displays_off={} sleeping={} plan_display_off={} plan_sleep={}\n",
            self.display_off_ms / 1000,
            self.sleep_ms / 1000,
            self.sleep_command(),
            idle_s,
            self.inhibited,
            self.inhibited_by(),
            self.displays_off,
            self.sleeping,
            plan_field(self.plan_display_off_ms),
            plan_field(self.plan_sleep_ms)
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
                self.lock_then_sleep();
                "ok\n".to_string()
            }
            ["timeouts", display, sleep] => {
                match (display.parse::<i64>(), sleep.parse::<i64>()) {
                    (Ok(d), Ok(s)) if d >= 0 && s >= 0 => {
                        let cfg = IdleConfig { display_off_s: d, sleep_s: s, sleep_command: self.sleep_command.clone() };
                        self.configure(&cfg);
                        if self.plan_active() {
                            format!(
                                "ok, as the config base; the power plan keeps display_off={}s sleep={}s in force until its mode changes\n",
                                self.display_off_ms / 1000,
                                self.sleep_ms / 1000
                            )
                        } else {
                            "ok\n".to_string()
                        }
                    }
                    _ => "error: idle timeouts <display_off_s> <sleep_s> (non-negative seconds, 0 = off)\n".to_string(),
                }
            }
            _ => "error: usage: idle [status|wake|display on|display off|sleep|timeouts <display_off_s> <sleep_s>]\n".to_string(),
        }
    }
}

/// `none`, or the plan's seconds, for the status line.
fn plan_field(ms: Option<i64>) -> String {
    ms.map(|v| format!("{}s", v / 1000)).unwrap_or_else(|| "none".to_string())
}

unsafe extern "C" fn handle_plan_poll(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let idle = &mut *(data as *mut IdleManager);
    idle.poll_plan();
    // wl timers fire once; re-arm for the next look.
    if !idle.plan_timer.is_null() {
        ffi::wl_event_source_timer_update(idle.plan_timer, PLAN_POLL_MS);
    }
    0
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
        idle.lock_then_sleep();
    }
    0
}

unsafe extern "C" fn handle_lock_fallback(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let idle = &mut *(data as *mut IdleManager);
    if idle.sleep_after_lock {
        log::warn!("idle: the lock did not complete in {}ms; sleeping anyway (the desktop is already hidden)", LOCK_BEFORE_SLEEP_MS);
        idle.sleep_after_lock = false;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_file_is_seconds_and_nothing_else() {
        assert_eq!(parse_plan_secs("600\n"), Some(600));
        assert_eq!(parse_plan_secs(" 0 "), Some(0));
        assert_eq!(parse_plan_secs("-5"), None);
        assert_eq!(parse_plan_secs("10m"), None);
        assert_eq!(parse_plan_secs(""), None);
    }

    #[test]
    fn the_plan_wins_while_present_and_the_config_returns_after() {
        assert_eq!(effective_ms(600_000, Some(120_000)), 120_000);
        assert_eq!(effective_ms(600_000, Some(0)), 0);
        assert_eq!(effective_ms(600_000, None), 600_000);
        assert_eq!(plan_field(Some(120_000)), "120s");
        assert_eq!(plan_field(None), "none");
    }

    #[test]
    fn an_absent_plan_file_stamps_as_zero() {
        assert_eq!(plan_stamp("/nonexistent/cce/idle_display_off"), 0);
    }
}
