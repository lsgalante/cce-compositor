// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::seat::Seat;
use crate::server::Server;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum XkbBindingStateChange {
    None,
    Pressed,
    StopRepeat,
    Released,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum XkbBindingsSeatRequestedNextKeyChange {
    None,
    EnsureEaten,
    CancelEnsureEaten,
}

#[derive(Clone, Copy, Debug)]
pub struct XkbBindingsSeatModsUpdate {
    pub old: u32,
    pub new: u32,
}

pub struct XkbBindingsSeat {
    pub object: *mut ffi::wl_resource,
    pub scheduled_ate_unbound_key: bool,
    pub scheduled_mods_update: Option<XkbBindingsSeatModsUpdate>,
    pub requested_next_key_change: XkbBindingsSeatRequestedNextKeyChange,
    pub requested_mods_watched: u32,
    pub ensure_next_key_eaten: bool,
}

impl Default for XkbBindingsSeat {
    fn default() -> Self {
        Self {
            object: std::ptr::null_mut(),
            scheduled_ate_unbound_key: false,
            scheduled_mods_update: None,
            requested_next_key_change: XkbBindingsSeatRequestedNextKeyChange::None,
            requested_mods_watched: 0,
            ensure_next_key_eaten: false,
        }
    }
}

impl XkbBindingsSeat {
    pub unsafe fn create_object(&mut self, client: *mut ffi::wl_client, version: u32, id: u32) {
        assert!(self.object.is_null());
        let resource = ffi::wl_resource_create(client, &ffi::river_xkb_bindings_seat_v1_interface, version as i32, id);
        if resource.is_null() {
            ffi::wl_client_post_no_memory(client);
            return;
        }
        self.object = resource;
        
        ffi::wl_resource_set_implementation(
            resource,
            &XKB_BINDINGS_SEAT_INTERFACE as *const _ as *const _,
            self as *mut XkbBindingsSeat as *mut _,
            Some(handle_bindings_seat_resource_destroy),
        );
    }
    
    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_XKB_BINDINGS_SEAT_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
            self.requested_next_key_change = XkbBindingsSeatRequestedNextKeyChange::None;
            self.requested_mods_watched = 0;
        }
    }
    
    pub unsafe fn manage_start(&mut self) {
        if self.scheduled_ate_unbound_key {
            if !self.object.is_null() {
                if ffi::wl_resource_get_version(self.object) >= 2 {
                    ffi::wl_resource_post_event(self.object, 0);
                }
            }
            self.scheduled_ate_unbound_key = false;
        }
        
        if let Some(mods) = self.scheduled_mods_update {
            if !self.object.is_null() {
                if ffi::wl_resource_get_version(self.object) >= 3 {
                    ffi::wl_resource_post_event(self.object, 1, mods.old, mods.new);
                }
            }
            self.scheduled_mods_update = None;
        }
    }
    
    pub unsafe fn manage_finish(&mut self) {
        match self.requested_next_key_change {
            XkbBindingsSeatRequestedNextKeyChange::None => {},
            XkbBindingsSeatRequestedNextKeyChange::EnsureEaten => {
                self.ensure_next_key_eaten = true;
            },
            XkbBindingsSeatRequestedNextKeyChange::CancelEnsureEaten => {
                self.ensure_next_key_eaten = false;
            },
        }
        self.requested_next_key_change = XkbBindingsSeatRequestedNextKeyChange::None;
    }
}

unsafe extern "C" fn handle_bindings_seat_resource_destroy(resource: *mut ffi::wl_resource) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut XkbBindingsSeat;
    if !seat.is_null() {
        (*seat).object = std::ptr::null_mut();
        (*seat).requested_next_key_change = XkbBindingsSeatRequestedNextKeyChange::None;
        (*seat).requested_mods_watched = 0;
    }
}

static XKB_BINDINGS_SEAT_INTERFACE: ffi::river_xkb_bindings_seat_v1_interface = ffi::river_xkb_bindings_seat_v1_interface {
    destroy: Some(bindings_seat_destroy),
    ensure_next_key_eaten: Some(bindings_seat_ensure_next_key_eaten),
    cancel_ensure_next_key_eaten: Some(bindings_seat_cancel_ensure_next_key_eaten),
    modifiers_watch: Some(bindings_seat_modifiers_watch),
};

