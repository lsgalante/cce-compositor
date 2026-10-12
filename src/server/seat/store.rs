// SPDX-License-Identifier: GPL-3.0-only

//! [`Seats`]: the one owner of every [`Seat`].
//!
//! Seats are created at startup ("default") or when a client asks for one by
//! name, and destroyed when a client asks for one to go (the manage pass
//! takes it out) or when the server shuts down. They are few, so the store
//! is a vector of slots and a [`SeatId`] is an index into it; a taken
//! seat's slot stays empty and is never reused, so an id that outlives its
//! seat looks up as `None`. It replaced the intrusive
//! `input_manager.seats` list, and [`Seat::destroy`] now takes the box the
//! store hands back ([`Seats::take`]) instead of freeing an address — a
//! seat used to free itself from inside its own `manage_start`.
//!
//! Each seat sits in its own heap allocation, which never moves while the
//! seat lives. The store keeps raw pointers rather than `Box`es for the
//! reason the window store gives. Nothing outside names a seat by that
//! address: its listeners are keyed by id, and a wlroots callback handed a
//! `wlr_seat` finds the seat that holds it ([`Seats::find`]; the
//! `wlr_seat`'s own `data` is unused). Safe code can swap two seats'
//! contents (`values_mut`), and the hold moves with them, so the holder
//! is always the right seat.
//!
//! [`place_holding`] does the same without the store, for a teardown that
//! runs inside window-manager calls (the pointer constraint's): every live
//! seat's place is also in a per-thread registry, kept by `insert` and
//! `take`.

use super::{Seat, SeatId};
use std::cell::RefCell;
use std::ptr::NonNull;

thread_local! {
    /// Every live seat's place, for [`place_holding`]: pushed by
    /// [`Seats::insert`], removed by [`Seats::take`] before the box goes.
    static PLACES: RefCell<Vec<NonNull<Seat>>> = const { RefCell::new(Vec::new()) };
}

/// Every live seat's place, copied out, found without the store: for
/// teardown that must reach every seat with no window manager (an input
/// popup leaving whichever relay holds it). Dereferencing one is the
/// caller's business, under [`place_holding`]'s rule.
pub fn live_places() -> Vec<*mut Seat> {
    PLACES.with(|places| places.borrow().iter().map(|p| p.as_ptr()).collect())
}

/// The place of the live seat that holds `wlr_seat` (null: `None`), found
/// without the store: for code that runs inside window-manager calls and
/// so must not read window-manager memory. `None` too while the seat tears
/// down: `Seat::destroy` takes its hold before `wlr_seat_destroy`.
///
/// # Safety
/// No reference to a seat is live: this reads each seat's hold.
pub unsafe fn place_holding(wlr_seat: *mut crate::ffi::wlr_seat) -> Option<*mut Seat> {
    if wlr_seat.is_null() {
        return None;
    }
    PLACES.with(|places| {
        places
            .borrow()
            .iter()
            .map(|p| p.as_ptr())
            // SAFETY: a registered place is a live seat (`take` removes it
            // before the box goes); the caller holds no reference to one.
            .find(|&p| unsafe { (*p).wlr_seat.ptr() } == wlr_seat)
    })
}

#[derive(Default)]
pub struct Seats {
    seats: Vec<Option<NonNull<Seat>>>,
}

