// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlList, wl_list_insert, wl_list_remove};
use crate::window::Window;
use crate::shell_surface::ShellSurface;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmNodeTag {
    Window,
    ShellSurface,
}

pub struct WmNode {
    pub tag: WmNodeTag,
    pub object: *mut ffi::wl_resource, // river_node_v1
    pub link: ffi::wl_list,
}

pub enum WmNodeType {
    Window(*mut Window),
    ShellSurface(*mut ShellSurface),
}

impl WmNode {
    pub unsafe fn init(&mut self, tag: WmNodeTag) {
        self.tag = tag;
        self.object = std::ptr::null_mut();
        self.link.prev = &mut self.link;
        self.link.next = &mut self.link;
    }

    pub unsafe fn deinit(&mut self) {
        self.make_inert();
        if !self.link.prev.is_null() && !self.link.next.is_null() {
            wl_list_remove(&mut self.link as *mut ffi::wl_list as *mut WlList);
        }
    }

    pub unsafe fn get(&self) -> WmNodeType {
        match self.tag {
            WmNodeTag::Window => {
                let window_ptr = crate::container_of!(self, Window, node);
                WmNodeType::Window(window_ptr)
            }
            WmNodeTag::ShellSurface => {
                let shell_surface_ptr = crate::container_of!(self, ShellSurface, node);
                WmNodeType::ShellSurface(shell_surface_ptr)
            }
        }
    }

    pub unsafe fn create_object(&mut self, client: *mut ffi::wl_client, version: u32, id: u32) {
        assert!(self.object.is_null());
        let resource = ffi::wl_resource_create(client, &ffi::river_node_v1_interface, version as i32, id);
        if resource.is_null() {
            log::error!("out of memory");
            ffi::wl_client_post_no_memory(client);
            return;
        }

        ffi::wl_resource_set_implementation(
            resource,
            &NODE_INTERFACE as *const _ as *const _,
            self as *mut WmNode as *mut _,
            Some(handle_destroy),
        );
        self.object = resource;
    }

    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_NODE_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
        }
    }
}

unsafe extern "C" fn handle_destroy(resource: *mut ffi::wl_resource) {
    let node = ffi::wl_resource_get_user_data(resource) as *mut WmNode;
    if !node.is_null() {
        (*node).object = std::ptr::null_mut();
    }
}

unsafe extern "C" fn node_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn node_set_position(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
) {
    let node = ffi::wl_resource_get_user_data(resource) as *mut WmNode;
    if node.is_null() {
        return;
    }
    
    let server = match (*node).get() {
        WmNodeType::Window(w) => (*w).server,
        WmNodeType::ShellSurface(s) => (*s).server,
    };
    
    if !(*server).wm.ensure_rendering() {
        return;
    }
    
    match (*node).get() {
        WmNodeType::Window(w) => {
            if (*w).get_parent().is_null() {
                (*w).rendering_requested.x = x;
                (*w).rendering_requested.y = y;
            }
        }
        WmNodeType::ShellSurface(s) => {
            (*s).rendering_requested.x = x;
            (*s).rendering_requested.y = y;
        }
    }
}

unsafe extern "C" fn node_place_top(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let node = ffi::wl_resource_get_user_data(resource) as *mut WmNode;
    if node.is_null() {
        return;
    }
    let server = match (*node).get() {
        WmNodeType::Window(w) => (*w).server,
        WmNodeType::ShellSurface(s) => (*s).server,
    };
    if !(*server).wm.ensure_rendering() {
        return;
    }
    wl_list_remove(&mut (*node).link as *mut ffi::wl_list as *mut WlList);
    
    let list_head = &mut (*server).wm.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
    wl_list_insert((*list_head).prev, &mut (*node).link as *mut ffi::wl_list as *mut WlList);
}

unsafe extern "C" fn node_place_bottom(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let node = ffi::wl_resource_get_user_data(resource) as *mut WmNode;
    if node.is_null() {
        return;
    }
    let server = match (*node).get() {
        WmNodeType::Window(w) => (*w).server,
        WmNodeType::ShellSurface(s) => (*s).server,
    };
    if !(*server).wm.ensure_rendering() {
        return;
    }
    wl_list_remove(&mut (*node).link as *mut ffi::wl_list as *mut WlList);
    
    let list_head = &mut (*server).wm.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
    wl_list_insert(list_head, &mut (*node).link as *mut ffi::wl_list as *mut WlList);
}

unsafe extern "C" fn node_place_above(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    other: *mut ffi::wl_resource,
) {
    let node = ffi::wl_resource_get_user_data(resource) as *mut WmNode;
    if node.is_null() {
        return;
    }
    let server = match (*node).get() {
        WmNodeType::Window(w) => (*w).server,
        WmNodeType::ShellSurface(s) => (*s).server,
    };
    if !(*server).wm.ensure_rendering() {
        return;
    }
    
    let other_node = ffi::wl_resource_get_user_data(other) as *mut WmNode;
    if other_node.is_null() || other_node == node {
        return;
    }
    
    wl_list_remove(&mut (*node).link as *mut ffi::wl_list as *mut WlList);
    wl_list_insert(
        &mut (*other_node).link as *mut ffi::wl_list as *mut WlList,
        &mut (*node).link as *mut ffi::wl_list as *mut WlList,
    );
}

unsafe extern "C" fn node_place_below(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    other: *mut ffi::wl_resource,
) {
    let node = ffi::wl_resource_get_user_data(resource) as *mut WmNode;
    if node.is_null() {
        return;
    }
    let server = match (*node).get() {
        WmNodeType::Window(w) => (*w).server,
        WmNodeType::ShellSurface(s) => (*s).server,
    };
    if !(*server).wm.ensure_rendering() {
        return;
    }
    
    let other_node = ffi::wl_resource_get_user_data(other) as *mut WmNode;
    if other_node.is_null() || other_node == node {
        return;
    }
    
    wl_list_remove(&mut (*node).link as *mut ffi::wl_list as *mut WlList);
    wl_list_insert(
        (*other_node).link.prev as *mut WlList,
        &mut (*node).link as *mut ffi::wl_list as *mut WlList,
    );
}

static NODE_INTERFACE: ffi::river_node_v1_interface = ffi::river_node_v1_interface {
    destroy: Some(node_destroy),
    set_position: Some(node_set_position),
    place_top: Some(node_place_top),
    place_bottom: Some(node_place_bottom),
    place_above: Some(node_place_above),
    place_below: Some(node_place_below),
};

static INERT_NODE_INTERFACE: ffi::river_node_v1_interface = ffi::river_node_v1_interface {
    destroy: Some(node_destroy),
    set_position: None,
    place_top: None,
    place_bottom: None,
    place_above: None,
    place_below: None,
};
