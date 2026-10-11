use crate::ffi;
use crate::seat::{Seat, Focus};
use crate::server::{WlList};
use crate::scene_node_data::SceneNodeDataVal;
use crate::drag_icon::DragIcon;
use std::collections::{HashMap, HashSet};

pub use crate::touch::{TouchPoint, TouchRoute};

#[path = "cursor/button.rs"]
mod button;
pub(crate) use button::*;
#[path = "cursor/axis.rs"]
mod axis;
pub use axis::*;
#[path = "cursor/gestures.rs"]
mod gestures;
pub(crate) use gestures::*;
#[path = "cursor/border_zone.rs"]
mod border_zone;
pub use border_zone::*;

pub struct Cursor {
    pub seat: *mut Seat,
    pub wlr_cursor: *mut ffi::wlr_cursor,
    pub xcursor_manager: *mut ffi::wlr_xcursor_manager,
    pub constraint: *mut crate::pointer_constraint::PointerConstraint,

    /// Trackpad-to-view-drag emulation for the apps `touchpad_view_apps`
    /// names (see `ViewDrag`); `None` while no emulated drag is in progress.
    pub view_drag: Option<ViewDrag>,
    /// Trackpad-to-wheel emulation over those apps' popups (see
    /// `PopupWheel`); `None` while no such gesture is in progress. The two
    /// are mutually exclusive — a view drag needs a window under the
    /// pointer, this one an override-redirect surface.
    pub popup_wheel: Option<PopupWheel>,
    /// Horizontal-scroll-as-Shift+vertical emulation over the apps
    /// `touchpad_hscroll_shift_apps` names (see `HScrollShift`); `None`
    /// while no such gesture is in progress.
    pub hscroll_shift: Option<HScrollShift>,
    /// Ends an emulated gesture — a `ViewDrag` or a `PopupWheel`, which
    /// never run at once — that has seen no finger event for a while: a
    /// two-finger scroll normally ends with a zero-delta axis event, but
    /// not every path delivers one. Each arms it with its own timeout.
    pub view_drag_timer: *mut ffi::wl_event_source,

    /// Animated-XCursor playback. An XCursor theme may ship several images per
    /// cursor with a per-frame `delay`; wlroots parses them but never advances
    /// them — `wlr_xcursor_frame` has no callers inside wlroots, so driving the
    /// animation is the compositor's job. This timer is the driver.
    ///
    /// `anim_xcursor` doubles as the "is anything animating" flag: it is null
    /// for every static cursor, which is the overwhelmingly common case, and
    /// the timer then stays disarmed and costs nothing. It borrows from
    /// `xcursor_manager`'s theme, so it MUST be cleared before that manager is
    /// destroyed (see `set_theme`) and whenever anything else takes over this
    /// seat's cursor image (a client surface). Tablet tools are not such a
    /// case: each drives its own `wlr_cursor`, never the seat's.
    pub anim_timer: *mut ffi::wl_event_source,
    pub anim_xcursor: *mut ffi::wlr_xcursor,
    /// Wall-clock ms at which the current animation started, so elapsed time
    /// (not absolute time) picks the frame and every cursor starts at frame 0.
    pub anim_started_msec: u32,

    pub motion_listener: crate::listener::Listener,
    pub motion_absolute_listener: crate::listener::Listener,
    pub button_listener: crate::listener::Listener,
    pub axis_listener: crate::listener::Listener,
    pub frame_listener: crate::listener::Listener,

    pub tablet_tool_axis_listener: crate::listener::Listener,
    pub tablet_tool_proximity_listener: crate::listener::Listener,
    pub tablet_tool_tip_listener: crate::listener::Listener,
    pub tablet_tool_button_listener: crate::listener::Listener,

    /// Every finger on a touchscreen, by touch id: where it is (layout
    /// coordinates; the drag icon of a touch drag follows it) and where its
    /// events go, decided once at touch-down (`TouchRoute`).
    pub touch_points: HashMap<i32, TouchPoint>,
    /// The cursor image is off because the last input was a touchscreen.
    /// A finger has no pointer to show, and the emulated pointer warping to
    /// every tap would otherwise leave an arrow wherever the hand last was.
    /// Real pointer motion brings it back (`unhide_after_touch`).
    pub hidden_by_touch: bool,
    /// A gesture the compositor has taken from the touchscreen: an edge
    /// swipe, a desk pan/zoom, or a three/four-finger swipe or pinch
    /// (`touch::Claim`). While one is live, every finger is its.
    pub touch_claim: crate::touch::Claim,
    /// The swipe being run through `handle_swipe_*` comes from the
    /// touchscreen (`touch::Claim::Multi`), not a touchpad: the trackpad's
    /// gesture switches do not gate it, and no client hears it as a
    /// pointer gesture.
    pub gesture_from_touch: bool,
    /// Buttons the compositor has seen pressed and not yet released.
    pub pressed: HashSet<u32>,
    /// Buttons whose PRESS was forwarded to the focused client. The paired
    /// release must reach the client no matter what the compositor is doing
    /// by then (seat op, overview, …) — an orphaned press wedges client-side
    /// input state (widget routers keep a grab armed forever).
    pub notified_pressed: HashSet<u32>,
    /// Layout origin of the surface the first notified press landed on: the
    /// implicit grab's frame of reference, so held-button motion stays
    /// surface-relative wherever the pointer goes (passthrough's grab branch).
    pub grab_origin: (f64, f64),
    /// Surface units per layout pixel for the grabbed surface, frozen at
    /// press: 1 for a buffer shown at its natural size, the output scale
    /// for an X11 surface under xwayland_hidpi (physical-pixel buffer drawn
    /// at 1/scale), 1/zoom in the overview. Without it a held-button drag
    /// reached an X11 client at half speed.
    pub grab_scale: f64,

    pub touch_down_listener: crate::listener::Listener,
    pub touch_motion_listener: crate::listener::Listener,
    pub touch_up_listener: crate::listener::Listener,
    pub touch_cancel_listener: crate::listener::Listener,
    pub touch_frame_listener: crate::listener::Listener,

    pub swipe_begin_listener: crate::listener::Listener,
    pub swipe_update_listener: crate::listener::Listener,
    pub swipe_end_listener: crate::listener::Listener,

    pub pinch_begin_listener: crate::listener::Listener,
    pub pinch_update_listener: crate::listener::Listener,
    pub pinch_end_listener: crate::listener::Listener,

    pub hold_begin_listener: crate::listener::Listener,
    pub hold_end_listener: crate::listener::Listener,

