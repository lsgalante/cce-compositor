use crate::ffi;
use crate::server::{Server, WlListener, wl_listener_remove, wl_signal_add};
use crate::cursor::Cursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatOpInput {
    Pointer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerOpType {
    Move,
    Resize { edges: crate::window::Edges },
}

#[derive(Clone, Copy, Debug)]
pub struct SeatOp {
    pub sent_release: bool,
    pub input: SeatOpInput,
    pub start_x: i32,
    pub start_y: i32,
    pub x: i32,
    pub y: i32,
    pub window_ptr: *mut crate::window::Window,
    pub op_type: PointerOpType,
    pub start_win_x: i32,
    pub start_win_y: i32,
    pub start_win_w: u32,
    pub start_win_h: u32,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    None,
    LayerSurface(*mut ffi::wlr_surface),
    Window(*mut crate::window::Window),
    LockSurface(*mut crate::lock_manager::LockSurface),
    OverrideRedirect(*mut crate::xwayland_override_redirect::XwaylandOverrideRedirect),
    ShellSurface(*mut crate::shell_surface::ShellSurface),
}

impl Focus {
    pub unsafe fn surface(&self) -> *mut ffi::wlr_surface {
        match *self {
            Focus::None => std::ptr::null_mut(),
            Focus::LayerSurface(surface) => surface,
            Focus::Window(window) => if window.is_null() { std::ptr::null_mut() } else { (*window).root_surface() },
            Focus::LockSurface(lock_surface) => if lock_surface.is_null() { std::ptr::null_mut() } else { (*(*lock_surface).wlr_lock_surface).surface },
            Focus::OverrideRedirect(or) => if or.is_null() { std::ptr::null_mut() } else { (*(*or).xsurface).surface },
            Focus::ShellSurface(shell_surface) => if shell_surface.is_null() { std::ptr::null_mut() } else { (*shell_surface).surface },
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragState {
    None,
    Pointer,
    Touch,
}

pub struct Seat {
    pub server: *mut Server,
    pub wlr_seat: *mut ffi::wlr_seat,
    pub cursor: Cursor,
    pub focused: Focus,
    pub relay: crate::input_relay::InputRelay,
    pub layer_shell: crate::layer_shell::LayerShellSeat,
    pub xkb_bindings_seat: crate::xkb_bindings::XkbBindingsSeat,
    pub xkb_bindings: ffi::wl_list,
    pub pointer_bindings: ffi::wl_list,
    pub keyboard_groups: ffi::wl_list,
    pub modifiers_old: u32,
    pub op: Option<SeatOp>,
    pub op_release: bool,
    pub wm_sent_x: i32,
    pub wm_sent_y: i32,

    pub request_set_cursor: ffi::wl_listener,
    pub request_set_selection: ffi::wl_listener,
    pub request_start_drag: ffi::wl_listener,
    pub start_drag: ffi::wl_listener,
    pub request_set_primary_selection: ffi::wl_listener,

    pub drag: DragState,
    pub drag_destroy: ffi::wl_listener,

    pub link: ffi::wl_list,
    pub link_sent: ffi::wl_list,
    pub object: *mut ffi::wl_resource,
    pub destroying: bool,
    pub focus_requested: bool,
}

impl Seat {
    pub unsafe fn create(server: *mut Server, name: &str) -> Result<*mut Self, &'static str> {
        let name_c = std::ffi::CString::new(name).unwrap();
        let wlr_seat = ffi::wlr_seat_create((*server).wl_server, name_c.as_ptr());
        if wlr_seat.is_null() {
            return Err("Failed to create wlr_seat");
        }

        let seat = Box::into_raw(Box::new(Self {
            server,
            wlr_seat,
            cursor: Cursor::default(),
            focused: Focus::None,
            relay: std::mem::zeroed(),
            layer_shell: crate::layer_shell::LayerShellSeat::default(),
            xkb_bindings_seat: crate::xkb_bindings::XkbBindingsSeat::default(),
            xkb_bindings: std::mem::zeroed(),
            pointer_bindings: std::mem::zeroed(),
            keyboard_groups: std::mem::zeroed(),
            modifiers_old: 0,
            op: None,
            op_release: false,
            wm_sent_x: 0,
            wm_sent_y: 0,
            request_set_cursor: std::mem::zeroed(),
            request_set_selection: std::mem::zeroed(),
            request_start_drag: std::mem::zeroed(),
            start_drag: std::mem::zeroed(),
            request_set_primary_selection: std::mem::zeroed(),
            drag: DragState::None,
            drag_destroy: std::mem::zeroed(),
            link: std::mem::zeroed(),
            link_sent: std::mem::zeroed(),
            object: std::ptr::null_mut(),
            destroying: false,
            focus_requested: false,
        }));

        ffi::wl_list_init(&mut (*seat).link);
        ffi::wl_list_init(&mut (*seat).link_sent);
        ffi::wl_list_init(&mut (*seat).xkb_bindings);
        ffi::wl_list_init(&mut (*seat).pointer_bindings);
        ffi::wl_list_init(&mut (*seat).keyboard_groups);

        // Add to input_manager seats
        let seats_list = &mut (*server).input_manager.seats as *mut ffi::wl_list as *mut crate::server::WlList;
        crate::server::wl_list_insert((*seats_list).prev, &mut (*seat).link as *mut ffi::wl_list as *mut crate::server::WlList);

        ffi::river_wlr_seat_set_data(wlr_seat, seat as *mut _);
        (*seat).relay.init(seat);

        // Initialize Cursor
        let output_layout = (*server).om.output_layout;
        (*seat).cursor.init(seat, output_layout)?;

        // Setup listeners
        let set_cursor_ptr = &mut (*seat).request_set_cursor as *mut ffi::wl_listener as *mut WlListener;
        (*set_cursor_ptr).notify = Some(handle_request_set_cursor);
        wl_signal_add(
            ffi::river_wlr_seat_get_request_set_cursor_signal(wlr_seat),
            &mut (*seat).request_set_cursor,
        );

        let set_sel_ptr = &mut (*seat).request_set_selection as *mut ffi::wl_listener as *mut WlListener;
        (*set_sel_ptr).notify = Some(handle_request_set_selection);
        wl_signal_add(
            ffi::river_wlr_seat_get_request_set_selection_signal(wlr_seat),
            &mut (*seat).request_set_selection,
        );

        let start_drag_req_ptr = &mut (*seat).request_start_drag as *mut ffi::wl_listener as *mut WlListener;
        (*start_drag_req_ptr).notify = Some(handle_request_start_drag);
        wl_signal_add(
            ffi::river_wlr_seat_get_request_start_drag_signal(wlr_seat),
            &mut (*seat).request_start_drag,
        );

        let start_drag_ptr = &mut (*seat).start_drag as *mut ffi::wl_listener as *mut WlListener;
        (*start_drag_ptr).notify = Some(handle_start_drag);
        wl_signal_add(
            ffi::river_wlr_seat_get_start_drag_signal(wlr_seat),
            &mut (*seat).start_drag,
        );

        let set_prim_ptr = &mut (*seat).request_set_primary_selection as *mut ffi::wl_listener as *mut WlListener;
        (*set_prim_ptr).notify = Some(handle_request_set_primary_selection);
        wl_signal_add(
            ffi::river_wlr_seat_get_request_set_primary_selection_signal(wlr_seat),
            &mut (*seat).request_set_primary_selection,
        );

        (*seat).update_capabilities();

        Ok(seat)
    }

    pub unsafe fn destroy(seat: *mut Self) {
        (*seat).layer_shell.make_inert();
        (*seat).xkb_bindings_seat.make_inert();

        let bindings_head = &mut (*seat).xkb_bindings as *mut ffi::wl_list as *mut crate::server::WlList;
        let mut curr = (*bindings_head).next;
        while curr != bindings_head {
            let next = (*curr).next;
            let binding = crate::container_of!(curr, crate::xkb_bindings::XkbBinding, link);
            crate::xkb_bindings::XkbBinding::destroy(binding);
            curr = next;
        }

        let ptr_bindings_head = &mut (*seat).pointer_bindings as *mut ffi::wl_list as *mut crate::server::WlList;
        let mut curr_ptr = (*ptr_bindings_head).next;
        while curr_ptr != ptr_bindings_head {
            let next = (*curr_ptr).next;
            let binding = crate::container_of!(curr_ptr, crate::pointer_binding::PointerBinding, link);
            crate::pointer_binding::PointerBinding::destroy(binding);
            curr_ptr = next;
        }

        (*seat).cursor.deinit();

        // Verify keyboard_groups is empty
        let groups_head = &mut (*seat).keyboard_groups as *mut ffi::wl_list as *mut crate::server::WlList;
        assert_eq!((*groups_head).next, groups_head);

        crate::server::wl_list_remove(&mut (*seat).link as *mut ffi::wl_list as *mut crate::server::WlList);
        crate::server::wl_list_remove(&mut (*seat).link_sent as *mut ffi::wl_list as *mut crate::server::WlList);

        wl_listener_remove(&mut (*seat).request_set_cursor);
        wl_listener_remove(&mut (*seat).request_set_selection);
        wl_listener_remove(&mut (*seat).request_start_drag);
        wl_listener_remove(&mut (*seat).start_drag);
        wl_listener_remove(&mut (*seat).request_set_primary_selection);

        if (*seat).drag != DragState::None {
            wl_listener_remove(&mut (*seat).drag_destroy);
        }

        ffi::wlr_seat_destroy((*seat).wlr_seat);
        let _boxed = Box::from_raw(seat);
    }

    pub unsafe fn attach_device(&mut self, device: *mut crate::input_device::InputDevice) {
        (*device).seat = self;
        let dev_type = ffi::river_wlr_input_device_get_type((*device).wlr_device);
        match dev_type {
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD => {
                let keyboard = (*device).destroy_data as *mut crate::keyboard::Keyboard;
                if !keyboard.is_null() {
                    (*keyboard).set_group();
                    if !(*keyboard).group.is_null() {
                        ffi::wlr_seat_set_keyboard(self.wlr_seat, &mut (*(*keyboard).group).wlr_keyboard);
                        let focused_surface = ffi::river_wlr_seat_get_keyboard_focused_surface(self.wlr_seat);
                        if !focused_surface.is_null() {
                            self.keyboard_notify_enter(focused_surface);
                        }
                    }
                }
            }
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_POINTER => {
                ffi::wlr_cursor_attach_input_device(self.cursor.wlr_cursor, (*device).wlr_device);
            }
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TOUCH | ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TABLET => {
                ffi::wlr_cursor_attach_input_device(self.cursor.wlr_cursor, (*device).wlr_device);
                if !(*device).config.map_to_output.is_null() {
                    ffi::wlr_cursor_map_input_to_output(self.cursor.wlr_cursor, (*device).wlr_device, (*device).config.map_to_output);
                }
                ffi::wlr_cursor_map_input_to_region(self.cursor.wlr_cursor, (*device).wlr_device, &mut (*device).config.map_to_rectangle);
            }
            _ => {}
        }
        self.update_capabilities();
    }

    pub unsafe fn detach_device(&mut self, device: *mut crate::input_device::InputDevice) {
        ffi::wlr_cursor_detach_input_device(self.cursor.wlr_cursor, (*device).wlr_device);

        let dev_type = ffi::river_wlr_input_device_get_type((*device).wlr_device);
        if dev_type == ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD {
            let keyboard = (*device).destroy_data as *mut crate::keyboard::Keyboard;
            if !keyboard.is_null() {
                if !(*keyboard).group.is_null() {
                    let keys: Vec<u32> = (*keyboard).pressed.iter().cloned().collect();
                    crate::server::wl_list_remove(&mut (*keyboard).group_link as *mut ffi::wl_list as *mut crate::server::WlList);
                    (*(*keyboard).group).unref(&keys);
                    (*keyboard).group = std::ptr::null_mut();
                }
            }
        }
        self.update_capabilities();
    }

    pub unsafe fn handle_activity(&mut self) {
        let notifier = (*self.server).input_manager.idle_notifier;
        if !notifier.is_null() {
            ffi::wlr_idle_notifier_v1_notify_activity(notifier, self.wlr_seat);
        }
    }

    pub unsafe fn update_capabilities(&mut self) {
        let caps = ffi::wl_seat_capability_WL_SEAT_CAPABILITY_POINTER
            | ffi::wl_seat_capability_WL_SEAT_CAPABILITY_KEYBOARD;
        ffi::wlr_seat_set_capabilities(self.wlr_seat, caps);
    }

    pub unsafe fn focus(&mut self, new_focus: Focus) {
        if self.focused == new_focus {
            return;
        }

        // If an exclusive layer surface is active and scheduled for focus,
        // block any window manager or other client focus requests (via focus_requested)
        // from stealing focus back to a regular window or shell surface.
        if self.focus_requested {
            if let crate::layer_shell::LayerShellSeatFocus::Exclusive(key) = self.layer_shell.scheduled_focus {
                let server = self.server;
                if let Some(&layer_surface) = (*server).layer_shell.surfaces.get(key) {
                    let wlr_surf = (*(*layer_surface).wlr_layer_surface).surface;
                    if new_focus != Focus::LayerSurface(wlr_surf) {
                        if let Focus::Window(_) | Focus::ShellSurface(_) | Focus::OverrideRedirect(_) | Focus::None = new_focus {
                            log::info!("[FocusDebug] Blocking window manager focus request because Exclusive layer surface {:?} is active", key);
                            return;
                        }
                    }
                }
            }
        }

        match new_focus {
            Focus::None => log::info!("[FocusDebug] Seat::focus set to None"),
            Focus::LayerSurface(surface) => log::info!("[FocusDebug] Seat::focus set to LayerSurface {:?}", surface),
            Focus::Window(window) => {
                let title = if window.is_null() { "null".to_string() } else { (*window).get_title_string().unwrap_or_default() };
                let app_id = if window.is_null() { "null".to_string() } else { (*window).get_app_id_string().unwrap_or_default() };
                log::info!("[FocusDebug] Seat::focus set to Window {:?} (title={:?}, app_id={:?})", window, title, app_id);
            }
            Focus::LockSurface(lock) => log::info!("[FocusDebug] Seat::focus set to LockSurface {:?}", lock),
            Focus::OverrideRedirect(or) => log::info!("[FocusDebug] Seat::focus set to OverrideRedirect {:?}", or),
            Focus::ShellSurface(ss) => log::info!("[FocusDebug] Seat::focus set to ShellSurface {:?}", ss),
        }

        match self.focused {
            Focus::None => {}
            Focus::LayerSurface(_) | Focus::Window(_) | Focus::LockSurface(_) | Focus::OverrideRedirect(_) | Focus::ShellSurface(_) => {
                ffi::wlr_seat_keyboard_notify_clear_focus(self.wlr_seat);
                let focused_client = ffi::river_wlr_seat_get_pointer_focused_client(self.wlr_seat);
                if !focused_client.is_null() {
                    ffi::wlr_seat_pointer_notify_clear_focus(self.wlr_seat);
                }
            }
        }

        self.focused = new_focus;
        (*self.server).wm.update_status();

        match new_focus {
            Focus::None => {}
            Focus::LayerSurface(surface) => {
                if !surface.is_null() {
                    let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
                    if !kbd.is_null() {
                        let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            modifiers,
                        );
                    } else {
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                        );
                    }
                }

                let lx = self.cursor.x();
                let ly = self.cursor.y();
                let server = self.server;
                if let Some(result) = (*server).scene.at(lx, ly) {
                    if result.surface == surface {
                        ffi::wlr_seat_pointer_notify_enter(self.wlr_seat, surface, result.sx, result.sy);
                    }
                }
            }
            Focus::Window(window) => {
                // Focus root surface of window
                let surface = (*window).root_surface();
                if !surface.is_null() {
                    let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
                    if !kbd.is_null() {
                        let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            modifiers,
                        );
                    } else {
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                        );
                    }

                    let lx = self.cursor.x();
                    let ly = self.cursor.y();
                    let server = self.server;
                    if let Some(result) = (*server).scene.at(lx, ly) {
                        if result.surface == surface {
                            ffi::wlr_seat_pointer_notify_enter(self.wlr_seat, surface, result.sx, result.sy);
                        }
                    }
                }
            }
            Focus::LockSurface(lock_surface) => {
                let surface = (*(*lock_surface).wlr_lock_surface).surface;
                if !surface.is_null() {
                    let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
                    if !kbd.is_null() {
                        let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            modifiers,
                        );
                    } else {
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                        );
                    }

                    let lx = self.cursor.x();
                    let ly = self.cursor.y();
                    let server = self.server;
                    if let Some(result) = (*server).scene.at(lx, ly) {
                        if result.surface == surface {
                            ffi::wlr_seat_pointer_notify_enter(self.wlr_seat, surface, result.sx, result.sy);
                        }
                    }
                }
            }
            Focus::OverrideRedirect(or) => {
                let surface = (*(*or).xsurface).surface;
                if !surface.is_null() {
                    let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
                    if !kbd.is_null() {
                        let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            modifiers,
                        );
                    } else {
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                        );
                    }
                }
            }
            Focus::ShellSurface(shell_surface) => {
                let surface = (*shell_surface).surface;
                if !surface.is_null() {
                    let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
                    if !kbd.is_null() {
                        let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            modifiers,
                        );
                    } else {
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                        );
                    }
                }
            }
        }
        let target_surface = new_focus.surface();
        self.relay.focus(target_surface);
    }

    pub unsafe fn keyboard_notify_enter(&mut self, wlr_surface: *mut ffi::wlr_surface) {
        if wlr_surface.is_null() {
            return;
        }
        let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
        if !kbd.is_null() {
            let group_ptr = ffi::river_wlr_keyboard_get_data(kbd) as *mut crate::keyboard_group::KeyboardGroup;
            if !group_ptr.is_null() {
                let mut buffer = [0u32; 32];
                let mut count = 0;
                for &keycode in (*group_ptr).pressed.keys() {
                    if count >= 32 {
                        break;
                    }
                    buffer[count] = keycode + 8;
                    count += 1;
                }
                let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                ffi::wlr_seat_keyboard_notify_enter(
                    self.wlr_seat,
                    wlr_surface,
                    buffer.as_mut_ptr(),
                    count,
                    modifiers,
                );
                return;
            }
        }
        ffi::wlr_seat_keyboard_notify_enter(
            self.wlr_seat,
            wlr_surface,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
        );
    }

    pub unsafe fn keyboard_enter_or_leave(&mut self, target_surface: *mut ffi::wlr_surface) {
        if !target_surface.is_null() {
            self.keyboard_notify_enter(target_surface);
        } else {
            ffi::wlr_seat_keyboard_notify_clear_focus(self.wlr_seat);
        }
        self.relay.focus(target_surface);
    }

    pub unsafe fn manage_start(&mut self) {
        if self.destroying {
            Self::destroy(self);
            return;
        }

        self.focus_requested = false;
        self.layer_shell.manage_start();

        let wm_v1 = (*self.server).wm.object;
        if !wm_v1.is_null() {
            let new = self.object.is_null();
            if new {
                let client = ffi::wl_resource_get_client(wm_v1);
                let version = ffi::wl_resource_get_version(wm_v1);
                let seat_v1 = ffi::wl_resource_create(client, &ffi::zcce_seat_v1_interface, version, 0);
                if seat_v1.is_null() {
                    log::error!("out of memory creating zcce_seat_v1");
                    return;
                }
                self.object = seat_v1;
                
                ffi::wl_resource_set_implementation(
                    seat_v1,
                    &SEAT_INTERFACE as *const _ as *const _,
                    self as *mut Seat as *mut _,
                    Some(handle_destroy_resource),
                );
                
                ffi::wl_resource_post_event(wm_v1, ffi::ZCCE_WINDOW_MANAGER_V1_SEAT, seat_v1); // zcce_window_manager_v1.seat

                crate::server::wl_list_remove(&mut self.link_sent as *mut ffi::wl_list as *mut crate::server::WlList);
                let sent_seats = &mut (*self.server).wm.sent.seats as *mut ffi::wl_list as *mut crate::server::WlList;
                crate::server::wl_list_insert((*sent_seats).prev, &mut self.link_sent as *mut ffi::wl_list as *mut crate::server::WlList);
            }

            if new {
                let seat_v1 = self.object;
                let client = ffi::wl_resource_get_client(seat_v1);
                let wl_seat_name = ffi::wl_global_get_name(ffi::river_wlr_seat_get_global(self.wlr_seat), client);
                ffi::wl_resource_post_event(seat_v1, 1, wl_seat_name); // river_seat_v1.wl_seat
            }

            self.xkb_bindings_seat.manage_start();

            // Dispatch xkb binding events
            let bindings_head = &mut self.xkb_bindings as *mut ffi::wl_list as *mut crate::server::WlList;
            let mut curr = (*bindings_head).next;
            while curr != bindings_head {
                let next = (*curr).next;
                let binding = crate::container_of!(curr, crate::xkb_bindings::XkbBinding, link);
                for state in (*binding).wm_scheduled.state_changes.drain(..) {
                    match state {
                        crate::xkb_bindings::XkbBindingStateChange::None => {},
                        crate::xkb_bindings::XkbBindingStateChange::Pressed => {
                            if !(*binding).sent_pressed {
                                (*binding).sent_pressed = true;
                                ffi::wl_resource_post_event((*binding).object, 0);
                            }
                        },
                        crate::xkb_bindings::XkbBindingStateChange::StopRepeat => {
                            if (*binding).sent_pressed {
                                if ffi::wl_resource_get_version((*binding).object) >= 2 {
                                    ffi::wl_resource_post_event((*binding).object, 2);
                                }
                            }
                        },
                        crate::xkb_bindings::XkbBindingStateChange::Released => {
                            if (*binding).sent_pressed {
                                (*binding).sent_pressed = false;
                                ffi::wl_resource_post_event((*binding).object, 1);
                            }
                        },
                    }
                }
                curr = next;
            }

            // Dispatch pointer binding events
            let ptr_bindings_head = &mut self.pointer_bindings as *mut ffi::wl_list as *mut crate::server::WlList;
            let mut curr_ptr = (*ptr_bindings_head).next;
            while curr_ptr != ptr_bindings_head {
                let next = (*curr_ptr).next;
                let binding = crate::container_of!(curr_ptr, crate::pointer_binding::PointerBinding, link);
                for state in (*binding).wm_scheduled.state_changes.drain(..) {
                    match state {
                        crate::pointer_binding::PointerBindingStateChange::None => {}
                        crate::pointer_binding::PointerBindingStateChange::Pressed => {
                            if !(*binding).sent_pressed {
                                (*binding).sent_pressed = true;
                                ffi::wl_resource_post_event((*binding).object, 0); // pressed
                            }
                        }
                        crate::pointer_binding::PointerBindingStateChange::Released => {
                            if (*binding).sent_pressed {
                                (*binding).sent_pressed = false;
                                ffi::wl_resource_post_event((*binding).object, 1); // released
                            }
                        }
                    }
                }
                curr_ptr = next;
            }

            // Dispatch pointer operation events
            if let Some(ref mut op) = self.op {
                let dx = op.x - op.start_x;
                let dy = op.y - op.start_y;
                ffi::wl_resource_post_event(self.object, 6, dx, dy); // op_delta

                if self.op_release && !op.sent_release {
                    ffi::wl_resource_post_event(self.object, 7); // op_release
                    self.op_release = false;
                    op.sent_release = true;
                }
            }

            // Dispatch pointer position event
            if ffi::wl_resource_get_version(self.object) >= 2 {
                let x = (*self.cursor.wlr_cursor).x as i32;
                let y = (*self.cursor.wlr_cursor).y as i32;
                if x != self.wm_sent_x || y != self.wm_sent_y {
                    ffi::wl_resource_post_event(self.object, 8, x, y); // pointer_position
                    self.wm_sent_x = x;
                    self.wm_sent_y = y;
                }
            }
        } else {
            crate::server::wl_list_remove(&mut self.link_sent as *mut ffi::wl_list as *mut crate::server::WlList);
            let sent_seats = &mut (*self.server).wm.sent.seats as *mut ffi::wl_list as *mut crate::server::WlList;
            crate::server::wl_list_insert((*sent_seats).prev, &mut self.link_sent as *mut ffi::wl_list as *mut crate::server::WlList);
        }
    }

    pub unsafe fn manage_finish(&mut self) {
        self.xkb_bindings_seat.manage_finish();

        if (*self.server).lock_manager.state != crate::lock_manager::LockState::Unlocked {
            return;
        }

        match self.layer_shell.sent_focus {
            crate::layer_shell::LayerShellSeatFocus::Exclusive(key) => {
                let server = self.server;
                if let Some(&layer_surface) = (*server).layer_shell.surfaces.get(key) {
                    let wlr_surf = (*(*layer_surface).wlr_layer_surface).surface;
                    self.focus(Focus::LayerSurface(wlr_surf));
                }
            }
            crate::layer_shell::LayerShellSeatFocus::NonExclusive(key) => {
                if !self.focus_requested {
                    let server = self.server;
                    if let Some(&layer_surface) = (*server).layer_shell.surfaces.get(key) {
                        let wlr_surf = (*(*layer_surface).wlr_layer_surface).surface;
                        self.focus(Focus::LayerSurface(wlr_surf));
                    }
                } else {
                    self.layer_shell.scheduled_focus = crate::layer_shell::LayerShellSeatFocus::None;
                    (*self.server).wm.dirty_windowing();
                }
            }
            crate::layer_shell::LayerShellSeatFocus::None => {}
        }
    }

    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_post_event(self.object, 0); // river_seat_v1.removed
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_SEAT_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
            (*self.server).wm.dirty_windowing();
        }
        self.layer_shell.make_inert();
        self.xkb_bindings_seat.make_inert();
    }

    pub unsafe fn match_xkb_binding(
        &self,
        keycode: u32,
        wlr_keyboard: *mut ffi::wlr_keyboard,
    ) -> Option<*mut crate::xkb_bindings::XkbBinding> {
        let xkb_state = (*wlr_keyboard).xkb_state;
        if xkb_state.is_null() {
            return None;
        }
        
        let modifiers = ffi::wlr_keyboard_get_modifiers(wlr_keyboard);
        
        let bindings_head = &self.xkb_bindings as *const ffi::wl_list as *mut crate::server::WlList;
        let mut curr = (*bindings_head).next;
        let mut found: Option<*mut crate::xkb_bindings::XkbBinding> = None;
        
        while curr != bindings_head {
            let next = (*curr).next;
            let binding = crate::container_of!(curr, crate::xkb_bindings::XkbBinding, link);
            if (*binding).match_keycode(keycode, modifiers, xkb_state, false) {
                if found.is_none() {
                    found = Some(binding);
                } else {
                    log::debug!("already found a matching xkb_binding, ignoring additional match");
                }
            }
            curr = next;
        }
        
        if found.is_some() {
            return found;
        }
        
        curr = (*bindings_head).next;
        while curr != bindings_head {
            let next = (*curr).next;
            let binding = crate::container_of!(curr, crate::xkb_bindings::XkbBinding, link);
            if (*binding).match_keycode(keycode, modifiers, xkb_state, true) {
                if found.is_none() {
                    found = Some(binding);
                } else {
                    log::debug!("already found a matching xkb_binding, ignoring additional match");
                }
            }
            curr = next;
        }
        
        found
    }

    pub unsafe fn match_pointer_binding(
        &self,
        button: u32,
    ) -> Option<*mut crate::pointer_binding::PointerBinding> {
        let wlr_keyboard = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
        if wlr_keyboard.is_null() {
            return None;
        }
        let modifiers = ffi::wlr_keyboard_get_modifiers(wlr_keyboard);

        let bindings_head = &self.pointer_bindings as *const ffi::wl_list as *mut crate::server::WlList;
        let mut curr = (*bindings_head).next;
        let mut found: Option<*mut crate::pointer_binding::PointerBinding> = None;

        while curr != bindings_head {
            let next = (*curr).next;
            let binding = crate::container_of!(curr, crate::pointer_binding::PointerBinding, link);
            if (*binding).match_binding(button, modifiers) {
                if found.is_none() {
                    found = Some(binding);
                } else {
                    log::debug!("already found a matching pointer binding, ignoring additional match");
                }
            }
            curr = next;
        }
        found
    }

    pub unsafe fn op_update(&mut self, x: i32, y: i32) {
        if let Some(ref mut op) = self.op {
            op.x = x;
            op.y = y;
            let dx = op.x - op.start_x;
            let dy = op.y - op.start_y;
            
            let win = op.window_ptr;
            if !win.is_null() && !(*win).closed {
                if (*win).tiling_mode != crate::tiling::TilingMode::Floating {
                    (*win).tiling_mode = crate::tiling::TilingMode::Floating;
                    (*win).mode_locked = true;
                }
                
                match op.op_type {
                    PointerOpType::Move => {
                        (*win).rendering_requested.x = op.start_win_x + dx;
                        (*win).rendering_requested.y = op.start_win_y + dy;
                        (*win).box_geom.x = op.start_win_x + dx;
                        (*win).box_geom.y = op.start_win_y + dy;
                    }
                    PointerOpType::Resize { edges } => {
                        let mut new_w = op.start_win_w;
                        let mut new_h = op.start_win_h;
                        let mut new_x = op.start_win_x;
                        let mut new_y = op.start_win_y;

                        if edges.left {
                            let w = std::cmp::max(50, op.start_win_w as i32 - dx) as u32;
                            let dw = w as i32 - op.start_win_w as i32;
                            new_w = w;
                            new_x = op.start_win_x - dw;
                        } else if edges.right {
                            new_w = std::cmp::max(50, op.start_win_w as i32 + dx) as u32;
                        }

                        if edges.top {
                            let h = std::cmp::max(50, op.start_win_h as i32 - dy) as u32;
                            let dh = h as i32 - op.start_win_h as i32;
                            new_h = h;
                            new_y = op.start_win_y - dh;
                        } else if edges.bottom {
                            new_h = std::cmp::max(50, op.start_win_h as i32 + dy) as u32;
                        }

                        (*win).rendering_requested.x = new_x;
                        (*win).rendering_requested.y = new_y;
                        (*win).box_geom.x = new_x;
                        (*win).box_geom.y = new_y;

                        (*win).wm_requested.resizing = true;
                        (*win).wm_requested.dimensions = Some(crate::window::Dimensions {
                            width: new_w,
                            height: new_h,
                        });
                        (*win).wm_requested.bounds = crate::window::Dimensions {
                            width: new_w,
                            height: new_h,
                        };
                        (*win).set_dimensions(new_w, new_h);
                    }
                }
                (*win).manage_finish();
            }
            (*self.server).wm.dirty_windowing();
        }
    }

    pub unsafe fn op_end(&mut self) {
        if let Some(op) = self.op.take() {
            log::debug!("end seat op");
            let win = op.window_ptr;
            if !win.is_null() && !(*win).closed {
                if let PointerOpType::Resize { .. } = op.op_type {
                    (*win).wm_requested.resizing = false;
                    (*win).manage_finish();
                    (*self.server).wm.dirty_windowing();
                }
            }
            match op.input {
                SeatOpInput::Pointer => {
                    self.cursor.op_end_pointer();
                }
            }
        }
    }
}

