// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::seat::Seat;
use crate::input_device::InputDevice;
use crate::keyboard_group::KeyboardGroup;
use std::collections::HashSet;

/// Key repeat defaults (keys/s, ms before the first repeat). A hardware
/// keyboard takes `input { repeat_rate repeat_delay }` over these
/// (`InputConfig::repeat_info`); the synthetic and virtual keyboards keep
/// them. A client can still change a device's via river's
/// `input_device.set_repeat_info`, until the next config load.
pub const DEFAULT_REPEAT_RATE: i32 = 40;
pub const DEFAULT_REPEAT_DELAY: i32 = 250;

#[derive(Clone, Copy)]
pub struct KeyboardConfig {
    pub keymap: *mut ffi::xkb_keymap,
    pub repeat_rate: i32,
    pub repeat_delay: i32,
}

pub struct Keyboard {
    pub device: *mut InputDevice,
    pub wlr_keyboard: *mut ffi::wlr_keyboard,
    key_listener: crate::listener::Listener,
    modifiers_listener: crate::listener::Listener,
    /// Registered for virtual keyboards only: their keymap arrives from the
    /// client *after* creation (the zwp_virtual_keyboard_v1 keymap request),
    /// so it must be forwarded to the group once it lands.
    keymap_listener: crate::listener::Listener,
    pub pressed: HashSet<u32>,
    pub group: *mut KeyboardGroup,
    pub config: KeyboardConfig,
}

// SAFETY: made by `Box::into_raw` and freed only by its own destroy path,
// after its listeners are gone, and reached only through raw pointers: no safe
// API returns a `&mut`, a `Box` or a value of it, so nothing safe can move it.
unsafe impl crate::listener::Anchored for Keyboard {}

