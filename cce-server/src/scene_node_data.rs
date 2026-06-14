// SPDX-FileCopyrightText: © 2023 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{WlListener, wl_signal_add, wl_listener_remove};
use crate::window_manager::{Window, ShellSurface, XwaylandOverrideRedirect};
use crate::layer_shell::LayerSurface;
use crate::lock_manager::LockSurface;

#[derive(Clone, Copy)]
pub enum SceneNodeDataVal {
    Window(*mut Window),
    ShellSurface(*mut ShellSurface),
    LockSurface(*mut LockSurface),
    LayerSurface(*mut LayerSurface),
    OverrideRedirect(*mut XwaylandOverrideRedirect),
}

#[repr(C)]
pub struct SceneNodeData {
    pub node: *mut ffi::wlr_scene_node,
    pub data: SceneNodeDataVal,
    pub destroy: ffi::wl_listener,
}

impl SceneNodeData {
    pub unsafe fn attach(node: *mut ffi::wlr_scene_node, data: SceneNodeDataVal) {
        let scene_node_data = Box::new(SceneNodeData {
            node,
            data,
            destroy: std::mem::zeroed(),
        });

        let raw = Box::into_raw(scene_node_data);
        ffi::river_scene_node_set_data(node, raw as *mut std::ffi::c_void);

        let destroy_listener = &mut (*raw).destroy as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_listener).notify = Some(handle_destroy);
        wl_signal_add(ffi::river_scene_node_get_destroy_signal(node), &mut (*raw).destroy);
    }

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
    let scene_node_data_ptr = crate::container_of!(listener, SceneNodeData, destroy);
    let mut scene_node_data = Box::from_raw(scene_node_data_ptr);
    wl_listener_remove(&mut scene_node_data.destroy);
    if !scene_node_data.node.is_null() {
        ffi::river_scene_node_set_data(scene_node_data.node, std::ptr::null_mut());
    }
}
