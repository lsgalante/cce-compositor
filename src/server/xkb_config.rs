// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::Server;

pub struct XkbConfig {
    pub global: *mut ffi::wl_global,
    pub context: *mut ffi::xkb_context,
    pub default_keymap: *mut ffi::xkb_keymap,
    /// Private registries: walks trust that what is registered is live
    /// (each unregisters before it is freed).
    objects: crate::registry::Registry<XkbConfigObject>,
    keyboards: crate::registry::Registry<crate::xkb_keyboard::XkbKeyboard>,
}

impl XkbConfig {
    /// Write empty registries into a zero-filled config (`Server::zeroed`),
    /// whose registry bytes are not yet valid `Vec`s.
    ///
    /// # Safety
    /// `this` points at an `XkbConfig` whose registries have not been
    /// initialised; they are overwritten without being dropped.
    pub unsafe fn init_registries_in_place(this: *mut Self) {
        std::ptr::write(std::ptr::addr_of_mut!((*this).objects), crate::registry::Registry::new());
        std::ptr::write(std::ptr::addr_of_mut!((*this).keyboards), crate::registry::Registry::new());
    }

    /// Its protocol objects (one per bound client), copied out.
    pub(crate) fn objects(&self) -> Vec<*mut XkbConfigObject> {
        self.objects.ptrs()
    }

    /// The xkb keyboards, copied out.
    pub(crate) fn keyboards(&self) -> Vec<*mut crate::xkb_keyboard::XkbKeyboard> {
        self.keyboards.ptrs()
    }

    /// # Safety
    /// As `Registry::push`: live until `unregister_keyboard`, which its
    /// `deinit` runs before it is freed.
    pub(crate) unsafe fn register_keyboard(&mut self, keyboard: *mut crate::xkb_keyboard::XkbKeyboard) {
        self.keyboards.push(keyboard);
    }

    pub(crate) fn unregister_keyboard(&mut self, keyboard: *mut crate::xkb_keyboard::XkbKeyboard) {
        self.keyboards.remove(keyboard);
    }
}

pub struct XkbConfigObject {
    pub config: *mut XkbConfig,
    pub resource: *mut ffi::wl_resource,
}

impl XkbConfig {
    /// # Safety
    /// `server` must point to the `Server`, its display up, and `self` must
    /// not move or be dropped until [`deinit`](Self::deinit): the global hands
    /// its address to every bind.
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        let context = ffi::xkb_context_new(ffi::xkb_context_flags_XKB_CONTEXT_NO_FLAGS);
        if context.is_null() {
            return Err("Failed to create xkb_context");
        }
        self.context = context;

        let default_keymap = ffi::xkb_keymap_new_from_names(
            context,
            std::ptr::null(),
            ffi::xkb_keymap_compile_flags_XKB_KEYMAP_COMPILE_NO_FLAGS,
        );
        if default_keymap.is_null() {
            ffi::xkb_context_unref(context);
            self.context = std::ptr::null_mut();
            return Err("Failed to create default xkb_keymap");
        }
        self.default_keymap = default_keymap;
        crate::shared::handles().default_keymap.set(default_keymap);

        self.global = ffi::wl_global_create(
            (*server).wl_server,
            &ffi::river_xkb_config_v1_interface,
            2,
            self as *mut XkbConfig as *mut _,
            Some(bind_xkb_config),
        );
        if self.global.is_null() {
            ffi::xkb_keymap_unref(self.default_keymap);
            ffi::xkb_context_unref(self.context);
            self.default_keymap = std::ptr::null_mut();
            crate::shared::handles().default_keymap.set(std::ptr::null_mut());
            self.context = std::ptr::null_mut();
            return Err("Failed to create river_xkb_config_v1 global");
        }

        Ok(())
    }

    /// # Safety
    /// Before the display is destroyed, with `global`, `context` and
    /// `default_keymap` null or as [`init`](Self::init) left them.
    pub unsafe fn deinit(&mut self) {
        if !self.global.is_null() {
            ffi::wl_global_destroy(self.global);
            self.global = std::ptr::null_mut();
        }

        for obj in self.objects.ptrs() {
            // Its destroy handler unregisters and frees it.
            ffi::wl_resource_destroy((*obj).resource);
        }

        if !self.default_keymap.is_null() {
            crate::shared::handles().default_keymap.set(std::ptr::null_mut());
            ffi::xkb_keymap_unref(self.default_keymap);
            self.default_keymap = std::ptr::null_mut();
        }
        if !self.context.is_null() {
            ffi::xkb_context_unref(self.context);
            self.context = std::ptr::null_mut();
        }
    }
}

