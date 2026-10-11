//! Scroll-wheel and touchpad axis events (`handle_axis`) and the three states that
//! take them over: the view drag (Space + drag), the popup wheel (Ctrl over a
//! popup) and the horizontal-scroll shift. Split out of cursor.rs on 2026-10-10.

use super::*;

pub(crate) unsafe extern "C" fn handle_axis(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, axis_listener);
    let event = data as *mut ffi::wlr_pointer_axis_event;
    (*cursor.seat).handle_activity();
    
    let seat = &mut *cursor.seat;
    
    let mut delta = (*event).delta;
    let mut delta_discrete = (*event).delta_discrete;

    if !(*event).pointer.is_null() {
        let wlr_device = &mut (*(*event).pointer).base as *mut ffi::wlr_input_device;
        let device_ptr = ffi::river_wlr_input_device_get_data(wlr_device) as *mut crate::input_device::InputDevice;
        if !device_ptr.is_null() {
            let factor = (*device_ptr).config.scroll_factor;
            delta *= factor;
            delta_discrete = (delta_discrete as f64 * factor) as i32;
        }
    }

    let wlr_keyboard = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
    // Modifiers held through `ccectl key-down` count too, so a headless
    // session can exercise the modifier branches below.
    let modifiers = if !wlr_keyboard.is_null() {
        ffi::wlr_keyboard_get_modifiers(wlr_keyboard)
    } else {
        0
    } | (*crate::reentry::wm(seat.server)).injected_key_mods;

    if (modifiers & 0x44) == 0x44 {
        if (*event).orientation == ffi::wl_pointer_axis_WL_POINTER_AXIS_VERTICAL_SCROLL {
            if delta != 0.0 {
                let wm = &mut (*crate::reentry::wm(seat.server));
                // Each notch advances the zoom TARGET (successive notches
                // accumulate into one glide); the animation tick eases the
                // zoom there in log space, pivoting about the cursor every
                // step. A pan glide or coast in flight yields to the zoom.
                let base = wm.target_desk_zoom.unwrap_or(wm.desk_zoom);
                let new_zoom = crate::policy::camera::wheel_zoom(base, delta);
                if new_zoom != base {
                    let cx = cursor.x();
                    let cy = cursor.y();
                    let wlr_output = (*(*seat).server).om.output_at(cx, cy);
                    let (phys_x, phys_y) = if !wlr_output.is_null() {
                        let mut output_box = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
                        ffi::wlr_output_layout_get_box((*(*seat).server).om.output_layout, wlr_output, &mut output_box);
                        (output_box.x as f64, output_box.y as f64)
                    } else {
                        (0.0, 0.0)
                    };
                    wm.stop_panning_animation();
                    wm.target_desk_zoom = Some(new_zoom);
                    wm.zoom_anchor = Some((cx - phys_x, cy - phys_y));
                    wm.set_mode(if crate::policy::camera::is_overview(new_zoom) { crate::window_manager::WindowManagerMode::Overview } else { crate::window_manager::WindowManagerMode::Normal });
                    wm.start_panning_animation();
                }
            }
        }
        return;
    }

    let (is_on_background, over_chrome) = {
        let lx = cursor.x();
        let ly = cursor.y();
        let mut over_interactive = false;
        // Chrome under the pointer — a Popup (the cce-cloud launcher) or an
        // Overlay dock, or a cce-cloud layer surface (context menu). Live UI
        // during overview, same as in the button and motion paths: a wheel
        // over the launcher's list scrolls the list, not the desktop.
        let mut over_chrome = false;
        if let Some(result) = crate::shared::scene().at(lx, ly) {
            match result.data {
                SceneNodeDataVal::Window(window) => {
                    over_interactive = true;
                    over_chrome = !window.is_null()
                        && matches!(
                            (*window).tiling_mode,
                            crate::tiling::TilingMode::Popup | crate::tiling::TilingMode::Overlay
                        );
                }
                SceneNodeDataVal::LayerSurface(layer_surface) => {
                    over_interactive = true;
                    over_chrome = is_cloud_layer(layer_surface);
                }
                SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                    over_interactive = true;
                }
            }
        }
        (!over_interactive, over_chrome)
    };
    // Overview pans on any scroll — except over chrome, which takes the
    // event itself.
    let is_overview = (*crate::reentry::wm((*seat).server)).mode == crate::window_manager::WindowManagerMode::Overview
        && !over_chrome;

    let is_finger = (*event).source == ffi::wl_pointer_axis_source_WL_POINTER_AXIS_SOURCE_FINGER
        || (*event).source == ffi::wl_pointer_axis_source_WL_POINTER_AXIS_SOURCE_CONTINUOUS;

    let mut was_panning = cursor.panning_gesture_active;

    if is_finger {
        if delta == 0.0 {
            cursor.panning_gesture_active = false;
        } else if !cursor.panning_gesture_active && (is_on_background || is_overview) {
            cursor.panning_gesture_active = true;
            was_panning = true;
        }
    }

    if (modifiers & 0x40) != 0 || is_on_background || is_overview || was_panning {
        let wm = &mut (*crate::reentry::wm(seat.server));
        let step = delta / wm.desk_zoom;
        let vertical =
            (*event).orientation == ffi::wl_pointer_axis_WL_POINTER_AXIS_VERTICAL_SCROLL;
        if is_finger {
            let axis = if vertical { 1 } else { 0 };
            let now_ms = (*event).time_msec;
            if delta != 0.0 {
                // Finger/continuous scroll tracks 1:1 — the surface follows
                // the gesture directly, no easing between the two — while a
                // velocity estimate is kept for the coast on the lift.
                wm.stop_panning_animation();
                if vertical {
                    wm.queue_pan(0.0, step);
                } else {
                    wm.queue_pan(step, 0.0);
                }
                let dt_ms = now_ms.wrapping_sub(cursor.pan_last_msec[axis]).clamp(4, 100) as f64;
                let sample = step / (dt_ms / 1000.0);
                cursor.pan_vel[axis] = if cursor.pan_last_msec[axis] == 0 {
                    sample
                } else {
                    cursor.pan_vel[axis] * 0.65 + sample * 0.35
                };
                cursor.pan_last_msec[axis] = now_ms;
                wm.pan_finger_v[axis] = cursor.pan_vel[axis];
            } else if was_panning {
                // The lift (libinput's zero-delta finger event): fling on the
                // estimated velocity unless the finger had come to rest first
                // or kinetic scrolling is off. Both axes launch together on
                // the first lift event; the second axis's lift finds them
                // already cleared.
                let mut vx = cursor.pan_vel[0];
                let mut vy = cursor.pan_vel[1];
                for (a, v) in [(0usize, &mut vx), (1usize, &mut vy)] {
                    let rest_ms = now_ms.wrapping_sub(cursor.pan_last_msec[a]);
                    if cursor.pan_last_msec[a] == 0 || rest_ms > 80 {
                        *v = 0.0;
                    }
                }
                cursor.pan_vel = [0.0, 0.0];
                cursor.pan_last_msec = [0, 0];
                wm.pan_finger_v = [0.0, 0.0];
                if wm.kinetic_scroll() && (vx != 0.0 || vy != 0.0) {
                    wm.pan_coast_vx = vx;
                    wm.pan_coast_vy = vy;
                    wm.start_panning_animation();
                }
            }
        } else {
            // Discrete wheel clicks glide: each click advances the pan
            // animation target, so successive clicks accumulate into one
            // smooth run instead of a stutter of jumps. A coast in flight
            // yields to the click.
            wm.pan_coast_vx = 0.0;
            wm.pan_coast_vy = 0.0;
            if vertical {
                let base = wm.target_desk_pan_y.unwrap_or(wm.desk_pan_y);
                wm.target_desk_pan_y = Some(base + step);
            } else {
                let base = wm.target_desk_pan_x.unwrap_or(wm.desk_pan_x);
                wm.target_desk_pan_x = Some(base + step);
            }
            wm.start_panning_animation();
        }
        return;
    }

    // A two-finger scroll over an app in `touchpad_view_apps` becomes a
    // view drag instead of a scroll (see `ViewDrag`), and one over that
    // app's own popup becomes a wheel (see `PopupWheel`).
    if is_finger && cursor.view_drag_axis(event, delta, modifiers) {
        return;
    }
    if is_finger && cursor.popup_wheel_axis(event, delta, modifiers) {
        return;
    }
    // A horizontal one over an app in `touchpad_hscroll_shift_apps` is
    // delivered as Shift + vertical (see `HScrollShift`).
    if is_finger && cursor.hscroll_shift_axis(event, delta, delta_discrete, modifiers) {
        return;
    }

    ffi::wlr_seat_pointer_notify_axis(
        seat.wlr_seat,
        (*event).time_msec,
        (*event).orientation,
        delta,
        delta_discrete,
        (*event).source,
        (*event).relative_direction,
    );
}

