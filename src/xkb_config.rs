// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, wl_signal_add, wl_listener_remove, WlList, wl_list_insert, wl_list_remove};

pub struct XkbConfig {
    pub server: *mut Server,
    pub global: *mut ffi::wl_global,
    pub context: *mut ffi::xkb_context,
    pub default_keymap: *mut ffi::xkb_keymap,
    pub objects: ffi::wl_list,
    pub keyboards: ffi::wl_list,
}

pub struct XkbConfigObject {
    pub config: *mut XkbConfig,
    pub resource: *mut ffi::wl_resource,
    pub link: ffi::wl_list,
}

impl Default for XkbConfig {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl XkbConfig {
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        ffi::wl_list_init(&mut self.objects);
        ffi::wl_list_init(&mut self.keyboards);

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
            self.context = std::ptr::null_mut();
            return Err("Failed to create river_xkb_config_v1 global");
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
            let obj = crate::container_of!(curr, XkbConfigObject, link);
            ffi::wl_resource_destroy((*obj).resource);
            curr = next;
        }

        if !self.default_keymap.is_null() {
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
        link: std::mem::zeroed(),
    }));

    ffi::wl_resource_set_implementation(
        resource,
        &XKB_CONFIG_INTERFACE as *const _ as *const _,
        obj as *mut _,
        Some(handle_object_destroy),
    );

    let list_head = &mut (*config).objects as *mut ffi::wl_list as *mut WlList;
    wl_list_insert((*list_head).prev, &mut (*obj).link as *mut ffi::wl_list as *mut WlList);

    let keyboards_head = &mut (*config).keyboards as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*keyboards_head).next;
    while curr != keyboards_head {
        let next = (*curr).next;
        let xkb_kbd = crate::container_of!(curr, crate::xkb_keyboard::XkbKeyboard, link);
        (*xkb_kbd).create_object(resource);
        curr = next;
    }
}

unsafe extern "C" fn handle_object_destroy(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbConfigObject;
    if !obj.is_null() {
        wl_list_remove(&mut (*obj).link as *mut ffi::wl_list as *mut WlList);
        let _ = Box::from_raw(obj);
    }
}

static XKB_CONFIG_INTERFACE: ffi::river_xkb_config_v1_interface = ffi::river_xkb_config_v1_interface {
    stop: Some(xkb_config_stop),
    destroy: Some(xkb_config_destroy),
    create_keymap: Some(xkb_config_create_keymap),
};

static XKB_CONFIG_INERT_INTERFACE: ffi::river_xkb_config_v1_interface = ffi::river_xkb_config_v1_interface {
    stop: None,
    destroy: Some(xkb_config_inert_destroy),
    create_keymap: None,
};

unsafe extern "C" fn xkb_config_stop(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbConfigObject;
    if obj.is_null() {
        return;
    }

    wl_list_remove(&mut (*obj).link as *mut ffi::wl_list as *mut WlList);
    ffi::wl_list_init(&mut (*obj).link);

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
        b"destroy before finished event sent\0".as_ptr() as *const _,
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
            b"invalid format enum value\0".as_ptr() as *const _,
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
        ffi::wl_resource_post_event(keymap_res, 1, b"failed to stat keymap fd\0".as_ptr() as *const _);
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
        ffi::wl_resource_post_event(keymap_res, 1, b"keymap too small\0".as_ptr() as *const _);
        return;
    }
    if stat.st_size > 1024 * 1024 {
        ffi::wl_resource_set_implementation(
            keymap_res,
            &XKB_KEYMAP_INTERFACE as *const _ as *const _,
            std::ptr::null_mut(),
            Some(xkb_keymap_destroy_func),
        );
        ffi::wl_resource_post_event(keymap_res, 1, b"keymap too large\0".as_ptr() as *const _);
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
        ffi::wl_resource_post_event(keymap_res, 1, b"failed to mmap() keymap fd\0".as_ptr() as *const _);
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
        ffi::wl_resource_post_event(keymap_res, 1, b"failed to parse xkb keymap\0".as_ptr() as *const _);
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
