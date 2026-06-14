// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::input_device::InputDevice;
use crate::server::WlList;

pub struct XkbKeyboardState {
    pub layout_index: Option<u32>,
    pub layout_name: Option<*const std::os::raw::c_char>,
    pub capslock: Option<bool>,
    pub numlock: Option<bool>,
}

pub struct XkbKeyboard {
    pub parent_device: *mut InputDevice,
    pub objects: ffi::wl_list, // list of XkbKeyboardObject
    pub link: ffi::wl_list,    // link inside XkbConfig::keyboards
    pub sent: XkbKeyboardState,
}

pub struct XkbKeyboardObject {
    pub keyboard: *mut XkbKeyboard,
    pub resource: *mut ffi::wl_resource,
    pub link: ffi::wl_list,
}

impl XkbKeyboard {
    pub unsafe fn init(
        parent_device: *mut InputDevice,
    ) -> Box<Self> {
        let mut xkb_kbd = Box::new(Self {
            parent_device,
            objects: std::mem::zeroed(),
            link: std::mem::zeroed(),
            sent: XkbKeyboardState {
                layout_index: None,
                layout_name: None,
                capslock: None,
                numlock: None,
            },
        });

        ffi::wl_list_init(&mut xkb_kbd.objects);
        ffi::wl_list_init(&mut xkb_kbd.link);

        let server = (*(*parent_device).seat).server;
        let keyboards_head = &mut (*server).xkb_config.keyboards as *mut ffi::wl_list as *mut WlList;
        crate::server::wl_list_insert((*keyboards_head).prev, &mut xkb_kbd.link as *mut ffi::wl_list as *mut WlList);

        let config_objects = &mut (*server).xkb_config.objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*config_objects).next;
        while curr != config_objects {
            let next = (*curr).next;
            let config_obj = crate::container_of!(curr, crate::xkb_config::XkbConfigObject, link);
            xkb_kbd.create_object((*config_obj).resource);
            curr = next;
        }

        xkb_kbd
    }

    pub unsafe fn deinit(&mut self) {
        let objects_head = &mut self.objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*objects_head).next;
        while curr != objects_head {
            let next = (*curr).next;
            let obj = crate::container_of!(curr, XkbKeyboardObject, link);

            crate::server::wl_list_remove(curr);
            ffi::wl_list_init(curr as *mut ffi::wl_list);

            ffi::wl_resource_post_event((*obj).resource, 0); // removed event

            ffi::wl_resource_set_implementation(
                (*obj).resource,
                &XKB_KEYBOARD_INERT_INTERFACE as *const _ as *const _,
                obj as *mut _,
                Some(handle_xkb_keyboard_object_destroy),
            );

            curr = next;
        }

        crate::server::wl_list_remove(&mut self.link as *mut ffi::wl_list as *mut WlList);
    }

    pub unsafe fn create_object(&mut self, config_v1_resource: *mut ffi::wl_resource) {
        let client = ffi::wl_resource_get_client(config_v1_resource);
        let version = ffi::wl_resource_get_version(config_v1_resource);

        let resource = ffi::wl_resource_create(
            client,
            &ffi::river_xkb_keyboard_v1_interface,
            version,
            0,
        );
        if resource.is_null() {
            log::error!("out of memory creating river_xkb_keyboard_v1");
            ffi::wl_client_post_no_memory(client);
            return;
        }

        let obj = Box::into_raw(Box::new(XkbKeyboardObject {
            keyboard: self,
            resource,
            link: std::mem::zeroed(),
        }));

        ffi::wl_resource_set_implementation(
            resource,
            &XKB_KEYBOARD_INTERFACE as *const _ as *const _,
            obj as *mut _,
            Some(handle_xkb_keyboard_object_destroy),
        );

        let list_head = &mut self.objects as *mut ffi::wl_list as *mut WlList;
        crate::server::wl_list_insert(list_head, &mut (*obj).link as *mut ffi::wl_list as *mut WlList);

        // Send xkb_keyboard event to config_v1
        ffi::wl_resource_post_event(config_v1_resource, 1, resource); // opcode 1: xkb_keyboard

        // Pair with input device resource
        let parent_dev = self.parent_device;
        let input_dev_objects = &mut (*parent_dev).objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*input_dev_objects).next;
        while curr != input_dev_objects {
            let next = (*curr).next;
            let input_dev_obj = crate::container_of!(curr, crate::input_device::InputDeviceObject, link);
            if ffi::wl_resource_get_client((*input_dev_obj).resource) == client {
                ffi::wl_resource_post_event(resource, 1, (*input_dev_obj).resource); // input_device event (opcode 1)
            }
            curr = next;
        }

        // Send current cached state to client
        let sent = &self.sent;
        if let Some(layout_index) = sent.layout_index {
            let name_ptr = sent.layout_name.unwrap_or(std::ptr::null());
            ffi::wl_resource_post_event(resource, 2, layout_index, name_ptr); // layout event (opcode 2)
        }
        if let Some(capslock) = sent.capslock {
            if capslock {
                ffi::wl_resource_post_event(resource, 3); // capslock_enabled (opcode 3)
            } else {
                ffi::wl_resource_post_event(resource, 4); // capslock_disabled (opcode 4)
            }
        }
        if let Some(numlock) = sent.numlock {
            if numlock {
                ffi::wl_resource_post_event(resource, 5); // numlock_enabled (opcode 5)
            } else {
                ffi::wl_resource_post_event(resource, 6); // numlock_disabled (opcode 6)
            }
        }

        if version >= 2 {
            ffi::wl_resource_post_event(resource, 7); // done event (opcode 7)
        }
    }

    pub unsafe fn send_state(
        &mut self,
        layout_index: u32,
        layout_name: *const std::os::raw::c_char,
        capslock: bool,
        numlock: bool,
    ) {
        let sent = &mut self.sent;
        let objects_head = &mut self.objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*objects_head).next;
        while curr != objects_head {
            let next = (*curr).next;
            let obj = crate::container_of!(curr, XkbKeyboardObject, link);
            let resource = (*obj).resource;
            let version = ffi::wl_resource_get_version(resource);

            let mut send_done = false;

            // Check layout changed
            let layout_changed = match sent.layout_index {
                None => true,
                Some(idx) => idx != layout_index,
            } || match (sent.layout_name, layout_name.is_null()) {
                (None, false) => true,
                (Some(_), true) => true,
                (Some(old_ptr), false) => {
                    libc::strcmp(old_ptr, layout_name) != 0
                }
                (None, true) => false,
            };

            if layout_changed {
                ffi::wl_resource_post_event(resource, 2, layout_index, layout_name); // layout event (opcode 2)
                send_done = true;
            }

            // Check capslock changed
            let capslock_changed = match sent.capslock {
                None => true,
                Some(state) => state != capslock,
            };
            if capslock_changed {
                if capslock {
                    ffi::wl_resource_post_event(resource, 3); // capslock_enabled
                } else {
                    ffi::wl_resource_post_event(resource, 4); // capslock_disabled
                }
                send_done = true;
            }

            // Check numlock changed
            let numlock_changed = match sent.numlock {
                None => true,
                Some(state) => state != numlock,
            };
            if numlock_changed {
                if numlock {
                    ffi::wl_resource_post_event(resource, 5); // numlock_enabled
                } else {
                    ffi::wl_resource_post_event(resource, 6); // numlock_disabled
                }
                send_done = true;
            }

            if send_done && version >= 2 {
                ffi::wl_resource_post_event(resource, 7); // done event
            }

            curr = next;
        }

        sent.layout_index = Some(layout_index);
        sent.layout_name = if layout_name.is_null() { None } else { Some(layout_name) };
        sent.capslock = Some(capslock);
        sent.numlock = Some(numlock);
    }
}