/// An emulated view drag: trackpad input over a window turned into what
/// a 3D app's view tool understands, a held key plus a button drag.
///
/// Why this exists: Houdini's own trackpad gestures cannot work under X11.
/// Xwayland attributes every scroll to a device Qt classifies as a
/// TouchPad, Houdini's touchpad "slide" then moves the view by the wheel
/// event's pixel deltas, and Qt's X11 backend never fills those in (it
/// does so only for a scroll increment above 15; Xwayland's is 1, the
/// libinput X driver's 15). Verified against Houdini 22 headless: the slide
/// is a no-op and the mouse wheel is swallowed with it, while Space + a
/// button drag tumbles, pans and dollies. So for apps listed in
/// `window_manager.touchpad_view_apps` the compositor synthesises exactly
/// that: Space down, button down, the finger deltas as pointer motion on
/// the surface (the on-screen cursor never moves), button and Space up
/// when the fingers lift. Two-finger swipe pans (middle button) or tumbles
/// (left) per `touchpad_view_swipe`, Shift picks the other, a pinch
/// dollies (right button, distance from the log of the scale), and Ctrl
/// + swipe passes through as a plain scroll — the same modifier Houdini
/// itself assigns to "simulate the mouse wheel" in gesture mode.
///
/// Natural scrolling is undone here. libinput flips the sign of a finger
/// delta before the compositor sees it, which is right for a scroll (the
/// content follows the fingers) and wrong for a drag replayed as pointer
/// motion: the view follows the pointer, so the pointer has to go where
/// the fingers went. `axis_event_is_natural` asks the source device, and
/// `touchpad_view_invert` then means "backwards" on top of that either way.
pub struct ViewDrag {
    pub button: u32,
    pub surface: *mut ffi::wlr_surface,
    pub window: *mut crate::window::Window,
    /// Synthetic pointer position, surface-local.
    pub sx: f64,
    pub sy: f64,
    pub origin_sx: f64,
    pub origin_sy: f64,
    /// Surface units per layout pixel (X11 HiDPI buffers, overview zoom).
    pub ratio: f64,
    pub from_pinch: bool,
    /// The button goes down on the first motion, one event after Space:
    /// Houdini feeds Qt input to its UI thread through a generator thread,
    /// and a button that arrives in the same instant as Space can be
    /// interpreted before the key — a right button then reads as a pan,
    /// not a dolly.
    pub button_down: bool,
    /// The keyboard's repeat settings before the drag, restored at its end.
    /// While Space is held synthetically the seat's repeat is switched off:
    /// Xwayland autorepeats a held key as release/press pairs, and Houdini
    /// left view mode on the first release, mid-drag.
    pub repeat: Option<(i32, i32)>,
}

