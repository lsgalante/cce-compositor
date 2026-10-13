// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::scene_node_data::{SceneNodeData, SceneNodeDataVal};

pub struct InputPopup {
    /// The seat whose relay the popup belongs to. An id, not a relay
    /// pointer: the relay lives inside its seat, which may move.
    pub sid: crate::seat::SeatId,
    /// Live until the popup's destroy handler frees this. Private, because
    /// the handlers and `update` dereference it.
    wlr_popup: *mut ffi::wlr_input_popup_surface_v2,
    /// Watched: wlroots destroys it with the popup surface.
    pub surface_tree: crate::scene_handle::SceneTree,

    destroy: crate::listener::Listener,
    map: crate::listener::Listener,
    unmap: crate::listener::Listener,
    commit: crate::listener::Listener,
}

// SAFETY: made by `Box::into_raw` and freed only by its own destroy path,
// after its listeners are gone, and reached only through raw pointers: no safe
// API returns a `&mut`, a `Box` or a value of it, so nothing safe can move it.
unsafe impl crate::listener::Anchored for InputPopup {}

impl InputPopup {
    /// # Safety
    /// `wlr_popup` must be a live input-popup surface no object of ours
    /// holds, as an input method's `new_popup_surface` hands one.
    pub unsafe fn create(
        wm: &mut crate::window_manager::WindowManager,
        wlr_popup: *mut ffi::wlr_input_popup_surface_v2,
        sid: crate::seat::SeatId,
    ) -> Result<(), &'static str> {
        if wm.seats.get(sid).is_none() {
            return Err("input popup for a seat that is gone");
        }
        let hidden_tree = crate::shared::scene().hidden_tree.raw();

        let surface_tree = ffi::wlr_scene_subsurface_tree_create(hidden_tree, (*wlr_popup).surface);
        if surface_tree.is_null() {
            return Err("Failed to create subsurface tree for input popup");
        }

        let input_popup = Box::new(InputPopup {
            sid,
            wlr_popup,
            surface_tree: crate::scene_handle::SceneTree::watch(surface_tree),
            destroy: std::mem::zeroed(),
            map: std::mem::zeroed(),
            unmap: std::mem::zeroed(),
            commit: std::mem::zeroed(),
        });

        let raw = Box::into_raw(input_popup);

        // Append to the relay's popups.
        if let Some(seat) = wm.seats.get_mut(sid) {
            seat.relay.register_popup(raw);
        }

        (*raw).destroy.connect(&mut (*wlr_popup).events.destroy, handle_destroy);
        (*raw).map.connect(ffi::river_wlr_surface_get_map_signal((*wlr_popup).surface), handle_map);
        (*raw).unmap.connect(ffi::river_wlr_surface_get_unmap_signal((*wlr_popup).surface), handle_unmap);
        (*raw).commit.connect(ffi::river_wlr_surface_get_commit_signal((*wlr_popup).surface), handle_commit);

        (*raw).update(wm);

        Ok(())
    }

    /// # Safety
    /// `self.wlr_popup` must be live, as it is for a popup
    /// [`create`](Self::create) made until its destroy handler frees it.
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

    // Out of whichever relay holds it — not only its seat's: safe code can
    // swap two seats' relays. Found without the window manager: popups go
    // inside `wlr_seat_destroy` (with their input method), which runs in
    // the manage pass.
    for seat in crate::seat::live_places() {
        (*seat).relay.unregister_popup(input_popup);
    }

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