unsafe extern "C" fn handle_xkb_keyboard_object_destroy(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbKeyboardObject;
    if !obj.is_null() {
        crate::server::wl_list_remove(&mut (*obj).link as *mut ffi::wl_list as *mut WlList);
        let _ = Box::from_raw(obj);
    }
}

static XKB_KEYBOARD_INTERFACE: ffi::river_xkb_keyboard_v1_interface = ffi::river_xkb_keyboard_v1_interface {
    destroy: Some(xkb_keyboard_destroy),
    set_keymap: Some(xkb_keyboard_set_keymap),
    set_layout_by_index: Some(xkb_keyboard_set_layout_by_index),
    set_layout_by_name: Some(xkb_keyboard_set_layout_by_name),
    capslock_enable: Some(xkb_keyboard_capslock_enable),
    capslock_disable: Some(xkb_keyboard_capslock_disable),
    numlock_enable: Some(xkb_keyboard_numlock_enable),
    numlock_disable: Some(xkb_keyboard_numlock_disable),
};

static XKB_KEYBOARD_INERT_INTERFACE: ffi::river_xkb_keyboard_v1_interface = ffi::river_xkb_keyboard_v1_interface {
    destroy: Some(xkb_keyboard_destroy),
    set_keymap: None,
    set_layout_by_index: None,
    set_layout_by_name: None,
    capslock_enable: None,
    capslock_disable: None,
    numlock_enable: None,
    numlock_disable: None,
};

unsafe extern "C" fn xkb_keyboard_destroy(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn xkb_keyboard_set_keymap(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    keymap_resource: *mut ffi::wl_resource,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbKeyboardObject;
    if obj.is_null() {
        return;
    }
    let keyboard = (*(*obj).keyboard).parent_device;
    let keymap_struct = ffi::wl_resource_get_user_data(keymap_resource) as *mut crate::xkb_config::XkbKeymap;
    if keymap_struct.is_null() {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_xkb_keyboard_v1_error_RIVER_XKB_KEYBOARD_V1_ERROR_INVALID_KEYMAP,
            b"client set invalid keymap\0".as_ptr() as *const _,
        );
        return;
    }

    let kbd = (*keyboard).destroy_data as *mut crate::keyboard::Keyboard;
    if !kbd.is_null() {
        (*kbd).set_keymap((*keymap_struct).xkb_keymap);
    }
}

