// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlList, wl_list_insert, wl_list_remove};

pub struct LibinputConfig {
    pub server: *mut Server,
    pub global: *mut ffi::wl_global,
    pub objects: ffi::wl_list,
    pub devices: ffi::wl_list,
}

pub struct LibinputConfigObject {
    pub config: *mut LibinputConfig,
    pub resource: *mut ffi::wl_resource,
    pub link: ffi::wl_list,
}

impl Default for LibinputConfig {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl LibinputConfig {
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        ffi::wl_list_init(&mut self.objects);
        ffi::wl_list_init(&mut self.devices);

        self.global = ffi::wl_global_create(
            (*server).wl_server,
            &ffi::river_libinput_config_v1_interface,
            2,
            self as *mut LibinputConfig as *mut _,
            Some(bind_libinput_config),
        );
        if self.global.is_null() {
            return Err("Failed to create river_libinput_config_v1 global");
        }

        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        if !self.global.is_null() {
            ffi::wl_global_destroy(self.global);
            self.global = std::ptr::null_mut();
        }

        let objects_head = &mut self.objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*objects_head).next;
        while curr != objects_head {
            let next = (*curr).next;
            let obj = crate::container_of!(curr, LibinputConfigObject, link);
            ffi::wl_resource_destroy((*obj).resource);
            curr = next;
        }
    }
}

unsafe extern "C" fn bind_libinput_config(
    client: *mut ffi::wl_client,
    data: *mut std::ffi::c_void,
    version: u32,
    id: u32,
) {
    let config = data as *mut LibinputConfig;
    let resource = ffi::wl_resource_create(client, &ffi::river_libinput_config_v1_interface, version as i32, id);
    if resource.is_null() {
        ffi::wl_client_post_no_memory(client);
        return;
    }

    let obj = Box::into_raw(Box::new(LibinputConfigObject {
        config,
        resource,
        link: std::mem::zeroed(),
    }));

    ffi::wl_resource_set_implementation(
        resource,
        &LIBINPUT_CONFIG_INTERFACE as *const _ as *const _,
        obj as *mut _,
        Some(handle_object_destroy),
    );

    let list_head = &mut (*config).objects as *mut ffi::wl_list as *mut WlList;
    wl_list_insert((*list_head).prev, &mut (*obj).link as *mut ffi::wl_list as *mut WlList);

    let devices_head = &mut (*config).devices as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*devices_head).next;
    while curr != devices_head {
        let next = (*curr).next;
        let dev = crate::container_of!(curr, crate::libinput_device::LibinputDevice, link);
        (*dev).create_object(resource);
        curr = next;
    }
}

unsafe extern "C" fn handle_object_destroy(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputConfigObject;
    if !obj.is_null() {
        wl_list_remove(&mut (*obj).link as *mut ffi::wl_list as *mut WlList);
        let _ = Box::from_raw(obj);
    }
}

static LIBINPUT_CONFIG_INTERFACE: ffi::river_libinput_config_v1_interface = ffi::river_libinput_config_v1_interface {
    stop: Some(libinput_config_stop),
    destroy: Some(libinput_config_destroy),
    create_accel_config: Some(libinput_config_create_accel_config),
};

static LIBINPUT_CONFIG_INERT_INTERFACE: ffi::river_libinput_config_v1_interface = ffi::river_libinput_config_v1_interface {
    stop: None,
    destroy: Some(libinput_config_inert_destroy),
    create_accel_config: None,
};

unsafe extern "C" fn libinput_config_stop(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputConfigObject;
    if obj.is_null() {
        return;
    }

    wl_list_remove(&mut (*obj).link as *mut ffi::wl_list as *mut WlList);
    ffi::wl_list_init(&mut (*obj).link);

    ffi::wl_resource_post_event(resource, 0); // finished event

    ffi::wl_resource_set_implementation(
        resource,
        &LIBINPUT_CONFIG_INERT_INTERFACE as *const _ as *const _,
        obj as *mut _,
        Some(handle_object_destroy),
    );
}

unsafe extern "C" fn libinput_config_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_post_error(
        resource,
        ffi::river_libinput_config_v1_error_RIVER_LIBINPUT_CONFIG_V1_ERROR_INVALID_DESTROY,
        b"destroy before finished event sent\0".as_ptr() as *const _,
    );
}

unsafe extern "C" fn libinput_config_inert_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn libinput_config_create_accel_config(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    profile: u32,
) {
    if let Err(e) = crate::libinput_accel_config::LibinputAccelConfig::create(
        client,
        ffi::wl_resource_get_version(resource) as u32,
        id,
        profile,
    ) {
        log::error!("Failed to create LibinputAccelConfig: {}", e);
        ffi::wl_resource_post_no_memory(resource);
    }
}
