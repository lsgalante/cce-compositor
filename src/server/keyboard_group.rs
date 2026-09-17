// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::seat::Seat;
use crate::keyboard::KeyboardConfig;
use crate::xkb_bindings::XkbBinding;
use crate::server::wl_listener_remove;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyConsumer {
    Builtin,
    Binding(*mut XkbBinding),
    CceBinding(crate::config::Keybind),
    EnsureEaten,
    ImGrab,
    Focus,
}

pub struct Press {
    pub consumer: KeyConsumer,
    pub count: u32,
}

pub struct KeyboardGroup {
    pub ref_count: u32,
    pub seat: *mut Seat,
    pub link: ffi::wl_list, // Seat.keyboard_groups
    pub virtual_device: bool,
    pub config: KeyboardConfig,
    pub wlr_keyboard: ffi::wlr_keyboard,
    pub modifiers_old: u32,
    pub pressed: HashMap<u32, Press>,
    pub key_listener: ffi::wl_listener,
    pub modifiers_listener: ffi::wl_listener,
    pub keyboards: ffi::wl_list, // list of keyboards in this group
}

impl KeyboardGroup {
    pub unsafe fn create(
        seat: *mut Seat,
        config: KeyboardConfig,
        virtual_device: bool,
    ) -> Result<*mut Self, &'static str> {
        let mut group = Box::new(Self {
            ref_count: 1,
            seat,
            link: std::mem::zeroed(),
            virtual_device,
            config,
            wlr_keyboard: std::mem::zeroed(),
            modifiers_old: 0,
            pressed: HashMap::new(),
            key_listener: std::mem::zeroed(),
            modifiers_listener: std::mem::zeroed(),
            keyboards: std::mem::zeroed(),
        });

        ffi::wl_list_init(&mut group.keyboards);
        ffi::wl_list_init(&mut group.link);

        // Add to seat.keyboard_groups
        let seat_groups_head = &mut (*seat).keyboard_groups as *mut ffi::wl_list as *mut crate::server::WlList;
        crate::server::wl_list_insert((*seat_groups_head).prev, &mut group.link as *mut ffi::wl_list as *mut crate::server::WlList);

        let group_ptr = Box::into_raw(group);

        ffi::river_wlr_keyboard_init(
            &mut (*group_ptr).wlr_keyboard,
            Some(led_update),
            b"river.KeyboardGroup\0".as_ptr() as *const _,
        );

        ffi::river_wlr_keyboard_set_data(&mut (*group_ptr).wlr_keyboard, group_ptr as *mut _);

        if !config.keymap.is_null() {
            ffi::wlr_keyboard_set_keymap(&mut (*group_ptr).wlr_keyboard, config.keymap);
        }
        ffi::wlr_keyboard_set_repeat_info(&mut (*group_ptr).wlr_keyboard, config.repeat_rate, config.repeat_delay);

        let key_listener_ptr = &mut (*group_ptr).key_listener as *mut ffi::wl_listener as *mut crate::server::WlListener;
        (*key_listener_ptr).notify = Some(handle_group_key);
        let key_signal = ffi::river_wlr_keyboard_get_key_signal(&mut (*group_ptr).wlr_keyboard);
        crate::server::wl_signal_add(key_signal, &mut (*group_ptr).key_listener);

        let modifiers_listener_ptr = &mut (*group_ptr).modifiers_listener as *mut ffi::wl_listener as *mut crate::server::WlListener;
        (*modifiers_listener_ptr).notify = Some(handle_group_modifiers);
        let modifiers_signal = ffi::river_wlr_keyboard_get_modifiers_signal(&mut (*group_ptr).wlr_keyboard);
        crate::server::wl_signal_add(modifiers_signal, &mut (*group_ptr).modifiers_listener);

        if !config.keymap.is_null() {
            ffi::xkb_keymap_ref(config.keymap);
        }

