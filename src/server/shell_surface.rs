// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlList, wl_list_insert};
use crate::wm_node::WmNode;

pub struct ShellSurfaceRenderingRequested {
    pub x: i32,
    pub y: i32,
    pub sync_next_commit: bool,
}

pub struct ShellSurface {
    pub server: *mut Server,
    pub object: *mut ffi::wl_resource, // zcce_shell_surface_v1
    pub surface: *mut ffi::wlr_surface,
    pub tree: *mut ffi::wlr_scene_tree,
    pub surfaces: crate::scene::SaveableSurfaces,
    pub popup_tree: *mut ffi::wlr_scene_tree,
    pub node: WmNode,
    pub rendering_requested: ShellSurfaceRenderingRequested,
}

impl ShellSurface {
    pub unsafe fn create(
        client: *mut ffi::wl_client,
        version: u32,
        id: u32,
        surface: *mut ffi::wlr_surface,
        server: *mut Server,
    ) -> Result<(), &'static str> {
        log::debug!("new zcce_shell_surface_v1");

        let shell_surface_v1 = ffi::wl_resource_create(client, &ffi::zcce_shell_surface_v1_interface, version as i32, id);
        if shell_surface_v1.is_null() {
            ffi::wl_client_post_no_memory(client);
            return Err("wl_resource_create failed");
        }

        if !ffi::wlr_surface_set_role(
            surface,
            &raw const SHELL_SURFACE_ROLE,
            shell_surface_v1,
            ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_ROLE,
        ) {
            return Err("wlr_surface_set_role failed");
        }
        ffi::river_wlr_surface_set_role_object(surface, shell_surface_v1);

        let shell_surface = Box::new(ShellSurface {
            server,
            object: shell_surface_v1,
            surface,
            tree: std::ptr::null_mut(),
            surfaces: std::mem::zeroed(),
            popup_tree: std::ptr::null_mut(),
            node: std::mem::zeroed(),
            rendering_requested: ShellSurfaceRenderingRequested {
                x: 0,
                y: 0,
                sync_next_commit: false,
            },
        });
        let raw = Box::into_raw(shell_surface);

        ffi::wl_resource_set_implementation(
            shell_surface_v1,
            &SHELL_SURFACE_INTERFACE as *const _ as *const _,
            raw as *mut _,
            Some(handle_shell_surface_destroy_resource),
        );

        let hidden_tree = (*server).scene.hidden_tree;
        let tree = ffi::wlr_scene_tree_create(hidden_tree);
        if tree.is_null() {
            let _ = Box::from_raw(raw);
            return Err("Failed to create scene tree");
        }
        let popup_tree = ffi::wlr_scene_tree_create(hidden_tree);
        if popup_tree.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            let _ = Box::from_raw(raw);
            return Err("Failed to create popup tree");
        }

        let surfaces = match crate::scene::SaveableSurfaces::init(tree) {
            Ok(s) => s,
            Err(e) => {
                ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
                ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
                let _ = Box::from_raw(raw);
                return Err(e);
            }
        };

        let subsurface_tree = ffi::wlr_scene_subsurface_tree_create(surfaces.tree, surface);
        if subsurface_tree.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
            let _ = Box::from_raw(raw);
            return Err("Failed to create scene subsurface tree");
        }

        (*raw).tree = tree;
        (*raw).popup_tree = popup_tree;
        (*raw).surfaces = surfaces;
        (*raw).node.init(crate::wm_node::WmNodeTag::ShellSurface);

        let list_head = &mut (*server).wm.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        wl_list_insert((*list_head).prev, &mut (*raw).node.link as *mut ffi::wl_list as *mut WlList);

        crate::scene_node_data::SceneNodeData::attach(
            tree as *mut ffi::wlr_scene_node,
            crate::scene_node_data::SceneNodeDataVal::ShellSurface(raw),
        );
        crate::scene_node_data::SceneNodeData::attach(
            popup_tree as *mut ffi::wlr_scene_node,
            crate::scene_node_data::SceneNodeDataVal::ShellSurface(raw),
        );

        Ok(())
    }

    pub unsafe fn render_finish(&mut self) {
        if self.rendering_requested.sync_next_commit {
            self.rendering_requested.sync_next_commit = false;

            if !self.surfaces.saved {
                ffi::wl_resource_post_error(
                    self.object,
                    ffi::zcce_shell_surface_v1_error_ZCCE_SHELL_SURFACE_V1_ERROR_NO_COMMIT,
                    b"no wl_surface.commit after sync_next_commit and before update_rendering_finish\0".as_ptr() as *const _,
                );
            }
        }

        self.surfaces.drop_saved();

        ffi::wlr_scene_node_set_position(
            self.tree as *mut ffi::wlr_scene_node,
            self.rendering_requested.x,
            self.rendering_requested.y,
        );
        ffi::wlr_scene_node_set_position(
            self.popup_tree as *mut ffi::wlr_scene_node,
            self.rendering_requested.x,
            self.rendering_requested.y,
        );
    }
}

