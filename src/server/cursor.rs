use crate::ffi;
use crate::seat::{Seat, Focus};
use crate::server::{WlListener, wl_listener_remove, wl_signal_add, WlList};
use crate::scene_node_data::SceneNodeDataVal;
use crate::drag_icon::DragIcon;
use std::collections::{HashMap, HashSet};

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

    pub motion_listener: ffi::wl_listener,
    pub motion_absolute_listener: ffi::wl_listener,
    pub button_listener: ffi::wl_listener,
    pub axis_listener: ffi::wl_listener,
    pub frame_listener: ffi::wl_listener,

    pub tablet_tool_axis_listener: ffi::wl_listener,
    pub tablet_tool_proximity_listener: ffi::wl_listener,
    pub tablet_tool_tip_listener: ffi::wl_listener,
    pub tablet_tool_button_listener: ffi::wl_listener,

    pub touch_points: HashMap<i32, (f64, f64)>,
    pub pressed: HashMap<u32, Option<*mut crate::pointer_binding::PointerBinding>>,
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

    pub touch_down_listener: ffi::wl_listener,
    pub touch_motion_listener: ffi::wl_listener,
    pub touch_up_listener: ffi::wl_listener,
    pub touch_cancel_listener: ffi::wl_listener,
    pub touch_frame_listener: ffi::wl_listener,

    pub swipe_begin_listener: ffi::wl_listener,
    pub swipe_update_listener: ffi::wl_listener,
    pub swipe_end_listener: ffi::wl_listener,

    pub pinch_begin_listener: ffi::wl_listener,
    pub pinch_update_listener: ffi::wl_listener,
    pub pinch_end_listener: ffi::wl_listener,

    pub hold_begin_listener: ffi::wl_listener,
    pub hold_end_listener: ffi::wl_listener,

    pub gesture_dx: f64,
    pub gesture_dy: f64,
    pub gesture_scale: f64,
    pub gesture_triggered: bool,
    /// Camera offset (virtual units, `[x, y]`) the in-flight swipe has
    /// peeked the desktop by so far — see `swipe_peek_for`. Zero outside a
    /// swipe, and once the swipe's bind has fired or the fingers lifted.
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
    /// Which of that window's 8 border zones is highlighted.
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
    pub left_click_on_bg_in_overview: bool,
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
            pressed: HashMap::new(),
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
            left_click_on_bg_in_overview: false,
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
        let motion_ptr = &mut self.motion_listener as *mut ffi::wl_listener as *mut WlListener;
        (*motion_ptr).notify = Some(handle_motion);
        wl_signal_add(
            ffi::river_wlr_cursor_get_motion_signal(wlr_cursor),
            &mut self.motion_listener,
        );

        let absolute_ptr = &mut self.motion_absolute_listener as *mut ffi::wl_listener as *mut WlListener;
        (*absolute_ptr).notify = Some(handle_motion_absolute);
        wl_signal_add(
            ffi::river_wlr_cursor_get_motion_absolute_signal(wlr_cursor),
            &mut self.motion_absolute_listener,
        );

        let button_ptr = &mut self.button_listener as *mut ffi::wl_listener as *mut WlListener;
        (*button_ptr).notify = Some(handle_button);
        wl_signal_add(
            ffi::river_wlr_cursor_get_button_signal(wlr_cursor),
            &mut self.button_listener,
        );

        let axis_ptr = &mut self.axis_listener as *mut ffi::wl_listener as *mut WlListener;
        (*axis_ptr).notify = Some(handle_axis);
        wl_signal_add(
            ffi::river_wlr_cursor_get_axis_signal(wlr_cursor),
            &mut self.axis_listener,
        );

        let frame_ptr = &mut self.frame_listener as *mut ffi::wl_listener as *mut WlListener;
        (*frame_ptr).notify = Some(handle_frame);
        wl_signal_add(
            ffi::river_wlr_cursor_get_frame_signal(wlr_cursor),
            &mut self.frame_listener,
        );

        let tablet_axis_ptr = &mut self.tablet_tool_axis_listener as *mut ffi::wl_listener as *mut WlListener;
        (*tablet_axis_ptr).notify = Some(handle_tablet_tool_axis);
        wl_signal_add(
            ffi::river_wlr_cursor_get_tablet_tool_axis_signal(wlr_cursor),
            &mut self.tablet_tool_axis_listener,
        );

        let tablet_proximity_ptr = &mut self.tablet_tool_proximity_listener as *mut ffi::wl_listener as *mut WlListener;
        (*tablet_proximity_ptr).notify = Some(handle_tablet_tool_proximity);
        wl_signal_add(
            ffi::river_wlr_cursor_get_tablet_tool_proximity_signal(wlr_cursor),
            &mut self.tablet_tool_proximity_listener,
        );

        let tablet_tip_ptr = &mut self.tablet_tool_tip_listener as *mut ffi::wl_listener as *mut WlListener;
        (*tablet_tip_ptr).notify = Some(handle_tablet_tool_tip);
        wl_signal_add(
            ffi::river_wlr_cursor_get_tablet_tool_tip_signal(wlr_cursor),
            &mut self.tablet_tool_tip_listener,
        );

        let tablet_button_ptr = &mut self.tablet_tool_button_listener as *mut ffi::wl_listener as *mut WlListener;
        (*tablet_button_ptr).notify = Some(handle_tablet_tool_button);
        wl_signal_add(
            ffi::river_wlr_cursor_get_tablet_tool_button_signal(wlr_cursor),
            &mut self.tablet_tool_button_listener,
        );

        let touch_down_ptr = &mut self.touch_down_listener as *mut ffi::wl_listener as *mut WlListener;
        (*touch_down_ptr).notify = Some(handle_touch_down);
        wl_signal_add(
            ffi::river_wlr_cursor_get_touch_down_signal(wlr_cursor),
            &mut self.touch_down_listener,
        );

        let touch_motion_ptr = &mut self.touch_motion_listener as *mut ffi::wl_listener as *mut WlListener;
        (*touch_motion_ptr).notify = Some(handle_touch_motion);
        wl_signal_add(
            ffi::river_wlr_cursor_get_touch_motion_signal(wlr_cursor),
            &mut self.touch_motion_listener,
        );

        let touch_up_ptr = &mut self.touch_up_listener as *mut ffi::wl_listener as *mut WlListener;
        (*touch_up_ptr).notify = Some(handle_touch_up);
        wl_signal_add(
            ffi::river_wlr_cursor_get_touch_up_signal(wlr_cursor),
            &mut self.touch_up_listener,
        );

        let touch_cancel_ptr = &mut self.touch_cancel_listener as *mut ffi::wl_listener as *mut WlListener;
        (*touch_cancel_ptr).notify = Some(handle_touch_cancel);
        wl_signal_add(
            ffi::river_wlr_cursor_get_touch_cancel_signal(wlr_cursor),
            &mut self.touch_cancel_listener,
        );

        let touch_frame_ptr = &mut self.touch_frame_listener as *mut ffi::wl_listener as *mut WlListener;
        (*touch_frame_ptr).notify = Some(handle_touch_frame);
        wl_signal_add(
            ffi::river_wlr_cursor_get_touch_frame_signal(wlr_cursor),
            &mut self.touch_frame_listener,
        );

        let swipe_begin_ptr = &mut self.swipe_begin_listener as *mut ffi::wl_listener as *mut WlListener;
        (*swipe_begin_ptr).notify = Some(handle_swipe_begin);
        wl_signal_add(
            ffi::river_wlr_cursor_get_swipe_begin_signal(wlr_cursor),
            &mut self.swipe_begin_listener,
        );

        let swipe_update_ptr = &mut self.swipe_update_listener as *mut ffi::wl_listener as *mut WlListener;
        (*swipe_update_ptr).notify = Some(handle_swipe_update);
        wl_signal_add(
            ffi::river_wlr_cursor_get_swipe_update_signal(wlr_cursor),
            &mut self.swipe_update_listener,
        );

        let swipe_end_ptr = &mut self.swipe_end_listener as *mut ffi::wl_listener as *mut WlListener;
        (*swipe_end_ptr).notify = Some(handle_swipe_end);
        wl_signal_add(
            ffi::river_wlr_cursor_get_swipe_end_signal(wlr_cursor),
            &mut self.swipe_end_listener,
        );

        let pinch_begin_ptr = &mut self.pinch_begin_listener as *mut ffi::wl_listener as *mut WlListener;
        (*pinch_begin_ptr).notify = Some(handle_pinch_begin);
        wl_signal_add(
            ffi::river_wlr_cursor_get_pinch_begin_signal(wlr_cursor),
            &mut self.pinch_begin_listener,
        );

        let pinch_update_ptr = &mut self.pinch_update_listener as *mut ffi::wl_listener as *mut WlListener;
        (*pinch_update_ptr).notify = Some(handle_pinch_update);
        wl_signal_add(
            ffi::river_wlr_cursor_get_pinch_update_signal(wlr_cursor),
            &mut self.pinch_update_listener,
        );

        let pinch_end_ptr = &mut self.pinch_end_listener as *mut ffi::wl_listener as *mut WlListener;
        (*pinch_end_ptr).notify = Some(handle_pinch_end);
        wl_signal_add(
            ffi::river_wlr_cursor_get_pinch_end_signal(wlr_cursor),
            &mut self.pinch_end_listener,
        );

        let hold_begin_ptr = &mut self.hold_begin_listener as *mut ffi::wl_listener as *mut WlListener;
        (*hold_begin_ptr).notify = Some(handle_hold_begin);
        wl_signal_add(
            ffi::river_wlr_cursor_get_hold_begin_signal(wlr_cursor),
            &mut self.hold_begin_listener,
        );

        let hold_end_ptr = &mut self.hold_end_listener as *mut ffi::wl_listener as *mut WlListener;
        (*hold_end_ptr).notify = Some(handle_hold_end);
        wl_signal_add(
            ffi::river_wlr_cursor_get_hold_end_signal(wlr_cursor),
            &mut self.hold_end_listener,
        );

        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        wl_listener_remove(&mut self.motion_listener);
        wl_listener_remove(&mut self.motion_absolute_listener);
        wl_listener_remove(&mut self.button_listener);
        wl_listener_remove(&mut self.axis_listener);
        wl_listener_remove(&mut self.frame_listener);

        wl_listener_remove(&mut self.tablet_tool_axis_listener);
        wl_listener_remove(&mut self.tablet_tool_proximity_listener);
        wl_listener_remove(&mut self.tablet_tool_tip_listener);
        wl_listener_remove(&mut self.tablet_tool_button_listener);

        wl_listener_remove(&mut self.touch_down_listener);
        wl_listener_remove(&mut self.touch_motion_listener);
        wl_listener_remove(&mut self.touch_up_listener);
        wl_listener_remove(&mut self.touch_cancel_listener);
        wl_listener_remove(&mut self.touch_frame_listener);

        wl_listener_remove(&mut self.swipe_begin_listener);
        wl_listener_remove(&mut self.swipe_update_listener);
        wl_listener_remove(&mut self.swipe_end_listener);

        wl_listener_remove(&mut self.pinch_begin_listener);
        wl_listener_remove(&mut self.pinch_update_listener);
        wl_listener_remove(&mut self.pinch_end_listener);

        wl_listener_remove(&mut self.hold_begin_listener);
        wl_listener_remove(&mut self.hold_end_listener);

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
        let drag_icons_tree = (*(*self.seat).server).scene.drag_icons;
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
                            BorderZone::None => {}
                        }
                    }
                }
                SceneNodeDataVal::ShellSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
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
        handle_frame(&mut self.frame_listener as *mut ffi::wl_listener, std::ptr::null_mut());
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
            &mut self.motion_listener as *mut ffi::wl_listener,
            &mut ev as *mut ffi::wlr_pointer_motion_event as *mut std::ffi::c_void,
        );
        handle_frame(&mut self.frame_listener as *mut ffi::wl_listener, std::ptr::null_mut());
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
            &mut self.button_listener as *mut ffi::wl_listener,
            &mut ev as *mut ffi::wlr_pointer_button_event as *mut std::ffi::c_void,
        );
        handle_frame(&mut self.frame_listener as *mut ffi::wl_listener, std::ptr::null_mut());
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
                &mut self.axis_listener as *mut ffi::wl_listener,
                &mut ev as *mut ffi::wlr_pointer_axis_event as *mut std::ffi::c_void,
            );
        }
        handle_frame(&mut self.frame_listener as *mut ffi::wl_listener, std::ptr::null_mut());
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
                &mut self.axis_listener as *mut ffi::wl_listener,
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
                handle_pinch_begin(&mut self.pinch_begin_listener as *mut ffi::wl_listener, &mut ev as *mut _ as *mut std::ffi::c_void);
            }
            "update" => {
                let mut ev = ffi::wlr_pointer_pinch_update_event { pointer: std::ptr::null_mut(), time_msec: time, fingers: 2, dx: 0.0, dy: 0.0, scale, rotation };
                handle_pinch_update(&mut self.pinch_update_listener as *mut ffi::wl_listener, &mut ev as *mut _ as *mut std::ffi::c_void);
            }
            _ => {
                let mut ev = ffi::wlr_pointer_pinch_end_event { pointer: std::ptr::null_mut(), time_msec: time, cancelled: false };
                handle_pinch_end(&mut self.pinch_end_listener as *mut ffi::wl_listener, &mut ev as *mut _ as *mut std::ffi::c_void);
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
                handle_swipe_begin(&mut self.swipe_begin_listener as *mut ffi::wl_listener, &mut ev as *mut _ as *mut std::ffi::c_void);
            }
            "update" => {
                let fingers = self.inject_swipe_fingers;
                let mut ev = ffi::wlr_pointer_swipe_update_event { pointer: std::ptr::null_mut(), time_msec: time, fingers, dx, dy };
                handle_swipe_update(&mut self.swipe_update_listener as *mut ffi::wl_listener, &mut ev as *mut _ as *mut std::ffi::c_void);
            }
            _ => {
                let mut ev = ffi::wlr_pointer_swipe_end_event { pointer: std::ptr::null_mut(), time_msec: time, cancelled: false };
                handle_swipe_end(&mut self.swipe_end_listener as *mut ffi::wl_listener, &mut ev as *mut _ as *mut std::ffi::c_void);
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
            &mut self.swipe_begin_listener as *mut ffi::wl_listener,
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
                &mut self.swipe_update_listener as *mut ffi::wl_listener,
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
            &mut self.swipe_end_listener as *mut ffi::wl_listener,
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
            &mut self.pinch_begin_listener as *mut ffi::wl_listener,
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
                &mut self.pinch_update_listener as *mut ffi::wl_listener,
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
            &mut self.pinch_end_listener as *mut ffi::wl_listener,
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

unsafe extern "C" fn handle_button(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, button_listener);
    let event = data as *mut ffi::wlr_pointer_button_event;
    (*cursor.seat).handle_activity();
    if cursor.view_drag.is_some() {
        cursor.end_view_drag("button");
    }
    // Before the button reaches the client, so a click on the popup is not
    // a Ctrl-click (see `PopupWheel`).
    cursor.end_popup_wheel("button");
    cursor.end_hscroll_shift("button");
    
    let seat = &mut *cursor.seat;
    let lx = cursor.x();
    let ly = cursor.y();
    let server = seat.server;

    // First deliberate input ends the session-restore settling phase (see
    // the focus gate in Window::map).
    if (*event).state == ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED {
        (*server).wm.startup_input_seen = true;
    }

    let mut is_app_surface = false;
    let mut is_overlay_window = false;
    if let Some(result) = (*server).scene.at(lx, ly) {
        match result.data {
            SceneNodeDataVal::Window(window) => {
                // The grid does not block: hitting it means the press is on a
                // desktop item (its input region covers nothing else), and
                // the item drag needs the press delivered in overview like
                // any chrome click.
                if !(*window).is_status_bar()
                    && !(*window).is_wallpaper()
                    && !(*window).is_grid()
                {
                    is_app_surface = true;
                    // Popup counts as chrome like Overlay: the cce-cloud
                    // launcher must keep receiving clicks in overview.
                    if (*window).tiling_mode == crate::tiling::TilingMode::Overlay
                        || (*window).tiling_mode == crate::tiling::TilingMode::Popup
                    {
                        is_overlay_window = true;
                    }
                }
            }
            SceneNodeDataVal::LayerSurface(layer_surface) => {
                if !layer_surface.is_null() {
                    is_app_surface = true;
                    if is_cloud_layer(layer_surface) {
                        is_overlay_window = true;
                    }
                }
            }
            SceneNodeDataVal::ShellSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                is_app_surface = true;
            }
            _ => {}
        }
    }
    let should_block_button = (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview && is_app_surface && !is_overlay_window;
    
    if (*event).state == ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED {
        if cursor.pressed.contains_key(&(*event).button) {
            log::error!("ignoring duplicate pointer button {} press", (*event).button);
            return;
        }

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

        // Status-bar segments are dragged either in adjust-position mode or
        // directly with super+left-drag (0x40 = WLR_MODIFIER_LOGO).
        let super_held = {
            let wlr_keyboard = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
            !wlr_keyboard.is_null() && (ffi::wlr_keyboard_get_modifiers(wlr_keyboard) & 0x40) != 0
        };
        if (*event).button == 0x110 && ((*(*seat).server).wm.adjust_position_mode || super_held) {
            let mut clicked_status: *mut crate::window::Window = std::ptr::null_mut();
            if let Some(result) = (*server).scene.at(lx, ly) {
                if let SceneNodeDataVal::Window(window) = result.data {
                    if (*window).is_status_bar() {
                        clicked_status = window;
                    }
                }
            }
            // An EXPANDED segment (menu open) is never grabbed: its rows are
            // clicked, and a grab here would swallow the press the "Done"
            // row needs to leave adjust mode — the one control that ends the
            // mode from the bar would be unreachable while it is on.
            if !clicked_status.is_null() && !(*server).wm.is_expanded_status_segment(clicked_status) {
                (*server).wm.stop_panning_animation();
                let cursor_x = (*cursor.wlr_cursor).x;
                let cursor_y = (*cursor.wlr_cursor).y;
                seat.op = Some(crate::seat::SeatOp {
                    sent_release: false,
                    input: crate::seat::SeatOpInput::Pointer,
                    start_x: cursor_x as i32,
                    start_y: cursor_y as i32,
                    x: cursor_x as i32,
                    y: cursor_y as i32,
                    window_ptr: clicked_status,
                    op_type: crate::seat::PointerOpType::Move,
                    start_win_x: (*clicked_status).box_geom.x,
                    start_win_y: (*clicked_status).box_geom.y,
                    start_win_w: (*clicked_status).box_geom.width as u32,
                    start_win_h: (*clicked_status).box_geom.height as u32,
                    start_win_virtual_x: (*clicked_status).virtual_x,
                    start_win_virtual_y: (*clicked_status).virtual_y,
                    start_was_tiled: false,
                    start_pan_x: (*server).wm.desk_pan_x,
                    start_pan_y: (*server).wm.desk_pan_y,
                    start_tiling_mode: (*clicked_status).tiling_mode,
                    start_mode_locked: (*clicked_status).mode_locked,
                    started_in_overview: false,
                });
                cursor.op_start_pointer();
                cursor.pressed.insert((*event).button, None);
                cursor.set_xcursor(b"grab\0".as_ptr() as *const _);
                return;
            }
        }

        // --- ADJUST-MODE CLICK HANDLING (overview, or Super held) ---
        // A press on a window's body grabs the whole window to move it; a
        // press on its ring falls through to the border path. Only in
        // overview does a background press mean anything (it exits); with
        // Super held at zoom 1 it falls through to the normal desktop press.
        let in_overview = (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview;
        if (*event).button == 0x110 && (*(*seat).server).wm.window_adjust_active() {
            let mut clicked_win: *mut crate::window::Window = std::ptr::null_mut();
            let mut clicked_cloud_layer = false;
            if let Some(result) = (*server).scene.at(lx, ly) {
                match result.data {
                    SceneNodeDataVal::Window(window) => clicked_win = window,
                    SceneNodeDataVal::LayerSurface(layer_surface) => {
                        clicked_cloud_layer = is_cloud_layer(layer_surface);
                    }
                    _ => {}
                }
            }

            // A press on the border band falls through to the normal
            // border path below (move/resize by zone, zoom-aware) — the
            // resize controls work at any zoom. Content presses grab the
            // whole window; true background presses exit overview.
            //
            // Chrome windows (Overlay docks, Popup surfaces like the
            // cce-cloud launcher) and cce-cloud layer surfaces (launcher,
            // desktop/app context menus) are neither: they stay
            // interactive UI during overview, so their presses fall
            // through to the normal path (focus + delivery to the app)
            // and overview stays up.
            let overview_chrome = clicked_cloud_layer
                || (!clicked_win.is_null()
                    && matches!(
                        (*clicked_win).tiling_mode,
                        crate::tiling::TilingMode::Overlay | crate::tiling::TilingMode::Popup
                    ));
            // Hitting the grid means the press landed on a DESKTOP ITEM —
            // its input region covers the item rects and nothing else — so
            // it is chrome-like: fall through to normal delivery and the
            // grid client starts its item drag, in overview exactly as in
            // normal mode. Bare canvas misses the grid entirely (that is
            // the input region again) and still exits overview below.
            let clicked_grid = !clicked_win.is_null() && (*clicked_win).is_grid();
            // The bare canvas counts as background in overview: a press on
            // it must exit overview like any desktop press, never grab the
            // canvas itself as if it were a window.
            let overview_win_valid = !clicked_win.is_null()
                && !(*clicked_win).is_status_bar()
                && !(*clicked_win).is_wallpaper()
                && !clicked_grid;
            let overview_border_zone = if overview_win_valid {
                get_border_zone(clicked_win, lx, ly)
            } else {
                BorderZone::None
            };
            if overview_chrome || clicked_grid {
                // fall through
            } else if overview_win_valid && matches!(overview_border_zone, BorderZone::None) {
                // The grab does NOT focus the window: moving a window is
                // not choosing it, and the ring already sits on it through
                // `adjust_hover`. A tap — press and release without motion
                // — is a click and focuses in `op_end`; a Floating window
                // is raised for the drag in `op_start_pointer`. (In
                // overview hover already focused it.)
                (*server).wm.stop_panning_animation();
                let cursor_x = (*cursor.wlr_cursor).x;
                let cursor_y = (*cursor.wlr_cursor).y;
                seat.op = Some(crate::seat::SeatOp {
                    sent_release: false,
                    input: crate::seat::SeatOpInput::Pointer,
                    start_x: cursor_x as i32,
                    start_y: cursor_y as i32,
                    x: cursor_x as i32,
                    y: cursor_y as i32,
                    window_ptr: clicked_win,
                    op_type: crate::seat::PointerOpType::Move,
                    start_win_x: (*clicked_win).box_geom.x,
                    start_win_y: (*clicked_win).box_geom.y,
                    start_win_w: (*clicked_win).box_geom.width as u32,
                    start_win_h: (*clicked_win).box_geom.height as u32,
                    start_win_virtual_x: (*clicked_win).virtual_x,
                    start_win_virtual_y: (*clicked_win).virtual_y,
                    start_was_tiled: (*clicked_win).tiling_mode == crate::tiling::TilingMode::Tiled,
                    start_pan_x: (*server).wm.desk_pan_x,
                    start_pan_y: (*server).wm.desk_pan_y,
                    start_tiling_mode: (*clicked_win).tiling_mode,
                    start_mode_locked: (*clicked_win).mode_locked,
                    started_in_overview: in_overview,
                });
                cursor.op_start_pointer();
                cursor.pressed.insert((*event).button, None);
                cursor.set_xcursor(b"grab\0".as_ptr() as *const _);
                return;
            } else if !overview_win_valid && in_overview {
                // Click-away with a cce-cloud popup open (desktop context
                // menu, launcher): the press dismisses the popup and does
                // nothing else — overview stays up. Dropping keyboard focus
                // IS the dismissal: cce-cloud closes itself on keyboard
                // leave, the same signal a normal-mode click-away produces
                // through its focus change.
                if let crate::layer_shell::LayerShellSeatFocus::Exclusive(key) =
                    seat.layer_shell.scheduled_focus
                {
                    if let Some(&layer_surface) = (*server).layer_shell.surfaces.get(key) {
                        if is_cloud_layer(layer_surface) {
                            seat.focus(Focus::None);
                            cursor.pressed.insert((*event).button, None);
                            return;
                        }
                    }
                }
                cursor.left_click_on_bg_in_overview = true;
                (*server).wm.execute_action(&crate::config::Action::Overview, None);
                cursor.pressed.insert((*event).button, None);
                return;
            }
        }

        let wlr_keyboard = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
        let modifiers = if !wlr_keyboard.is_null() {
            ffi::wlr_keyboard_get_modifiers(wlr_keyboard)
        } else {
            0
        };

        if (*event).button == 0x111 && modifiers == 0 {
            let mut clicked_interactive = false;
            if let Some(result) = (*server).scene.at(lx, ly) {
                match result.data {
                    SceneNodeDataVal::Window(window) => {
                        if !(*window).is_wallpaper() {
                            clicked_interactive = true;
                        }
                    }
                    SceneNodeDataVal::LayerSurface(_) | SceneNodeDataVal::ShellSurface(_) | SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                        clicked_interactive = true;
                    }
                }
            }
            if !clicked_interactive {
                cursor.right_click_on_bg = true;
                let x = cursor.x() as i32;
                let y = cursor.y() as i32;
                let home = std::env::var("HOME").unwrap_or_default();
                let cmd = format!("{}/.local/bin/cce-desktop-menu -x {} -y {}", home, x, y);
                (*server).wm.execute_action(&crate::config::Action::Spawn, Some(&cmd));

                seat.focus(Focus::None);
                (*(*seat).server).wm.dirty_windowing();

                cursor.pressed.insert((*event).button, None);
                return;
            }
        }
        
        let mut matched_pb: Option<crate::config::PointerBind> = None;
        for pb in &(*(*seat).server).wm.pointer_binds {
            if pb.button == (*event).button && pb.mods == modifiers {
                matched_pb = Some(pb.clone());
                break;
            }
        }
        
        if let Some(pb) = matched_pb {
            let lx = cursor.x();
            let ly = cursor.y();
            let server = seat.server;
            let mut target_win: *mut crate::window::Window = std::ptr::null_mut();
            if let Some(result) = (*server).scene.at(lx, ly) {
                if let SceneNodeDataVal::Window(window) = result.data {
                    target_win = window;
                }
            }
            
            if !target_win.is_null() && !(*target_win).is_status_bar() && !(*target_win).is_wallpaper() {
                // Before the un-tile below: a tiled window's drag snaps hard
                // to whole squares, and by op_update the mode reads Floating.
                let grabbed_tiled =
                    (*target_win).tiling_mode == crate::tiling::TilingMode::Tiled;
                if (*target_win).tiling_mode != crate::tiling::TilingMode::Floating
                    && (*target_win).tiling_mode != crate::tiling::TilingMode::Popup
                    && (*target_win).tiling_mode != crate::tiling::TilingMode::Fullscreen
                    && (*target_win).tiling_mode != crate::tiling::TilingMode::Overlay
                    // A drag moves a Utility window; it must not re-class it.
                    && (*target_win).tiling_mode != crate::tiling::TilingMode::Utility
                {
                    // A tiled window un-tiles for the drag but keeps its
                    // cell-quantized geometry; landing grid-aligned re-tiles
                    // it (op_end geometric detection).
                    (*target_win).was_tiled = false;
                    (*target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                    (*target_win).mode_locked = true;
                }
                // No focus on the grab: a bound move/resize drag acts on
                // the window under the pointer without choosing it (a tap
                // focuses in `op_end`, the raise is in `op_start_pointer`).
                let op_type = match pb.action {
                    crate::config::Action::Move => Some(crate::seat::PointerOpType::Move),
                    // The modifier binding is a resize path the border zones
                    // never see, so it carries its own Utility rejection.
                    crate::config::Action::Resize
                        if (*target_win).tiling_mode == crate::tiling::TilingMode::Utility =>
                    {
                        None
                    }
                    crate::config::Action::Resize => {
                        let edges = get_closest_edges(target_win, lx, ly);
                        Some(crate::seat::PointerOpType::Resize { edges })
                    }
                    _ => None,
                };
                
                if let Some(ot) = op_type {
                    (*server).wm.stop_panning_animation();
                    let cursor_x = (*cursor.wlr_cursor).x;
                    let cursor_y = (*cursor.wlr_cursor).y;
                    seat.op = Some(crate::seat::SeatOp {
                        sent_release: false,
                        input: crate::seat::SeatOpInput::Pointer,
                        start_x: cursor_x as i32,
                        start_y: cursor_y as i32,
                        x: cursor_x as i32,
                        y: cursor_y as i32,
                        window_ptr: target_win,
                        op_type: ot,
                        start_win_x: (*target_win).box_geom.x,
                        start_win_y: (*target_win).box_geom.y,
                        start_win_w: (*target_win).box_geom.width as u32,
                        start_win_h: (*target_win).box_geom.height as u32,
                        start_win_virtual_x: (*target_win).virtual_x,
                        start_win_virtual_y: (*target_win).virtual_y,
                        start_tiling_mode: (*target_win).tiling_mode,
                        start_was_tiled: grabbed_tiled,
                        start_mode_locked: (*target_win).mode_locked,
                        start_pan_x: (*(*seat).server).wm.desk_pan_x,
                        start_pan_y: (*(*seat).server).wm.desk_pan_y,
                        started_in_overview: (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview,
                    });
                    cursor.op_start_pointer();
                    cursor.pressed.insert((*event).button, None);

                    match ot {
                        crate::seat::PointerOpType::Resize { edges } => {
                            let cursor_name = get_resize_cursor_name(edges);
                            cursor.set_xcursor(cursor_name.as_ptr() as *const _);
                        }
                        crate::seat::PointerOpType::Move => {
                            cursor.set_xcursor(b"grab\0".as_ptr() as *const _);
                        }
                    }
                    return;
                }
            }
        }

        let lx = cursor.x();
        let ly = cursor.y();
        let server = seat.server;
        let mut border_target_win: *mut crate::window::Window = std::ptr::null_mut();
        if let Some(result) = (*server).scene.at(lx, ly) {
            if let SceneNodeDataVal::Window(window) = result.data {
                border_target_win = window;
            }
        }

        if !border_target_win.is_null() && !(*border_target_win).is_status_bar() && !(*border_target_win).is_wallpaper() && !(*border_target_win).is_grid() && (
            (*border_target_win).tiling_mode != crate::tiling::TilingMode::Popup
            && (*border_target_win).tiling_mode != crate::tiling::TilingMode::Fullscreen
        ) {
            let initial_mode = (*border_target_win).tiling_mode;
            let zone = get_border_zone(border_target_win, lx, ly);
            // The window context menu (`scripts/cce-app-menu`, a cce-cloud
            // popup like the desktop menu and cce-grid's item menu). A
            // right-click on a handle disc opens it in either adjust mode;
            // in OVERVIEW a right-click anywhere on the window does: the
            // client never sees buttons there (`should_block_button`), so
            // the press is the compositor's to spend, and the menu is how a
            // window's mode is set from the overview. Overlay docks are
            // chrome and keep their clicks (`overview_chrome` above);
            // Utility windows take no handles and have no mode to set.
            let menu_on_body = in_overview
                && !matches!(
                    initial_mode,
                    crate::tiling::TilingMode::Overlay | crate::tiling::TilingMode::Utility
                );
            if (*event).button == 0x111
                && modifiers == 0
                && (!matches!(zone, BorderZone::None) || menu_on_body)
            {
                cursor.right_click_on_border = true;
                let x = cursor.x() as i32;
                let y = cursor.y() as i32;
                let index = (*border_target_win).ref_key.index;
                let app_id = (*border_target_win).get_app_id_string().unwrap_or_else(|| "unknown".to_string());
                // The command runs under `sh -c`: quote the app_id, which
                // is client-chosen text.
                let app_id = format!("'{}'", app_id.replace('\'', "'\\''"));
                let home = std::env::var("HOME").unwrap_or_default();
                let cmd = format!("{}/.local/bin/cce-app-menu -x {} -y {} -i {} -a {}", home, x, y, index, app_id);
                (*server).wm.execute_action(&crate::config::Action::Spawn, Some(&cmd));

                cursor.pressed.insert((*event).button, None);
                return;
            }

            match zone {
                BorderZone::Resize(edges) => {
                    if (*event).button == 0x110 { // BTN_LEFT
                        // Unreachable for Utility (get_border_zone maps its
                        // whole band to Move), and gated here regardless.
                        if initial_mode == crate::tiling::TilingMode::Utility {
                            return;
                        }
                        if initial_mode != crate::tiling::TilingMode::Floating {
                            // A tiled window un-tiles for the drag but keeps its
                            // cell-quantized geometry; landing grid-aligned re-tiles
                            // it (op_end geometric detection).
                            (*border_target_win).was_tiled = false;
                            (*border_target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                            (*border_target_win).mode_locked = true;
                        }

                        // No focus on the grab (see the body grab above).
                        (*server).wm.stop_panning_animation();
                        let cursor_x = (*cursor.wlr_cursor).x;
                        let cursor_y = (*cursor.wlr_cursor).y;
                        seat.op = Some(crate::seat::SeatOp {
                            sent_release: false,
                            input: crate::seat::SeatOpInput::Pointer,
                            start_x: cursor_x as i32,
                            start_y: cursor_y as i32,
                            x: cursor_x as i32,
                            y: cursor_y as i32,
                            window_ptr: border_target_win,
                            op_type: crate::seat::PointerOpType::Resize { edges },
                            start_win_x: (*border_target_win).box_geom.x,
                            start_win_y: (*border_target_win).box_geom.y,
                            start_win_w: (*border_target_win).box_geom.width as u32,
                            start_win_h: (*border_target_win).box_geom.height as u32,
                            start_win_virtual_x: (*border_target_win).virtual_x,
                            start_win_virtual_y: (*border_target_win).virtual_y,
                            start_tiling_mode: (*border_target_win).tiling_mode,
                            start_was_tiled: initial_mode == crate::tiling::TilingMode::Tiled,
                            start_mode_locked: (*border_target_win).mode_locked,
                            start_pan_x: (*(*seat).server).wm.desk_pan_x,
                        start_pan_y: (*(*seat).server).wm.desk_pan_y,
                        started_in_overview: (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview,
                        });
                        cursor.op_start_pointer();
                        cursor.pressed.insert((*event).button, None);

                        let cursor_name = get_resize_cursor_name(edges);
                        cursor.set_xcursor(cursor_name.as_ptr() as *const _);
                        return;
                    }
                }
                BorderZone::Move => {
                    if (*event).button == 0x110 { // BTN_LEFT
                        let current_time = (*event).time_msec;
                        let is_titlebar = ly < (*border_target_win).box_geom.y as f64;
                        let is_double_click = is_titlebar
                            && border_target_win == cursor.last_click_window
                            && current_time.saturating_sub(cursor.last_click_time) < 300;

                        cursor.last_click_time = current_time;
                        cursor.last_click_window = border_target_win;

                        // A Utility window has no Tiled state to toggle;
                        // the double-click falls through to an ordinary move.
                        if is_double_click && initial_mode != crate::tiling::TilingMode::Utility {
                            if initial_mode == crate::tiling::TilingMode::Tiled {
                                (*border_target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                                (*border_target_win).mode_locked = true;
                            } else {
                                (*border_target_win).tiling_mode = crate::tiling::TilingMode::Tiled;
                                (*border_target_win).mode_locked = true;
                            }
                            seat.focus(Focus::Window(border_target_win));
                            (*server).wm.dirty_windowing();
                            cursor.last_click_time = 0;
                            cursor.last_click_window = std::ptr::null_mut();
                            return;
                        }

                        if initial_mode != crate::tiling::TilingMode::Floating
                            && initial_mode != crate::tiling::TilingMode::Overlay
                            // A drag moves a Utility window; it must not
                            // re-class it.
                            && initial_mode != crate::tiling::TilingMode::Utility
                        {
                            // A tiled window un-tiles for the drag but keeps its
                            // cell-quantized geometry; landing grid-aligned re-tiles
                            // it (op_end geometric detection).
                            (*border_target_win).was_tiled = false;
                            (*border_target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                            (*border_target_win).mode_locked = true;
                        }

                        // No focus on the grab (see the body grab above).
                        (*server).wm.stop_panning_animation();
                        let cursor_x = (*cursor.wlr_cursor).x;
                        let cursor_y = (*cursor.wlr_cursor).y;
                        seat.op = Some(crate::seat::SeatOp {
                            sent_release: false,
                            input: crate::seat::SeatOpInput::Pointer,
                            start_x: cursor_x as i32,
                            start_y: cursor_y as i32,
                            x: cursor_x as i32,
                            y: cursor_y as i32,
                            window_ptr: border_target_win,
                            op_type: crate::seat::PointerOpType::Move,
                            start_win_x: (*border_target_win).box_geom.x,
                            start_win_y: (*border_target_win).box_geom.y,
                            start_win_w: (*border_target_win).box_geom.width as u32,
                            start_win_h: (*border_target_win).box_geom.height as u32,
                            start_win_virtual_x: (*border_target_win).virtual_x,
                            start_win_virtual_y: (*border_target_win).virtual_y,
                            start_tiling_mode: (*border_target_win).tiling_mode,
                            start_was_tiled: initial_mode == crate::tiling::TilingMode::Tiled,
                            start_mode_locked: (*border_target_win).mode_locked,
                            start_pan_x: (*(*seat).server).wm.desk_pan_x,
                        start_pan_y: (*(*seat).server).wm.desk_pan_y,
                        started_in_overview: (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview,
                        });
                        cursor.op_start_pointer();
                        cursor.pressed.insert((*event).button, None);

                        cursor.set_xcursor(b"grab\0".as_ptr() as *const _);
                        return;
                    }
                }
                BorderZone::None => {}
            }
        }

        if let Some(binding) = seat.match_pointer_binding((*event).button) {
            cursor.pressed.insert((*event).button, Some(binding));
            (*binding).pressed();
            return;
        }

        cursor.pressed.insert((*event).button, None);

        if !should_block_button {
            let first_grab_button = cursor.notified_pressed.is_empty();
            ffi::wlr_seat_pointer_notify_button(
                seat.wlr_seat,
                (*event).time_msec,
                (*event).button,
                (*event).state,
            );
            cursor.notified_pressed.insert((*event).button);
            // First grab button: record the pressed surface's layout origin
            // as the implicit grab's frame of reference (passthrough keeps
            // motion surface-relative through it while the button is held).
            if first_grab_button {
                let glx = cursor.x();
                let gly = cursor.y();
                if let Some(result) = (*seat.server).scene.at(glx, gly) {
                    // A surface node's scene buffer begins with its node.
                    let mut ratio = 1.0;
                    if !result.surface.is_null() && !result.node.is_null() {
                        let dest_w = ffi::river_scene_buffer_get_dest_width(result.node as *mut ffi::wlr_scene_buffer);
                        let surf_w = ffi::river_wlr_surface_get_width(result.surface);
                        if dest_w > 0 && surf_w > 0 {
                            ratio = surf_w as f64 / dest_w as f64;
                        }
                    }
                    cursor.grab_scale = ratio;
                    cursor.grab_origin = (glx - result.sx / ratio, gly - result.sy / ratio);
                }
                // A grab that starts on the grid freezes its node mapping
                // here instead of using grab_origin: passthrough maps motion
                // against these values for the whole grab, so camera motion
                // (which moves the node every frame) reads as nothing rather
                // than as pointer motion. See the grab branch in passthrough.
                cursor.grab_grid = None;
                let grab_focused =
                    ffi::river_wlr_seat_get_pointer_focused_surface(seat.wlr_seat);
                if !grab_focused.is_null() {
                    if let Some((gsurf, nx, ny, scale, _, _)) = grid_node_info(seat.server) {
                        if gsurf == grab_focused {
                            cursor.grab_grid = Some((nx, ny, scale));
                        }
                    }
                }
            }
        }

        // If pressed, update focus to window under cursor
        let lx = cursor.x();
        let ly = cursor.y();
        let server = seat.server;
        let mut clicked_something = false;
        if let Some(result) = (*server).scene.at(lx, ly) {
            match result.data {
                SceneNodeDataVal::Window(window) => {
                    clicked_something = true;
                    if !(*window).is_status_bar() && !(*window).is_wallpaper() {
                        if !seat.object.is_null() {
                            if !(*window).object.is_null() {
                                ffi::wl_resource_post_event(seat.object, 4, (*window).object);
                                (*(*seat).server).wm.dirty_windowing();
                            }
                        } else {
                            seat.focus(Focus::Window(window));
                        }
                    }
                }
                SceneNodeDataVal::LayerSurface(_) => {
                    clicked_something = true;
                    seat.focus(Focus::LayerSurface(result.surface));
                }
                SceneNodeDataVal::ShellSurface(_) | SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                    clicked_something = true;
                }
            }
        }

        if !clicked_something && (*event).button == 0x110 {
            // Restore placeholders are bare scene rects the hit-test can't
            // see, but they stand in for restored windows — clicking one
            // gets the same camera rules as clicking the real window.
            let wm = &mut (*server).wm;
            if let Some((pvx, pvy, pw, ph)) = wm.placeholder_at(lx, ly) {
                wm.pan_to_virtual_rect(pvx, pvy, pw, ph);
            } else {
                seat.focus(Focus::None);
                (*(*seat).server).wm.dirty_windowing();
            }
        }
    } else {
        assert_eq!((*event).state, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_RELEASED);
        // Pair the release for the client BEFORE any compositor-side
        // consumption below: if the press was forwarded, the release always
        // is too, or the client is left with an orphaned press.
        if cursor.notified_pressed.remove(&(*event).button) {
            ffi::wlr_seat_pointer_notify_button(
                seat.wlr_seat,
                (*event).time_msec,
                (*event).button,
                (*event).state,
            );
        }
        // The implicit grab ends with its last button; the frozen grid
        // mapping must not outlive it into the next grab.
        if cursor.notified_pressed.is_empty() {
            cursor.grab_grid = None;
        }
        if seat.op.is_some() {
            let cursor_x = (*cursor.wlr_cursor).x;
            let cursor_y = (*cursor.wlr_cursor).y;
            seat.op_update(cursor_x as i32, cursor_y as i32);
            
            let op = seat.op.unwrap();

            #[allow(unused_assignments)]
            if !op.window_ptr.is_null()
                && (*op.window_ptr).is_status_bar()
                && op.op_type == crate::seat::PointerOpType::Move
                && (*event).button == 0x110
            {
                let win = op.window_ptr;
                let app_id = (*win).get_app_id_string().unwrap_or_default();

                // A press that never travelled is a CLICK, not a drag: end
                // the grab without snapping — snapping classifies the
                // release point alone, so a still click on a top-edge
                // segment away from the corners re-homed it to top-center
                // — and replay press+release to the segment, which never
                // saw the press. The grab is taken on press so drag
                // feedback is immediate; this is where the two are told
                // apart.
                const STATUS_CLICK_TRAVEL: f64 = 6.0;
                let travel = (lx - op.start_x as f64).hypot(ly - op.start_y as f64);
                if travel < STATUS_CLICK_TRAVEL {
                    log::info!("[StatusRelease] click (travel {:.1}px) on app_id={} — replayed, not snapped", travel, app_id);
                    seat.op_end();
                    cursor.pressed.remove(&(*event).button);
                    let time = (*event).time_msec;
                    // Re-evaluate pointer focus onto the surface under the
                    // pointer (nothing was notified during the grab), then
                    // deliver the click.
                    cursor.passthrough(time);
                    ffi::wlr_seat_pointer_notify_button(seat.wlr_seat, time, (*event).button, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED);
                    ffi::wlr_seat_pointer_notify_frame(seat.wlr_seat);
                    ffi::wlr_seat_pointer_notify_button(seat.wlr_seat, time, (*event).button, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_RELEASED);
                    ffi::wlr_seat_pointer_notify_frame(seat.wlr_seat);
                    (*server).wm.dirty_windowing();
                    return;
                }

                let mut closest_edge = crate::window::StatusEdge::TopLeft;
                let mut min_dist = f64::MAX;
                log::info!("[StatusRelease] Released status window: app_id={}, lx={}, ly={}", app_id, lx, ly);
                
                let outputs_list = &mut (*server).om.outputs as *mut ffi::wl_list as *mut WlList;
                let mut curr_out = (*outputs_list).next;
                let mut best_output: *mut crate::output::Output = std::ptr::null_mut();
                let mut min_output_dist = f64::MAX;
                
                while curr_out != outputs_list {
                    let output = crate::container_of!(curr_out, crate::output::Output, link);
                    if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                        let wlr_box = (*output).sent.box_layout();
                        let ox = wlr_box.x as f64;
                        let oy = wlr_box.y as f64;
                        let ow = wlr_box.width as f64;
                        let oh = wlr_box.height as f64;
                        
                        let clamp = |val: f64, min: f64, max: f64| {
                            if val < min { min } else if val > max { max } else { val }
                        };
                        let cx = clamp(lx, ox, ox + ow);
                        let cy = clamp(ly, oy, oy + oh);
                        let dx = lx - cx;
                        let dy = ly - cy;
                        let dist = dx * dx + dy * dy;
                        if dist < min_output_dist {
                            min_output_dist = dist;
                            best_output = output;
                        }
                    }
                    curr_out = (*curr_out).next;
                }

                let mut found_out = false;
                if !best_output.is_null() {
                    found_out = true;
                    let wlr_box = (*best_output).sent.box_layout();
                    let ox = wlr_box.x as f64;
                    let oy = wlr_box.y as f64;
                    let ow = wlr_box.width as f64;
                    let oh = wlr_box.height as f64;

                    log::info!("[StatusRelease] Checking best output: box_geom=({}, {}, {}, {})", ox, oy, ow, oh);
                    
                    let dt = ly - oy;
                    let db = (oy + oh) - ly;
                    let dl = lx - ox;
                    let dr = (ox + ow) - lx;
                    
                    log::info!("[StatusRelease] Distances: top={}, bottom={}, left={}, right={}", dt, db, dl, dr);
                    enum EdgeBasic { Top, Bottom, Left, Right }
                    let mut edge = EdgeBasic::Top;
                    if dt < min_dist { min_dist = dt; edge = EdgeBasic::Top; }
                    if db < min_dist { min_dist = db; edge = EdgeBasic::Bottom; }
                    if dl < min_dist { min_dist = dl; edge = EdgeBasic::Left; }
                    if dr < min_dist { min_dist = dr; edge = EdgeBasic::Right; }

                    let corner_threshold = 120.0;
                    let is_near_top = ly < oy + corner_threshold;
                    let is_near_bottom = ly > oy + oh - corner_threshold;
                    let is_near_left = lx < ox + corner_threshold;
                    let is_near_right = lx > ox + ow - corner_threshold;

                    let semicircle_centers = [
                        (crate::window::StatusEdge::TopLeft, ox + 60.0, oy + 0.0),
                        (crate::window::StatusEdge::TopCenter, ox + ow / 2.0, oy + 0.0),
                        (crate::window::StatusEdge::TopRight, ox + ow - 60.0, oy + 0.0),
                        (crate::window::StatusEdge::BottomLeft, ox + 60.0, oy + oh),
                        (crate::window::StatusEdge::BottomCenter, ox + ow / 2.0, oy + oh),
                        (crate::window::StatusEdge::BottomRight, ox + ow - 60.0, oy + oh),
                        (crate::window::StatusEdge::Left, ox + 0.0, oy + oh / 2.0),
                        (crate::window::StatusEdge::Right, ox + ow, oy + oh / 2.0),
                    ];

                    let mut snapped_to_semicircle = false;
                    for (edge_type, cx, cy) in semicircle_centers {
                        let dx = lx - cx;
                        let dy = ly - cy;
                        if dx * dx + dy * dy <= 60.0 * 60.0 {
                            closest_edge = edge_type;
                            snapped_to_semicircle = true;
                            break;
                        }
                    }

                    if !snapped_to_semicircle {
                        match edge {
                            EdgeBasic::Top => {
                                if is_near_left {
                                    closest_edge = crate::window::StatusEdge::TopLeft;
                                } else if is_near_right {
                                    closest_edge = crate::window::StatusEdge::TopRight;
                                } else {
                                    closest_edge = crate::window::StatusEdge::TopCenter;
                                }
                            }
                            EdgeBasic::Bottom => {
                                if is_near_left {
                                    closest_edge = crate::window::StatusEdge::BottomLeft;
                                } else if is_near_right {
                                    closest_edge = crate::window::StatusEdge::BottomRight;
                                } else {
                                    closest_edge = crate::window::StatusEdge::BottomCenter;
                                }
                            }
                            EdgeBasic::Left => {
                                if is_near_top {
                                    closest_edge = crate::window::StatusEdge::TopLeft;
                                } else if is_near_bottom {
                                    closest_edge = crate::window::StatusEdge::BottomLeft;
                                } else {
                                    closest_edge = crate::window::StatusEdge::Left;
                                }
                            }
                            EdgeBasic::Right => {
                                if is_near_top {
                                    closest_edge = crate::window::StatusEdge::TopRight;
                                } else if is_near_bottom {
                                    closest_edge = crate::window::StatusEdge::BottomRight;
                                } else {
                                    closest_edge = crate::window::StatusEdge::Right;
                                }
                            }
                        }
                    }
                }
                
                log::info!("[StatusRelease] Snapping app_id={} closest_edge={:?}, found_out={}", app_id, closest_edge, found_out);
                (*win).status_edge = closest_edge;
                
                let name = if let Some(stripped) = app_id.strip_prefix("cce-status-interface-left-").or_else(|| app_id.strip_prefix("cce-status-left-")) {
                    stripped
                } else if let Some(stripped) = app_id.strip_prefix("cce-status-interface-right-").or_else(|| app_id.strip_prefix("cce-status-right-")) {
                    stripped
                } else {
                    &app_id
                };
                let edge_str = match closest_edge {
                    crate::window::StatusEdge::Left => "left",
                    crate::window::StatusEdge::Right => "right",
                    crate::window::StatusEdge::TopLeft => "top-left",
                    crate::window::StatusEdge::TopCenter => "top-center",
                    crate::window::StatusEdge::TopRight => "top-right",
                    crate::window::StatusEdge::BottomLeft => "bottom-left",
                    crate::window::StatusEdge::BottomCenter => "bottom-center",
                    crate::window::StatusEdge::BottomRight => "bottom-right",
                    _ => "top-left",
                };
                let key_path = format!("layout.status_bar.{}", name);
                cce_ui::config::write_config_value(
                    &cce_ui::config::get_config_path().to_string_lossy(),
                    &key_path,
                    &format!("\"{}\"", edge_str),
                    "layout"
                );

                seat.op_end();
                cursor.pressed.remove(&(*event).button);
                (*server).wm.dirty_windowing();
                return;
            }
            if op.started_in_overview && (*event).button == 0x110 {
                let moved = (cursor_x as i32 - op.start_x).abs() > 5 || (cursor_y as i32 - op.start_y).abs() > 5;
                if !moved && !op.window_ptr.is_null() {
                    let win_ptr = op.window_ptr;
                    // Restore original tiling mode & lock status
                    (*win_ptr).tiling_mode = op.start_tiling_mode;
                    (*win_ptr).mode_locked = op.start_mode_locked;

                    let server = seat.server;
                    if !(*win_ptr).closed && !(*win_ptr).is_status_bar() && !(*win_ptr).is_wallpaper() {
                        // The Overview toggle's exit path: it centers on the
                        // HOVERED window (the cursor is on the clicked one),
                        // focuses it, and ANIMATES the camera home along the
                        // configured overview ramp — the same flight the
                        // background-click exit takes, instead of the
                        // instant cut this block used to hand-roll.
                        (*server).wm.execute_action(&crate::config::Action::Overview, None);
                    }
                }
            }
            
            seat.op_end();
            cursor.pressed.remove(&(*event).button);
            return;
        }
        if let Some(binding_opt) = cursor.pressed.remove(&(*event).button) {
            if let Some(binding) = binding_opt {
                (*binding).released();
                if cursor.pressed.is_empty() && seat.op.is_some() {
                    seat.op_release = true;
                    (*(*seat).server).wm.dirty_windowing();
                }
                return;
            }

            if (*event).button == 0x110 && cursor.left_click_on_bg_in_overview {
                cursor.left_click_on_bg_in_overview = false;
                if cursor.pressed.is_empty() && seat.op.is_some() {
                    seat.op_release = true;
                    (*(*seat).server).wm.dirty_windowing();
                }
                return;
            }

            if (*event).button == 0x111 && (cursor.right_click_on_bg || cursor.right_click_on_border) {
                cursor.right_click_on_bg = false;
                cursor.right_click_on_border = false;
                if cursor.pressed.is_empty() && seat.op.is_some() {
                    seat.op_release = true;
                    (*(*seat).server).wm.dirty_windowing();
                }
                return;
            }

            // The client-facing release (when the press was forwarded) is
            // already paired at the top of the release path.
            if cursor.pressed.is_empty() && seat.op.is_some() {
                seat.op_release = true;
                (*(*seat).server).wm.dirty_windowing();
            }
        } else {
            log::error!("ignoring duplicate pointer button {} release", (*event).button);
        }
    }
}

unsafe extern "C" fn handle_axis(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
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
    } | (*seat.server).wm.injected_key_mods;

    if (modifiers & 0x44) == 0x44 {
        if (*event).orientation == ffi::wl_pointer_axis_WL_POINTER_AXIS_VERTICAL_SCROLL {
            if delta != 0.0 {
                let wm = &mut (*seat.server).wm;
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
        let server = seat.server;
        let mut over_interactive = false;
        // Chrome under the pointer — a Popup (the cce-cloud launcher) or an
        // Overlay dock, or a cce-cloud layer surface (context menu). Live UI
        // during overview, same as in the button and motion paths: a wheel
        // over the launcher's list scrolls the list, not the desktop.
        let mut over_chrome = false;
        if let Some(result) = (*server).scene.at(lx, ly) {
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
                SceneNodeDataVal::ShellSurface(_) | SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                    over_interactive = true;
                }
            }
        }
        (!over_interactive, over_chrome)
    };
    // Overview pans on any scroll — except over chrome, which takes the
    // event itself.
    let is_overview = (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview
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
        let wm = &mut (*seat.server).wm;
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

const KEY_SPACE: u32 = 57;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
/// Safety net that ends a swipe drag when the lift event never came.
/// Only that: libinput posts nothing at all while the fingers rest on the
/// pad mid-gesture — a captured Houdini swipe paused 3.3 s between two
/// halves of one scroll, the zero-delta lift arriving only at the true end
/// — so a short timeout tears the drag down inside the gesture, and the
/// Space release/re-press a resume then costs can drop Houdini out of view
/// mode for the rest of the swipe (the ordering hazard `button_down`
/// documents, now mid-swipe). The lift ends a drag; so, at once, do real
/// pointer motion, a button and a key press.
const VIEW_DRAG_IDLE_MS: i32 = 5000;
/// Drag distance (layout px) per e-fold of pinch scale. Measured against
/// Houdini 22 (depth of the world origin in view space, which is what a
/// dolly changes — Houdini dollies toward the point under the pointer, so
/// distances to a fixed pivot mislead): Space+RMB dollies on the VERTICAL
/// drag, up is in, and 45 px up shortened the depth by a factor of 1.34,
/// about 150 px per e-fold. So a pinch of scale s becomes an upward drag
/// of ln(s) e-folds and the depth ends near 1/s.
const VIEW_DRAG_PINCH_PX: f64 = 150.0;

impl Cursor {
    /// The window under the pointer, if `touchpad_view_apps` names its app.
    unsafe fn view_drag_target(&mut self) -> Option<(*mut crate::window::Window, *mut ffi::wlr_surface, f64, f64, f64)> {
        let server = (*self.seat).server;
        let wm = &(*server).wm;
        if wm.touchpad_view_apps.is_empty() {
            return None;
        }
        let result = (*server).scene.at(self.x(), self.y())?;
        let SceneNodeDataVal::Window(window) = result.data else { return None };
        if window.is_null() || result.surface.is_null() || result.node.is_null() {
            return None;
        }
        let app_id = (*window).get_app_id_string().unwrap_or_default();
        if !wm.touchpad_view_apps.iter().any(|p| crate::window_manager::app_id_matches(p, &app_id)) {
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
            let wm = &(*(*self.seat).server).wm;
            let tumble = wm.touchpad_view_swipe_tumble != (modifiers & SHIFT != 0);
            let button = if tumble { BTN_LEFT } else { BTN_MIDDLE };
            if !self.begin_view_drag(button, false) {
                return false;
            }
        }
        let wm = &(*(*self.seat).server).wm;
        let mut step = delta * wm.touchpad_view_sensitivity;
        if self.axis_event_is_natural(event) {
            step = -step;
        }
        if wm.touchpad_view_invert {
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
        let wm = &(*(*self.seat).server).wm;
        // Pinch out (scale > 1) dollies in: an upward drag.
        let px = -scale.max(0.05).ln() * VIEW_DRAG_PINCH_PX * wm.touchpad_view_sensitivity;
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

const KEY_LEFTCTRL: u32 = 29;
/// Layout pixels per emitted notch — the unit wl_pointer and `inject_scroll`
/// already use for one wheel click.
const POPUP_WHEEL_NOTCH_PX: f64 = 15.0;
/// Finger silence that ends a popup gesture when the lift never came. Far
/// shorter than the view drag's net, because the two failure modes are not
/// alike: a resumed swipe only re-presses Ctrl, with no view mode to fall
/// out of, while a modifier left held would turn the user's next click into
/// a Ctrl-click.
const POPUP_WHEEL_IDLE_MS: i32 = 400;

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
        let wm = &(*server).wm;
        if wm.touchpad_view_apps.is_empty() {
            return None;
        }
        let result = (*server).scene.at(self.x(), self.y())?;
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
            if !wm.touchpad_view_apps.iter().any(|p| crate::window_manager::app_id_matches(p, &app_id)) {
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

unsafe extern "C" fn handle_view_drag_timeout(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
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

const KEY_LEFTSHIFT: u32 = 42;
/// Finger silence that ends the gesture when the lift never came; a short
/// one is safe here because nothing is held that a re-press would break
/// (unlike `ViewDrag`'s Space).
const HSCROLL_SHIFT_IDLE_MS: i32 = 300;

impl Cursor {
    /// The surface under the pointer, if it belongs to a window of an app
    /// in `touchpad_hscroll_shift_apps`.
    unsafe fn hscroll_shift_target(&mut self) -> Option<*mut ffi::wlr_surface> {
        let server = (*self.seat).server;
        let wm = &(*server).wm;
        if wm.touchpad_hscroll_shift_apps.is_empty() {
            return None;
        }
        let result = (*server).scene.at(self.x(), self.y())?;
        let SceneNodeDataVal::Window(window) = result.data else { return None };
        if window.is_null() || result.surface.is_null() {
            return None;
        }
        let app_id = (*window).get_app_id_string().unwrap_or_default();
        if !wm.touchpad_hscroll_shift_apps.iter().any(|p| crate::window_manager::app_id_matches(p, &app_id)) {
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
    let _cursor = &mut *crate::container_of!(listener, Cursor, tablet_tool_proximity_listener);
    let event = data as *mut ffi::wlr_tablet_tool_proximity_event;

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

unsafe extern "C" fn handle_touch_down(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_down_listener);
    let event = data as *mut ffi::wlr_touch_down_event;

    let seat = &mut *cursor.seat;
    seat.handle_activity();

    let mut lx = 0.0;
    let mut ly = 0.0;
    let wlr_device = &mut (*(*event).touch).base as *mut ffi::wlr_input_device;
    ffi::wlr_cursor_absolute_to_layout_coords(
        cursor.wlr_cursor,
        wlr_device,
        (*event).x,
        (*event).y,
        &mut lx,
        &mut ly,
    );

    cursor.touch_points.insert((*event).touch_id, (lx, ly));

    let server = seat.server;
    if let Some(result) = (*server).scene.at(lx, ly) {
        match result.data {
            SceneNodeDataVal::LayerSurface(_) => {
                seat.focus(Focus::LayerSurface(result.surface));
            }
            _ => {}
        }
        
        let mut is_app_surface = false;
        let mut is_overlay_window = false;
        match result.data {
            SceneNodeDataVal::Window(window) => {
                if !(*window).is_status_bar() && !(*window).is_wallpaper() {
                    is_app_surface = true;
                    // Popup counts as chrome like Overlay: the cce-cloud
                    // launcher must keep receiving clicks in overview.
                    if (*window).tiling_mode == crate::tiling::TilingMode::Overlay
                        || (*window).tiling_mode == crate::tiling::TilingMode::Popup
                    {
                        is_overlay_window = true;
                    }
                }
            }
            SceneNodeDataVal::LayerSurface(layer_surface) => {
                if !layer_surface.is_null() {
                    is_app_surface = true;
                    if is_cloud_layer(layer_surface) {
                        is_overlay_window = true;
                    }
                }
            }
            SceneNodeDataVal::ShellSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                is_app_surface = true;
            }
            _ => {}
        }
        let should_block_touch = (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview && is_app_surface && !is_overlay_window;

        if !result.surface.is_null() && !should_block_touch {
            ffi::wlr_seat_touch_notify_down(
                seat.wlr_seat,
                result.surface,
                (*event).time_msec,
                (*event).touch_id,
                result.sx,
                result.sy,
            );
        }
    }
}

unsafe extern "C" fn handle_touch_motion(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_motion_listener);
    let event = data as *mut ffi::wlr_touch_motion_event;

    let seat = &mut *cursor.seat;
    seat.handle_activity();

    if cursor.touch_points.contains_key(&(*event).touch_id) {
        let wlr_device = &mut (*(*event).touch).base as *mut ffi::wlr_input_device;
        let mut lx: f64 = 0.0;
        let mut ly: f64 = 0.0;
        ffi::wlr_cursor_absolute_to_layout_coords(
            cursor.wlr_cursor,
            wlr_device,
            (*event).x,
            (*event).y,
            &mut lx,
            &mut ly,
        );

        cursor.touch_points.insert((*event).touch_id, (lx, ly));

        cursor.update_drag_icons();

        let server = seat.server;
        if let Some(result) = (*server).scene.at(lx, ly) {
            let mut is_app_surface = false;
            let mut is_overlay_window = false;
            match result.data {
                SceneNodeDataVal::Window(window) => {
                    if !(*window).is_status_bar() && !(*window).is_wallpaper() {
                        is_app_surface = true;
                        if (*window).tiling_mode == crate::tiling::TilingMode::Overlay {
                            is_overlay_window = true;
                        }
                    }
                }
                SceneNodeDataVal::LayerSurface(layer_surface) => {
                    if !layer_surface.is_null() {
                        is_app_surface = true;
                        let wlr_layer_surface = (*layer_surface).wlr_layer_surface;
                        if !wlr_layer_surface.is_null() && !(*wlr_layer_surface).namespace.is_null() {
                            let ns = std::ffi::CStr::from_ptr((*wlr_layer_surface).namespace).to_string_lossy();
                            if ns.starts_with("cce-cloud") {
                                is_overlay_window = true;
                            }
                        }
                    }
                }
                SceneNodeDataVal::ShellSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                    is_app_surface = true;
                }
                _ => {}
            }
            let should_block_touch = (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview && is_app_surface && !is_overlay_window;

            if !should_block_touch {
                ffi::wlr_seat_touch_notify_motion(
                    seat.wlr_seat,
                    (*event).time_msec,
                    (*event).touch_id,
                    result.sx,
                    result.sy,
                );
            }
        }
    }
}

unsafe extern "C" fn handle_touch_up(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_up_listener);
    let event = data as *mut ffi::wlr_touch_up_event;

    let seat = &mut *cursor.seat;
    seat.handle_activity();

    if let Some((lx, ly)) = cursor.touch_points.remove(&(*event).touch_id) {
        let server = seat.server;
        let mut is_app_surface = false;
        let mut is_overlay_window = false;
        if let Some(result) = (*server).scene.at(lx, ly) {
            match result.data {
                SceneNodeDataVal::Window(window) => {
                    if !(*window).is_status_bar() && !(*window).is_wallpaper() {
                        is_app_surface = true;
                        if (*window).tiling_mode == crate::tiling::TilingMode::Overlay {
                            is_overlay_window = true;
                        }
                    }
                }
                SceneNodeDataVal::LayerSurface(layer_surface) => {
                    if !layer_surface.is_null() {
                        is_app_surface = true;
                        let wlr_layer_surface = (*layer_surface).wlr_layer_surface;
                        if !wlr_layer_surface.is_null() && !(*wlr_layer_surface).namespace.is_null() {
                            let ns = std::ffi::CStr::from_ptr((*wlr_layer_surface).namespace).to_string_lossy();
                            if ns.starts_with("cce-cloud") {
                                is_overlay_window = true;
                            }
                        }
                    }
                }
                SceneNodeDataVal::ShellSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                    is_app_surface = true;
                }
                _ => {}
            }
        }
        let should_block_touch = (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview && is_app_surface && !is_overlay_window;

        if !should_block_touch {
            ffi::wlr_seat_touch_notify_up(
                seat.wlr_seat,
                (*event).time_msec,
                (*event).touch_id,
            );
        }
    }
}

unsafe extern "C" fn handle_touch_cancel(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_cancel_listener);
    
    let seat = &mut *cursor.seat;
    seat.handle_activity();

    cursor.touch_points.clear();

    ffi::river_wlr_seat_touch_cancel_all(seat.wlr_seat);
}

unsafe extern "C" fn handle_touch_frame(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_frame_listener);
    
    let seat = &mut *cursor.seat;
    seat.handle_activity();

    ffi::wlr_seat_touch_notify_frame(seat.wlr_seat);
}

/// A directional swipe bind fires once the accumulated travel (libinput
/// units) passes `window_manager { swipe_threshold }`
/// (`WindowManager::swipe_threshold`, default 50). Until then the camera
/// *peeks*: it pans toward the swipe direction in proportion to the
/// travel, up to `window_manager { swipe_peek }` screen px
/// (`WindowManager::swipe_peek_px`, default 60) at the threshold, and
/// eases back if the fingers lift short of it — so a hesitant
/// three-finger swipe shows where it would go without going.

/// Does firing `action` carry the view in the swipe's direction? Only
/// such binds peek the camera beforehand: an overview toggle or a spawn
/// on a swipe has no direction the desktop could lean toward.
fn action_navigates(action: crate::config::Action) -> bool {
    use crate::config::Action::*;
    matches!(action, FocusLeft | FocusRight | FocusUp | FocusDown | PanLeft | PanRight | PanUp | PanDown)
}

/// The peek the accumulated travel `d` along one axis calls for, in
/// virtual units: proportional and clamped at `threshold`, `peek_px` on
/// screen there, and only toward a direction that has a navigating bind
/// (`neg` / `pos`) — a swipe with nothing bound its way leaves the
/// desktop still.
fn swipe_peek_for(d: f64, neg: bool, pos: bool, threshold: f64, peek_px: f64, zoom: f64) -> f64 {
    if (d < 0.0 && neg) || (d > 0.0 && pos) {
        (d / threshold).clamp(-1.0, 1.0) * peek_px / zoom.max(1e-6)
    } else {
        0.0
    }
}

unsafe extern "C" fn handle_swipe_begin(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, swipe_begin_listener);
    let event = data as *mut ffi::wlr_pointer_swipe_begin_event;

    let seat = &mut *cursor.seat;
    let wm = &(*seat.server).wm;
    let swipe_enabled = wm.input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.swipe).unwrap_or(true);
    if !swipe_enabled {
        return;
    }
    seat.handle_activity();

    cursor.gesture_dx = 0.0;
    cursor.gesture_dy = 0.0;
    cursor.gesture_triggered = false;
    cursor.swipe_peek = [0.0, 0.0];

    log::info!("handle_swipe_begin: fingers={}", (*event).fingers);

    let server = seat.server;
    let pointer_gestures = (*server).input_manager.pointer_gestures;
    if !pointer_gestures.is_null() {
        ffi::wlr_pointer_gestures_v1_send_swipe_begin(
            pointer_gestures,
            seat.wlr_seat,
            (*event).time_msec,
            (*event).fingers,
        );
    }
}

unsafe extern "C" fn handle_swipe_update(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, swipe_update_listener);
    let event = data as *mut ffi::wlr_pointer_swipe_update_event;

    let seat = &mut *cursor.seat;
    let wm = &(*seat.server).wm;
    let swipe_enabled = wm.input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.swipe).unwrap_or(true);
    if !swipe_enabled {
        return;
    }
    seat.handle_activity();



    if cursor.gesture_triggered {
        return;
    }

    cursor.gesture_dx += (*event).dx;
    cursor.gesture_dy += (*event).dy;

    log::info!(
        "handle_swipe_update: fingers={}, dx={}, dy={}, accumulated_dx={}, accumulated_dy={}",
        (*event).fingers,
        (*event).dx,
        (*event).dy,
        cursor.gesture_dx,
        cursor.gesture_dy
    );

    let wlr_keyboard = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
    let modifiers = if !wlr_keyboard.is_null() {
        ffi::wlr_keyboard_get_modifiers(wlr_keyboard) & 0x4d
    } else {
        0
    };

    let threshold = (*seat.server).wm.swipe_threshold;
    let mut matched_action = crate::config::Action::None;
    let mut matched_command = None;
    // Which directions this finger count + chord could still fire a
    // navigating bind in — the directions the camera may peek toward.
    let mut navigates = [false; 4]; // left, right, up, down

    for gb in &(*seat.server).wm.gesture_binds {
        if gb.gesture_type == "swipe" && gb.fingers == (*event).fingers && gb.mods == modifiers {
            let (matched, slot) = match gb.direction.as_str() {
                "left" => (cursor.gesture_dx < -threshold, Some(0)),
                "right" => (cursor.gesture_dx > threshold, Some(1)),
                "up" => (cursor.gesture_dy < -threshold, Some(2)),
                "down" => (cursor.gesture_dy > threshold, Some(3)),
                _ => (false, None),
            };
            if matched {
                matched_action = gb.action;
                matched_command = gb.command.clone();
                break;
            }
            // First match wins in this table, so only the first bind per
            // direction decides whether that way peeks.
            if let Some(i) = slot {
                if !navigates[i] && action_navigates(gb.action) {
                    navigates[i] = true;
                }
            }
        }
    }

    if matched_action != crate::config::Action::None {
        log::info!("Swipe gesture matched action: {:?}", matched_action);
        cursor.gesture_triggered = true;

        // Hand the action the camera as it stood before the peek, so a
        // focus lands exactly where a keyed one would; then resume from
        // the peeked position so the ease runs from where the screen is.
        // An action that leaves the camera alone still gets a target: the
        // origin, so the peek eases back instead of sticking.
        let peek = std::mem::replace(&mut cursor.swipe_peek, [0.0, 0.0]);
        let peeked = if peek != [0.0, 0.0] {
            let wm = &mut (*seat.server).wm;
            wm.desk_pan_x += wm.pan_pending[0];
            wm.desk_pan_y += wm.pan_pending[1];
            wm.pan_pending = [0.0, 0.0];
            let peeked = (wm.desk_pan_x, wm.desk_pan_y);
            wm.desk_pan_x -= peek[0];
            wm.desk_pan_y -= peek[1];
            Some(peeked)
        } else {
            None
        };

        if matched_action == crate::config::Action::Overview && (*seat.server).wm.mode == crate::window_manager::WindowManagerMode::Overview {
            let lx = cursor.x();
            let ly = cursor.y();
            let mut hovered_win: *mut crate::window::Window = std::ptr::null_mut();
            if let Some(result) = (*seat.server).scene.at(lx, ly) {
                if let SceneNodeDataVal::Window(window) = result.data {
                    hovered_win = window;
                }
            }
            if !hovered_win.is_null() && !(*hovered_win).is_status_bar() && !(*hovered_win).is_wallpaper() {
                seat.focus(Focus::Window(hovered_win));
                if !seat.object.is_null() && !(*hovered_win).object.is_null() {
                    ffi::wl_resource_post_event(seat.object, 4, (*hovered_win).object);
                }
            }
        }

        // The overview toggle lands on the hovered window, else on the
        // FOCUSED one — never on the empty desktop under the pointer.
        let matched_action = if matched_action == crate::config::Action::Overview {
            (*seat.server).wm.overview_action_for_gesture()
        } else {
            matched_action
        };
        (*seat.server).wm.execute_action(&matched_action, matched_command.as_deref());

        if let Some((px, py)) = peeked {
            let wm = &mut (*seat.server).wm;
            let origin = (px - peek[0], py - peek[1]);
            // An action that set the camera outright (no ease) owns it now.
            if (wm.desk_pan_x, wm.desk_pan_y) == origin {
                if let Some(ramp) = wm.camera_ramp_anim.as_mut() {
                    ramp.start.pan_x = px;
                    ramp.start.pan_y = py;
                } else {
                    wm.target_desk_pan_x.get_or_insert(origin.0);
                    wm.target_desk_pan_y.get_or_insert(origin.1);
                }
                wm.desk_pan_x = px;
                wm.desk_pan_y = py;
                wm.start_panning_animation();
            }
        }

        let pointer_gestures = (*seat.server).input_manager.pointer_gestures;
        if !pointer_gestures.is_null() {
            ffi::wlr_pointer_gestures_v1_send_swipe_end(
                pointer_gestures,
                seat.wlr_seat,
                (*event).time_msec,
                true, // cancelled: true
            );
        }
        return;
    }

    // Short of the threshold: lean the camera toward the bind the swipe
    // is heading for, 1:1 with the fingers like a two-finger pan (queued
    // for the frame, no easing), recomputed from the total travel so a
    // reversal leans back through zero.
    {
        let wm = &mut (*seat.server).wm;
        let peek_px = wm.swipe_peek_px;
        let want = [
            swipe_peek_for(cursor.gesture_dx, navigates[0], navigates[1], threshold, peek_px, wm.desk_zoom),
            swipe_peek_for(cursor.gesture_dy, navigates[2], navigates[3], threshold, peek_px, wm.desk_zoom),
        ];
        let delta = [want[0] - cursor.swipe_peek[0], want[1] - cursor.swipe_peek[1]];
        if delta != [0.0, 0.0] {
            wm.stop_panning_animation();
            wm.queue_pan(delta[0], delta[1]);
            cursor.swipe_peek = want;
        }
    }

    let server = seat.server;
    let pointer_gestures = (*server).input_manager.pointer_gestures;
    if !pointer_gestures.is_null() {
        ffi::wlr_pointer_gestures_v1_send_swipe_update(
            pointer_gestures,
            seat.wlr_seat,
            (*event).time_msec,
            (*event).dx,
            (*event).dy,
        );
    }
}

unsafe extern "C" fn handle_swipe_end(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, swipe_end_listener);
    let event = data as *mut ffi::wlr_pointer_swipe_end_event;

    let seat = &mut *cursor.seat;
    let wm = &(*seat.server).wm;
    let swipe_enabled = wm.input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.swipe).unwrap_or(true);
    if !swipe_enabled {
        return;
    }
    seat.handle_activity();

    log::info!("handle_swipe_end: cancelled={}", (*event).cancelled);

    if cursor.gesture_triggered {
        cursor.gesture_triggered = false;
        return;
    }

    // Lifted short of the threshold: ease the camera back to where the
    // swipe found it.
    let peek = std::mem::replace(&mut cursor.swipe_peek, [0.0, 0.0]);
    if peek != [0.0, 0.0] {
        let wm = &mut (*seat.server).wm;
        wm.desk_pan_x += wm.pan_pending[0];
        wm.desk_pan_y += wm.pan_pending[1];
        wm.pan_pending = [0.0, 0.0];
        wm.target_desk_pan_x = Some(wm.desk_pan_x - peek[0]);
        wm.target_desk_pan_y = Some(wm.desk_pan_y - peek[1]);
        wm.start_panning_animation();
    }

    let server = seat.server;
    let pointer_gestures = (*server).input_manager.pointer_gestures;
    if !pointer_gestures.is_null() {
        ffi::wlr_pointer_gestures_v1_send_swipe_end(
            pointer_gestures,
            seat.wlr_seat,
            (*event).time_msec,
            (*event).cancelled,
        );
    }
}

unsafe extern "C" fn handle_pinch_begin(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, pinch_begin_listener);
    let event = data as *mut ffi::wlr_pointer_pinch_begin_event;

    let seat = &mut *cursor.seat;
    let wm = &(*seat.server).wm;
    let pinch_enabled = wm.input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.pinch).unwrap_or(true);
    if !pinch_enabled {
        return;
    }
    seat.handle_activity();

    cursor.gesture_scale = 1.0;
    cursor.gesture_triggered = false;

    let server = seat.server;

    // A pinch starting over the desktop background (wallpaper or bare
    // desktop, same test as the right-click context menu) zooms the camera
    // for the whole gesture. Clients never see a begin, so update/end stay
    // ours too.
    let mut on_background = true;
    if let Some(result) = (*server).scene.at(cursor.x(), cursor.y()) {
        match result.data {
            SceneNodeDataVal::Window(window) => {
                if !(*window).is_wallpaper() {
                    on_background = false;
                }
            }
            SceneNodeDataVal::LayerSurface(_) | SceneNodeDataVal::ShellSurface(_) | SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                on_background = false;
            }
        }
    }
    if on_background {
        let wm = &mut (*server).wm;
        wm.stop_panning_animation();
        cursor.pinch_zoom_active = true;
        cursor.pinch_start_zoom = wm.desk_zoom;
        return;
    }

    // A pinch over an app in `touchpad_view_apps` becomes a dolly drag.
    if cursor.view_drag_pinch_begin() {
        return;
    }

    let pointer_gestures = (*server).input_manager.pointer_gestures;
    if !pointer_gestures.is_null() {
        ffi::wlr_pointer_gestures_v1_send_pinch_begin(
            pointer_gestures,
            seat.wlr_seat,
            (*event).time_msec,
            (*event).fingers,
        );
    }
}