pub(crate) const KEY_SPACE: u32 = 57;
pub(crate) const BTN_LEFT: u32 = 0x110;
pub(crate) const BTN_RIGHT: u32 = 0x111;
pub(crate) const BTN_MIDDLE: u32 = 0x112;
/// Safety net that ends a swipe drag when the lift event never came.
/// Only that: libinput posts nothing at all while the fingers rest on the
/// pad mid-gesture — a captured Houdini swipe paused 3.3 s between two
/// halves of one scroll, the zero-delta lift arriving only at the true end
/// — so a short timeout tears the drag down inside the gesture, and the
/// Space release/re-press a resume then costs can drop Houdini out of view
/// mode for the rest of the swipe (the ordering hazard `button_down`
/// documents, now mid-swipe). The lift ends a drag; so, at once, do real
/// pointer motion, a button and a key press.
pub(crate) const VIEW_DRAG_IDLE_MS: i32 = 5000;
/// Drag distance (layout px) per e-fold of pinch scale. Measured against
/// Houdini 22 (depth of the world origin in view space, which is what a
/// dolly changes — Houdini dollies toward the point under the pointer, so
/// distances to a fixed pivot mislead): Space+RMB dollies on the VERTICAL
/// drag, up is in, and 45 px up shortened the depth by a factor of 1.34,
/// about 150 px per e-fold. So a pinch of scale s becomes an upward drag
/// of ln(s) e-folds and the depth ends near 1/s.
pub(crate) const VIEW_DRAG_PINCH_PX: f64 = 150.0;

impl Cursor {
    /// The window under the pointer, if `touchpad_view_apps` names its app.
    unsafe fn view_drag_target(&mut self) -> Option<(*mut crate::window::Window, *mut ffi::wlr_surface, f64, f64, f64)> {
        if crate::shared::layout().touchpad_view_apps.is_empty() {
            return None;
        }
        let result = crate::shared::scene().at(self.x(), self.y())?;
        let SceneNodeDataVal::Window(window) = result.data else { return None };
        if window.is_null() || result.surface.is_null() || result.node.is_null() {
            return None;
        }
        let app_id = (*window).get_app_id_string().unwrap_or_default();
        if !crate::shared::layout().touchpad_view_apps.iter().any(|p| crate::window_manager::app_id_matches(p, &app_id)) {
            return None;
        }
        // The app may have narrowed the drag to its own view panes (see
        // `touchpad-view-regions`); elsewhere the scroll passes through.
        // `result.sx`/`sy` are surface-local, the same pixels the client
        // measures its panes in.
        if let Some(regions) = &(*window).view_regions {
            if !crate::window_manager::point_in_view_regions(regions, result.sx, result.sy) {
                return None;
            }
        }
        let mut ratio = 1.0;
        let dest_w = ffi::river_scene_buffer_get_dest_width(result.node as *mut ffi::wlr_scene_buffer);
        let surf_w = ffi::river_wlr_surface_get_width(result.surface);
        if dest_w > 0 && surf_w > 0 {
            ratio = surf_w as f64 / dest_w as f64;
        }
        Some((window, result.surface, result.sx, result.sy, ratio))
    }

