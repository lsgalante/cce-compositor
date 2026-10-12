// SPDX-License-Identifier: GPL-3.0-only

//! `Listener`: a `wl_listener` that is linked into at most one signal,
//! unlinks itself when it is dropped, and lives at a fixed heap address no
//! matter where its owner moves.
//!
//! The compositor embeds listeners in its structs, as wlroots code does.
//! Until 2026-10-12 a `Listener` WAS its `wl_listener` (`repr(transparent)`),
//! and a callback recovered its struct with `container_of!` from the
//! listener's address. That tied the signal list to the owner's address: a
//! safe `mem::swap` of two seats (two `&mut Seat` from `wm.seats`) moved
//! their linked listeners, and libwayland walked into the other seat's
//! list. Now each `Listener` owns a heap cell holding the `wl_listener` and
//! a record of how to find its owner, so moving a `Listener` — or the
//! struct around it — never touches a signal list.
//!
//! A callback finds its owner through the cell, in one of two ways:
//!
//! - **By field** (`listener_owner!`), for an owner nothing safe can move
//!   (an [`Anchored`] type: reachable only through raw pointers, or a part
//!   of the leaked `Server`). `connect` records the address of the field it
//!   was called on; the macro checks the field there still holds this cell
//!   and returns the struct around it, as `container_of!` did.
//! - **By key** (`listener_id!`), for an owner safe code CAN move — one a
//!   store hands out as `&mut` or `Box` (`Window`, `Output`, `Seat` and the
//!   `Cursor` and `InputRelay` inside it). `connect_keyed` records the
//!   owner's store id instead of any address; the macro looks the id up in
//!   the store and checks the owner found there holds this cell. Nothing is
//!   dereferenced that a move could have left stale.
//!
//! Either check failing means an owner moved after it connected, which the
//! types are built to rule out: the callback logs and aborts
//! ([`misrouted`]) rather than run on the wrong object.
//!
//! A field-routed listener must also stay in its FIELD while connected.
//! Moved out (`mem::take` / `replace` / `swap` of the `Listener` itself)
//! and kept past its owner's free, its cell still names the freed field,
//! and the route reads freed memory. So every `Listener` field is private
//! to its owner's module, where `connect` / `disconnect` are all that is
//! done to one — never move one.
//!
//! - `connect` unlinks first, so connecting a listener that is already
//!   connected moves it rather than linking it into a second list, which
//!   corrupts both.
//! - `disconnect` is idempotent, and dropping a `Listener` disconnects it
//!   and frees its cell, so a struct freed with `Box::from_raw` leaves no
//!   listener behind in a signal it outlived. A callback may drop its own
//!   listener: every libwayland emit is done with a listener once its
//!   `notify` returns.
//! - A zeroed `Listener` (what `std::mem::zeroed()` and `Server::zeroed`'s
//!   zero-fill produce) is a valid unconnected one with no cell yet.

use crate::ffi;
use crate::server::{wl_list_insert, wl_list_remove, WlList};

/// The callback a signal calls: the listener it was connected through, and
/// the signal's data.
pub type Notify = unsafe extern "C" fn(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void);

/// How a callback finds the struct its listener belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Owner {
    /// Not recorded: never connected, nor handed out by `as_ptr`.
    Unset,
    /// The `Listener` field the cell was connected through, at that moment.
    Field(*const Listener),
    /// The owner's id in its store (`ListenerKey::to_key`).
    Key(u64),
}

/// The heap half of a `Listener`. `raw` comes first: the pointer a signal
/// hands its callback is the cell's own address.
#[repr(C)]
struct Cell {
    raw: ffi::wl_listener,
    owner: Owner,
}

/// A `wl_listener` at a fixed heap address; see the module documentation.
/// Null until first connected.
#[repr(transparent)]
pub struct Listener(*mut Cell);

impl Listener {
    /// An unconnected listener.
    pub const fn new() -> Self {
        Listener(std::ptr::null_mut())
    }

    /// The cell, made on first use.
    fn cell(&mut self) -> *mut Cell {
        if self.0.is_null() {
            self.0 = Box::into_raw(Box::new(Cell {
                raw: ffi::wl_listener {
                    link: ffi::wl_list { prev: std::ptr::null_mut(), next: std::ptr::null_mut() },
                    notify: None,
                },
                owner: Owner::Unset,
            }));
        }
        self.0
    }

    /// Whether this listener is linked into a signal.
    pub fn is_connected(&self) -> bool {
        if self.0.is_null() {
            return false;
        }
        // SAFETY: a non-null cell is ours until drop.
        let link = unsafe { &(*self.0).raw.link };
        !link.prev.is_null() && !link.next.is_null() && link.prev as *const ffi::wl_list != link as *const ffi::wl_list
    }