    pub gesture_dx: f64,
    pub gesture_dy: f64,
    pub gesture_scale: f64,
    /// A bind has fired during the in-flight swipe or pinch. For a swipe
    /// it does not end the gesture — the travel restarts and can fire
    /// again — it records that clients were sent a cancelled end and hear
    /// nothing more of it; a pinch fires once.
    pub gesture_triggered: bool,
    /// The in-flight swipe fired a bind that is not a step (anything
    /// `action_navigates` rejects: the overview toggle, a spawn): the rest
    /// of the gesture fires nothing more, so one swipe toggles the
    /// overview once however far the fingers go. Cleared at swipe begin.
    pub swipe_spent: bool,
    /// A focus bind the in-flight swipe reached with no window that way
    /// (`WindowManager::focus_toward_lands`): it does not fire, and while
    /// the swipe keeps matching it the lean holds at its limit rather than
    /// stepping. Cleared at swipe begin and by a step that lands.
    pub swipe_dead_end: Option<crate::config::Action>,
    /// Camera offset (virtual units, `[x, y]`) the in-flight swipe has
    /// peeked the desktop by so far — see `swipe_peek_for`. Zero outside a
    /// swipe, and restarts from zero at each fire, like the travel.
    pub swipe_peek: [f64; 2],
    /// Finger count of a staged injected swipe (`pointer-swipe begin`),
    /// carried into its updates the way libinput repeats it per event.
    pub inject_swipe_fingers: u32,
    pub panning_gesture_active: bool,
    /// What `pointer-scroll ... natural` sets: an injected finger scroll
    /// (no device behind it) reads as coming from a natural-scrolling
    /// touchpad, so the view drag's un-inversion can be exercised headlessly.
    pub inject_natural: bool,
    /// Finger-pan velocity estimate per axis (`[x, y]`, virtual units/s) and
    /// the hardware timestamp of each axis's last finger event, for the
    /// kinetic desktop coast on the lift.
    pub pan_vel: [f64; 2],
    pub pan_last_msec: [u32; 2],
    /// A pinch that began over the desktop background zooms the camera
    /// continuously instead of forwarding to a client or matching gesture
    /// binds. Decided once at pinch begin; update/end must follow the same
    /// branch so the client protocol never sees an update without a begin.
    pub pinch_zoom_active: bool,
    /// Camera zoom captured at pinch begin: libinput's pinch `scale` is
    /// absolute w.r.t. the gesture start, so every update maps off this.
    pub pinch_start_zoom: f64,
    pub last_click_time: u32,
    pub last_click_window: *mut crate::window::Window,
    /// Window whose border currently draws highlighted, kept to un-highlight
    /// on hover transitions. May dangle after a close — validate against
    /// `wm.windows` before dereferencing.
    pub hovered_border_window: *mut crate::window::Window,
    /// Which of that window's handle discs is highlighted.
    pub hovered_border_element: Option<crate::window::BorderElement>,
    /// The toplevel under the pointer while adjust mode (overview, or Super
    /// held) is on: the window the ring lands on and whose handles are live,
    /// focused or not. Set by `passthrough` on every hover evaluation and
    /// cleared when the pointer rests on nothing adjustable or the mode is
    /// off — see `Window::is_adjust_target`. May dangle after a close:
    /// compared by address only, and nulled in `Window::destroy`.
    pub adjust_hover: *mut crate::window::Window,
    pub right_click_on_bg: bool,
    pub right_click_on_border: bool,
    /// A left press on a window button (minimize / maximize / float-tile),
    /// waiting for its release: a button acts on the release, and only when
    /// the pointer is still on the same button of the same window. May
    /// dangle after a close — validated against `wm.windows` at release.
    pub button_press: Option<(*mut crate::window::Window, crate::window::BorderElement)>,
    /// The grid surface's node mapping — (node_x, node_y, scale) — FROZEN at
    /// the start of an implicit grab that landed on the grid, cleared when
    /// the grab's last button releases. Motion during the grab is mapped
    /// against these values, never the live node: the camera moves the node
    /// per frame, and a flight mid-grab (overview enter/exit) read as
    /// thousands of px of "pointer motion" under a live mapping — one click
    /// on a desktop item during a flight flung it off the canvas.
    pub grab_grid: Option<(f64, f64, f64)>,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            seat: std::ptr::null_mut(),
            wlr_cursor: std::ptr::null_mut(),
            xcursor_manager: std::ptr::null_mut(),
            constraint: std::ptr::null_mut(),
            anim_timer: std::ptr::null_mut(),
            anim_xcursor: std::ptr::null_mut(),
            anim_started_msec: 0,
            motion_listener: unsafe { std::mem::zeroed() },
            motion_absolute_listener: unsafe { std::mem::zeroed() },
            button_listener: unsafe { std::mem::zeroed() },
            axis_listener: unsafe { std::mem::zeroed() },
            frame_listener: unsafe { std::mem::zeroed() },
            tablet_tool_axis_listener: unsafe { std::mem::zeroed() },
            tablet_tool_proximity_listener: unsafe { std::mem::zeroed() },
            tablet_tool_tip_listener: unsafe { std::mem::zeroed() },
            tablet_tool_button_listener: unsafe { std::mem::zeroed() },

            touch_points: HashMap::new(),
            hidden_by_touch: false,
            touch_claim: crate::touch::Claim::None,
            gesture_from_touch: false,
            pressed: HashSet::new(),
            notified_pressed: HashSet::new(),
            grab_origin: (0.0, 0.0),
            grab_scale: 1.0,

            touch_down_listener: unsafe { std::mem::zeroed() },
            touch_motion_listener: unsafe { std::mem::zeroed() },
            touch_up_listener: unsafe { std::mem::zeroed() },
            touch_cancel_listener: unsafe { std::mem::zeroed() },
            touch_frame_listener: unsafe { std::mem::zeroed() },

            swipe_begin_listener: unsafe { std::mem::zeroed() },
            swipe_update_listener: unsafe { std::mem::zeroed() },
            swipe_end_listener: unsafe { std::mem::zeroed() },

            pinch_begin_listener: unsafe { std::mem::zeroed() },
            pinch_update_listener: unsafe { std::mem::zeroed() },
            pinch_end_listener: unsafe { std::mem::zeroed() },

            hold_begin_listener: unsafe { std::mem::zeroed() },
            hold_end_listener: unsafe { std::mem::zeroed() },

            gesture_dx: 0.0,
            gesture_dy: 0.0,
            gesture_scale: 1.0,
            gesture_triggered: false,
            swipe_spent: false,
            swipe_dead_end: None,
            swipe_peek: [0.0, 0.0],
            inject_swipe_fingers: 3,
            panning_gesture_active: false,
            inject_natural: false,
            pan_vel: [0.0, 0.0],
            pan_last_msec: [0, 0],
            pinch_zoom_active: false,
            pinch_start_zoom: 1.0,
            view_drag: None,
            popup_wheel: None,
            hscroll_shift: None,
            view_drag_timer: std::ptr::null_mut(),
            last_click_time: 0,
            last_click_window: std::ptr::null_mut(),
            hovered_border_window: std::ptr::null_mut(),
            hovered_border_element: None,
            adjust_hover: std::ptr::null_mut(),
            right_click_on_bg: false,
            right_click_on_border: false,
            button_press: None,
            grab_grid: None,
        }
    }
}

impl Cursor {
    pub unsafe fn init(
        &mut self,
        seat: *mut Seat,
        output_layout: *mut ffi::wlr_output_layout,
    ) -> Result<(), &'static str> {
        let wlr_cursor = ffi::wlr_cursor_create();
        if wlr_cursor.is_null() {
            return Err("Failed to create wlr_cursor");
        }
        ffi::wlr_cursor_attach_output_layout(wlr_cursor, output_layout);

        let xcursor_manager = ffi::wlr_xcursor_manager_create(std::ptr::null(), 24);
        if xcursor_manager.is_null() {
            ffi::wlr_cursor_destroy(wlr_cursor);
            return Err("Failed to create wlr_xcursor_manager");
        }

        self.seat = seat;
        self.wlr_cursor = wlr_cursor;
        self.xcursor_manager = xcursor_manager;