    /// Whether the touchpad behind an axis event has natural scrolling on,
    /// i.e. its deltas arrive sign-flipped. An injected event has no device
    /// and answers with `inject_natural` (see `pointer-scroll ... natural`).
    unsafe fn axis_event_is_natural(&self, event: *const ffi::wlr_pointer_axis_event) -> bool {
        let pointer = (*event).pointer;
        if pointer.is_null() {
            return self.inject_natural;
        }
        let dev = &mut (*pointer).base as *mut ffi::wlr_input_device;
        if !ffi::wlr_input_device_is_libinput(dev) {
            return false;
        }
        let handle = ffi::wlr_libinput_get_device_handle(dev);
        !handle.is_null()
            && ffi::libinput_device_config_scroll_has_natural_scroll(handle) != 0
            && ffi::libinput_device_config_scroll_get_natural_scroll_enabled(handle) != 0
    }

    unsafe fn begin_view_drag(&mut self, button: u32, from_pinch: bool) -> bool {
        let Some((window, surface, sx, sy, ratio)) = self.view_drag_target() else { return false };
        let seat = &mut *self.seat;
        // Space must reach the window: give it keyboard focus as a click would.
        if seat.focused != crate::seat::Focus::Window(window) {
            seat.focus(crate::seat::Focus::Window(window));
        }
        seat.ensure_synthetic_keyboard();
        let time = crate::util::msec_timestamp();
        let mut repeat = None;
        let kbd = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
        if !kbd.is_null() {
            repeat = Some(((*kbd).repeat_info.rate, (*kbd).repeat_info.delay));
            ffi::wlr_keyboard_set_repeat_info(kbd, 0, 0);
        }
        ffi::wlr_seat_pointer_notify_enter(seat.wlr_seat, surface, sx, sy);
        ffi::wlr_seat_keyboard_notify_key(seat.wlr_seat, time, KEY_SPACE, ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED);
        ffi::wlr_seat_pointer_notify_frame(seat.wlr_seat);
        log::info!("[ViewDrag] begin button={:#x} from_pinch={} at surface ({:.0}, {:.0}) ratio={}", button, from_pinch, sx, sy, ratio);
        self.view_drag = Some(ViewDrag { button, surface, window, sx, sy, origin_sx: sx, origin_sy: sy, ratio, from_pinch, button_down: false, repeat });
        self.arm_view_drag_timer();
        true
    }

    unsafe fn arm_view_drag_timer(&mut self) {
        if !self.view_drag_timer.is_null() {
            ffi::wl_event_source_timer_update(self.view_drag_timer, VIEW_DRAG_IDLE_MS);
        }
    }

    /// Move the synthetic pointer by layout pixels.
    unsafe fn move_view_drag(&mut self, dx: f64, dy: f64) {
        let Some(d) = self.view_drag.as_mut() else { return };
        let seat = &mut *self.seat;
        let time = crate::util::msec_timestamp();
        if !d.button_down {
            ffi::wlr_seat_pointer_notify_button(seat.wlr_seat, time, d.button, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED);
            ffi::wlr_seat_pointer_notify_frame(seat.wlr_seat);
            d.button_down = true;
        }
        d.sx += dx * d.ratio;
        d.sy += dy * d.ratio;
        let (sx, sy) = (d.sx, d.sy);
        ffi::wlr_seat_pointer_notify_motion(seat.wlr_seat, time, sx, sy);
        ffi::wlr_seat_pointer_notify_frame(seat.wlr_seat);
        self.arm_view_drag_timer();
    }

    /// `reason` names what ended it — "lift", "idle", "motion", "button",
    /// "key", "ctrl" or "pinch" — so a stall reported later is diagnosable
    /// from the session log alone.
    pub unsafe fn end_view_drag(&mut self, reason: &str) {
        let Some(d) = self.view_drag.take() else { return };
        log::info!("[ViewDrag] end reason={} button={:#x} from_pinch={} at surface ({:.0}, {:.0})", reason, d.button, d.from_pinch, d.sx, d.sy);
        if !self.view_drag_timer.is_null() {
            ffi::wl_event_source_timer_update(self.view_drag_timer, 0);
        }
        let seat = &mut *self.seat;
        let time = crate::util::msec_timestamp();
        if d.button_down {
            ffi::wlr_seat_pointer_notify_button(seat.wlr_seat, time, d.button, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_RELEASED);
            ffi::wlr_seat_pointer_notify_frame(seat.wlr_seat);
        }
        ffi::wlr_seat_keyboard_notify_key(seat.wlr_seat, time, KEY_SPACE, ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED);
        if let Some((rate, delay)) = d.repeat {
            let kbd = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
            if !kbd.is_null() {
                ffi::wlr_keyboard_set_repeat_info(kbd, rate, delay);
            }
        }
        // Put the client's idea of the pointer back where the cursor is.
        self.passthrough(time);
        ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
    }

