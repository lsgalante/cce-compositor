// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::input_device::InputDevice;
use crate::server::{WlListener, wl_listener_remove, wl_signal_add};
use crate::keyboard_group::KeyboardGroup;
use std::collections::HashSet;

#[derive(Clone, Copy)]
pub struct KeyboardConfig {
    pub keymap: *mut ffi::xkb_keymap,
    pub repeat_rate: i32,
    pub repeat_delay: i32,
}

pub struct Keyboard {
    pub device: *mut InputDevice,
    pub wlr_keyboard: *mut ffi::wlr_keyboard,
    pub key_listener: ffi::wl_listener,
    pub modifiers_listener: ffi::wl_listener,
    /// Registered for virtual keyboards only: their keymap arrives from the
    /// client *after* creation (the zwp_virtual_keyboard_v1 keymap request),
    /// so it must be forwarded to the group once it lands.
    pub keymap_listener: ffi::wl_listener,
    pub pressed: HashSet<u32>,
    pub group: *mut KeyboardGroup,
    pub group_link: ffi::wl_list,
    pub config: KeyboardConfig,
}

impl Keyboard {
    pub unsafe fn create(device: *mut InputDevice) -> *mut Self {
        let wlr_keyboard = ffi::wlr_keyboard_from_input_device((*device).wlr_device);
        
        let virtual_device = (*device).virtual_device;
        let mut keymap = std::ptr::null_mut();
        if virtual_device {
            let kbd_keymap = ffi::river_wlr_keyboard_get_keymap(wlr_keyboard);
            if !kbd_keymap.is_null() {
                keymap = kbd_keymap;
                ffi::xkb_keymap_ref(keymap);
            }
        } else {
            let server = (*(*device).seat).server;
            keymap = (*server).xkb_config.default_keymap;
            if !keymap.is_null() {
                ffi::xkb_keymap_ref(keymap);
            }
        }

        let keyboard = Box::into_raw(Box::new(Self {
            device,
            wlr_keyboard,
            key_listener: std::mem::zeroed(),
            modifiers_listener: std::mem::zeroed(),
            keymap_listener: std::mem::zeroed(),
            pressed: HashSet::new(),
            group: std::ptr::null_mut(),
            group_link: std::mem::zeroed(),
            config: KeyboardConfig {
                keymap,
                repeat_rate: 40,
                repeat_delay: 400,
            },
        }));

        ffi::wl_list_init(&mut (*keyboard).group_link);
        ffi::river_wlr_keyboard_set_data(wlr_keyboard, keyboard as *mut _);

        let key_listener_ptr = &mut (*keyboard).key_listener as *mut ffi::wl_listener as *mut WlListener;
        (*key_listener_ptr).notify = Some(handle_key);

        let modifiers_listener_ptr = &mut (*keyboard).modifiers_listener as *mut ffi::wl_listener as *mut WlListener;
        (*modifiers_listener_ptr).notify = Some(handle_modifiers);

        let key_signal = ffi::river_wlr_keyboard_get_key_signal(wlr_keyboard);
        wl_signal_add(key_signal, &mut (*keyboard).key_listener);

        let modifiers_signal = ffi::river_wlr_keyboard_get_modifiers_signal(wlr_keyboard);
        wl_signal_add(modifiers_signal, &mut (*keyboard).modifiers_listener);

        if virtual_device {
            let keymap_listener_ptr = &mut (*keyboard).keymap_listener as *mut ffi::wl_listener as *mut WlListener;
            (*keymap_listener_ptr).notify = Some(handle_keymap);
            let keymap_signal = ffi::river_wlr_keyboard_get_keymap_signal(wlr_keyboard);
            wl_signal_add(keymap_signal, &mut (*keyboard).keymap_listener);
        }

        if !virtual_device && should_set_keymap((*(*device).seat).server) {
            if !keymap.is_null() {
                ffi::wlr_keyboard_set_keymap(wlr_keyboard, keymap);
            }
        }

        (*device).destroy_fn = Some(destroy_callback);
        (*device).destroy_data = keyboard as *mut _;

        keyboard
    }

