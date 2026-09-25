// SPDX-FileCopyrightText: © 2023 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{WlListener, wl_listener_remove, wl_signal_add};

pub struct XdgPopup {
    pub wlr_popup: *mut ffi::wlr_xdg_popup,
    pub tree: *mut ffi::wlr_scene_tree,
    pub capture_tree: *mut ffi::wlr_scene_tree,

    pub destroy: ffi::wl_listener,
    pub commit: ffi::wl_listener,
    pub new_popup: ffi::wl_listener,
    pub reposition: ffi::wl_listener,
}

impl XdgPopup {
    pub unsafe fn create(
        wlr_popup: *mut ffi::wlr_xdg_popup,
        parent: *mut ffi::wlr_scene_tree,
        capture_parent: *mut ffi::wlr_scene_tree,
    ) -> Result<*mut Self, &'static str> {
        let base_surface = ffi::river_wlr_xdg_popup_get_base(wlr_popup);
        let tree = ffi::wlr_scene_xdg_surface_create(parent, base_surface);
        if tree.is_null() {
            return Err("wlr_scene_xdg_surface_create failed");
        }

        let mut capture_tree = std::ptr::null_mut();
        if !capture_parent.is_null() {
            capture_tree = ffi::wlr_scene_xdg_surface_create(capture_parent, base_surface);
            if capture_tree.is_null() {
                ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
                return Err("wlr_scene_xdg_surface_create for capture parent failed");
            }
        }

        let popup = Box::into_raw(Box::new(XdgPopup {
            wlr_popup,
            tree,
            capture_tree,
            destroy: std::mem::zeroed(),
            commit: std::mem::zeroed(),
            new_popup: std::mem::zeroed(),
            reposition: std::mem::zeroed(),
        }));

        let destroy_ptr = &mut (*popup).destroy as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_ptr).notify = Some(handle_destroy);
        wl_signal_add(ffi::river_wlr_xdg_popup_get_destroy_signal(wlr_popup), &mut (*popup).destroy);

        let commit_ptr = &mut (*popup).commit as *mut ffi::wl_listener as *mut WlListener;
        (*commit_ptr).notify = Some(handle_commit);
        let wlr_surface = ffi::river_wlr_xdg_surface_get_surface(base_surface);
        wl_signal_add(ffi::river_wlr_surface_get_commit_signal(wlr_surface), &mut (*popup).commit);

        let new_popup_ptr = &mut (*popup).new_popup as *mut ffi::wl_listener as *mut WlListener;
        (*new_popup_ptr).notify = Some(handle_new_popup);
        wl_signal_add(ffi::river_wlr_xdg_surface_get_new_popup_signal(base_surface), &mut (*popup).new_popup);

        let reposition_ptr = &mut (*popup).reposition as *mut ffi::wl_listener as *mut WlListener;
        (*reposition_ptr).notify = Some(handle_reposition);
        wl_signal_add(ffi::river_wlr_xdg_popup_get_reposition_signal(wlr_popup), &mut (*popup).reposition);

        Ok(popup)
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let popup = crate::container_of!(listener, XdgPopup, destroy);

    wl_listener_remove(&mut (*popup).destroy);
    wl_listener_remove(&mut (*popup).commit);
    wl_listener_remove(&mut (*popup).new_popup);
    wl_listener_remove(&mut (*popup).reposition);

    let _ = Box::from_raw(popup);
}

unsafe extern "C" fn handle_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let popup = crate::container_of!(listener, XdgPopup, commit);
    let base_surface = ffi::river_wlr_xdg_popup_get_base((*popup).wlr_popup);
    if ffi::river_wlr_xdg_surface_get_initial_commit(base_surface) {
        handle_reposition(&mut (*popup).reposition, std::ptr::null_mut());
        return;
    }
    update_blur(popup, base_surface);
}