    /// A finger-source axis event over a `touchpad_view_apps` window.
    /// Returns true when it was consumed by the emulation.
    pub unsafe fn view_drag_axis(&mut self, event: *const ffi::wlr_pointer_axis_event, delta: f64, modifiers: u32) -> bool {
        const SHIFT: u32 = 0x1;
        const CTRL: u32 = 0x4;
        if matches!(&self.view_drag, Some(d) if d.from_pinch) {
            return false;
        }
        if delta == 0.0 {
            // The fingers lifted.
            if self.view_drag.is_some() {
                self.end_view_drag("lift");
                return true;
            }
            return false;
        }
        if modifiers & CTRL != 0 {
            // Houdini's own wheel modifier: a plain scroll.
            if self.view_drag.is_some() {
                self.end_view_drag("ctrl");
            }
            return false;
        }
        if self.view_drag.is_none() {
            let tumble = crate::shared::layout().touchpad_view_swipe_tumble != (modifiers & SHIFT != 0);
            let button = if tumble { BTN_LEFT } else { BTN_MIDDLE };
            if !self.begin_view_drag(button, false) {
                return false;
            }
        }
        let mut step = delta * crate::shared::layout().touchpad_view_sensitivity;
        if self.axis_event_is_natural(event) {
            step = -step;
        }
        if crate::shared::layout().touchpad_view_invert {
            step = -step;
        }
        if (*event).orientation == ffi::wl_pointer_axis_WL_POINTER_AXIS_VERTICAL_SCROLL {
            self.move_view_drag(0.0, step);
        } else {
            self.move_view_drag(step, 0.0);
        }
        true
    }

    pub unsafe fn view_drag_pinch_begin(&mut self) -> bool {
        if self.view_drag.is_some() {
            self.end_view_drag("pinch");
        }
        self.begin_view_drag(BTN_RIGHT, true)
    }

    pub unsafe fn view_drag_pinch_update(&mut self, scale: f64) -> bool {
        let Some(d) = self.view_drag.as_ref() else { return false };
        if !d.from_pinch {
            return false;
        }
        // Pinch out (scale > 1) dollies in: an upward drag.
        let px = -scale.max(0.05).ln() * VIEW_DRAG_PINCH_PX * crate::shared::layout().touchpad_view_sensitivity;
        let target_sy = d.origin_sy + px * d.ratio;
        let dy_layout = (target_sy - d.sy) / d.ratio;
        self.move_view_drag(0.0, dy_layout);
        true
    }

    pub unsafe fn view_drag_pinch_end(&mut self) -> bool {
        if matches!(&self.view_drag, Some(d) if d.from_pinch) {
            self.end_view_drag("lift");
            return true;
        }
        false
    }
}

/// A trackpad scroll over one of those apps' own popups, turned into the
/// wheel the popup understands.
///
/// Why this exists: `ViewDrag` covers the app's window, but a menu is an
/// X11 override-redirect surface, so a scroll over one falls through as a
/// plain axis event — and Houdini with "Enable Trackpad Gestures" on routes
/// every scroll, a mouse wheel included, into its touchpad slide, which
/// moves the content by `QWheelEvent::pixelDelta`. Under Xwayland that is
/// always zero, and no event the compositor can shape changes it: Qt's xcb
/// backend synthesises pixelDelta only above a scroll increment of 15, and
/// Xwayland hardcodes its XIScrollClass increment at 1 (measured on a
/// shadow session — `xinput list --long` reports `increment: 1.000000` on
/// both valuators, and a Qt6 probe logged `pixel=0` for wheel-source and
/// finger-source scrolls alike, on a QMenu the compositor delivered to
/// correctly). What does work is the app's own escape hatch: Houdini's
/// `touchpadwheelmodifier` — "simulate the mouse wheel", Ctrl — makes it
/// read a scroll as a wheel again. So over such a popup the compositor
/// holds Ctrl for the gesture and forwards the finger deltas as whole
/// notches, which is what the user otherwise has to do by hand.
pub struct PopupWheel {
    /// The popup the gesture started on. It also ends the gesture: if the
    /// pointer focus moves off it (the menu closed, or the pointer left),
    /// the held Ctrl must not ride along onto whatever took its place.
    pub surface: *mut ffi::wlr_surface,
    /// Sub-notch remainder per axis (0 horizontal, 1 vertical), layout px.
    pub accum: [f64; 2],
    pub ctrl_down: bool,
    /// The keyboard's repeat settings before the gesture, restored at its
    /// end. Xwayland autorepeats a held key as release/press pairs, which
    /// would drop the modifier mid-swipe — `ViewDrag` hit the same thing
    /// with Space.
    pub repeat: Option<(i32, i32)>,
}

pub(crate) const KEY_LEFTCTRL: u32 = 29;
/// Layout pixels per emitted notch — the unit wl_pointer and `inject_scroll`
/// already use for one wheel click.
pub(crate) const POPUP_WHEEL_NOTCH_PX: f64 = 15.0;
/// Finger silence that ends a popup gesture when the lift never came. Far
/// shorter than the view drag's net, because the two failure modes are not
/// alike: a resumed swipe only re-presses Ctrl, with no view mode to fall
/// out of, while a modifier left held would turn the user's next click into
/// a Ctrl-click.
pub(crate) const POPUP_WHEEL_IDLE_MS: i32 = 400;

