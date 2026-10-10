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
use crate::server::{Server};
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

/// The Power plan's per-mode timeouts, written by `cce-power-apply`;
/// cce-core's `plan` spells the paths for both sides. Seconds, 0 = never;
/// absent = use the config.
pub use cce_core::plan::{DIR as PLAN_DIR, IDLE_DISPLAY_OFF_FILE as PLAN_DISPLAY_OFF_FILE, IDLE_SLEEP_FILE as PLAN_SLEEP_FILE};

/// Where one plan file lives. `CCE_IDLE_PLAN_DIR` moves the directory for
/// one process, for testing: /run/cce is root's, and a shadow session must
/// not read the live machine's plan files either.
fn plan_path(file: &str) -> String {
    format!("{}/{}", plan_dir(), file)
}

fn plan_dir() -> String {
    std::env::var("CCE_IDLE_PLAN_DIR").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| PLAN_DIR.to_string())
}

/// How often the plan files are stat'ed when the directory cannot be
/// watched (`watch_plan_dir`). A mode change is a plug or an unplug, so a
/// second is instant to a person.
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
    /// Polls the plan files when they cannot be watched; see `PLAN_POLL_MS`.
    plan_timer: *mut ffi::wl_event_source,
    /// An inotify fd on the plan directory, and its event-loop source: the
    /// plan files are re-read when the directory reports a change, and
    /// nothing runs at rest. -1 / null while polling instead. Until
    /// 2026-10-05 the timer above stat'ed both files every second, forever —
    /// the one thing that still ticked in an idle compositor.
    plan_inotify: i32,
    plan_inotify_source: *mut ffi::wl_event_source,
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
    /// The Wayland half of `inhibitors`, as `IdleInhibitManager` last
    /// reported it; `external` is the other half.
    wayland_inhibitors: Option<Vec<String>>,
    /// External leases (see the module doc), in grant order. `Option` for
    /// the zeroed init, like `inhibitors`.
    external: Option<Vec<ExternalLease>>,
    /// Fires at the earliest lease expiry; disarmed when there are none.
    lease_timer: *mut ffi::wl_event_source,
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
    session_active: crate::listener::Listener,
    session_listening: bool,
}

/// One `idle inhibit` lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLease {
    pub token: String,
    pub who: String,
    pub expires_ms: u64,
}

/// The longest lease one request may take: a holder that wants longer
/// renews. Bounds how long a dead holder's lease can outlive it.
pub const MAX_LEASE_S: u64 = 600;

/// `idle inhibit <token> <ttl_s> <who...>`: the lease it asks for, or the
/// usage error. `who` is what `idle status` reports (`steam`, `org.mozilla.firefox`).
pub fn parse_lease(args: &[&str], now: u64) -> Result<ExternalLease, String> {
    let usage = || "error: idle inhibit <token> <ttl_s 1-600> <who>\n".to_string();
    let [token, ttl, who @ ..] = args else { return Err(usage()) };
    if who.is_empty() {
        return Err(usage());
    }
    let ttl: u64 = ttl.parse().map_err(|_| usage())?;
    if ttl == 0 || ttl > MAX_LEASE_S {
        return Err(usage());
    }
    Ok(ExternalLease { token: token.to_string(), who: who.join(" "), expires_ms: now + ttl * 1000 })
}