    /// Whether `listener` — what a signal handed a callback — is this
    /// listener's cell.
    pub fn holds(&self, listener: *mut ffi::wl_listener) -> bool {
        !self.0.is_null() && self.0 as *mut ffi::wl_listener == listener
    }

    unsafe fn link_into(&mut self, signal: *mut ffi::wl_signal, notify: Notify, owner: Owner) {
        self.disconnect();
        if signal.is_null() {
            log::error!("Listener::connect: signal is null");
            return;
        }
        let cell = self.cell();
        (*cell).raw.notify = Some(notify);
        (*cell).owner = owner;
        let list = &mut (*signal).listener_list as *mut ffi::wl_list as *mut WlList;
        wl_list_insert((*list).prev, &mut (*cell).raw.link as *mut ffi::wl_list as *mut WlList);
    }

    /// Connect to `signal`, calling `notify` when it is emitted; the
    /// callback finds its owner with `listener_owner!`. A listener already
    /// connected is disconnected first. A null `signal` is logged and
    /// leaves the listener unconnected.
    ///
    /// # Safety
    /// `signal` must be a live `wl_signal`, and the listener must be
    /// disconnected or dropped before the signal is destroyed — wlroots
    /// asserts its objects' signals have no listeners left when it frees
    /// them, so a destroy handler disconnects the struct's listeners.
    /// `self` must be a field of an [`Anchored`] struct, which is where the
    /// callback will look for it, and must stay in that field until it is
    /// disconnected: not moved out by `mem::take` / `replace` / `swap`.
    pub unsafe fn connect(&mut self, signal: *mut ffi::wl_signal, notify: Notify) {
        let field = self as *const Listener;
        self.link_into(signal, notify, Owner::Field(field));
    }

    /// Connect to `signal` as `connect` does, recording the owner's store
    /// id `key` instead of where the listener is: the callback finds its
    /// owner with `listener_id!`, so the owner may move.
    ///
    /// # Safety
    /// As `connect`, except the owner need not be `Anchored`: while the
    /// listener is connected the owner must be in the store `key` names it
    /// in, or the callback aborts.
    pub unsafe fn connect_keyed(&mut self, signal: *mut ffi::wl_signal, notify: Notify, key: impl ListenerKey) {
        self.link_into(signal, notify, Owner::Key(key.to_key()));
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
        let field = self as *const Listener;
        let cell = self.cell();
        (*cell).raw.notify = Some(notify);
        (*cell).owner = Owner::Field(field);
        &mut (*cell).raw
    }

