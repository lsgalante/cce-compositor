// SPDX-License-Identifier: GPL-3.0-only

//! Owned handles to wlroots scene-graph nodes.
//!
//! A `Handle` owns one node: dropping it destroys the node. What makes that
//! sound in a tree is the other half: the handle listens to the node's
//! `destroy` signal and forgets the node when wlroots destroys it by any
//! route — its parent going, `wlr_scene_node_destroy` called on the raw
//! pointer elsewhere, the scene itself torn down. A handle whose node is
//! gone reads as null and every method on it does nothing, so a pool of
//! child handles can simply be cleared after its parent tree died with
//! them, which is the case the raw pointers needed a comment for.
//!
//! An empty handle is all zero bits (`Option<Box<_>>`'s `None`), so a struct
//! zero-initialised with `std::mem::zeroed()` holds empty handles.
//!
//! The kinds share one type, `Handle<K>`, aliased as `SceneTree`,
//! `SceneRect` and so on; `raw()` hands back the kind's own pointer for the
//! calls this module does not wrap, and `node()` the base node.

use crate::ffi;
use crate::listener::Listener;
use std::marker::PhantomData;

/// A scene node kind: the wlroots struct a handle of this kind points to.
/// Every one of them begins with its `wlr_scene_node`, which is what lets
/// a `*mut Raw` be used as a `*mut wlr_scene_node`.
pub trait Kind {
    type Raw;
}

