// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlList};
use crate::scene_node_data::{SceneNodeData, SceneNodeDataVal};
use crate::seat::Focus;

/// Delay before each successive attempt to bring the locker back, in ms. The
/// length of this is also the attempt limit.
///
/// It backs off because the bad case is a locker that dies during startup:
/// respawning that at full speed is a fork bomb against a user who cannot see
/// the screen. It gives up rather than retrying forever for the same reason,
/// and giving up is safe — the session simply stays locked, which is where it
/// already was.
const RESPAWN_BACKOFF_MS: [i32; 5] = [200, 500, 1000, 2000, 5000];

/// The locker to run. Mirrors `cce_cloud_cmd()`: prefer the installed path,
/// fall back to the bare name on PATH.
fn cce_lock_cmd() -> String {
    if let Ok(home) = std::env::var("HOME") {
        let path = format!("{}/.local/bin/cce-lock", home);
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }
    "cce-lock".to_string()
}

pub struct LockManager {
    pub wlr_manager: *mut ffi::wlr_session_lock_manager_v1,
    /// Read freely; write through [`LockManager::set_state`], which keeps
    /// `crate::shared::lock_state()` in step.
    pub state: LockState,
    /// The live lock, or null: `handle_new_lock` sets it and the lock's
    /// destroy handler clears it. Private, because the safe
    /// `lock_surface_from_output` trusts it.
    lock: *mut ffi::wlr_session_lock_v1,
    pub lock_surfaces_timer: crate::event_source::Timer,
    /// Fires to bring a locker back after one died mid-lock; see
    /// [`LockManager::schedule_locker_respawn`].
    pub respawn_timer: crate::event_source::Timer,
    /// Respawns since the last locker that got as far as drawing. Indexes
    /// [`RESPAWN_BACKOFF_MS`]; past its end, we stop trying.
    pub respawn_attempts: usize,
    /// Replies owed to `ccectl lock` callers (the lock-before-sleep thread
    /// among them), sent once the session is `Locked`. `Option` because the
    /// struct is zero-initialised: `None` is the null niche, an empty `Vec`
    /// is not.
    pub lock_waiters: Option<Vec<std::sync::mpsc::Sender<String>>>,

    new_lock: crate::listener::Listener,
    unlock: crate::listener::Listener,
    destroy: crate::listener::Listener,
    new_surface: crate::listener::Listener,
}

// SAFETY: a field of the one `Server`, which only the unsafe `Server::zeroed`
// makes and `run_server` never frees. It has no safe constructor, so safe code
// holding the one `&mut` to it has nothing to swap it with or replace it by.
unsafe impl crate::listener::Anchored for LockManager {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LockState {
    Unlocked,
    Locked,
    WaitingForBlank,
    WaitingForLockSurfaces,
}

impl LockManager {
    pub fn set_state(&mut self, state: LockState) {
        self.state = state;
        crate::shared::shared().store_lock_state(state);
    }

    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.set_state(LockState::Unlocked);
        std::ptr::write(&mut self.lock_waiters, None);

        let wlr_manager = ffi::wlr_session_lock_manager_v1_create((*server).wl_server);
        if wlr_manager.is_null() {
            return Err("Failed to create wlr_session_lock_manager_v1");
        }
        self.wlr_manager = wlr_manager;

        let timer = crate::event_source::Timer::on_shared_loop(handle_lock_surfaces_timeout, 0);
        if !timer.is_set() {
            return Err("Failed to create lock surfaces timer");
        }
        self.lock_surfaces_timer = timer;

        let respawn_timer = crate::event_source::Timer::on_shared_loop(handle_respawn_timeout, 0);
        if !respawn_timer.is_set() {
            return Err("Failed to create the locker respawn timer");
        }
        self.respawn_timer = respawn_timer;

        self.new_lock.connect(&mut (*self.wlr_manager).events.new_lock, handle_new_lock);

        Ok(())
    }

    pub fn deinit(&mut self) {
        self.lock_surfaces_timer.remove();
        self.respawn_timer.remove();
        self.new_lock.disconnect();
    }

