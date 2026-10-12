// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::cursor::Cursor;

#[repr(C)]
pub struct DragIcon {
    /// The wlroots icon; the destroy handler takes it before the box goes.
    wlr_drag_icon: crate::wlr::Wlr<ffi::wlr_drag_icon>,
    /// Watched: wlroots destroys it with the drag icon.
    pub scene_drag_icon: crate::scene_handle::SceneTree,
    destroy: crate::listener::Listener,
}

// SAFETY: made by `Box::into_raw` and freed only by its own destroy path,
// after its listeners are gone, and reached only through raw pointers: no safe
// API returns a `&mut`, a `Box` or a value of it, so nothing safe can move it.
unsafe impl crate::listener::Anchored for DragIcon {}

impl DragIcon {
    pub unsafe fn create(
        wlr_drag_icon: *mut ffi::wlr_drag_icon,
        cursor: &mut Cursor,
    ) -> Result<(), &'static str> {
        let scene_drag_icon = ffi::wlr_scene_drag_icon_create(crate::shared::scene().drag_icons.raw(), wlr_drag_icon);
        if scene_drag_icon.is_null() {
            return Err("Failed to create scene drag icon");
        }

        let raw = Box::into_raw(Box::new(Self {
            // SAFETY: the destroy handler takes it before the box goes.
            wlr_drag_icon: crate::wlr::Wlr::from_raw(wlr_drag_icon),
            scene_drag_icon: crate::scene_handle::SceneTree::watch(scene_drag_icon),
            destroy: std::mem::zeroed(),
        }));

        (*raw).update_position(cursor);
        (*raw).destroy.connect(&mut (*wlr_drag_icon).events.destroy as *mut ffi::wl_signal, handle_destroy);
        // Registered until the destroy handler unregisters it, before the
        // box is freed.
        cursor.register_drag_icon(raw);

        Ok(())
    }

    /// Follow what drives the drag: the cursor, or the touch point.
    pub fn update_position(&mut self, cursor: &Cursor) {
        match self.wlr_drag_icon.grab_type() {
            ffi::wlr_drag_grab_type_WLR_DRAG_GRAB_KEYBOARD_POINTER => {
                self.scene_drag_icon.set_position(cursor.x() as i32, cursor.y() as i32);
            }
            ffi::wlr_drag_grab_type_WLR_DRAG_GRAB_KEYBOARD_TOUCH => {
                let touch_id = self.wlr_drag_icon.touch_id();
                if let Some(&crate::cursor::TouchPoint { lx, ly, .. }) = cursor.touch_points.get(&touch_id) {
                    self.scene_drag_icon.set_position(lx as i32, ly as i32);
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
    let drag_icon_ptr = crate::listener_owner!(listener, DragIcon, destroy);
    // Out of every seat's registry, not just the one it was made on: the
    // removal is idempotent, and the registry it sits in must not depend on
    // a seat id anything else could have changed.
    let server = crate::shared::server_ptr();
    if !server.is_null() {
        for seat in (*crate::reentry::wm(server)).seats.values_mut() {
            seat.cursor.unregister_drag_icon(drag_icon_ptr);
        }
    }
    let mut drag_icon = Box::from_raw(drag_icon_ptr);
    drag_icon.destroy.disconnect();
    drag_icon.wlr_drag_icon.take();
}
