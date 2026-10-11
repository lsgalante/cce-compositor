// SPDX-FileCopyrightText: © 2021 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{WlList};
use crate::seat::Seat;

#[repr(C)]
pub struct TextInput {
    pub link: ffi::wl_list,
    pub wlr_text_input: *mut ffi::wlr_text_input_v3,

    pub enable: crate::listener::Listener,
    pub commit: crate::listener::Listener,
    pub disable: crate::listener::Listener,
    pub destroy: crate::listener::Listener,
}

impl TextInput {
    pub unsafe fn create(wlr_text_input: *mut ffi::wlr_text_input_v3) -> Result<(), &'static str> {
        let wlr_seat = (*wlr_text_input).seat;
        let seat = ffi::river_wlr_seat_get_data(wlr_seat) as *mut Seat;
        if seat.is_null() {
            return Err("Seat is null");
        }

        let seat_name = std::ffi::CStr::from_ptr(ffi::river_wlr_seat_get_name(wlr_seat))
            .to_str()
            .unwrap_or("unknown");
        log::debug!("new text input on seat {}", seat_name);

        let mut text_input = Box::new(TextInput {
            link: std::mem::zeroed(),
            wlr_text_input,
            enable: std::mem::zeroed(),
            commit: std::mem::zeroed(),
            disable: std::mem::zeroed(),
            destroy: std::mem::zeroed(),
        });

        ffi::wl_list_init(&mut text_input.link);

        let raw = Box::into_raw(text_input);

        // Append to seat.relay.text_inputs
        let text_inputs_list = &mut (*seat).relay.text_inputs as *mut ffi::wl_list as *mut WlList;
        crate::server::wl_list_insert((*text_inputs_list).prev, &mut (*raw).link as *mut ffi::wl_list as *mut WlList);

        (*raw).enable.connect(&mut (*wlr_text_input).events.enable, handle_enable);
        (*raw).commit.connect(&mut (*wlr_text_input).events.commit, handle_commit);
        (*raw).disable.connect(&mut (*wlr_text_input).events.disable, handle_disable);
        (*raw).destroy.connect(&mut (*wlr_text_input).events.destroy, handle_destroy);

        // A client that binds its text input after its surface took focus
        // (cce-ui binds lazily) is entered now; `InputRelay::focus` only
        // enters on a focus change.
        let focused = (*seat).focused.surface();
        if !focused.is_null()
            && ffi::wl_resource_get_client(ffi::river_wlr_surface_get_resource(focused))
                == ffi::wl_resource_get_client((*wlr_text_input).resource)
        {
            ffi::wlr_text_input_v3_send_enter(wlr_text_input, focused);
        }

        Ok(())
    }
}

unsafe extern "C" fn handle_enable(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let text_input = crate::container_of!(listener, TextInput, enable);
    let seat = ffi::river_wlr_seat_get_data((*(*text_input).wlr_text_input).seat) as *mut Seat;
    if seat.is_null() {
        return;
    }

    if (*(*text_input).wlr_text_input).focused_surface.is_null() {
        log::error!("client requested to enable text input without focus, ignoring request");
        return;
    }

    if !(*seat).relay.text_input.is_null() {
        if text_input != (*seat).relay.text_input {
            log::error!("client requested to enable more than one text input on a single seat, ignoring request");
            return;
        }
    }

    (*seat).relay.text_input = text_input;
    (*seat).relay.osk.field_active(&mut *crate::reentry::wm((*seat).server));

    let input_method = (*seat).relay.input_method;
    if !input_method.is_null() {
        ffi::wlr_input_method_v2_send_activate(input_method);
        (*seat).relay.send_input_method_state();
    }
}

unsafe extern "C" fn handle_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let text_input = crate::container_of!(listener, TextInput, commit);
    let seat = ffi::river_wlr_seat_get_data((*(*text_input).wlr_text_input).seat) as *mut Seat;
    if seat.is_null() {
        return;
    }

    if (*seat).relay.text_input != text_input {
        log::error!("inactive text input tried to commit an update, client bug?");
        return;
    }

    (*seat).relay.osk.field_active(&mut *crate::reentry::wm((*seat).server));
    if !(*seat).relay.input_method.is_null() {
        (*seat).relay.send_input_method_state();
    }
}

unsafe extern "C" fn handle_disable(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let text_input = crate::container_of!(listener, TextInput, disable);
    let seat = ffi::river_wlr_seat_get_data((*(*text_input).wlr_text_input).seat) as *mut Seat;
    if seat.is_null() {
        return;
    }

    if (*seat).relay.text_input == text_input {
        (*seat).relay.disable_text_input(&mut *crate::reentry::wm((*seat).server));
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let text_input = crate::container_of!(listener, TextInput, destroy);
    let seat = ffi::river_wlr_seat_get_data((*(*text_input).wlr_text_input).seat) as *mut Seat;
    if seat.is_null() {
        return;
    }

    if (*seat).relay.text_input == text_input {
        (*seat).relay.disable_text_input(&mut *crate::reentry::wm((*seat).server));
    }

    (*text_input).enable.disconnect();
    (*text_input).commit.disconnect();
    (*text_input).disable.disconnect();
    (*text_input).destroy.disconnect();

    ffi::wl_list_remove(&mut (*text_input).link);

    let _ = Box::from_raw(text_input);
}
