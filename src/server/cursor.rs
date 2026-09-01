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
    pub panning_gesture_active: bool,
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
            panning_gesture_active: false,
            pinch_zoom_active: false,
            pinch_start_zoom: 1.0,
            last_click_time: 0,
            last_click_window: std::ptr::null_mut(),
            hovered_border_window: std::ptr::null_mut(),
            hovered_border_element: None,
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
                    lx - self.grab_origin.0,
                    ly - self.grab_origin.1,
                );
                return;
            }
        }

        if let Some(result) = (*server).scene.at(lx, ly) {
            let lock_state = (*server).lock_manager.state;
            if lock_state != crate::lock_manager::LockState::Unlocked {
                if !matches!(result.data, SceneNodeDataVal::LockSurface(_)) {
                    self.set_border_hover(std::ptr::null_mut(), None);
                    self.clear_focus();
                    return;
                }
            } else {
                if matches!(result.data, SceneNodeDataVal::LockSurface(_)) {
                    self.set_border_hover(std::ptr::null_mut(), None);
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
                }
                _ => {}
            }
            self.set_border_hover(std::ptr::null_mut(), None);

            if is_window && (*server).wm.mode == crate::window_manager::WindowManagerMode::Overview {
                // Focus follows the pointer in overview: the ring is drawn on
                // the focused window only, so hovering is how it moves between
                // windows without a click. Guarded on an actual change —
                // seat.focus raises a Floating window BEFORE its same-focus
                // short-circuit, so an unguarded call would raise and relayout
                // on every motion event. And with the pan suppressed: hovering
                // must not move the camera, only the click and keyboard paths
                // may. (Zoom was never at stake — focus_follow_pan pans at the
                // current zoom — but a partially visible window would still
                // get dragged on-screen mid-hover.)
                if !hovered_toplevel.is_null()
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
    pub unsafe fn inject_scroll(&mut self, dy: f64, dx: f64) {
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
                source: ffi::wl_pointer_axis_source_WL_POINTER_AXIS_SOURCE_WHEEL,
                orientation,
                relative_direction: ffi::wl_pointer_axis_relative_direction_WL_POINTER_AXIS_RELATIVE_DIRECTION_IDENTICAL,
                delta,
                delta_discrete: ((delta / 15.0) * 120.0) as i32,
            };
            handle_axis(
                &mut self.axis_listener as *mut ffi::wl_listener,
                &mut ev as *mut ffi::wlr_pointer_axis_event as *mut std::ffi::c_void,
            );
        }
        handle_frame(&mut self.frame_listener as *mut ffi::wl_listener, std::ptr::null_mut());
    }
}