impl Keyboard {
    /// # Safety
    /// `device` must be a live `InputDevice` (from `InputDevice::new`) whose
    /// `wlr_device` is a keyboard and that has no keyboard yet: the device
    /// takes the result over, and its destroy path frees it (`destroy_fn`).
    pub unsafe fn create(device: *mut InputDevice) -> *mut Self {
        let wlr_keyboard = ffi::wlr_keyboard_from_input_device((*device).wlr_device());
        
        let virtual_device = (*device).virtual_device;
        let (repeat_rate, repeat_delay) = if virtual_device {
            (DEFAULT_REPEAT_RATE, DEFAULT_REPEAT_DELAY)
        } else {
            crate::shared::layout().input_config.repeat_info()
        };
        let mut keymap = std::ptr::null_mut();
        if virtual_device {
            let kbd_keymap = ffi::river_wlr_keyboard_get_keymap(wlr_keyboard);
            if !kbd_keymap.is_null() {
                keymap = kbd_keymap;
                ffi::xkb_keymap_ref(keymap);
            }
        } else {
            keymap = crate::shared::handles().default_keymap.ptr();
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
            config: KeyboardConfig {
                keymap,
                repeat_rate,
                repeat_delay,
            },
        }));

        ffi::river_wlr_keyboard_set_data(wlr_keyboard, keyboard as *mut _);



        let key_signal = ffi::river_wlr_keyboard_get_key_signal(wlr_keyboard);
        (*keyboard).key_listener.connect(key_signal, handle_key);

        let modifiers_signal = ffi::river_wlr_keyboard_get_modifiers_signal(wlr_keyboard);
        (*keyboard).modifiers_listener.connect(modifiers_signal, handle_modifiers);

        if virtual_device {
            let keymap_signal = ffi::river_wlr_keyboard_get_keymap_signal(wlr_keyboard);
            (*keyboard).keymap_listener.connect(keymap_signal, handle_keymap);
        }

        if !virtual_device && should_set_keymap() && !keymap.is_null() {
            ffi::wlr_keyboard_set_keymap(wlr_keyboard, keymap);
        }

        // The device's destroy handler frees it.
        (*device).set_keyboard(keyboard);

        keyboard
    }

    /// # Safety
    /// `keyboard` must be one [`create`](Self::create) returned, not yet
    /// destroyed, whose device is still live (this runs as its `destroy_fn`).
    /// Still in a group, no reference to a seat may be live (`KeyboardGroup::unref`).
    pub unsafe fn destroy(keyboard: *mut Self) {
        (*keyboard).key_listener.disconnect();
        (*keyboard).modifiers_listener.disconnect();
        // The device's destroy handler runs this before it frees the
        // device, so `device` is valid.
        if (*(*keyboard).device).virtual_device {
            (*keyboard).keymap_listener.disconnect();
        }

        // Normally gone already: the device is detached (which drops the
        // group) before its keyboard is destroyed. A group left here has no
        // seat to hand the active keyboard over on, and no keys to release:
        // wlroots lets go of a dying keyboard's keys (`wlr_keyboard_finish`)
        // before the device's destroy handler runs, and that releases any it
        // missed (`InputDevice::release_keys`). Out of the group before the
        // free: a group other keyboards keep alive would otherwise walk this
        // one, freed, on its next LED update.
        Self::leave_group(keyboard, std::ptr::null_mut());

        if !(*keyboard).config.keymap.is_null() {
            ffi::xkb_keymap_unref((*keyboard).config.keymap);
        }

        let _boxed = Box::from_raw(keyboard);
    }

    /// Let go of every key this keyboard holds, through its group: each one
    /// no other keyboard of the group still holds is released to whatever
    /// took its press (the focused client, a binding, a portal shortcut, the
    /// input method), as if lifted. The keyboard forgets them all. Run
    /// before the keyboard leaves its group, which releases nothing
    /// (`KeyboardGroup::unref`); a keyboard that leaves without it leaves
    /// its keys held down for the client that had them.
    ///
    /// # Safety
    /// `this` is live, and nothing holds a reference to it, to its group,
    /// to the window manager or to a `Sys`'s subsystems: a release runs the
    /// group's key handler (`handle_group_key`), an entry point that takes
    /// them all afresh. So this runs from an entry point before it binds any
    /// of them, never from a function handed `wm` or `sys`.
    pub unsafe fn release_pressed(this: *mut Self) {
        let keys: Vec<u32> = (*this).pressed.drain().collect();
        let group = (*this).group;
        if group.is_null() {
            return;
        }
        for keycode in keys {
            let event = ffi::wlr_keyboard_key_event {
                time_msec: crate::util::msec_timestamp(),
                keycode,
                update_state: true,
                state: ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED,
            };
            KeyboardGroup::process_key(group, &event);
        }
    }

    /// Take the keyboard out of its group, if it has one. Its keys are
    /// forgotten, not released: run `release_pressed` first wherever the
    /// key handler may run.
    ///
    /// # Safety
    /// `this` is live; `seat` is null or the group's live seat; no
    /// reference to a seat is live (as `KeyboardGroup::unref`).
    pub unsafe fn leave_group(this: *mut Self, seat: *mut Seat) {
        let group = (*this).group;
        if group.is_null() {
            return;
        }
        (*this).pressed.clear();
        (*group).unregister_keyboard(this);
        (*this).group = std::ptr::null_mut();
        KeyboardGroup::unref(group, seat);
    }

    /// Put the keyboard in a group of `seat` (its device's seat); nothing
    /// without one.
    ///
    /// # Safety
    /// `self` is a keyboard [`create`](Self::create) made, whose device is
    /// live; the group keeps its address until it leaves. `seat` is null or
    /// a live `Seat` the call may borrow mutably.
    pub unsafe fn set_group(&mut self, seat: *mut Seat) {
        assert!(self.group.is_null());
        if seat.is_null() {
            return;
        }

        if !(*self.device).virtual_device {
            for group in (*seat).keyboard_groups() {
                // A virtual keyboard's group is private to it (its keymap can
                // change under it, and its IME grab is disabled) — hardware
                // keyboards must never join one.
                if (*group).virtual_device {
                    continue;
                }
                let config_ptr = &mut self.config as *mut KeyboardConfig;
                if (*group).match_config(config_ptr) {
                    self.group = (*group).ref_group();
                    (*self.group).register_keyboard(self);
                    return;
                }
            }
        }

        match KeyboardGroup::create(seat, self.config, (*self.device).virtual_device) {
            Ok(group_ptr) => {
                self.group = group_ptr;
                (*self.group).register_keyboard(self);
            }
            Err(err) => {
                log::error!("failed to create KeyboardGroup: {}", err);
            }
        }
    }

    /// Regroup the keyboard under a new repeat rate and delay (groups are
    /// keyed by them), releasing what it holds first.
    ///
    /// # Safety
    /// As `release_pressed`; `seat` is null or the keyboard's live seat, and
    /// no reference to any seat may be live: leaving the old group may free
    /// it (`KeyboardGroup::unref`).
    pub unsafe fn set_repeat_info(this: *mut Self, seat: *mut Seat, rate: i32, delay: i32) {
        assert!(!(*(*this).device).virtual_device);
        Self::release_pressed(this);
        Self::leave_group(this, seat);
        (*this).config.repeat_rate = rate;
        (*this).config.repeat_delay = delay;
        (*this).set_group(seat);
    }

    /// Regroup the keyboard under a new keymap, releasing what it holds
    /// first.
    ///
    /// # Safety
    /// As `set_repeat_info`; `keymap` is null or a live `xkb_keymap` (the
    /// keyboard takes its own reference). Setting the keymap of the
    /// device's own keyboard runs `handle_modifiers`, which takes the window
    /// manager.
    pub unsafe fn set_keymap(this: *mut Self, seat: *mut Seat, keymap: *mut ffi::xkb_keymap) {
        assert!(!(*(*this).device).virtual_device);
        Self::release_pressed(this);
        if should_set_keymap() {
            ffi::wlr_keyboard_set_keymap((*this).wlr_keyboard, keymap);
        }
        Self::leave_group(this, seat);
        if !(*this).config.keymap.is_null() {
            ffi::xkb_keymap_unref((*this).config.keymap);
        }
        (*this).config.keymap = keymap;
        if !keymap.is_null() {
            ffi::xkb_keymap_ref(keymap);
        }
        (*this).set_group(seat);
    }
}

