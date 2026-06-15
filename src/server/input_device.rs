// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::seat::Seat;
use crate::server::{WlListener, wl_listener_remove, WlList, wl_list_insert, wl_list_remove};

pub struct InputDeviceConfig {
    pub scroll_factor: f64,
    pub map_to_output: *mut ffi::wlr_output,
    pub map_to_rectangle: ffi::wlr_box,
}

pub struct InputDevice {
    pub seat: *mut Seat,
    pub wlr_device: *mut ffi::wlr_input_device,
    pub virtual_device: bool,
    pub destroy_listener: ffi::wl_listener,
    pub config: InputDeviceConfig,
    pub destroy_fn: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    pub destroy_data: *mut std::ffi::c_void,

    pub objects: ffi::wl_list, // list of InputDeviceObject
    pub link: ffi::wl_list,    // link inside InputManager::devices
    pub libinput: Option<Box<crate::libinput_device::LibinputDevice>>,
    pub xkb_keyboard: Option<Box<crate::xkb_keyboard::XkbKeyboard>>,
}

pub struct InputDeviceObject {
    pub device: *mut InputDevice,
    pub resource: *mut ffi::wl_resource,
    pub link: ffi::wl_list,
}

impl InputDevice {
    pub unsafe fn new(
        seat: *mut Seat,
        wlr_device: *mut ffi::wlr_input_device,
        virtual_device: bool,
    ) -> *mut Self {
        let server = (*seat).server;

        let device = Box::into_raw(Box::new(Self {
            seat,
            wlr_device,
            virtual_device,
            destroy_listener: std::mem::zeroed(),
            config: InputDeviceConfig {
                scroll_factor: 1.0,
                map_to_output: std::ptr::null_mut(),
                map_to_rectangle: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
            },
            destroy_fn: None,
            destroy_data: std::ptr::null_mut(),
            objects: std::mem::zeroed(),
            link: std::mem::zeroed(),
            libinput: None,
            xkb_keyboard: None,
        }));

        ffi::wl_list_init(&mut (*device).objects);
        ffi::wl_list_init(&mut (*device).link);

        // Insert into InputManager::devices
        let manager_devices_head = &mut (*server).input_manager.devices as *mut ffi::wl_list as *mut WlList;
        wl_list_insert(manager_devices_head, &mut (*device).link as *mut ffi::wl_list as *mut WlList);

        ffi::river_wlr_input_device_set_data(wlr_device, device as *mut _);

        let destroy_listener_ptr = &mut (*device).destroy_listener as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_listener_ptr).notify = Some(handle_device_destroy);
        
        let destroy_signal = ffi::river_wlr_input_device_get_destroy_signal(wlr_device);
        crate::server::wl_signal_add(destroy_signal, &mut (*device).destroy_listener);

        if !virtual_device {
            let config_objects = &mut (*server).input_manager.objects as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*config_objects).next;
            while curr != config_objects {
                let next = (*curr).next;
                let im_obj = crate::container_of!(curr, crate::input_manager::InputManagerObject, link);
                (*device).create_object((*im_obj).resource);
                curr = next;
            }

            let handle = if ffi::wlr_input_device_is_libinput(wlr_device) {
                ffi::wlr_libinput_get_device_handle(wlr_device)
            } else {
                std::ptr::null_mut()
            };
            if !handle.is_null() {
                let libinput_dev = crate::libinput_device::LibinputDevice::init(device, handle);
                let wm = &(*(*seat).server).wm;
                libinput_dev.apply_config(&wm.input_config);
                (*device).libinput = Some(libinput_dev);
            }

