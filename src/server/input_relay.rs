// SPDX-FileCopyrightText: © 2021 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::text_input::TextInput;
use crate::seat::Seat;

#[repr(C)]
pub struct InputRelay {
    pub text_inputs: crate::registry::Registry<TextInput>,
    pub input_method: *mut ffi::wlr_input_method_v2,
    pub input_popups: ffi::wl_list,
    pub text_input: *mut TextInput,

    pub input_method_commit: crate::listener::Listener,
    pub grab_keyboard: crate::listener::Listener,
    pub input_method_destroy: crate::listener::Listener,
    pub input_method_new_popup: crate::listener::Listener,

    pub grab_keyboard_destroy: crate::listener::Listener,

    /// The on-screen keyboard following a touched field (`osk.rs`).
    pub osk: crate::osk::Osk,
}

impl InputRelay {
    /// A relay not yet in place; `init` finishes it once it is
    /// (the popup list head must not move after it is initialised).
    pub fn new() -> Self {
        InputRelay {
            text_inputs: crate::registry::Registry::new(),
            input_method: std::ptr::null_mut(),
            // SAFETY: a zeroed wl_list is a valid not-yet-initialised head;
            // `init` initialises it in place.
            input_popups: unsafe { std::mem::zeroed() },
            text_input: std::ptr::null_mut(),
            input_method_commit: crate::listener::Listener::default(),
            grab_keyboard: crate::listener::Listener::default(),
            input_method_destroy: crate::listener::Listener::default(),
            input_method_new_popup: crate::listener::Listener::default(),
            grab_keyboard_destroy: crate::listener::Listener::default(),
            osk: crate::osk::Osk::new(),
        }
    }

    pub unsafe fn init(&mut self) {
        ffi::wl_list_init(&mut self.input_popups);
    }

    /// The input-method popups, oldest first, copied out: `InputPopup::update`
    /// reads the relay back through its own pointer, so it runs with no
    /// borrow of the relay live.
    fn popups(&self) -> Vec<*mut crate::input_popup::InputPopup> {
        let mut out = Vec::new();
        let head = &self.input_popups as *const ffi::wl_list as *mut ffi::wl_list;
        // SAFETY: `init` initialised the head in place, and every link on
        // it is a live popup's (a popup unlinks itself on destroy).
        unsafe {
            let mut pos = (*head).next;
            while pos != head {
                out.push(crate::container_of!(pos, crate::input_popup::InputPopup, link));
                pos = (*pos).next;
            }
        }
        out
    }

    /// Reposition every input-method popup (the field or the window moved).
    unsafe fn update_popups(wm: &crate::window_manager::WindowManager, sid: crate::seat::SeatId) {
        let Some(seat) = wm.seats.get(sid) else { return };
        for popup in seat.relay.popups() {
            (*popup).update(wm);
        }
    }

    pub unsafe fn new_input_method(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, input_method: *mut ffi::wlr_input_method_v2) {
        let seat = seat_mut!(wm, sid);
        let seat_name = seat.wlr_seat.name();
        log::debug!("new input method on seat {}", seat_name);
        let this = &mut seat.relay;

        if !this.input_method.is_null() {
            log::info!("seat {} already has an input method", seat_name);
            ffi::wlr_input_method_v2_send_unavailable(input_method);
            return;
        }

        this.input_method = input_method;

        this.input_method_commit.connect(&mut (*input_method).events.commit, handle_input_method_commit);
        this.grab_keyboard.connect(&mut (*input_method).events.grab_keyboard, handle_input_method_grab_keyboard);
        this.input_method_destroy.connect(&mut (*input_method).events.destroy, handle_input_method_destroy);
        this.input_method_new_popup.connect(&mut (*input_method).events.new_popup_surface, handle_input_method_new_popup);

        // Text inputs are entered whether or not an input method exists
        // (`focus`), so one that arrives late only needs telling about the
        // field already enabled.
        if !this.text_input.is_null() {
            ffi::wlr_input_method_v2_send_activate(input_method);
            InputRelay::send_input_method_state(wm, sid);
        }
    }

