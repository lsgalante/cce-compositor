// SPDX-License-Identifier: GPL-3.0-only

//! `Timer`: a `wl_event_loop` timer source that is owned by one field and
//! removed when that field is dropped.
//!
//! The compositor kept its timers as raw `*mut wl_event_source` fields, so
//! arming one (`wl_event_source_timer_update`) was an FFI call, and every
//! function that re-armed a timer was `unsafe` for that alone. `Timer` puts
//! the one fact that makes arming sound — a non-null source is live until
//! this field removes it — behind the type:
//!
//! - `add` is the only `unsafe` step: the callback's `data` must stay valid
//!   while the timer can fire.
//! - `arm` / `disarm` are safe, and do nothing on an unset timer, so a timer
//!   whose creation failed (or that was never created) needs no null check
//!   at each use.
//! - `remove` is idempotent and leaves the timer unset, and dropping a
//!   `Timer` removes it, so a struct freed with `Box::from_raw` leaves no
//!   timer behind to fire into freed memory.
//! - A zeroed field (`std::mem::zeroed()`, `Server::zeroed`'s zero-fill)
//!   is a valid unset timer: a null source and `shared: false`.
//! - `on_shared_loop` is the usual way to make one: the callback gets an
//!   integer `token`, never a pointer, and finds what it acts on itself (the
//!   window manager through `reentry::wm`, a seat by id), so nothing has to
//!   outlive the timer. Its one `unsafe` promise is that: the callback is an
//!   event-loop entry point that reads its data as that integer, never as a
//!   pointer. Such a timer is on the compositor's own loop, and
//!   once that loop is shutting down (`Pending::loop_live` false) arming it
//!   does nothing and removing it only forgets the source, so a field
//!   dropped after the display goes cannot touch a freed loop.

use crate::ffi;

/// The callback a timer calls with the `data` it was added with. Its return
/// value is ignored by libwayland for timers.
pub type TimerFn = unsafe extern "C" fn(data: *mut std::ffi::c_void) -> std::ffi::c_int;

pub struct Timer {
    source: *mut ffi::wl_event_source,
    /// On the compositor's own loop (`on_shared_loop`): see the module doc.
    shared: bool,
}

impl Timer {
    /// An unset timer: arming it does nothing.
    pub const fn new() -> Self {
        Timer { source: std::ptr::null_mut(), shared: false }
    }

    /// A timer on the compositor's event loop that calls `callback(token)`
    /// when it fires; it starts disarmed. `token` is an integer the callback
    /// interprets (`0` when it needs none) — never a pointer, so nothing
    /// has to outlive the timer. Unset if the loop is not running.
    ///
    /// # Safety
    /// `callback` must be sound to run from the event loop as an entry point
    /// (it may take the window manager with `reentry::wm`) with `token` as
    /// its data: it reads that data as an integer, never as a pointer.
    pub unsafe fn on_shared_loop(callback: TimerFn, token: usize) -> Self {
        let event_loop = crate::shared::event_loop();
        if event_loop.is_null() || !crate::shared::pending().loop_live() {
            return Self::new();
        }
        // SAFETY: the loop is the live one `Shared::set_server` recorded;
        // libwayland only hands `token` back to `callback`.
        let source = unsafe { ffi::wl_event_loop_add_timer(event_loop, Some(callback), token as *mut std::ffi::c_void) };
        Timer { source, shared: true }
    }

    /// Whether this timer's loop is gone (a shared one after shutdown).
    fn loop_gone(&self) -> bool {
        self.shared && !crate::shared::pending().loop_live()
    }

    /// Add a timer to `event_loop` that calls `callback(data)` when it
    /// fires. It starts disarmed. On failure the timer is unset (check with
    /// `is_set`).
    ///
    /// # Safety
    /// `event_loop` must be live. `data` must be valid for `callback` for as
    /// long as the timer exists: until `remove`, or until the `Timer` is
    /// dropped. The timer must be removed before the event loop is
    /// destroyed.
    pub unsafe fn add(
        event_loop: *mut ffi::wl_event_loop,
        callback: TimerFn,
        data: *mut std::ffi::c_void,
    ) -> Self {
        Timer { source: ffi::wl_event_loop_add_timer(event_loop, Some(callback), data), shared: false }
    }

    /// Whether the timer exists (was added and not yet removed).
    pub fn is_set(&self) -> bool {
        !self.source.is_null()
    }