        // The animated-cursor driver. Created disarmed; `set_xcursor` arms it
        // only for a cursor that actually has more than one frame.
        let event_loop = ffi::wl_display_get_event_loop((*(*seat).server).wl_server);
        let anim_timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_xcursor_anim),
            self as *mut Cursor as *mut _,
        );
        if anim_timer.is_null() {
            ffi::wlr_xcursor_manager_destroy(xcursor_manager);
            ffi::wlr_cursor_destroy(wlr_cursor);
            self.xcursor_manager = std::ptr::null_mut();
            self.wlr_cursor = std::ptr::null_mut();
            return Err("Failed to create xcursor animation timer");
        }
        self.anim_timer = anim_timer;
        self.view_drag_timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_view_drag_timeout),
            self as *mut Cursor as *mut _,
        );

        // Load default cursor theme
        ffi::wlr_xcursor_manager_load(xcursor_manager, 1.0);
        self.set_xcursor(b"default\0".as_ptr() as *const _);

        // Setup listeners
        self.motion_listener.connect(ffi::river_wlr_cursor_get_motion_signal(wlr_cursor), handle_motion);

        self.motion_absolute_listener.connect(ffi::river_wlr_cursor_get_motion_absolute_signal(wlr_cursor), handle_motion_absolute);

        self.button_listener.connect(ffi::river_wlr_cursor_get_button_signal(wlr_cursor), handle_button);

        self.axis_listener.connect(ffi::river_wlr_cursor_get_axis_signal(wlr_cursor), handle_axis);

        self.frame_listener.connect(ffi::river_wlr_cursor_get_frame_signal(wlr_cursor), handle_frame);

        self.tablet_tool_axis_listener.connect(ffi::river_wlr_cursor_get_tablet_tool_axis_signal(wlr_cursor), handle_tablet_tool_axis);

        self.tablet_tool_proximity_listener.connect(ffi::river_wlr_cursor_get_tablet_tool_proximity_signal(wlr_cursor), handle_tablet_tool_proximity);

        self.tablet_tool_tip_listener.connect(ffi::river_wlr_cursor_get_tablet_tool_tip_signal(wlr_cursor), handle_tablet_tool_tip);

        self.tablet_tool_button_listener.connect(ffi::river_wlr_cursor_get_tablet_tool_button_signal(wlr_cursor), handle_tablet_tool_button);

        self.touch_down_listener.connect(ffi::river_wlr_cursor_get_touch_down_signal(wlr_cursor), crate::touch::handle_touch_down);

        self.touch_motion_listener.connect(ffi::river_wlr_cursor_get_touch_motion_signal(wlr_cursor), crate::touch::handle_touch_motion);

        self.touch_up_listener.connect(ffi::river_wlr_cursor_get_touch_up_signal(wlr_cursor), crate::touch::handle_touch_up);

        self.touch_cancel_listener.connect(ffi::river_wlr_cursor_get_touch_cancel_signal(wlr_cursor), crate::touch::handle_touch_cancel);

        self.touch_frame_listener.connect(ffi::river_wlr_cursor_get_touch_frame_signal(wlr_cursor), crate::touch::handle_touch_frame);

        self.swipe_begin_listener.connect(ffi::river_wlr_cursor_get_swipe_begin_signal(wlr_cursor), handle_swipe_begin);

        self.swipe_update_listener.connect(ffi::river_wlr_cursor_get_swipe_update_signal(wlr_cursor), handle_swipe_update);

        self.swipe_end_listener.connect(ffi::river_wlr_cursor_get_swipe_end_signal(wlr_cursor), handle_swipe_end);

        self.pinch_begin_listener.connect(ffi::river_wlr_cursor_get_pinch_begin_signal(wlr_cursor), handle_pinch_begin);

        self.pinch_update_listener.connect(ffi::river_wlr_cursor_get_pinch_update_signal(wlr_cursor), handle_pinch_update);

        self.pinch_end_listener.connect(ffi::river_wlr_cursor_get_pinch_end_signal(wlr_cursor), handle_pinch_end);

        self.hold_begin_listener.connect(ffi::river_wlr_cursor_get_hold_begin_signal(wlr_cursor), handle_hold_begin);

        self.hold_end_listener.connect(ffi::river_wlr_cursor_get_hold_end_signal(wlr_cursor), handle_hold_end);

        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        self.motion_listener.disconnect();
        self.motion_absolute_listener.disconnect();
        self.button_listener.disconnect();
        self.axis_listener.disconnect();
        self.frame_listener.disconnect();

        self.tablet_tool_axis_listener.disconnect();
        self.tablet_tool_proximity_listener.disconnect();
        self.tablet_tool_tip_listener.disconnect();
        self.tablet_tool_button_listener.disconnect();

        self.touch_down_listener.disconnect();
        self.touch_motion_listener.disconnect();
        self.touch_up_listener.disconnect();
        self.touch_cancel_listener.disconnect();
        self.touch_frame_listener.disconnect();

        self.swipe_begin_listener.disconnect();
        self.swipe_update_listener.disconnect();
        self.swipe_end_listener.disconnect();

        self.pinch_begin_listener.disconnect();
        self.pinch_update_listener.disconnect();
        self.pinch_end_listener.disconnect();

        self.hold_begin_listener.disconnect();
        self.hold_end_listener.disconnect();

        self.stop_xcursor_animation();
        if !self.anim_timer.is_null() {
            ffi::wl_event_source_remove(self.anim_timer);
            self.anim_timer = std::ptr::null_mut();
        }

        if !self.xcursor_manager.is_null() {
            ffi::wlr_xcursor_manager_destroy(self.xcursor_manager);
            self.xcursor_manager = std::ptr::null_mut();
        }
        if !self.wlr_cursor.is_null() {
            ffi::wlr_cursor_destroy(self.wlr_cursor);
            self.wlr_cursor = std::ptr::null_mut();
        }
    }

    pub unsafe fn x(&self) -> f64 {
        ffi::river_wlr_cursor_get_x(self.wlr_cursor)
    }

    pub unsafe fn y(&self) -> f64 {
        ffi::river_wlr_cursor_get_y(self.wlr_cursor)
    }

    pub unsafe fn update_hovered(&mut self) {
        // Keyboard focus remains stable on pointer hover/motion.
        // It is only changed on mapping, click, touch, or shortcut focus transitions.
    }

    pub unsafe fn update_state(&mut self) {
        if !self.constraint.is_null() {
            (*self.constraint).update_state();
        }
        self.update_hovered();
        self.passthrough(crate::util::msec_timestamp());
    }

    /// Re-evaluate pointer focus at the current position after the scene
    /// mapping changed beneath a STATIONARY cursor — a commit moved/resized a
    /// surface or re-anchored its window geometry. No motion fired (the
    /// cursor did not move), so enter/leave and the surface-local coordinates
    /// would otherwise go stale and the next click could be dispatched
    /// against the old mapping or dropped entirely (the cce-ui overflow-rim
    /// popovers exposed exactly this). Skips seats mid-op: the compositor
    /// owns the pointer during a drag/resize op and `op_end_pointer` restores
    /// focus itself; an implicit client grab is already handled inside
    /// `passthrough`.
    pub unsafe fn refresh_after_scene_change(&mut self) {
        if (*self.seat).op.is_some() {
            return;
        }
        self.update_state();
    }

    pub unsafe fn update_drag_icons(&mut self) {
        let drag_icons_tree = (*(*self.seat).server).scene.drag_icons.raw();
        let children_head = ffi::river_scene_tree_get_children(drag_icons_tree) as *mut WlList;
        let mut curr = (*children_head).next;
        while curr != children_head {
            let next = (*curr).next;
            let node = ffi::river_scene_node_from_children_link(curr as *mut ffi::wl_list);
            let drag_icon_raw = ffi::river_scene_node_get_data(node) as *mut DragIcon;
            if !drag_icon_raw.is_null() {
                let drag_icon = &mut *drag_icon_raw;
                let drag_seat = ffi::river_wlr_drag_get_seat((*drag_icon.wlr_drag_icon).drag);
                if drag_seat == (*self.seat).wlr_seat {
                    drag_icon.update_position(self);
                }
            }
            curr = next;
        }
    }

    pub unsafe fn set_theme(
        &mut self,
        theme: *const std::os::raw::c_char,
        size: u32,
    ) -> Result<(), &'static str> {
        let size = if size == 0 { 24 } else { size };

        let xcursor_manager = ffi::wlr_xcursor_manager_create(theme, size);
        if xcursor_manager.is_null() {
            return Err("Failed to create wlr_xcursor_manager");
        }

        // If this cursor belongs to the default seat, update the Xwayland cursor to match the theme.
        let default_seat = (*(*self.seat).server).input_manager.default_seat;
        if self.seat == default_seat {
            if !(*(*self.seat).server).xwayland.is_null() {
                ffi::wlr_xcursor_manager_load(xcursor_manager, 1.0);
                let wlr_xcursor = ffi::wlr_xcursor_manager_get_xcursor(
                    xcursor_manager,
                    b"default\0".as_ptr() as *const _,
                    1.0,
                );
                if !wlr_xcursor.is_null() && (*wlr_xcursor).image_count > 0 {
                    let image = *(*wlr_xcursor).images;
                    if !image.is_null() {
                        let buffer = ffi::wlr_xcursor_image_get_buffer(image);
                        ffi::wlr_xwayland_set_cursor(
                            (*(*self.seat).server).xwayland,
                            buffer,
                            (*image).hotspot_x as i32,
                            (*image).hotspot_y as i32,
                        );
                    }
                }
            }
        }

        // Before the old theme is freed: a running animation holds a pointer
        // into its images.
        self.stop_xcursor_animation();

        if !self.xcursor_manager.is_null() {
            ffi::wlr_xcursor_manager_destroy(self.xcursor_manager);
        }
        self.xcursor_manager = xcursor_manager;

        ffi::wlr_xcursor_manager_load(self.xcursor_manager, 1.0);
        self.set_xcursor(b"default\0".as_ptr() as *const _);

        Ok(())
    }

    pub unsafe fn set_xcursor(&mut self, name: *const std::os::raw::c_char) {
        if self.hidden_by_touch {
            return;
        }
        ffi::wlr_cursor_set_xcursor(
            self.wlr_cursor,
            self.xcursor_manager,
            name,
        );
        // `wlr_cursor_set_xcursor` paints frame 0 and stops there; if this
        // cursor has more frames, take over from here.
        self.start_xcursor_animation(name);
    }

    /// Stop any running xcursor animation and disarm the timer. Idempotent, and
    /// safe to call when nothing is animating.
    ///
    /// Every path that hands this seat's cursor image to someone else must call
    /// this: a client surface cursor, or a theme swap that frees the images
    /// `anim_xcursor` borrows. The gate is deliberately on the
    /// unsafe transitions rather than a list of states where animating is known
    /// to be fine — a state nobody thought of must land on "stop", not on
    /// "keep dereferencing a freed theme".
    pub unsafe fn stop_xcursor_animation(&mut self) {
        self.anim_xcursor = std::ptr::null_mut();
        if !self.anim_timer.is_null() {
            // 0 disarms a libwayland timer.
            ffi::wl_event_source_timer_update(self.anim_timer, 0);
        }
    }

    /// Begin driving `name`'s frames, if it has more than one. Static cursors —
    /// every cursor in a stock theme — leave the timer disarmed.
    unsafe fn start_xcursor_animation(&mut self, name: *const std::os::raw::c_char) {
        if self.anim_timer.is_null() || self.xcursor_manager.is_null() {
            self.stop_xcursor_animation();
            return;
        }

        let xcursor = ffi::wlr_xcursor_manager_get_xcursor(self.xcursor_manager, name, 1.0);

        // Already playing this exact cursor: leave its clock running. The
        // compositor re-asserts its cursor constantly — `clear_focus` sets
        // "default" on EVERY pointer motion over the background — and
        // restarting here would re-arm the timer sooner than the frame delay
        // and pin the animation on frame 0 for as long as the mouse moves.
        // The pointer is stable per (theme, name, scale) for the manager's
        // lifetime, and `set_theme` stops the animation before freeing the old
        // manager, so a stale pointer can never compare equal.
        if !xcursor.is_null() && xcursor == self.anim_xcursor {
            return;
        }

        self.stop_xcursor_animation();
        if xcursor.is_null() {
            return;
        }
        // One image is a static cursor. `total_delay == 0` would also make
        // `wlr_xcursor_frame`'s `time % total_delay` divide by zero, so a theme
        // with all-zero delays must never reach the driver.
        if (*xcursor).image_count <= 1 || (*xcursor).total_delay == 0 {
            return;
        }

        self.anim_xcursor = xcursor;
        self.anim_started_msec = crate::util::msec_timestamp();
        self.arm_next_frame(0);
    }

    /// Arm the timer for the delay of frame `idx`, clamped to >= 1ms: a zero
    /// delay on a single frame would re-arm instantly and spin the event loop.
    unsafe fn arm_next_frame(&mut self, idx: u32) {
        let xcursor = self.anim_xcursor;
        if xcursor.is_null() || idx >= (*xcursor).image_count {
            return;
        }
        let image = *(*xcursor).images.offset(idx as isize);
        if image.is_null() {
            return;
        }
        let delay = (*image).delay.max(1).min(i32::MAX as u32) as i32;
        ffi::wl_event_source_timer_update(self.anim_timer, delay);
    }

    /// Timer body: pick the frame for the elapsed time and paint it.
    ///
    /// The frame is derived from elapsed wall-clock rather than a running
    /// counter, so a late or coalesced timer resyncs to where the animation
    /// should be instead of drifting further behind.
    unsafe fn advance_xcursor_frame(&mut self) {
        let xcursor = self.anim_xcursor;
        if xcursor.is_null() || self.wlr_cursor.is_null() {
            return;
        }

        let elapsed = crate::util::msec_timestamp().wrapping_sub(self.anim_started_msec);
        let idx = ffi::wlr_xcursor_frame(xcursor, elapsed);
        if idx < 0 || idx as u32 >= (*xcursor).image_count {
            return;
        }
        let idx = idx as u32;

        let image = *(*xcursor).images.offset(idx as isize);
        if image.is_null() {
            return;
        }
        let buffer = ffi::wlr_xcursor_image_get_buffer(image);
        if !buffer.is_null() {
            ffi::wlr_cursor_set_buffer(
                self.wlr_cursor,
                buffer,
                (*image).hotspot_x as i32,
                (*image).hotspot_y as i32,
                1.0,
            );
        }
        self.arm_next_frame(idx);
    }

    pub unsafe fn op_start_pointer(&mut self) {
        if !self.constraint.is_null() {
            if let crate::pointer_constraint::PointerConstraintState::Active { .. } = (*self.constraint).state {
                (*self.constraint).deactivate();
            }
        }

        log::debug!("entering cursor mode op");
        ffi::wlr_seat_pointer_notify_clear_focus((*self.seat).wlr_seat);

        // A grab does not focus the window it moves or resizes, and it was
        // the press's `seat.focus` that used to raise a Floating one — so
        // the raise happens here for whatever the op grabbed. (A window
        // un-tiled by the drag is raised in `op_update` instead.)
        let grabbed = match &(*self.seat).op {
            Some(op) => op.window_ptr,
            None => std::ptr::null_mut(),
        };
        if !grabbed.is_null()
            && !(*grabbed).closed
            && !(*grabbed).is_status_bar()
            && (*grabbed).tiling_mode == crate::tiling::TilingMode::Floating
        {
            (*(*self.seat).server).wm.raise_window(grabbed);
            (*(*self.seat).server).wm.dirty_windowing();
        }
    }

    pub unsafe fn op_end_pointer(&mut self) {
        if self.pressed.is_empty() {
            log::debug!("entering cursor mode passthrough");
            self.update_state();
        } else {
            log::debug!("entering cursor mode ignore");
        }
    }

    /// Move the border hover highlight to `target`'s `element` (null/None to
    /// clear): updates `hovered_border_element` on the old and new windows
    /// and repaints their borders. The stored pointer may refer to a closed
    /// window, so it is only dereferenced after checking it is still in
    /// `wm.windows`.
    pub unsafe fn set_border_hover(
        &mut self,
        target: *mut crate::window::Window,
        element: Option<crate::window::BorderElement>,
    ) {
        if self.hovered_border_window == target && self.hovered_border_element == element {
            return;
        }
        let old = self.hovered_border_window;
        if !old.is_null() && old != target {
            let wm = &(*(*self.seat).server).wm;
            if wm.windows.iter().any(|&w| w == old) && !(*old).closed {
                (*old).hovered_border_element = None;
            }
        }
        self.hovered_border_window = target;
        self.hovered_border_element = element;
        if !target.is_null() {
            (*target).hovered_border_element = element;
        }
        // Borders rest invisible and fade in, so the change in hover target is
        // the start of an animation rather than a repaint: the fade timer
        // repaints every affected window as it steps.
        (*(*self.seat).server).wm.arm_border_fade();
    }

    /// Move the Super-held adjust target to `target` (null to clear). The
    /// ring eases off the old window and onto the new one through the same
    /// border fade a hover swap uses, so a change arms that timer.
    pub unsafe fn set_adjust_hover(&mut self, target: *mut crate::window::Window) {
        if self.adjust_hover == target {
            return;
        }
        self.adjust_hover = target;
        (*(*self.seat).server).wm.arm_border_fade();
    }

    pub unsafe fn passthrough(&mut self, time_msec: u32) {
        let lx = self.x();
        let ly = self.y();
        let server = (*self.seat).server;

        // Implicit grab (standard Wayland drag semantics): while any button
        // the focused client saw pressed is still held, motion keeps flowing
        // to that surface — relative to its origin at press time — wherever
        // the pointer goes, so a client-side drag (slider, ramp key) tracks
        // outside the window. Focus is neither re-evaluated nor cleared until
        // the last such button releases; wlroots nulls the focused surface if
        // it is destroyed mid-grab, which falls through to normal dispatch.
        // A DRAG supersedes the implicit grab: wlroots' drag grab owns pointer
        // focus for its duration, and the whole point is that focus follows
        // the pointer onto whatever it is dragged over. Holding focus on the
        // source here means the drag target never changes, so no drop is ever
        // delivered anywhere — the source keeps receiving motion instead.
        if !self.notified_pressed.is_empty() && (*self.seat).drag == crate::seat::DragState::None {
            let focused =
                ffi::river_wlr_seat_get_pointer_focused_surface((*self.seat).wlr_seat);
            if !focused.is_null() {
                // A grab held on the GRID maps through the surface node, not
                // through the fixed origin: the offset formula assumes an
                // unscaled surface, and the grid displays at the patch's
                // resolution ratio — near 1, but off it whenever the zoom
                // sits between pow2 quantization steps (most of overview) —
                // so offset deltas would drag the item faster or slower than
                // the pointer by exactly that ratio. The mapping is the one
                // FROZEN at press time (`grab_grid`), never the live node:
                // the camera moves the node per frame while the patch stays
                // put, so a flight mid-grab (overview enter/exit — union
                // patches are pre-issued, so the client's origin-change
                // re-baseline never fires) read as the whole flight distance
                // of "pointer motion" under a live mapping and flung the
                // grabbed item off the canvas.
                if let Some((nx, ny, scale)) = self.grab_grid {
                    // Still guarded on the grid owning the focus: if the
                    // grid remapped mid-grab (client restart), the frozen
                    // values must not map points for its replacement.
                    if let Some((gsurf, ..)) = grid_node_info(server) {
                        if gsurf == focused {
                            ffi::wlr_seat_pointer_notify_motion(
                                (*self.seat).wlr_seat,
                                time_msec,
                                (lx - nx) / scale,
                                (ly - ny) / scale,
                            );
                            return;
                        }
                    }
                }
                ffi::wlr_seat_pointer_notify_motion(
                    (*self.seat).wlr_seat,
                    time_msec,
                    (lx - self.grab_origin.0) * self.grab_scale,
                    (ly - self.grab_origin.1) * self.grab_scale,
                );
                return;
            }
        }

        if let Some(result) = (*server).scene.at(lx, ly) {
            let lock_state = (*server).lock_manager.state;
            if lock_state != crate::lock_manager::LockState::Unlocked {
                if !matches!(result.data, SceneNodeDataVal::LockSurface(_)) {
                    self.set_border_hover(std::ptr::null_mut(), None);
                    self.set_adjust_hover(std::ptr::null_mut());
                    self.clear_focus();
                    return;
                }
            } else {
                if matches!(result.data, SceneNodeDataVal::LockSurface(_)) {
                    self.set_border_hover(std::ptr::null_mut(), None);
                    self.set_adjust_hover(std::ptr::null_mut());
                    self.clear_focus();
                    return;
                }
            }

            let mut is_window = false;
            let mut hovered_toplevel: *mut crate::window::Window = std::ptr::null_mut();
            match result.data {
                SceneNodeDataVal::Window(window) => {
                    // The grid is not a toplevel for hover purposes: hitting it
                    // means the pointer is on a desktop item (its input region
                    // covers nothing else), and the item drag needs the
                    // enter/motion delivery below — in overview too, where the
                    // is_window branch would clear pointer focus and try to
                    // focus-follow onto a surface seat.focus refuses.
                    if !(*window).is_status_bar()
                        && !(*window).is_wallpaper()
                        && !(*window).is_grid()
                    {
                        is_window = true;
                        hovered_toplevel = window;
                    }
                    // Adjust mode: the ring lands on the window under the
                    // pointer, focused or not. Set BEFORE the zone test
                    // below, so the band is live on the first hover. (Null
                    // for the status bar, wallpaper and grid.)
                    self.set_adjust_hover(if (*server).wm.window_adjust_active() {
                        hovered_toplevel
                    } else {
                        std::ptr::null_mut()
                    });
                    // No mode gate: the band scales with the window
                    // (get_border_zone is zoom-aware), so the resize/move
                    // controls reveal and work at any zoom, not just 1.
                    if !(*window).is_status_bar()
                        && !(*window).is_wallpaper()
                        && (*window).tiling_mode != crate::tiling::TilingMode::Popup
                        && (*window).tiling_mode != crate::tiling::TilingMode::Fullscreen
                        && (*window).tiling_mode != crate::tiling::TilingMode::Status
                    {
                        match get_border_zone(window, lx, ly) {
                            BorderZone::Resize(edges) => {
                                self.set_border_hover(window, Some(border_element_for_edges(edges)));
                                ffi::wlr_seat_pointer_notify_clear_focus((*self.seat).wlr_seat);
                                let cursor_name = get_resize_cursor_name(edges);
                                self.set_xcursor(cursor_name.as_ptr() as *const _);
                                return;
                            }
                            BorderZone::Move => {
                                self.set_border_hover(window, Some(crate::window::BorderElement::Top));
                                ffi::wlr_seat_pointer_notify_clear_focus((*self.seat).wlr_seat);
                                self.set_xcursor(b"grab\0".as_ptr() as *const _);
                                return;
                            }
                            BorderZone::Button(elem) => {
                                self.set_border_hover(window, Some(elem));
                                ffi::wlr_seat_pointer_notify_clear_focus((*self.seat).wlr_seat);
                                self.set_xcursor(b"pointer\0".as_ptr() as *const _);
                                return;
                            }
                            BorderZone::None => {}
                        }
                    }
                }
                SceneNodeDataVal::OverrideRedirect(_) => {
                    is_window = true;
                    self.set_adjust_hover(std::ptr::null_mut());
                }
                _ => {
                    self.set_adjust_hover(std::ptr::null_mut());
                }
            }
            self.set_border_hover(std::ptr::null_mut(), None);

            // Chrome — a Popup (the cce-cloud launcher) or an Overlay dock —
            // is live UI during overview, not a spatial thumbnail: the button
            // path already lets its presses through to the app, and hover has
            // to reach it the same way or its rows never highlight and a
            // press lands on a surface that never saw an enter. So it skips
            // the overview branch below and takes normal delivery.
            let hovered_chrome = !hovered_toplevel.is_null()
                && matches!(
                    (*hovered_toplevel).tiling_mode,
                    crate::tiling::TilingMode::Popup | crate::tiling::TilingMode::Overlay
                );

            if is_window
                && !hovered_chrome
                && (*server).wm.window_adjust_active()
            {
                // Focus follows the pointer in overview, so a click-less
                // hover chooses the window a focus chord or the exit lands
                // on. (The ring itself keys on `adjust_hover`, not focus.) (Super-held adjust mode takes this
                // branch too, for the pointer-focus clear below, but not the
                // refocus.) Guarded on an actual change —
                // seat.focus raises a Floating window BEFORE its same-focus
                // short-circuit, so an unguarded call would raise and relayout
                // on every motion event. And with the pan suppressed: hovering
                // must not move the camera, only the click and keyboard paths
                // may. (Zoom was never at stake — focus_follow_pan pans at the
                // current zoom — but a partially visible window would still
                // get dragged on-screen mid-hover.) And never while chrome
                // holds the keyboard: the cce-cloud launcher closes itself on
                // keyboard leave, and this path runs not only on real motion
                // but on the idle pointer refresh a commit schedules — the
                // launcher's own first configure — so a stationary pointer
                // resting on a world window was refocusing that window and
                // dismissing the launcher the instant it mapped.
                // Overview only: with Super held at zoom 1 focus stays put,
                // so a focus chord pressed next acts on the window the user
                // had — the ring follows the pointer through `adjust_hover`
                // instead (`Window::is_adjust_target`).
                if (*server).wm.mode == crate::window_manager::WindowManagerMode::Overview
                    && !hovered_toplevel.is_null()
                    && !(*self.seat).focus_is_chrome()
                    && (*self.seat).focused
                        != crate::seat::Focus::Window(hovered_toplevel)
                {
                    let prev = (*self.seat).suppress_focus_pan;
                    (*self.seat).suppress_focus_pan = true;
                    (*self.seat).focus(crate::seat::Focus::Window(hovered_toplevel));
                    (*self.seat).suppress_focus_pan = prev;
                }
                self.clear_focus();
                return;
            }

            if !result.surface.is_null() {
                ffi::wlr_seat_pointer_notify_enter((*self.seat).wlr_seat, result.surface, result.sx, result.sy);
                ffi::wlr_seat_pointer_notify_motion((*self.seat).wlr_seat, time_msec, result.sx, result.sy);
                return;
            }
        }

        // Nothing under the pointer: the desktop background. A DRAG in
        // progress is the one case that still needs a surface here — Wayland
        // delivers drops to surfaces, and the background is not one, so
        // without this every drag onto the desktop is cancelled on release.
        // The grid client stands in as the desktop's drop target: it already
        // covers the canvas and renders it, so it is the thing that can say
        // what "dropped at this spot" means. It stays input-transparent for
        // every other purpose (see `Scene::at`) — only the drag resolves onto
        // it, and only while the pointer is over the background, so clicks,
        // hover and the overview background-exit are untouched.
        if (*self.seat).drag != crate::seat::DragState::None {
            if let Some((surface, sx, sy)) = grid_surface_at(server, lx, ly) {
                log::debug!("[drag] focus -> grid at ({lx:.0}, {ly:.0})");
                ffi::wlr_seat_pointer_notify_enter((*self.seat).wlr_seat, surface, sx, sy);
                ffi::wlr_seat_pointer_notify_motion((*self.seat).wlr_seat, time_msec, sx, sy);
                return;
            }
        }

        self.set_border_hover(std::ptr::null_mut(), None);
        self.set_adjust_hover(std::ptr::null_mut());
        self.clear_focus();
    }

    /// Take the cursor image off for a touch (see `hidden_by_touch`).
    pub(crate) unsafe fn hide_for_touch(&mut self) {
        if self.hidden_by_touch {
            return;
        }
        self.hidden_by_touch = true;
        self.stop_xcursor_animation();
        ffi::wlr_cursor_unset_image(self.wlr_cursor);
    }

    /// Bring the cursor image back once a real pointer moves. Called before
    /// the motion's passthrough, which re-enters the surface under the
    /// pointer: the clear here is what makes that enter fresh, so the
    /// client sets its cursor again — `handle_request_set_cursor` dropped
    /// whatever it asked for while the image was off. Not while a client
    /// holds an implicit grab: clearing pointer focus would end it.
    pub unsafe fn unhide_after_touch(&mut self) {
        if !self.hidden_by_touch {
            return;
        }
        self.hidden_by_touch = false;
        if self.notified_pressed.is_empty() {
            ffi::wlr_seat_pointer_notify_clear_focus((*self.seat).wlr_seat);
        }
        self.set_xcursor(b"default\0".as_ptr() as *const _);
    }

    pub unsafe fn clear_focus(&mut self) {
        ffi::wlr_seat_pointer_notify_clear_focus((*self.seat).wlr_seat);
        self.set_xcursor(b"default\0".as_ptr() as *const _);
    }

    // ── Synthetic pointer injection (`ccectl pointer-*`) ─────────────────────────
    // Each call builds the event a real device would deliver and runs it through the
    // REAL handler (via its listener — the same entry wlroots invokes), so compositor
    // policy — window ops, overview gating, grabs, focus, pointer constraints — treats
    // injected input exactly like hardware. Every handler null-checks `event.pointer`,
    // so a device-less event is safe. Buttons and axes finish with a frame, like a
    // real device batch. Press and release are separate entry points on purpose: a
    // held drag is press → any number of moves → release.

    /// Warp to layout coordinates and run the motion tail (hover, drag icons, and
    /// op-update-or-passthrough, mirroring `handle_motion`). `wlr_cursor_warp` takes
    /// layout pixels — `wlr_cursor_warp_absolute` is 0..1-normalized, which is the
    /// bug the old `pointer-move-to` had.
    pub unsafe fn inject_motion_to(&mut self, x: f64, y: f64) {
        self.unhide_after_touch();
        self.warp_to(x, y);
    }

    /// `inject_motion_to` without bringing a touch-hidden cursor back: the
    /// emulated pointer of a touch (`TouchRoute::Pointer`) moves this way.
    pub(crate) unsafe fn warp_to(&mut self, x: f64, y: f64) {
        (*self.seat).handle_activity();
        ffi::wlr_cursor_warp(self.wlr_cursor, std::ptr::null_mut(), x, y);
        self.update_hovered();
        self.update_drag_icons();
        let seat = &mut *self.seat;
        if seat.op.is_some() {
            let lx = (*self.wlr_cursor).x as i32;
            let ly = (*self.wlr_cursor).y as i32;
            seat.op_update(lx, ly);
            return;
        }
        self.passthrough(crate::util::msec_timestamp());
        // Real devices terminate every motion batch with a frame; sctk-based clients
        // queue pointer events until they see one.
        handle_frame(self.frame_listener.as_ptr(), std::ptr::null_mut());
    }

    pub unsafe fn inject_motion_by(&mut self, dx: f64, dy: f64) {
        let mut ev = ffi::wlr_pointer_motion_event {
            pointer: std::ptr::null_mut(),
            time_msec: crate::util::msec_timestamp(),
            delta_x: dx,
            delta_y: dy,
            unaccel_dx: dx,
            unaccel_dy: dy,
        };
        handle_motion(
            self.motion_listener.as_ptr(),
            &mut ev as *mut ffi::wlr_pointer_motion_event as *mut std::ffi::c_void,
        );
        handle_frame(self.frame_listener.as_ptr(), std::ptr::null_mut());
    }

    pub unsafe fn inject_button(&mut self, button: u32, pressed: bool) {
        let state = if pressed {
            ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED
        } else {
            ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_RELEASED
        };
        let mut ev = ffi::wlr_pointer_button_event {
            pointer: std::ptr::null_mut(),
            time_msec: crate::util::msec_timestamp(),
            button,
            state,
        };
        handle_button(
            self.button_listener.as_ptr(),
            &mut ev as *mut ffi::wlr_pointer_button_event as *mut std::ffi::c_void,
        );
        handle_frame(self.frame_listener.as_ptr(), std::ptr::null_mut());
    }

    /// Wheel scroll; positive `dy` scrolls down (content up), matching a real wheel.
    /// One notch is 15 delta units / 120 `value120` steps (the libinput convention).
    /// `finger` injects a touchpad two-finger scroll (axis source FINGER, no
    /// discrete steps) instead of a wheel click, so a headless session can
    /// exercise the trackpad paths — the compositor's own desk pan and what
    /// X11/Wayland clients receive. A finger scroll ends with a zero-delta
    /// event, which `finger_stop` sends.
    pub unsafe fn inject_scroll(&mut self, dy: f64, dx: f64, finger: bool) {
        let time = crate::util::msec_timestamp();
        for (delta, orientation) in [
            (dy, ffi::wl_pointer_axis_WL_POINTER_AXIS_VERTICAL_SCROLL),
            (dx, ffi::wl_pointer_axis_WL_POINTER_AXIS_HORIZONTAL_SCROLL),
        ] {
            if delta == 0.0 {
                continue;
            }
            let mut ev = ffi::wlr_pointer_axis_event {
                pointer: std::ptr::null_mut(),
                time_msec: time,
                source: if finger {
                    ffi::wl_pointer_axis_source_WL_POINTER_AXIS_SOURCE_FINGER
                } else {
                    ffi::wl_pointer_axis_source_WL_POINTER_AXIS_SOURCE_WHEEL
                },
                orientation,
                relative_direction: ffi::wl_pointer_axis_relative_direction_WL_POINTER_AXIS_RELATIVE_DIRECTION_IDENTICAL,
                delta,
                delta_discrete: if finger { 0 } else { ((delta / 15.0) * 120.0) as i32 },
            };
            handle_axis(
                self.axis_listener.as_ptr(),
                &mut ev as *mut ffi::wlr_pointer_axis_event as *mut std::ffi::c_void,
            );
        }
        handle_frame(self.frame_listener.as_ptr(), std::ptr::null_mut());
        // Every libinput axis event is followed by a frame; clients
        // (Xwayland among them) deliver only on the frame, and wlroots
        // asserts if a later axis event changes source within one.
        ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
    }

    /// The zero-delta event that ends a finger scroll (libinput sends one
    /// when the fingers lift); see `inject_scroll`.
    pub unsafe fn inject_finger_stop(&mut self) {
        let time = crate::util::msec_timestamp();
        for orientation in [
            ffi::wl_pointer_axis_WL_POINTER_AXIS_VERTICAL_SCROLL,
            ffi::wl_pointer_axis_WL_POINTER_AXIS_HORIZONTAL_SCROLL,
        ] {
            let mut ev = ffi::wlr_pointer_axis_event {
                pointer: std::ptr::null_mut(),
                time_msec: time,
                source: ffi::wl_pointer_axis_source_WL_POINTER_AXIS_SOURCE_FINGER,
                orientation,
                relative_direction: ffi::wl_pointer_axis_relative_direction_WL_POINTER_AXIS_RELATIVE_DIRECTION_IDENTICAL,
                delta: 0.0,
                delta_discrete: 0,
            };
            handle_axis(
                self.axis_listener.as_ptr(),
                &mut ev as *mut ffi::wlr_pointer_axis_event as *mut std::ffi::c_void,
            );
        }
        // Every libinput axis event is followed by a frame; clients
        // (Xwayland among them) deliver only on the frame, and wlroots
        // asserts if a later axis event changes source within one.
        ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
    }

    /// Inject a whole two-finger pinch: begin, `steps` updates easing the
    /// scale from 1 to `scale` (and the rotation to `rotation` degrees),
    /// end. Exercises the compositor's pinch policy and the
    /// pointer-gestures forward to clients headlessly.
    /// One stage of a pinch, for a test that paces the updates itself:
    /// `stage` is "begin", "update" (with `scale`/`rotation`) or "end".
    pub unsafe fn inject_pinch_stage(&mut self, stage: &str, scale: f64, rotation: f64) {
        let time = crate::util::msec_timestamp();
        match stage {
            "begin" => {
                let mut ev = ffi::wlr_pointer_pinch_begin_event { pointer: std::ptr::null_mut(), time_msec: time, fingers: 2 };
                handle_pinch_begin(self.pinch_begin_listener.as_ptr(), &mut ev as *mut _ as *mut std::ffi::c_void);
            }
            "update" => {
                let mut ev = ffi::wlr_pointer_pinch_update_event { pointer: std::ptr::null_mut(), time_msec: time, fingers: 2, dx: 0.0, dy: 0.0, scale, rotation };
                handle_pinch_update(self.pinch_update_listener.as_ptr(), &mut ev as *mut _ as *mut std::ffi::c_void);
            }
            _ => {
                let mut ev = ffi::wlr_pointer_pinch_end_event { pointer: std::ptr::null_mut(), time_msec: time, cancelled: false };
                handle_pinch_end(self.pinch_end_listener.as_ptr(), &mut ev as *mut _ as *mut std::ffi::c_void);
            }
        }
        ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
    }

    /// One stage of a touchpad swipe, paced by the caller (`pointer-swipe
    /// begin <fingers> | update <dx> <dy> | end`): what lets a shadow hold
    /// a swipe short of its threshold and read the camera's peek back.
    pub unsafe fn inject_swipe_stage(&mut self, stage: &str, fingers: u32, dx: f64, dy: f64) {
        let time = crate::util::msec_timestamp();
        match stage {
            "begin" => {
                self.inject_swipe_fingers = fingers;
                let mut ev = ffi::wlr_pointer_swipe_begin_event { pointer: std::ptr::null_mut(), time_msec: time, fingers };
                handle_swipe_begin(self.swipe_begin_listener.as_ptr(), &mut ev as *mut _ as *mut std::ffi::c_void);
            }
            "update" => {
                let fingers = self.inject_swipe_fingers;
                let mut ev = ffi::wlr_pointer_swipe_update_event { pointer: std::ptr::null_mut(), time_msec: time, fingers, dx, dy };
                handle_swipe_update(self.swipe_update_listener.as_ptr(), &mut ev as *mut _ as *mut std::ffi::c_void);
            }
            _ => {
                let mut ev = ffi::wlr_pointer_swipe_end_event { pointer: std::ptr::null_mut(), time_msec: time, cancelled: false };
                handle_swipe_end(self.swipe_end_listener.as_ptr(), &mut ev as *mut _ as *mut std::ffi::c_void);
            }
        }
        ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
    }

    /// Inject a whole touchpad swipe: begin with `fingers`, `steps` updates
    /// that together move the gesture centre by (`dx`, `dy`), end. Drives
    /// the gesture-bind table (`swipe3_left` in input.kdl) headlessly;
    /// the pointer-gestures forward to clients runs too.
    pub unsafe fn inject_swipe(&mut self, fingers: u32, dx: f64, dy: f64, steps: u32) {
        let time = crate::util::msec_timestamp();
        let mut begin = ffi::wlr_pointer_swipe_begin_event {
            pointer: std::ptr::null_mut(),
            time_msec: time,
            fingers,
        };
        handle_swipe_begin(
            self.swipe_begin_listener.as_ptr(),
            &mut begin as *mut ffi::wlr_pointer_swipe_begin_event as *mut std::ffi::c_void,
        );
        ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
        let steps = steps.max(1);
        for i in 1..=steps {
            let mut update = ffi::wlr_pointer_swipe_update_event {
                pointer: std::ptr::null_mut(),
                time_msec: time + i,
                fingers,
                dx: dx / steps as f64,
                dy: dy / steps as f64,
            };
            handle_swipe_update(
                self.swipe_update_listener.as_ptr(),
                &mut update as *mut ffi::wlr_pointer_swipe_update_event as *mut std::ffi::c_void,
            );
            ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
        }
        let mut end = ffi::wlr_pointer_swipe_end_event {
            pointer: std::ptr::null_mut(),
            time_msec: time + steps + 1,
            cancelled: false,
        };
        handle_swipe_end(
            self.swipe_end_listener.as_ptr(),
            &mut end as *mut ffi::wlr_pointer_swipe_end_event as *mut std::ffi::c_void,
        );
        ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
    }

    pub unsafe fn inject_pinch(&mut self, scale: f64, rotation: f64, steps: u32) {
        let time = crate::util::msec_timestamp();
        let mut begin = ffi::wlr_pointer_pinch_begin_event {
            pointer: std::ptr::null_mut(),
            time_msec: time,
            fingers: 2,
        };
        handle_pinch_begin(
            self.pinch_begin_listener.as_ptr(),
            &mut begin as *mut ffi::wlr_pointer_pinch_begin_event as *mut std::ffi::c_void,
        );
        ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
        let steps = steps.max(1);
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let mut update = ffi::wlr_pointer_pinch_update_event {
                pointer: std::ptr::null_mut(),
                time_msec: time + i,
                fingers: 2,
                dx: 0.0,
                dy: 0.0,
                scale: 1.0 + (scale - 1.0) * t,
                rotation: rotation * t,
            };
            handle_pinch_update(
                self.pinch_update_listener.as_ptr(),
                &mut update as *mut ffi::wlr_pointer_pinch_update_event as *mut std::ffi::c_void,
            );
            ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
        }
        let mut end = ffi::wlr_pointer_pinch_end_event {
            pointer: std::ptr::null_mut(),
            time_msec: time + steps + 1,
            cancelled: false,
        };
        handle_pinch_end(
            self.pinch_end_listener.as_ptr(),
            &mut end as *mut ffi::wlr_pointer_pinch_end_event as *mut std::ffi::c_void,
        );
        ffi::wlr_seat_pointer_notify_frame((*self.seat).wlr_seat);
    }
}

