//! Touchpad gestures: swipe, pinch and hold, how a swipe leans and which way it
//! navigates, and running the bound action. Split out of cursor.rs on 2026-10-10.

use super::*;

// The gesture arithmetic and what an action means for navigation are the
// policy crate's (`cce_window_manager::gesture`); re-exported under the names
// the handlers here use.
pub(crate) use crate::policy::gesture::{action_navigates, is_directional_focus, swipe_lean};

/// The sense a swipe along `finger_dir` gives its axis when aiming focus,
/// from the first bind that matches it (first match wins, as in the fire
/// itself): `gesture::focus_sense` of that bind's action.
pub(crate) fn focus_axis_sense(binds: &[crate::config::GestureBind], fingers: u32, mods: u32, finger_dir: &str) -> Option<f64> {
    let gb = binds
        .iter()
        .find(|gb| gb.gesture_type == "swipe" && gb.fingers == fingers && gb.mods == mods && gb.direction == finger_dir)?;
    crate::policy::gesture::focus_sense(gb.action, finger_dir)
}

/// The direction a focus swipe aims in, from this step's `travel`: each
/// axis's travel times its sense (`focus_axis_sense`), zero on an axis
/// with no focus bind the way the fingers went. `None` when neither axis
/// aims.
pub(crate) fn swipe_focus_vector(binds: &[crate::config::GestureBind], fingers: u32, mods: u32, travel: (f64, f64)) -> Option<(f64, f64)> {
    let (dx, dy) = travel;
    let sx = if dx != 0.0 { focus_axis_sense(binds, fingers, mods, if dx < 0.0 { "left" } else { "right" }) } else { None };
    let sy = if dy != 0.0 { focus_axis_sense(binds, fingers, mods, if dy < 0.0 { "up" } else { "down" }) } else { None };
    let v = (sx.map_or(0.0, |s| s * dx), sy.map_or(0.0, |s| s * dy));
    (v != (0.0, 0.0)).then_some(v)
}

pub(crate) unsafe extern "C" fn handle_swipe_begin(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let sid = (*crate::container_of!(listener, Cursor, swipe_begin_listener)).sid;
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    on_swipe_begin(wm, sid, data, sys);
}

/// `handle_swipe_begin` with the window manager passed in; `ccectl`'s injection calls this from inside `process_ipc_command`, with its `self`.
pub(crate) unsafe fn on_swipe_begin(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, data: *mut std::ffi::c_void, sys: &mut crate::ctx::Sys) {
    let Some(sptr) = wm.seats.ptr(sid) else { return () };
    let cptr: *mut Cursor = std::ptr::addr_of_mut!((*sptr).cursor);
    let _ = cptr;
    let event = data as *mut ffi::wlr_pointer_swipe_begin_event;

    let cursor = cursor_mut!(wm, sid, ());
    let swipe_enabled = crate::shared::layout().input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.swipe).unwrap_or(true)
        || cursor.gesture_from_touch;
    if !swipe_enabled {
        return;
    }
    crate::seat::Seat::handle_activity(wm, sid, sys);
    // A swipe or pinch is deliberate input, like a click or a key: it ends
    // the session-restore settling phase (`WindowManager::startup_input_seen`),
    // so the first focus of a restored window pans to it. Until 2026-09-26
    // only buttons and keys counted, and after a login navigated purely by
    // three-finger swipes, each restored window's first focus left it
    // wherever it sat, often half off screen. Holds do not count: one begins
    // whenever fingers merely rest on the pad.
    wm.startup_input_seen = true;

    let cursor = cursor_mut!(wm, sid, ());
    cursor.gesture_dx = 0.0;
    cursor.gesture_dy = 0.0;
    cursor.gesture_triggered = false;
    cursor.swipe_spent = false;
    cursor.swipe_dead_end = None;
    cursor.swipe_peek = [0.0, 0.0];

    log::info!("handle_swipe_begin: fingers={}", (*event).fingers);

    let pointer_gestures = crate::shared::handles().pointer_gestures.ptr();
    if !pointer_gestures.is_null() && !cursor.gesture_from_touch {
        let seat = seat_mut!(wm, sid, ());
        ffi::wlr_pointer_gestures_v1_send_swipe_begin(
            pointer_gestures,
            seat.wlr_seat.ptr(),
            (*event).time_msec,
            (*event).fingers,
        );
    }
}

