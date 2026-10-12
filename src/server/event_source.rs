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
//! - A zeroed field (`std::mem::zeroed()`, `Server::default`'s zero-fill)
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
