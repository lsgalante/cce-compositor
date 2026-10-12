// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{WlList};
use crate::scene_node_data::{SceneNodeData, SceneNodeDataVal};

#[repr(C)]
pub struct InputPopup {
    pub link: ffi::wl_list,
    /// The seat whose relay the popup belongs to. An id, not a relay
    /// pointer: the relay lives inside its seat, which may move.
    pub sid: crate::seat::SeatId,
    pub wlr_popup: *mut ffi::wlr_input_popup_surface_v2,
    /// Watched: wlroots destroys it with the popup surface.
    pub surface_tree: crate::scene_handle::SceneTree,

    pub destroy: crate::listener::Listener,
    pub map: crate::listener::Listener,
    pub unmap: crate::listener::Listener,
    pub commit: crate::listener::Listener,
}

// SAFETY: made by `Box::into_raw` and freed only by its own destroy path,
// after its listeners are gone, and reached only through raw pointers: no safe
// API returns a `&mut`, a `Box` or a value of it, so nothing safe can move it.
unsafe impl crate::listener::Anchored for InputPopup {}

impl InputPopup {
    pub unsafe fn create(
        wm: &mut crate::window_manager::WindowManager,
        wlr_popup: *mut ffi::wlr_input_popup_surface_v2,
        sid: crate::seat::SeatId,
    ) -> Result<(), &'static str> {
        let Some(seat) = wm.seats.get_mut(sid) else {
            return Err("input popup for a seat that is gone");
        };
        let popups_list = seat.relay.input_popups_head() as *mut WlList;
        let hidden_tree = crate::shared::scene().hidden_tree.raw();

        let surface_tree = ffi::wlr_scene_subsurface_tree_create(hidden_tree, (*wlr_popup).surface);
        if surface_tree.is_null() {
            return Err("Failed to create subsurface tree for input popup");
        }

        let mut input_popup = Box::new(InputPopup {
            link: std::mem::zeroed(),
            sid,
            wlr_popup,
            surface_tree: crate::scene_handle::SceneTree::watch(surface_tree),
            destroy: std::mem::zeroed(),
            map: std::mem::zeroed(),
            unmap: std::mem::zeroed(),
            commit: std::mem::zeroed(),
        });

        ffi::wl_list_init(&mut input_popup.link);

        let raw = Box::into_raw(input_popup);

        // Append to the relay's popups.
        crate::server::wl_list_insert((*popups_list).prev, &mut (*raw).link as *mut ffi::wl_list as *mut WlList);

        (*raw).destroy.connect(&mut (*wlr_popup).events.destroy, handle_destroy);
        (*raw).map.connect(ffi::river_wlr_surface_get_map_signal((*wlr_popup).surface), handle_map);
        (*raw).unmap.connect(ffi::river_wlr_surface_get_unmap_signal((*wlr_popup).surface), handle_unmap);
        (*raw).commit.connect(ffi::river_wlr_surface_get_commit_signal((*wlr_popup).surface), handle_commit);

        (*raw).update(wm);