        Ok(group_ptr)
    }

    pub unsafe fn ref_group(&mut self) -> *mut Self {
        self.ref_count += 1;
        self
    }

    pub unsafe fn unref(&mut self, to_release: &[u32]) {
        for &keycode in to_release {
            let mut event = ffi::wlr_keyboard_key_event {
                time_msec: crate::util::msec_timestamp(),
                keycode,
                update_state: true,
                state: ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED,
            };
            self.process_key(&mut event);
        }

        self.ref_count -= 1;
        if self.ref_count > 0 {
            return;
        }

        crate::server::wl_list_remove(&mut self.link as *mut ffi::wl_list as *mut crate::server::WlList);
        wl_listener_remove(&mut self.key_listener);
        wl_listener_remove(&mut self.modifiers_listener);

        // If the currently active keyboard of a seat is destroyed, we need to set a new active keyboard.
        let active_wlr_kbd = ffi::river_wlr_seat_get_keyboard((*self.seat).wlr_seat);
        if active_wlr_kbd == &mut self.wlr_keyboard {
            let seat_groups_head = &mut (*self.seat).keyboard_groups as *mut ffi::wl_list as *mut crate::server::WlList;
            let first_node = (*seat_groups_head).next;
            if first_node != seat_groups_head {
                let other_group = crate::container_of!(first_node, KeyboardGroup, link);
                ffi::wlr_seat_set_keyboard((*self.seat).wlr_seat, &mut (*other_group).wlr_keyboard);
            } else {
                ffi::wlr_seat_set_keyboard((*self.seat).wlr_seat, std::ptr::null_mut());
            }
        }

        ffi::wlr_keyboard_finish(&mut self.wlr_keyboard);

        if !self.config.keymap.is_null() {
            ffi::xkb_keymap_unref(self.config.keymap);
        }

        let _boxed = Box::from_raw(self);
    }

    pub unsafe fn match_config(&self, config: *mut KeyboardConfig) -> bool {
        if self.config.repeat_rate != (*config).repeat_rate {
            return false;
        }
        if self.config.repeat_delay != (*config).repeat_delay {
            return false;
        }
        if self.config.keymap == (*config).keymap {
            return true;
        }
        if self.config.keymap.is_null() || (*config).keymap.is_null() {
            return false;
        }

        let a_string_ptr = ffi::xkb_keymap_get_as_string(self.config.keymap, ffi::xkb_keymap_format_XKB_KEYMAP_FORMAT_TEXT_V1);
        if a_string_ptr.is_null() {
            return false;
        }
        let b_string_ptr = ffi::xkb_keymap_get_as_string((*config).keymap, ffi::xkb_keymap_format_XKB_KEYMAP_FORMAT_TEXT_V1);
        if b_string_ptr.is_null() {
            libc::free(a_string_ptr as *mut _);
            return false;
        }

        let a_str = std::ffi::CStr::from_ptr(a_string_ptr);
        let b_str = std::ffi::CStr::from_ptr(b_string_ptr);
        let matched = a_str == b_str;

        libc::free(a_string_ptr as *mut _);
        libc::free(b_string_ptr as *mut _);

        if matched {
            ffi::xkb_keymap_unref((*config).keymap);
            (*config).keymap = self.config.keymap;
            ffi::xkb_keymap_ref((*config).keymap);
        }

        matched
    }

    pub unsafe fn process_key(&mut self, event: *const ffi::wlr_keyboard_key_event) {
        if let Some(key) = self.pressed.get_mut(&(*event).keycode) {
            assert!(key.count > 0);
            if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED {
                key.count += 1;
            } else {
                key.count -= 1;
                if key.count == 0 {
                    let mut key_event = ffi::wlr_keyboard_key_event {
                        time_msec: (*event).time_msec,
                        keycode: (*event).keycode,
                        update_state: true,
                        state: ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED,
                    };
                    ffi::wlr_keyboard_notify_key(&mut self.wlr_keyboard, &mut key_event);
                }
            }
        } else if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED {
            if self.pressed.len() < 32 {
                let mut key_event = ffi::wlr_keyboard_key_event {
                    time_msec: (*event).time_msec,
                    keycode: (*event).keycode,
                    update_state: true,
                    state: ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED,
                };
                ffi::wlr_keyboard_notify_key(&mut self.wlr_keyboard, &mut key_event);
            }
        }
    }

    pub unsafe fn process_modifiers(&mut self, modifiers: ffi::wlr_keyboard_modifiers) {
        ffi::wlr_keyboard_notify_modifiers(
            &mut self.wlr_keyboard,
            modifiers.depressed,
            modifiers.latched,
            modifiers.locked,
            modifiers.group,
        );
    }

    pub unsafe fn process_keymap(&mut self, keymap: *mut ffi::xkb_keymap) {
        ffi::wlr_keyboard_set_keymap(&mut self.wlr_keyboard, keymap);
    }

    pub unsafe fn get_input_method_grab(&self) -> *mut ffi::wlr_input_method_keyboard_grab_v2 {
        if self.virtual_device {
            return std::ptr::null_mut();
        }
        let input_method = (*self.seat).relay.input_method;
        if !input_method.is_null() {
            return (*input_method).keyboard_grab;
        }
        std::ptr::null_mut()
    }

    pub unsafe fn send_state(&mut self) {
        let keymap = self.config.keymap;
        if keymap.is_null() {
            return;
        }
        let layout_index = self.wlr_keyboard.modifiers.group;
        let layout_name = ffi::xkb_keymap_layout_get_name(keymap, layout_index);
        let caps_idx = ffi::xkb_keymap_mod_get_index(keymap, b"Caps Lock\0".as_ptr() as *const _);
        let capslock = if caps_idx != ffi::XKB_MOD_INVALID {
            let caps_mask = 1 << caps_idx;
            (self.wlr_keyboard.modifiers.locked & caps_mask) != 0
        } else {
            false
        };
        let num_idx = ffi::xkb_keymap_mod_get_index(keymap, b"Num Lock\0".as_ptr() as *const _);
        let numlock = if num_idx != ffi::XKB_MOD_INVALID {
            let num_mask = 1 << num_idx;
            (self.wlr_keyboard.modifiers.locked & num_mask) != 0
        } else {
            false
        };

        let server = (*self.seat).server;
        let keyboards_head = &mut (*server).xkb_config.keyboards as *mut ffi::wl_list as *mut crate::server::WlList;
        let mut curr = (*keyboards_head).next;
        while curr != keyboards_head {
            let next = (*curr).next;
            let xkb_kbd = crate::container_of!(curr, crate::xkb_keyboard::XkbKeyboard, link);
            let parent_dev = (*xkb_kbd).parent_device;
            let kbd = (*parent_dev).destroy_data as *mut crate::keyboard::Keyboard;
            if !kbd.is_null() && (*kbd).group == self as *mut KeyboardGroup {
                (*xkb_kbd).send_state(layout_index, layout_name, capslock, numlock);
            }
            curr = next;
        }
    }
}

