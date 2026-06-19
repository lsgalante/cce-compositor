// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::seat::Seat;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PointerBindingStateChange {
    None,
    Pressed,
    Released,
}

pub struct PointerBindingScheduled {
    pub state_changes: Vec<PointerBindingStateChange>,
}

pub struct PointerBindingRequested {
    pub enabled: bool,
}

pub struct PointerBinding {
    pub seat: *mut Seat,
    pub object: *mut ffi::wl_resource,
    pub button: u32,
    pub modifiers: u32,
    pub wm_scheduled: PointerBindingScheduled,
    pub wm_requested: PointerBindingRequested,
    pub sent_pressed: bool,
    pub link: ffi::wl_list,
}

impl PointerBinding {
    pub unsafe fn create(
        seat: *mut Seat,
        client: *mut ffi::wl_client,
        version: u32,
        id: u32,
        button: u32,
        modifiers: u32,
    ) -> Result<(), &'static str> {
        let binding_ptr = Box::into_raw(Box::new(Self {
            seat,
            object: std::ptr::null_mut(),
            button,
            modifiers,
            wm_scheduled: PointerBindingScheduled {
                state_changes: Vec::new(),
            },
            wm_requested: PointerBindingRequested {
                enabled: false,
            },
            sent_pressed: false,
            link: std::mem::zeroed(),
        }));

        let resource = ffi::wl_resource_create(client, &ffi::zcce_pointer_binding_v1_interface, version as i32, id);
        if resource.is_null() {
            let _ = Box::from_raw(binding_ptr);
            return Err("wl_resource_create failed");
        }

        (*binding_ptr).object = resource;
        ffi::wl_resource_set_implementation(
            resource,
            &POINTER_BINDING_INTERFACE as *const _ as *const _,
            binding_ptr as *mut _,
            Some(handle_binding_resource_destroy),
        );

        let pointer_bindings_list = &mut (*seat).pointer_bindings as *mut ffi::wl_list as *mut crate::server::WlList;
        crate::server::wl_list_insert((*pointer_bindings_list).prev, &mut (*binding_ptr).link as *mut ffi::wl_list as *mut crate::server::WlList);

        log::debug!(
            "new zcce_pointer_binding_v1: button: {} modifiers: {}",
            button,
            modifiers
        );

        Ok(())
    }

    pub unsafe fn destroy(binding: *mut Self) {
        ffi::wl_resource_set_implementation(
            (*binding).object,
            std::ptr::null(),
            std::ptr::null_mut(),
            None,
        );
        handle_binding_resource_destroy((*binding).object);
    }

    pub unsafe fn pressed(&mut self) {
        self.wm_scheduled.state_changes.push(PointerBindingStateChange::Pressed);
        (*(*self.seat).server).wm.dirty_windowing();
    }

    pub unsafe fn released(&mut self) {
        self.wm_scheduled.state_changes.push(PointerBindingStateChange::Released);
        (*(*self.seat).server).wm.dirty_windowing();
    }

    pub fn match_binding(&self, button: u32, modifiers: u32) -> bool {
        if !self.wm_requested.enabled {
            return false;
        }
        button == self.button && modifiers == self.modifiers
    }
}

unsafe extern "C" fn handle_binding_resource_destroy(resource: *mut ffi::wl_resource) {
    let binding = ffi::wl_resource_get_user_data(resource) as *mut PointerBinding;
    if !binding.is_null() {
        // It is possible for the window manager to create duplicate pointer bindings.
        let cursor = &mut (*(*binding).seat).cursor;
        if let Some(val) = cursor.pressed.get_mut(&(*binding).button) {
            if *val == Some(binding) {
                *val = None;
            }
        }

        crate::server::wl_list_remove(&mut (*binding).link as *mut ffi::wl_list as *mut crate::server::WlList);
        let _ = Box::from_raw(binding);
    }
}

static POINTER_BINDING_INTERFACE: ffi::zcce_pointer_binding_v1_interface = ffi::zcce_pointer_binding_v1_interface {
    destroy: Some(pointer_binding_destroy),
    enable: Some(pointer_binding_enable),
    disable: Some(pointer_binding_disable),
};

unsafe extern "C" fn pointer_binding_destroy(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn pointer_binding_enable(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let binding = ffi::wl_resource_get_user_data(resource) as *mut PointerBinding;
    if binding.is_null() {
        return;
    }
    let server = (*(*binding).seat).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*binding).wm_requested.enabled = true;
}

unsafe extern "C" fn pointer_binding_disable(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let binding = ffi::wl_resource_get_user_data(resource) as *mut PointerBinding;
    if binding.is_null() {
        return;
    }
    let server = (*(*binding).seat).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*binding).wm_requested.enabled = false;
}