    /// Unlink from the signal, if connected. Safe to call any number of
    /// times; the link is left null and the cell kept for a reconnect.
    pub fn disconnect(&mut self) {
        if self.0.is_null() {
            return;
        }
        // SAFETY: the cell is ours. A listener is only ever linked by
        // `connect` / `connect_keyed` / `prepare`, whose contract keeps its
        // signal alive until this runs, so the neighbours named by `prev`
        // and `next` are live list nodes. A null link (never connected) and
        // a self-loop (initialised but not in a list — what
        // `wl_priv_signal_final_emit` leaves) have no neighbours to touch.
        unsafe {
            let link = &mut (*self.0).raw.link as *mut ffi::wl_list as *mut WlList;
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

    /// The raw listener, for a handler called directly: unconnected, it is
    /// recorded as found through this field, as `connect` would.
    pub fn as_ptr(&mut self) -> *mut ffi::wl_listener {
        let connected = self.is_connected();
        let field = self as *const Listener;
        let cell = self.cell();
        // SAFETY: the cell is ours until drop.
        unsafe {
            if !connected && !matches!((*cell).owner, Owner::Key(_)) {
                (*cell).owner = Owner::Field(field);
            }
            &mut (*cell).raw
        }
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
        if !self.0.is_null() {
            // SAFETY: made by `Box::into_raw` in `cell` and freed only here;
            // unlinked above, so no signal still names it.
            drop(unsafe { Box::from_raw(self.0) });
            self.0 = std::ptr::null_mut();
        }
    }
}

/// A struct whose listeners find it by field (`connect`, `listener_owner!`).
///
/// # Safety
/// Once one of its listeners is connected, a value of the type must stay
/// where it is until the listener is disconnected or dropped: no safe code
/// may be able to move it, so no safe API may hand out a `&mut` to one
/// alongside another (a `mem::swap`), a value of the type (a
/// `mem::replace` / `take`), or a `Box` of one (moved out of with `*`).
/// Implement it for a type that is reached only through raw pointers, or
/// that lives in the one `Server` (whose constructor, `Server::zeroed`, is
/// unsafe for this reason); an owner a store hands out as `&mut` keys its
/// listeners instead (`connect_keyed`, `listener_id!`). Its `Listener`
/// fields are private, for `connect`'s sake. A linked `wl_list` head or
/// link needs the same of whatever holds it, and is private too (other
/// modules use raw-pointer accessors: `*_head()`, `from_link`). In a
/// movable owner a head is boxed (`Seat::keyboard_groups`,
/// `InputRelay::input_popups`), and the owner has no safe constructor or
/// safe drop (`InputRelay::new`, `Seats::take` are unsafe): a box survives
/// a move, not a free.
pub unsafe trait Anchored {}

/// A store id a keyed listener records (`connect_keyed`, `listener_id!`).
pub trait ListenerKey: Copy {
    fn to_key(self) -> u64;
    fn from_key(key: u64) -> Self;
}

/// A slot-map key (`WindowId`, `OutputId`) as a listener key: generation
/// high, index low.
pub fn slot_key_bits(key: crate::slotmap::Key) -> u64 {
    (key.generation as u64) << 32 | key.index as u64
}

/// `slot_key_bits` undone.
pub fn slot_key_from_bits(bits: u64) -> crate::slotmap::Key {
    crate::slotmap::Key { generation: (bits >> 32) as u32, index: bits as u32 }
}

/// The field a field-routed `listener` was connected through, if that
/// field still holds it: `None` when it was keyed, or its owner moved.
/// `listener_owner!` is the way to call it.
///
/// # Safety
/// `listener` is what a signal handed a callback (or `as_ptr` returned),
/// and the struct it was connected from has not been freed: an `Anchored`
/// owner frees itself only after its listeners are gone.
pub unsafe fn field_of(listener: *mut ffi::wl_listener) -> Option<*const Listener> {
    let cell = listener as *mut Cell;
    match (*cell).owner {
        Owner::Field(field) if (*field).0 == cell => Some(field),
        _ => None,
    }
}

/// The store id a keyed `listener` was connected with, or `None` for one
/// connected by field. `listener_id!` is the way to call it.
///
/// # Safety
/// `listener` is what a signal handed a callback.
pub unsafe fn key_of(listener: *mut ffi::wl_listener) -> Option<u64> {
    match (*(listener as *mut Cell)).owner {
        Owner::Key(key) => Some(key),
        _ => None,
    }
}

/// A callback whose listener's owner is not where it connected from: an
/// owner moved, which corrupts state if the callback runs on. Log and stop.
#[cold]
pub fn misrouted(what: &str) -> ! {
    log::error!("listener {what}: its owner moved or is gone after it connected; aborting");
    eprintln!("cce-fx: listener {what}: its owner moved or is gone after it connected; aborting");
    std::process::abort()
}

/// The `$owner` whose `$field` (a `Listener`) a signal called back through,
/// as `*mut $owner`; aborts (`misrouted`) if that field no longer holds the
/// listener. `$owner` must be `Anchored`: a type safe code can move keys its
/// listeners instead. Unsafe: see `listener::field_of`.
#[macro_export]
macro_rules! listener_owner {
    ($listener:expr, $owner:path, $field:ident) => {{
        fn _anchored<T: $crate::listener::Anchored>() {}
        _anchored::<$owner>();
        let _: fn(&$owner) -> &$crate::listener::Listener = |o| &o.$field;
        match $crate::listener::field_of($listener) {
            Some(field) => $crate::container_of!(field, $owner, $field),
            None => $crate::listener::misrouted(concat!(stringify!($owner), "::", stringify!($field))),
        }
    }};
}

/// The id of the owner a keyed listener belongs to: `$store.get(id)` must
/// find an owner whose `$($field)+` holds `$listener`, or this aborts
/// (`misrouted`). Unsafe: see `listener::key_of`.
///
/// `listener_id!(listener, wm.seats, SeatId, cursor.motion_listener)`
#[macro_export]
macro_rules! listener_id {
    ($listener:expr, $store:expr, $id:ty, $($field:tt)+) => {{
        let listener: *mut $crate::ffi::wl_listener = $listener;
        let id = match $crate::listener::key_of(listener) {
            Some(key) => <$id as $crate::listener::ListenerKey>::from_key(key),
            None => $crate::listener::misrouted(stringify!($($field)+)),
        };
        match $store.get(id) {
            Some(owner) if owner.$($field)+.holds(listener) => id,
            _ => $crate::listener::misrouted(stringify!($($field)+)),
        }
    }};
}

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

    fn emit(s: &mut ffi::wl_signal) {
        unsafe { ffi::wl_signal_emit_mutable(s, std::ptr::null_mut()) };
    }

    // Per thread: libtest runs tests in parallel, and a shared counter let
    // one test's emits land between another's read and its assert.
    thread_local! {
        static CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    fn calls() -> usize {
        CALLS.with(|c| c.get())
    }
    unsafe extern "C" fn bump(_l: *mut ffi::wl_listener, _d: *mut std::ffi::c_void) {
        CALLS.with(|c| c.set(c.get() + 1));
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
        let before = calls();
        emit(&mut s);
        assert_eq!(calls(), before + 1);
    }

    /// Two owners, each with a listener on its own signal.
    struct Owned {
        tag: u32,
        hook: Listener,
    }
    unsafe impl Anchored for Owned {}

    static SEEN: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn record(l: *mut ffi::wl_listener, _d: *mut std::ffi::c_void) {
        if let Some(field) = field_of(l) {
            let owner = crate::container_of!(field, Owned, hook);
            SEEN.store((*owner).tag as usize, Ordering::SeqCst);
        } else {
            SEEN.store(usize::MAX, Ordering::SeqCst);
        }
    }

    #[test]
    fn swapping_owners_leaves_both_signal_lists_whole() {
        let (mut s1, mut s2) = (signal(), signal());
        let mut a = Owned { tag: 1, hook: Listener::new() };
        let mut b = Owned { tag: 2, hook: Listener::new() };
        unsafe {
            a.hook.connect(&mut *s1, bump);
            b.hook.connect(&mut *s2, bump);
        }
        // What a safe `&mut` pair from a store allows. With the listener
        // inline this cross-linked the two lists.
        std::mem::swap(&mut a, &mut b);
        assert_eq!((count(&s1), count(&s2)), (1, 1));
        assert!(a.hook.is_connected() && b.hook.is_connected());
        let before = calls();
        emit(&mut s1);
        emit(&mut s2);
        assert_eq!(calls(), before + 2);
        drop(a);
        assert_eq!((count(&s1), count(&s2)), (1, 0));
        drop(b);
        assert_eq!(count(&s1), 0);
    }

    #[test]
    fn a_field_route_finds_its_owner_and_refuses_a_moved_one() {
        let mut s = signal();
        let mut a = Box::new(Owned { tag: 7, hook: Listener::new() });
        unsafe { a.hook.connect(&mut *s, record) };
        emit(&mut s);
        assert_eq!(SEEN.load(Ordering::SeqCst), 7);

        // Swap the contents of two boxes: the cell's recorded field now
        // holds the other owner's listener, and the route says so.
        let mut b = Box::new(Owned { tag: 8, hook: Listener::new() });
        std::mem::swap(&mut *a, &mut *b);
        emit(&mut s);
        assert_eq!(SEEN.load(Ordering::SeqCst), usize::MAX);
        drop(b);
        assert_eq!(count(&s), 0);
    }

    #[derive(Clone, Copy, PartialEq, Debug)]
    struct Id(u32);
    impl ListenerKey for Id {
        fn to_key(self) -> u64 {
            self.0 as u64
        }
        fn from_key(key: u64) -> Self {
            Id(key as u32)
        }
    }

    #[test]
    fn a_keyed_listener_records_its_key_and_no_field() {
        let mut s = signal();
        let mut a = Listener::new();
        unsafe { a.connect_keyed(&mut *s, bump, Id(42)) };
        let raw = a.as_ptr();
        assert!(a.holds(raw));
        unsafe {
            assert_eq!(key_of(raw).map(Id::from_key), Some(Id(42)));
            assert_eq!(field_of(raw), None);
        }
        // Moving a keyed listener is harmless: nothing recorded where it was.
        let moved = a;
        assert!(moved.is_connected());
        unsafe { assert_eq!(key_of(raw), Some(42)) };
        drop(moved);
        assert_eq!(count(&s), 0);
    }

    #[test]
    fn a_callback_may_drop_its_own_listener() {
        thread_local! {
            static SLOT: std::cell::RefCell<Option<Box<Listener>>> = const { std::cell::RefCell::new(None) };
        }
        unsafe extern "C" fn drop_self(_l: *mut ffi::wl_listener, _d: *mut std::ffi::c_void) {
            // The destroy-handler shape: the owner, and with it this very
            // listener's cell, is freed inside its own notify.
            let mine = SLOT.with(|s| s.borrow_mut().take());
            drop(mine);
        }
        let mut s = signal();
        let mut l = Box::new(Listener::new());
        let mut other = Box::new(Listener::new());
        unsafe {
            l.connect(&mut *s, drop_self);
            other.connect(&mut *s, bump);
        }
        SLOT.with(|s| *s.borrow_mut() = Some(l));
        let before = calls();
        emit(&mut s);
        assert_eq!(calls(), before + 1);
        assert_eq!(count(&s), 1);
        drop(other);
    }
}
