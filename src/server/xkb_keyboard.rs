// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::input_device::InputDevice;

pub struct XkbKeyboardState {
    pub layout_index: Option<u32>,
    /// A copy, not the pointer `xkb_keymap_layout_get_name` returned: that
    /// points into the keymap, which a regroup or a client's `set_keymap`
    /// can free while this keeps it. Until 2026-10-13 it was the pointer,
    /// and the next `send_state` compared against freed memory (and
    /// `create_object` sent it to a newly bound client).
    pub layout_name: Option<std::ffi::CString>,
    pub capslock: Option<bool>,
    pub numlock: Option<bool>,
}

impl XkbKeyboardState {
    /// Whether layout `index` named `name` differs from what was last sent.
    pub fn layout_changed(&self, index: u32, name: Option<&std::ffi::CStr>) -> bool {
        self.layout_index != Some(index) || self.layout_name.as_deref() != name
    }

    /// Record layout `index` named `name` as sent, keeping a copy of the
    /// name.
    pub fn record_layout(&mut self, index: u32, name: Option<&std::ffi::CStr>) {
        self.layout_index = Some(index);
        self.layout_name = name.map(std::ffi::CStr::to_owned);
    }
}

pub struct XkbKeyboard {
    pub parent_device: *mut InputDevice,
    /// Its protocol objects (one per bound client). Private: walks trust
    /// that a registered object is live; `deinit` lets go of them all.
    objects: crate::registry::Registry<XkbKeyboardObject>,
    pub sent: XkbKeyboardState,
}

pub struct XkbKeyboardObject {
    /// Null once the keyboard is gone (`deinit`): the object is inert.
    pub keyboard: *mut XkbKeyboard,
    pub resource: *mut ffi::wl_resource,
}

impl XkbKeyboard {
    /// # Safety
    /// `server` must point to the live `Server`, and `parent_device` to the
    /// live device that keeps the box. The box must be
    /// [`deinit`](Self::deinit)ed before it is dropped: its address is
    /// registered with the xkb config and handed to clients.
    pub unsafe fn init(server: *mut crate::server::Server, parent_device: *mut InputDevice,
    ) -> Box<Self> {
        let mut xkb_kbd = Box::new(Self {
            parent_device,
            objects: crate::registry::Registry::new(),
            sent: XkbKeyboardState {
                layout_index: None,
                layout_name: None,
                capslock: None,
                numlock: None,
            },
        });

        // The box's contents stay put while the box moves; `deinit`
        // unregisters it before the box is dropped.
        (*server).xkb_config.register_keyboard(&mut *xkb_kbd);

        for config_obj in (*server).xkb_config.objects() {
            xkb_kbd.create_object((*config_obj).resource);
        }

        xkb_kbd
    }

    /// # Safety
    /// The box is dropped next, with no [`create_object`](Self::create_object)
    /// in between: an object made after this would name a freed keyboard.
    pub unsafe fn deinit(&mut self) {
        for obj in self.objects.ptrs() {
            self.objects.remove(obj);
            // Inert from here: its destroy handler finds no keyboard.
            (*obj).keyboard = std::ptr::null_mut();

            ffi::wl_resource_post_event((*obj).resource, 0); // removed event

            ffi::wl_resource_set_implementation(
                (*obj).resource,
                &XKB_KEYBOARD_INERT_INTERFACE as *const _ as *const _,
                obj as *mut _,
                Some(handle_xkb_keyboard_object_destroy),
            );
        }

        let server = crate::shared::server_ptr();
        if !server.is_null() {
            (*server).xkb_config.unregister_keyboard(self);
        }
    }

