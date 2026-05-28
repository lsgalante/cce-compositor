// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::cursor::Cursor;
use crate::server::{WlListener, wl_listener_remove, wl_signal_add};

#[repr(C)]
pub struct DragIcon {
    pub wlr_drag_icon: *mut ffi::wlr_drag_icon,
    pub scene_drag_icon: *mut ffi::wlr_scene_tree,
    pub destroy: ffi::wl_listener,
}

impl DragIcon {
    pub unsafe fn create(
        wlr_drag_icon: *mut ffi::wlr_drag_icon,
        cursor: *mut Cursor,
    ) -> Result<(), &'static str> {
        let server = (*(*cursor).seat).server;
        
        let scene_drag_icon = ffi::wlr_scene_drag_icon_create((*server).scene.drag_icons, wlr_drag_icon);
        if scene_drag_icon.is_null() {
            return Err("Failed to create scene drag icon");
        }

        let drag_icon = Box::new(Self {
            wlr_drag_icon,
            scene_drag_icon,
            destroy: std::mem::zeroed(),
        });
        let raw = Box::into_raw(drag_icon);

        ffi::river_scene_node_set_data(
            scene_drag_icon as *mut ffi::wlr_scene_node,
            raw as *mut std::ffi::c_void,
        );

        let drag_icon_ref = &mut *raw;
        drag_icon_ref.update_position(cursor);

        let destroy_listener = &mut drag_icon_ref.destroy as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_listener).notify = Some(handle_destroy);
        wl_signal_add(
            &mut (*wlr_drag_icon).events.destroy as *mut ffi::wl_signal,
            &mut drag_icon_ref.destroy,
        );

        Ok(())
    }

    pub unsafe fn update_position(&mut self, cursor: *mut Cursor) {
        let grab_type = ffi::river_wlr_drag_get_grab_type((*self.wlr_drag_icon).drag);
        match grab_type {
            ffi::wlr_drag_grab_type_WLR_DRAG_GRAB_KEYBOARD => {
                // unreachable
            }
            ffi::wlr_drag_grab_type_WLR_DRAG_GRAB_KEYBOARD_POINTER => {
                let x = (*cursor).x();
                let y = (*cursor).y();
                ffi::wlr_scene_node_set_position(
                    self.scene_drag_icon as *mut ffi::wlr_scene_node,
                    x as i32,
                    y as i32,
                );
            }
            ffi::wlr_drag_grab_type_WLR_DRAG_GRAB_KEYBOARD_TOUCH => {
                let touch_id = ffi::river_wlr_drag_get_touch_id((*self.wlr_drag_icon).drag);
                if let Some(&(lx, ly)) = (*cursor).touch_points.get(&touch_id) {
                    ffi::wlr_scene_node_set_position(
                        self.scene_drag_icon as *mut ffi::wlr_scene_node,
                        lx as i32,
                        ly as i32,
                    );
                }
            }
            _ => {}
        }
    }
}

unsafe extern "C" fn handle_destroy(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let drag_icon_ptr = crate::container_of!(listener, DragIcon, destroy);
    let mut drag_icon = Box::from_raw(drag_icon_ptr);
    wl_listener_remove(&mut drag_icon.destroy);

    // Destroy the scene node hierarchy.
    ffi::wlr_scene_node_destroy(drag_icon.scene_drag_icon as *mut ffi::wlr_scene_node);
}
