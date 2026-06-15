use crate::ffi;
use crate::seat::{Seat, Focus};
use crate::server::{WlListener, wl_listener_remove, wl_signal_add, WlList};
use crate::scene_node_data::SceneNodeDataVal;
use crate::drag_icon::DragIcon;
use std::collections::HashMap;

pub struct Cursor {
    pub seat: *mut Seat,
    pub wlr_cursor: *mut ffi::wlr_cursor,
    pub xcursor_manager: *mut ffi::wlr_xcursor_manager,
    pub constraint: *mut crate::pointer_constraint::PointerConstraint,

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
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            seat: std::ptr::null_mut(),
            wlr_cursor: std::ptr::null_mut(),
            xcursor_manager: std::ptr::null_mut(),
            constraint: std::ptr::null_mut(),
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

        // Load default cursor theme
        ffi::wlr_xcursor_manager_load(xcursor_manager, 1.0);
        ffi::wlr_cursor_set_xcursor(wlr_cursor, xcursor_manager, b"default\0".as_ptr() as *const _);

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
                        ffi::wlr_xwayland_set_cursor(
                            (*(*self.seat).server).xwayland,
                            (*image).buffer,
                            (*image).width * 4,
                            (*image).width,
                            (*image).height,
                            (*image).hotspot_x as i32,
                            (*image).hotspot_y as i32,
                        );
                    }
                }
            }
        }

        if !self.xcursor_manager.is_null() {
            ffi::wlr_xcursor_manager_destroy(self.xcursor_manager);
        }
        self.xcursor_manager = xcursor_manager;

        ffi::wlr_xcursor_manager_load(self.xcursor_manager, 1.0);
        ffi::wlr_cursor_set_xcursor(
            self.wlr_cursor,
            self.xcursor_manager,
            b"default\0".as_ptr() as *const _,
        );

        Ok(())
    }

    pub unsafe fn set_xcursor(&mut self, name: *const std::os::raw::c_char) {
        ffi::wlr_cursor_set_xcursor(
            self.wlr_cursor,
            self.xcursor_manager,
            name,
        );
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

    pub unsafe fn passthrough(&mut self, time_msec: u32) {
        let lx = self.x();
        let ly = self.y();
        let server = (*self.seat).server;

        if let Some(result) = (*server).scene.at(lx, ly) {
            let lock_state = (*server).lock_manager.state;
            if lock_state != crate::lock_manager::LockState::Unlocked {
                if !matches!(result.data, SceneNodeDataVal::LockSurface(_)) {
                    self.clear_focus();
                    return;
                }
            } else {
                if matches!(result.data, SceneNodeDataVal::LockSurface(_)) {
                    self.clear_focus();
                    return;
                }
            }

            if let SceneNodeDataVal::Window(window) = result.data {
                if (*window).tiling_mode == crate::tiling::TilingMode::Floating
                    || (*window).tiling_mode == crate::tiling::TilingMode::Cascade
                {
                    match get_border_zone(window, lx, ly) {
                        BorderZone::Resize(edges) => {
                            ffi::wlr_seat_pointer_notify_clear_focus((*self.seat).wlr_seat);
                            let cursor_name = get_resize_cursor_name(edges);
                            self.set_xcursor(cursor_name.as_ptr() as *const _);
                            return;
                        }
                        BorderZone::Move => {
                            ffi::wlr_seat_pointer_notify_clear_focus((*self.seat).wlr_seat);
                            self.set_xcursor(b"grab\0".as_ptr() as *const _);
                            return;
                        }
                        BorderZone::None => {}
                    }
                }
            }

            if !result.surface.is_null() {
                ffi::wlr_seat_pointer_notify_enter((*self.seat).wlr_seat, result.surface, result.sx, result.sy);
                ffi::wlr_seat_pointer_notify_motion((*self.seat).wlr_seat, time_msec, result.sx, result.sy);
                return;
            }
        }

        self.clear_focus();
    }

    pub unsafe fn clear_focus(&mut self) {
        ffi::wlr_seat_pointer_notify_clear_focus((*self.seat).wlr_seat);
        ffi::wlr_cursor_set_xcursor(
            self.wlr_cursor,
            self.xcursor_manager,
            b"default\0".as_ptr() as *const _,
        );
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

    ffi::wlr_cursor_move(cursor.wlr_cursor, std::ptr::null_mut(), dx, dy);
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

unsafe extern "C" fn handle_button(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, button_listener);
    let event = data as *mut ffi::wlr_pointer_button_event;
    
    let seat = &mut *cursor.seat;
    
    if (*event).state == ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED {
        if cursor.pressed.contains_key(&(*event).button) {
            log::error!("ignoring duplicate pointer button {} press", (*event).button);
            return;
        }

        let wlr_keyboard = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
        let modifiers = if !wlr_keyboard.is_null() {
            ffi::wlr_keyboard_get_modifiers(wlr_keyboard)
        } else {
            0
        };
        
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
            
            if !target_win.is_null() {
                if (*target_win).tiling_mode == crate::tiling::TilingMode::Cascade {
                    (*target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                    (*target_win).mode_locked = true;
                }
                seat.focus(Focus::Window(target_win));
                
                let op_type = match pb.action {
                    crate::config::Action::Move => Some(crate::seat::PointerOpType::Move),
                    crate::config::Action::Resize => {
                        let edges = get_closest_edges(target_win, lx, ly);
                        Some(crate::seat::PointerOpType::Resize { edges })
                    }
                    _ => None,
                };
                
                if let Some(ot) = op_type {
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

        if !border_target_win.is_null() && (
            (*border_target_win).tiling_mode == crate::tiling::TilingMode::Floating
            || (*border_target_win).tiling_mode == crate::tiling::TilingMode::Cascade
        ) {
            let initial_mode = (*border_target_win).tiling_mode;
            match get_border_zone(border_target_win, lx, ly) {
                BorderZone::Resize(edges) => {
                    if (*event).button == 0x110 { // BTN_LEFT
                        if initial_mode == crate::tiling::TilingMode::Cascade {
                            (*border_target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                            (*border_target_win).mode_locked = true;
                        }

                        seat.focus(Focus::Window(border_target_win));
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
                        if initial_mode == crate::tiling::TilingMode::Cascade {
                            (*border_target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                            (*border_target_win).mode_locked = true;
                        }

                        seat.focus(Focus::Window(border_target_win));
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

        ffi::wlr_seat_pointer_notify_button(
            seat.wlr_seat,
            (*event).time_msec,
            (*event).button,
            (*event).state,
        );

        // If pressed, update focus to window under cursor
        let lx = cursor.x();
        let ly = cursor.y();
        let server = seat.server;
        if let Some(result) = (*server).scene.at(lx, ly) {
            match result.data {
                SceneNodeDataVal::Window(window) => {
                    seat.focus(Focus::Window(window));
                    if !seat.object.is_null() && !(*window).object.is_null() {
                        ffi::wl_resource_post_event(seat.object, 4, (*window).object);
                        (*(*seat).server).wm.dirty_windowing();
                    }
                }
                SceneNodeDataVal::LayerSurface(_) => {
                    seat.focus(Focus::LayerSurface(result.surface));
                }
                _ => {}
            }
        }
    } else {
        assert_eq!((*event).state, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_RELEASED);
        if seat.op.is_some() {
            let cursor_x = (*cursor.wlr_cursor).x;
            let cursor_y = (*cursor.wlr_cursor).y;
            seat.op_update(cursor_x as i32, cursor_y as i32);
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

            ffi::wlr_seat_pointer_notify_button(
                seat.wlr_seat,
                (*event).time_msec,
                (*event).button,
                (*event).state,
            );

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

unsafe extern "C" fn handle_tablet_tool_axis(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, tablet_tool_axis_listener);
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
    let cursor = &mut *crate::container_of!(listener, Cursor, tablet_tool_tip_listener);
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
    let cursor = &mut *crate::container_of!(listener, Cursor, tablet_tool_button_listener);
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
        if !result.surface.is_null() {
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

unsafe extern "C" fn handle_touch_up(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_up_listener);
    let event = data as *mut ffi::wlr_touch_up_event;

    let seat = &mut *cursor.seat;
    seat.handle_activity();

    if cursor.touch_points.remove(&(*event).touch_id).is_some() {
        ffi::wlr_seat_touch_notify_up(
            seat.wlr_seat,
            (*event).time_msec,
            (*event).touch_id,
        );
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
    seat.handle_activity();

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
    seat.handle_activity();

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
    seat.handle_activity();

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
    seat.handle_activity();

    let server = seat.server;
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
    seat.handle_activity();

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
    seat.handle_activity();

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

pub unsafe fn get_border_zone(window: *mut crate::window::Window, lx: f64, ly: f64) -> BorderZone {
    if (*window).tiling_mode != crate::tiling::TilingMode::Floating
        && (*window).tiling_mode != crate::tiling::TilingMode::Cascade
    {
        return BorderZone::None;
    }
    
    let bw = (*window).rendering_requested.border.width as f64;
    if bw <= 0.0 {
        return BorderZone::None;
    }

    let geom = (*window).box_geom;
    let rx = lx - geom.x as f64;
    let ry = ly - geom.y as f64;

    let content_w = geom.width as f64;
    let content_h = geom.height as f64;

    if rx >= 0.0 && rx < content_w && ry >= 0.0 && ry < content_h {
        return BorderZone::None;
    }

    if rx >= -bw && rx < content_w + bw && ry >= -bw && ry < content_h + bw {
        let dist_left = rx + bw;
        let dist_right = (content_w + bw) - rx;
        let dist_top = ry + bw;
        let dist_bottom = (content_h + bw) - ry;

        let corner_threshold = bw + 2.0;
        let left = dist_left < corner_threshold;
        let right = dist_right < corner_threshold;
        let top = dist_top < corner_threshold;
        let bottom = dist_bottom < corner_threshold;

        return BorderZone::Resize(crate::window::Edges { top, bottom, left, right });
    }

    BorderZone::None
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

