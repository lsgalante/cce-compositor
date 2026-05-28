// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::input_device::InputDevice;
use crate::server::WlList;

pub struct LibinputDevice {
    pub parent_device: *mut InputDevice,
    pub libinput: *mut ffi::libinput_device,
    pub objects: ffi::wl_list, // list of LibinputDeviceObject
    pub link: ffi::wl_list,    // link inside LibinputConfig::devices
}

pub struct LibinputDeviceObject {
    pub device: *mut LibinputDevice,
    pub resource: *mut ffi::wl_resource,
    pub link: ffi::wl_list,
}

impl LibinputDevice {
    pub unsafe fn init(
        parent_device: *mut InputDevice,
        handle: *mut ffi::libinput_device,
    ) -> Box<Self> {
        let mut dev = Box::new(Self {
            parent_device,
            libinput: handle,
            objects: std::mem::zeroed(),
            link: std::mem::zeroed(),
        });

        ffi::wl_list_init(&mut dev.objects);
        ffi::wl_list_init(&mut dev.link);

        let server = (*(*parent_device).seat).server;
        let devices_head = &mut (*server).libinput_config.devices as *mut ffi::wl_list as *mut WlList;
        crate::server::wl_list_insert((*devices_head).prev, &mut dev.link as *mut ffi::wl_list as *mut WlList);

        let config_objects = &mut (*server).libinput_config.objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*config_objects).next;
        while curr != config_objects {
            let next = (*curr).next;
            let config_obj = crate::container_of!(curr, crate::libinput_config::LibinputConfigObject, link);
            dev.create_object((*config_obj).resource);
            curr = next;
        }