unsafe extern "C" fn handle_motion(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, motion_listener);
    let event = data as *mut ffi::wlr_pointer_motion_event;
    // Pointer input is activity for the idle timeouts and the idle-notify
    // clients. Injected events (`ccectl pointer-*`) arrive here too, and
    // count: a script driving the pointer is someone using the desk.
    (*cursor.seat).handle_activity();
    // Real pointer motion ends an emulated view drag: the client must not
    // see the synthetic drag position and the true one interleaved.
    if cursor.view_drag.is_some() {
        cursor.end_view_drag("motion");
    }
    // Likewise the held Shift must not ride along to wherever the pointer
    // goes next.
    if cursor.hscroll_shift.is_some() {
        cursor.end_hscroll_shift("motion");
    }
    
    cursor.unhide_after_touch();

    let mut dx = (*event).delta_x;
    let mut dy = (*event).delta_y;

    if !cursor.constraint.is_null() {
        (*cursor.constraint).confine(&mut dx, &mut dy);
    }

    // Real libinput motion collapses the compositor to ~3fps, while an injected
    // warp at the SAME event rate sustains ~58fps — so the cost is somewhere in
    // this handler rather than in rendering or the scene. Time each stage.
    let t_start = if crate::output::frame_debug() {
        Some(std::time::Instant::now())
    } else {
        None
    };

    ffi::wlr_cursor_move(cursor.wlr_cursor, std::ptr::null_mut(), dx, dy);
    let t_move = t_start.map(|s| s.elapsed().as_micros());
    cursor.update_hovered();
    let t_hovered = t_start.map(|s| s.elapsed().as_micros());
    cursor.update_drag_icons();
    let t_drag = t_start.map(|s| s.elapsed().as_micros());

    let seat = &mut *cursor.seat;
    if (*seat).op.is_some() {
        let lx = (*cursor.wlr_cursor).x as i32;
        let ly = (*cursor.wlr_cursor).y as i32;
        (*seat).op_update(lx, ly);
        if let (Some(s), Some(m), Some(h), Some(d)) = (t_start, t_move, t_hovered, t_drag) {
            log::info!(
                "[cce-frame] t={} motion(op) total={}us move={}us hovered={}us drag={}us",
                crate::util::msec_timestamp() % 100000,
                s.elapsed().as_micros(), m, h - m, d - h
            );
        }
        return;
    }

    cursor.passthrough((*event).time_msec);

    if let (Some(s), Some(m), Some(h), Some(d)) = (t_start, t_move, t_hovered, t_drag) {
        let total = s.elapsed().as_micros();
        log::info!(
            "[cce-frame] t={} motion total={}us move={}us hovered={}us drag={}us passthrough={}us",
            crate::util::msec_timestamp() % 100000,
            total, m, h - m, d - h, total - d
        );
    }
}