pub(crate) unsafe extern "C" fn handle_swipe_update(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let sid = (*crate::container_of!(listener, Cursor, swipe_update_listener)).sid;
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    on_swipe_update(wm, sid, data, sys);
}

/// `handle_swipe_update` with the window manager passed in; `ccectl`'s injection calls this from inside `process_ipc_command`, with its `self`.
pub(crate) unsafe fn on_swipe_update(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, data: *mut std::ffi::c_void, sys: &mut crate::ctx::Sys) {
    let Some(sptr) = wm.seats.ptr(sid) else { return () };
    let cptr: *mut Cursor = std::ptr::addr_of_mut!((*sptr).cursor);
    let _ = cptr;
    let event = data as *mut ffi::wlr_pointer_swipe_update_event;

    let cursor = cursor_mut!(wm, sid, ());
    let swipe_enabled = crate::shared::layout().input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.swipe).unwrap_or(true)
        || cursor.gesture_from_touch;
    if !swipe_enabled {
        return;
    }
    crate::seat::Seat::handle_activity(wm, sid, sys);

    // A step does not end the gesture: the travel restarts from zero at
    // the fire (below), so the fingers can keep going and step focus
    // again — or turn round and step back — without lifting.
    // `gesture_triggered` only records that clients were sent their
    // (cancelled) end, so they hear nothing more of this swipe. Any other
    // bind (the overview toggle, a spawn) fires once per gesture: after
    // it the swipe is spent and the rest of it is ignored.
    let cursor = cursor_mut!(wm, sid, ());
    if cursor.swipe_spent {
        return;
    }

    cursor.gesture_dx += (*event).dx;
    cursor.gesture_dy += (*event).dy;

    log::debug!(
        "handle_swipe_update: fingers={}, dx={}, dy={}, accumulated_dx={}, accumulated_dy={}",
        (*event).fingers,
        (*event).dx,
        (*event).dy,
        cursor.gesture_dx,
        cursor.gesture_dy
    );

    let seat = seat_mut!(wm, sid, ());
    let wlr_keyboard = seat.wlr_seat.keyboard();
    let modifiers = if !wlr_keyboard.is_null() {
        ffi::wlr_keyboard_get_modifiers(wlr_keyboard) & 0x4d
    } else {
        0
    };

    // The first step of a swipe comes at `swipe_threshold`; every further
    // one at `swipe_repeat_threshold` (default four times that), so a swipe
    // that has just switched focus meets resistance before switching
    // again rather than running on through the next window. The lean
    // scales with the threshold in force, so it stays a preview of how
    // far the fingers are from the next step.
    let cursor = cursor_mut!(wm, sid, ());
    let threshold = if cursor.gesture_triggered {
        crate::shared::layout().swipe_repeat_threshold
    } else {
        crate::shared::layout().swipe_threshold
    };
    let mut matched_action = crate::config::Action::None;
    let mut matched_command = None;
    // Which directions this finger count + chord could still fire a
    // navigating bind in — the directions the camera may peek toward.
    let mut navigates = [false; 4]; // left, right, up, down

    for gb in &crate::shared::layout().gesture_binds {
        if gb.gesture_type == "swipe" && gb.fingers == (*event).fingers && gb.mods == modifiers {
            let (matched, slot) = match gb.direction.as_str() {
                "left" => (cursor.gesture_dx < -threshold, Some(0)),
                "right" => (cursor.gesture_dx > threshold, Some(1)),
                "up" => (cursor.gesture_dy < -threshold, Some(2)),
                "down" => (cursor.gesture_dy > threshold, Some(3)),
                _ => (false, None),
            };
            // First match wins in this table. The scan goes on past it
            // because a focus step that turns out to have nowhere to go
            // (below) leans instead, and the lean needs every direction.
            if matched && matched_action == crate::config::Action::None {
                matched_action = gb.action;
                matched_command = gb.command.clone();
            }
            if let Some(i) = slot {
                if !navigates[i] && action_navigates(gb.action) {
                    navigates[i] = true;
                }
            }
        }
    }

    // A focus step with no window that way does not fire. Firing it moved
    // nothing but the camera: the step took the lean, restarted the
    // travel, and the camera eased back while the fingers were still
    // going out, then leaned out again toward the next threshold and
    // snapped back at it — a sawtooth for as long as the swipe ran on.
    // Instead the lean holds at its limit, as at a wall, and the lift eases
    // it out (`handle_swipe_end`). The travel is scaled back onto the
    // threshold, keeping its direction, so turning the fingers round
    // unwinds the lean at once rather than first spending the overshoot.
    // The answer is kept for the rest of the swipe, so the policy is asked
    // once per dead end, not once per event.
    if is_directional_focus(matched_action) {
        let travel = (cursor.gesture_dx, cursor.gesture_dy);
        let dead = cursor.swipe_dead_end == Some(matched_action) || {
            let v = swipe_focus_vector(&crate::shared::layout().gesture_binds, (*event).fingers, modifiers, travel);
            !wm.focus_toward_lands(v, &matched_action)
        };
        if dead {
            let cursor = cursor_mut!(wm, sid, ());
            if cursor.swipe_dead_end != Some(matched_action) {
                log::info!("Swipe gesture {:?}: no window that way, holding the lean", matched_action);
            }
            cursor.swipe_dead_end = Some(matched_action);
            let over = travel.0.abs().max(travel.1.abs()) / threshold;
            if over > 1.0 {
                cursor.gesture_dx /= over;
                cursor.gesture_dy /= over;
            }
            matched_action = crate::config::Action::None;
            matched_command = None;
        }
    }

    if matched_action != crate::config::Action::None {
        log::info!("Swipe gesture matched action: {:?}", matched_action);
        let cursor = cursor_mut!(wm, sid, ());
        cursor.swipe_dead_end = None;
        // This step's travel, before it restarts below: a focus swipe
        // aims along it (`swipe_focus_vector`).
        let travel = (cursor.gesture_dx, cursor.gesture_dy);
        let first_fire = !cursor.gesture_triggered;
        if !action_navigates(matched_action) {
            cursor.swipe_spent = true;
        }
        cursor.gesture_triggered = true;
        // The next step needs a full threshold of fresh travel from here,
        // on both axes: a long swipe steps once per threshold, and a
        // reversal after a step goes back rather than first having to
        // undo the travel that got here.
        cursor.gesture_dx = 0.0;
        cursor.gesture_dy = 0.0;

        // The lean is where the camera IS now: the action runs against
        // it, and whatever it asks for is applied from here. The one rule
        // is that the camera never reverses at the fire — see below.
        let lean = std::mem::replace(&mut cursor.swipe_peek, [0.0, 0.0]);
        if lean != [0.0, 0.0] {
            wm.desk_pan_x += wm.pan_pending[0];
            wm.desk_pan_y += wm.pan_pending[1];
            wm.pan_pending = [0.0, 0.0];
        }

        if matched_action == crate::config::Action::Overview && crate::shared::mode() == crate::window_manager::WindowManagerMode::Overview {
            let lx = cursor.x();
            let ly = cursor.y();
            let mut hovered: Option<crate::window::WindowId> = None;
            if let Some(result) = crate::shared::scene().at(wm, lx, ly) {
                if let SceneNodeDataVal::Window(window_id) = result.data {
                    hovered = Some(window_id);
                }
            }
            let hovered = hovered.filter(|&id| wm.windows.get(id).is_some_and(|w| !w.is_status_bar() && !w.is_wallpaper()));
            if let Some(id) = hovered {
                crate::seat::Seat::focus(wm, sid, Focus::Window(id));
            }
        }

        // The overview toggle lands on the hovered window, else on the
        // FOCUSED one — never on the empty desktop under the pointer.
        let matched_action = if matched_action == crate::config::Action::Overview {
            wm.overview_action_for_gesture()
        } else {
            matched_action
        };
        // A focus swipe aims where the fingers went, not at one of four
        // directions: the bind table gives each axis its sense (a
        // `focus_left` on `swipe3_left` follows the fingers, one on
        // `swipe3_right` mirrors them), and the window manager picks the
        // nearest window center along the result.
        let focus_vector = swipe_focus_vector(&crate::shared::layout().gesture_binds, (*event).fingers, modifiers, travel);
        match focus_vector {
            Some(v) if is_directional_focus(matched_action) => wm.focus_toward(v, &matched_action),
            _ => wm.execute_action(&matched_action, matched_command.as_deref()),
        }

        // The action ran against the leaned camera. It set a pan target
        // only if the window it focused crosses a screen edge from there,
        // and only as far as bringing it in needs (`pan_into_view`), so a
        // window the lean left fully in view asks for nothing and the
        // camera stops right here rather than springing back. A target
        // that heads back against the lean is kept: it means the lean
        // pushed the window's near edge off screen, or leaned away from
        // the side the window sits on, and dropping it (as this did until
        // 2026-09-24, to keep the camera from ever reversing) left the
        // newly focused window clipped.

        if first_fire {
            let pointer_gestures = crate::shared::handles().pointer_gestures.ptr();
            let cursor = cursor_mut!(wm, sid, ());
            if !pointer_gestures.is_null() && !cursor.gesture_from_touch {
                let seat = seat_mut!(wm, sid, ());
                ffi::wlr_pointer_gestures_v1_send_swipe_end(
                    pointer_gestures,
                    seat.wlr_seat.ptr(),
                    (*event).time_msec,
                    true, // cancelled: true
                );
            }
        }
        return;
    }

    // Short of the threshold: lean the camera toward the bind the swipe
    // is heading for, 1:1 with the fingers like a two-finger pan (queued
    // for the frame, no easing), recomputed from the total travel so a
    // reversal leans back through zero. In the fingers' direction
    // (`swipe_lean`): a diagonal swipe leans diagonally, while a nearly
    // straight one stays on its dominant axis, because a hand swiping
    // left drifts a little up or down and leaning with that drift was a
    // wobble on top of the real move.
    {
        // After a step the lean is slower as well as longer to fill: it
        // reaches `swipe_repeat_peek` (default half of `swipe_peek`) at the
        // repeat threshold, so a swipe that has just switched focus does
        // not tug the camera toward the next window as eagerly. With
        // animations off (`cce_core::motion`) there is no lean at all: the
        // camera stays put until the bind fires, then jumps (the step's
        // ease is instant then, `advance_camera_animation`). A lean
        // already showing when they were turned off goes back the same way.
        let cursor = cursor_mut!(wm, sid, ());
        let peek_px = if !cce_core::motion::enabled() {
            0.0
        } else if cursor.gesture_triggered {
            crate::shared::layout().swipe_repeat_peek_px
        } else {
            crate::shared::layout().swipe_peek_px
        };
        let cursor = cursor_mut!(wm, sid, ());
        let (dx, dy) = (cursor.gesture_dx, cursor.gesture_dy);
        let want = swipe_lean(dx, dy, navigates, threshold, peek_px, wm.desk_zoom);
        let delta = [want[0] - cursor.swipe_peek[0], want[1] - cursor.swipe_peek[1]];
        if delta != [0.0, 0.0] {
            if cursor.gesture_triggered {
                // After a step the camera may still be easing the window
                // it focused into view. The lean rides on top of that ease
                // rather than freezing it short: the ease's target moves
                // with the fingers, and the lift (`handle_swipe_end`)
                // moves it back.
                if let Some(t) = wm.target_desk_pan_x.as_mut() { *t += delta[0]; }
                if let Some(t) = wm.target_desk_pan_y.as_mut() { *t += delta[1]; }
            } else {
                wm.stop_panning_animation();
            }
            wm.queue_pan(delta[0], delta[1]);
            let cursor = cursor_mut!(wm, sid, ());
            cursor.swipe_peek = want;
        }
    }

    let cursor = cursor_mut!(wm, sid, ());
    if cursor.gesture_triggered {
        return;
    }
    let pointer_gestures = crate::shared::handles().pointer_gestures.ptr();
    if !pointer_gestures.is_null() && !cursor.gesture_from_touch {
        let seat = seat_mut!(wm, sid, ());
        ffi::wlr_pointer_gestures_v1_send_swipe_update(
            pointer_gestures,
            seat.wlr_seat.ptr(),
            (*event).time_msec,
            (*event).dx,
            (*event).dy,
        );
    }
}