        dev
    }

    pub unsafe fn deinit(&mut self) {
        let objects_head = &mut self.objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*objects_head).next;
        while curr != objects_head {
            let next = (*curr).next;
            let obj = crate::container_of!(curr, LibinputDeviceObject, link);

            crate::server::wl_list_remove(curr);
            ffi::wl_resource_post_event((*obj).resource, ffi::RIVER_LIBINPUT_DEVICE_V1_REMOVED);

            ffi::wl_resource_set_implementation(
                (*obj).resource,
                std::ptr::null(),
                std::ptr::null_mut(),
                None,
            );

            let _ = Box::from_raw(obj);
            curr = next;
        }

        crate::server::wl_list_remove(&mut self.link as *mut ffi::wl_list as *mut WlList);
    }

    pub unsafe fn create_object(&mut self, config_v1_resource: *mut ffi::wl_resource) {
        let client = ffi::wl_resource_get_client(config_v1_resource);
        let version = ffi::wl_resource_get_version(config_v1_resource);

        let resource = ffi::wl_resource_create(
            client,
            &ffi::river_libinput_device_v1_interface,
            version,
            0,
        );
        if resource.is_null() {
            log::error!("out of memory creating river_libinput_device_v1");
            ffi::wl_client_post_no_memory(client);
            return;
        }

        let obj = Box::into_raw(Box::new(LibinputDeviceObject {
            device: self,
            resource,
            link: std::mem::zeroed(),
        }));

        ffi::wl_resource_set_implementation(
            resource,
            &LIBINPUT_DEVICE_INTERFACE as *const _ as *const _,
            obj as *mut _,
            Some(handle_device_object_destroy),
        );

        let objects_head = &mut self.objects as *mut ffi::wl_list as *mut WlList;
        let link_ptr = &mut (*obj).link as *mut ffi::wl_list as *mut WlList;
        crate::server::wl_list_insert(objects_head, link_ptr);

        // Send libinput_device event
        ffi::wl_resource_post_event(config_v1_resource, ffi::RIVER_LIBINPUT_CONFIG_V1_LIBINPUT_DEVICE, resource);

        // Send corresponding river input device
        let input_dev_objects = &mut (*self.parent_device).objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*input_dev_objects).next;
        while curr != input_dev_objects {
            let next = (*curr).next;
            let input_dev_obj = crate::container_of!(curr, crate::input_device::InputDeviceObject, link);
            if ffi::wl_resource_get_client((*input_dev_obj).resource) == client {
                ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_INPUT_DEVICE, (*input_dev_obj).resource);
            }
            curr = next;
        }

        // SendEvents support and defaults
        let send_events_modes = ffi::libinput_device_config_send_events_get_modes(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_SEND_EVENTS_SUPPORT, send_events_modes);
        let send_events_default = ffi::libinput_device_config_send_events_get_default_mode(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_SEND_EVENTS_DEFAULT, send_events_default);
        let send_events_current = ffi::libinput_device_config_send_events_get_mode(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_SEND_EVENTS_CURRENT, send_events_current);

        // Tap support and defaults
        let tap_finger_count = ffi::libinput_device_config_tap_get_finger_count(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_TAP_SUPPORT, tap_finger_count as i32);
        if tap_finger_count > 0 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_TAP_DEFAULT, ffi::libinput_device_config_tap_get_default_enabled(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_TAP_CURRENT, ffi::libinput_device_config_tap_get_enabled(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_TAP_BUTTON_MAP_DEFAULT, ffi::libinput_device_config_tap_get_default_button_map(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_TAP_BUTTON_MAP_CURRENT, ffi::libinput_device_config_tap_get_button_map(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DRAG_DEFAULT, ffi::libinput_device_config_tap_get_default_drag_enabled(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DRAG_CURRENT, ffi::libinput_device_config_tap_get_drag_enabled(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DRAG_LOCK_DEFAULT, ffi::libinput_device_config_tap_get_default_drag_lock_enabled(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DRAG_LOCK_CURRENT, ffi::libinput_device_config_tap_get_drag_lock_enabled(self.libinput));
        }

        // Three finger drag
        let three_finger_drag_finger_count = ffi::libinput_device_config_3fg_drag_get_finger_count(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_THREE_FINGER_DRAG_SUPPORT, three_finger_drag_finger_count as i32);
        if three_finger_drag_finger_count >= 3 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_THREE_FINGER_DRAG_DEFAULT, ffi::libinput_device_config_3fg_drag_get_default_enabled(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_THREE_FINGER_DRAG_CURRENT, ffi::libinput_device_config_3fg_drag_get_enabled(self.libinput));
        }

        // Calibration matrix
        let has_matrix = ffi::libinput_device_config_calibration_has_matrix(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_CALIBRATION_MATRIX_SUPPORT, has_matrix as i32);
        if has_matrix != 0 {
            let mut matrix = [0.0f32; 6];
            ffi::libinput_device_config_calibration_get_default_matrix(self.libinput, matrix.as_mut_ptr());
            let mut arr_default = ffi::wl_array {
                size: std::mem::size_of_val(&matrix),
                alloc: std::mem::size_of_val(&matrix),
                data: matrix.as_mut_ptr() as *mut _,
            };
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_CALIBRATION_MATRIX_DEFAULT, &mut arr_default as *mut _);

            ffi::libinput_device_config_calibration_get_matrix(self.libinput, matrix.as_mut_ptr());
            let mut arr_current = ffi::wl_array {
                size: std::mem::size_of_val(&matrix),
                alloc: std::mem::size_of_val(&matrix),
                data: matrix.as_mut_ptr() as *mut _,
            };
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_CALIBRATION_MATRIX_CURRENT, &mut arr_current as *mut _);
        }

        // Acceleration configs
        let profiles = ffi::libinput_device_config_accel_get_profiles(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_ACCEL_PROFILES_SUPPORT, profiles);
        if profiles != 0 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_ACCEL_PROFILE_DEFAULT, ffi::libinput_device_config_accel_get_default_profile(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_ACCEL_PROFILE_CURRENT, ffi::libinput_device_config_accel_get_profile(self.libinput));

            let default_speed = ffi::libinput_device_config_accel_get_default_speed(self.libinput);
            let mut arr_default = ffi::wl_array {
                size: std::mem::size_of::<f64>(),
                alloc: std::mem::size_of::<f64>(),
                data: &default_speed as *const f64 as *mut _,
            };
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_ACCEL_SPEED_DEFAULT, &mut arr_default as *mut _);

            let current_speed = ffi::libinput_device_config_accel_get_speed(self.libinput);
            let mut arr_current = ffi::wl_array {
                size: std::mem::size_of::<f64>(),
                alloc: std::mem::size_of::<f64>(),
                data: &current_speed as *const f64 as *mut _,
            };
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_ACCEL_SPEED_CURRENT, &mut arr_current as *mut _);
        }

        // Natural scroll
        let natural_scroll = ffi::libinput_device_config_scroll_has_natural_scroll(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_NATURAL_SCROLL_SUPPORT, natural_scroll as i32);
        if natural_scroll != 0 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_NATURAL_SCROLL_DEFAULT, ffi::libinput_device_config_scroll_get_default_natural_scroll_enabled(self.libinput) as u32);
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_NATURAL_SCROLL_CURRENT, ffi::libinput_device_config_scroll_get_natural_scroll_enabled(self.libinput) as u32);
        }

        // Left handed mode
        let left_handed = ffi::libinput_device_config_left_handed_is_available(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_LEFT_HANDED_SUPPORT, left_handed as i32);
        if left_handed != 0 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_LEFT_HANDED_DEFAULT, ffi::libinput_device_config_left_handed_get_default(self.libinput) as u32);
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_LEFT_HANDED_CURRENT, ffi::libinput_device_config_left_handed_get(self.libinput) as u32);
        }

        // Click methods
        let click_methods = ffi::libinput_device_config_click_get_methods(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_CLICK_METHOD_SUPPORT, click_methods);
        if click_methods != 0 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_CLICK_METHOD_DEFAULT, ffi::libinput_device_config_click_get_default_method(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_CLICK_METHOD_CURRENT, ffi::libinput_device_config_click_get_method(self.libinput));
            if (click_methods & ffi::libinput_config_click_method_LIBINPUT_CONFIG_CLICK_METHOD_CLICKFINGER) != 0 {
                ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_CLICKFINGER_BUTTON_MAP_DEFAULT, ffi::libinput_device_config_click_get_default_clickfinger_button_map(self.libinput));
                ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_CLICKFINGER_BUTTON_MAP_CURRENT, ffi::libinput_device_config_click_get_clickfinger_button_map(self.libinput));
            }
        }

        // Middle mouse emulation
        let middle_emulation = ffi::libinput_device_config_middle_emulation_is_available(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_MIDDLE_EMULATION_SUPPORT, middle_emulation as i32);
        if middle_emulation != 0 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_MIDDLE_EMULATION_DEFAULT, ffi::libinput_device_config_middle_emulation_get_default_enabled(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_MIDDLE_EMULATION_CURRENT, ffi::libinput_device_config_middle_emulation_get_enabled(self.libinput));
        }

        // Scroll methods
        let scroll_methods = ffi::libinput_device_config_scroll_get_methods(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_SCROLL_METHOD_SUPPORT, scroll_methods);
        if scroll_methods != 0 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_SCROLL_METHOD_DEFAULT, ffi::libinput_device_config_scroll_get_default_method(self.libinput));
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_SCROLL_METHOD_CURRENT, ffi::libinput_device_config_scroll_get_method(self.libinput));
            if (scroll_methods & ffi::libinput_config_scroll_method_LIBINPUT_CONFIG_SCROLL_ON_BUTTON_DOWN) != 0 {
                ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_SCROLL_BUTTON_DEFAULT, ffi::libinput_device_config_scroll_get_default_button(self.libinput));
                ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_SCROLL_BUTTON_CURRENT, ffi::libinput_device_config_scroll_get_button(self.libinput));
                ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_SCROLL_BUTTON_LOCK_DEFAULT, ffi::libinput_device_config_scroll_get_default_button_lock(self.libinput));
                ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_SCROLL_BUTTON_LOCK_CURRENT, ffi::libinput_device_config_scroll_get_button_lock(self.libinput));
            }
        }

        // Disable While Typing (DWT)
        let dwt = ffi::libinput_device_config_dwt_is_available(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DWT_SUPPORT, dwt as i32);
        if dwt != 0 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DWT_DEFAULT, ffi::libinput_device_config_dwt_get_default_enabled(self.libinput) as u32);
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DWT_CURRENT, ffi::libinput_device_config_dwt_get_enabled(self.libinput) as u32);
        }

        // Disable While Trackpointing (DWTP)
        let dwtp = ffi::libinput_device_config_dwtp_is_available(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DWTP_SUPPORT, dwtp as i32);
        if dwtp != 0 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DWTP_DEFAULT, ffi::libinput_device_config_dwtp_get_default_enabled(self.libinput) as u32);
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DWTP_CURRENT, ffi::libinput_device_config_dwtp_get_enabled(self.libinput) as u32);
        }

        // Rotation
        let rotation = ffi::libinput_device_config_rotation_is_available(self.libinput);
        ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_ROTATION_SUPPORT, rotation as i32);
        if rotation != 0 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_ROTATION_DEFAULT, ffi::libinput_device_config_rotation_get_default_angle(self.libinput) as u32);
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_ROTATION_CURRENT, ffi::libinput_device_config_rotation_get_angle(self.libinput) as u32);
        }

        if version >= 2 {
            ffi::wl_resource_post_event(resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DONE);
        }
    }
}