unsafe extern "C" fn handle_pinch_update(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, pinch_update_listener);
    let event = data as *mut ffi::wlr_pointer_pinch_update_event;

    let seat = &mut *cursor.seat;
    let wm = &(*seat.server).wm;
    let pinch_enabled = wm.input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.pinch).unwrap_or(true);
    if !pinch_enabled {
        return;
    }
    seat.handle_activity();

    if cursor.view_drag_pinch_update((*event).scale) {
        return;
    }

    if cursor.pinch_zoom_active {
        let wm = &mut (*seat.server).wm;
        let new_zoom = crate::policy::camera::pinch_zoom(cursor.pinch_start_zoom, (*event).scale);
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

    let wlr_keyboard = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
    let modifiers = if !wlr_keyboard.is_null() {
        ffi::wlr_keyboard_get_modifiers(wlr_keyboard) & 0x4d
    } else {
        0
    };

    let mut matched_action = crate::config::Action::None;
    let mut matched_command = None;

    for gb in &(*seat.server).wm.gesture_binds {
        if gb.gesture_type == "pinch" && gb.fingers == (*event).fingers && gb.mods == modifiers {
            let matched = match gb.direction.as_str() {
                "in" => cursor.gesture_scale < 0.7,
                "out" => cursor.gesture_scale > 1.3,
                _ => false,
            };
            if matched {
                matched_action = gb.action;
                matched_command = gb.command.clone();
                break;
            }
        }
    }

    if matched_action != crate::config::Action::None {
        cursor.gesture_triggered = true;
        // The overview toggle lands on the hovered window, else on the
        // FOCUSED one — never on the empty desktop under the pointer.
        let matched_action = if matched_action == crate::config::Action::Overview {
            (*seat.server).wm.overview_action_for_gesture()
        } else {
            matched_action
        };
        (*seat.server).wm.execute_action(&matched_action, matched_command.as_deref());

        let pointer_gestures = (*seat.server).input_manager.pointer_gestures;
        if !pointer_gestures.is_null() {
            ffi::wlr_pointer_gestures_v1_send_pinch_end(
                pointer_gestures,
                seat.wlr_seat,
                (*event).time_msec,
                true, // cancelled: true
            );
        }
        return;
    }

    let server = seat.server;
    let pointer_gestures = (*server).input_manager.pointer_gestures;
    if !pointer_gestures.is_null() {
        ffi::wlr_pointer_gestures_v1_send_pinch_update(
            pointer_gestures,
            seat.wlr_seat,
            (*event).time_msec,
            (*event).dx,
            (*event).dy,
            (*event).scale,
            (*event).rotation,
        );
    }
}