    pub unsafe fn destroy(keyboard: *mut Self) {
        wl_listener_remove(&mut (*keyboard).key_listener);
        wl_listener_remove(&mut (*keyboard).modifiers_listener);
        // destroy_fn runs before the InputDevice is freed, so device is valid.
        if (*(*keyboard).device).virtual_device {
            wl_listener_remove(&mut (*keyboard).keymap_listener);
        }

        if !(*keyboard).group.is_null() {
            let keys: Vec<u32> = (*keyboard).pressed.iter().cloned().collect();
            (*(*keyboard).group).unref(&keys);
            (*keyboard).group = std::ptr::null_mut();
        }

        if !(*keyboard).config.keymap.is_null() {
            ffi::xkb_keymap_unref((*keyboard).config.keymap);
        }

        let _boxed = Box::from_raw(keyboard);
    }

    pub unsafe fn set_group(&mut self) {
        assert!(self.group.is_null());
        let seat = (*self.device).seat;

        if !(*self.device).virtual_device {
            let seat_groups_head = &mut (*seat).keyboard_groups as *mut ffi::wl_list as *mut crate::server::WlList;
            let mut curr = (*seat_groups_head).next;
            while curr != seat_groups_head {
                let next = (*curr).next;
                let group = crate::container_of!(curr, KeyboardGroup, link);
                // A virtual keyboard's group is private to it (its keymap can
                // change under it, and its IME grab is disabled) — hardware
                // keyboards must never join one.
                if (*group).virtual_device {
                    curr = next;
                    continue;
                }
                let config_ptr = &mut self.config as *mut KeyboardConfig;
                if (*group).match_config(config_ptr) {
                    self.group = (*group).ref_group();
                    let keyboards_head = &mut (*self.group).keyboards as *mut ffi::wl_list as *mut crate::server::WlList;
                    crate::server::wl_list_insert(keyboards_head, &mut self.group_link as *mut ffi::wl_list as *mut crate::server::WlList);
                    return;
                }
                curr = next;
            }
        }

        match KeyboardGroup::create(seat, self.config, (*self.device).virtual_device) {
            Ok(group_ptr) => {
                self.group = group_ptr;
                let keyboards_head = &mut (*self.group).keyboards as *mut ffi::wl_list as *mut crate::server::WlList;
                crate::server::wl_list_insert(keyboards_head, &mut self.group_link as *mut ffi::wl_list as *mut crate::server::WlList);
            }
            Err(err) => {
                log::error!("failed to create KeyboardGroup: {}", err);
            }
        }
    }

    pub unsafe fn set_repeat_info(&mut self, rate: i32, delay: i32) {
        assert!(!(*self.device).virtual_device);
        self.config.repeat_rate = rate;
        self.config.repeat_delay = delay;
        if !self.group.is_null() {
            let keys: Vec<u32> = self.pressed.iter().cloned().collect();
            crate::server::wl_list_remove(&mut self.group_link as *mut ffi::wl_list as *mut crate::server::WlList);
            (*self.group).unref(&keys);
            self.group = std::ptr::null_mut();
        }
        self.set_group();
    }

    pub unsafe fn set_keymap(&mut self, keymap: *mut ffi::xkb_keymap) {
        assert!(!(*self.device).virtual_device);
        if should_set_keymap((*(*self.device).seat).server) {
            ffi::wlr_keyboard_set_keymap(self.wlr_keyboard, keymap);
        }
        if !self.config.keymap.is_null() {
            ffi::xkb_keymap_unref(self.config.keymap);
        }
        self.config.keymap = keymap;
        if !keymap.is_null() {
            ffi::xkb_keymap_ref(keymap);
        }
        if !self.group.is_null() {
            let keys: Vec<u32> = self.pressed.iter().cloned().collect();
            crate::server::wl_list_remove(&mut self.group_link as *mut ffi::wl_list as *mut crate::server::WlList);
            (*self.group).unref(&keys);
            self.group = std::ptr::null_mut();
        }
        self.set_group();
    }
}

unsafe extern "C" fn destroy_callback(data: *mut std::ffi::c_void) {
    Keyboard::destroy(data as *mut Keyboard);
}

unsafe fn should_set_keymap(server: *mut crate::server::Server) -> bool {
    let backend = (*server).backend;
    ffi::wlr_backend_is_wl(backend) || ffi::wlr_backend_is_x11(backend)
}

