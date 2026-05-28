use crate::ffi;
use crate::tablet::Tablet;
use crate::seat::Seat;
use crate::server::{WlListener, wl_listener_remove, wl_signal_add};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TabletToolMode {
    Passthrough,
    Down {
        lx: f64,
        ly: f64,
        sx: f64,
        sy: f64,
    },
}

pub struct TabletTool {
    pub wp_tool: *mut ffi::wlr_tablet_v2_tablet_tool,
    pub wlr_cursor: *mut ffi::wlr_cursor,
    pub mode: TabletToolMode,
    pub tilt_x: f64,
    pub tilt_y: f64,

    pub destroy_listener: ffi::wl_listener,
    pub set_cursor_listener: ffi::wl_listener,
}

impl TabletTool {
    pub unsafe fn get(
        wlr_seat: *mut ffi::wlr_seat,
        wlr_tool: *mut ffi::wlr_tablet_tool,
    ) -> Result<*mut Self, &'static str> {
        let data_ptr = (*wlr_tool).data as *mut Self;
        if !data_ptr.is_null() {
            Ok(data_ptr)
        } else {
            Self::create(wlr_seat, wlr_tool)
        }
    }

    pub unsafe fn create(
        wlr_seat: *mut ffi::wlr_seat,
        wlr_tool: *mut ffi::wlr_tablet_tool,
    ) -> Result<*mut Self, &'static str> {
        let wlr_cursor = ffi::wlr_cursor_create();
        if wlr_cursor.is_null() {
            return Err("Failed to create wlr_cursor");
        }

        let seat = ffi::river_wlr_seat_get_data(wlr_seat) as *mut Seat;
        if seat.is_null() {
            ffi::wlr_cursor_destroy(wlr_cursor);
            return Err("Seat data is null");
        }
        let server = (*seat).server;
        let output_layout = (*server).om.output_layout;

        ffi::wlr_cursor_attach_output_layout(wlr_cursor, output_layout);

        let tablet_manager = (*server).input_manager.tablet_manager;
        let wp_tool = ffi::wlr_tablet_tool_create(tablet_manager, wlr_seat, wlr_tool);
        if wp_tool.is_null() {
            ffi::wlr_cursor_destroy(wlr_cursor);
            return Err("Failed to create wp_tool");
        }

        let tool = Box::into_raw(Box::new(Self {
            wp_tool,
            wlr_cursor,
            mode: TabletToolMode::Passthrough,
            tilt_x: 0.0,
            tilt_y: 0.0,
            destroy_listener: std::mem::zeroed(),
            set_cursor_listener: std::mem::zeroed(),
        }));

        (*wlr_tool).data = tool as *mut _;

        let destroy_listener_ptr = &mut (*tool).destroy_listener as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_listener_ptr).notify = Some(handle_destroy);
        wl_signal_add(&mut (*wlr_tool).events.destroy, &mut (*tool).destroy_listener);

        let set_cursor_listener_ptr = &mut (*tool).set_cursor_listener as *mut ffi::wl_listener as *mut WlListener;
        (*set_cursor_listener_ptr).notify = Some(handle_set_cursor);
        let set_cursor_signal = ffi::river_wlr_tablet_v2_tablet_tool_get_set_cursor_signal(wp_tool);
        wl_signal_add(set_cursor_signal, &mut (*tool).set_cursor_listener);

        Ok(tool)
    }

    pub unsafe fn cursor_x(&self) -> f64 {
        ffi::river_wlr_cursor_get_x(self.wlr_cursor)
    }

    pub unsafe fn cursor_y(&self) -> f64 {
        ffi::river_wlr_cursor_get_y(self.wlr_cursor)
    }

    pub unsafe fn allow_set_cursor(
        &self,
        seat_client: *mut ffi::wlr_seat_client,
        serial: u32,
    ) -> bool {
        let focused_surface = ffi::river_wlr_tablet_v2_tablet_tool_get_focused_surface(self.wp_tool);
        if focused_surface.is_null() {
            log::debug!("client tried to set cursor without focus");
            return false;
        }

        let surface_resource = ffi::river_wlr_surface_get_resource(focused_surface);
        if surface_resource.is_null() {
            return false;
        }

        let surface_client = ffi::wl_resource_get_client(surface_resource);
        let seat_client_ptr = ffi::river_wlr_seat_client_get_client(seat_client);

        if surface_client != seat_client_ptr {
            log::debug!("client tried to set cursor without focus");
            return false;
        }

        let proximity_serial = ffi::river_wlr_tablet_v2_tablet_tool_get_proximity_serial(self.wp_tool);
        if serial != proximity_serial {
            log::debug!("focused client tried to set cursor with incorrect serial");
            return false;
        }

        true
    }

    pub unsafe fn attach(&mut self, tablet: *mut Tablet) {
        let dev = (*(*tablet).device).wlr_device;
        ffi::wlr_cursor_attach_input_device(self.wlr_cursor, dev);
        ffi::wlr_cursor_map_input_to_output(self.wlr_cursor, dev, (*(*tablet).device).config.map_to_output);
        ffi::wlr_cursor_map_input_to_region(self.wlr_cursor, dev, &mut (*(*tablet).device).config.map_to_rectangle as *mut _);
    }

    pub unsafe fn detach(&mut self, tablet: *mut Tablet) {
        let dev = (*(*tablet).device).wlr_device;
        ffi::wlr_cursor_detach_input_device(self.wlr_cursor, dev);
    }

    pub unsafe fn axis(&mut self, tablet: *mut Tablet, event: *mut ffi::wlr_tablet_tool_axis_event) {
        self.attach(tablet);

        let updated = (*event).updated_axes;
        let has_x = (updated & ffi::wlr_tablet_tool_axes_WLR_TABLET_TOOL_AXIS_X) != 0;
        let has_y = (updated & ffi::wlr_tablet_tool_axes_WLR_TABLET_TOOL_AXIS_Y) != 0;

        let wlr_tool = ffi::river_wlr_tablet_v2_tablet_tool_get_wlr_tool(self.wp_tool);
        let tool_type = (*wlr_tool).type_;

        if has_x || has_y {
            match tool_type {
                ffi::wlr_tablet_tool_type_WLR_TABLET_TOOL_TYPE_PEN |
                ffi::wlr_tablet_tool_type_WLR_TABLET_TOOL_TYPE_ERASER |
                ffi::wlr_tablet_tool_type_WLR_TABLET_TOOL_TYPE_BRUSH |
                ffi::wlr_tablet_tool_type_WLR_TABLET_TOOL_TYPE_PENCIL |
                ffi::wlr_tablet_tool_type_WLR_TABLET_TOOL_TYPE_AIRBRUSH |
                ffi::wlr_tablet_tool_type_WLR_TABLET_TOOL_TYPE_TOTEM => {
                    let warp_x = if has_x { (*event).x } else { f64::NAN };
                    let warp_y = if has_y { (*event).y } else { f64::NAN };
                    ffi::wlr_cursor_warp_absolute(self.wlr_cursor, (*(*tablet).device).wlr_device, warp_x, warp_y);
                }
                ffi::wlr_tablet_tool_type_WLR_TABLET_TOOL_TYPE_LENS |
                ffi::wlr_tablet_tool_type_WLR_TABLET_TOOL_TYPE_MOUSE => {
                    ffi::wlr_cursor_move(self.wlr_cursor, (*(*tablet).device).wlr_device, (*event).dx, (*event).dy);
                }
                _ => {}
            }

            match self.mode {
                TabletToolMode::Passthrough => {
                    self.passthrough(tablet);
                }
                TabletToolMode::Down { lx, ly, sx, sy } => {
                    let dx = self.cursor_x() - lx;
                    let dy = self.cursor_y() - ly;
                    ffi::wlr_tablet_v2_tablet_tool_notify_motion(self.wp_tool, sx + dx, sy + dy);
                }
            }
        }

        let has_dist = (updated & ffi::wlr_tablet_tool_axes_WLR_TABLET_TOOL_AXIS_DISTANCE) != 0;
        if has_dist {
            ffi::wlr_tablet_v2_tablet_tool_notify_distance(self.wp_tool, (*event).distance);
        }

        let has_press = (updated & ffi::wlr_tablet_tool_axes_WLR_TABLET_TOOL_AXIS_PRESSURE) != 0;
        if has_press {
            ffi::wlr_tablet_v2_tablet_tool_notify_pressure(self.wp_tool, (*event).pressure);
        }

        let has_tilt_x = (updated & ffi::wlr_tablet_tool_axes_WLR_TABLET_TOOL_AXIS_TILT_X) != 0;
        let has_tilt_y = (updated & ffi::wlr_tablet_tool_axes_WLR_TABLET_TOOL_AXIS_TILT_Y) != 0;
        if has_tilt_x || has_tilt_y {
            if has_tilt_x {
                self.tilt_x = (*event).tilt_x;
            }
            if has_tilt_y {
                self.tilt_y = (*event).tilt_y;
            }
            ffi::wlr_tablet_v2_tablet_tool_notify_tilt(self.wp_tool, self.tilt_x, self.tilt_y);
        }

        let has_rot = (updated & ffi::wlr_tablet_tool_axes_WLR_TABLET_TOOL_AXIS_ROTATION) != 0;
        if has_rot {
            ffi::wlr_tablet_v2_tablet_tool_notify_rotation(self.wp_tool, (*event).rotation);
        }

        let has_slider = (updated & ffi::wlr_tablet_tool_axes_WLR_TABLET_TOOL_AXIS_SLIDER) != 0;
        if has_slider {
            ffi::wlr_tablet_v2_tablet_tool_notify_slider(self.wp_tool, (*event).slider);
        }

        let has_wheel = (updated & ffi::wlr_tablet_tool_axes_WLR_TABLET_TOOL_AXIS_WHEEL) != 0;
        if has_wheel {
            ffi::wlr_tablet_v2_tablet_tool_notify_wheel(self.wp_tool, (*event).wheel_delta, 0);
        }

        self.detach(tablet);
    }

    pub unsafe fn proximity(
        &mut self,
        tablet: *mut Tablet,
        event: *mut ffi::wlr_tablet_tool_proximity_event,
    ) {
        let state = (*event).state;
        if state == ffi::wlr_tablet_tool_proximity_state_WLR_TABLET_TOOL_PROXIMITY_IN {
            self.attach(tablet);

            ffi::wlr_cursor_warp_absolute(
                self.wlr_cursor,
                (*(*tablet).device).wlr_device,
                (*event).x,
                (*event).y,
            );

            let xcursor_manager = (*(*(*tablet).device).seat).cursor.xcursor_manager;
            ffi::wlr_cursor_set_xcursor(
                self.wlr_cursor,
                xcursor_manager,
                b"pencil\0".as_ptr() as *const _,
            );

            self.passthrough(tablet);
            self.detach(tablet);
        } else {
            ffi::wlr_tablet_v2_tablet_tool_notify_proximity_out(self.wp_tool);
            ffi::wlr_cursor_unset_image(self.wlr_cursor);
        }
    }

    pub unsafe fn tip(&mut self, tablet: *mut Tablet, event: *mut ffi::wlr_tablet_tool_tip_event) {
        let state = (*event).state;
        let is_down = ffi::river_wlr_tablet_v2_tablet_tool_get_is_down(self.wp_tool);
        if state == ffi::wlr_tablet_tool_tip_state_WLR_TABLET_TOOL_TIP_DOWN {
            if is_down {
                return;
            }
            ffi::wlr_send_tablet_v2_tablet_tool_down(self.wp_tool);

            let server = (*(*(*tablet).device).seat).server;
            let lx = self.cursor_x();
            let ly = self.cursor_y();
            if let Some(result) = (*server).scene.at(lx, ly) {
                if !result.surface.is_null() {
                    self.mode = TabletToolMode::Down {
                        lx,
                        ly,
                        sx: result.sx,
                        sy: result.sy,
                    };
                }
            }
        } else {
            if !is_down {
                return;
            }
            ffi::wlr_send_tablet_v2_tablet_tool_up(self.wp_tool);
            self.maybe_exit_down(tablet);
        }
    }

    pub unsafe fn button(
        &mut self,
        tablet: *mut Tablet,
        event: *mut ffi::wlr_tablet_tool_button_event,
    ) {
        ffi::wlr_tablet_v2_tablet_tool_notify_button(
            self.wp_tool,
            (*event).button,
            (*event).state,
        );
        self.maybe_exit_down(tablet);
    }

    pub unsafe fn maybe_exit_down(&mut self, tablet: *mut Tablet) {
        let is_down = ffi::river_wlr_tablet_v2_tablet_tool_get_is_down(self.wp_tool);
        let num_buttons = ffi::river_wlr_tablet_v2_tablet_tool_get_num_buttons(self.wp_tool);
        if !matches!(self.mode, TabletToolMode::Down { .. }) || is_down || num_buttons > 0 {
            return;
        }
        self.mode = TabletToolMode::Passthrough;
        self.passthrough(tablet);
    }

    pub unsafe fn passthrough(&mut self, tablet: *mut Tablet) {
        let server = (*(*(*tablet).device).seat).server;
        let lx = self.cursor_x();
        let ly = self.cursor_y();

        if let Some(result) = (*server).scene.at(lx, ly) {
            if matches!(result.data, crate::scene_node_data::SceneNodeDataVal::LockSurface(_)) {
                assert!((*server).lock_manager.state != crate::lock_manager::LockState::Unlocked);
            } else {
                assert!((*server).lock_manager.state != crate::lock_manager::LockState::Locked);
            }

            if !result.surface.is_null() {
                ffi::wlr_send_tablet_v2_tablet_tool_proximity_in(self.wp_tool, (*tablet).wp_tablet, result.surface);
                ffi::wlr_tablet_v2_tablet_tool_notify_motion(self.wp_tool, result.sx, result.sy);
                return;
            }
        } else {
            let xcursor_manager = (*(*(*tablet).device).seat).cursor.xcursor_manager;
            ffi::wlr_cursor_set_xcursor(self.wlr_cursor, xcursor_manager, b"pencil\0".as_ptr() as *const _);
        }

        ffi::wlr_tablet_v2_tablet_tool_notify_proximity_out(self.wp_tool);
    }
}

unsafe extern "C" fn handle_destroy(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let tool_ptr = crate::container_of!(listener, TabletTool, destroy_listener) as *mut TabletTool;
    let tool = &mut *tool_ptr;

    ffi::wlr_cursor_destroy(tool.wlr_cursor);

    wl_listener_remove(&mut tool.destroy_listener);
    wl_listener_remove(&mut tool.set_cursor_listener);

    let _boxed = Box::from_raw(tool_ptr);
}

unsafe extern "C" fn handle_set_cursor(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let tool = &mut *crate::container_of!(listener, TabletTool, set_cursor_listener);
    let event = data as *mut ffi::wlr_tablet_v2_event_cursor;

    if tool.allow_set_cursor((*event).seat_client, (*event).serial) {
        ffi::wlr_cursor_set_surface(
            tool.wlr_cursor,
            (*event).surface,
            (*event).hotspot_x,
            (*event).hotspot_y,
        );
    }
}
