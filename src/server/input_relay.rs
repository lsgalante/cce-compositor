// SPDX-FileCopyrightText: © 2021 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{WlListener, wl_signal_add};
use crate::text_input::TextInput;
use crate::seat::Seat;

#[repr(C)]
pub struct InputRelay {
    pub seat: *mut Seat,
    pub text_inputs: ffi::wl_list,
    pub input_method: *mut ffi::wlr_input_method_v2,
    pub input_popups: ffi::wl_list,
    pub text_input: *mut TextInput,

    pub input_method_commit: ffi::wl_listener,
    pub grab_keyboard: ffi::wl_listener,
    pub input_method_destroy: ffi::wl_listener,
    pub input_method_new_popup: ffi::wl_listener,

    pub grab_keyboard_destroy: ffi::wl_listener,
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

impl InputRelay {
    pub unsafe fn init(&mut self, seat: *mut Seat) {
        self.seat = seat;
        ffi::wl_list_init(&mut self.text_inputs);
        ffi::wl_list_init(&mut self.input_popups);
        self.input_method = std::ptr::null_mut();
        self.text_input = std::ptr::null_mut();
        self.input_method_commit = std::mem::zeroed();
        self.grab_keyboard = std::mem::zeroed();
        self.input_method_destroy = std::mem::zeroed();
        self.input_method_new_popup = std::mem::zeroed();
        self.grab_keyboard_destroy = std::mem::zeroed();
    }

    pub unsafe fn new_input_method(&mut self, input_method: *mut ffi::wlr_input_method_v2) {
        let seat = crate::container_of!(self, Seat, relay);
        let seat_name = std::ffi::CStr::from_ptr(ffi::river_wlr_seat_get_name((*seat).wlr_seat))
            .to_str()
            .unwrap_or("unknown");
        log::debug!("new input method on seat {}", seat_name);

        if !self.input_method.is_null() {
            log::info!("seat {} already has an input method", seat_name);
            ffi::wlr_input_method_v2_send_unavailable(input_method);
            return;
        }

        self.input_method = input_method;

        connect_listener(&mut (*input_method).events.commit, &mut self.input_method_commit, handle_input_method_commit);
        connect_listener(&mut (*input_method).events.grab_keyboard, &mut self.grab_keyboard, handle_input_method_grab_keyboard);
        connect_listener(&mut (*input_method).events.destroy, &mut self.input_method_destroy, handle_input_method_destroy);
        connect_listener(&mut (*input_method).events.new_popup_surface, &mut self.input_method_new_popup, handle_input_method_new_popup);

        let focused_surface = (*seat).focused.surface();
        if !focused_surface.is_null() {
            self.focus(focused_surface);
        }
    }

    pub unsafe fn disable_text_input(&mut self) {
        assert!(!self.text_input.is_null());
        self.text_input = std::ptr::null_mut();

        if !self.input_method.is_null() {
            let mut pos = self.input_popups.next;
            let head_ptr = &self.input_popups as *const ffi::wl_list;
            while pos != head_ptr as *mut ffi::wl_list {
                let next_pos = (*pos).next;
                let popup = crate::container_of!(pos, crate::input_popup::InputPopup, link);
                (*popup).update();
                pos = next_pos;
            }

            ffi::wlr_input_method_v2_send_deactivate(self.input_method);
            ffi::wlr_input_method_v2_send_done(self.input_method);
        }
    }

    pub unsafe fn send_input_method_state(&mut self) {
        let input_method = self.input_method;
        let wlr_text_input = (*self.text_input).wlr_text_input;

        let active_features = (*wlr_text_input).active_features;
        let feature_surrounding = ffi::wlr_text_input_v3_features_WLR_TEXT_INPUT_V3_FEATURE_SURROUNDING_TEXT;
        let feature_content = ffi::wlr_text_input_v3_features_WLR_TEXT_INPUT_V3_FEATURE_CONTENT_TYPE;

        if (active_features & feature_surrounding) != 0 {
            let text = (*wlr_text_input).current.surrounding.text;
            if !text.is_null() {
                ffi::wlr_input_method_v2_send_surrounding_text(
                    input_method,
                    text,
                    (*wlr_text_input).current.surrounding.cursor,
                    (*wlr_text_input).current.surrounding.anchor,
                );
            }
        }

        ffi::wlr_input_method_v2_send_text_change_cause(input_method, (*wlr_text_input).current.text_change_cause);

        if (active_features & feature_content) != 0 {
            ffi::wlr_input_method_v2_send_content_type(
                input_method,
                (*wlr_text_input).current.content_type.hint,
                (*wlr_text_input).current.content_type.purpose,
            );
        }

        let mut pos = self.input_popups.next;
        let head_ptr = &self.input_popups as *const ffi::wl_list;
        while pos != head_ptr as *mut ffi::wl_list {
            let next_pos = (*pos).next;
            let popup = crate::container_of!(pos, crate::input_popup::InputPopup, link);
            (*popup).update();
            pos = next_pos;
        }

        ffi::wlr_input_method_v2_send_done(input_method);
    }

