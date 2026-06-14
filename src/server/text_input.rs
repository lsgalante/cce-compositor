// SPDX-FileCopyrightText: © 2021 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{WlListener, wl_listener_remove, wl_signal_add, WlList};
use crate::seat::Seat;

#[repr(C)]
pub struct TextInput {
    pub link: ffi::wl_list,
    pub wlr_text_input: *mut ffi::wlr_text_input_v3,

    pub enable: ffi::wl_listener,
    pub commit: ffi::wl_listener,
    pub disable: ffi::wl_listener,
    pub destroy: ffi::wl_listener,
}

unsafe fn connect_listener(
    signal: *mut ffi::wl_signal,
    listener: *mut ffi::wl_listener,
    callback: unsafe extern "C" fn(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void),
) {
    let wl_lis = listener as *mut WlListener;
    (*wl_lis).notify = Some(callback);
    wl_signal_add(signal, listener);
}

unsafe fn wl_listener_remove_safe(listener: *mut ffi::wl_listener) {
    let prev = (*listener).link.prev;
    let next = (*listener).link.next;
    if !prev.is_null() && !next.is_null() && prev != listener as *mut ffi::wl_list && next != listener as *mut ffi::wl_list {
        ffi::wl_list_remove(&mut (*listener).link);
        (*listener).link.prev = std::ptr::null_mut();
        (*listener).link.next = std::ptr::null_mut();
    }
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

        connect_listener(&mut (*wlr_text_input).events.enable, &mut (*raw).enable, handle_enable);
        connect_listener(&mut (*wlr_text_input).events.commit, &mut (*raw).commit, handle_commit);
        connect_listener(&mut (*wlr_text_input).events.disable, &mut (*raw).disable, handle_disable);
        connect_listener(&mut (*wlr_text_input).events.destroy, &mut (*raw).destroy, handle_destroy);

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
        (*seat).relay.disable_text_input();
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let text_input = crate::container_of!(listener, TextInput, destroy);
    let seat = ffi::river_wlr_seat_get_data((*(*text_input).wlr_text_input).seat) as *mut Seat;
    if seat.is_null() {
        return;
    }

    if (*seat).relay.text_input == text_input {
        (*seat).relay.disable_text_input();
    }

    wl_listener_remove_safe(&mut (*text_input).enable);
    wl_listener_remove_safe(&mut (*text_input).commit);
    wl_listener_remove_safe(&mut (*text_input).disable);
    wl_listener_remove_safe(&mut (*text_input).destroy);

    ffi::wl_list_remove(&mut (*text_input).link);

    let _ = Box::from_raw(text_input);
}