/// The inhibitor names `idle status` and the log show: the Wayland holders,
/// then each external holder once, as `portal:<who>`.
pub fn merge_inhibitors(wayland: &[String], external: &[ExternalLease]) -> Vec<String> {
    let mut names: Vec<String> = wayland.to_vec();
    for lease in external {
        let name = format!("portal:{}", lease.who);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
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
        self.lease_timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_lease_expiry),
            self as *mut IdleManager as *mut _,
        );
        if self.lease_timer.is_null() {
            ffi::wl_event_source_remove(self.display_timer);
            ffi::wl_event_source_remove(self.sleep_timer);
            ffi::wl_event_source_remove(self.plan_timer);
            self.display_timer = std::ptr::null_mut();
            self.sleep_timer = std::ptr::null_mut();
            self.plan_timer = std::ptr::null_mut();
            return Err("Failed to create idle lease timer");
        }
        self.display_off_ms = 0;
        self.sleep_ms = 0;
        self.cfg_display_off_ms = 0;
        self.cfg_sleep_ms = 0;
        self.plan_display_off_ms = None;
        self.plan_sleep_ms = None;
        self.plan_stamps = [0, 0];
        // Zeroed by `Server::new`: 0 is a real fd (stdin), so say "none".
        self.plan_inotify = -1;
        self.plan_inotify_source = std::ptr::null_mut();
        if !self.watch_plan_dir(event_loop) {
            ffi::wl_event_source_timer_update(self.plan_timer, PLAN_POLL_MS);
        }
        self.sleep_command = None;
        self.inhibited = false;
        self.inhibitors = None;
        self.wayland_inhibitors = None;
        self.external = None;
        self.displays_off = false;
        self.sleeping = false;
        self.sleep_after_lock = false;
        self.armed_at_ms = 0;
        self.last_activity_ms = now_ms();

        // Headless and nested backends have no session; only DRM does.
        let session = (*server).session;
        if !session.is_null() {
            self.session_active.connect(ffi::river_wlr_session_get_active_signal(session), handle_session_active);
            self.session_listening = true;
        }
        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        if self.session_listening {
            self.session_active.disconnect();
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
        if !self.lease_timer.is_null() {
            ffi::wl_event_source_remove(self.lease_timer);
            self.lease_timer = std::ptr::null_mut();
        }
        self.unwatch_plan_dir();
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

    /// Watch the plan directory for any file appearing, changing or going
    /// away. False — and the caller polls — when the directory does not
    /// exist or cannot be watched.
    unsafe fn watch_plan_dir(&mut self, event_loop: *mut ffi::wl_event_loop) -> bool {
        let Ok(dir) = std::ffi::CString::new(plan_dir()) else { return false };
        let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
        if fd < 0 {
            return false;
        }
        let mask = libc::IN_CLOSE_WRITE
            | libc::IN_MOVED_TO
            | libc::IN_MOVED_FROM
            | libc::IN_CREATE
            | libc::IN_DELETE
            | libc::IN_DELETE_SELF
            | libc::IN_MOVE_SELF;
        if libc::inotify_add_watch(fd, dir.as_ptr(), mask) < 0 {
            libc::close(fd);
            return false;
        }
        let source = ffi::wl_event_loop_add_fd(
            event_loop,
            fd,
            ffi::WL_EVENT_READABLE as u32,
            Some(handle_plan_inotify),
            self as *mut IdleManager as *mut _,
        );
        if source.is_null() {
            libc::close(fd);
            return false;
        }
        self.plan_inotify = fd;
        self.plan_inotify_source = source;
        true
    }

    unsafe fn unwatch_plan_dir(&mut self) {
        if !self.plan_inotify_source.is_null() {
            ffi::wl_event_source_remove(self.plan_inotify_source);
            self.plan_inotify_source = std::ptr::null_mut();
        }
        if self.plan_inotify >= 0 {
            libc::close(self.plan_inotify);
            self.plan_inotify = -1;
        }
    }

    /// From `plan_timer` or the directory watch: re-read the plan files when either changed, and
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
        self.wayland_inhibitors = Some(names);
        self.apply_inhibitors();
    }

    /// Recompute the holder list from both halves and act on a change.
    unsafe fn apply_inhibitors(&mut self) {
        let names = merge_inhibitors(
            self.wayland_inhibitors.as_deref().unwrap_or(&[]),
            self.external.as_deref().unwrap_or(&[]),
        );
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
    /// Grant or renew a lease (`idle inhibit`).
    unsafe fn grant_lease(&mut self, lease: ExternalLease) {
        let leases = self.external.get_or_insert_with(Vec::new);
        match leases.iter_mut().find(|l| l.token == lease.token) {
            Some(existing) => *existing = lease,
            None => {
                log::info!("idle: lease {} granted to {}", lease.token, lease.who);
                leases.push(lease);
            }
        }
        self.arm_lease_timer();
        self.apply_inhibitors();
    }

    /// End leases: one by token, or every one (`None`).
    unsafe fn end_leases(&mut self, token: Option<&str>) {
        if let Some(leases) = self.external.as_mut() {
            leases.retain(|l| token.is_some_and(|t| l.token != t));
        }
        self.arm_lease_timer();
        self.apply_inhibitors();
    }

    /// Drop the leases whose time is up.
    unsafe fn expire_leases(&mut self) {
        let now = now_ms();
        if let Some(leases) = self.external.as_mut() {
            leases.retain(|l| {
                let live = l.expires_ms > now;
                if !live {
                    log::warn!("idle: lease {} ({}) lapsed without renewal", l.token, l.who);
                }
                live
            });
        }
        self.arm_lease_timer();
        self.apply_inhibitors();
    }

    unsafe fn arm_lease_timer(&mut self) {
        if self.lease_timer.is_null() {
            return;
        }
        let next = self.external.as_deref().unwrap_or(&[]).iter().map(|l| l.expires_ms).min();
        let ms = match next {
            // A timer of 0 disarms, so an already-due lease fires in 1 ms.
            Some(at) => at.saturating_sub(now_ms()).clamp(1, i32::MAX as u64) as i32,
            None => 0,
        };
        ffi::wl_event_source_timer_update(self.lease_timer, ms);
    }

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
            ["inhibit", rest @ ..] => match parse_lease(rest, now_ms()) {
                Ok(lease) => {
                    self.grant_lease(lease);
                    "ok\n".to_string()
                }
                Err(e) => e,
            },
            ["uninhibit", token] => {
                self.end_leases(Some(*token));
                "ok\n".to_string()
            }
            ["inhibit-clear"] => {
                self.end_leases(None);
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
            _ => "error: usage: idle [status|wake|display on|display off|sleep|timeouts <display_off_s> <sleep_s>|inhibit <token> <ttl_s> <who>|uninhibit <token>|inhibit-clear]\n".to_string(),
        }
    }
}

