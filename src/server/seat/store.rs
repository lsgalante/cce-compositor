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
//! seat lives: its listeners, its cursor and the `wlr_seat`'s `data` hold
//! its address. The store keeps raw pointers rather than `Box`es for the
//! reason the window store gives.

use super::{Seat, SeatId};
use std::ptr::NonNull;

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
        id
    }

    /// Give up the seat `id` names; dropping or destroying the box frees
    /// it. Its slot stays empty: ids are never reused.
    ///
    /// # Safety
    /// The seat has been torn down (`Seat::destroy`): no keyboard group or
    /// input popup is still linked into the list heads the drop frees.
    pub unsafe fn take(&mut self, id: SeatId) -> Option<Box<Seat>> {
        // SAFETY: every pointer came from `Box::leak` in `insert`, and
        // leaves the store only here, once.
        self.seats.get_mut(id.0 as usize)?.take().map(|p| unsafe { Box::from_raw(p.as_ptr()) })
    }

    /// Every live seat's id, oldest first.
    pub fn ids(&self) -> Vec<SeatId> {
        (0..self.seats.len() as u32).map(SeatId).filter(|&id| self.ptr(id).is_some()).collect()
    }

    /// The seat's address, for handing to C as user data and for code that
    /// still works through raw pointers.
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