unsafe extern "C" fn handle_device_object_destroy(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if !obj.is_null() {
        crate::server::wl_list_remove(&mut (*obj).link as *mut ffi::wl_list as *mut WlList);
        let _ = Box::from_raw(obj);
    }
}

static LIBINPUT_DEVICE_INTERFACE: ffi::river_libinput_device_v1_interface = ffi::river_libinput_device_v1_interface {
    destroy: Some(device_destroy),
    set_send_events: Some(device_set_send_events),
    set_tap: Some(device_set_tap),
    set_tap_button_map: Some(device_set_tap_button_map),
    set_drag: Some(device_set_drag),
    set_drag_lock: Some(device_set_drag_lock),
    set_three_finger_drag: Some(device_set_three_finger_drag),
    set_calibration_matrix: Some(device_set_calibration_matrix),
    set_accel_profile: Some(device_set_accel_profile),
    set_accel_speed: Some(device_set_accel_speed),
    apply_accel_config: Some(device_apply_accel_config),
    set_natural_scroll: Some(device_set_natural_scroll),
    set_left_handed: Some(device_set_left_handed),
    set_click_method: Some(device_set_click_method),
    set_clickfinger_button_map: Some(device_set_clickfinger_button_map),
    set_middle_emulation: Some(device_set_middle_emulation),
    set_scroll_method: Some(device_set_scroll_method),
    set_scroll_button: Some(device_set_scroll_button),
    set_scroll_button_lock: Some(device_set_scroll_button_lock),
    set_dwt: Some(device_set_dwt),
    set_dwtp: Some(device_set_dwtp),
    set_rotation: Some(device_set_rotation),
};