impl Cursor {
    /// The popup under the pointer, when it belongs to the same process as
    /// a window `touchpad_view_apps` names and the keyboard is inside that
    /// app. Houdini's menus carry no WM_CLASS of their own, so the pid is
    /// what ties one to its app — the same test
    /// `XwaylandOverrideRedirect::focus_if_desired` uses. The keyboard
    /// check is what keeps a synthetic Ctrl from landing in some other
    /// client: the popup takes focus itself when it wants it, otherwise
    /// focus stays on the window it belongs to.
    unsafe fn popup_wheel_target(&mut self) -> Option<*mut ffi::wlr_surface> {
        let server = (*self.seat).server;
        let wm = &(*crate::reentry::wm(server));
        if crate::shared::layout().touchpad_view_apps.is_empty() {
            return None;
        }
        let result = crate::shared::scene().at(self.x(), self.y())?;
        let SceneNodeDataVal::OverrideRedirect(or) = result.data else { return None };
        if or.is_null() || result.surface.is_null() || (*or).xsurface.is_null() {
            return None;
        }
        let pid = (*(*or).xsurface).pid;
        let focused = ffi::river_wlr_seat_get_keyboard_focused_surface((*self.seat).wlr_seat);
        if focused.is_null() {
            return None;
        }
        for &window in wm.windows.iter() {
            if window.is_null() {
                continue;
            }
            let crate::window::WindowImpl::Xwayland(xwindow) = (*window).impl_type else { continue };
            if xwindow.is_null() || (*(*xwindow).xsurface).pid != pid {
                continue;
            }
            let app_id = (*window).get_app_id_string().unwrap_or_default();
            if !crate::shared::layout().touchpad_view_apps.iter().any(|p| crate::window_manager::app_id_matches(p, &app_id)) {
                continue;
            }
            if focused == result.surface || focused == (*window).root_surface() {
                return Some(result.surface);
            }
        }
        None
    }

    /// Hold, or drop, the app's "simulate the mouse wheel" modifier on the
    /// client's behalf. The mask is OR'd over the keyboard's live state and
    /// never into `injected_key_mods`, so every modifier read the compositor
    /// makes for itself — the Ctrl branch in `view_drag_axis` among them —
    /// keeps seeing the user's real keys and not this one.
    unsafe fn hold_popup_ctrl(&mut self, down: bool) {
        match self.popup_wheel.as_mut() {
            Some(p) if p.ctrl_down != down => p.ctrl_down = down,
            _ => return,
        }
        self.hold_synthetic_modifier(KEY_LEFTCTRL, b"Control\0", down);
    }

    /// Press or release `key` on the client's behalf and OR its xkb
    /// modifier (`xkb_name`, NUL-terminated) over the keyboard's live state.
    /// Never touches `injected_key_mods`, so the compositor's own modifier
    /// reads keep seeing the user's real keys. Shared by `PopupWheel`
    /// (Ctrl) and `HScrollShift` (Shift).
    unsafe fn hold_synthetic_modifier(&mut self, key: u32, xkb_name: &[u8], down: bool) {
        let seat = &mut *self.seat;
        let time = crate::util::msec_timestamp();
        let state = if down {
            ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED
        } else {
            ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED
        };
        ffi::wlr_seat_keyboard_notify_key(seat.wlr_seat, time, key, state);
        let kb = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
        if !kb.is_null() && !(*kb).keymap.is_null() {
            let idx = ffi::xkb_keymap_mod_get_index((*kb).keymap, xkb_name.as_ptr() as *const _);
            if idx != ffi::XKB_MOD_INVALID {
                // Released sends the device's own state back, which is the
                // user's keys minus this bit.
                let mut mods = (*kb).modifiers;
                if down {
                    mods.depressed |= 1u32 << idx;
                }
                ffi::wlr_seat_keyboard_notify_modifiers(seat.wlr_seat, &mut mods);
            }
        }
    }

    unsafe fn arm_popup_wheel_timer(&mut self) {
        if !self.view_drag_timer.is_null() {
            ffi::wl_event_source_timer_update(self.view_drag_timer, POPUP_WHEEL_IDLE_MS);
        }
    }

    /// `reason` names what ended it, as `end_view_drag`'s does.
    pub unsafe fn end_popup_wheel(&mut self, reason: &str) {
        if self.popup_wheel.is_none() {
            return;
        }
        self.hold_popup_ctrl(false);
        let Some(p) = self.popup_wheel.take() else { return };
        log::info!("[PopupWheel] end reason={}", reason);
        if !self.view_drag_timer.is_null() {
            ffi::wl_event_source_timer_update(self.view_drag_timer, 0);
        }
        if let Some((rate, delay)) = p.repeat {
            let kbd = ffi::river_wlr_seat_get_keyboard((*self.seat).wlr_seat);
            if !kbd.is_null() {
                ffi::wlr_keyboard_set_repeat_info(kbd, rate, delay);
            }
        }
    }