unsafe extern "C" fn handle_motion_absolute(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, motion_absolute_listener);
    let event = data as *mut ffi::wlr_pointer_motion_absolute_event;
    (*cursor.seat).handle_activity();
    cursor.unhide_after_touch();
    
    let wlr_device = if (*event).pointer.is_null() {
        std::ptr::null_mut()
    } else {
        &mut (*(*event).pointer).base as *mut ffi::wlr_input_device
    };
    ffi::wlr_cursor_warp_absolute(cursor.wlr_cursor, wlr_device, (*event).x, (*event).y);
    cursor.update_hovered();
    cursor.update_drag_icons();

    let seat = &mut *cursor.seat;
    if (*seat).op.is_some() {
        let lx = (*cursor.wlr_cursor).x as i32;
        let ly = (*cursor.wlr_cursor).y as i32;
        (*seat).op_update(lx, ly);
        return;
    }

    cursor.passthrough((*event).time_msec);
}

/// True when the layer surface's namespace marks it as cce-cloud chrome —
/// the launcher and the desktop/app context menus. These are screen-anchored
/// popups that stay interactive during overview and dismiss on click-away.
unsafe fn is_cloud_layer(layer_surface: *mut crate::layer_shell::LayerSurface) -> bool {
    if layer_surface.is_null() {
        return false;
    }
    let wlr_layer_surface = (*layer_surface).wlr_layer_surface;
    if wlr_layer_surface.is_null() || (*wlr_layer_surface).namespace.is_null() {
        return false;
    }
    std::ffi::CStr::from_ptr((*wlr_layer_surface).namespace)
        .to_string_lossy()
        .starts_with("cce-cloud")
}

