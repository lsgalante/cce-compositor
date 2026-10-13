// SPDX-FileCopyrightText: © 2023 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::window_manager::XwaylandOverrideRedirect;
use crate::layer_shell::LayerSurface;
use crate::lock_manager::LockSurface;

#[derive(Clone, Copy)]
pub enum SceneNodeDataVal {
    /// A window's trees: looked up by id, so a hit test never hands out
    /// the window's address.
    Window(crate::window::WindowId),
    LockSurface(*mut LockSurface),
    LayerSurface(*mut LayerSurface),
    OverrideRedirect(*mut XwaylandOverrideRedirect),
}

#[repr(C)]
pub struct SceneNodeData {
    pub node: *mut ffi::wlr_scene_node,
    pub data: SceneNodeDataVal,
    destroy: crate::listener::Listener,
}

// SAFETY: made by `Box::into_raw` and freed only by its own destroy path,
// after its listeners are gone, and reached only through raw pointers: no safe
// API returns a `&mut`, a `Box` or a value of it, so nothing safe can move it.
unsafe impl crate::listener::Anchored for SceneNodeData {}

impl SceneNodeData {
    /// # Safety
    /// `node` must be a live scene node. A pointer in `data` must stay valid
    /// until the node is destroyed: callers of [`from_node`](Self::from_node)
    /// dereference it.
    pub unsafe fn attach(node: *mut ffi::wlr_scene_node, data: SceneNodeDataVal) {
        let scene_node_data = Box::new(SceneNodeData {
            node,
            data,
            destroy: std::mem::zeroed(),
        });

        let raw = Box::into_raw(scene_node_data);
        ffi::river_scene_node_set_data(node, raw as *mut std::ffi::c_void);

        (*raw).destroy.connect(ffi::river_scene_node_get_destroy_signal(node), handle_destroy);
    }

    /// # Safety
    /// `node` is null or a live scene node. The `'static` is nominal: the
    /// data is freed with the node that carries it, so do not keep the
    /// reference across anything that may destroy scene nodes.
    pub unsafe fn from_node(node: *mut ffi::wlr_scene_node) -> Option<&'static SceneNodeData> {
        let mut n = node;
        while !n.is_null() {
            let data_ptr = ffi::river_scene_node_get_data(n) as *mut SceneNodeData;
            if !data_ptr.is_null() {
                return Some(&*data_ptr);
            }
            let parent_tree = ffi::river_scene_node_get_parent(n);
            if !parent_tree.is_null() {
                n = parent_tree as *mut ffi::wlr_scene_node;
            } else {
                break;
            }
        }
        None
    }

    /// # Safety
    /// `surface` is null or a live `wlr_surface`, whose root's user data is
    /// null or its live scene node (as every role sets it); the reference is
    /// as [`from_node`](Self::from_node)'s.
    pub unsafe fn from_surface(surface: *mut ffi::wlr_surface) -> Option<&'static SceneNodeData> {
        if surface.is_null() {
            return None;
        }
        let root_surface = ffi::wlr_surface_get_root_surface(surface);
        if root_surface.is_null() {
            return None;
        }
        let node_ptr = ffi::river_wlr_surface_get_data(root_surface) as *mut ffi::wlr_scene_node;
        if !node_ptr.is_null() {
            Self::from_node(node_ptr)
        } else {
            None
        }
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let scene_node_data_ptr = crate::listener_owner!(listener, SceneNodeData, destroy);
    let mut scene_node_data = Box::from_raw(scene_node_data_ptr);
    scene_node_data.destroy.disconnect();
    if !scene_node_data.node.is_null() {
        ffi::river_scene_node_set_data(scene_node_data.node, std::ptr::null_mut());
    }
}