unsafe extern "C" fn bind_xkb_config(
    client: *mut ffi::wl_client,
    data: *mut std::ffi::c_void,
    version: u32,
    id: u32,
) {
    let config = data as *mut XkbConfig;
    let resource = ffi::wl_resource_create(client, &ffi::river_xkb_config_v1_interface, version as i32, id);
    if resource.is_null() {
        ffi::wl_client_post_no_memory(client);
        return;
    }

    let obj = Box::into_raw(Box::new(XkbConfigObject {
        config,
        resource,
    }));

    ffi::wl_resource_set_implementation(
        resource,
        &XKB_CONFIG_INTERFACE as *const _ as *const _,
        obj as *mut _,
        Some(handle_object_destroy),
    );

    // Unregistered by its destroy handler before it is freed.
    (*config).objects.push(obj);

    for xkb_kbd in (*config).keyboards() {
        (*xkb_kbd).create_object(resource);
    }
}

unsafe extern "C" fn handle_object_destroy(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbConfigObject;
    if !obj.is_null() {
        // The config is a server field, live for the server's life.
        (*(*obj).config).objects.remove(obj);
        let _ = Box::from_raw(obj);
    }
}

static XKB_CONFIG_INTERFACE: ffi::river_xkb_config_v1_interface = ffi::river_xkb_config_v1_interface {
    stop: Some(xkb_config_stop),
    destroy: Some(xkb_config_destroy),
    create_keymap: Some(xkb_config_create_keymap),
};

// Every request has a handler: libwayland aborts the whole compositor on a
// null entry, and a client can send any request on an inert object (one it
// sent before it saw the event that made the object inert, or a repeat).
static XKB_CONFIG_INERT_INTERFACE: ffi::river_xkb_config_v1_interface = ffi::river_xkb_config_v1_interface {
    stop: Some(xkb_config_inert_stop),
    destroy: Some(xkb_config_inert_destroy),
    create_keymap: Some(xkb_config_inert_create_keymap),
};

unsafe extern "C" fn xkb_config_inert_stop(_client: *mut ffi::wl_client, _resource: *mut ffi::wl_resource) {}

unsafe extern "C" fn xkb_config_inert_create_keymap(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _id: u32,
    fd: i32,
    _format: u32,
) {
    // The fd is the handler's to close.
    libc::close(fd);
}

unsafe extern "C" fn xkb_config_stop(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbConfigObject;
    if obj.is_null() {
        return;
    }

    (*(*obj).config).objects.remove(obj);

    ffi::wl_resource_post_event(resource, 0); // finished event

    ffi::wl_resource_set_implementation(
        resource,
        &XKB_CONFIG_INERT_INTERFACE as *const _ as *const _,
        obj as *mut _,
        Some(handle_object_destroy),
    );
}

unsafe extern "C" fn xkb_config_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_post_error(
        resource,
        ffi::river_xkb_config_v1_error_RIVER_XKB_CONFIG_V1_ERROR_INVALID_DESTROY,
        c"destroy before finished event sent".as_ptr(),
    );
}

unsafe extern "C" fn xkb_config_inert_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

struct FdGuard(std::os::raw::c_int);
impl Drop for FdGuard {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.0);
        }
    }
}

pub struct XkbKeymap {
    pub xkb_keymap: *mut ffi::xkb_keymap,
}

static XKB_KEYMAP_INTERFACE: ffi::river_xkb_keymap_v1_interface = ffi::river_xkb_keymap_v1_interface {
    destroy: Some(xkb_keymap_destroy),
};

unsafe extern "C" fn xkb_keymap_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn xkb_keymap_destroy_func(resource: *mut ffi::wl_resource) {
    let keymap = ffi::wl_resource_get_user_data(resource) as *mut XkbKeymap;
    if !keymap.is_null() {
        ffi::xkb_keymap_unref((*keymap).xkb_keymap);
        let _ = Box::from_raw(keymap);
    }
}