unsafe fn make_result(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    status: ffi::libinput_config_status,
) -> bool {
    let result_res = ffi::wl_resource_create(
        client,
        &ffi::river_libinput_result_v1_interface,
        ffi::wl_resource_get_version(resource),
        result_id,
    );
    if result_res.is_null() {
        ffi::wl_client_post_no_memory(client);
        return false;
    }

    match status {
        ffi::libinput_config_status_LIBINPUT_CONFIG_STATUS_SUCCESS => {
            ffi::wl_resource_post_event(result_res, 0); // success
        }
        ffi::libinput_config_status_LIBINPUT_CONFIG_STATUS_UNSUPPORTED => {
            ffi::wl_resource_post_event(result_res, 1); // unsupported
        }
        _ => {
            ffi::wl_resource_post_event(result_res, 2); // invalid
        }
    }
    ffi::wl_resource_destroy(result_res);
    status == ffi::libinput_config_status_LIBINPUT_CONFIG_STATUS_SUCCESS
}

unsafe fn broadcast_update(dev: *mut LibinputDevice, opcode: u32, arg: u32) {
    let objects_head = &mut (*dev).objects as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*objects_head).next;
    while curr != objects_head {
        let next = (*curr).next;
        let obj = crate::container_of!(curr, LibinputDeviceObject, link);
        ffi::wl_resource_post_event((*obj).resource, opcode, arg);
        if ffi::wl_resource_get_version((*obj).resource) >= 2 {
            ffi::wl_resource_post_event((*obj).resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DONE);
        }
        curr = next;
    }
}