            let dev_type = ffi::river_wlr_input_device_get_type(wlr_device);
            if dev_type == ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD {
                (*device).xkb_keyboard = Some(crate::xkb_keyboard::XkbKeyboard::init(device));
            }
        }

        // Output mapping logic for pointers and touch screens (as done in Zig version)
        let output_name = match ffi::river_wlr_input_device_get_type(wlr_device) {
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_POINTER => {
                let ptr = ffi::wlr_pointer_from_input_device(wlr_device);
                if !ptr.is_null() && !(*ptr).output_name.is_null() {
                    Some(std::ffi::CStr::from_ptr((*ptr).output_name))
                } else {
                    None
                }
            }
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TOUCH => {
                let touch = ffi::wlr_touch_from_input_device(wlr_device);
                if !touch.is_null() && !(*touch).output_name.is_null() {
                    Some(std::ffi::CStr::from_ptr((*touch).output_name))
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(name) = output_name {
            let outputs_head = &mut (*server).om.outputs as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*outputs_head).next;
            while curr != outputs_head {
                let next = (*curr).next;
                let output = crate::container_of!(curr, crate::output::Output, link);
                let wlr_output = (*output).wlr_output;
                if !wlr_output.is_null() {
                    let wlr_name = std::ffi::CStr::from_ptr(ffi::river_wlr_output_get_name(wlr_output));
                    if wlr_name == name {
                        (*device).config.map_to_output = wlr_output;
                        break;
                    }
                }
                curr = next;
            }
        }

        device
    }

    pub unsafe fn assign_to_seat(&mut self, new_seat: *mut Seat) {
        let old_seat = self.seat;
        if old_seat == new_seat {
            return;
        }
        if !old_seat.is_null() {
            (*old_seat).detach_device(self);
        }
        self.seat = new_seat;
        if !new_seat.is_null() {
            (*new_seat).attach_device(self);
        }
        if !old_seat.is_null() {
            (*old_seat).update_capabilities();
        }
        if !new_seat.is_null() {
            (*new_seat).update_capabilities();
        }
    }

    pub unsafe fn active_mapping(&self) -> ffi::wlr_box {
        let mut mapping = self.config.map_to_rectangle;
        if !ffi::wlr_box_empty(&mapping) {
            return mapping;
        }
        if !self.config.map_to_output.is_null() {
            let server = (*self.seat).server;
            ffi::wlr_output_layout_get_box((*server).om.output_layout, self.config.map_to_output, &mut mapping);
        }
        mapping
    }

    pub unsafe fn create_object(&mut self, im_v1_resource: *mut ffi::wl_resource) {
        if self.virtual_device {
            return;
        }

        let dev_type = ffi::river_wlr_input_device_get_type(self.wlr_device);
        let proto_type = match dev_type {
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD => 0, // keyboard
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_POINTER => 1,  // pointer
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TOUCH => 2,    // touch
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TABLET => 3,   // tablet
            _ => return,
        };

        let client = ffi::wl_resource_get_client(im_v1_resource);
        let version = ffi::wl_resource_get_version(im_v1_resource);

        let resource = ffi::wl_resource_create(
            client,
            &ffi::river_input_device_v1_interface,
            version,
            0,
        );
        if resource.is_null() {
            log::error!("out of memory creating river_input_device_v1");
            ffi::wl_client_post_no_memory(client);
            return;
        }

        let obj = Box::into_raw(Box::new(InputDeviceObject {
            device: self,
            resource,
            link: std::mem::zeroed(),
        }));

        ffi::wl_resource_set_implementation(
            resource,
            &INPUT_DEVICE_INTERFACE as *const _ as *const _,
            obj as *mut _,
            Some(handle_device_object_destroy),
        );

        // Insert into self.objects
        let list_head = &mut self.objects as *mut ffi::wl_list as *mut WlList;
        wl_list_insert(list_head, &mut (*obj).link as *mut ffi::wl_list as *mut WlList);

        // Send input_device event to the manager resource
        ffi::wl_resource_post_event(im_v1_resource, 1, resource); // opcode 1 is input_device in river_input_manager_v1

        // Send type and name to client
        ffi::wl_resource_post_event(resource, 1, proto_type); // type event
        
        let name = ffi::river_wlr_input_device_get_name(self.wlr_device);
        let name_ptr = if !name.is_null() { name } else { b"\0".as_ptr() as *const _ };
        ffi::wl_resource_post_event(resource, 2, name_ptr); // name event

        if version >= 2 {
            ffi::wl_resource_post_event(resource, 3); // done event
        }

        // Connect client-side libinput device if matching exists
        if let Some(ref mut libinput) = self.libinput {
            let libinput_config_objects = &mut (*(*self.seat).server).libinput_config.objects as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*libinput_config_objects).next;
            while curr != libinput_config_objects {
                let next = (*curr).next;
                let config_obj = crate::container_of!(curr, crate::libinput_config::LibinputConfigObject, link);
                if ffi::wl_resource_get_client((*config_obj).resource) == client {
                    libinput.create_object((*config_obj).resource);
                }
                curr = next;
            }
        }
    }
}