unsafe extern "C" fn handle_request_set_cursor(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, request_set_cursor);
    let event = data as *mut ffi::wlr_seat_pointer_request_set_cursor_event;
    
    let focused_client = ffi::river_wlr_seat_get_pointer_focused_client(seat.wlr_seat);
    
    let event_client = ffi::river_wlr_seat_client_get_client((*event).seat_client);
    let wm_client = if !(*seat.server).wm.object.is_null() {
        ffi::wl_resource_get_client((*seat.server).wm.object)
    } else {
        std::ptr::null_mut()
    };
    let is_wm = !wm_client.is_null() && event_client == wm_client;

    if focused_client == (*event).seat_client || is_wm {
        ffi::wlr_cursor_set_surface(
            seat.cursor.wlr_cursor,
            (*event).surface,
            (*event).hotspot_x,
            (*event).hotspot_y,
        );
    }
}

unsafe extern "C" fn handle_request_set_selection(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, request_set_selection);
    let event = data as *mut ffi::wlr_seat_request_set_selection_event;
    ffi::wlr_seat_set_selection(seat.wlr_seat, (*event).source, (*event).serial);
}

unsafe extern "C" fn handle_request_start_drag(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, request_start_drag);
    let event = data as *mut ffi::wlr_seat_request_start_drag_event;

    assert!(seat.drag == DragState::None);

    if ffi::wlr_seat_validate_pointer_grab_serial(seat.wlr_seat, (*event).origin, (*event).serial) {
        ffi::wlr_seat_start_pointer_drag(seat.wlr_seat, (*event).drag, (*event).serial);
        return;
    }

    let mut point: *mut ffi::wlr_touch_point = std::ptr::null_mut();
    if ffi::wlr_seat_validate_touch_grab_serial(seat.wlr_seat, (*event).origin, (*event).serial, &mut point) {
        ffi::wlr_seat_start_touch_drag(seat.wlr_seat, (*event).drag, (*event).serial, point);
        return;
    }

    let source = ffi::river_wlr_drag_get_source((*event).drag);
    if !source.is_null() {
        ffi::wlr_data_source_destroy(source);
    }
}