pub(crate) unsafe extern "C" fn handle_swipe_end(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let sid = (*crate::container_of!(listener, Cursor, swipe_end_listener)).sid;
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    on_swipe_end(wm, sid, data, sys);
}

/// `handle_swipe_end` with the window manager passed in; `ccectl`'s injection calls this from inside `process_ipc_command`, with its `self`.
pub(crate) unsafe fn on_swipe_end(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, data: *mut std::ffi::c_void, sys: &mut crate::ctx::Sys) {
    let Some(sptr) = wm.seats.ptr(sid) else { return () };
    let cptr: *mut Cursor = std::ptr::addr_of_mut!((*sptr).cursor);
    let _ = cptr;
    let event = data as *mut ffi::wlr_pointer_swipe_end_event;

    let cursor = cursor_mut!(wm, sid, ());
    let swipe_enabled = crate::shared::layout().input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.swipe).unwrap_or(true)
        || cursor.gesture_from_touch;
    if !swipe_enabled {
        return;
    }
    crate::seat::Seat::handle_activity(wm, sid, sys);

    log::info!("handle_swipe_end: cancelled={}", (*event).cancelled);

    let cursor = cursor_mut!(wm, sid, ());
    let fired = std::mem::replace(&mut cursor.gesture_triggered, false);

    // Lifted short of a threshold — the first, or the next one after a
    // step — ease the lean back out: to where the swipe found the camera,
    // or, after a step, to where the step's own ease was heading before
    // the lean moved its target along (`handle_swipe_update`). The steps
    // themselves stay: the camera never returns to where the swipe began.
    let peek = std::mem::replace(&mut cursor.swipe_peek, [0.0, 0.0]);
    if peek != [0.0, 0.0] {
        wm.desk_pan_x += wm.pan_pending[0];
        wm.desk_pan_y += wm.pan_pending[1];
        wm.pan_pending = [0.0, 0.0];
        wm.target_desk_pan_x = Some(wm.target_desk_pan_x.unwrap_or(wm.desk_pan_x) - peek[0]);
        wm.target_desk_pan_y = Some(wm.target_desk_pan_y.unwrap_or(wm.desk_pan_y) - peek[1]);
        wm.start_panning_animation();
    }

    // Clients heard a cancelled end at the first step; the lift after one
    // is not theirs.
    if fired {
        return;
    }

    let pointer_gestures = crate::shared::handles().pointer_gestures.ptr();
    let cursor = cursor_mut!(wm, sid, ());
    if !pointer_gestures.is_null() && !cursor.gesture_from_touch {
        let seat = seat_mut!(wm, sid, ());
        ffi::wlr_pointer_gestures_v1_send_swipe_end(
            pointer_gestures,
            seat.wlr_seat.ptr(),
            (*event).time_msec,
            (*event).cancelled,
        );
    }
}