    /// A finger-source axis event over such a popup, emitted to it as whole
    /// wheel notches under a held Ctrl. Returns true when consumed.
    pub unsafe fn popup_wheel_axis(&mut self, event: *const ffi::wlr_pointer_axis_event, delta: f64, modifiers: u32) -> bool {
        const CTRL: u32 = 0x4;
        if delta == 0.0 {
            // The fingers lifted.
            if self.popup_wheel.is_some() {
                self.end_popup_wheel("lift");
                return true;
            }
            return false;
        }
        if modifiers & CTRL != 0 {
            // The user is already holding the app's wheel modifier: the
            // scroll passes through as it does today.
            if self.popup_wheel.is_some() {
                self.end_popup_wheel("ctrl");
            }
            return false;
        }
        if self.popup_wheel.is_none() {
            let Some(surface) = self.popup_wheel_target() else { return false };
            let seat = &mut *self.seat;
            seat.ensure_synthetic_keyboard();
            let mut repeat = None;
            let kbd = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
            if !kbd.is_null() {
                repeat = Some(((*kbd).repeat_info.rate, (*kbd).repeat_info.delay));
                ffi::wlr_keyboard_set_repeat_info(kbd, 0, 0);
            }
            log::info!("[PopupWheel] begin on popup surface {:p}", surface);
            self.popup_wheel = Some(PopupWheel { surface, accum: [0.0, 0.0], ctrl_down: false, repeat });
            self.hold_popup_ctrl(true);
        }
        // The gesture belongs to the popup it started on.
        let focused = ffi::river_wlr_seat_get_pointer_focused_surface((*self.seat).wlr_seat);
        if matches!(&self.popup_wheel, Some(p) if focused != p.surface) {
            self.end_popup_wheel("left-popup");
            return false;
        }
        let vertical = (*event).orientation == ffi::wl_pointer_axis_WL_POINTER_AXIS_VERTICAL_SCROLL;
        let axis = if vertical { 1usize } else { 0usize };
        let notches = {
            let Some(p) = self.popup_wheel.as_mut() else { return false };
            p.accum[axis] += delta;
            let whole = (p.accum[axis] / POPUP_WHEEL_NOTCH_PX).trunc();
            p.accum[axis] -= whole * POPUP_WHEEL_NOTCH_PX;
            whole
        };
        if notches != 0.0 {
            let seat = &mut *self.seat;
            let time = crate::util::msec_timestamp();
            let step = POPUP_WHEEL_NOTCH_PX * notches.signum();
            let discrete = 120 * notches.signum() as i32;
            for _ in 0..notches.abs() as i32 {
                ffi::wlr_seat_pointer_notify_axis(
                    seat.wlr_seat,
                    time,
                    (*event).orientation,
                    step,
                    discrete,
                    ffi::wl_pointer_axis_source_WL_POINTER_AXIS_SOURCE_WHEEL,
                    (*event).relative_direction,
                );
                ffi::wlr_seat_pointer_notify_frame(seat.wlr_seat);
            }
        }
        self.arm_popup_wheel_timer();
        true
    }
}

pub(crate) unsafe extern "C" fn handle_view_drag_timeout(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let cursor = &mut *(data as *mut Cursor);
    cursor.end_view_drag("idle");
    cursor.end_popup_wheel("idle");
    cursor.end_hscroll_shift("idle");
    0
}

/// A horizontal trackpad scroll over an app whose widgets cannot use one,
/// turned into the Shift + vertical scroll they can.
///
/// Why this exists: Houdini's native panes -- the geometry spreadsheet
/// first of all -- read a wheel event's magnitude and ignore its axis: a
/// horizontal wheel scrolls the rows, and the documented way to scroll the
/// columns is to hold Shift while scrolling. Measured live: a horizontal
/// two-finger swipe reached Houdini as a proper horizontal QWheelEvent
/// (angleDelta x only) and moved the rows; the same swipe with Shift held
/// moved the columns. So over such a window the compositor holds Shift for
/// the gesture and re-emits each horizontal finger delta as a vertical one,
/// same sign, same source. A vertical delta arriving mid-gesture (a swipe
/// drifting off axis) is dropped rather than sent sideways; the gesture
/// ends on the lift, on real pointer motion, a button, a key, or silence.
///
/// The user already holding Shift or Ctrl passes through untouched: Shift
/// means they are doing it by hand, Ctrl is the app's own wheel modifier.
pub struct HScrollShift {
    /// The window's surface the gesture started on; pointer focus moving
    /// off it ends the gesture, so the held Shift never reaches another.
    pub surface: *mut ffi::wlr_surface,
    pub shift_down: bool,
    /// Keyboard repeat before the gesture, restored at its end (see
    /// `PopupWheel::repeat`).
    pub repeat: Option<(i32, i32)>,
}