    /// # Safety
    /// `config_v1_resource` is a live `river_xkb_config_v1` resource; the
    /// keyboard is between [`init`](Self::init) and [`deinit`](Self::deinit)
    /// (which makes the new object inert), its `parent_device` live, and its
    /// recorded layout name valid (as [`send_state`](Self::send_state)).
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
        }));

        ffi::wl_resource_set_implementation(
            resource,
            &XKB_KEYBOARD_INTERFACE as *const _ as *const _,
            obj as *mut _,
            Some(handle_xkb_keyboard_object_destroy),
        );

        // At the front, as the list's head insert did; unregistered by its
        // destroy handler or by `deinit`.
        self.objects.push_front(obj);

        // Send xkb_keyboard event to config_v1
        ffi::wl_resource_post_event(config_v1_resource, 1, resource); // opcode 1: xkb_keyboard

        // Pair with input device resource
        let parent_dev = self.parent_device;
        for input_dev_obj in (*parent_dev).objects() {
            if ffi::wl_resource_get_client((*input_dev_obj).resource) == client {
                ffi::wl_resource_post_event(resource, 1, (*input_dev_obj).resource); // input_device event (opcode 1)
            }
        }

        // Send current cached state to client
        let sent = &self.sent;
        if let Some(layout_index) = sent.layout_index {
            let name_ptr = sent.layout_name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr());
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

    /// Tell the bound clients what changed since the last call.
    ///
    /// # Safety
    /// `layout_name` is null or a C string live for the call (the group's
    /// keymap's own name for the layout).
    pub unsafe fn send_state(
        &mut self,
        layout_index: u32,
        layout_name: *const std::os::raw::c_char,
        capslock: bool,
        numlock: bool,
    ) {
        let name = (!layout_name.is_null()).then(|| std::ffi::CStr::from_ptr(layout_name));
        let layout_changed = self.sent.layout_changed(layout_index, name);
        let sent = &mut self.sent;
        for obj in self.objects.ptrs() {
            let resource = (*obj).resource;
            let version = ffi::wl_resource_get_version(resource);

            let mut send_done = false;

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
        }

        sent.record_layout(layout_index, name);
        sent.capslock = Some(capslock);
        sent.numlock = Some(numlock);
    }
}

unsafe extern "C" fn handle_xkb_keyboard_object_destroy(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut XkbKeyboardObject;
    if !obj.is_null() {
        if !(*obj).keyboard.is_null() {
            (*(*obj).keyboard).objects.remove(obj);
        }
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

// Every request has a handler: libwayland aborts the whole compositor on a
// null entry, and a client can send any request on an inert object (one it
// sent before it saw the event that made the object inert, or a repeat).
static XKB_KEYBOARD_INERT_INTERFACE: ffi::river_xkb_keyboard_v1_interface = ffi::river_xkb_keyboard_v1_interface {
    destroy: Some(xkb_keyboard_destroy),
    set_keymap: Some(inert_set_keymap),
    set_layout_by_index: Some(inert_set_layout_by_index),
    set_layout_by_name: Some(inert_set_layout_by_name),
    capslock_enable: Some(inert_toggle),
    capslock_disable: Some(inert_toggle),
    numlock_enable: Some(inert_toggle),
    numlock_disable: Some(inert_toggle),
};

unsafe extern "C" fn inert_set_keymap(_: *mut ffi::wl_client, _: *mut ffi::wl_resource, _keymap: *mut ffi::wl_resource) {}
unsafe extern "C" fn inert_set_layout_by_index(_: *mut ffi::wl_client, _: *mut ffi::wl_resource, _index: i32) {}
unsafe extern "C" fn inert_set_layout_by_name(_: *mut ffi::wl_client, _: *mut ffi::wl_resource, _name: *const std::os::raw::c_char) {}
unsafe extern "C" fn inert_toggle(_: *mut ffi::wl_client, _: *mut ffi::wl_resource) {}

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
            c"client set invalid keymap".as_ptr(),
        );
        return;
    }

    let kbd = (*keyboard).keyboard();
    if !kbd.is_null() {
        let seat = {
            let wm = &*crate::reentry::wm(crate::shared::server_ptr());
            (*keyboard).sid.and_then(|sid| wm.seats.ptr(sid)).unwrap_or(std::ptr::null_mut())
        };
        // With the window manager let go: the regroup releases the
        // keyboard's keys, which runs the group's key handler.
        crate::keyboard::Keyboard::set_keymap(kbd, seat, (*keymap_struct).xkb_keymap);
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
    let kbd = (*keyboard).keyboard();
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).keymap();
    if keymap.is_null() {
        return;
    }

    let num_layouts = ffi::xkb_keymap_num_layouts(keymap);
    if index < 0 || (index as u32) >= num_layouts {
        return;
    }

    let mut modifiers = (*group).modifiers();
    modifiers.group = index as u32;
    crate::keyboard_group::KeyboardGroup::process_modifiers(group, modifiers);
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
    let kbd = (*keyboard).keyboard();
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).keymap();
    if keymap.is_null() {
        return;
    }

    let index = ffi::xkb_keymap_layout_get_index(keymap, name);
    if index == ffi::XKB_LAYOUT_INVALID {
        return;
    }

    let mut modifiers = (*group).modifiers();
    modifiers.group = index;
    crate::keyboard_group::KeyboardGroup::process_modifiers(group, modifiers);
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
    let kbd = (*keyboard).keyboard();
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).keymap();
    if keymap.is_null() {
        return;
    }

    let caps_idx = ffi::xkb_keymap_mod_get_index(keymap, c"Caps Lock".as_ptr());
    if caps_idx != ffi::XKB_MOD_INVALID {
        let mask = 1 << caps_idx;
        let mut modifiers = (*group).modifiers();
        modifiers.locked |= mask;
        crate::keyboard_group::KeyboardGroup::process_modifiers(group, modifiers);
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
    let kbd = (*keyboard).keyboard();
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).keymap();
    if keymap.is_null() {
        return;
    }

    let caps_idx = ffi::xkb_keymap_mod_get_index(keymap, c"Caps Lock".as_ptr());
    if caps_idx != ffi::XKB_MOD_INVALID {
        let mask = 1 << caps_idx;
        let mut modifiers = (*group).modifiers();
        modifiers.locked &= !mask;
        crate::keyboard_group::KeyboardGroup::process_modifiers(group, modifiers);
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
    let kbd = (*keyboard).keyboard();
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).keymap();
    if keymap.is_null() {
        return;
    }

    let num_idx = ffi::xkb_keymap_mod_get_index(keymap, c"Num Lock".as_ptr());
    if num_idx != ffi::XKB_MOD_INVALID {
        let mask = 1 << num_idx;
        let mut modifiers = (*group).modifiers();
        modifiers.locked |= mask;
        crate::keyboard_group::KeyboardGroup::process_modifiers(group, modifiers);
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
    let kbd = (*keyboard).keyboard();
    if kbd.is_null() {
        return;
    }
    let group = (*kbd).group;
    if group.is_null() {
        return;
    }

    let keymap = (*group).keymap();
    if keymap.is_null() {
        return;
    }

    let num_idx = ffi::xkb_keymap_mod_get_index(keymap, c"Num Lock".as_ptr());
    if num_idx != ffi::XKB_MOD_INVALID {
        let mask = 1 << num_idx;
        let mut modifiers = (*group).modifiers();
        modifiers.locked &= !mask;
        crate::keyboard_group::KeyboardGroup::process_modifiers(group, modifiers);
    }
}