/// The modifiers a gesture bind is matched under: the keyboard's, less
/// Caps and Num Lock (`& 0x4d`), as for every gesture.
pub(crate) fn gesture_mods(wlr_seat: &crate::wlr::Wlr<ffi::wlr_seat>) -> u32 {
    wlr_seat.keyboard_modifiers() & 0x4d
}

/// The first gesture bind of `kind` ("swipe", "pinch", "edge") for this
/// finger count and modifiers whose direction `hit` accepts. First match
/// wins, as everywhere in the bind table.
pub(crate) fn gesture_bind(
    kind: &str,
    fingers: u32,
    mods: u32,
    hit: impl Fn(&str) -> bool,
) -> Option<(crate::config::Action, Option<String>)> {
    crate::shared::layout().gesture_binds
        .iter()
        .find(|gb| gb.gesture_type == kind && gb.fingers == fingers && gb.mods == mods && hit(&gb.direction))
        .map(|gb| (gb.action, gb.command.clone()))
}

/// Whether a pinch at `scale` (the fingers' spread relative to the start)
/// has gone far enough to fire a bind on `direction`.
pub(crate) fn pinch_hits(direction: &str, scale: f64) -> bool {
    match direction {
        "in" => scale < 0.7,
        "out" => scale > 1.3,
        _ => false,
    }
}