/// Blur behind the popup as behind a window: a translucent menu is frosted
/// glass, and with no blur node it is a clear pane over whatever it opened
/// above. cce-ui paints its context-menu popup as the surface's ROOT plate
/// for exactly this — translucent, with the frost left to the compositor,
/// since the client has no backdrop to frost from inside its own popup.
///
/// Masked by the surface's own alpha (`ignore_transparent`), so the menu's
/// rounded corners and its shadow margin stay clear; never the optimized
/// (cached) blur, which re-bakes on every change beneath a surface stacked
/// above windows — see `handle_layer_surface_commit`. Sized from the
/// surface, so it runs every commit: a popup is resized by its configure.
unsafe fn update_blur(popup: *mut XdgPopup, base_surface: *mut ffi::wlr_xdg_surface) {
    let server = popup_server(popup);
    if server.is_null() {
        return;
    }
    let wlr_surface = ffi::river_wlr_xdg_surface_get_surface(base_surface);
    if wlr_surface.is_null() {
        return;
    }
    ffi::river_scene_node_enable_blur(
        (*popup).tree as *mut ffi::wlr_scene_node,
        (*server).wm.layout.window_blur,
        false,
        (*server).wm.layout.window_backdrop_blur_ignore_transparent,
        0,
        0,
        ffi::river_wlr_surface_get_width(wlr_surface),
        ffi::river_wlr_surface_get_height(wlr_surface),
        0,
    );
}

/// The server a popup belongs to, found through its parent's scene node —
/// a window or a shell surface. Null when the parent is neither.
unsafe fn popup_server(popup: *mut XdgPopup) -> *mut crate::server::Server {
    let parent_tree = ffi::river_wlr_scene_tree_get_parent((*popup).tree);
    if parent_tree.is_null() {
        return std::ptr::null_mut();
    }
    match crate::scene_node_data::SceneNodeData::from_node(parent_tree as *mut ffi::wlr_scene_node) {
        Some(node_data) => match node_data.data {
            crate::scene_node_data::SceneNodeDataVal::Window(w) => (*w).server,
            crate::scene_node_data::SceneNodeDataVal::ShellSurface(s) => (*s).server,
            _ => std::ptr::null_mut(),
        },
        None => std::ptr::null_mut(),
    }
}

unsafe extern "C" fn handle_new_popup(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let popup = crate::container_of!(listener, XdgPopup, new_popup);
    let wlr_xdg_popup = data as *mut ffi::wlr_xdg_popup;

    if let Err(e) = XdgPopup::create(wlr_xdg_popup, (*popup).tree, (*popup).capture_tree) {
        log::error!("Failed to create nested popup: {}", e);
        ffi::wl_resource_post_no_memory((*wlr_xdg_popup).resource);
    }
}

unsafe extern "C" fn handle_reposition(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let popup = crate::container_of!(listener, XdgPopup, reposition);

    let mut parent_lx: i32 = 0;
    let mut parent_ly: i32 = 0;
    let parent_tree = ffi::river_wlr_scene_tree_get_parent((*popup).tree);
    if parent_tree.is_null() {
        return;
    }

    ffi::wlr_scene_node_coords(parent_tree as *mut ffi::wlr_scene_node, &mut parent_lx, &mut parent_ly);

    let mut anchor = std::mem::zeroed();
    ffi::river_wlr_xdg_popup_get_anchor_rect((*popup).wlr_popup, &mut anchor);
    anchor.x += parent_lx;
    anchor.y += parent_ly;

    let server = popup_server(popup);
    if server.is_null() {
        return;
    }

    let wlr_output = (*server).om.max_overlap_output(&anchor);
    if wlr_output.is_null() {
        return;
    }

    let mut constraint = std::mem::zeroed();
    ffi::wlr_output_layout_get_box((*server).om.output_layout, wlr_output, &mut constraint);
    constraint.x -= parent_lx;
    constraint.y -= parent_ly;

    ffi::wlr_xdg_popup_unconstrain_from_box((*popup).wlr_popup, &mut constraint);
    ffi::wlr_xdg_surface_schedule_configure(ffi::river_wlr_xdg_popup_get_base((*popup).wlr_popup));
}