pub(crate) const KEY_LEFTSHIFT: u32 = 42;
/// Finger silence that ends the gesture when the lift never came; a short
/// one is safe here because nothing is held that a re-press would break
/// (unlike `ViewDrag`'s Space).
pub(crate) const HSCROLL_SHIFT_IDLE_MS: i32 = 300;

impl Cursor {
    /// The surface under the pointer, if it belongs to a window of an app
    /// in `touchpad_hscroll_shift_apps`.
    unsafe fn hscroll_shift_target(&mut self) -> Option<*mut ffi::wlr_surface> {
        if crate::shared::layout().touchpad_hscroll_shift_apps.is_empty() {
            return None;
        }
        let result = crate::shared::scene().at(self.x(), self.y())?;
        let SceneNodeDataVal::Window(window) = result.data else { return None };
        if window.is_null() || result.surface.is_null() {
            return None;
        }
        let app_id = (*window).get_app_id_string().unwrap_or_default();
        if !crate::shared::layout().touchpad_hscroll_shift_apps.iter().any(|p| crate::window_manager::app_id_matches(p, &app_id)) {
            return None;
        }
        Some(result.surface)
    }

    unsafe fn hold_hscroll_shift(&mut self, down: bool) {
        match self.hscroll_shift.as_mut() {
            Some(h) if h.shift_down != down => h.shift_down = down,
            _ => return,
        }
        self.hold_synthetic_modifier(KEY_LEFTSHIFT, b"Shift\0", down);
    }

    /// `reason` names what ended it, as `end_view_drag`'s does.
    pub unsafe fn end_hscroll_shift(&mut self, reason: &str) {
        if self.hscroll_shift.is_none() {
            return;
        }
        self.hold_hscroll_shift(false);
        let Some(h) = self.hscroll_shift.take() else { return };
        log::info!("[HScrollShift] end reason={}", reason);
        if !self.view_drag_timer.is_null() {
            ffi::wl_event_source_timer_update(self.view_drag_timer, 0);
        }
        if let Some((rate, delay)) = h.repeat {
            let kbd = ffi::river_wlr_seat_get_keyboard((*self.seat).wlr_seat);
            if !kbd.is_null() {
                ffi::wlr_keyboard_set_repeat_info(kbd, rate, delay);
            }
        }
    }

    /// A finger-source axis event over such a window. Returns true when
    /// consumed (re-emitted as Shift + vertical, or dropped).
    pub unsafe fn hscroll_shift_axis(&mut self, event: *const ffi::wlr_pointer_axis_event, delta: f64, delta_discrete: i32, modifiers: u32) -> bool {
        const SHIFT: u32 = 0x1;
        const CTRL: u32 = 0x4;
        let vertical = (*event).orientation == ffi::wl_pointer_axis_WL_POINTER_AXIS_VERTICAL_SCROLL;
        if delta == 0.0 {
            // The fingers lifted. Both axes lift; the first one ends it and
            // the second finds nothing to do -- and neither must reach the
            // client as a stray horizontal event.
            if self.hscroll_shift.is_some() {
                self.end_hscroll_shift("lift");
                return true;
            }
            return false;
        }
        if modifiers & (SHIFT | CTRL) != 0 {
            if self.hscroll_shift.is_some() {
                self.end_hscroll_shift("modifier");
            }
            return false;
        }
        if self.hscroll_shift.is_none() {
            if vertical {
                return false;
            }
            let Some(surface) = self.hscroll_shift_target() else { return false };
            let seat = &mut *self.seat;
            seat.ensure_synthetic_keyboard();
            let mut repeat = None;
            let kbd = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
            if !kbd.is_null() {
                repeat = Some(((*kbd).repeat_info.rate, (*kbd).repeat_info.delay));
                ffi::wlr_keyboard_set_repeat_info(kbd, 0, 0);
            }
            log::info!("[HScrollShift] begin on surface {:p}", surface);
            self.hscroll_shift = Some(HScrollShift { surface, shift_down: false, repeat });
            self.hold_hscroll_shift(true);
        }
        let focused = ffi::river_wlr_seat_get_pointer_focused_surface((*self.seat).wlr_seat);
        if matches!(&self.hscroll_shift, Some(h) if focused != h.surface) {
            self.end_hscroll_shift("left-window");
            return false;
        }
        if vertical {
            // Off-axis drift mid-gesture: under the held Shift it would
            // scroll sideways too. Swallow it.
            self.arm_hscroll_shift_timer();
            return true;
        }
        let seat = &mut *self.seat;
        ffi::wlr_seat_pointer_notify_axis(
            seat.wlr_seat,
            (*event).time_msec,
            ffi::wl_pointer_axis_WL_POINTER_AXIS_VERTICAL_SCROLL,
            delta,
            delta_discrete,
            (*event).source,
            (*event).relative_direction,
        );
        self.arm_hscroll_shift_timer();
        true
    }

    unsafe fn arm_hscroll_shift_timer(&mut self) {
        if !self.view_drag_timer.is_null() {
            ffi::wl_event_source_timer_update(self.view_drag_timer, HSCROLL_SHIFT_IDLE_MS);
        }
    }
}