    pub unsafe fn disable_text_input(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId) {
        let this = &mut seat_mut!(wm, sid).relay;
        assert!(!this.text_input.is_null());
        this.text_input = std::ptr::null_mut();
        this.osk.field_gone(sid);

        let input_method = this.input_method;
        if !input_method.is_null() {
            InputRelay::update_popups(wm, sid);
            ffi::wlr_input_method_v2_send_deactivate(input_method);
            ffi::wlr_input_method_v2_send_done(input_method);
        }
    }

    pub unsafe fn send_input_method_state(wm: &crate::window_manager::WindowManager, sid: crate::seat::SeatId) {
        let Some(seat) = wm.seats.get(sid) else { return };
        let input_method = seat.relay.input_method;
        let wlr_text_input = (*seat.relay.text_input).wlr_text_input;

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

        InputRelay::update_popups(wm, sid);

        ffi::wlr_input_method_v2_send_done(input_method);
    }

    /// Keyboard focus moved to `new_focus` (null: nowhere). Text inputs of
    /// the client that lost it are left, and those of the client that got it
    /// entered — whether or not an input method is running. river entered
    /// them only with one, but then no client ever enabled a field, and the
    /// on-screen keyboard (`osk.rs`) is driven by exactly that enable.
    pub unsafe fn focus(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, new_focus: *mut ffi::wlr_surface) {
        let this = &mut seat_mut!(wm, sid).relay;
        // Send leave events. A text input already on the new surface keeps
        // it: a refocus of the same surface is not a leave.
        for text_input in this.text_inputs.ptrs() {
            let focused = (*(*text_input).wlr_text_input).focused_surface;
            if !focused.is_null() && focused != new_focus {
                ffi::wlr_text_input_v3_send_leave((*text_input).wlr_text_input);
            }
        }

        // Clear the enabled text input unless its surface kept focus
        if !this.text_input.is_null() {
            let focused = (*(*this.text_input).wlr_text_input).focused_surface;
            if focused.is_null() || focused != new_focus {
                InputRelay::disable_text_input(wm, sid);
            }
        }

        if new_focus.is_null() {
            return;
        }
        let this = &mut seat_mut!(wm, sid).relay;
        let new_client = ffi::wl_resource_get_client(ffi::river_wlr_surface_get_resource(new_focus));
        for text_input in this.text_inputs.ptrs() {
            let wlr_text_input = (*text_input).wlr_text_input;
            if (*wlr_text_input).focused_surface.is_null()
                && ffi::wl_resource_get_client((*wlr_text_input).resource) == new_client
            {
                ffi::wlr_text_input_v3_send_enter(wlr_text_input, new_focus);
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

    (*relay).input_method_commit.disconnect();
    (*relay).grab_keyboard.disconnect();
    (*relay).input_method_destroy.disconnect();
    (*relay).input_method_new_popup.disconnect();
    (*relay).input_method = std::ptr::null_mut();
    // The text inputs stay entered and enabled: they never depended on the
    // input method (`InputRelay::focus`).
}

unsafe extern "C" fn handle_input_method_grab_keyboard(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let relay = crate::container_of!(listener, InputRelay, grab_keyboard);
    let keyboard_grab = data as *mut ffi::wlr_input_method_keyboard_grab_v2;
    let seat = crate::container_of!(relay, Seat, relay);

    let active_keyboard = (*seat).wlr_seat.keyboard();
    ffi::wlr_input_method_keyboard_grab_v2_set_keyboard(keyboard_grab, active_keyboard);

    (*relay).grab_keyboard_destroy.connect(ffi::river_wlr_input_method_keyboard_grab_v2_get_destroy_signal(keyboard_grab), handle_input_method_grab_keyboard_destroy);
}

unsafe extern "C" fn handle_input_method_new_popup(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let relay = crate::container_of!(listener, InputRelay, input_method_new_popup);
    let wlr_popup = data as *mut ffi::wlr_input_popup_surface_v2;

    let wm = &*crate::reentry::wm(crate::shared::server_ptr());
    if let Err(e) = crate::input_popup::InputPopup::create(wm, wlr_popup, relay) {
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
    (*relay).grab_keyboard_destroy.disconnect();

    let keyboard = ffi::river_wlr_input_method_keyboard_grab_v2_get_keyboard(keyboard_grab);
    if !keyboard.is_null() {
        let modifiers = ffi::river_wlr_keyboard_get_modifiers(keyboard);
        ffi::wlr_seat_keyboard_notify_modifiers((*input_method).seat, modifiers);
    }
}
