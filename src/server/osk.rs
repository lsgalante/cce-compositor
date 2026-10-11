//! The on-screen keyboard follows a touched text field.
//!
//! When a client enables (or updates) a text-input-v3 field right after a
//! finger landed on one of its windows, the board is shown with
//! `cce-keyboard show`; when the field lets go it is hidden again with
//! `cce-keyboard hide`, but only if this module showed it — a board summoned
//! by hand (Super+O) stays until it is dismissed by hand.
//!
//! The touch is the whole signal: text-input-v3 says that a field is active,
//! never why, and a field focused from the keyboard or with the pointer must
//! not raise the board. So a touch on a window arms a short window
//! (`TOUCH_WINDOW`), and the first activation inside it spends it. Spending
//! it matters: every key tapped on the board makes the client commit a new
//! caret, and a touch that could be reused would re-launch `show` per key.
//! Touches on the board itself never arm it (they land on a layer surface).
//!
//! The hide waits `HIDE_DELAY_MS`: moving from one field to the next
//! disables one text input and enables the other, and the board should not
//! close and reopen in between.
//!
//! Off with `window_manager { osk_on_touch (bool)false }`.

use std::time::{Duration, Instant};

use crate::ffi;
use crate::server::Server;

/// How long after a touch on a window an activation still counts as the
/// touch's. A tap that the client sees as a button press (no wl_touch) is
/// only delivered at the lift, and the field enables a frame or two after
/// that; a long press must still be inside it.
const TOUCH_WINDOW: Duration = Duration::from_millis(800);

/// How long a field may be gone before the board follows it.
const HIDE_DELAY_MS: i32 = 250;

const SHOW_CMD: &str = "cce-keyboard show";
const HIDE_CMD: &str = "cce-keyboard hide";

pub struct Osk {
    server: *mut Server,
    /// The last touch on a window, until an activation spends it.
    armed_at: Option<Instant>,
    /// This module put the board up, so it may take it down.
    shown_by_us: bool,
    hide_timer: *mut ffi::wl_event_source,
}

impl Osk {
    pub fn new(server: *mut Server) -> Self {
        Osk { server, armed_at: None, shown_by_us: false, hide_timer: std::ptr::null_mut() }
    }

    unsafe fn enabled(&self) -> bool {
        !self.server.is_null() && crate::shared::layout().osk_on_touch
    }

    /// A finger landed on (or, for a tap, lifted from) a window.
    pub fn note_touch(&mut self) {
        self.armed_at = Some(Instant::now());
    }

    /// Whether a touch armed the board recently enough to count, disarming
    /// it either way.
    fn spend_touch(&mut self, now: Instant) -> bool {
        self.armed_at.take().is_some_and(|t| now.saturating_duration_since(t) <= TOUCH_WINDOW)
    }

    /// The seat's text input was enabled, or committed new state while
    /// enabled.
    pub unsafe fn field_active(&mut self, wm: &mut crate::window_manager::WindowManager) {
        // Whatever raised the field, a hide queued by the last one is moot.
        self.cancel_hide();
        if !self.enabled() {
            return;
        }
        if !self.spend_touch(Instant::now()) {
            return;
        }
        log::debug!("osk: a touched field activated; showing the board");
        wm.execute_action(&crate::config::Action::Spawn, Some(SHOW_CMD));
        self.shown_by_us = true;
    }

    /// The seat's text input went away (disabled, destroyed, or focus left).
    pub unsafe fn field_gone(&mut self, wm: &mut crate::window_manager::WindowManager) {
        if !self.shown_by_us || self.server.is_null() {
            return;
        }
        if self.hide_timer.is_null() {
            let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
            self.hide_timer = ffi::wl_event_loop_add_timer(event_loop, Some(handle_hide_timer), self as *mut Osk as *mut _);
            if self.hide_timer.is_null() {
                log::error!("osk: failed to create the hide timer; hiding now");
                self.hide_now(wm);
                return;
            }
        }
        ffi::wl_event_source_timer_update(self.hide_timer, HIDE_DELAY_MS);
    }

    unsafe fn cancel_hide(&mut self) {
        if !self.hide_timer.is_null() {
            ffi::wl_event_source_timer_update(self.hide_timer, 0);
        }
    }

    unsafe fn hide_now(&mut self, wm: &mut crate::window_manager::WindowManager) {
        if !self.shown_by_us {
            return;
        }
        self.shown_by_us = false;
        log::debug!("osk: the field let go; hiding the board");
        wm.execute_action(&crate::config::Action::Spawn, Some(HIDE_CMD));
    }
}

unsafe extern "C" fn handle_hide_timer(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let osk = &mut *(data as *mut Osk);
    osk.hide_now(&mut *crate::reentry::wm(osk.server));
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_touch_is_spent_by_one_activation() {
        let mut osk = Osk::new(std::ptr::null_mut());
        osk.note_touch();
        let now = Instant::now();
        assert!(osk.spend_touch(now));
        assert!(!osk.spend_touch(now), "the second activation is the caret moving, not a new tap");
    }

    #[test]
    fn a_stale_touch_does_not_count() {
        let mut osk = Osk::new(std::ptr::null_mut());
        osk.note_touch();
        assert!(!osk.spend_touch(Instant::now() + TOUCH_WINDOW + Duration::from_millis(1)));
        assert!(osk.armed_at.is_none());
    }

    #[test]
    fn without_a_touch_nothing_shows() {
        let mut osk = Osk::new(std::ptr::null_mut());
        assert!(!osk.spend_touch(Instant::now()));
    }
}