    /// The lock surface on `wlr_output` (null: none).
    pub fn lock_surface_from_output(
        &self,
        wlr_output: *mut ffi::wlr_output,
    ) -> Option<*mut LockSurface> {
        if wlr_output.is_null() {
            return None;
        }
        if self.lock.is_null() {
            return None;
        }

        // SAFETY: a non-null `lock` is live: `handle_new_lock` sets it and
        // the lock's destroy handler clears it. Its surfaces list holds its
        // live lock surfaces; each one's data is null until
        // `LockSurface::create` sets it, and the LockSurface is freed only
        // in the surface's destroy handler, after which wlroots unlinks the
        // surface before anything of ours runs again.
        unsafe {
            let surfaces_head = &mut (*self.lock).surfaces as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*surfaces_head).next;
            while curr != surfaces_head {
                let next = (*curr).next;
                let wlr_lock_surface = crate::container_of!(curr, ffi::wlr_session_lock_surface_v1, link);
                let lock_surface = (*wlr_lock_surface).data as *mut LockSurface;
                if !lock_surface.is_null() && (*lock_surface).wlr_output() == wlr_output {
                    return Some(lock_surface);
                }
                curr = next;
            }
        }

        None
    }

    /// Complete a pending lock once every enabled output shows what the
    /// state waits for. Returns true when this call completed it: the
    /// caller then tells the idle manager (`IdleManager::on_locked`).
    #[must_use]
    pub fn maybe_lock(&mut self, outputs: &crate::output::Outputs) -> bool {
        let mut all_outputs_blanked = true;
        let mut all_outputs_rendered_lock_surface = true;

        for output in outputs.values() {
            if output.wlr_output().enabled() {
                match output.lock_render_state {
                    crate::output::LockRenderState::PendingUnlock
                    | crate::output::LockRenderState::Unlocked
                    | crate::output::LockRenderState::PendingBlank
                    | crate::output::LockRenderState::PendingLockSurface => {
                        all_outputs_blanked = false;
                        all_outputs_rendered_lock_surface = false;
                    }
                    crate::output::LockRenderState::Blanked => {
                        all_outputs_rendered_lock_surface = false;
                    }
                    crate::output::LockRenderState::LockSurface => {}
                }
            }
        }

        match self.state {
            LockState::WaitingForLockSurfaces => {
                if all_outputs_rendered_lock_surface {
                    self.send_locked();
                    crate::shared::scene().normal_tree.set_enabled(false);
                    self.lock_surfaces_timer.arm(0);
                    return true;
                }
            }
            LockState::WaitingForBlank => {
                if all_outputs_blanked {
                    self.send_locked();
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    pub fn send_locked(&mut self) {
        log::info!("session locked");
        if !self.lock.is_null() {
            // SAFETY: a non-null `lock` is live (the field's invariant: set
            // by `handle_new_lock`, cleared by the lock's destroy handler).
            unsafe { ffi::wlr_session_lock_v1_send_locked(self.lock) };
        }
        self.set_state(LockState::Locked);
        for tx in self.lock_waiters.take().unwrap_or_default() {
            let _ = tx.send("ok locked\n".to_string());
        }
        crate::shared::pending().dirty_windowing();
    }

    /// Lock the session from the compositor's side, with no lock client yet.
    ///
    /// Until 2026-10-01 only a client could lock — the protocol's `lock()` —
    /// and nothing ever started one: the idle timeout and a lid close both
    /// suspended an UNLOCKED session, so whoever opened the lid had the
    /// desktop. Now the compositor hides the desktop at once (the normal tree
    /// off, every output rendering the blank locked tree) and then starts
    /// `cce-lock`, which binds through `handle_new_lock`'s "already locked
    /// session" branch and puts the prompt up. This is the state a crashed
    /// locker leaves behind, so the respawn timer is what starts it, with the
    /// same backoff and the same fail-closed end: a locker that never comes
    /// leaves the session locked, not open.
    ///
    /// No-op when the session is already locked or locking. Returns true
    /// when the lock completed at once (no output to wait for), as
    /// [`Self::maybe_lock`] does.
    #[must_use]
    pub fn lock_now(&mut self, wm: &mut crate::window_manager::WindowManager) -> bool {
        if self.state != LockState::Unlocked {
            return false;
        }
        log::info!("locking the session (compositor-initiated)");
        self.set_state(LockState::WaitingForBlank);
        let scene = crate::shared::scene();
        scene.locked_tree.set_enabled(true);
        scene.normal_tree.set_enabled(false);

        for seat in wm.seats.ids() {
            crate::seat::Seat::focus_window(wm, seat, None);
        }

        // Each enabled output reports `Blanked` after its next frame, and the
        // last of them completes the lock in `maybe_lock`. With none enabled
        // (the idle timeout darkened them all) there is nothing to wait for,
        // and this call completes it now.
        for output in wm.outputs.values() {
            if !output.wlr_output().is_null() && output.wlr_output().enabled() {
                output.wlr_output().schedule_frame();
            }
        }
        crate::shared::pending().dirty_windowing();
        let locked = self.maybe_lock(&wm.outputs);

        self.respawn_attempts = 0;
        self.respawn_timer.arm(1);
        locked
    }

    /// Answer `tx` once the session is locked: now if it is, else from
    /// `send_locked`.
    pub fn reply_when_locked(&mut self, tx: std::sync::mpsc::Sender<String>) {
        if self.state == LockState::Locked {
            let _ = tx.send("ok locked\n".to_string());
        } else {
            self.lock_waiters.get_or_insert_with(Vec::new).push(tx);
        }
    }

    /// Bring a locker back after the one holding the session went away
    /// without unlocking.
    ///
    /// The session stays locked when a locker dies — that is the protocol's
    /// guarantee and this does not weaken it. What it fixes is that the
    /// screen was then a dead end: locked, blank, with no process left to
    /// type a password into, so the only way back into the session was a TTY
    /// and a kill. `handle_new_lock` already knows how to hand an
    /// already-locked session to a fresh client ("given control of already
    /// locked session"); nothing ever started one.
    ///
    /// Backed off and capped by [`RESPAWN_BACKOFF_MS`]. Exhausting it leaves
    /// the session exactly as it is now — locked — which is why giving up is
    /// an acceptable outcome and looping forever is not.
    fn schedule_locker_respawn(&mut self) {
        if !self.respawn_timer.is_set() {
            return;
        }
        let Some(&delay) = RESPAWN_BACKOFF_MS.get(self.respawn_attempts) else {
            log::error!(
                "the locker died {} times without drawing; giving up. The session \
                 STAYS LOCKED and there is no prompt to type into — switch to a TTY \
                 (ctrl+alt+F2), log in, and run `cce-lock` against this display, or \
                 kill the session.",
                self.respawn_attempts
            );
            return;
        };
        self.respawn_attempts += 1;
        // Worded for both callers: the one after a locker vanished, and the
        // one right after a spawn that arms this as a "did it take?" check.
        // The check is the common case and is cancelled by `handle_new_lock`
        // without ever firing, so this must not promise a respawn outright.
        log::warn!(
            "no lock client for a locked session; starting one in {}ms unless one \
             binds first (attempt {} of {})",
            delay,
            self.respawn_attempts,
            RESPAWN_BACKOFF_MS.len()
        );
        self.respawn_timer.arm(delay);
    }

    /// Stop a respawn that is armed but no longer wanted — a locker is here.
    /// Without this, a timer armed during the gap could fire after the
    /// session was unlocked and lock it again out of nowhere.
    fn cancel_locker_respawn(&mut self) {
        self.respawn_timer.arm(0);
    }
}

pub struct LockSurface {
    /// Watched: wlroots destroys it with the lock surface.
    pub tree: crate::scene_handle::SceneTree,
    /// Set once, by `create`; private because the safe `wlr_output`,
    /// `surface` and `configure` trust it.
    wlr_lock_surface: *mut ffi::wlr_session_lock_surface_v1,
    pub lock: *mut ffi::wlr_session_lock_v1,
    pub manager: *mut LockManager,

    pub idle_update_focus: *mut ffi::wl_event_source,

    map: crate::listener::Listener,
    surface_destroy: crate::listener::Listener,
}

// SAFETY: made by `Box::into_raw` and freed only by its own destroy path,
// after its listeners are gone, and reached only through raw pointers: no safe
// API returns a `&mut`, a `Box` or a value of it, so nothing safe can move it.
unsafe impl crate::listener::Anchored for LockSurface {}

thread_local! {
    /// Every live `LockSurface`: `create` adds it and `destroy` removes it
    /// before freeing it. A seat's lock-surface focus is checked against
    /// this (`LockSurface::is_live`), so naming one needs no caller promise.
    static LIVE_LOCK_SURFACES: std::cell::RefCell<Vec<*mut LockSurface>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl LockSurface {
    /// Whether `lock_surface` is a live lock surface.
    pub fn is_live(lock_surface: *mut LockSurface) -> bool {
        !lock_surface.is_null() && LIVE_LOCK_SURFACES.with(|l| l.borrow().contains(&lock_surface))
    }

    pub unsafe fn create(
        wlr_lock_surface: *mut ffi::wlr_session_lock_surface_v1,
        lock: *mut ffi::wlr_session_lock_v1,
        manager: *mut LockManager,
    ) -> Result<*mut Self, &'static str> {
        let tree = ffi::wlr_scene_subsurface_tree_create(
            crate::shared::scene().locked_tree.raw(),
            (*wlr_lock_surface).surface,
        );
        if tree.is_null() {
            return Err("Failed to create subsurface tree for lock surface");
        }

        let lock_surface = Box::into_raw(Box::new(Self {
            tree: crate::scene_handle::SceneTree::watch(tree),
            wlr_lock_surface,
            lock,
            manager,
            idle_update_focus: std::ptr::null_mut(),
            map: std::mem::zeroed(),
            surface_destroy: std::mem::zeroed(),
        }));

        (*wlr_lock_surface).data = lock_surface as *mut _;
        LIVE_LOCK_SURFACES.with(|l| l.borrow_mut().push(lock_surface));

        SceneNodeData::attach(tree as *mut ffi::wlr_scene_node, SceneNodeDataVal::LockSurface(lock_surface));
        ffi::river_wlr_surface_set_data((*wlr_lock_surface).surface, tree as *mut ffi::wlr_scene_node as *mut _);

        (*lock_surface).map.connect(ffi::river_wlr_surface_get_map_signal((*wlr_lock_surface).surface), handle_lock_surface_map);

        (*lock_surface).surface_destroy.connect(&mut (*wlr_lock_surface).events.destroy, handle_lock_surface_destroy);

        (*lock_surface).configure();

        Ok(lock_surface)
    }

    pub unsafe fn destroy(wm: &mut crate::window_manager::WindowManager, lock_surface: *mut Self) {
        let mut new_focus = Focus::None;
        let surfaces_head = &mut (*(*lock_surface).lock).surfaces as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*surfaces_head).next;
        while curr != surfaces_head {
            let next = (*curr).next;
            let wlr_lock_surface = crate::container_of!(curr, ffi::wlr_session_lock_surface_v1, link);
            if wlr_lock_surface != (*lock_surface).wlr_lock_surface {
                let other_surf = (*wlr_lock_surface).data as *mut LockSurface;
                if !other_surf.is_null() {
                    new_focus = Focus::LockSurface(other_surf);
                    break;
                }
            }
            curr = next;
        }

        for seat in wm.seats.ids() {
            let focused = wm.seats.get(seat).map(|s| s.focused);
            if let Some(Focus::LockSurface(focused_surf)) = focused {
                if focused_surf == lock_surface {
                    crate::seat::Seat::focus(wm, seat, new_focus);
                }
            }
            crate::cursor::Cursor::update_state(wm, seat);
        }

        if !(*lock_surface).idle_update_focus.is_null() {
            ffi::wl_event_source_remove((*lock_surface).idle_update_focus);
        }

        (*lock_surface).map.disconnect();
        (*lock_surface).surface_destroy.disconnect();

        ffi::river_wlr_surface_set_data((*(*lock_surface).wlr_lock_surface).surface, std::ptr::null_mut());
        // wlroots unlinks the lock surface only after this destroy handler:
        // a walk of the lock's surfaces before then (another lock surface's
        // own destroy, `lock_surface_from_output`) must not find this box.
        (*(*lock_surface).wlr_lock_surface).data = std::ptr::null_mut();

        LIVE_LOCK_SURFACES.with(|l| l.borrow_mut().retain(|&p| p != lock_surface));
        let _ = Box::from_raw(lock_surface);
    }

    /// The lock surface's wl_surface.
    pub fn surface(&self) -> *mut ffi::wlr_surface {
        // SAFETY: as `wlr_output`.
        unsafe { (*self.wlr_lock_surface).surface }
    }

    /// The `wlr_output` it is on; wlroots destroys a lock surface with its
    /// output, so this is live while the lock surface is.
    pub fn wlr_output(&self) -> *mut ffi::wlr_output {
        // SAFETY: the wlroots lock surface outlives this LockSurface (freed
        // in its destroy handler).
        unsafe { (*self.wlr_lock_surface).output }
    }

    pub fn configure(&self) {
        let mut width: i32 = 0;
        let mut height: i32 = 0;
        // SAFETY: the wlroots lock surface outlives this LockSurface (freed
        // in its destroy handler; the private field is set once, by
        // `create`), and so does the output it is on: wlroots destroys an
        // output's lock surface with the output.
        unsafe {
            ffi::wlr_output_effective_resolution((*self.wlr_lock_surface).output, &mut width, &mut height);
            ffi::wlr_session_lock_surface_v1_configure(self.wlr_lock_surface, width as u32, height as u32);
        }
    }
}

unsafe extern "C" fn handle_lock_surfaces_timeout(_data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    // The server field this timer belongs to; the timer carries no pointer.
    let manager = &mut *std::ptr::addr_of_mut!((*crate::shared::server_ptr()).lock_manager);
    log::error!("waiting for lock surfaces timed out, imperfect frames may be shown");

    assert!(manager.state == LockState::WaitingForLockSurfaces);
    manager.set_state(LockState::WaitingForBlank);

    crate::shared::scene().normal_tree.set_enabled(false);

    let server = crate::shared::server_ptr();
    if manager.maybe_lock(&(*crate::reentry::wm(server)).outputs) {
        (*server).idle.on_locked();
    }

    0
}

/// Start a locker for a session that is locked and has none.
///
/// Forked and detached like every other client the compositor starts (the
/// server's SIGCHLD source reaps it). The new process calls
/// `ext_session_lock_manager_v1.lock()` and `handle_new_lock` hands it the
/// session that is already locked, so the screen never unlocks across the
/// gap — the user just gets a prompt back.
unsafe extern "C" fn handle_respawn_timeout(_data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    // The server field this timer belongs to; the timer carries no pointer.
    let manager = &mut *std::ptr::addr_of_mut!((*crate::shared::server_ptr()).lock_manager);

    // The world may have moved while the timer was armed: a locker of the
    // user's own may have attached, or the session may be unlocked. Either
    // way, spawning now would seize a session nobody asked us to.
    if manager.state == LockState::Unlocked || !manager.lock.is_null() {
        return 0;
    }

    let cmd = cce_lock_cmd();
    log::info!("starting the locker: {}", cmd);
    match crate::process::spawn_sh(&cmd, &[]) {
        Ok(_) => {
            // Arm the NEXT step as a "did it take?" check, and let
            // `handle_new_lock` cancel it when the new locker binds. A
            // forked child proves nothing: the likeliest real failure is a
            // locker that cannot start at all — no PAM stack, no GPU, binary
            // missing — and that one dies without ever creating a
            // `wlr_session_lock_v1`, so no destroy event is coming to
            // trigger another attempt. Without this, the single attempt
            // failed silently and the user stayed locked out with nothing in
            // the log to say why.
            manager.schedule_locker_respawn();
        }
        Err(e) => {
            log::error!("failed to fork the locker respawn: {}", e);
            manager.schedule_locker_respawn();
        }
    }

    0
}

unsafe extern "C" fn handle_new_lock(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let manager = &mut *crate::listener_owner!(listener, LockManager, new_lock);
    let lock = data as *mut ffi::wlr_session_lock_v1;

    log::debug!("session lock client made lock request");

    if !manager.lock.is_null() {
        log::info!("denying new session lock client, an active one already exists");
        ffi::wlr_session_lock_v1_destroy(lock);
        return;
    }

    manager.lock = lock;

    // Someone is holding the session now — whether that is the respawn we
    // asked for or a locker the user started themselves, we must not spawn
    // another on top of it.
    manager.cancel_locker_respawn();

    if manager.state == LockState::Unlocked {
        manager.set_state(LockState::WaitingForLockSurfaces);

        crate::shared::scene().locked_tree.set_enabled(true);

        manager.lock_surfaces_timer.arm(200);

        let server = crate::shared::server_ptr();
        let wm = &mut *crate::reentry::wm(server);
        for seat in wm.seats.ids() {
            crate::seat::Seat::focus_window(wm, seat, None);
        }
    } else {
        if manager.state == LockState::Locked {
            ffi::wlr_session_lock_v1_send_locked(lock);
        }
        log::info!("new session lock client given control of already locked session");
    }

    manager.unlock.connect(&mut (*lock).events.unlock, handle_unlock);

    manager.destroy.connect(&mut (*lock).events.destroy, handle_destroy);

    manager.new_surface.connect(&mut (*lock).events.new_surface, handle_surface);
}

unsafe extern "C" fn handle_unlock(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let manager = &mut *crate::listener_owner!(listener, LockManager, unlock);

    manager.set_state(LockState::Unlocked);
    log::info!("session unlocked");
    // Nobody is waiting for a lock that is over.
    manager.lock_waiters = None;

    // The session is going away legitimately: no respawn is wanted, and the
    // next lock starts with a full budget.
    manager.cancel_locker_respawn();
    manager.respawn_attempts = 0;

    crate::shared::scene().normal_tree.set_enabled(true);
    crate::shared::scene().locked_tree.set_enabled(false);

    let server = crate::shared::server_ptr();
    let wm = &mut *crate::reentry::wm(server);
    for seat in wm.seats.ids() {
        crate::seat::Seat::focus_window(wm, seat, None);
    }

    handle_destroy(manager.destroy.as_ptr(), std::ptr::null_mut());

    crate::shared::pending().dirty_windowing();
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let manager = &mut *crate::listener_owner!(listener, LockManager, destroy);

    log::debug!("ext_session_lock_v1 destroyed");

    manager.new_surface.disconnect();
    manager.unlock.disconnect();
    manager.destroy.disconnect();

    manager.lock = std::ptr::null_mut();
    if manager.state == LockState::WaitingForLockSurfaces {
        manager.set_state(LockState::WaitingForBlank);
        manager.lock_surfaces_timer.arm(0);
    }

    // Reached two ways: from `handle_unlock`, which has already set the state
    // to Unlocked and is just tearing down; or from wlroots because the lock
    // client died. Only the second leaves the user facing a locked screen
    // with nothing to authenticate against.
    if manager.state != LockState::Unlocked {
        manager.schedule_locker_respawn();
    }
}

unsafe extern "C" fn handle_surface(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let manager = &mut *crate::listener_owner!(listener, LockManager, new_surface);
    let wlr_lock_surface = data as *mut ffi::wlr_session_lock_surface_v1;

    log::debug!("new ext_session_lock_surface_v1 created");

    // Far enough to put something on screen, so this is not the startup crash
    // loop the cap exists for: give the next failure a full budget again.
    manager.respawn_attempts = 0;

    assert!(manager.state != LockState::Unlocked);
    assert!(!manager.lock.is_null());

    if LockSurface::create(wlr_lock_surface, manager.lock, manager).is_err() {
        log::error!("out of memory");
        ffi::wl_resource_post_no_memory((*wlr_lock_surface).resource);
    }
}

unsafe extern "C" fn update_focus(data: *mut std::ffi::c_void) {
    let lock_surface = data as *mut LockSurface;
    let server = crate::shared::server_ptr();
    let wm = &mut *crate::reentry::wm(server);

    for seat in wm.seats.ids() {
        let focused = wm.seats.get(seat).map(|s| s.focused);
        if !matches!(focused, Some(Focus::LockSurface(s)) if s == lock_surface) {
            crate::seat::Seat::focus(wm, seat, Focus::LockSurface(lock_surface));
        }
        if let Some(p) = wm.seats.ptr(seat) {
            crate::cursor::Cursor::update_state(wm, (*p).cursor.sid());
        }
    }

    (*lock_surface).idle_update_focus = std::ptr::null_mut();
}

unsafe extern "C" fn handle_lock_surface_map(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let lock_surface = crate::listener_owner!(listener, LockSurface, map);

    let wm = &*crate::reentry::wm(crate::shared::server_ptr());
    if let Some(output) = wm.outputs.find((*lock_surface).wlr_output()).and_then(|id| wm.outputs.get(id)) {
        (*lock_surface).tree.set_position(output.sent.x, output.sent.y);
    }

    let event_loop = crate::shared::event_loop();
    assert!((*lock_surface).idle_update_focus.is_null());

    let idle = ffi::wl_event_loop_add_idle(
        event_loop,
        Some(update_focus),
        lock_surface as *mut _,
    );
    if idle.is_null() {
        log::error!("Failed to create idle update focus event source");
        return;
    }
    (*lock_surface).idle_update_focus = idle;
}

unsafe extern "C" fn handle_lock_surface_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let lock_surface = crate::listener_owner!(listener, LockSurface, surface_destroy);
    let server = crate::shared::server_ptr();
    LockSurface::destroy(&mut *crate::reentry::wm(server), lock_surface);
}