/// Run a gesture's bound action. The overview toggle lands on the hovered
/// window, else on the FOCUSED one — never on the empty desktop under the
/// pointer.
pub(crate) unsafe fn run_gesture_action(
    wm: &mut crate::window_manager::WindowManager,
    action: crate::config::Action,
    command: Option<&str>,
) {
    let action = if action == crate::config::Action::Overview {
        wm.overview_action_for_gesture()
    } else {
        action
    };
    wm.execute_action(&action, command);
}

pub(crate) unsafe extern "C" fn handle_pinch_begin(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let sid = (*crate::container_of!(listener, Cursor, pinch_begin_listener)).sid;
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    on_pinch_begin(wm, sid, data, sys);
}

/// `handle_pinch_begin` with the window manager passed in; `ccectl`'s injection calls this from inside `process_ipc_command`, with its `self`.
pub(crate) unsafe fn on_pinch_begin(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, data: *mut std::ffi::c_void, sys: &mut crate::ctx::Sys) {
    let Some(sptr) = wm.seats.ptr(sid) else { return () };
    let cptr: *mut Cursor = std::ptr::addr_of_mut!((*sptr).cursor);
    let _ = cptr;
    let event = data as *mut ffi::wlr_pointer_pinch_begin_event;

    let pinch_enabled = crate::shared::layout().input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.pinch).unwrap_or(true);
    if !pinch_enabled {
        return;
    }
    crate::seat::Seat::handle_activity(wm, sid, sys);
    // A swipe or pinch is deliberate input, like a click or a key: it ends
    // the session-restore settling phase (`WindowManager::startup_input_seen`),
    // so the first focus of a restored window pans to it. Until 2026-09-26
    // only buttons and keys counted, and after a login navigated purely by
    // three-finger swipes, each restored window's first focus left it
    // wherever it sat, often half off screen. Holds do not count: one begins
    // whenever fingers merely rest on the pad.
    wm.startup_input_seen = true;

    let cursor = cursor_mut!(wm, sid, ());
    cursor.gesture_scale = 1.0;
    cursor.gesture_triggered = false;


    // A pinch starting over the desktop background (wallpaper or bare
    // desktop, same test as the right-click context menu) zooms the camera
    // for the whole gesture. Clients never see a begin, so update/end stay
    // ours too.
    let mut on_background = true;
    let cursor = cursor_mut!(wm, sid, ());
    let (cx, cy) = (cursor.x(), cursor.y());
    if let Some(result) = crate::shared::scene().at(wm, cx, cy) {
        match result.data {
            SceneNodeDataVal::Window(window_id) => {
                if let Some(window) = wm.windows.get(window_id) {
                    if !window.is_wallpaper() {
                        on_background = false;
                    }
                }
            }
            SceneNodeDataVal::LayerSurface(_) | SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                on_background = false;
            }
        }
    }
    if on_background {
        wm.stop_panning_animation();
        let cursor = cursor_mut!(wm, sid, ());
        cursor.pinch_zoom_active = true;
        cursor.pinch_start_zoom = wm.desk_zoom;
        return;
    }

    // A pinch over an app in `touchpad_view_apps` becomes a dolly drag.
    if crate::cursor::Cursor::view_drag_pinch_begin(wm, sid) {
        return;
    }

    let pointer_gestures = crate::shared::handles().pointer_gestures.ptr();
    if !pointer_gestures.is_null() {
        let seat = seat_mut!(wm, sid, ());
        ffi::wlr_pointer_gestures_v1_send_pinch_begin(
            pointer_gestures,
            seat.wlr_seat.ptr(),
            (*event).time_msec,
            (*event).fingers,
        );
    }
}