unsafe extern "C" fn handle_motion(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, motion_listener);
    let event = data as *mut ffi::wlr_pointer_motion_event;
    
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
            if !clicked_status.is_null() {
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

        // --- ZOOMED OUT CLICK HANDLING ---
        if (*event).button == 0x110 && (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview {
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
                    started_in_overview: true,
                });
                cursor.op_start_pointer();
                cursor.pressed.insert((*event).button, None);
                cursor.set_xcursor(b"grab\0".as_ptr() as *const _);
                return;
            } else if !overview_win_valid {
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
                seat.focus(Focus::Window(target_win));
                
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
            if (*event).button == 0x111 && modifiers == 0 && !matches!(zone, BorderZone::None) {
                cursor.right_click_on_border = true;
                let x = cursor.x() as i32;
                let y = cursor.y() as i32;
                let index = (*border_target_win).ref_key.index;
                let app_id = (*border_target_win).get_app_id_string().unwrap_or_else(|| "unknown".to_string());
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

                        seat.focus(Focus::Window(border_target_win));
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

                        seat.focus(Focus::Window(border_target_win));
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
                    cursor.grab_origin = (glx - result.sx, gly - result.sy);
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
                let mut closest_edge = crate::window::StatusEdge::TopLeft;
                let mut min_dist = f64::MAX;
                let app_id = (*win).get_app_id_string().unwrap_or_default();
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
    let modifiers = if !wlr_keyboard.is_null() {
        ffi::wlr_keyboard_get_modifiers(wlr_keyboard)
    } else {
        0
    };

    if (modifiers & 0x44) == 0x44 {
        if (*event).orientation == ffi::wl_pointer_axis_WL_POINTER_AXIS_VERTICAL_SCROLL {
            if delta != 0.0 {
                let wm = &mut (*seat.server).wm;
                wm.stop_panning_animation();
                let old_zoom = wm.desk_zoom;
                let new_zoom = crate::policy::camera::wheel_zoom(old_zoom, delta);
                if new_zoom != old_zoom {
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
                    // Wheel zoom pivots about the cursor: the virtual point
                    // under it stays put on screen.
                    let cam = crate::policy::camera::zoom_about_anchor(
                        wm.camera(),
                        cx - phys_x,
                        cy - phys_y,
                        new_zoom,
                    );
                    wm.desk_pan_x = cam.pan_x;
                    wm.desk_pan_y = cam.pan_y;
                    wm.desk_zoom = cam.zoom;
                    wm.set_mode(if crate::policy::camera::is_overview(cam.zoom) { crate::window_manager::WindowManagerMode::Overview } else { crate::window_manager::WindowManagerMode::Normal });
                    if matches!(wm.state, crate::window_manager::WindowManagerState::Idle) {
                        wm.update_viewport_local();
                    } else {
                        wm.dirty_windowing();
                    }
                }
            }
        }
        return;
    }

    let is_on_background = {
        let lx = cursor.x();
        let ly = cursor.y();
        let server = seat.server;
        let mut over_interactive = false;
        if let Some(result) = (*server).scene.at(lx, ly) {
            match result.data {
                SceneNodeDataVal::Window(_) | SceneNodeDataVal::LayerSurface(_) | SceneNodeDataVal::ShellSurface(_) | SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                    over_interactive = true;
                }
            }
        }
        !over_interactive
    };
    let is_overview = (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview;

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
            // Finger/continuous scroll tracks 1:1 — the surface follows the
            // gesture directly, no easing between the two.
            wm.stop_panning_animation();
            if vertical {
                wm.desk_pan_y += step;
            } else {
                wm.desk_pan_x += step;
            }
            if matches!(wm.state, crate::window_manager::WindowManagerState::Idle) {
                wm.update_viewport_local();
            } else {
                wm.dirty_windowing();
            }
        } else {
            // Discrete wheel clicks glide: each click advances the pan
            // animation target, so successive clicks accumulate into one
            // smooth run instead of a stutter of jumps.
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

    let mut matched_action = crate::config::Action::None;
    let mut matched_command = None;

    for gb in &(*seat.server).wm.gesture_binds {
        if gb.gesture_type == "swipe" && gb.fingers == (*event).fingers && gb.mods == modifiers {
            let matched = match gb.direction.as_str() {
                "left" => cursor.gesture_dx < -50.0,
                "right" => cursor.gesture_dx > 50.0,
                "up" => cursor.gesture_dy < -50.0,
                "down" => cursor.gesture_dy > 50.0,
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
        log::info!("Swipe gesture matched action: {:?}", matched_action);
        cursor.gesture_triggered = true;

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

        (*seat.server).wm.execute_action(&matched_action, matched_command.as_deref());

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

    if cursor.pinch_zoom_active {
        let wm = &mut (*seat.server).wm;
        let old_zoom = wm.desk_zoom;
        let new_zoom = crate::policy::camera::pinch_zoom(cursor.pinch_start_zoom, (*event).scale);
        if new_zoom != old_zoom {
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
            // Like the wheel, pinch pivots about the cursor: the virtual
            // point under it stays put on screen.
            let cam = crate::policy::camera::zoom_about_anchor(
                wm.camera(),
                cx - phys_x,
                cy - phys_y,
                new_zoom,
            );
            wm.desk_pan_x = cam.pan_x;
            wm.desk_pan_y = cam.pan_y;
            wm.desk_zoom = cam.zoom;
            wm.set_mode(if crate::policy::camera::is_overview(cam.zoom) { crate::window_manager::WindowManagerMode::Overview } else { crate::window_manager::WindowManagerMode::Normal });
            if matches!(wm.state, crate::window_manager::WindowManagerState::Idle) {
                wm.update_viewport_local();
            } else {
                wm.dirty_windowing();
            }
        }
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

/// Where a layout point falls on a window's interactive band.
///
/// The band lives INSIDE the content rect — an inset ring hugging the
/// window's own edges — and exists only in overview mode. Two consequences
/// worth stating, because both are deliberate:
///
///   - In normal mode there is no band at all, so a window cannot be moved
///     or resized with the pointer. The keyboard and IPC paths
///     (`move_window_*`, `ccectl move-window`, a client repositioning
///     itself) are untouched; this is only about dragging.
///   - Inside the ring, all four edges RESIZE, the top included. Moving is
///     what dragging the window's body already does in overview, so the top
///     edge does not have to be spent on it the way the old outside band
///     did.
///
/// `window.rs`'s `draw_borders` draws the handles from the same band width
/// and corner length, so the zones and the visuals cannot drift.
pub unsafe fn get_border_zone(window: *mut crate::window::Window, lx: f64, ly: f64) -> BorderZone {
    if (*(*window).server).wm.mode != crate::window_manager::WindowManagerMode::Overview {
        return BorderZone::None;
    }
    if !crate::window::window_takes_handles(window) {
        return BorderZone::None;
    }
    // Focused window only, matching what draw_borders draws. Unfocused
    // windows show no ring, and a grab that is not drawn is the failure mode
    // this file keeps warning about; hover-to-focus (the motion path) means
    // reaching a window's edge focuses it on the way, so its band is live by
    // the time the pointer arrives.
    if !(*window).is_seat_focused() {
        return BorderZone::None;
    }

    let bw_unscaled = crate::window::border_band_width((*window).rendering_requested.border.width);
    if bw_unscaled <= 0.0 {
        return BorderZone::None;
    }

    // box_geom holds the UNSCALED content size; on screen the window covers
    // `size * scale`, and lx/ly are layout px — so the CONTENT extents scale
    // but the band does NOT. The grab width is a screen width, matching what
    // draw_borders draws: overview is zoomed out, and a band that shrank with
    // the window would be thinnest exactly where it is the only way to
    // resize. Keep the two in step.
    let scale = if (*window).scale > 0.0 { (*window).scale } else { 1.0 };
    let geom = (*window).box_geom;
    let rx = lx - geom.x as f64;
    let ry = ly - geom.y as f64;
    let content_w = geom.width as f64 * scale;
    let content_h = geom.height as f64 * scale;

    let bw = ((*(*window).server).wm.layout.border_handle_width as f64)
        .max(crate::window::HOVER_BAND_MIN)
        // The same fifth-of-the-short-side cap draw_borders applies, so the
        // grab zone never outgrows the ring the user can see.
        .min(content_w.min(content_h).max(1.0) * 0.2);

    // Outside the window entirely, or in the body beyond the ring: not ours.
    // The body case is what leaves overview's drag-to-move working.
    if rx < 0.0 || rx >= content_w || ry < 0.0 || ry >= content_h {
        return BorderZone::None;
    }
    // A client popover (set_popover_region) owns its rect outright: the menu
    // reads as in front of the chrome, so nothing under it may grab. Checked
    // before the pads and the band — it beats both.
    if let Some(r) = (*window).popover_region {
        let (ex, ey) = (r.x as f64 * scale, r.y as f64 * scale);
        let (ew, eh) = (r.width as f64 * scale, r.height as f64 * scale);
        if rx >= ex && rx < ex + ew && ry >= ey && ry < ey + eh {
            return BorderZone::None;
        }
    }

    // The corner pads reach further inward than the band (their radius plus
    // the corner arc), and what is drawn must grab: a point within a pad's
    // radius of its corner resizes on that corner's two edges.
    let pad = ((*(*window).server).wm.layout.border_corner_bulge as f64)
        .min(content_w.min(content_h) * 0.3);
    if pad > 0.0 {
        let cx = if rx < content_w / 2.0 { 0.0 } else { content_w };
        let cy = if ry < content_h / 2.0 { 0.0 } else { content_h };
        let (dx, dy) = (rx - cx, ry - cy);
        if (dx * dx + dy * dy).sqrt() < pad {
            return BorderZone::Resize(crate::window::Edges {
                top: cy == 0.0,
                bottom: cy > 0.0,
                left: cx == 0.0,
                right: cx > 0.0,
            });
        }
    }

    let (near_l, near_r) = (rx < bw, rx >= content_w - bw);
    let (near_t, near_b) = (ry < bw, ry >= content_h - bw);
    if !(near_l || near_r || near_t || near_b) {
        return BorderZone::None;
    }

    // Corner squares of `corner_len`, measured from the content corners —
    // the same length draw_borders gives the corner handles.
    // Floored at the band, then clamped per AXIS to 0.45 of that side, the
    // same rule the shader draws with: two corner runs on one side must
    // never meet, or its midpoint would resize diagonally.
    let corner_len = (crate::window::border_corner_len(
        bw_unscaled,
        (*(*window).server).wm.layout.border_corner_length,
        0.0,
    ) * scale)
        .max(bw);
    let cl_x = corner_len.min(content_w * 0.45);
    let cl_y = corner_len.min(content_h * 0.45);
    let corner_l = rx < cl_x;
    let corner_r = rx >= content_w - cl_x;
    let corner_t = ry < cl_y;
    let corner_b = ry >= content_h - cl_y;

    if (corner_l || corner_r) && (corner_t || corner_b) {
        return BorderZone::Resize(crate::window::Edges {
            top: corner_t,
            bottom: corner_b && !corner_t,
            left: corner_l,
            right: corner_r && !corner_l,
        });
    }

    BorderZone::Resize(crate::window::Edges {
        top: near_t,
        bottom: near_b && !near_t,
        left: near_l,
        right: near_r && !near_l,
    })
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