unsafe extern "C" fn handle_pinch_end(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, pinch_end_listener);
    let event = data as *mut ffi::wlr_pointer_pinch_end_event;

    let seat = &mut *cursor.seat;
    let wm = &(*seat.server).wm;
    let pinch_enabled = wm.input_config.touchpad.as_ref().and_then(|t| t.gestures.as_ref()).and_then(|g| g.pinch).unwrap_or(true);
    if !pinch_enabled {
        return;
    }
    seat.handle_activity();

    if cursor.view_drag_pinch_end() {
        return;
    }

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

    let server = seat.server;
    let pointer_gestures = (*server).input_manager.pointer_gestures;
    if !pointer_gestures.is_null() {
        ffi::wlr_pointer_gestures_v1_send_pinch_end(
            pointer_gestures,
            seat.wlr_seat,
            (*event).time_msec,
            (*event).cancelled,
        );
    }
}

unsafe extern "C" fn handle_hold_begin(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, hold_begin_listener);
    let event = data as *mut ffi::wlr_pointer_hold_begin_event;

    let seat = &mut *cursor.seat;
    seat.handle_activity();

    let server = seat.server;
    let pointer_gestures = (*server).input_manager.pointer_gestures;
    if !pointer_gestures.is_null() {
        ffi::wlr_pointer_gestures_v1_send_hold_begin(
            pointer_gestures,
            seat.wlr_seat,
            (*event).time_msec,
            (*event).fingers,
        );
    }
}

