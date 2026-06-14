// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;

pub struct LibinputAccelConfig {
    pub resource: *mut ffi::wl_resource,
    pub libinput: *mut ffi::libinput_config_accel,
}

impl LibinputAccelConfig {
    pub unsafe fn create(
        client: *mut ffi::wl_client,
        version: u32,
        id: u32,
        profile: u32,
    ) -> Result<*mut Self, &'static str> {
        let libinput = ffi::libinput_config_accel_create(profile);
        if libinput.is_null() {
            return Err("Failed to create libinput_config_accel");
        }

        let resource = ffi::wl_resource_create(
            client,
            &ffi::river_libinput_accel_config_v1_interface,
            version as i32,
            id,
        );
        if resource.is_null() {
            ffi::libinput_config_accel_destroy(libinput);
            return Err("Failed to create river_libinput_accel_config_v1 resource");
        }

        let accel_config = Box::into_raw(Box::new(Self {
            resource,
            libinput,
        }));

        ffi::wl_resource_set_implementation(
            resource,
            &ACCEL_CONFIG_INTERFACE as *const _ as *const _,
            accel_config as *mut _,
            Some(handle_accel_config_destroy_resource),
        );

        Ok(accel_config)
    }
}

unsafe extern "C" fn handle_accel_config_destroy_resource(resource: *mut ffi::wl_resource) {
    let accel_config = ffi::wl_resource_get_user_data(resource) as *mut LibinputAccelConfig;
    if !accel_config.is_null() {
        if !(*accel_config).libinput.is_null() {
            ffi::libinput_config_accel_destroy((*accel_config).libinput);
        }
        let _ = Box::from_raw(accel_config);
    }
}

static ACCEL_CONFIG_INTERFACE: ffi::river_libinput_accel_config_v1_interface = ffi::river_libinput_accel_config_v1_interface {
    destroy: Some(accel_config_destroy),
    set_points: Some(accel_config_set_points),
};

unsafe extern "C" fn accel_config_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn accel_config_set_points(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    result_id: u32,
    accel_type_raw: u32,
    step_arr: *mut ffi::wl_array,
    points_arr: *mut ffi::wl_array,
) {
    let accel_config = ffi::wl_resource_get_user_data(resource) as *mut LibinputAccelConfig;
    if accel_config.is_null() {
        return;
    }

    let accel_type = match accel_type_raw {
        0 => ffi::libinput_config_accel_type_LIBINPUT_ACCEL_TYPE_FALLBACK,
        1 => ffi::libinput_config_accel_type_LIBINPUT_ACCEL_TYPE_MOTION,
        2 => ffi::libinput_config_accel_type_LIBINPUT_ACCEL_TYPE_SCROLL,
        _ => {
            ffi::wl_resource_post_error(
                resource,
                ffi::river_libinput_accel_config_v1_error_RIVER_LIBINPUT_ACCEL_CONFIG_V1_ERROR_INVALID_ARG,
                b"invalid accel_type enum value\0".as_ptr() as *const _,
            );
            return;
        }
    };

    if (*step_arr).size != std::mem::size_of::<f64>() as usize {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_libinput_accel_config_v1_error_RIVER_LIBINPUT_ACCEL_CONFIG_V1_ERROR_INVALID_ARG,
            b"invalid step argument\0".as_ptr() as *const _,
        );
        return;
    }

    let step = *((*step_arr).data as *const f64);

    if (*points_arr).size == 0 || (*points_arr).size % std::mem::size_of::<f64>() as usize != 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_libinput_accel_config_v1_error_RIVER_LIBINPUT_ACCEL_CONFIG_V1_ERROR_INVALID_ARG,
            b"invalid points argument\0".as_ptr() as *const _,
        );
        return;
    }

    let points_count = (*points_arr).size / std::mem::size_of::<f64>();
    let points_ptr = (*points_arr).data as *const f64;

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

    let libinput = (*accel_config).libinput;
    if libinput.is_null() {
        ffi::wl_resource_post_event(result_res, 2); // invalid
        ffi::wl_resource_destroy(result_res);
        return;
    }

    let status = ffi::libinput_config_accel_set_points(
        libinput,
        accel_type,
        step,
        points_count,
        points_ptr,
    );

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
}
