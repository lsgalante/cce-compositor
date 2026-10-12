// SPDX-License-Identifier: GPL-3.0-only

//! [`Registry`]: the objects of one kind that are alive, for an owner to
//! walk — input devices, protocol resources, keyboard groups, inhibitors.
//!
//! It replaced intrusive `wl_list`s threaded through a `link` field in each
//! object and walked with `container_of!`. An object still allocates and
//! frees itself; it [`push`](Registry::push)es its address when it is
//! created and [`remove`](Registry::remove)s it in its destroy path, before
//! it is freed — so an address in a registry always names a live object,
//! and removal by address cannot hit a later object at the same address.
//!
//! wlroots' own lists (`wlr_xdg_popup`, `wlr_session_lock_surface_v1`,
//! output-configuration heads) stay `wl_list`s: C owns them.

pub struct Registry<T>(Vec<*mut T>);

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Registry(Vec::new())
    }
}

impl<T> Registry<T> {
    pub const fn new() -> Self {
        Registry(Vec::new())
    }

    /// Register `object` (at the back: walks run oldest first, as the
    /// lists' tail inserts did).
    ///
    /// # Safety
    /// `object` is live, and stays live until it is [`remove`](Self::remove)d
    /// — which its destroy path does before freeing it. Owners walking the
    /// registry rely on this.
    pub unsafe fn push(&mut self, object: *mut T) {
        self.0.push(object);
    }

    /// Register `object` at the front (the lists' head inserts).
    ///
    /// # Safety
    /// As [`push`](Self::push).
    pub unsafe fn push_front(&mut self, object: *mut T) {
        self.0.insert(0, object);
    }

    /// Unregister `object`, if it is registered.
    pub fn remove(&mut self, object: *mut T) {
        self.0.retain(|&p| p != object);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The registered addresses, in order, copied out: a walk may destroy
    /// the object it is visiting (which unregisters it) or create others.
    pub fn ptrs(&self) -> Vec<*mut T> {
        self.0.clone()
    }
}
