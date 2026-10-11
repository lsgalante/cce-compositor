// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{WlList, wl_list_remove};
use crate::window::Window;

/// A window's place in the render list (`wm.rendering_requested.list`):
/// the list is linked through `node.link`, and the window is recovered from
/// the node by `window`.
pub struct WmNode {
    pub link: ffi::wl_list,
}

impl WmNode {
    /// # Safety
    /// The node must not be linked into a list, and must not move afterwards
    /// (the link points at itself).
    pub unsafe fn init(&mut self) {
        self.link.prev = &mut self.link;
        self.link.next = &mut self.link;
    }

    pub unsafe fn deinit(&mut self) {
        if !self.link.prev.is_null() && !self.link.next.is_null() {
            wl_list_remove(&mut self.link as *mut ffi::wl_list as *mut WlList);
        }
    }

    /// The window this node is embedded in.
    pub unsafe fn window(&self) -> *mut Window {
        crate::container_of!(self, Window, node)
    }
}