    pub unsafe fn focus(&mut self, new_focus: *mut ffi::wlr_surface) {
        // Send leave events
        let mut pos = self.text_inputs.next;
        let head_ptr = &self.text_inputs as *const ffi::wl_list;
        while pos != head_ptr as *mut ffi::wl_list {
            let next_pos = (*pos).next;
            let text_input = crate::container_of!(pos, TextInput, link);
            let focused = (*(*text_input).wlr_text_input).focused_surface;
            if !focused.is_null() {
                assert!(focused != new_focus);
                ffi::wlr_text_input_v3_send_leave((*text_input).wlr_text_input);
            }
            pos = next_pos;
        }

        // Clear currently enabled text input
        if !self.text_input.is_null() {
            self.disable_text_input();
        }

        // Send enter events if we have an input method
        if !new_focus.is_null() && !self.input_method.is_null() {
            let new_client = ffi::wl_resource_get_client(ffi::river_wlr_surface_get_resource(new_focus));
            let mut pos = self.text_inputs.next;
            while pos != head_ptr as *mut ffi::wl_list {
                let next_pos = (*pos).next;
                let text_input = crate::container_of!(pos, TextInput, link);
                let text_input_resource = (*(*text_input).wlr_text_input).resource;
                let client = ffi::wl_resource_get_client(text_input_resource);
                if client == new_client {
                    ffi::wlr_text_input_v3_send_enter((*text_input).wlr_text_input, new_focus);
                }
                pos = next_pos;
            }
        }
    }
}

unsafe extern "C" fn handle_input_method_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let relay = crate::container_of!(listener, InputRelay, input_method_commit);
    let input_method = (*relay).input_method;

    if !(*input_method).client_active {
        return;
    }

    let text_input = (*relay).text_input;
    if text_input.is_null() {
        return;
    }

    let preedit_text = (*input_method).current.preedit.text;
    if !preedit_text.is_null() {
        ffi::wlr_text_input_v3_send_preedit_string(
            (*text_input).wlr_text_input,
            preedit_text,
            (*input_method).current.preedit.cursor_begin,
            (*input_method).current.preedit.cursor_end,
        );
    }

    let commit_text = (*input_method).current.commit_text;
    if !commit_text.is_null() {
        ffi::wlr_text_input_v3_send_commit_string((*text_input).wlr_text_input, commit_text);
    }

    if (*input_method).current.delete.before_length != 0
        || (*input_method).current.delete.after_length != 0
    {
        ffi::wlr_text_input_v3_send_delete_surrounding_text(
            (*text_input).wlr_text_input,
            (*input_method).current.delete.before_length,
            (*input_method).current.delete.after_length,
        );
    }

    ffi::wlr_text_input_v3_send_done((*text_input).wlr_text_input);
}

unsafe extern "C" fn handle_input_method_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let relay = crate::container_of!(listener, InputRelay, input_method_destroy);

    wl_listener_remove_safe(&mut (*relay).input_method_commit);
    wl_listener_remove_safe(&mut (*relay).grab_keyboard);
    wl_listener_remove_safe(&mut (*relay).input_method_destroy);
    wl_listener_remove_safe(&mut (*relay).input_method_new_popup);
    (*relay).input_method = std::ptr::null_mut();

    (*relay).focus(std::ptr::null_mut());

    assert!((*relay).text_input.is_null());
}

unsafe extern "C" fn handle_input_method_grab_keyboard(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let relay = crate::container_of!(listener, InputRelay, grab_keyboard);
    let keyboard_grab = data as *mut ffi::wlr_input_method_keyboard_grab_v2;
    let seat = crate::container_of!(relay, Seat, relay);

    let active_keyboard = ffi::river_wlr_seat_get_keyboard((*seat).wlr_seat);
    ffi::wlr_input_method_keyboard_grab_v2_set_keyboard(keyboard_grab, active_keyboard);

    connect_listener(
        ffi::river_wlr_input_method_keyboard_grab_v2_get_destroy_signal(keyboard_grab),
        &mut (*relay).grab_keyboard_destroy,
        handle_input_method_grab_keyboard_destroy,
    );
}

unsafe extern "C" fn handle_input_method_new_popup(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let relay = crate::container_of!(listener, InputRelay, input_method_new_popup);
    let wlr_popup = data as *mut ffi::wlr_input_popup_surface_v2;

    if let Err(e) = crate::input_popup::InputPopup::create(wlr_popup, relay) {
        log::error!("failed to create input popup: {}", e);
    }
}

unsafe extern "C" fn handle_input_method_grab_keyboard_destroy(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let relay = crate::container_of!(listener, InputRelay, grab_keyboard_destroy);
    let input_method = (*relay).input_method;
    let keyboard_grab = (*input_method).keyboard_grab;
    wl_listener_remove_safe(&mut (*relay).grab_keyboard_destroy);

    let keyboard = ffi::river_wlr_input_method_keyboard_grab_v2_get_keyboard(keyboard_grab);
    if !keyboard.is_null() {
        let modifiers = ffi::river_wlr_keyboard_get_modifiers(keyboard);
        ffi::wlr_seat_keyboard_notify_modifiers((*input_method).seat, modifiers);
    }
}