unsafe fn handle_builtin_binding(seat: *mut Seat, keysym: u32, modifiers: u32) -> bool {
    match keysym {
        ffi::XKB_KEY_XF86Switch_VT_1..=ffi::XKB_KEY_XF86Switch_VT_12 => {
            log::debug!("switch VT keysym received");
            let server = (*seat).server;
            let session = (*server).session;
            if !session.is_null() {
                let vt = keysym - ffi::XKB_KEY_XF86Switch_VT_1 + 1;
                log::info!("switching to VT {}", vt);
                ffi::wlr_session_change_vt(session, vt);
            }
            true
        }
        // Plain Escape closes any open in-surface status menu — the keyboard
        // twin of the click-away dismiss in cursor.rs, consumed the same way
        // a builtin is (the release is eaten with the press via the consumer
        // map), so the focused window never sees it. Gated on a menu
        // actually being open and on NO modifiers: a chorded Escape stays a
        // bindable/forwardable key, and with nothing expanded this arm never
        // fires at all.
        ffi::XKB_KEY_Escape if modifiers == 0 => {
            let server = (*seat).server;
            if !(*server).wm.any_expanded_status_segment(std::ptr::null_mut()) {
                return false;
            }
            log::debug!("Escape dismisses the open status menu");
            if let Some(ref sender) = (*server).wm.status_sender {
                sender.send_menu_dismiss("-");
            }
            true
        }
        _ => false,
    }
}