unsafe extern "C" fn handle_device_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let device_ptr = crate::container_of!(listener, InputDevice, destroy_listener) as *mut InputDevice;
    let device = &mut *device_ptr;
    
    log::debug!(
        "removed input device: {:?}",
        ffi::river_wlr_input_device_get_type(device.wlr_device)
    );

    // Detach from seat if attached
    if !device.seat.is_null() {
        (*device.seat).detach_device(device);
        (*device.seat).update_capabilities();
    }

    // Free objects and set inert
    let objects_head = &mut device.objects as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*objects_head).next;
    while curr != objects_head {
        let next = (*curr).next;
        let obj = crate::container_of!(curr, InputDeviceObject, link);

        wl_list_remove(curr);
        ffi::wl_list_init(curr as *mut ffi::wl_list);

        ffi::wl_resource_post_event((*obj).resource, 0); // removed event

        ffi::wl_resource_set_implementation(
            (*obj).resource,
            &INPUT_DEVICE_INERT_INTERFACE as *const _ as *const _,
            obj as *mut _,
            Some(handle_device_object_destroy),
        );

        curr = next;
    }

    if let Some(mut libinput) = device.libinput.take() {
        libinput.deinit();
    }

    if let Some(mut xkb_kbd) = device.xkb_keyboard.take() {
        xkb_kbd.deinit();
    }

    // Call custom destroy callback if set (e.g. to clean up Tablet/Keyboard wrappers)
    if let Some(destroy_fn) = device.destroy_fn {
        destroy_fn(device.destroy_data);
    }

    // Remove destroy listener
    wl_listener_remove(&mut device.destroy_listener);

    // Remove from InputManager::devices
    wl_list_remove(&mut device.link as *mut ffi::wl_list as *mut WlList);

    // Free wrapper memory
    let _boxed = Box::from_raw(device_ptr);
}

unsafe extern "C" fn handle_device_object_destroy(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputDeviceObject;
    if !obj.is_null() {
        wl_list_remove(&mut (*obj).link as *mut ffi::wl_list as *mut WlList);
        let _ = Box::from_raw(obj);
    }
}

static INPUT_DEVICE_INTERFACE: ffi::river_input_device_v1_interface = ffi::river_input_device_v1_interface {
    destroy: Some(input_device_destroy),
    assign_to_seat: Some(input_device_assign_to_seat),
    set_repeat_info: Some(input_device_set_repeat_info),
    set_scroll_factor: Some(input_device_set_scroll_factor),
    map_to_output: Some(input_device_map_to_output),
    map_to_rectangle: Some(input_device_map_to_rectangle),
};

static INPUT_DEVICE_INERT_INTERFACE: ffi::river_input_device_v1_interface = ffi::river_input_device_v1_interface {
    destroy: Some(input_device_destroy),
    assign_to_seat: None,
    set_repeat_info: None,
    set_scroll_factor: None,
    map_to_output: None,
    map_to_rectangle: None,
};