    /// Fire once after `ms` milliseconds, replacing any pending deadline.
    /// `0` disarms. Does nothing on an unset timer.
    pub fn arm(&self, ms: i32) {
        if self.source.is_null() || self.loop_gone() {
            return;
        }
        // SAFETY: a non-null source is the one `add` created; only `remove`
        // (and so `drop`) releases it, and it nulls the field when it does.
        // `add`'s contract keeps the event loop alive until then.
        unsafe {
            ffi::wl_event_source_timer_update(self.source, ms);
        }
    }

    /// Cancel a pending deadline. Does nothing on an unset timer.
    pub fn disarm(&self) {
        self.arm(0);
    }

    /// Remove the timer from its event loop, if set. Safe to call any
    /// number of times; the timer is left unset.
    pub fn remove(&mut self) {
        let source = std::mem::replace(&mut self.source, std::ptr::null_mut());
        if source.is_null() || self.loop_gone() {
            return;
        }
        // SAFETY: as `arm`; the field is already null, so this is the only
        // removal of `source`.
        unsafe {
            ffi::wl_event_source_remove(source);
        }
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.remove();
    }
}

// A zeroed `Timer` must be an unset one: every field is a pointer or a bool,
// and both are valid all-zero (null, false).

/// The callback an fd source calls when its fd is ready, with the `data`
/// it was added with.
pub type FdFn = unsafe extern "C" fn(fd: std::ffi::c_int, mask: u32, data: *mut std::ffi::c_void) -> std::ffi::c_int;

/// A `wl_event_loop` fd source owned by one field: `Timer`'s pattern for a
/// file descriptor. Removed when the field is dropped or `remove`d; a
/// zeroed one is unset; on the shared loop after shutdown, removing it
/// only forgets the source.
pub struct FdSource {
    source: *mut ffi::wl_event_source,
    shared: bool,
}

impl FdSource {
    /// An unset source.
    pub const fn new() -> Self {
        FdSource { source: std::ptr::null_mut(), shared: false }
    }

    /// Watch `fd` for `mask` (`WL_EVENT_READABLE`, …) on the compositor's
    /// loop, calling `callback(fd, mask, token)`. Unset if the loop is not
    /// running or libwayland fails (check with `is_set`).
    ///
    /// # Safety
    /// As `Timer::on_shared_loop`: `callback` is an event-loop entry point
    /// that reads its data as the integer `token`. And `fd` must stay open
    /// for as long as the source exists: close it only after `remove` (or
    /// after this field is dropped).
    pub unsafe fn on_shared_loop(fd: std::os::fd::RawFd, mask: u32, callback: FdFn, token: usize) -> Self {
        let event_loop = crate::shared::event_loop();
        if event_loop.is_null() || !crate::shared::pending().loop_live() {
            return Self::new();
        }
        // SAFETY: the loop is the live one `Shared::set_server` recorded;
        // libwayland only hands `token` back to `callback`.
        let source = unsafe { ffi::wl_event_loop_add_fd(event_loop, fd, mask, Some(callback), token as *mut std::ffi::c_void) };
        FdSource { source, shared: true }
    }

    /// Watch `fd` on `event_loop` with `data` (the unit tests' private
    /// loops).
    ///
    /// # Safety
    /// As `Timer::add`, and `fd` stays open while the source exists.
    pub unsafe fn add(
        event_loop: *mut ffi::wl_event_loop,
        fd: std::os::fd::RawFd,
        mask: u32,
        callback: FdFn,
        data: *mut std::ffi::c_void,
    ) -> Self {
        FdSource { source: ffi::wl_event_loop_add_fd(event_loop, fd, mask, Some(callback), data), shared: false }
    }

    /// Whether the source exists (was added and not yet removed).
    pub fn is_set(&self) -> bool {
        !self.source.is_null()
    }

    /// Stop watching, if set. Safe to call any number of times.
    pub fn remove(&mut self) {
        let source = std::mem::replace(&mut self.source, std::ptr::null_mut());
        if source.is_null() || (self.shared && !crate::shared::pending().loop_live()) {
            return;
        }
        // SAFETY: a non-null source is the one `add`/`on_shared_loop`
        // created, on a loop that is still live; the field is already null,
        // so this is its only removal.
        unsafe {
            ffi::wl_event_source_remove(source);
        }
    }
}