pub unsafe fn keysym_is_modifier(sym: u32) -> bool {
    match sym {
        ffi::XKB_KEY_Shift_L |
        ffi::XKB_KEY_Shift_R |
        ffi::XKB_KEY_Control_L |
        ffi::XKB_KEY_Control_R |
        ffi::XKB_KEY_Caps_Lock |
        ffi::XKB_KEY_Shift_Lock |

        ffi::XKB_KEY_Meta_L |
        ffi::XKB_KEY_Meta_R |
        ffi::XKB_KEY_Alt_L |
        ffi::XKB_KEY_Alt_R |
        ffi::XKB_KEY_Super_L |
        ffi::XKB_KEY_Super_R |
        ffi::XKB_KEY_Hyper_L |
        ffi::XKB_KEY_Hyper_R |

        ffi::XKB_KEY_Num_Lock |

        ffi::XKB_KEY_ISO_Lock |
        ffi::XKB_KEY_ISO_Level2_Latch |
        ffi::XKB_KEY_ISO_Level3_Shift |
        ffi::XKB_KEY_ISO_Level3_Latch |
        ffi::XKB_KEY_ISO_Level3_Lock |
        ffi::XKB_KEY_ISO_Level5_Shift |
        ffi::XKB_KEY_ISO_Level5_Latch |
        ffi::XKB_KEY_ISO_Level5_Lock |
        ffi::XKB_KEY_ISO_Group_Shift |
        ffi::XKB_KEY_ISO_Group_Latch |
        ffi::XKB_KEY_ISO_Group_Lock |
        ffi::XKB_KEY_ISO_Next_Group |
        ffi::XKB_KEY_ISO_Next_Group_Lock |
        ffi::XKB_KEY_ISO_Prev_Group |
        ffi::XKB_KEY_ISO_Prev_Group_Lock |
        ffi::XKB_KEY_ISO_First_Group |
        ffi::XKB_KEY_ISO_First_Group_Lock |
        ffi::XKB_KEY_ISO_Last_Group |
        ffi::XKB_KEY_ISO_Last_Group_Lock => true,
        _ => false,
    }
}

unsafe extern "C" fn handle_key(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let keyboard = &mut *crate::container_of!(listener, Keyboard, key_listener);
    let event = data as *mut ffi::wlr_keyboard_key_event;

    if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED {
        keyboard.pressed.remove(&(*event).keycode);
    } else if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED {
        if keyboard.pressed.len() < 32 {
            keyboard.pressed.insert((*event).keycode);
        }
        // First deliberate input ends the session-restore settling phase
        // (see the focus gate in Window::map).
        if !keyboard.group.is_null() && !(*keyboard.group).seat.is_null() {
            (*(*(*keyboard.group).seat).server).wm.startup_input_seen = true;
        }
    }

    if !keyboard.group.is_null() {
        (*keyboard.group).process_key(event);
    }
}

/// Virtual keyboards only. The client's keymap request lands after the group
/// already exists, and the group's own wlr_keyboard is what processes keys —
/// with no keymap it has no xkb_state and every key is dropped. The group is
/// private to this keyboard (see set_group), so its config can be rewritten
/// in place instead of regrouping.
unsafe extern "C" fn handle_keymap(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let keyboard = &mut *crate::container_of!(listener, Keyboard, keymap_listener);
    let keymap = ffi::river_wlr_keyboard_get_keymap(keyboard.wlr_keyboard);

    if !keymap.is_null() {
        ffi::xkb_keymap_ref(keymap);
    }
    if !keyboard.config.keymap.is_null() {
        ffi::xkb_keymap_unref(keyboard.config.keymap);
    }
    keyboard.config.keymap = keymap;

    if !keyboard.group.is_null() {
        let group = &mut *keyboard.group;
        if !keymap.is_null() {
            ffi::xkb_keymap_ref(keymap);
        }
        if !group.config.keymap.is_null() {
            ffi::xkb_keymap_unref(group.config.keymap);
        }
        group.config.keymap = keymap;
        if !keymap.is_null() {
            group.process_keymap(keymap);
        }
    }
}

unsafe extern "C" fn handle_modifiers(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let keyboard = &mut *crate::container_of!(listener, Keyboard, modifiers_listener);

    if !keyboard.group.is_null() {
        let modifiers = ffi::river_wlr_keyboard_get_modifiers(keyboard.wlr_keyboard);
        (*keyboard.group).process_modifiers(*modifiers);
        
        let seat = (*keyboard.group).seat;
        if !seat.is_null() && !(*seat).server.is_null() {
            (*(*seat).server).wm.update_status();
            (*(*seat).server).wm.refresh_adjust_held();
        }
    }
}