        Ok(())
    }

    pub unsafe fn update(&mut self, wm: &crate::window_manager::WindowManager) {
        let text_input = wm.seats.get(self.sid).map_or(std::ptr::null_mut(), |seat| seat.relay.text_input());
        if text_input.is_null() {
            let hidden_tree = crate::shared::scene().hidden_tree.raw();
            ffi::wlr_scene_node_reparent(
                self.surface_tree.node(),
                hidden_tree,
            );
            return;
        }

        if !ffi::river_wlr_surface_is_mapped((*self.wlr_popup).surface) {
            return;
        }

        let focused_surface = (*(*text_input).wlr_text_input).focused_surface;
        if focused_surface.is_null() {
            return;
        }

        assert_eq!(ffi::wlr_surface_get_root_surface(focused_surface), focused_surface);

        let focused = match SceneNodeData::from_surface(focused_surface) {
            Some(f) => f,
            None => return,
        };


        let popup_tree = match focused.data {
            SceneNodeDataVal::Window(id) => match wm.windows.get(id) {
                Some(w) => w.popup_tree.raw(),
                None => return,
            },
            SceneNodeDataVal::LockSurface(_) => crate::shared::scene().layers.popups.raw(),
            SceneNodeDataVal::LayerSurface(layer_surface) => (*layer_surface).popup_tree.raw(),
            SceneNodeDataVal::OverrideRedirect(_) => panic!("Xwayland doesn't use text-input protocol"),
        };

        ffi::wlr_scene_node_reparent(self.surface_tree.node(), popup_tree);

        // cursor_rectangle features check: check if WLR_TEXT_INPUT_V3_FEATURE_CURSOR_RECTANGLE is active
        let active_features = (*(*text_input).wlr_text_input).active_features;
        let feature_cursor_rect = ffi::wlr_text_input_v3_features_WLR_TEXT_INPUT_V3_FEATURE_CURSOR_RECTANGLE;
        if (active_features & feature_cursor_rect) == 0 {
            self.surface_tree.set_position(0, 0);
            return;
        }

        let mut focused_x: std::os::raw::c_int = 0;
        let mut focused_y: std::os::raw::c_int = 0;
        ffi::wlr_scene_node_coords(focused.node, &mut focused_x, &mut focused_y);

        let mut cursor_box = (*(*text_input).wlr_text_input).current.cursor_rectangle;

        let wlr_output = crate::output_manager::output_at(
            (focused_x + cursor_box.x) as f64,
            (focused_y + cursor_box.y) as f64,
        );
        if wlr_output.is_null() {
            return;
        }

        let mut output_box = ffi::wlr_box {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        ffi::wlr_output_layout_get_box(crate::shared::handles().output_layout.ptr(), wlr_output, &mut output_box);

        cursor_box.x += focused_x - output_box.x;
        cursor_box.y += focused_y - output_box.y;

        let popup_width = ffi::river_wlr_surface_get_width((*self.wlr_popup).surface);
        let popup_height = ffi::river_wlr_surface_get_height((*self.wlr_popup).surface);

        let popup_x = if output_box.width - cursor_box.x >= popup_width {
            cursor_box.x
        } else {
            cursor_box.x + cursor_box.width - popup_width
        };

        let popup_y = if output_box.height - (cursor_box.y + cursor_box.height) >= popup_height {
            cursor_box.y + cursor_box.height
        } else {
            cursor_box.y - popup_height
        };

        self.surface_tree.set_position(popup_x - focused_x + output_box.x, popup_y - focused_y + output_box.y);

        cursor_box.x -= popup_x;
        cursor_box.y -= popup_y;
        ffi::wlr_input_popup_surface_v2_send_text_input_rectangle(self.wlr_popup, &mut cursor_box);
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let input_popup = crate::listener_owner!(listener, InputPopup, destroy);

    (*input_popup).destroy.disconnect();
    (*input_popup).map.disconnect();
    (*input_popup).unmap.disconnect();
    (*input_popup).commit.disconnect();

    ffi::wl_list_remove(&mut (*input_popup).link);

    let _ = Box::from_raw(input_popup);
}

unsafe extern "C" fn handle_map(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let input_popup = crate::listener_owner!(listener, InputPopup, map);
    (*input_popup).update(&*crate::reentry::wm(crate::shared::server_ptr()));
}

unsafe extern "C" fn handle_unmap(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let input_popup = crate::listener_owner!(listener, InputPopup, unmap);
    let hidden_tree = crate::shared::scene().hidden_tree.raw();

    ffi::wlr_scene_node_reparent(
        (*input_popup).surface_tree.node(),
        hidden_tree,
    );
}

unsafe extern "C" fn handle_commit(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let input_popup = crate::listener_owner!(listener, InputPopup, commit);
    (*input_popup).update(&*crate::reentry::wm(crate::shared::server_ptr()));
}