impl Seats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take ownership of `seat` and give it its id. Its address stays fixed
    /// until [`Self::take`].
    pub fn insert(&mut self, seat: Box<Seat>) -> SeatId {
        let id = SeatId(self.seats.len() as u32);
        let ptr = NonNull::from(Box::leak(seat));
        // SAFETY: just leaked from a Box, and nothing else holds it yet.
        unsafe { (*ptr.as_ptr()).id = id };
        self.seats.push(Some(ptr));
        PLACES.with(|places| places.borrow_mut().push(ptr));
        id
    }

    /// Give up the seat `id` names; dropping or destroying the box frees
    /// it. Its slot stays empty: ids are never reused. `Seat::destroy`
    /// tears the seat down first; a seat dropped without that leaks its
    /// wlroots objects but leaves nothing dangling (it holds registries,
    /// not linked list heads, and nothing names it by address).
    pub fn take(&mut self, id: SeatId) -> Option<Box<Seat>> {
        let p = self.seats.get_mut(id.0 as usize)?.take()?;
        PLACES.with(|places| places.borrow_mut().retain(|&q| q != p));
        // SAFETY: every pointer came from `Box::leak` in `insert`, and
        // leaves the store only here, once.
        Some(unsafe { Box::from_raw(p.as_ptr()) })
    }

    /// The id of the seat that holds `wlr_seat`, for a wlroots callback
    /// handed one; `None` for null, for a `wlr_seat` no seat holds, and
    /// for one whose seat is tearing down (`Seat::destroy` takes the hold
    /// first). The seat holds its `wlr_seat` privately and the hold moves
    /// with it, so a safe swap of two seats cannot make this name the
    /// wrong one.
    pub fn find(&self, wlr_seat: *mut crate::ffi::wlr_seat) -> Option<SeatId> {
        if wlr_seat.is_null() {
            return None;
        }
        self.seats.iter().enumerate().find_map(|(i, slot)| {
            // SAFETY: live while in the store, which this borrows.
            let seat = unsafe { slot.as_ref()?.as_ref() };
            (seat.wlr_seat.ptr() == wlr_seat).then_some(SeatId(i as u32))
        })
    }

    /// Every live seat's id, oldest first.
    pub fn ids(&self) -> Vec<SeatId> {
        (0..self.seats.len() as u32).map(SeatId).filter(|&id| self.ptr(id).is_some()).collect()
    }

    /// The seat's address, for code that still works through raw
    /// pointers.
    pub fn ptr(&self, id: SeatId) -> Option<*mut Seat> {
        self.seats.get(id.0 as usize).copied().flatten().map(|p| p.as_ptr())
    }

    pub fn get(&self, id: SeatId) -> Option<&Seat> {
        // SAFETY: live while in the store; the reference borrows `self`.
        self.seats.get(id.0 as usize).copied().flatten().map(|p| unsafe { p.as_ref() })
    }

    pub fn get_mut(&mut self, id: SeatId) -> Option<&mut Seat> {
        // SAFETY: as `get`, and `&mut self` makes the reference unique among
        // the store's.
        self.seats.get_mut(id.0 as usize).and_then(|p| p.as_mut()).map(|p| unsafe { p.as_mut() })
    }

    /// Every live seat's address, oldest first, copied out: for code not yet
    /// converted to borrows.
    pub fn ptrs(&self) -> Vec<*mut Seat> {
        self.seats.iter().flatten().map(|p| p.as_ptr()).collect()
    }

    /// Whether any seat's keyboard focus is window `id`.
    pub fn focuses(&self, id: crate::window::WindowId) -> bool {
        self.values().any(|seat| seat.focused == super::Focus::Window(id))
    }

    /// Whether any seat's pointer hovers window `id` in adjust mode
    /// (`Cursor::adjust_hover`).
    pub fn adjust_hovers(&self, id: crate::window::WindowId) -> bool {
        self.values().any(|seat| seat.cursor.adjust_hover == Some(id))
    }

    /// Whether any seat is resizing window `id` with the pointer.
    pub fn resizing(&self, id: crate::window::WindowId) -> bool {
        self.values().any(|seat| {
            seat.op.as_ref().is_some_and(|op| {
                op.window == Some(id) && matches!(op.op_type, super::PointerOpType::Resize { .. })
            })
        })
    }

    /// Every live seat, mutably, oldest first. Each seat is its own
    /// allocation, so the references are disjoint.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Seat> + '_ {
        // SAFETY: the iterator borrows the store mutably, and every slot is
        // a distinct allocation yielded once.
        self.seats.iter().flatten().map(|p| unsafe { &mut *p.as_ptr() })
    }

    /// Every live seat, oldest first.
    pub fn values(&self) -> impl Iterator<Item = &Seat> + '_ {
        // SAFETY: live while in the store, which this borrows.
        self.seats.iter().flatten().map(|p| unsafe { p.as_ref() })
    }
}