unsafe extern "C" fn handle_hold_end(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, hold_end_listener);
    let event = data as *mut ffi::wlr_pointer_hold_end_event;

    let seat = &mut *cursor.seat;
    seat.handle_activity();

    let server = seat.server;
    let pointer_gestures = (*server).input_manager.pointer_gestures;
    if !pointer_gestures.is_null() {
        ffi::wlr_pointer_gestures_v1_send_hold_end(
            pointer_gestures,
            seat.wlr_seat,
            (*event).time_msec,
            (*event).cancelled,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderZone {
    None,
    Move,
    Resize(crate::window::Edges),
}

pub use crate::window::HOVER_BAND_MIN;

/// The grid client's surface and the surface-local coordinates of a layout
/// point, ignoring the input region.
///
/// Hit-testing cannot be used for this: the grid's input region covers only
/// its desktop items, so a point over bare canvas — where drops mostly land —
/// misses it by design. A drop target does not need to be hit-testable, only
/// named, so the surface is resolved directly and the point is mapped through
/// the surface node's own layout origin and the window's display scale, which
/// is the same pair the renderer draws with.
pub unsafe fn grid_surface_at(
    server: *mut crate::server::Server,
    lx: f64,
    ly: f64,
) -> Option<(*mut ffi::wlr_surface, f64, f64)> {
    let (surface, nx, ny, scale, bw, bh) = grid_node_info(server)?;
    let (sx, sy) = ((lx - nx) / scale, (ly - ny) / scale);
    // Only claim points that actually fall on the grid's patch. box_geom
    // is the surface's own logical size, which is the space sx/sy are in.
    if sx < 0.0 || sy < 0.0 || (bw > 0.0 && sx >= bw) || (bh > 0.0 && sy >= bh) {
        return None;
    }
    Some((surface, sx, sy))
}

/// The grid window's surface plus the raw mapping ingredients: its surface
/// tree's layout origin, its display scale, and its logical size. This is
/// the node state a caller may need to FREEZE (the implicit-grab path
/// captures it at press time), which is why it is exposed separately from
/// the point mapping above.
pub unsafe fn grid_node_info(
    server: *mut crate::server::Server,
) -> Option<(*mut ffi::wlr_surface, f64, f64, f64, f64, f64)> {
    for &w in (*server).wm.windows.iter() {
        if w.is_null() || (*w).closed || !(*w).is_grid() {
            continue;
        }
        if !matches!((*w).state, crate::window::WindowState::Mapped) {
            continue;
        }
        let surface = (*w).root_surface();
        if surface.is_null() {
            continue;
        }
        let node = (*w).surfaces.tree as *mut ffi::wlr_scene_node;
        let (mut nx, mut ny) = (0, 0);
        if !ffi::wlr_scene_node_coords(node, &mut nx, &mut ny) {
            continue;
        }
        let scale = if (*w).scale > 0.0 { (*w).scale } else { 1.0 };
        return Some((
            surface,
            nx as f64,
            ny as f64,
            scale,
            (*w).box_geom.width as f64,
            (*w).box_geom.height as f64,
        ));
    }
    None
}

/// Which resize handle, if any, a layout point falls on.
///
/// The handles are eight discs INSIDE the content rect — one at the
/// midpoint of each side, one on each corner — and exist only in adjust
/// mode (overview, or Super held). Two consequences worth stating, because
/// both are deliberate:
///
///   - Outside adjust mode there are no handles at all, so a window cannot
///     be resized with the pointer. The keyboard and IPC paths
///     (`move_window_*`, `ccectl move-window`, a client repositioning
///     itself) are untouched; this is only about dragging.
///   - Every disc RESIZES, the top one included. Moving is what dragging
///     the window's body does in adjust mode, so no handle has to be spent
///     on it; between two discs the pointer belongs to the body.
///
/// `window::handle_disc_layout` places the discs for the hit test here,
/// the catchers in `draw_borders`, and (mirrored in the frame shader) the
/// drawing, so the zones and the visuals cannot drift.
pub unsafe fn get_border_zone(window: *mut crate::window::Window, lx: f64, ly: f64) -> BorderZone {
    // Overview, or Super held (window-adjust mode): the same ring either way.
    if !(*(*window).server).wm.window_adjust_active() {
        return BorderZone::None;
    }
    if !crate::window::window_takes_handles(window) {
        return BorderZone::None;
    }
    // The adjust target only — the window under the pointer — matching what
    // draw_borders draws. A window showing no ring has no band, and a grab
    // that is not drawn is the failure mode this file keeps warning about.
    // The pointer reaches a window's edge through its body, so the band is
    // live by the time it arrives.
    if !(*window).is_adjust_target() {
        return BorderZone::None;
    }

    let bw_unscaled = crate::window::border_band_width((*window).rendering_requested.border.width);
    if bw_unscaled <= 0.0 {
        return BorderZone::None;
    }

    // box_geom holds the UNSCALED content size; on screen the window covers
    // `size * scale`, and lx/ly are layout px — so the CONTENT extents scale
    // but the discs do NOT. The disc diameter is a screen size, matching
    // what draw_borders draws: overview is zoomed out, and a handle that
    // shrank with the window would be smallest exactly where it is the only
    // way to resize. Keep the two in step.
    let scale = if (*window).scale > 0.0 { (*window).scale } else { 1.0 };
    let geom = (*window).box_geom;
    let rx = lx - geom.x as f64;
    let ry = ly - geom.y as f64;
    let content_w = geom.width as f64 * scale;
    let content_h = geom.height as f64 * scale;

    let bw = ((*(*window).server).wm.layout.border_handle_width as f64)
        .max(crate::window::HOVER_BAND_MIN)
        // The same fifth-of-the-short-side cap draw_borders applies, so the
        // grab zone never outgrows the disc the user can see.
        .min(content_w.min(content_h).max(1.0) * 0.2);

    // Outside the window entirely: not ours.
    if rx < 0.0 || rx >= content_w || ry < 0.0 || ry >= content_h {
        return BorderZone::None;
    }
    // A client popover (set_popover_region) owns its rect outright: the menu
    // reads as in front of the chrome, so nothing under it may grab. Checked
    // before the discs — it beats them.
    if let Some(r) = (*window).popover_region {
        let (ex, ey) = (r.x as f64 * scale, r.y as f64 * scale);
        let (ew, eh) = (r.width as f64 * scale, r.height as f64 * scale);
        if rx >= ex && rx < ex + ew && ry >= ey && ry < ey + eh {
            return BorderZone::None;
        }
    }

    // The discs, from the same on-screen size, silhouette radius and
    // diameter draw_borders hands the frame shader, so what is drawn is
    // what grabs. A pixel of slack covers the antialiased rim. The corner
    // discs place against the content radius: the widened root plate
    // radius the corner clip uses, on screen.
    let r_in = crate::window::widen_corner_radius(
        (*window).root_plate_radius_base(),
        geom.width,
        geom.height,
    ) as f64;
    let r_in = (r_in * scale) as i32 as f64;
    let (centres, r) = crate::window::handle_disc_layout(content_w, content_h, r_in, bw);
    let reach = (r + 1.0) * (r + 1.0);
    for (i, &(cx, cy)) in centres.iter().enumerate() {
        let (dx, dy) = (rx - cx, ry - cy);
        if dx * dx + dy * dy <= reach {
            return BorderZone::Resize(edges_for_border_element(
                crate::window::BorderElement::ALL[i],
            ));
        }
    }
    // In the body, between the discs: not ours. This is what leaves the
    // body drag-to-move working.
    BorderZone::None
}

/// The resize edges a handle disc stands for — the inverse of
/// `border_element_for_edges`.
pub fn edges_for_border_element(element: crate::window::BorderElement) -> crate::window::Edges {
    use crate::window::BorderElement::*;
    let (top, bottom, left, right) = match element {
        Top => (true, false, false, false),
        Bottom => (false, true, false, false),
        Left => (false, false, true, false),
        Right => (false, false, false, true),
        TopLeft => (true, false, true, false),
        TopRight => (true, false, false, true),
        BottomLeft => (false, true, true, false),
        BottomRight => (false, true, false, true),
    };
    crate::window::Edges { top, bottom, left, right }
}

/// Map a resize zone's edges to the border element that should highlight.
pub fn border_element_for_edges(edges: crate::window::Edges) -> crate::window::BorderElement {
    use crate::window::BorderElement::*;
    match (edges.top, edges.bottom, edges.left, edges.right) {
        (true, _, true, _) => TopLeft,
        (true, _, _, true) => TopRight,
        (_, true, true, _) => BottomLeft,
        (_, true, _, true) => BottomRight,
        (_, true, _, _) => Bottom,
        (_, _, true, _) => Left,
        (_, _, _, true) => Right,
        _ => Top,
    }
}

pub fn get_resize_cursor_name(edges: crate::window::Edges) -> &'static [u8] {
    if edges.top && edges.left {
        b"nw-resize\0"
    } else if edges.top && edges.right {
        b"ne-resize\0"
    } else if edges.bottom && edges.left {
        b"sw-resize\0"
    } else if edges.bottom && edges.right {
        b"se-resize\0"
    } else if edges.top {
        b"n-resize\0"
    } else if edges.bottom {
        b"s-resize\0"
    } else if edges.left {
        b"w-resize\0"
    } else if edges.right {
        b"e-resize\0"
    } else {
        b"default\0"
    }
}

pub unsafe fn get_closest_edges(window: *mut crate::window::Window, lx: f64, ly: f64) -> crate::window::Edges {
    let geom = (*window).box_geom;
    let rx = lx - geom.x as f64;
    let ry = ly - geom.y as f64;
    let w = geom.width as f64;
    let h = geom.height as f64;

    let left = rx < w / 2.0;
    let right = !left;
    let top = ry < h / 2.0;
    let bottom = !top;

    crate::window::Edges { top, bottom, left, right }
}

