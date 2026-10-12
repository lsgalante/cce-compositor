// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::Server;

pub struct LibinputConfig {
    pub global: *mut ffi::wl_global,
    /// Private registries: walks trust that what is registered is live
    /// (each unregisters before it is freed).
    objects: crate::registry::Registry<LibinputConfigObject>,
    devices: crate::registry::Registry<crate::libinput_device::LibinputDevice>,
}

impl LibinputConfig {
    /// Write empty registries into a zero-filled config (`Server::zeroed`),
    /// whose registry bytes are not yet valid `Vec`s.
    ///
    /// # Safety
    /// `this` points at a `LibinputConfig` whose registries have not been
    /// initialised; they are overwritten without being dropped.
    pub unsafe fn init_registries_in_place(this: *mut Self) {
        std::ptr::write(std::ptr::addr_of_mut!((*this).objects), crate::registry::Registry::new());
        std::ptr::write(std::ptr::addr_of_mut!((*this).devices), crate::registry::Registry::new());
    }

    /// Its protocol objects (one per bound client), copied out.
    pub(crate) fn objects(&self) -> Vec<*mut LibinputConfigObject> {
        self.objects.ptrs()
    }

    /// The libinput devices, copied out.
    pub(crate) fn devices(&self) -> Vec<*mut crate::libinput_device::LibinputDevice> {
        self.devices.ptrs()
    }

    /// # Safety
    /// As `Registry::push`: live until `unregister_device`, which its
    /// `deinit` runs before it is freed.
    pub(crate) unsafe fn register_device(&mut self, device: *mut crate::libinput_device::LibinputDevice) {
        self.devices.push(device);
    }

    pub(crate) fn unregister_device(&mut self, device: *mut crate::libinput_device::LibinputDevice) {
        self.devices.remove(device);
    }
}

pub struct LibinputConfigObject {
    pub config: *mut LibinputConfig,
    pub resource: *mut ffi::wl_resource,
}

impl LibinputConfig {
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
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

        for obj in self.objects.ptrs() {
            // Its destroy handler unregisters and frees it.
            ffi::wl_resource_destroy((*obj).resource);
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
    }));

    ffi::wl_resource_set_implementation(
        resource,
        &LIBINPUT_CONFIG_INTERFACE as *const _ as *const _,
        obj as *mut _,
        Some(handle_object_destroy),
    );

    // Unregistered by its destroy handler before it is freed.
    (*config).objects.push(obj);

    for dev in (*config).devices() {
        (*dev).create_object(resource);
    }
}

unsafe extern "C" fn handle_object_destroy(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputConfigObject;
    if !obj.is_null() {
        // The config is a server field, live for the server's life.
        (*(*obj).config).objects.remove(obj);
        let _ = Box::from_raw(obj);
    }
}

static LIBINPUT_CONFIG_INTERFACE: ffi::river_libinput_config_v1_interface = ffi::river_libinput_config_v1_interface {
    stop: Some(libinput_config_stop),
    destroy: Some(libinput_config_destroy),
    create_accel_config: Some(libinput_config_create_accel_config),
};

// Every request has a handler: libwayland aborts the whole compositor on a
// null entry, and a client can send any request on an inert object (one it
// sent before it saw the event that made the object inert, or a repeat).
static LIBINPUT_CONFIG_INERT_INTERFACE: ffi::river_libinput_config_v1_interface = ffi::river_libinput_config_v1_interface {
    stop: Some(libinput_config_inert_stop),
    destroy: Some(libinput_config_inert_destroy),
    create_accel_config: Some(libinput_config_inert_create_accel_config),
};

unsafe extern "C" fn libinput_config_inert_stop(_client: *mut ffi::wl_client, _resource: *mut ffi::wl_resource) {}

unsafe extern "C" fn libinput_config_inert_create_accel_config(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _id: u32,
    _profile: u32,
) {
}

unsafe extern "C" fn libinput_config_stop(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LibinputConfigObject;
    if obj.is_null() {
        return;
    }

    (*(*obj).config).objects.remove(obj);

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