pub(crate) unsafe extern "C" fn handle_pinch_update(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let sid = (*crate::container_of!(listener, Cursor, pinch_update_listener)).sid;
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    on_pinch_update(wm, sid, data, sys);
}

/// `handle_pinch_update` with the window manager passed in; `ccectl`'s injection calls this from inside `process_ipc_command`, with its `self`.
pub(crate) unsafe fn on_pinch_update(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, data: *mut std::ffi::c_void, sys: &mut crate::ctx::Sys) {
    let Some(sptr) = wm.seats.ptr(sid) else { return () };
    let cptr: *mut Cursor = std::ptr::addr_of_mut!((*sptr).cursor);
    let _ = cptr;
    let event = data as *mut ffi::wlr_pointer_pinch_update_event;

    let pinch_enabled = crate::shared::layout().input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.pinch).unwrap_or(true);
    if !pinch_enabled {
        return;
    }
    crate::seat::Seat::handle_activity(wm, sid, sys);

    let cursor = cursor_mut!(wm, sid, ());
    if cursor.view_drag_pinch_update((*event).scale) {
        return;
    }

    if cursor.pinch_zoom_active {
        let new_zoom = crate::policy::camera::pinch_zoom(cursor.pinch_start_zoom, (*event).scale);
        let cx = cursor.x();
        let cy = cursor.y();
        let wlr_output = crate::output_manager::output_at(cx, cy);
        let (phys_x, phys_y) = if !wlr_output.is_null() {
            let mut output_box = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
            ffi::wlr_output_layout_get_box(crate::shared::handles().output_layout.ptr(), wlr_output, &mut output_box);
            (output_box.x as f64, output_box.y as f64)
        } else {
            (0.0, 0.0)
        };
        // Applied on the next output frame, like finger pans: libinput
        // delivers pinch updates faster than the refresh rate, and stepping
        // the camera per event relaid out the desktop for frames nobody
        // saw and zoomed unevenly (two steps in one frame, one in the next).
        wm.queue_pinch(new_zoom, cx - phys_x, cy - phys_y);
        return;
    }

    if cursor.gesture_triggered {
        return;
    }

    cursor.gesture_scale = (*event).scale;

    let seat = seat_mut!(wm, sid, ());
    let modifiers = gesture_mods(&seat.wlr_seat);
    let cursor = cursor_mut!(wm, sid, ());
    let scale = cursor.gesture_scale;
    let bind = gesture_bind("pinch", (*event).fingers, modifiers, |d| pinch_hits(d, scale));

    if let Some((matched_action, matched_command)) = bind {
        cursor.gesture_triggered = true;
        run_gesture_action(wm, matched_action, matched_command.as_deref());

        let pointer_gestures = crate::shared::handles().pointer_gestures.ptr();
        if !pointer_gestures.is_null() {
            let seat = seat_mut!(wm, sid, ());
            ffi::wlr_pointer_gestures_v1_send_pinch_end(
                pointer_gestures,
                seat.wlr_seat.ptr(),
                (*event).time_msec,
                true, // cancelled: true
            );
        }
        return;
    }

    let pointer_gestures = crate::shared::handles().pointer_gestures.ptr();
    if !pointer_gestures.is_null() {
        let seat = seat_mut!(wm, sid, ());
        ffi::wlr_pointer_gestures_v1_send_pinch_update(
            pointer_gestures,
            seat.wlr_seat.ptr(),
            (*event).time_msec,
            (*event).dx,
            (*event).dy,
            (*event).scale,
            (*event).rotation,
        );
    }
}