impl Default for FdSource {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FdSource {
    fn drop(&mut self) {
        self.remove();
    }
}

/// The callback an idle source calls once, with the `data` it was added
/// with.
pub type IdleFn = unsafe extern "C" fn(data: *mut std::ffi::c_void);

/// A coalesced one-shot deferred call: however many times `schedule` is
/// called before the loop next goes idle, the callback runs once. An idle
/// source frees itself after it runs, so the field cannot own it the way a
/// `Timer` owns a timer: the callback's first act is `ran()`, which forgets
/// it. Usable through `&self` (the field is a `Cell`); a zeroed one is
/// unset.
pub struct IdleOnce {
    source: std::cell::Cell<*mut ffi::wl_event_source>,
    shared: std::cell::Cell<bool>,
}

impl IdleOnce {
    pub const fn new() -> Self {
        IdleOnce { source: std::cell::Cell::new(std::ptr::null_mut()), shared: std::cell::Cell::new(false) }
    }

    /// Whether a call is scheduled and has not run yet.
    pub fn pending(&self) -> bool {
        !self.source.get().is_null()
    }

    /// Schedule `callback(token)` on the compositor's loop, unless one is
    /// already pending. False when nothing is pending afterwards (no
    /// running loop, or libwayland failed).
    ///
    /// # Safety
    /// `callback` is an event-loop entry point that reads its data as the
    /// integer `token`, and calls `ran()` on this same `IdleOnce` before
    /// anything that could `schedule` or `cancel` it.
    pub unsafe fn schedule(&self, callback: IdleFn, token: usize) -> bool {
        if self.pending() {
            return true;
        }
        let event_loop = crate::shared::event_loop();
        if event_loop.is_null() || !crate::shared::pending().loop_live() {
            return false;
        }
        // SAFETY: the loop is the live one `Shared::set_server` recorded.
        unsafe { self.schedule_on(event_loop, callback, token as *mut std::ffi::c_void, true) }
    }

    /// `schedule` on `event_loop` with `data` (the unit tests' private
    /// loops).
    ///
    /// # Safety
    /// `event_loop` is live until the call runs or is cancelled, `data` is
    /// valid for `callback` until then, and `callback` calls `ran()` first.
    pub unsafe fn schedule_on(&self, event_loop: *mut ffi::wl_event_loop, callback: IdleFn, data: *mut std::ffi::c_void, shared: bool) -> bool {
        if self.pending() {
            return true;
        }
        let source = ffi::wl_event_loop_add_idle(event_loop, Some(callback), data);
        self.source.set(source);
        self.shared.set(shared);
        !source.is_null()
    }

    /// The callback has started: libwayland frees the source after it
    /// returns, so it is forgotten here.
    pub fn ran(&self) {
        self.source.set(std::ptr::null_mut());
    }

    /// Drop a pending call. Safe to call any number of times.
    pub fn cancel(&self) {
        let source = self.source.replace(std::ptr::null_mut());
        if source.is_null() || (self.shared.get() && !crate::shared::pending().loop_live()) {
            return;
        }
        // SAFETY: a recorded source has not run (`ran` forgets it first
        // thing, per `schedule`'s contract), so it is still on its live
        // loop; the field is already null, so this is its only removal.
        unsafe {
            ffi::wl_event_source_remove(source);
        }
    }
}

impl Default for IdleOnce {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for IdleOnce {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    unsafe extern "C" fn bump(data: *mut std::ffi::c_void) -> std::ffi::c_int {
        (*(data as *const AtomicUsize)).fetch_add(1, Ordering::SeqCst);
        0
    }

    /// Run the loop for up to `ms`, returning once a source has dispatched.
    fn dispatch(event_loop: *mut ffi::wl_event_loop, ms: i32) {
        unsafe {
            ffi::wl_event_loop_dispatch(event_loop, ms);
        }
    }

    #[test]
    fn unset_timer_ignores_arm_and_remove() {
        let mut timer = Timer::new();
        assert!(!timer.is_set());
        timer.arm(10);
        timer.disarm();
        timer.remove();
        timer.remove();
        assert!(!timer.is_set());
        // A zeroed timer is an unset one.
        let zeroed: Timer = unsafe { std::mem::zeroed() };
        assert!(!zeroed.is_set());
        zeroed.arm(10);
    }