unsafe extern "C" fn xkb_keyboard_set_layout_by_index(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    index: i32,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbKeyboardObject;
    if obj.is_null() {
        return;
    }
    let keyboard = (*(*obj).keyboard).parent_device;
    let kbd = (*keyboard).destroy_data as *mut crate::keyboard::Keyboard;
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).config.keymap;
    if keymap.is_null() {
        return;
    }

    let num_layouts = ffi::xkb_keymap_num_layouts(keymap);
    if index < 0 || (index as u32) >= num_layouts {
        return;
    }

    let mut modifiers = (*group).wlr_keyboard.modifiers;
    modifiers.group = index as u32;
    (*group).process_modifiers(modifiers);
}

unsafe extern "C" fn xkb_keyboard_set_layout_by_name(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    name: *const std::os::raw::c_char,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbKeyboardObject;
    if obj.is_null() || name.is_null() {
        return;
    }
    let keyboard = (*(*obj).keyboard).parent_device;
    let kbd = (*keyboard).destroy_data as *mut crate::keyboard::Keyboard;
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).config.keymap;
    if keymap.is_null() {
        return;
    }

    let index = ffi::xkb_keymap_layout_get_index(keymap, name);
    if index == ffi::XKB_LAYOUT_INVALID {
        return;
    }

    let mut modifiers = (*group).wlr_keyboard.modifiers;
    modifiers.group = index;
    (*group).process_modifiers(modifiers);
}

unsafe extern "C" fn xkb_keyboard_capslock_enable(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbKeyboardObject;
    if obj.is_null() {
        return;
    }
    let keyboard = (*(*obj).keyboard).parent_device;
    let kbd = (*keyboard).destroy_data as *mut crate::keyboard::Keyboard;
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).config.keymap;
    if keymap.is_null() {
        return;
    }

    let caps_idx = ffi::xkb_keymap_mod_get_index(keymap, b"Caps Lock\0".as_ptr() as *const _);
    if caps_idx != ffi::XKB_MOD_INVALID {
        let mask = 1 << caps_idx;
        let mut modifiers = (*group).wlr_keyboard.modifiers;
        modifiers.locked |= mask;
        (*group).process_modifiers(modifiers);
    }
}

unsafe extern "C" fn xkb_keyboard_capslock_disable(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbKeyboardObject;
    if obj.is_null() {
        return;
    }
    let keyboard = (*(*obj).keyboard).parent_device;
    let kbd = (*keyboard).destroy_data as *mut crate::keyboard::Keyboard;
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).config.keymap;
    if keymap.is_null() {
        return;
    }

    let caps_idx = ffi::xkb_keymap_mod_get_index(keymap, b"Caps Lock\0".as_ptr() as *const _);
    if caps_idx != ffi::XKB_MOD_INVALID {
        let mask = 1 << caps_idx;
        let mut modifiers = (*group).wlr_keyboard.modifiers;
        modifiers.locked &= !mask;
        (*group).process_modifiers(modifiers);
    }
}

unsafe extern "C" fn xkb_keyboard_numlock_enable(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbKeyboardObject;
    if obj.is_null() {
        return;
    }
    let keyboard = (*(*obj).keyboard).parent_device;
    let kbd = (*keyboard).destroy_data as *mut crate::keyboard::Keyboard;
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).config.keymap;
    if keymap.is_null() {
        return;
    }

    let num_idx = ffi::xkb_keymap_mod_get_index(keymap, b"Num Lock\0".as_ptr() as *const _);
    if num_idx != ffi::XKB_MOD_INVALID {
        let mask = 1 << num_idx;
        let mut modifiers = (*group).wlr_keyboard.modifiers;
        modifiers.locked |= mask;
        (*group).process_modifiers(modifiers);
    }
}

unsafe extern "C" fn xkb_keyboard_numlock_disable(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbKeyboardObject;
    if obj.is_null() {
        return;
    }
    let keyboard = (*(*obj).keyboard).parent_device;
    let kbd = (*keyboard).destroy_data as *mut crate::keyboard::Keyboard;
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).config.keymap;
    if keymap.is_null() {
        return;
    }

    let num_idx = ffi::xkb_keymap_mod_get_index(keymap, b"Num Lock\0".as_ptr() as *const _);
    if num_idx != ffi::XKB_MOD_INVALID {
        let mask = 1 << num_idx;
        let mut modifiers = (*group).wlr_keyboard.modifiers;
        modifiers.locked &= !mask;
        (*group).process_modifiers(modifiers);
    }
}
