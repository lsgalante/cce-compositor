// SPDX-License-Identifier: GPL-3.0-only

//! `Listener`: a `wl_listener` that is linked into at most one signal and
//! unlinks itself when it is dropped.
//!
//! The compositor embeds listeners in its structs and recovers the struct in
//! the callback with `container_of!`, as wlroots code does, and that does not
//! change: `Listener` is `repr(transparent)` over `ffi::wl_listener`, so a
//! field of this type sits where the raw one did and `container_of!` finds it
//! the same way. What it replaces is the hand-rolled registration around it:
//! casting the field to `WlListener` to set `notify`, `wl_signal_add`, and a
//! matching `wl_listener_remove` that every teardown path had to remember.
//!
//! - `connect` unlinks first, so connecting a listener that is already
//!   connected moves it rather than linking it into a second list, which
//!   corrupts both.
//! - `disconnect` is idempotent, and dropping a `Listener` disconnects it,
//!   so a struct freed with `Box::from_raw` leaves no listener behind in a
//!   signal it outlived.
//! - A zeroed `Listener` (what `std::mem::zeroed()` and `Server::default`'s
//!   zero-fill produce) is a valid unconnected one.

use crate::ffi;
use crate::server::{wl_list_insert, wl_list_remove, WlList, WlListener};

/// The callback a signal calls: the listener it was connected through, and
/// the signal's data.
pub type Notify = unsafe extern "C" fn(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void);

#[repr(transparent)]
pub struct Listener(ffi::wl_listener);

impl Listener {
    /// An unconnected listener.
    pub const fn new() -> Self {
        Listener(ffi::wl_listener {
            link: ffi::wl_list { prev: std::ptr::null_mut(), next: std::ptr::null_mut() },
            notify: None,
        })
    }

    fn link(&mut self) -> *mut WlList {
        &mut self.0.link as *mut ffi::wl_list as *mut WlList
    }

    /// Whether this listener is linked into a signal.
    pub fn is_connected(&self) -> bool {
        let link = &self.0.link as *const ffi::wl_list;
        !self.0.link.prev.is_null()
            && !self.0.link.next.is_null()
            && self.0.link.prev as *const ffi::wl_list != link
    }

    /// Connect to `signal`, calling `notify` when it is emitted. A listener
    /// already connected is disconnected first. A null `signal` is logged and
    /// leaves the listener unconnected.
    ///
    /// # Safety
    /// `signal` must be a live `wl_signal`. While connected, `self` must not
    /// move (the signal's list holds its address), and it must be
    /// disconnected or dropped before the signal is destroyed — wlroots
    /// asserts its objects' signals have no listeners left when it frees
    /// them, so a destroy handler disconnects the struct's listeners.
    pub unsafe fn connect(&mut self, signal: *mut ffi::wl_signal, notify: Notify) {
        self.disconnect();
        if signal.is_null() {
            log::error!("Listener::connect: signal is null");
            return;
        }
        self.0.notify = Some(notify);
        let list = &mut (*signal).listener_list as *mut ffi::wl_list as *mut WlList;
        wl_list_insert((*list).prev, self.link());
    }

    /// Set `notify` and hand back the raw listener, disconnected, for a
    /// registration call that takes one: `wl_display_add_destroy_listener`,
    /// `wl_resource_add_destroy_listener`, `wl_client_add_destroy_listener`,
    /// `wl_event_loop_add_destroy_listener`.
    ///
    /// # Safety
    /// As `connect`, for the signal the registration call links it into.
    pub unsafe fn prepare(&mut self, notify: Notify) -> *mut ffi::wl_listener {
        self.disconnect();
        self.0.notify = Some(notify);
        &mut self.0
    }

    /// Unlink from the signal, if connected. Safe to call any number of
    /// times; the link is left null.
    pub fn disconnect(&mut self) {
        let link = self.link();
        // SAFETY: a listener is only ever linked by `connect` / `prepare`,
        // whose contract keeps its signal alive until this runs, so the
        // neighbours named by `prev` and `next` are live list nodes. A null
        // link (never connected, or zeroed) and a self-loop (initialised
        // but not in a list) have no neighbours to touch.
        unsafe {
            if (*link).prev.is_null() || (*link).next.is_null() {
                return;
            }
            if (*link).prev == link {
                (*link).prev = std::ptr::null_mut();
                (*link).next = std::ptr::null_mut();
                return;
            }
            wl_list_remove(link);
        }
    }

    /// The raw listener, for the `container_of!` comparisons and the few
    /// C calls that want one.
    pub fn as_ptr(&mut self) -> *mut ffi::wl_listener {
        &mut self.0
    }
}

impl Default for Listener {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.disconnect();
    }
}

// `WlListener` is the hand-written mirror the old registrations cast to; the
// two must stay the same shape for `connect` to link what C reads.
const _: () = assert!(std::mem::size_of::<WlListener>() == std::mem::size_of::<ffi::wl_listener>());

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn signal() -> Box<ffi::wl_signal> {
        let mut s: Box<ffi::wl_signal> = Box::new(unsafe { std::mem::zeroed() });
        let head = &mut s.listener_list as *mut ffi::wl_list as *mut WlList;
        unsafe {
            (*head).prev = head;
            (*head).next = head;
        }
        s
    }

    fn count(s: &ffi::wl_signal) -> usize {
        let head = &s.listener_list as *const ffi::wl_list;
        let mut n = 0;
        let mut cur = s.listener_list.next as *const ffi::wl_list;
        while cur != head {
            n += 1;
            cur = unsafe { (*cur).next };
        }
        n
    }

    static CALLS: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn bump(_l: *mut ffi::wl_listener, _d: *mut std::ffi::c_void) {
        CALLS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn connecting_twice_links_once_and_dropping_unlinks() {
        let mut s = signal();
        let mut a = Box::new(Listener::new());
        assert!(!a.is_connected());
        unsafe {
            a.connect(&mut *s, bump);
            a.connect(&mut *s, bump);
        }
        assert!(a.is_connected());
        assert_eq!(count(&s), 1);
        let mut b = Box::new(Listener::new());
        unsafe { b.connect(&mut *s, bump) };
        assert_eq!(count(&s), 2);
        drop(a);
        assert_eq!(count(&s), 1);
        b.disconnect();
        b.disconnect();
        assert_eq!(count(&s), 0);
        assert!(!b.is_connected());
    }

    #[test]
    fn a_zeroed_listener_is_unconnected_and_drops_cleanly() {
        let z: Listener = unsafe { std::mem::zeroed() };
        assert!(!z.is_connected());
        drop(z);
    }

    #[test]
    fn an_emitted_signal_calls_the_listener() {
        let mut s = signal();
        let mut a = Box::new(Listener::new());
        unsafe { a.connect(&mut *s, bump) };
        let before = CALLS.load(Ordering::SeqCst);
        unsafe { ffi::wl_signal_emit_mutable(&mut *s, std::ptr::null_mut()) };
        assert_eq!(CALLS.load(Ordering::SeqCst), before + 1);
    }
}