/// Whether a click or touch on this layer surface may give it keyboard
/// focus. Not when it asked for none: per wlr-layer-shell such a surface is
/// never given keyboard focus, and the one that relies on it — an on-screen
/// keyboard (cce-keyboard), typing into the window it was clicked over —
/// cannot work if a click on its keys takes focus off that window.
pub(crate) unsafe fn layer_takes_click_focus(layer_surface: *mut crate::layer_shell::LayerSurface) -> bool {
    if layer_surface.is_null() || (*layer_surface).wlr_layer_surface.is_null() {
        return false;
    }
    (*(*layer_surface).wlr_layer_surface).current.keyboard_interactive
        != ffi::zwlr_layer_surface_v1_keyboard_interactivity_ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE
}

/// What every press does before anyone handles it: close the menus it
/// lands outside of. Shared by a pointer press (`handle_button`) and a
/// touch that goes to a client as touch (`handle_touch_down`); an
/// emulated touch arrives as a pointer press and runs it there.
pub(crate) unsafe fn press_dismissals(server: *mut crate::server::Server, lx: f64, ly: f64) {
    // Click-away-close for in-surface status menus: if any status
    // segment is expanded (menu open) and this press did not land on it,
    // push a one-shot dismiss over the status socket. The line carries
    // the pressed segment's app_id so a press ON an expanded segment
    // exempts that segment (it handles its own clicks) while still
    // dismissing any other open menu.
    {
        let mut target_status: *mut crate::window::Window = std::ptr::null_mut();
        if let Some(result) = (*server).scene.at(lx, ly) {
            if let SceneNodeDataVal::Window(window) = result.data {
                if (*window).is_status_bar() {
                    target_status = window;
                }
            }
        }
        let any_other_expanded = (*server).wm.any_expanded_status_segment(target_status);
        if any_other_expanded {
            let except = if target_status.is_null() {
                "-".to_string()
            } else {
                (*target_status).get_app_id_string().unwrap_or_else(|| "-".to_string())
            };
            if let Some(ref sender) = (*server).wm.status_sender {
                sender.send_menu_dismiss(&except);
            }
        }
    }

    // Click-away for X11 popups. Xwayland only sees the pointer over
    // its own surfaces, so a popup menu an X11 app opened (Wine's, most
    // of all — a tray icon's menu opened through cce-xembed-tray) never
    // hears a press on a Wayland window and stays open; only a click on
    // one of the app's own X windows used to close it. So a press that
    // lands on no X11 surface while some override-redirect window is
    // showing is reported on the status socket's `clickaway` topic, and
    // the bridge closes the popup it opened. The tray bridge's own
    // containers have no scene tree, so they never count as showing.
    {
        let or_showing = (*server)
            .wm
            .override_redirects
            .iter()
            .any(|&or| !or.is_null() && !(*or).surface_tree.is_null());
        if or_showing {
            let on_x11 = match (*server).scene.at(lx, ly) {
                Some(result) => match result.data {
                    SceneNodeDataVal::Window(window) => {
                        matches!((*window).impl_type, crate::window::WindowImpl::Xwayland(_))
                    }
                    SceneNodeDataVal::OverrideRedirect(_) => true,
                    _ => false,
                },
                None => false,
            };
            if !on_x11 {
                if let Some(ref sender) = (*server).wm.status_sender {
                    sender.send_click_away();
                }
            }
        }
    }
}