#[cfg(test)]
mod tests {
    use super::XkbKeyboardState;
    use crate::ffi;

    fn unsent() -> XkbKeyboardState {
        XkbKeyboardState { layout_index: None, layout_name: None, capslock: None, numlock: None }
    }

    #[test]
    fn a_layout_changes_with_its_index_or_its_name() {
        let mut sent = unsent();
        assert!(sent.layout_changed(0, None), "nothing sent yet");
        sent.record_layout(0, Some(c"English (US)"));
        assert!(!sent.layout_changed(0, Some(c"English (US)")));
        assert!(sent.layout_changed(1, Some(c"English (US)")));
        assert!(sent.layout_changed(0, Some(c"German")));
        assert!(sent.layout_changed(0, None));
        sent.record_layout(0, None);
        assert!(!sent.layout_changed(0, None));
    }

    #[test]
    fn the_sent_layout_name_outlives_the_keymap_it_came_from() {
        let mut sent = unsent();
        {
            let context = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
            let keymap = xkbcommon::xkb::Keymap::new_from_names(
                &context, "", "", "us", "", None, xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS,
            )
            .expect("compile a us keymap");
            // The pointer `KeyboardGroup::send_state` hands over: into the
            // keymap's own memory.
            let name = unsafe { ffi::xkb_keymap_layout_get_name(keymap.get_raw_ptr() as *mut ffi::xkb_keymap, 0) };
            assert!(!name.is_null());
            sent.record_layout(0, Some(unsafe { std::ffi::CStr::from_ptr(name) }));
            // The last reference goes here, and the keymap with it: a group
            // unref or a client's `set_keymap` does this to the live one.
        }
        assert_eq!(sent.layout_name.as_deref(), Some(c"English (US)"));
        assert!(!sent.layout_changed(0, Some(c"English (US)")));
    }
}