pub(crate) unsafe extern "C" fn handle_pinch_end(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let sid = (*crate::container_of!(listener, Cursor, pinch_end_listener)).sid;
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    on_pinch_end(wm, sid, data, sys);
}

/// `handle_pinch_end` with the window manager passed in; `ccectl`'s injection calls this from inside `process_ipc_command`, with its `self`.
pub(crate) unsafe fn on_pinch_end(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, data: *mut std::ffi::c_void, sys: &mut crate::ctx::Sys) {
    let Some(sptr) = wm.seats.ptr(sid) else { return () };
    let cptr: *mut Cursor = std::ptr::addr_of_mut!((*sptr).cursor);
    let _ = cptr;
    let event = data as *mut ffi::wlr_pointer_pinch_end_event;

    let pinch_enabled = crate::shared::layout().input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.pinch).unwrap_or(true);
    if !pinch_enabled {
        return;
    }
    crate::seat::Seat::handle_activity(wm, sid, sys);

    if crate::cursor::Cursor::view_drag_pinch_end(wm, sid) {
        return;
    }

    let cursor = cursor_mut!(wm, sid, ());
    if cursor.pinch_zoom_active {
        // Camera zoom consumed the whole gesture; clients got no begin, so
        // they get no end. The camera simply stays where the fingers left it.
        cursor.pinch_zoom_active = false;
        return;
    }

    if cursor.gesture_triggered {
        cursor.gesture_triggered = false;
        return;
    }

    let pointer_gestures = crate::shared::handles().pointer_gestures.ptr();
    if !pointer_gestures.is_null() {
        let seat = seat_mut!(wm, sid, ());
        ffi::wlr_pointer_gestures_v1_send_pinch_end(
            pointer_gestures,
            seat.wlr_seat.ptr(),
            (*event).time_msec,
            (*event).cancelled,
        );
    }
}