unsafe extern "C" fn led_update(wlr_keyboard: *mut ffi::wlr_keyboard, leds: u32) {
    let group = ffi::river_wlr_keyboard_get_data(wlr_keyboard) as *mut KeyboardGroup;
    if group.is_null() {
        return;
    }

    let keyboards_head = &mut (*group).keyboards as *mut ffi::wl_list as *mut crate::server::WlList;
    let mut curr = (*keyboards_head).next;
    while curr != keyboards_head {
        let next = (*curr).next;
        let keyboard = crate::container_of!(curr, crate::keyboard::Keyboard, group_link);
        ffi::wlr_keyboard_led_update((*keyboard).wlr_keyboard, leds);
        curr = next;
    }
}

unsafe extern "C" fn handle_group_key(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let group = &mut *crate::container_of!(listener, KeyboardGroup, key_listener);
    let event = data as *mut ffi::wlr_keyboard_key_event;

    let xkb_state = group.wlr_keyboard.xkb_state;
    if xkb_state.is_null() {
        log::error!("no xkb_state available");
        return;
    }

    // Keys are activity for the idle timeouts (and the idle-notify clients)
    // exactly as pointer events are; before 2026-09-16 only tablet, touch
    // and gestures counted, so idle-notify clients never saw a key.
    if !group.seat.is_null() {
        (*group.seat).handle_activity();
    }

    // A real key press ends an emulated view drag (see `cursor::ViewDrag`):
    // the drag holds Space and a pointer button down on the client's behalf,
    // and a key pressed on top of that reaches the app as a chord nobody
    // asked for. The drag's own synthetic Space goes straight to the seat and
    // never passes through here.
    if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED
        && !group.seat.is_null()
        && (*group.seat).cursor.view_drag.is_some()
    {
        (*group.seat).cursor.end_view_drag("key");
    }
    if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED
        && !group.seat.is_null()
        && (*group.seat).cursor.popup_wheel.is_some()
    {
        (*group.seat).cursor.end_popup_wheel("key");
    }
    if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED
        && !group.seat.is_null()
        && (*group.seat).cursor.hscroll_shift.is_some()
    {
        (*group.seat).cursor.end_hscroll_shift("key");
    }

    // Cancel active binding repeats
    let seat_groups_head = &mut (*group.seat).keyboard_groups as *mut ffi::wl_list as *mut crate::server::WlList;
    let mut curr_g = (*seat_groups_head).next;
    while curr_g != seat_groups_head {
        let next_g = (*curr_g).next;
        let g = crate::container_of!(curr_g, KeyboardGroup, link);
        for press in (*g).pressed.values() {
            if let KeyConsumer::Binding(binding) = press.consumer {
                if !binding.is_null() {
                    (*binding).stop_repeat();
                }
            }
        }
        curr_g = next_g;
    }

    let consumer: KeyConsumer = if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED {
        if let Some(kv) = group.pressed.remove(&(*event).keycode) {
            assert!(kv.count == 0);
            kv.consumer
        } else {
            KeyConsumer::Focus
        }
    } else {
        let xkb_keycode = (*event).keycode + 8;
        let modifiers = ffi::wlr_keyboard_get_modifiers(&mut group.wlr_keyboard);
        
        let mut matched_builtin = false;
        let mut syms_ptr: *const ffi::xkb_keysym_t = std::ptr::null();
        let num_syms = ffi::xkb_state_key_get_syms(xkb_state, xkb_keycode, &mut syms_ptr);
        log::debug!("handle_group_key keycode={}, xkb_keycode={}, num_syms={}", (*event).keycode, xkb_keycode, num_syms);
        if num_syms > 0 && !syms_ptr.is_null() {
            let syms = std::slice::from_raw_parts(syms_ptr, num_syms as usize);
            for &sym in syms {
                log::debug!("  keysym={:#x}", sym);
                if handle_builtin_binding(group.seat, sym, modifiers) {
                    matched_builtin = true;
                    break;
                }
            }
        }

        if matched_builtin {
            KeyConsumer::Builtin
        } else if let Some(kb) = match_cce_keybind(&(*(*group.seat).server).wm, xkb_keycode, modifiers, xkb_state) {
            log::debug!("matched CCE monolithic keybind: {:?}", kb);
            KeyConsumer::CceBinding(kb)
        } else if let Some(binding) = (*group.seat).match_xkb_binding(xkb_keycode, &mut group.wlr_keyboard) {
            log::debug!("matched xkb binding");
            (*group.seat).xkb_bindings_seat.ensure_next_key_eaten = false;
            KeyConsumer::Binding(if (*binding).sent_pressed {
                std::ptr::null_mut()
            } else {
                binding
            })
        } else if (*group.seat).xkb_bindings_seat.ensure_next_key_eaten {
            let mut has_non_modifier = false;
            let mut syms_ptr: *const ffi::xkb_keysym_t = std::ptr::null();
            let num_syms = ffi::xkb_state_key_get_syms(xkb_state, xkb_keycode, &mut syms_ptr);
            if num_syms > 0 && !syms_ptr.is_null() {
                let syms = std::slice::from_raw_parts(syms_ptr, num_syms as usize);
                for &sym in syms {
                    if !crate::keyboard::keysym_is_modifier(sym) {
                        has_non_modifier = true;
                        break;
                    }
                }
            }
            if has_non_modifier {
                (*group.seat).xkb_bindings_seat.ensure_next_key_eaten = false;
                KeyConsumer::EnsureEaten
            } else {
                KeyConsumer::Focus
            }
        } else if !group.get_input_method_grab().is_null() {
            KeyConsumer::ImGrab
        } else {
            KeyConsumer::Focus
        }
    };

    if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED {
        group.pressed.insert(
            (*event).keycode,
            Press {
                consumer: consumer.clone(),
                count: 1,
            },
        );
    }

    match consumer {
        KeyConsumer::Builtin => {}
        KeyConsumer::CceBinding(kb) => {
            if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED {
                let wm = &mut (*(*group.seat).server).wm;
                // A key carries no pointer position: the overview toggle's
                // exit must land on the focused window, not on whatever the
                // pointer was left hovering (or the empty desktop under it).
                // Same substitution the control socket makes.
                let action = if kb.action == crate::config::Action::Overview {
                    wm.overview_action_pointerless()
                } else {
                    kb.action
                };
                log::info!("executing CCE monolithic action: {:?}", action);
                wm.execute_action(&action, kb.command.as_deref());
            }
        }
        KeyConsumer::Binding(binding) => {
            if !binding.is_null() {
                if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED {
                    (*binding).pressed();
                } else {
                    (*binding).released();
                }
            }
        }
        KeyConsumer::EnsureEaten => {
            if (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED {
                (*group.seat).xkb_bindings_seat.scheduled_ate_unbound_key = true;
                (*(*group.seat).server).wm.dirty_windowing();
            }
        }
        KeyConsumer::ImGrab => {
            let grab = group.get_input_method_grab();
            if !grab.is_null() {
                ffi::wlr_input_method_keyboard_grab_v2_set_keyboard(grab, &mut group.wlr_keyboard);
                ffi::wlr_input_method_keyboard_grab_v2_send_key(grab, (*event).time_msec, (*event).keycode, (*event).state);
            }
        }
        KeyConsumer::Focus => {
            // Overlay AND Popup: desktop chrome (docks, the cce-cloud
            // launcher) stays keyboard-interactive in overview — only world
            // windows (spatial thumbnails) have their presses eaten.
            let is_overlay_mode = (*group.seat).focus_is_chrome();

            if (*(*group.seat).server).wm.mode != crate::window_manager::WindowManagerMode::Overview
                || is_overlay_mode
                || (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED
            {
                ffi::wlr_seat_set_keyboard((*group.seat).wlr_seat, &mut group.wlr_keyboard);
                ffi::wlr_seat_keyboard_notify_key((*group.seat).wlr_seat, (*event).time_msec, (*event).keycode, (*event).state);
            }
        }
    }

    group.send_state();
}

pub unsafe fn match_cce_keybind(
    wm: &crate::window_manager::WindowManager,
    keycode: u32,
    modifiers: u32,
    xkb_state: *mut ffi::xkb_state,
) -> Option<crate::config::Keybind> {
    if xkb_state.is_null() {
        return None;
    }
    let keymap = ffi::xkb_state_get_keymap(xkb_state);
    if keymap.is_null() {
        return None;
    }
    let layout = ffi::xkb_state_key_get_layout(xkb_state, keycode);

    let mut syms_ptr: *const ffi::xkb_keysym_t = std::ptr::null();
    let num_syms = ffi::xkb_keymap_key_get_syms_by_level(keymap, keycode, layout, 0, &mut syms_ptr);
    if num_syms > 0 && !syms_ptr.is_null() {
        let syms = std::slice::from_raw_parts(syms_ptr, num_syms as usize);
        for kb in &wm.keybinds {
            if kb.mods == modifiers {
                for &sym in syms {
                    if sym == kb.keysym {
                        return Some(kb.clone());
                    }
                }
            }
        }
    }

    let level = ffi::xkb_state_key_get_level(xkb_state, keycode, layout);
    let mut syms_ptr_level: *const ffi::xkb_keysym_t = std::ptr::null();
    let num_syms_level = ffi::xkb_keymap_key_get_syms_by_level(keymap, keycode, layout, level, &mut syms_ptr_level);
    if num_syms_level > 0 && !syms_ptr_level.is_null() {
        let syms = std::slice::from_raw_parts(syms_ptr_level, num_syms_level as usize);
        let consumed = ffi::xkb_state_key_get_consumed_mods2(xkb_state, keycode, ffi::xkb_consumed_mode_XKB_CONSUMED_MODE_XKB);
        let modifiers_translated = modifiers & !consumed;
        for kb in &wm.keybinds {
            if kb.mods == modifiers_translated {
                for &sym in syms {
                    if sym == kb.keysym {
                        return Some(kb.clone());
                    }
                }
            }
        }
    }

    None
}

unsafe extern "C" fn handle_group_modifiers(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let group = &mut *crate::container_of!(listener, KeyboardGroup, modifiers_listener);

    let old = group.modifiers_old;
    let new = ffi::wlr_keyboard_get_modifiers(&mut group.wlr_keyboard);
    let watched = (*group.seat).xkb_bindings_seat.requested_mods_watched;
    if (old & watched) != (new & watched) {
        (*group.seat).xkb_bindings_seat.scheduled_mods_update = Some(crate::xkb_bindings::XkbBindingsSeatModsUpdate {
            old,
            new,
        });
        (*(*group.seat).server).wm.dirty_windowing();
    }
    group.modifiers_old = new;

    let grab = group.get_input_method_grab();
    if !grab.is_null() {
        ffi::wlr_input_method_keyboard_grab_v2_set_keyboard(grab, &mut group.wlr_keyboard);
        ffi::wlr_input_method_keyboard_grab_v2_send_modifiers(grab, &mut group.wlr_keyboard.modifiers);
    } else {
        ffi::wlr_seat_set_keyboard((*group.seat).wlr_seat, &mut group.wlr_keyboard);
        ffi::wlr_seat_keyboard_notify_modifiers((*group.seat).wlr_seat, &mut group.wlr_keyboard.modifiers);
    }

    group.send_state();
}