macro_rules! kinds {
    ($($(#[$doc:meta])* $kind:ident => $raw:ident, $alias:ident;)*) => {$(
        $(#[$doc])*
        pub enum $kind {}
        impl Kind for $kind {
            type Raw = ffi::$raw;
        }
        pub type $alias = Handle<$kind>;
    )*};
}

kinds! {
    /// A `wlr_scene_tree`: a node that holds others.
    Tree => wlr_scene_tree, SceneTree;
    /// A `wlr_scene_rect`: a solid rectangle.
    Rect => wlr_scene_rect, SceneRect;
    /// A `wlr_scene_buffer`.
    Buffer => wlr_scene_buffer, SceneBuffer;
    /// A scenefx `wlr_scene_bevel`: the lit chamfer rim.
    Bevel => wlr_scene_bevel, SceneBevel;
    /// A scenefx `wlr_scene_shadow`.
    Shadow => wlr_scene_shadow, SceneShadow;
    /// A scenefx `wlr_scene_frame`.
    Frame => wlr_scene_frame, SceneFrame;
    /// A scenefx `wlr_scene_droplet`.
    Droplet => wlr_scene_droplet, SceneDroplet;
}

/// The heap half of a handle: where the node is, and the listener that
/// clears it. Boxed so the listener's address stays put while the handle
/// moves.
struct Slot {
    node: *mut ffi::wlr_scene_node,
    destroy: Listener,
}

unsafe extern "C" fn handle_node_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let slot = crate::container_of!(listener, Slot, destroy);
    (*slot).node = std::ptr::null_mut();
    // wlroots asserts a node's destroy signal has no listeners left once
    // it has been emitted.
    (*slot).destroy.disconnect();
}

/// An owned scene node of kind `K`; see the module documentation.
pub struct Handle<K: Kind> {
    slot: Option<Box<Slot>>,
    _kind: PhantomData<*mut K>,
}

impl<K: Kind> Handle<K> {
    /// An empty handle.
    pub const fn none() -> Self {
        Handle { slot: None, _kind: PhantomData }
    }

    /// Take ownership of `raw`; a null `raw` gives an empty handle.
    ///
    /// # Safety
    /// `raw` must be null or a live node of kind `K` that nothing else owns:
    /// the handle will destroy it when dropped.
    pub unsafe fn adopt(raw: *mut K::Raw) -> Self {
        if raw.is_null() {
            return Self::none();
        }
        let node = raw as *mut ffi::wlr_scene_node;
        let mut slot = Box::new(Slot { node, destroy: Listener::new() });
        slot.destroy.connect(ffi::river_scene_node_get_destroy_signal(node), handle_node_destroy);
        Handle { slot: Some(slot), _kind: PhantomData }
    }

    /// The base node, or null when the handle is empty or its node is gone.
    pub fn node(&self) -> *mut ffi::wlr_scene_node {
        self.slot.as_ref().map_or(std::ptr::null_mut(), |s| s.node)
    }

    /// The node as its own kind, or null.
    pub fn raw(&self) -> *mut K::Raw {
        self.node() as *mut K::Raw
    }

    /// Whether there is no node: never set, destroyed, or gone with its
    /// parent.
    pub fn is_null(&self) -> bool {
        self.node().is_null()
    }

    /// Destroy the node now and leave the handle empty. Same as dropping it.
    pub fn destroy(&mut self) {
        let Some(mut slot) = self.slot.take() else { return };
        if slot.node.is_null() {
            return;
        }
        let node = slot.node;
        // Unlink first, so our own destroy handler does not run on a slot
        // that is about to go.
        slot.destroy.disconnect();
        slot.node = std::ptr::null_mut();
        // SAFETY: `node` is non-null, so its destroy signal has not been
        // emitted (the handler would have nulled it): the node is live, and
        // the handle owns it.
        unsafe { ffi::wlr_scene_node_destroy(node) };
    }

    /// Give up ownership without destroying the node, returning it (null if
    /// there was none). The node is then the caller's again.
    pub fn release(&mut self) -> *mut K::Raw {
        let raw = self.raw();
        if let Some(mut slot) = self.slot.take() {
            slot.destroy.disconnect();
        }
        raw
    }

    pub fn set_enabled(&self, enabled: bool) {
        let n = self.node();
        // SAFETY (this and the methods below): a non-null `node()` is a
        // live node; see `destroy`.
        if !n.is_null() {
            unsafe { ffi::wlr_scene_node_set_enabled(n, enabled) };
        }
    }

    pub fn set_position(&self, x: i32, y: i32) {
        let n = self.node();
        if !n.is_null() {
            unsafe { ffi::wlr_scene_node_set_position(n, x, y) };
        }
    }

    pub fn raise_to_top(&self) {
        let n = self.node();
        if !n.is_null() {
            unsafe { ffi::wlr_scene_node_raise_to_top(n) };
        }
    }

    pub fn lower_to_bottom(&self) {
        let n = self.node();
        if !n.is_null() {
            unsafe { ffi::wlr_scene_node_lower_to_bottom(n) };
        }
    }

    /// Restack just above `sibling`, which must share this node's parent
    /// (wlroots asserts it). Does nothing if either is gone.
    pub fn place_above<J: Kind>(&self, sibling: &Handle<J>) {
        let (n, s) = (self.node(), sibling.node());
        if !n.is_null() && !s.is_null() && n != s {
            unsafe { ffi::wlr_scene_node_place_above(n, s) };
        }
    }

    /// Restack just below `sibling`; as `place_above`.
    pub fn place_below<J: Kind>(&self, sibling: &Handle<J>) {
        let (n, s) = (self.node(), sibling.node());
        if !n.is_null() && !s.is_null() && n != s {
            unsafe { ffi::wlr_scene_node_place_below(n, s) };
        }
    }

    /// Move under `parent`. Does nothing if either is gone.
    pub fn reparent(&self, parent: &SceneTree) {
        let (n, p) = (self.node(), parent.raw());
        if !n.is_null() && !p.is_null() {
            unsafe { ffi::wlr_scene_node_reparent(n, p) };
        }
    }

    /// The node's position in layout coordinates and whether it, and every
    /// ancestor, is enabled; `None` if it is gone.
    pub fn coords(&self) -> Option<(i32, i32, bool)> {
        let n = self.node();
        if n.is_null() {
            return None;
        }
        let (mut x, mut y) = (0, 0);
        let enabled = unsafe { ffi::wlr_scene_node_coords(n, &mut x, &mut y) };
        Some((x, y, enabled))
    }
}

impl<K: Kind> Default for Handle<K> {
    fn default() -> Self {
        Self::none()
    }
}

impl<K: Kind> Drop for Handle<K> {
    fn drop(&mut self) {
        self.destroy();
    }
}

impl SceneTree {
    /// A new tree under the raw `parent`; empty if `parent` is null or the
    /// allocation fails.
    ///
    /// # Safety
    /// `parent` must be null or a live tree.
    pub unsafe fn create_in(parent: *mut ffi::wlr_scene_tree) -> Self {
        if parent.is_null() {
            return Self::none();
        }
        Self::adopt(ffi::wlr_scene_tree_create(parent))
    }

    /// A new tree under `parent`; empty if `parent` is gone.
    pub fn create(parent: &SceneTree) -> Self {
        // SAFETY: a non-null `raw()` is a live tree.
        unsafe { Self::create_in(parent.raw()) }
    }
}

impl SceneRect {
    /// A new `width` x `height` rectangle of `color` (premultiplied RGBA)
    /// under the raw `parent`; empty if `parent` is null.
    ///
    /// # Safety
    /// `parent` must be null or a live tree.
    pub unsafe fn create_in(parent: *mut ffi::wlr_scene_tree, width: i32, height: i32, color: &[f32; 4]) -> Self {
        if parent.is_null() {
            return Self::none();
        }
        Self::adopt(ffi::wlr_scene_rect_create(parent, width, height, color.as_ptr()))
    }

    /// A new rectangle under `parent`; empty if `parent` is gone.
    pub fn create(parent: &SceneTree, width: i32, height: i32, color: &[f32; 4]) -> Self {
        unsafe { Self::create_in(parent.raw(), width, height, color) }
    }

    pub fn set_size(&self, width: i32, height: i32) {
        let r = self.raw();
        if !r.is_null() {
            unsafe { ffi::wlr_scene_rect_set_size(r, width, height) };
        }
    }

    pub fn set_color(&self, color: &[f32; 4]) {
        let r = self.raw();
        if !r.is_null() {
            unsafe { ffi::wlr_scene_rect_set_color(r, color.as_ptr()) };
        }
    }
}

impl SceneBevel {
    /// A new bevel rim under the raw `parent`; empty if `parent` is null.
    ///
    /// # Safety
    /// `parent` must be null or a live tree.
    pub unsafe fn create_in(
        parent: *mut ffi::wlr_scene_tree,
        width: i32,
        height: i32,
        corner_radius: i32,
        thickness: f32,
        color: &[f32; 4],
    ) -> Self {
        if parent.is_null() {
            return Self::none();
        }
        Self::adopt(ffi::wlr_scene_bevel_create(parent, width, height, corner_radius, thickness, color.as_ptr()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scene to hang nodes on, destroyed at the end of the test.
    struct Scene(*mut ffi::wlr_scene);
    impl Scene {
        fn new() -> Self {
            let s = unsafe { ffi::wlr_scene_create() };
            assert!(!s.is_null());
            Scene(s)
        }
        fn root(&self) -> *mut ffi::wlr_scene_tree {
            // `tree` is the scene's first field.
            self.0 as *mut ffi::wlr_scene_tree
        }
    }
    impl Drop for Scene {
        fn drop(&mut self) {
            unsafe { ffi::wlr_scene_node_destroy(self.0 as *mut ffi::wlr_scene_node) };
        }
    }

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

    #[test]
    fn a_child_dies_with_its_parent_and_then_drops_as_nothing() {
        let scene = Scene::new();
        let mut parent = unsafe { SceneTree::create_in(scene.root()) };
        let child = SceneRect::create(&parent, 10, 10, &RED);
        let grandchild_tree = SceneTree::create(&parent);
        let grandchild = SceneRect::create(&grandchild_tree, 4, 4, &RED);
        assert!(!child.is_null() && !grandchild.is_null());
        parent.destroy();
        assert!(parent.is_null());
        assert!(child.is_null(), "the child went with its parent");
        assert!(grandchild.is_null(), "and so did the grandchild");
        // Methods on a gone node do nothing; dropping it destroys nothing.
        child.set_position(3, 3);
        child.set_size(1, 1);
        assert_eq!(child.coords(), None);
        drop(child);
        drop(grandchild);
    }

    #[test]
    fn dropping_a_handle_destroys_its_node_and_its_subtree() {
        let scene = Scene::new();
        let tree = unsafe { SceneTree::create_in(scene.root()) };
        let rect = SceneRect::create(&tree, 10, 10, &RED);
        let node = rect.node();
        drop(tree);
        assert!(rect.is_null());
        assert!(!node.is_null());
    }

    #[test]
    fn a_raw_destroy_elsewhere_is_seen() {
        let scene = Scene::new();
        let rect = unsafe { SceneRect::create_in(scene.root(), 5, 5, &RED) };
        unsafe { ffi::wlr_scene_node_destroy(rect.node()) };
        assert!(rect.is_null());
    }

    #[test]
    fn released_nodes_are_the_callers_again() {
        let scene = Scene::new();
        let mut rect = unsafe { SceneRect::create_in(scene.root(), 5, 5, &RED) };
        let raw = rect.release();
        assert!(rect.is_null() && !raw.is_null());
        drop(rect);
        // Still alive: positioning it is fine, and the scene frees it.
        unsafe { ffi::wlr_scene_node_set_position(raw as *mut ffi::wlr_scene_node, 1, 2) };
    }

    #[test]
    fn handles_move_and_position_and_restack() {
        let scene = Scene::new();
        let tree = unsafe { SceneTree::create_in(scene.root()) };
        let a = SceneRect::create(&tree, 5, 5, &RED);
        let b = SceneRect::create(&tree, 5, 5, &RED);
        let moved = vec![a];
        moved[0].set_position(7, 9);
        assert_eq!(moved[0].coords(), Some((7, 9, true)));
        moved[0].place_above(&b);
        b.raise_to_top();
        tree.set_enabled(false);
        assert_eq!(b.coords().map(|c| c.2), Some(false));
    }

    #[test]
    fn a_zeroed_handle_is_empty() {
        let h: SceneTree = unsafe { std::mem::zeroed() };
        assert!(h.is_null());
        drop(h);
    }
}