pub unsafe fn from_wlr_surface(surface: *mut ffi::wlr_surface) -> *mut ShellSurface {
    if surface.is_null() {
        return std::ptr::null_mut();
    }
    let role_ptr = ffi::river_wlr_surface_get_role(surface);
    if role_ptr != &raw const SHELL_SURFACE_ROLE {
        return std::ptr::null_mut();
    }
    let resource = ffi::river_wlr_surface_get_role_resource(surface);
    if resource.is_null() {
        return std::ptr::null_mut();
    }
    ffi::wl_resource_get_user_data(resource) as *mut ShellSurface
}

unsafe extern "C" fn client_commit(surface: *mut ffi::wlr_surface) {
    let shell_surface = from_wlr_surface(surface);
    if shell_surface.is_null() {
        return;
    }
    if (*shell_surface).rendering_requested.sync_next_commit {
        (*shell_surface).surfaces.save();
    }
}

unsafe extern "C" fn commit(surface: *mut ffi::wlr_surface) {
    if ffi::wlr_surface_has_buffer(surface) {
        ffi::wlr_surface_map(surface);
    }
}

unsafe extern "C" fn handle_shell_surface_destroy_resource(resource: *mut ffi::wl_resource) {
    let shell_surface = ffi::wl_resource_get_user_data(resource) as *mut ShellSurface;
    if !shell_surface.is_null() {
        ffi::river_wlr_surface_set_role_object((*shell_surface).surface, std::ptr::null_mut());
        (*shell_surface).object = std::ptr::null_mut();
        
        ffi::wlr_surface_unmap((*shell_surface).surface);
        (*shell_surface).node.make_inert();
        (*shell_surface).node.deinit();
        ffi::wlr_scene_node_destroy((*shell_surface).tree as *mut ffi::wlr_scene_node);
        ffi::wlr_scene_node_destroy((*shell_surface).popup_tree as *mut ffi::wlr_scene_node);

        let _ = Box::from_raw(shell_surface);
    }
}

unsafe extern "C" fn role_destroy(surface: *mut ffi::wlr_surface) {
    let shell_surface = from_wlr_surface(surface);
    if shell_surface.is_null() {
        return;
    }

    ffi::river_wlr_surface_set_role_object(surface, std::ptr::null_mut());
    if !(*shell_surface).object.is_null() {
        ffi::wl_resource_set_user_data((*shell_surface).object, std::ptr::null_mut());
        ffi::wl_resource_destroy((*shell_surface).object);
        (*shell_surface).object = std::ptr::null_mut();
    }

    ffi::wlr_surface_unmap((*shell_surface).surface);

    (*shell_surface).node.make_inert();
    (*shell_surface).node.deinit();

    ffi::wlr_scene_node_destroy((*shell_surface).tree as *mut ffi::wlr_scene_node);
    ffi::wlr_scene_node_destroy((*shell_surface).popup_tree as *mut ffi::wlr_scene_node);

    let _ = Box::from_raw(shell_surface);
}

unsafe extern "C" fn shell_surface_destroy(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn shell_surface_get_node(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
) {
    let shell_surface = ffi::wl_resource_get_user_data(resource) as *mut ShellSurface;
    if shell_surface.is_null() {
        return;
    }

    if !(*shell_surface).node.object.is_null() {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_shell_surface_v1_error_ZCCE_SHELL_SURFACE_V1_ERROR_NODE_EXISTS,
            b"shell surface already has a node object\0".as_ptr() as *const _,
        );
        return;
    }

    (*shell_surface).node.create_object(
        client,
        ffi::wl_resource_get_version(resource) as u32,
        id,
    );
}

unsafe extern "C" fn shell_surface_sync_next_commit(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let shell_surface = ffi::wl_resource_get_user_data(resource) as *mut ShellSurface;
    if shell_surface.is_null() {
        return;
    }

    let server = (*shell_surface).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }

    (*shell_surface).rendering_requested.sync_next_commit = true;
}

static SHELL_SURFACE_INTERFACE: ffi::zcce_shell_surface_v1_interface = ffi::zcce_shell_surface_v1_interface {
    destroy: Some(shell_surface_destroy),
    get_node: Some(shell_surface_get_node),
    sync_next_commit: Some(shell_surface_sync_next_commit),
};

#[no_mangle]
pub static mut SHELL_SURFACE_ROLE: ffi::wlr_surface_role = ffi::wlr_surface_role {
    name: b"zcce_shell_surface_v1\0".as_ptr() as *const _,
    no_object: false,
    client_commit: Some(client_commit),
    commit: Some(commit),
    map: None,
    unmap: None,
    destroy: Some(role_destroy),
};