unsafe extern "C" fn input_device_destroy(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn input_device_assign_to_seat(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    name: *const std::os::raw::c_char,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputDeviceObject;
    if obj.is_null() {
        return;
    }
    let device = (*obj).device;
    if name.is_null() {
        return;
    }
    let server = (*(*device).seat).server;
    let seats_head = &mut (*server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*seats_head).next;
    let mut found = false;
    let name_str = std::ffi::CStr::from_ptr(name).to_string_lossy();
    while curr != seats_head {
        let next = (*curr).next;
        let seat = crate::container_of!(curr, Seat, link);
        let seat_name = std::ffi::CStr::from_ptr(ffi::river_wlr_seat_get_name((*seat).wlr_seat)).to_string_lossy();
        if seat_name == name_str {
            (*device).assign_to_seat(seat);
            found = true;
            break;
        }
        curr = next;
    }
    if !found {
        log::info!("client requested input device be assigned to non-existent seat '{}'", name_str);
    }
}

unsafe extern "C" fn input_device_set_repeat_info(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    rate: i32,
    delay: i32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputDeviceObject;
    if obj.is_null() {
        return;
    }
    let device = (*obj).device;
    if rate < 0 || delay < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_input_device_v1_error_RIVER_INPUT_DEVICE_V1_ERROR_INVALID_REPEAT_INFO,
            b"negative rate/delay\0".as_ptr() as *const _,
        );
        return;
    }
    let dev_type = ffi::river_wlr_input_device_get_type((*device).wlr_device);
    if dev_type == ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD {
        let keyboard = (*device).destroy_data as *mut crate::keyboard::Keyboard;
        if !keyboard.is_null() {
            (*keyboard).set_repeat_info(rate, delay);
        }
    }
}

unsafe extern "C" fn input_device_set_scroll_factor(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    factor_raw: ffi::wl_fixed_t,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputDeviceObject;
    if obj.is_null() {
        return;
    }
    let device = (*obj).device;
    let factor = factor_raw as f64 / 256.0;
    if factor < 0.0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_input_device_v1_error_RIVER_INPUT_DEVICE_V1_ERROR_INVALID_SCROLL_FACTOR,
            b"negative scroll factor\0".as_ptr() as *const _,
        );
        return;
    }
    (*device).config.scroll_factor = factor;
}

unsafe extern "C" fn input_device_map_to_output(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    output_resource: *mut ffi::wl_resource,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputDeviceObject;
    if obj.is_null() {
        return;
    }
    let device = (*obj).device;

    let dev_type = ffi::river_wlr_input_device_get_type((*device).wlr_device);
    match dev_type {
        ffi::wlr_input_device_type_WLR_INPUT_DEVICE_POINTER |
        ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TOUCH |
        ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TABLET => {},
        _ => return,
    }

    let wlr_output = if !output_resource.is_null() {
        let out = ffi::wlr_output_from_resource(output_resource);
        if out.is_null() {
            return;
        }
        out
    } else {
        std::ptr::null_mut()
    };

    (*device).config.map_to_output = wlr_output;

    match dev_type {
        ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TOUCH |
        ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TABLET => {
            if !(*device).seat.is_null() {
                let cursor = &mut (*(*device).seat).cursor;
                ffi::wlr_cursor_map_input_to_output(cursor.wlr_cursor, (*device).wlr_device, wlr_output);
            }
        }
        _ => {}
    }
}

unsafe extern "C" fn input_device_map_to_rectangle(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputDeviceObject;
    if obj.is_null() {
        return;
    }
    let device = (*obj).device;

    if width < 0 || height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_input_device_v1_error_RIVER_INPUT_DEVICE_V1_ERROR_INVALID_MAP_TO_RECTANGLE,
            b"negative rectangle width/height\0".as_ptr() as *const _,
        );
        return;
    }

    let dev_type = ffi::river_wlr_input_device_get_type((*device).wlr_device);
    match dev_type {
        ffi::wlr_input_device_type_WLR_INPUT_DEVICE_POINTER |
        ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TOUCH |
        ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TABLET => {},
        _ => return,
    }

    (*device).config.map_to_rectangle = ffi::wlr_box {
        x,
        y,
        width,
        height,
    };

    match dev_type {
        ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TOUCH |
        ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TABLET => {
            if !(*device).seat.is_null() {
                let cursor = &mut (*(*device).seat).cursor;
                ffi::wlr_cursor_map_input_to_region(
                    cursor.wlr_cursor,
                    (*device).wlr_device,
                    &mut (*device).config.map_to_rectangle,
                );
            }
        }
        _ => {}
    }
}