    #[test]
    fn armed_timer_fires_once_and_disarm_cancels() {
        let event_loop = unsafe { ffi::wl_event_loop_create() };
        assert!(!event_loop.is_null());
        let fired = AtomicUsize::new(0);
        let mut timer = unsafe { Timer::add(event_loop, bump, &fired as *const _ as *mut _) };
        assert!(timer.is_set());

        timer.arm(1);
        dispatch(event_loop, 1000);
        assert_eq!(fired.load(Ordering::SeqCst), 1);

        // Fired once; nothing re-arms it.
        dispatch(event_loop, 20);
        assert_eq!(fired.load(Ordering::SeqCst), 1);

        timer.arm(5);
        timer.disarm();
        dispatch(event_loop, 30);
        assert_eq!(fired.load(Ordering::SeqCst), 1);

        timer.remove();
        assert!(!timer.is_set());
        timer.remove();
        unsafe { ffi::wl_event_loop_destroy(event_loop) };
    }

    unsafe extern "C" fn read_byte(fd: std::ffi::c_int, _mask: u32, data: *mut std::ffi::c_void) -> std::ffi::c_int {
        let mut b = [0u8; 1];
        libc::read(fd, b.as_mut_ptr() as *mut _, 1);
        (*(data as *const AtomicUsize)).fetch_add(1, Ordering::SeqCst);
        0
    }

    #[test]
    fn fd_source_fires_when_readable_and_stops_once_removed() {
        let event_loop = unsafe { ffi::wl_event_loop_create() };
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let fired = AtomicUsize::new(0);
        let mut source = unsafe {
            FdSource::add(event_loop, fds[0], ffi::WL_EVENT_READABLE as u32, read_byte, &fired as *const _ as *mut _)
        };
        assert!(source.is_set());
        unsafe { libc::write(fds[1], b"x".as_ptr() as *const _, 1) };
        dispatch(event_loop, 1000);
        assert_eq!(fired.load(Ordering::SeqCst), 1);
        source.remove();
        source.remove();
        assert!(!source.is_set());
        unsafe { libc::write(fds[1], b"x".as_ptr() as *const _, 1) };
        dispatch(event_loop, 30);
        assert_eq!(fired.load(Ordering::SeqCst), 1);
        // A zeroed source is an unset one.
        let mut zeroed: FdSource = unsafe { std::mem::zeroed() };
        assert!(!zeroed.is_set());
        zeroed.remove();
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
            ffi::wl_event_loop_destroy(event_loop);
        }
    }

    struct Once {
        idle: IdleOnce,
        runs: AtomicUsize,
    }

    unsafe extern "C" fn once_ran(data: *mut std::ffi::c_void) {
        let once = &*(data as *const Once);
        once.idle.ran();
        once.runs.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn idle_once_coalesces_runs_once_and_cancels() {
        let event_loop = unsafe { ffi::wl_event_loop_create() };
        let once = Once { idle: IdleOnce::new(), runs: AtomicUsize::new(0) };
        let data = &once as *const Once as *mut std::ffi::c_void;
        unsafe {
            assert!(once.idle.schedule_on(event_loop, once_ran, data, false));
            assert!(once.idle.schedule_on(event_loop, once_ran, data, false));
            ffi::wl_event_loop_dispatch_idle(event_loop);
        }
        assert_eq!(once.runs.load(Ordering::SeqCst), 1);
        assert!(!once.idle.pending());
        unsafe {
            ffi::wl_event_loop_dispatch_idle(event_loop);
        }
        assert_eq!(once.runs.load(Ordering::SeqCst), 1);
        // Cancelled before the loop idles: it never runs.
        unsafe { assert!(once.idle.schedule_on(event_loop, once_ran, data, false)) };
        once.idle.cancel();
        once.idle.cancel();
        unsafe { ffi::wl_event_loop_dispatch_idle(event_loop) };
        assert_eq!(once.runs.load(Ordering::SeqCst), 1);
        let zeroed: IdleOnce = unsafe { std::mem::zeroed() };
        assert!(!zeroed.pending());
        zeroed.cancel();
        unsafe { ffi::wl_event_loop_destroy(event_loop) };
    }

    #[test]
    fn dropping_an_armed_timer_removes_it() {
        let event_loop = unsafe { ffi::wl_event_loop_create() };
        let fired = AtomicUsize::new(0);
        {
            let timer = unsafe { Timer::add(event_loop, bump, &fired as *const _ as *mut _) };
            timer.arm(1);
        }
        dispatch(event_loop, 30);
        assert_eq!(fired.load(Ordering::SeqCst), 0);
        unsafe { ffi::wl_event_loop_destroy(event_loop) };
    }
}