unsafe extern "C" fn device_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn device_set_send_events(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    mode: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_send_events_set_mode((*dev).libinput, mode);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_send_events_get_mode((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_SEND_EVENTS_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_tap(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    state: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_tap_set_enabled((*dev).libinput, state);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_tap_get_enabled((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_TAP_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_tap_button_map(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    button_map: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_tap_set_button_map((*dev).libinput, button_map);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_tap_get_button_map((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_TAP_BUTTON_MAP_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_drag(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    state: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_tap_set_drag_enabled((*dev).libinput, state);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_tap_get_drag_enabled((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_DRAG_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_drag_lock(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    state: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_tap_set_drag_lock_enabled((*dev).libinput, state);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_tap_get_drag_lock_enabled((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_DRAG_LOCK_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_three_finger_drag(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    state: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_3fg_drag_set_enabled((*dev).libinput, state);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_3fg_drag_get_enabled((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_THREE_FINGER_DRAG_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_calibration_matrix(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    matrix_arr: *mut ffi::wl_array,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    if (*matrix_arr).size != std::mem::size_of::<[f32; 6]>() {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_libinput_device_v1_error_RIVER_LIBINPUT_DEVICE_V1_ERROR_INVALID_ARG,
            b"invalid calibration matrix\0".as_ptr() as *const _,
        );
        return;
    }

    let matrix_ptr = (*matrix_arr).data as *const f32;
    let status = ffi::libinput_device_config_calibration_set_matrix((*dev).libinput, matrix_ptr);
    if make_result(client, resource, result_id, status) {
        let mut current = [0.0f32; 6];
        ffi::libinput_device_config_calibration_get_matrix((*dev).libinput, current.as_mut_ptr());
        let mut arr = ffi::wl_array {
            size: std::mem::size_of_val(&current),
            alloc: std::mem::size_of_val(&current),
            data: current.as_mut_ptr() as *mut _,
        };

        let objects_head = &mut (*dev).objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*objects_head).next;
        while curr != objects_head {
            let next = (*curr).next;
            let dest_obj = crate::container_of!(curr, LibinputDeviceObject, link);
            ffi::wl_resource_post_event((*dest_obj).resource, ffi::RIVER_LIBINPUT_DEVICE_V1_CALIBRATION_MATRIX_CURRENT, &mut arr as *mut _);
            if ffi::wl_resource_get_version((*dest_obj).resource) >= 2 {
                ffi::wl_resource_post_event((*dest_obj).resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DONE);
            }
            curr = next;
        }
    }
}

unsafe extern "C" fn device_set_accel_profile(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    profile: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_accel_set_profile((*dev).libinput, profile);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_accel_get_profile((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_ACCEL_PROFILE_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_accel_speed(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    speed_arr: *mut ffi::wl_array,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    if (*speed_arr).size != std::mem::size_of::<f64>() {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_libinput_device_v1_error_RIVER_LIBINPUT_DEVICE_V1_ERROR_INVALID_ARG,
            b"invalid accel speed\0".as_ptr() as *const _,
        );
        return;
    }

    let speed = *((*speed_arr).data as *const f64);
    let status = ffi::libinput_device_config_accel_set_speed((*dev).libinput, speed);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_accel_get_speed((*dev).libinput);
        let mut arr = ffi::wl_array {
            size: std::mem::size_of::<f64>(),
            alloc: std::mem::size_of::<f64>(),
            data: &current as *const f64 as *mut _,
        };

        let objects_head = &mut (*dev).objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*objects_head).next;
        while curr != objects_head {
            let next = (*curr).next;
            let dest_obj = crate::container_of!(curr, LibinputDeviceObject, link);
            ffi::wl_resource_post_event((*dest_obj).resource, ffi::RIVER_LIBINPUT_DEVICE_V1_ACCEL_SPEED_CURRENT, &mut arr as *mut _);
            if ffi::wl_resource_get_version((*dest_obj).resource) >= 2 {
                ffi::wl_resource_post_event((*dest_obj).resource, ffi::RIVER_LIBINPUT_DEVICE_V1_DONE);
            }
            curr = next;
        }
    }
}

unsafe extern "C" fn device_apply_accel_config(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    config_res: *mut ffi::wl_resource,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let accel_config = ffi::wl_resource_get_user_data(config_res) as *mut crate::libinput_accel_config::LibinputAccelConfig;
    let config = if !accel_config.is_null() {
        (*accel_config).libinput
    } else {
        std::ptr::null_mut()
    };

    let result_res = ffi::wl_resource_create(
        client,
        &ffi::river_libinput_result_v1_interface,
        ffi::wl_resource_get_version(resource),
        result_id,
    );
    if result_res.is_null() {
        ffi::wl_client_post_no_memory(client);
        return;
    }

    if config.is_null() {
        ffi::wl_resource_post_event(result_res, 2); // invalid
        ffi::wl_resource_destroy(result_res);
        return;
    }

    let status = ffi::libinput_device_config_accel_apply((*dev).libinput, config);

    let success = match status {
        ffi::libinput_config_status_LIBINPUT_CONFIG_STATUS_SUCCESS => {
            ffi::wl_resource_post_event(result_res, 0); // success
            true
        }
        ffi::libinput_config_status_LIBINPUT_CONFIG_STATUS_UNSUPPORTED => {
            ffi::wl_resource_post_event(result_res, 1); // unsupported
            false
        }
        _ => {
            ffi::wl_resource_post_event(result_res, 2); // invalid
            false
        }
    };

    ffi::wl_resource_destroy(result_res);

    if success {
        let current = ffi::libinput_device_config_accel_get_profile((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_ACCEL_PROFILE_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_natural_scroll(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    state: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_scroll_set_natural_scroll_enabled((*dev).libinput, state as i32);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_scroll_get_natural_scroll_enabled((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_NATURAL_SCROLL_CURRENT, current as u32);
    }
}

unsafe extern "C" fn device_set_left_handed(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    state: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_left_handed_set((*dev).libinput, state as i32);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_left_handed_get((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_LEFT_HANDED_CURRENT, current as u32);
    }
}

unsafe extern "C" fn device_set_click_method(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    method: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_click_set_method((*dev).libinput, method);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_click_get_method((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_CLICK_METHOD_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_clickfinger_button_map(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    button_map: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_click_set_clickfinger_button_map((*dev).libinput, button_map);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_click_get_clickfinger_button_map((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_CLICKFINGER_BUTTON_MAP_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_middle_emulation(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    state: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_middle_emulation_set_enabled((*dev).libinput, state);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_middle_emulation_get_enabled((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_MIDDLE_EMULATION_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_scroll_method(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    method: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_scroll_set_method((*dev).libinput, method);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_scroll_get_method((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_SCROLL_METHOD_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_scroll_button(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    button: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_scroll_set_button((*dev).libinput, button);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_scroll_get_button((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_SCROLL_BUTTON_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_scroll_button_lock(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    state: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_scroll_set_button_lock((*dev).libinput, state);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_scroll_get_button_lock((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_SCROLL_BUTTON_LOCK_CURRENT, current);
    }
}

unsafe extern "C" fn device_set_dwt(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    state: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_dwt_set_enabled((*dev).libinput, state);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_dwt_get_enabled((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_DWT_CURRENT, current as u32);
    }
}

unsafe extern "C" fn device_set_dwtp(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    state: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_dwtp_set_enabled((*dev).libinput, state);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_dwtp_get_enabled((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_DWTP_CURRENT, current as u32);
    }
}

unsafe extern "C" fn device_set_rotation(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    angle: u32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputDeviceObject;
    if obj.is_null() { return; }
    let dev = (*obj).device;

    let status = ffi::libinput_device_config_rotation_set_angle((*dev).libinput, angle);
    if make_result(client, resource, result_id, status) {
        let current = ffi::libinput_device_config_rotation_get_angle((*dev).libinput);
        broadcast_update(dev, ffi::RIVER_LIBINPUT_DEVICE_V1_ROTATION_CURRENT, current);
    }
}
