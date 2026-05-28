// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, wl_signal_add, WlListener, wl_listener_remove};
use crate::seat::Seat;

pub struct InputManager {
    pub server: *mut Server,
    pub default_seat: *mut Seat,
    pub seats: ffi::wl_list,
    
    pub idle_notifier: *mut ffi::wlr_idle_notifier_v1,
    pub relative_pointer_manager: *mut ffi::wlr_relative_pointer_manager_v1,
    pub pointer_gestures: *mut ffi::wlr_pointer_gestures_v1,
    pub virtual_pointer_manager: *mut ffi::wlr_virtual_pointer_manager_v1,
    pub virtual_keyboard_manager: *mut ffi::wlr_virtual_keyboard_manager_v1,
    pub pointer_constraints: *mut ffi::wlr_pointer_constraints_v1,
    pub input_method_manager: *mut ffi::wlr_input_method_manager_v2,
    pub text_input_manager: *mut ffi::wlr_text_input_manager_v3,
    pub tablet_manager: *mut ffi::wlr_tablet_manager_v2,

    pub new_input_listener: ffi::wl_listener,
    pub new_text_input: ffi::wl_listener,
    pub new_input_method: ffi::wl_listener,
}

impl Default for InputManager {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl InputManager {
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        let wl_server = (*server).wl_server;

        ffi::wl_list_init(&mut self.seats);

        self.idle_notifier = ffi::wlr_idle_notifier_v1_create(wl_server);
        self.relative_pointer_manager = ffi::wlr_relative_pointer_manager_v1_create(wl_server);
        self.pointer_gestures = ffi::wlr_pointer_gestures_v1_create(wl_server);
        self.virtual_pointer_manager = ffi::wlr_virtual_pointer_manager_v1_create(wl_server);
        self.virtual_keyboard_manager = ffi::wlr_virtual_keyboard_manager_v1_create(wl_server);
        self.pointer_constraints = ffi::wlr_pointer_constraints_v1_create(wl_server);
        self.input_method_manager = ffi::wlr_input_method_manager_v2_create(wl_server);
        self.text_input_manager = ffi::wlr_text_input_manager_v3_create(wl_server);
        self.tablet_manager = ffi::wlr_tablet_v2_create(wl_server);

        // Create default seat
        self.default_seat = Seat::create(server, "default")?;

        let new_input_ptr = &mut self.new_input_listener as *mut ffi::wl_listener as *mut WlListener;
        (*new_input_ptr).notify = Some(handle_new_input);
        
        let signal = ffi::river_wlr_backend_get_new_input_signal((*server).backend);
        wl_signal_add(signal, &mut self.new_input_listener);

        // Connect new_text_input listener
        let new_text_input_ptr = &mut self.new_text_input as *mut ffi::wl_listener as *mut WlListener;
        (*new_text_input_ptr).notify = Some(handle_new_text_input);
        wl_signal_add(&mut (*self.text_input_manager).events.new_text_input, &mut self.new_text_input);

        // Connect new_input_method listener
        let new_input_method_ptr = &mut self.new_input_method as *mut ffi::wl_listener as *mut WlListener;
        (*new_input_method_ptr).notify = Some(handle_new_input_method);
        wl_signal_add(&mut (*self.input_method_manager).events.new_input_method, &mut self.new_input_method);

        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        if !self.default_seat.is_null() {
            Seat::destroy(self.default_seat);
            self.default_seat = std::ptr::null_mut();
        }
        wl_listener_remove(&mut self.new_input_listener);
        wl_listener_remove(&mut self.new_text_input);
        wl_listener_remove(&mut self.new_input_method);
    }
}

unsafe extern "C" fn handle_new_input(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let im = &mut *crate::container_of!(listener, InputManager, new_input_listener);
    let wlr_device = data as *mut ffi::wlr_input_device;

    log::info!("new input device connected");

    let device = crate::input_device::InputDevice::new(im.default_seat, wlr_device, false);
    
    let dev_type = ffi::river_wlr_input_device_get_type(wlr_device);
    if dev_type == ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD {
        crate::keyboard::Keyboard::create(device);
    } else if dev_type == ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TABLET {
        let _ = crate::tablet::Tablet::create(im.default_seat, wlr_device, false);
    }
}

unsafe extern "C" fn handle_new_text_input(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let _im = crate::container_of!(listener, InputManager, new_text_input);
    let wlr_text_input = data as *mut ffi::wlr_text_input_v3;

    if let Err(e) = crate::text_input::TextInput::create(wlr_text_input) {
        log::error!("failed to create text input: {}", e);
        ffi::wl_resource_post_no_memory((*wlr_text_input).resource);
    }
}

unsafe extern "C" fn handle_new_input_method(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let im = &mut *crate::container_of!(listener, InputManager, new_input_method);
    let input_method = data as *mut ffi::wlr_input_method_v2;
    let seat = ffi::river_wlr_seat_get_data((*input_method).seat) as *mut Seat;
    if !seat.is_null() {
        (*seat).relay.new_input_method(input_method);
    } else {
        // Fallback to default seat if input_method.seat has no data set yet
        (*im.default_seat).relay.new_input_method(input_method);
    }
}