unsafe extern "C" fn handle_frame(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, frame_listener);

    let seat = &mut *cursor.seat;
    ffi::wlr_seat_pointer_notify_frame(seat.wlr_seat);
}

/// Animated-xcursor tick. `data` is the `Cursor`, which lives inline in a
/// `Box::into_raw`'d `Seat` and so never moves. Re-arms itself from
/// `advance_xcursor_frame`; a stopped animation leaves the timer disarmed and
/// this is not called again until something arms it.
unsafe extern "C" fn handle_xcursor_anim(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let cursor = &mut *(data as *mut Cursor);
    cursor.advance_xcursor_frame();
    0
}

unsafe extern "C" fn handle_tablet_tool_axis(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let _cursor = &mut *crate::container_of!(listener, Cursor, tablet_tool_axis_listener);
    let event = data as *mut ffi::wlr_tablet_tool_axis_event;

    let wlr_tablet = (*event).tablet;
    let wlr_device = &mut (*wlr_tablet).base as *mut ffi::wlr_input_device;
    let device = ffi::river_wlr_input_device_get_data(wlr_device) as *mut crate::input_device::InputDevice;
    if device.is_null() {
        return;
    }
    let seat = (*device).seat;
    (*seat).handle_activity();

    let tablet = (*device).destroy_data as *mut crate::tablet::Tablet;
    if tablet.is_null() {
        return;
    }

    if let Ok(tool) = crate::tablet_tool::TabletTool::get((*seat).wlr_seat, (*event).tool) {
        (*tool).axis(tablet, event);
    }
}

