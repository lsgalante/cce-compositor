// SPDX-License-Identifier: GPL-3.0-only

//! wp_tearing_control_v1: a client's hint that a surface would rather tear
//! than wait for vblank (a game presenting with vsync off; Xwayland forwards
//! an X client's async Present flips as the same hint).
//!
//! The hint itself is read where it is used — `Window::presentation_hint`
//! asks wlroots for the surface's current hint, and the render pass
//! (`render_finish`) lets an output tear only for the fullscreen window on
//! top of it. What lives here is the wake-up: a hint changes on a surface
//! commit that runs no transaction of its own, and a game typically starts
//! presenting async only after it went fullscreen, so each hint object asks
//! for a render pass when it changes or goes away.

use crate::ffi;

/// Listens to one `wlr_tearing_control_v1`; freed with it.
struct TearingHint {
    control: *mut ffi::wlr_tearing_control_v1,
    set_hint: crate::listener::Listener,
    destroy: crate::listener::Listener,
}

pub unsafe extern "C" fn handle_new_tearing_hint(_listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let control = data as *mut ffi::wlr_tearing_control_v1;
    if control.is_null() {
        return;
    }
    let hint = Box::into_raw(Box::new(TearingHint {
        control,
        set_hint: crate::listener::Listener::new(),
        destroy: crate::listener::Listener::new(),
    }));
    (*hint).set_hint.connect(&mut (*control).events.set_hint, handle_set_hint);
    (*hint).destroy.connect(&mut (*control).events.destroy, handle_destroy);
}

/// Emitted (with no data) by the surface commit that changed the current
/// hint, so `current` is already the new one.
unsafe extern "C" fn handle_set_hint(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let hint = crate::container_of!(listener, TearingHint, set_hint);
    let control = (*hint).control;
    let async_hint = (*control).current
        == ffi::wp_tearing_control_v1_presentation_hint_WP_TEARING_CONTROL_V1_PRESENTATION_HINT_ASYNC;
    log::debug!(
        "tearing hint: surface {:p} -> {}",
        (*control).surface,
        if async_hint { "async" } else { "vsync" },
    );
    crate::shared::pending().dirty_rendering();
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let hint = crate::container_of!(listener, TearingHint, destroy);
    (*hint).set_hint.disconnect();
    (*hint).destroy.disconnect();
    drop(Box::from_raw(hint));
    crate::shared::pending().dirty_rendering();
}
