// SPDX-License-Identifier: GPL-3.0-only

//! [`Outputs`]: the one owner of every [`Output`].
//!
//! An output is created into the store ([`Outputs::insert`]) and freed by
//! taking it out ([`Outputs::remove`]); nothing else allocates or frees one.
//! It replaced the intrusive `om.outputs` list, whose entries freed
//! themselves (`Box::from_raw(self)`) from inside their own manage pass.
//!
//! Each output sits in its own heap allocation, which never moves while the
//! output lives: its listeners and its `wlr_output`'s `data` hold its
//! address. The store keeps those allocations as raw pointers, not `Box`es,
//! for the same reason the window store does: a `Box` asserts unique access
//! that the addresses wlroots hands back would violate.
//!
//! Iteration is in CREATION order, as the list's was: several walks take
//! the first enabled output, and a slot map alone would put a hot-plugged
//! output into a freed earlier slot.

use super::{Output, OutputId};
use std::ptr::NonNull;

#[derive(Default)]
pub struct Outputs {
    map: crate::slotmap::SlotMap<NonNull<Output>>,
    /// Live keys, oldest first.
    order: Vec<crate::slotmap::Key>,
}

impl Outputs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take ownership of `output` and give it its id. Its address stays
    /// fixed until [`Self::remove`].
    pub fn insert(&mut self, output: Box<Output>) -> OutputId {
        let ptr = NonNull::from(Box::leak(output));
        let key = self.map.put(ptr);
        self.order.push(key);
        // SAFETY: just leaked from a Box, and nothing else holds it yet.
        unsafe { (*ptr.as_ptr()).id = OutputId(key) };
        OutputId(key)
    }

    /// Give up the output `id` names, if it lives. Its id goes stale: every
    /// later lookup of it is `None`. Dropping the box frees the output.
    pub fn remove(&mut self, id: OutputId) -> Option<Box<Output>> {
        self.order.retain(|&k| k != id.0);
        // SAFETY: every pointer in the map came from `Box::leak` in
        // `insert`, and leaves the map only here, once.
        self.map.remove(id.0).map(|p| unsafe { Box::from_raw(p.as_ptr()) })
    }

    /// The id of the output that holds `wlr_output`, for a wlroots
    /// callback handed one; `None` for null, for a `wlr_output` no output
    /// holds, and for one whose output is being destroyed (its destroy
    /// handler takes the hold). The output holds its `wlr_output`
    /// privately and the hold moves with it, so a safe swap of two
    /// outputs cannot make this name the wrong one.
    pub fn find(&self, wlr_output: *mut crate::ffi::wlr_output) -> Option<OutputId> {
        if wlr_output.is_null() {
            return None;
        }
        self.order.iter().copied().find(|&k| {
            // SAFETY: live while in the map, which this borrows.
            self.map.get(k).is_some_and(|p| unsafe { p.as_ref() }.wlr_output.ptr() == wlr_output)
        }).map(OutputId)
    }

    /// The output's address, for code that still works through raw
    /// pointers.
    pub fn ptr(&self, id: OutputId) -> Option<*mut Output> {
        self.map.get(id.0).map(|p| p.as_ptr())
    }

    pub fn get(&self, id: OutputId) -> Option<&Output> {
        // SAFETY: live while in the map; the reference borrows `self`.
        self.map.get(id.0).map(|p| unsafe { p.as_ref() })
    }

    pub fn get_mut(&mut self, id: OutputId) -> Option<&mut Output> {
        // SAFETY: as `get`, and `&mut self` makes the reference unique among
        // the store's.
        self.map.get_mut(id.0).map(|p| unsafe { p.as_mut() })
    }

    pub fn contains(&self, id: OutputId) -> bool {
        self.map.get(id.0).is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Every output's id, oldest first, copied out: for a loop whose body
    /// needs the store (or what holds it) mutably.
    pub fn ids(&self) -> Vec<OutputId> {
        self.order.iter().map(|&k| OutputId(k)).collect()
    }

    /// Every output's address, oldest first, copied out: for code not yet
    /// converted to borrows. The copy does not follow the store, so a loop
    /// that can destroy an output walks [`Self::ids`] instead.
    pub fn ptrs(&self) -> Vec<*mut Output> {
        self.order.iter().filter_map(|&k| self.map.get(k)).map(|p| p.as_ptr()).collect()
    }

    /// The oldest output whose applied state is Enabled.
    pub fn first_enabled(&self) -> Option<&Output> {
        self.values().find(|o| o.sent.state == super::OutputStateValue::Enabled)
    }

    /// Layout box of the first enabled output — `(x, y, width, height)`,
    /// the viewport placement is measured against; 1920x1080 at the origin
    /// before any output is up.
    pub fn first_enabled_box(&self) -> (f64, f64, f64, f64) {
        match self.values().find(|o| o.sent.state == super::OutputStateValue::Enabled) {
            Some(o) => {
                let b = o.sent.box_layout();
                (b.x as f64, b.y as f64, b.width as f64, b.height as f64)
            }
            None => (0.0, 0.0, 1920.0, 1080.0),
        }
    }

    /// Every output, mutably, oldest first. Each output is its own
    /// allocation, so the references are disjoint.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Output> + '_ {
        // SAFETY: the iterator borrows the store mutably, and every key in
        // `order` names a distinct allocation, yielded once.
        let map = &self.map;
        self.order.iter().filter_map(move |&k| map.get(k)).map(|p| unsafe { &mut *p.as_ptr() })
    }

    /// Every output, oldest first.
    pub fn values(&self) -> impl Iterator<Item = &Output> + '_ {
        // SAFETY: live while in the map, which this borrows.
        self.order.iter().filter_map(|&k| self.map.get(k)).map(|p| unsafe { p.as_ref() })
    }
}