unsafe extern "C" fn handle_start_drag(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, start_drag);
    let wlr_drag = data as *mut ffi::wlr_drag;

    assert!(seat.drag == DragState::None);
    let grab_type = ffi::river_wlr_drag_get_grab_type(wlr_drag);
    match grab_type {
        ffi::wlr_drag_grab_type_WLR_DRAG_GRAB_KEYBOARD_POINTER => {
            seat.drag = DragState::Pointer;
        }
        ffi::wlr_drag_grab_type_WLR_DRAG_GRAB_KEYBOARD_TOUCH => {
            seat.drag = DragState::Touch;
        }
        _ => {}
    }

    let drag_destroy_ptr = &mut seat.drag_destroy as *mut ffi::wl_listener as *mut WlListener;
    (*drag_destroy_ptr).notify = Some(handle_drag_destroy);
    wl_signal_add(
        ffi::river_wlr_drag_get_destroy_signal(wlr_drag),
        &mut seat.drag_destroy,
    );

    let wlr_drag_icon = ffi::river_wlr_drag_get_icon(wlr_drag);
    if !wlr_drag_icon.is_null() {
        if let Err(err) = crate::drag_icon::DragIcon::create(wlr_drag_icon, &mut seat.cursor) {
            log::error!("Failed to create drag icon: {}", err);
            let seat_client = ffi::river_wlr_drag_get_seat_client(wlr_drag);
            if !seat_client.is_null() {
                let client = ffi::river_wlr_seat_client_get_client(seat_client);
                if !client.is_null() {
                    ffi::wl_client_post_no_memory(client);
                }
            }
        }
    }
}