unsafe extern "C" fn xkb_config_create_keymap(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    fd: i32,
    format: u32,
) {
    let _fd_guard = FdGuard(fd);
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbConfigObject;
    if obj.is_null() {
        return;
    }
    let config = (*obj).config;

    let keymap_res = ffi::wl_resource_create(
        client,
        &ffi::river_xkb_keymap_v1_interface,
        ffi::wl_resource_get_version(resource),
        id,
    );
    if keymap_res.is_null() {
        ffi::wl_client_post_no_memory(client);
        return;
    }

    if format != ffi::river_xkb_config_v1_keymap_format_RIVER_XKB_CONFIG_V1_KEYMAP_FORMAT_TEXT_V1 {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_xkb_config_v1_error_RIVER_XKB_CONFIG_V1_ERROR_INVALID_FORMAT,
            c"invalid format enum value".as_ptr(),
        );
        return;
    }

    let mut statbuf = std::mem::MaybeUninit::<libc::stat>::uninit();
    if libc::fstat(fd, statbuf.as_mut_ptr()) != 0 {
        ffi::wl_resource_set_implementation(
            keymap_res,
            &XKB_KEYMAP_INTERFACE as *const _ as *const _,
            std::ptr::null_mut(),
            Some(xkb_keymap_destroy_func),
        );
        ffi::wl_resource_post_event(keymap_res, 1, c"failed to stat keymap fd".as_ptr());
        return;
    }
    let stat = statbuf.assume_init();
    if stat.st_size < 1 {
        ffi::wl_resource_set_implementation(
            keymap_res,
            &XKB_KEYMAP_INTERFACE as *const _ as *const _,
            std::ptr::null_mut(),
            Some(xkb_keymap_destroy_func),
        );
        ffi::wl_resource_post_event(keymap_res, 1, c"keymap too small".as_ptr());
        return;
    }
    if stat.st_size > 1024 * 1024 {
        ffi::wl_resource_set_implementation(
            keymap_res,
            &XKB_KEYMAP_INTERFACE as *const _ as *const _,
            std::ptr::null_mut(),
            Some(xkb_keymap_destroy_func),
        );
        ffi::wl_resource_post_event(keymap_res, 1, c"keymap too large".as_ptr());
        return;
    }

    let keymap_len = stat.st_size as usize;
    let map = libc::mmap(
        std::ptr::null_mut(),
        keymap_len,
        libc::PROT_READ,
        libc::MAP_PRIVATE,
        fd,
        0,
    );
    if map == libc::MAP_FAILED {
        ffi::wl_resource_set_implementation(
            keymap_res,
            &XKB_KEYMAP_INTERFACE as *const _ as *const _,
            std::ptr::null_mut(),
            Some(xkb_keymap_destroy_func),
        );
        ffi::wl_resource_post_event(keymap_res, 1, c"failed to mmap() keymap fd".as_ptr());
        return;
    }

    let context = (*config).context;
    let xkb_keymap = ffi::xkb_keymap_new_from_buffer(
        context,
        map as *const _,
        keymap_len - 1,
        ffi::xkb_keymap_format_XKB_KEYMAP_FORMAT_TEXT_V1,
        ffi::xkb_keymap_compile_flags_XKB_KEYMAP_COMPILE_NO_FLAGS,
    );

    libc::munmap(map, keymap_len);

    if xkb_keymap.is_null() {
        ffi::wl_resource_set_implementation(
            keymap_res,
            &XKB_KEYMAP_INTERFACE as *const _ as *const _,
            std::ptr::null_mut(),
            Some(xkb_keymap_destroy_func),
        );
        ffi::wl_resource_post_event(keymap_res, 1, c"failed to parse xkb keymap".as_ptr());
        return;
    }

    let xkb_keymap_struct = Box::into_raw(Box::new(XkbKeymap {
        xkb_keymap,
    }));

    ffi::wl_resource_set_implementation(
        keymap_res,
        &XKB_KEYMAP_INTERFACE as *const _ as *const _,
        xkb_keymap_struct as *mut _,
        Some(xkb_keymap_destroy_func),
    );

    ffi::wl_resource_post_event(keymap_res, 0); // success event
}