unsafe extern "C" fn bindings_seat_inert_ensure_next_key_eaten(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn bindings_seat_inert_cancel_ensure_next_key_eaten(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn bindings_seat_inert_modifiers_watch(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _modifiers: u32,
) {}

static INERT_XKB_BINDINGS_SEAT_INTERFACE: ffi::river_xkb_bindings_seat_v1_interface = ffi::river_xkb_bindings_seat_v1_interface {
    destroy: Some(bindings_seat_destroy),
    ensure_next_key_eaten: Some(bindings_seat_inert_ensure_next_key_eaten),
    cancel_ensure_next_key_eaten: Some(bindings_seat_inert_cancel_ensure_next_key_eaten),
    modifiers_watch: Some(bindings_seat_inert_modifiers_watch),
};

unsafe extern "C" fn bindings_seat_destroy(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn bindings_seat_ensure_next_key_eaten(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let bindings_seat = ffi::wl_resource_get_user_data(resource) as *mut XkbBindingsSeat;
    if bindings_seat.is_null() {
        return;
    }
    let seat = crate::container_of!(bindings_seat, Seat, xkb_bindings_seat);
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    (*bindings_seat).requested_next_key_change = XkbBindingsSeatRequestedNextKeyChange::EnsureEaten;
}

unsafe extern "C" fn bindings_seat_cancel_ensure_next_key_eaten(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let bindings_seat = ffi::wl_resource_get_user_data(resource) as *mut XkbBindingsSeat;
    if bindings_seat.is_null() {
        return;
    }
    let seat = crate::container_of!(bindings_seat, Seat, xkb_bindings_seat);
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    (*bindings_seat).requested_next_key_change = XkbBindingsSeatRequestedNextKeyChange::CancelEnsureEaten;
}

unsafe extern "C" fn bindings_seat_modifiers_watch(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    modifiers: u32,
) {
    let bindings_seat = ffi::wl_resource_get_user_data(resource) as *mut XkbBindingsSeat;
    if bindings_seat.is_null() {
        return;
    }
    let seat = crate::container_of!(bindings_seat, Seat, xkb_bindings_seat);
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    (*bindings_seat).requested_mods_watched = modifiers;
}

pub struct XkbBindings {
    pub global: *mut ffi::wl_global,
    pub server: *mut Server,
}

impl XkbBindings {
    pub unsafe fn init(&mut self, server: *mut Server, wl_display: *mut ffi::wl_display) -> Result<(), ()> {
        self.server = server;
        self.global = ffi::wl_global_create(
            wl_display,
            &ffi::river_xkb_bindings_v1_interface,
            3,
            self as *mut XkbBindings as *mut _,
            Some(bind_xkb_bindings),
        );
        if self.global.is_null() {
            return Err(());
        }
        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        if !self.global.is_null() {
            ffi::wl_global_destroy(self.global);
            self.global = std::ptr::null_mut();
        }
    }
}

unsafe extern "C" fn bind_xkb_bindings(
    client: *mut ffi::wl_client,
    data: *mut std::ffi::c_void,
    version: u32,
    id: u32,
) {
    let bindings = data as *mut XkbBindings;
    let resource = ffi::wl_resource_create(client, &ffi::river_xkb_bindings_v1_interface, version as i32, id);
    if resource.is_null() {
        ffi::wl_client_post_no_memory(client);
        return;
    }
    
    ffi::wl_resource_set_implementation(
        resource,
        &XKB_BINDINGS_INTERFACE as *const _ as *const _,
        bindings as *mut _,
        None,
    );
}

static XKB_BINDINGS_INTERFACE: ffi::river_xkb_bindings_v1_interface = ffi::river_xkb_bindings_v1_interface {
    destroy: Some(bindings_destroy_request),
    get_xkb_binding: Some(bindings_get_xkb_binding),
    get_seat: Some(bindings_get_seat),
};

unsafe extern "C" fn bindings_destroy_request(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn bindings_get_xkb_binding(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    seat_resource: *mut ffi::wl_resource,
    id: u32,
    keysym: u32,
    modifiers: u32,
) {
    let seat = ffi::wl_resource_get_user_data(seat_resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    
    let version = ffi::wl_resource_get_version(resource) as u32;
    if XkbBinding::create(seat, client, version, id, keysym, modifiers).is_err() {
        log::error!("failed to create xkb binding");
    }
}

unsafe extern "C" fn bindings_get_seat(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    seat_resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(seat_resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    
    if !(*seat).xkb_bindings_seat.object.is_null() {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_xkb_bindings_v1_error_RIVER_XKB_BINDINGS_V1_ERROR_OBJECT_ALREADY_CREATED,
            b"river_xkb_bindings_seat_v1 already created\0".as_ptr() as *const _,
        );
        return;
    }
    
    let version = ffi::wl_resource_get_version(resource) as u32;
    (*seat).xkb_bindings_seat.create_object(client, version, id);
}

pub struct XkbBindingScheduled {
    pub state_change: XkbBindingStateChange,
}

pub struct XkbBindingRequested {
    pub enabled: bool,
    pub layout: Option<u32>,
}

pub struct XkbBinding {
    pub seat: *mut Seat,
    pub object: *mut ffi::wl_resource,
    pub keysym: u32,
    pub modifiers: u32,
    pub wm_scheduled: XkbBindingScheduled,
    pub wm_requested: XkbBindingRequested,
    pub sent_pressed: bool,
    pub link: ffi::wl_list,
}

impl XkbBinding {
    pub unsafe fn create(
        seat: *mut Seat,
        client: *mut ffi::wl_client,
        version: u32,
        id: u32,
        keysym: u32,
        modifiers: u32,
    ) -> Result<(), ()> {
        let binding_ptr = Box::into_raw(Box::new(Self {
            seat,
            object: std::ptr::null_mut(),
            keysym,
            modifiers,
            wm_scheduled: XkbBindingScheduled {
                state_change: XkbBindingStateChange::None,
            },
            wm_requested: XkbBindingRequested {
                enabled: false,
                layout: None,
            },
            sent_pressed: false,
            link: std::mem::zeroed(),
        }));
        
        let resource = ffi::wl_resource_create(client, &ffi::river_xkb_binding_v1_interface, version as i32, id);
        if resource.is_null() {
            let _ = Box::from_raw(binding_ptr);
            return Err(());
        }
        
        (*binding_ptr).object = resource;
        ffi::wl_resource_set_implementation(
            resource,
            &XKB_BINDING_INTERFACE as *const _ as *const _,
            binding_ptr as *mut _,
            Some(handle_binding_resource_destroy),
        );
        
        let bindings_list = &mut (*seat).xkb_bindings as *mut ffi::wl_list as *mut crate::server::WlList;
        crate::server::wl_list_insert((*bindings_list).prev, &mut (*binding_ptr).link as *mut ffi::wl_list as *mut crate::server::WlList);
        
        log::debug!("new river_xkb_binding_v1: keysym: {}, modifiers: {}", keysym, modifiers);
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
        assert!(!self.sent_pressed);
        if (*(*self.seat).server).wm.object.is_null() {
            log::warn!("Pressed keybind while window manager is disconnected");
            self.wm_scheduled.state_change = XkbBindingStateChange::None;
            return;
        }
        assert!(matches!(self.wm_scheduled.state_change, XkbBindingStateChange::None));
        self.wm_scheduled.state_change = XkbBindingStateChange::Pressed;
        (*(*self.seat).server).wm.dirty_windowing();
    }
    
    pub unsafe fn released(&mut self) {
        self.wm_scheduled.state_change = XkbBindingStateChange::Released;
        (*(*self.seat).server).wm.dirty_windowing();
    }

    pub unsafe fn stop_repeat(&mut self) {
        if self.sent_pressed {
            if matches!(self.wm_scheduled.state_change, XkbBindingStateChange::None) {
                self.wm_scheduled.state_change = XkbBindingStateChange::StopRepeat;
                (*(*self.seat).server).wm.dirty_windowing();
            }
        }
    }

    pub unsafe fn match_keycode(
        &self,
        keycode: u32,
        modifiers: u32,
        xkb_state: *mut ffi::xkb_state,
        translate: bool,
    ) -> bool {
        if !self.wm_requested.enabled {
            return false;
        }
        
        let keymap = ffi::xkb_state_get_keymap(xkb_state);
        if keymap.is_null() {
            return false;
        }
        
        let layout = match self.wm_requested.layout {
            Some(l) => l,
            None => ffi::xkb_state_key_get_layout(xkb_state, keycode),
        };
        
        if !translate {
            let mut syms_ptr: *const ffi::xkb_keysym_t = std::ptr::null();
            let num_syms = ffi::xkb_keymap_key_get_syms_by_level(keymap, keycode, layout, 0, &mut syms_ptr);
            if num_syms <= 0 || syms_ptr.is_null() {
                return false;
            }
            let syms = std::slice::from_raw_parts(syms_ptr, num_syms as usize);
            
            if modifiers == self.modifiers {
                for &sym in syms {
                    if sym == self.keysym {
                        return true;
                    }
                }
            }
        } else {
            let level = ffi::xkb_state_key_get_level(xkb_state, keycode, layout);
            let mut syms_ptr: *const ffi::xkb_keysym_t = std::ptr::null();
            let num_syms = ffi::xkb_keymap_key_get_syms_by_level(keymap, keycode, layout, level, &mut syms_ptr);
            if num_syms <= 0 || syms_ptr.is_null() {
                return false;
            }
            let syms = std::slice::from_raw_parts(syms_ptr, num_syms as usize);
            
            let consumed = ffi::xkb_state_key_get_consumed_mods2(xkb_state, keycode, ffi::xkb_consumed_mode_XKB_CONSUMED_MODE_XKB);
            let modifiers_translated = modifiers & !consumed;
            
            if modifiers_translated == self.modifiers {
                for &sym in syms {
                    if sym == self.keysym {
                        return true;
                    }
                }
            }
        }
        
        false
    }
}

unsafe extern "C" fn handle_binding_resource_destroy(resource: *mut ffi::wl_resource) {
    let binding = ffi::wl_resource_get_user_data(resource) as *mut XkbBinding;
    if !binding.is_null() {
        let seat = (*binding).seat;
        if !seat.is_null() {
            let seat_groups_head = &mut (*seat).keyboard_groups as *mut ffi::wl_list as *mut crate::server::WlList;
            let mut curr_g = (*seat_groups_head).next;
            while curr_g != seat_groups_head {
                let next_g = (*curr_g).next;
                let g = crate::container_of!(curr_g, crate::keyboard_group::KeyboardGroup, link);
                for press in (*g).pressed.values_mut() {
                    if let crate::keyboard_group::KeyConsumer::Binding(b) = press.consumer {
                        if b == binding {
                            press.consumer = crate::keyboard_group::KeyConsumer::Binding(std::ptr::null_mut());
                        }
                    }
                }
                curr_g = next_g;
            }
        }
        crate::server::wl_list_remove(&mut (*binding).link as *mut ffi::wl_list as *mut crate::server::WlList);
        let _ = Box::from_raw(binding);
    }
}

static XKB_BINDING_INTERFACE: ffi::river_xkb_binding_v1_interface = ffi::river_xkb_binding_v1_interface {
    destroy: Some(binding_destroy),
    set_layout_override: Some(binding_set_layout_override),
    enable: Some(binding_enable),
    disable: Some(binding_disable),
};

unsafe extern "C" fn binding_destroy(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn binding_set_layout_override(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    layout: u32,
) {
    let binding = ffi::wl_resource_get_user_data(resource) as *mut XkbBinding;
    if binding.is_null() {
        return;
    }
    let server = (*(*binding).seat).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    if layout == 0xffffffff {
        (*binding).wm_requested.layout = None;
    } else {
        (*binding).wm_requested.layout = Some(layout);
    }
}

unsafe extern "C" fn binding_enable(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let binding = ffi::wl_resource_get_user_data(resource) as *mut XkbBinding;
    if binding.is_null() {
        return;
    }
    let server = (*(*binding).seat).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*binding).wm_requested.enabled = true;
}

unsafe extern "C" fn binding_disable(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let binding = ffi::wl_resource_get_user_data(resource) as *mut XkbBinding;
    if binding.is_null() {
        return;
    }
    let server = (*(*binding).seat).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*binding).wm_requested.enabled = false;
}