fn should_set_keymap() -> bool {
    let backend = &crate::shared::handles().backend;
    backend.is_wl() || backend.is_x11()
}

pub fn keysym_is_modifier(sym: u32) -> bool {
    matches!(
        sym,
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
        ffi::XKB_KEY_ISO_Last_Group_Lock
    )
}

unsafe extern "C" fn handle_key(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    // Raw, not `&mut`: the group's key handler that `process_key` runs reads
    // this keyboard back through its device (`KeyboardGroup::send_state`).
    let keyboard = crate::listener_owner!(listener, Keyboard, key_listener);
    let event = data as *mut ffi::wlr_keyboard_key_event;

    if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED {
        (*keyboard).pressed.remove(&(*event).keycode);
    } else if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED {
        if (*keyboard).pressed.len() < 32 {
            (*keyboard).pressed.insert((*event).keycode);
        }
        // First deliberate input ends the session-restore settling phase
        // (see the focus gate in Window::map).
        if !(*keyboard).group.is_null() && (*(*keyboard).group).sid.is_some() {
            (*crate::reentry::wm(crate::shared::server_ptr())).startup_input_seen = true;
        }
    }

    let group = (*keyboard).group;
    if !group.is_null() {
        KeyboardGroup::process_key(group, event);
    }
}

/// Virtual keyboards only. The client's keymap request lands after the group
/// already exists, and the group's own wlr_keyboard is what processes keys —
/// with no keymap it has no xkb_state and every key is dropped. The group is
/// private to this keyboard (see set_group), so its config can be rewritten
/// in place instead of regrouping.
unsafe extern "C" fn handle_keymap(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    // Raw, as in `handle_key`: `replace_keymap` runs the group's handler.
    let keyboard = crate::listener_owner!(listener, Keyboard, keymap_listener);
    let keymap = ffi::river_wlr_keyboard_get_keymap((*keyboard).wlr_keyboard);

    if !keymap.is_null() {
        ffi::xkb_keymap_ref(keymap);
    }
    if !(*keyboard).config.keymap.is_null() {
        ffi::xkb_keymap_unref((*keyboard).config.keymap);
    }
    (*keyboard).config.keymap = keymap;

    let group = (*keyboard).group;
    if !group.is_null() {
        KeyboardGroup::replace_keymap(group, keymap);
    }
}

unsafe extern "C" fn handle_modifiers(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    // Raw, as in `handle_key`: `process_modifiers` runs the group's handler.
    let keyboard = crate::listener_owner!(listener, Keyboard, modifiers_listener);
    let group = (*keyboard).group;

    if !group.is_null() {
        let modifiers = ffi::river_wlr_keyboard_get_modifiers((*keyboard).wlr_keyboard);
        KeyboardGroup::process_modifiers(group, *modifiers);
        
        if (*group).sid.is_some() && !crate::shared::server_ptr().is_null() {
            let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
            wm.update_status();
            wm.refresh_adjust_held();
        }
    }
}
