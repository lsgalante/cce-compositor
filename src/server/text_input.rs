// SPDX-FileCopyrightText: © 2021 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::seat::Seat;

#[repr(C)]
pub struct TextInput {
    pub wlr_text_input: *mut ffi::wlr_text_input_v3,

    pub enable: crate::listener::Listener,
    pub commit: crate::listener::Listener,
    pub disable: crate::listener::Listener,
    pub destroy: crate::listener::Listener,
}

impl TextInput {
    pub unsafe fn create(wm: &mut crate::window_manager::WindowManager, wlr_text_input: *mut ffi::wlr_text_input_v3) -> Result<(), &'static str> {
        let Some(sid) = Seat::id_of((*wlr_text_input).seat) else { return Err("Seat is null") };
        let Some(seat) = wm.seats.get_mut(sid) else { return Err("Seat is gone") };
        log::debug!("new text input on seat {}", seat.wlr_seat.name());

        let raw = Box::into_raw(Box::new(TextInput {
            wlr_text_input,
            enable: crate::listener::Listener::default(),
            commit: crate::listener::Listener::default(),
            disable: crate::listener::Listener::default(),
            destroy: crate::listener::Listener::default(),
        }));

        // Append to seat.relay.text_inputs
        seat.relay.text_inputs.push(raw);

        (*raw).enable.connect(&mut (*wlr_text_input).events.enable, handle_enable);
        (*raw).commit.connect(&mut (*wlr_text_input).events.commit, handle_commit);
        (*raw).disable.connect(&mut (*wlr_text_input).events.disable, handle_disable);
        (*raw).destroy.connect(&mut (*wlr_text_input).events.destroy, handle_destroy);

        // A client that binds its text input after its surface took focus
        // (cce-ui binds lazily) is entered now; `InputRelay::focus` only
        // enters on a focus change.
        let focus = seat.focused;
        let focused = focus.surface(wm);
        if !focused.is_null()
            && ffi::wl_resource_get_client(ffi::river_wlr_surface_get_resource(focused))
                == ffi::wl_resource_get_client((*wlr_text_input).resource)
        {
            ffi::wlr_text_input_v3_send_enter(wlr_text_input, focused);
        }

        Ok(())
    }
}

/// The seat id a text input's events are for, and the window manager.
unsafe fn entry(text_input: *mut TextInput) -> Option<(&'static mut crate::window_manager::WindowManager, crate::seat::SeatId)> {
    let sid = Seat::id_of((*(*text_input).wlr_text_input).seat)?;
    Some((&mut *crate::reentry::wm(crate::shared::server_ptr()), sid))
}

unsafe extern "C" fn handle_enable(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let text_input = crate::container_of!(listener, TextInput, enable);
    let Some((wm, sid)) = entry(text_input) else { return };
    let Some(seat) = wm.seats.get_mut(sid) else { return };

    if (*(*text_input).wlr_text_input).focused_surface.is_null() {
        log::error!("client requested to enable text input without focus, ignoring request");
        return;
    }

    if !seat.relay.text_input.is_null() && text_input != seat.relay.text_input {
        log::error!("client requested to enable more than one text input on a single seat, ignoring request");
        return;
    }

    seat.relay.text_input = text_input;
    seat.relay.osk.field_active();

    let input_method = seat.relay.input_method;
    if !input_method.is_null() {
        ffi::wlr_input_method_v2_send_activate(input_method);
        crate::input_relay::InputRelay::send_input_method_state(wm, sid);
    }
}

unsafe extern "C" fn handle_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let text_input = crate::container_of!(listener, TextInput, commit);
    let Some((wm, sid)) = entry(text_input) else { return };
    let Some(seat) = wm.seats.get_mut(sid) else { return };

    if seat.relay.text_input != text_input {
        log::error!("inactive text input tried to commit an update, client bug?");
        return;
    }

    seat.relay.osk.field_active();
    if !seat.relay.input_method.is_null() {
        crate::input_relay::InputRelay::send_input_method_state(wm, sid);
    }
}

unsafe extern "C" fn handle_disable(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let text_input = crate::container_of!(listener, TextInput, disable);
    let Some((wm, sid)) = entry(text_input) else { return };

    if wm.seats.get(sid).is_some_and(|seat| seat.relay.text_input == text_input) {
        crate::input_relay::InputRelay::disable_text_input(wm, sid);
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let text_input = crate::container_of!(listener, TextInput, destroy);
    // The seat's side first, if it is still in the store. The text input
    // itself is unhooked and freed either way: its signals are going.
    if let Some((wm, sid)) = entry(text_input) {
        if wm.seats.get(sid).is_some_and(|seat| seat.relay.text_input == text_input) {
            crate::input_relay::InputRelay::disable_text_input(wm, sid);
        }
        if let Some(seat) = wm.seats.get_mut(sid) {
            seat.relay.text_inputs.remove(text_input);
        }
    }

    (*text_input).enable.disconnect();
    (*text_input).commit.disconnect();
    (*text_input).disable.disconnect();
    (*text_input).destroy.disconnect();

    let _ = Box::from_raw(text_input);
}