unsafe extern "C" fn handle_tablet_tool_proximity(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, tablet_tool_proximity_listener);
    let event = data as *mut ffi::wlr_tablet_tool_proximity_event;
    // A stylus sets its own image (`TabletTool::proximity`); the touch
    // hide must not then block the compositor's next one.
    cursor.hidden_by_touch = false;

    let wlr_tablet = (*event).tablet;
    let wlr_device = &mut (*wlr_tablet).base as *mut ffi::wlr_input_device;
    let device = ffi::river_wlr_input_device_get_data(wlr_device) as *mut crate::input_device::InputDevice;
    if device.is_null() {
        return;
    }
    let seat = (*device).seat;
    (*seat).handle_activity();

    let tablet = (*device).destroy_data as *mut crate::tablet::Tablet;
    if tablet.is_null() {
        return;
    }

    if let Ok(tool) = crate::tablet_tool::TabletTool::get((*seat).wlr_seat, (*event).tool) {
        (*tool).proximity(tablet, event);
    }
}

unsafe extern "C" fn handle_tablet_tool_tip(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let _cursor = &mut *crate::container_of!(listener, Cursor, tablet_tool_tip_listener);
    let event = data as *mut ffi::wlr_tablet_tool_tip_event;

    let wlr_tablet = (*event).tablet;
    let wlr_device = &mut (*wlr_tablet).base as *mut ffi::wlr_input_device;
    let device = ffi::river_wlr_input_device_get_data(wlr_device) as *mut crate::input_device::InputDevice;
    if device.is_null() {
        return;
    }
    let seat = (*device).seat;
    (*seat).handle_activity();

    let tablet = (*device).destroy_data as *mut crate::tablet::Tablet;
    if tablet.is_null() {
        return;
    }

    if let Ok(tool) = crate::tablet_tool::TabletTool::get((*seat).wlr_seat, (*event).tool) {
        (*tool).tip(tablet, event);
    }
}

unsafe extern "C" fn handle_tablet_tool_button(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let _cursor = &mut *crate::container_of!(listener, Cursor, tablet_tool_button_listener);
    let event = data as *mut ffi::wlr_tablet_tool_button_event;

    let wlr_tablet = (*event).tablet;
    let wlr_device = &mut (*wlr_tablet).base as *mut ffi::wlr_input_device;
    let device = ffi::river_wlr_input_device_get_data(wlr_device) as *mut crate::input_device::InputDevice;
    if device.is_null() {
        return;
    }
    let seat = (*device).seat;
    (*seat).handle_activity();

    let tablet = (*device).destroy_data as *mut crate::tablet::Tablet;
    if tablet.is_null() {
        return;
    }

    if let Ok(tool) = crate::tablet_tool::TabletTool::get((*seat).wlr_seat, (*event).tool) {
        (*tool).button(tablet, event);
    }
}

/// A directional swipe bind fires once the accumulated travel (libinput
/// units) passes `window_manager { swipe_threshold }`
/// (`WindowManager::swipe_threshold`, default 70). Until then the camera
/// *peeks*: it pans toward the swipe direction in proportion to the
/// travel, up to `window_manager { swipe_peek }` screen px
/// (`WindowManager::swipe_peek_px`, default 60) at the threshold, and
/// eases back if the fingers lift short of it — so a hesitant
/// three-finger swipe shows where it would go without going. Firing does
/// not end the swipe: the travel restarts from zero, and each further
/// `swipe_repeat_threshold` of travel (default four times the first) steps
/// again — or back, on a reversal — until the fingers lift.

#[cfg(test)]
mod tests {
    use super::*;

    // The swipe-lean tests live with the arithmetic, in cce_window_manager::gesture.

    fn bind(direction: &str, action: crate::config::Action) -> crate::config::GestureBind {
        crate::config::GestureBind {
            mods: 0,
            gesture_type: "swipe".into(),
            fingers: 3,
            direction: direction.into(),
            action,
            command: None,
        }
    }

    #[test]
    fn focus_vector_follows_natural_binds() {
        use crate::config::Action::*;
        let binds = [bind("left", FocusLeft), bind("right", FocusRight), bind("up", FocusUp), bind("down", FocusDown)];
        assert_eq!(swipe_focus_vector(&binds, 3, 0, (50.0, -30.0)), Some((50.0, -30.0)));
        // Another finger count or chord has no binds here.
        assert_eq!(swipe_focus_vector(&binds, 4, 0, (50.0, -30.0)), Option::None);
    }

    #[test]
    fn focus_vector_mirrors_mirrored_binds() {
        use crate::config::Action::*;
        let binds = [bind("left", FocusRight), bind("right", FocusLeft), bind("up", FocusDown), bind("down", FocusUp)];
        assert_eq!(swipe_focus_vector(&binds, 3, 0, (50.0, -30.0)), Some((-50.0, 30.0)));
    }

    #[test]
    fn focus_vector_ignores_an_axis_without_focus_binds() {
        use crate::config::Action::*;
        // Up/down pan rather than focus: a diagonal swipe aims sideways only.
        let binds = [bind("left", FocusLeft), bind("right", FocusRight), bind("up", PanUp), bind("down", PanDown)];
        assert_eq!(swipe_focus_vector(&binds, 3, 0, (50.0, -30.0)), Some((50.0, 0.0)));
        assert_eq!(swipe_focus_vector(&binds, 3, 0, (0.0, -30.0)), Option::None);
    }

}