unsafe extern "C" fn handle_drag_destroy(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, drag_destroy);
    wl_listener_remove(&mut seat.drag_destroy);

    match seat.drag {
        DragState::None => unreachable!(),
        DragState::Pointer => {
            seat.cursor.update_state();
        }
        DragState::Touch => {}
    }
    seat.drag = DragState::None;
}

unsafe extern "C" fn handle_request_set_primary_selection(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, request_set_primary_selection);
    let event = data as *mut ffi::wlr_seat_request_set_primary_selection_event;
    ffi::wlr_seat_set_primary_selection(seat.wlr_seat, (*event).source, (*event).serial);
}

unsafe extern "C" fn seat_destroy(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn seat_focus_window(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    window_resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    (*seat).focus_requested = true;
    if window_resource.is_null() {
        (*seat).focus(Focus::None);
        return;
    }
    let window = ffi::wl_resource_get_user_data(window_resource) as *mut crate::window::Window;
    if !window.is_null() {
        (*seat).focus(Focus::Window(window));
    }
}

unsafe extern "C" fn seat_focus_shell_surface(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    shell_surface_resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    (*seat).focus_requested = true;
    if shell_surface_resource.is_null() {
        (*seat).focus(Focus::None);
        return;
    }
    let shell_surface = ffi::wl_resource_get_user_data(shell_surface_resource) as *mut crate::shell_surface::ShellSurface;
    if !shell_surface.is_null() {
        (*seat).focus(Focus::ShellSurface(shell_surface));
    }
}

unsafe extern "C" fn seat_clear_focus(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if !seat.is_null() {
        if (*(*seat).server).wm.ensure_windowing() {
            (*seat).focus_requested = true;
            (*seat).focus(Focus::None);
        }
    }
}

unsafe extern "C" fn seat_op_start_pointer(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    if (*seat).op.is_none() {
        log::debug!("start seat op pointer");
        let cursor_x = (*(*seat).cursor.wlr_cursor).x;
        let cursor_y = (*(*seat).cursor.wlr_cursor).y;
        (*seat).op = Some(SeatOp {
            sent_release: false,
            input: SeatOpInput::Pointer,
            start_x: cursor_x as i32,
            start_y: cursor_y as i32,
            x: cursor_x as i32,
            y: cursor_y as i32,
            window_ptr: std::ptr::null_mut(),
            op_type: PointerOpType::Move,
            start_win_x: 0,
            start_win_y: 0,
            start_win_w: 0,
            start_win_h: 0,
        });
        (*seat).cursor.op_start_pointer();
    }
}

unsafe extern "C" fn seat_op_end(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    (*seat).op_end();
}

unsafe extern "C" fn seat_get_pointer_binding(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    button: u32,
    modifiers: u32,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    let version = ffi::wl_resource_get_version(resource) as u32;
    if let Err(err) = crate::pointer_binding::PointerBinding::create(
        seat,
        client,
        version,
        id,
        button,
        modifiers,
    ) {
        log::error!("failed to create pointer binding: {}", err);
        ffi::wl_client_post_no_memory(client);
    }
}

unsafe extern "C" fn seat_set_xcursor_theme(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    name: *const ::std::os::raw::c_char,
    size: u32,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if let Err(err) = (*seat).cursor.set_theme(name, size) {
        log::error!("failed to set xcursor theme: {}", err);
        ffi::wl_client_post_no_memory(client);
    }
}

unsafe extern "C" fn seat_pointer_warp(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    let cursor = &mut (*seat).cursor;
    ffi::wlr_cursor_warp_absolute(cursor.wlr_cursor, std::ptr::null_mut(), x as f64, y as f64);
}

static SEAT_INTERFACE: ffi::zcce_seat_v1_interface = ffi::zcce_seat_v1_interface {
    destroy: Some(seat_destroy),
    focus_window: Some(seat_focus_window),
    focus_shell_surface: Some(seat_focus_shell_surface),
    clear_focus: Some(seat_clear_focus),
    op_start_pointer: Some(seat_op_start_pointer),
    op_end: Some(seat_op_end),
    get_pointer_binding: Some(seat_get_pointer_binding),
    set_xcursor_theme: Some(seat_set_xcursor_theme),
    pointer_warp: Some(seat_pointer_warp),
};

unsafe extern "C" fn seat_inert_focus_window(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _window_resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn seat_inert_focus_shell_surface(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _shell_surface_resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn seat_inert_clear_focus(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn seat_inert_op_start_pointer(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn seat_inert_op_end(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn seat_inert_get_pointer_binding(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _id: u32,
    _button: u32,
    _modifiers: u32,
) {}

unsafe extern "C" fn seat_inert_set_xcursor_theme(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _name: *const ::std::os::raw::c_char,
    _size: u32,
) {}

unsafe extern "C" fn seat_inert_pointer_warp(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _x: i32,
    _y: i32,
) {}

static INERT_SEAT_INTERFACE: ffi::zcce_seat_v1_interface = ffi::zcce_seat_v1_interface {
    destroy: Some(seat_destroy),
    focus_window: Some(seat_inert_focus_window),
    focus_shell_surface: Some(seat_inert_focus_shell_surface),
    clear_focus: Some(seat_inert_clear_focus),
    op_start_pointer: Some(seat_inert_op_start_pointer),
    op_end: Some(seat_inert_op_end),
    get_pointer_binding: Some(seat_inert_get_pointer_binding),
    set_xcursor_theme: Some(seat_inert_set_xcursor_theme),
    pointer_warp: Some(seat_inert_pointer_warp),
};

unsafe extern "C" fn handle_destroy_resource(resource: *mut ffi::wl_resource) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if !seat.is_null() {
        if (*seat).object != resource {
            return;
        }
        (*seat).object = std::ptr::null_mut();
    }
}
