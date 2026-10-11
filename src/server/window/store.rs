// SPDX-License-Identifier: GPL-3.0-only

//! [`Windows`]: the one owner of every [`Window`].
//!
//! A window is created into the store ([`Windows::insert`]) and freed by
//! taking it out ([`Windows::remove`]); nothing else allocates or frees one.
//! Everything else names a window by its [`WindowId`] and looks it up here.
//!
//! Each window sits in its own heap allocation, which never moves while the
//! window lives: its listeners, its scene-node data and the back-pointers
//! of its protocol objects all hold its address. The store keeps those
//! allocations as raw pointers, not `Box`es, because a `Box` asserts unique
//! access to what it points at, and the addresses wlroots hands back (a
//! listener recovered with `container_of!`, a handle's `data`) would then
//! be pointers the box had invalidated.
//!
//! [`get`](Windows::get) and [`get_mut`](Windows::get_mut) are safe: a
//! reference into the store borrows the store, so the borrow checker keeps
//! two of them apart. Rust code reaches windows only through them (and
//! [`values`](Windows::values) / [`values_mut`](Windows::values_mut));
//! [`ptr`](Windows::ptr) is for handing an address to C.

use super::{Window, WindowId};
use std::ptr::NonNull;

#[derive(Default)]
pub struct Windows {
    map: crate::slotmap::SlotMap<NonNull<Window>>,
}

impl Windows {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take ownership of `window` and give it its id (`ref_key`). Its
    /// address stays fixed until [`Self::remove`].
    pub fn insert(&mut self, window: Box<Window>) -> WindowId {
        let ptr = NonNull::from(Box::leak(window));
        let key = self.map.put(ptr);
        // SAFETY: just leaked from a Box, and nothing else holds it yet.
        unsafe { (*ptr.as_ptr()).ref_key = key };
        WindowId(key)
    }

    /// Give up the window `id` names, if it lives. Its id goes stale: every
    /// later lookup of it is `None`. Dropping the box frees the window.
    pub fn remove(&mut self, id: WindowId) -> Option<Box<Window>> {
        // SAFETY: every pointer in the map came from `Box::leak` in
        // `insert`, and leaves the map only here, once.
        self.map.remove(id.0).map(|p| unsafe { Box::from_raw(p.as_ptr()) })
    }

    /// The window's address, for handing to C as user data (a wlroots
    /// handle's `data`); Rust code borrows with `get` / `get_mut`.
    pub fn ptr(&self, id: WindowId) -> Option<*mut Window> {
        self.map.get(id.0).map(|p| p.as_ptr())
    }

    pub fn get(&self, id: WindowId) -> Option<&Window> {
        // SAFETY: live while in the map; the reference borrows `self`.
        self.map.get(id.0).map(|p| unsafe { p.as_ref() })
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        // SAFETY: as `get`, and `&mut self` makes the reference unique among
        // the store's.
        self.map.get_mut(id.0).map(|p| unsafe { p.as_mut() })
    }

    pub fn contains(&self, id: WindowId) -> bool {
        self.map.get(id.0).is_some()
    }

    pub fn count(&self) -> u32 {
        self.map.count()
    }

    /// Every window, in slot order.
    pub fn values(&self) -> Values<'_> {
        Values(self.map.iter())
    }

    /// Every window, mutably, in slot order. Each window is its own
    /// allocation, so the references are disjoint.
    pub fn values_mut(&mut self) -> ValuesMut<'_> {
        ValuesMut(self.map.iter(), std::marker::PhantomData)
    }

    /// Every window's id, in slot order, copied out: for a loop whose body
    /// needs the store (or the window manager around it) mutably.
    pub fn ids(&self) -> Vec<WindowId> {
        self.values().map(|w| w.id()).collect()
    }
}

// Named iterator types rather than `impl Iterator`: an opaque type may have
// drop glue that uses the borrow, so a loop that returns after handing the
// window manager on mutably would not compile against one.

pub struct Values<'a>(crate::slotmap::Iter<'a, NonNull<Window>>);

impl<'a> Iterator for Values<'a> {
    type Item = &'a Window;
    fn next(&mut self) -> Option<&'a Window> {
        // SAFETY: live while in the map, which this borrows.
        self.0.next().map(|p| unsafe { p.as_ref() })
    }
}

pub struct ValuesMut<'a>(
    crate::slotmap::Iter<'a, NonNull<Window>>,
    std::marker::PhantomData<&'a mut Window>,
);

impl<'a> Iterator for ValuesMut<'a> {
    type Item = &'a mut Window;
    fn next(&mut self) -> Option<&'a mut Window> {
        // SAFETY: the iterator borrows the store mutably, and every slot is
        // a distinct allocation yielded once.
        self.0.next().map(|p| unsafe { &mut *p.as_ptr() })
    }
}