pub(crate) unsafe extern "C" fn handle_hold_begin(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let cursor = crate::container_of!(listener, Cursor, hold_begin_listener);
    let (sid, wlr_seat) = ((*cursor).sid, (*cursor).wlr_seat.ptr());
    let event = data as *mut ffi::wlr_pointer_hold_begin_event;

    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    crate::seat::Seat::handle_activity(wm, sid, sys);

    let pointer_gestures = crate::shared::handles().pointer_gestures.ptr();
    if !pointer_gestures.is_null() {
        ffi::wlr_pointer_gestures_v1_send_hold_begin(
            pointer_gestures,
            wlr_seat,
            (*event).time_msec,
            (*event).fingers,
        );
    }
}

pub(crate) unsafe extern "C" fn handle_hold_end(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let cursor = crate::container_of!(listener, Cursor, hold_end_listener);
    let (sid, wlr_seat) = ((*cursor).sid, (*cursor).wlr_seat.ptr());
    let event = data as *mut ffi::wlr_pointer_hold_end_event;

    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    crate::seat::Seat::handle_activity(wm, sid, sys);

    let pointer_gestures = crate::shared::handles().pointer_gestures.ptr();
    if !pointer_gestures.is_null() {
        ffi::wlr_pointer_gestures_v1_send_hold_end(
            pointer_gestures,
            wlr_seat,
            (*event).time_msec,
            (*event).cancelled,
        );
    }
}