/// `none`, or the plan's seconds, for the status line.
fn plan_field(ms: Option<i64>) -> String {
    ms.map(|v| format!("{}s", v / 1000)).unwrap_or_else(|| "none".to_string())
}

/// The plan directory changed: drain the events, then re-read. The
/// directory itself going away ends the watch, and polling takes over.
unsafe extern "C" fn handle_plan_inotify(_fd: i32, _mask: u32, data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let idle = &mut *(data as *mut IdleManager);
    let mut buf = [0u8; 4096];
    let mut dir_gone = false;
    loop {
        let n = libc::read(idle.plan_inotify, buf.as_mut_ptr() as *mut _, buf.len());
        if n <= 0 {
            break;
        }
        let mut off = 0usize;
        while off + std::mem::size_of::<libc::inotify_event>() <= n as usize {
            let ev = std::ptr::read_unaligned(buf.as_ptr().add(off) as *const libc::inotify_event);
            if ev.mask & (libc::IN_IGNORED | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF) != 0 {
                dir_gone = true;
            }
            off += std::mem::size_of::<libc::inotify_event>() + ev.len as usize;
        }
    }
    if dir_gone {
        log::info!("idle: power-plan directory went away; polling for it");
        idle.unwatch_plan_dir();
        if !idle.plan_timer.is_null() {
            ffi::wl_event_source_timer_update(idle.plan_timer, PLAN_POLL_MS);
        }
    }
    idle.poll_plan();
    0
}

unsafe extern "C" fn handle_plan_poll(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let idle = &mut *(data as *mut IdleManager);
    idle.poll_plan();
    // The directory may exist now: watch it, and stop polling.
    let event_loop = ffi::wl_display_get_event_loop((*idle.server).wl_server);
    if idle.watch_plan_dir(event_loop) {
        log::info!("idle: watching the power-plan directory");
        idle.poll_plan();
        return 0;
    }
    // wl timers fire once; re-arm for the next look.
    if !idle.plan_timer.is_null() {
        ffi::wl_event_source_timer_update(idle.plan_timer, PLAN_POLL_MS);
    }
    0
}

unsafe extern "C" fn handle_lease_expiry(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let idle = &mut *(data as *mut IdleManager);
    idle.expire_leases();
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
    fn the_portal_lease_line_cce_core_builds_parses_here() {
        use cce_core::ipc::ctl::{IdleRequest, Request};
        let line = Request::Idle(IdleRequest::Inhibit {
            token: "tok-1".into(),
            ttl_s: 60,
            who: "org.example.Player playing video".into(),
        })
        .to_string();
        let words: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(&words[..2], ["idle", "inhibit"]);
        let lease = parse_lease(&words[2..], 1_000).expect("the compositor accepts the line clients send");
        assert_eq!((lease.token.as_str(), lease.who.as_str()), ("tok-1", "org.example.Player playing video"));
        assert_eq!(lease.expires_ms, 61_000);
    }

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
    fn a_lease_request_is_token_ttl_and_who() {
        let lease = parse_lease(&["portal-7", "90", "steam"], 1_000).unwrap();
        assert_eq!(lease, ExternalLease { token: "portal-7".into(), who: "steam".into(), expires_ms: 91_000 });
        let spaced = parse_lease(&["ss-2", "30", "Firefox", "video"], 0).unwrap();
        assert_eq!(spaced.who, "Firefox video", "who keeps its spaces");
        for bad in [&["t", "0", "x"][..], &["t", "601", "x"], &["t", "ten", "x"], &["t", "30"], &[]] {
            assert!(parse_lease(bad, 0).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn external_holders_follow_the_wayland_ones_once_each() {
        let lease = |t: &str, w: &str| ExternalLease { token: t.into(), who: w.into(), expires_ms: 0 };
        let names = merge_inhibitors(
            &["mpv".to_string()],
            &[lease("a", "steam"), lease("b", "firefox"), lease("c", "steam")],
        );
        assert_eq!(names, ["mpv", "portal:steam", "portal:firefox"]);
        assert!(merge_inhibitors(&[], &[]).is_empty());
    }

    #[test]
    fn an_absent_plan_file_stamps_as_zero() {
        assert_eq!(plan_stamp("/nonexistent/cce/idle_display_off"), 0);
    }
}
