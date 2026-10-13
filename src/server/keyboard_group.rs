// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::seat::Seat;
use crate::keyboard::KeyboardConfig;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyConsumer {
    Builtin,
    CceBinding(crate::config::Keybind),
    /// A chord bound through the GlobalShortcuts portal backend (see
    /// `global_shortcuts`): the press and the release are both reported on
    /// the status socket's `shortcuts` topic and neither reaches the client.
    PortalShortcut { session: String, id: String },
    ImGrab,
    Focus,
}

pub struct Press {
    pub consumer: KeyConsumer,
    pub count: u32,
}

pub struct KeyboardGroup {
    pub ref_count: u32,
    /// The owning seat's id; `None` without one. The group keeps no
    /// pointer to its seat: the event handlers look it up by this id, and
    /// `unref`'s teardown is handed the seat by its caller.
    pub sid: Option<crate::seat::SeatId>,
    pub virtual_device: bool,
    /// What the group is keyed by. Private: the group holds a reference on
    /// `config.keymap`, which `send_state` and the xkb protocol handlers
    /// hand to xkbcommon; read it with `keymap()`, replace it with
    /// `replace_keymap`.
    config: KeyboardConfig,
    /// The group's own keyboard. Private: `match_chord` trusts its
    /// `xkb_state` (null, or the state wlroots made for the keymap it was
    /// last given), so only `wlr_keyboard_set_keymap` through
    /// `wlr_keyboard_ptr` may change it.
    wlr_keyboard: ffi::wlr_keyboard,
    pub modifiers_old: u32,
    pub pressed: HashMap<u32, Press>,
    key_listener: crate::listener::Listener,
    modifiers_listener: crate::listener::Listener,
    /// The keyboards in this group. Private: walks trust that a registered
    /// keyboard is live (it unregisters before it leaves the group or is
    /// freed).
    keyboards: crate::registry::Registry<crate::keyboard::Keyboard>,
}

impl KeyboardGroup {
    /// Its keyboards, copied out.
    pub(crate) fn keyboards(&self) -> Vec<*mut crate::keyboard::Keyboard> {
        self.keyboards.ptrs()
    }

    /// Add `keyboard` to the group.
    ///
    /// # Safety
    /// As `Registry::push`: `keyboard` is live until `unregister_keyboard`,
    /// which it runs before it leaves the group or is freed.
    pub(crate) unsafe fn register_keyboard(&mut self, keyboard: *mut crate::keyboard::Keyboard) {
        // At the front, as the list's head insert did.
        self.keyboards.push_front(keyboard);
    }

    pub(crate) fn unregister_keyboard(&mut self, keyboard: *mut crate::keyboard::Keyboard) {
        self.keyboards.remove(keyboard);
    }
}

// SAFETY: made by `Box::into_raw` and freed only by its own destroy path,
// after its listeners are gone, and reached only through raw pointers: no safe
// API returns a `&mut`, a `Box` or a value of it, so nothing safe can move it.
unsafe impl crate::listener::Anchored for KeyboardGroup {}

impl KeyboardGroup {
    /// The group keyboard's current modifier state.
    pub fn modifiers(&self) -> ffi::wlr_keyboard_modifiers {
        self.wlr_keyboard.modifiers
    }

    /// The group's keymap (it holds a reference), or null.
    pub fn keymap(&self) -> *mut ffi::xkb_keymap {
        self.config.keymap
    }

    /// Give a virtual keyboard's private group the keymap its client sent,
    /// in place of its own: groups of virtual keyboards are never shared
    /// (`Keyboard::set_group`), so nothing else is keyed by the old one.
    ///
    /// # Safety
    /// As `process_keymap`, which this runs; `keymap` is null or live.
    pub unsafe fn replace_keymap(this: *mut Self, keymap: *mut ffi::xkb_keymap) {
        if !keymap.is_null() {
            ffi::xkb_keymap_ref(keymap);
        }
        if !(*this).config.keymap.is_null() {
            ffi::xkb_keymap_unref((*this).config.keymap);
        }
        (*this).config.keymap = keymap;
        if !keymap.is_null() {
            Self::process_keymap(this, keymap);
        }
    }

    /// The group's `wlr_keyboard`, for wlroots calls.
    pub fn wlr_keyboard_ptr(&mut self) -> *mut ffi::wlr_keyboard {
        &mut self.wlr_keyboard
    }

    /// # Safety
    /// `seat` must be null or a live seat with no other live reference to
    /// it, and `config.keymap` null or a live keymap (the group takes its
    /// own reference).
    pub unsafe fn create(
        seat: *mut Seat,
        config: KeyboardConfig,
        virtual_device: bool,
    ) -> Result<*mut Self, &'static str> {
        let group = Box::new(Self {
            ref_count: 1,
            sid: if seat.is_null() { None } else { Some((*seat).id) },
            virtual_device,
            config,
            wlr_keyboard: std::mem::zeroed(),
            modifiers_old: 0,
            pressed: HashMap::new(),
            key_listener: std::mem::zeroed(),
            modifiers_listener: std::mem::zeroed(),
            keyboards: crate::registry::Registry::new(),
        });

        let group_ptr = Box::into_raw(group);
        if !seat.is_null() {
            (*seat).register_keyboard_group(group_ptr);
        }

        ffi::river_wlr_keyboard_init(
            &mut (*group_ptr).wlr_keyboard,
            Some(led_update),
            c"river.KeyboardGroup".as_ptr(),
        );

        ffi::river_wlr_keyboard_set_data(&mut (*group_ptr).wlr_keyboard, group_ptr as *mut _);

        if !config.keymap.is_null() {
            ffi::wlr_keyboard_set_keymap(&mut (*group_ptr).wlr_keyboard, config.keymap);
        }
        ffi::wlr_keyboard_set_repeat_info(&mut (*group_ptr).wlr_keyboard, config.repeat_rate, config.repeat_delay);

        let key_signal = ffi::river_wlr_keyboard_get_key_signal(&mut (*group_ptr).wlr_keyboard);
        (*group_ptr).key_listener.connect(key_signal, handle_group_key);

        let modifiers_signal = ffi::river_wlr_keyboard_get_modifiers_signal(&mut (*group_ptr).wlr_keyboard);
        (*group_ptr).modifiers_listener.connect(modifiers_signal, handle_group_modifiers);

        if !config.keymap.is_null() {
            ffi::xkb_keymap_ref(config.keymap);
        }

        Ok(group_ptr)
    }

    pub fn ref_group(&mut self) -> *mut Self {
        self.ref_count += 1;
        self
    }

    /// Drop a reference; the last one tears the group down. `seat` is the
    /// group's seat, from the caller (null when there is none to hand the
    /// active keyboard over on: wlroots then clears it from the seat as the
    /// keyboard finishes).
    ///
    /// It releases no keys, so it never runs the group's handlers: a
    /// keyboard leaving the group lets go of its keys first, from where the
    /// key handler may run (`Keyboard::release_pressed`), and the group's
    /// listeners are gone before `wlr_keyboard_finish` releases whatever
    /// its keyboard still holds. Until 2026-10-13 this took the keys to
    /// release and fed them through `process_key`, which ran
    /// `handle_group_key` — and it, the window manager and the subsystems
    /// afresh — from inside `Seat::detach_device` and the config reload,
    /// which held them.
    ///
    /// # Safety
    /// `this` is a live group and no reference to it is live: the last
    /// unref frees it. `seat` is null or live, and no reference to a seat
    /// is live: the last unref takes the group out of every seat's registry
    /// by place.
    pub unsafe fn unref(this: *mut Self, seat: *mut Seat) {
        (*this).ref_count -= 1;
        if (*this).ref_count > 0 {
            return;
        }

        // Out of whichever seat's registry holds it — not only the seat
        // handed in, which its caller found by slot: safe code can swap two
        // seats' contents. Found without the window manager, which device
        // teardown may be inside.
        for place in crate::seat::live_places() {
            (*place).unregister_keyboard_group(this);
        }
        (*this).key_listener.disconnect();
        (*this).modifiers_listener.disconnect();

        // If the currently active keyboard of a seat is destroyed, we need to set a new active keyboard.
        let wlr_keyboard = std::ptr::addr_of_mut!((*this).wlr_keyboard);
        if !seat.is_null() && std::ptr::eq((*seat).wlr_seat().keyboard(), wlr_keyboard) {
            match (*seat).keyboard_groups().first() {
                Some(&other_group) => ffi::wlr_seat_set_keyboard((*seat).wlr_seat().ptr(), &mut (*other_group).wlr_keyboard),
                None => ffi::wlr_seat_set_keyboard((*seat).wlr_seat().ptr(), std::ptr::null_mut()),
            }
        }

        ffi::wlr_keyboard_finish(wlr_keyboard);

        if !(*this).config.keymap.is_null() {
            ffi::xkb_keymap_unref((*this).config.keymap);
        }

        drop(Box::from_raw(this));
    }

    /// Change the group's repeat rate and delay in place, for a config
    /// reload that gives every member the same new values
    /// (`WindowManager::apply_key_repeat`). Runs none of the compositor's
    /// handlers: wlroots tells the seat's clients and the input method.
    ///
    /// # Safety
    /// `this` is a live group.
    pub unsafe fn set_repeat_info(this: *mut Self, rate: i32, delay: i32) {
        if (*this).config.repeat_rate == rate && (*this).config.repeat_delay == delay {
            return;
        }
        (*this).config.repeat_rate = rate;
        (*this).config.repeat_delay = delay;
        ffi::wlr_keyboard_set_repeat_info(std::ptr::addr_of_mut!((*this).wlr_keyboard), rate, delay);
    }

    /// # Safety
    /// `config` must point to a valid `KeyboardConfig`, and its keymap and
    /// the group's each be null or live. On a match `config`'s keymap
    /// reference is traded for one on the group's.
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

    /// Feed a member keyboard's key event to the group keyboard, counting
    /// presses across the group's keyboards: only the first press and the
    /// last release of a key reach it.
    ///
    /// The process_* functions take the group by pointer, not `&mut self`:
    /// what they hand wlroots makes it emit the group keyboard's key or
    /// modifiers signal, whose handlers (`handle_group_key`,
    /// `handle_group_modifiers`) borrow this same group afresh through
    /// their listener, and `led_update` reads it. No reference to the group
    /// may be live across that.
    ///
    /// # Safety
    /// `this` is a live group and nothing holds a reference to it, to the
    /// window manager or to a `Sys`'s subsystems: a key that reaches the
    /// group keyboard runs `handle_group_key`, an entry point that takes
    /// them all.
    pub unsafe fn process_key(this: *mut Self, event: *const ffi::wlr_keyboard_key_event) {
        let keycode = (*event).keycode;
        let pressed = (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED;
        // Decided with the press map borrowed, which ends before wlroots runs
        // the handler that changes it.
        let notify = if let Some(key) = (*this).pressed.get_mut(&keycode) {
            assert!(key.count > 0);
            if pressed {
                key.count += 1;
                None
            } else {
                key.count -= 1;
                (key.count == 0).then_some(ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED)
            }
        } else if pressed && (*this).pressed.len() < 32 {
            Some(ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED)
        } else {
            None
        };
        if let Some(state) = notify {
            let mut key_event = ffi::wlr_keyboard_key_event {
                time_msec: (*event).time_msec,
                keycode,
                update_state: true,
                state,
            };
            ffi::wlr_keyboard_notify_key(std::ptr::addr_of_mut!((*this).wlr_keyboard), &mut key_event);
        }
    }

    /// # Safety
    /// As `process_key`: this runs `handle_group_modifiers`.
    pub unsafe fn process_modifiers(this: *mut Self, modifiers: ffi::wlr_keyboard_modifiers) {
        ffi::wlr_keyboard_notify_modifiers(
            std::ptr::addr_of_mut!((*this).wlr_keyboard),
            modifiers.depressed,
            modifiers.latched,
            modifiers.locked,
            modifiers.group,
        );
    }

    /// # Safety
    /// As `process_key`: a new keymap updates the modifiers, which runs
    /// `handle_group_modifiers`. `keymap` must be a live `xkb_keymap`
    /// (wlroots takes its own reference).
    pub unsafe fn process_keymap(this: *mut Self, keymap: *mut ffi::xkb_keymap) {
        ffi::wlr_keyboard_set_keymap(std::ptr::addr_of_mut!((*this).wlr_keyboard), keymap);
    }

    /// # Safety
    /// The group's seat's `relay.input_method` must be null or live (its
    /// destroy handler clears it).
    pub unsafe fn get_input_method_grab(&self, wm: &crate::window_manager::WindowManager) -> *mut ffi::wlr_input_method_keyboard_grab_v2 {
        if self.virtual_device {
            return std::ptr::null_mut();
        }
        let Some(seat) = self.sid.and_then(|sid| wm.seats.get(sid)) else { return std::ptr::null_mut() };
        let input_method = seat.relay.input_method();
        if !input_method.is_null() {
            return (*input_method).keyboard_grab;
        }
        std::ptr::null_mut()
    }

    /// # Safety
    /// `server` must point to the live `Server`, and the group's
    /// `config.keymap` be null or live.
    pub unsafe fn send_state(&mut self, server: *mut crate::server::Server) {
        let keymap = self.config.keymap;
        if keymap.is_null() {
            return;
        }
        let layout_index = self.wlr_keyboard.modifiers.group;
        let layout_name = ffi::xkb_keymap_layout_get_name(keymap, layout_index);
        let caps_idx = ffi::xkb_keymap_mod_get_index(keymap, c"Caps Lock".as_ptr());
        let capslock = if caps_idx != ffi::XKB_MOD_INVALID {
            let caps_mask = 1 << caps_idx;
            (self.wlr_keyboard.modifiers.locked & caps_mask) != 0
        } else {
            false
        };
        let num_idx = ffi::xkb_keymap_mod_get_index(keymap, c"Num Lock".as_ptr());
        let numlock = if num_idx != ffi::XKB_MOD_INVALID {
            let num_mask = 1 << num_idx;
            (self.wlr_keyboard.modifiers.locked & num_mask) != 0
        } else {
            false
        };

        for xkb_kbd in (*server).xkb_config.keyboards() {
            let parent_dev = (*xkb_kbd).parent_device;
            let kbd = (*parent_dev).keyboard();
            if !kbd.is_null() && std::ptr::eq((*kbd).group, self) {
                (*xkb_kbd).send_state(layout_index, layout_name, capslock, numlock);
            }
        }
    }
}

fn is_vt_switch(keysym: u32) -> bool {
    (ffi::XKB_KEY_XF86Switch_VT_1..=ffi::XKB_KEY_XF86Switch_VT_12).contains(&keysym)
}

/// The keys that still act while the session is locked. Everything else —
/// every config keybind (close window, spawn, reload), portal shortcut,
/// client-registered xkb binding and input-method grab — goes to the lock
/// screen instead. Until 2026-10-01 nothing here asked about the lock, so at
/// the lock screen super+q closed the focused window behind it and any
/// `spawn` bind ran. What stays: switching VTs (a builtin), and a config
/// keybind on a volume, brightness or media key, the controls a locked
/// laptop still needs — the same set other compositors bind `--locked`.
fn allowed_while_locked(keysym: u32) -> bool {
    is_vt_switch(keysym)
        || matches!(
            keysym,
            ffi::XKB_KEY_XF86AudioRaiseVolume
                | ffi::XKB_KEY_XF86AudioLowerVolume
                | ffi::XKB_KEY_XF86AudioMute
                | ffi::XKB_KEY_XF86AudioMicMute
                | ffi::XKB_KEY_XF86AudioPlay
                | ffi::XKB_KEY_XF86AudioPause
                | ffi::XKB_KEY_XF86AudioStop
                | ffi::XKB_KEY_XF86AudioNext
                | ffi::XKB_KEY_XF86AudioPrev
                | ffi::XKB_KEY_XF86MonBrightnessUp
                | ffi::XKB_KEY_XF86MonBrightnessDown
                | ffi::XKB_KEY_XF86KbdBrightnessUp
                | ffi::XKB_KEY_XF86KbdBrightnessDown
        )
}

fn handle_builtin_binding(wm: &mut crate::window_manager::WindowManager, keysym: u32, modifiers: u32) -> bool {
    match keysym {
        ffi::XKB_KEY_XF86Switch_VT_1..=ffi::XKB_KEY_XF86Switch_VT_12 => {
            log::debug!("switch VT keysym received");
            let session = &crate::shared::handles().session;
            if !session.is_null() {
                let vt = keysym - ffi::XKB_KEY_XF86Switch_VT_1 + 1;
                log::info!("switching to VT {}", vt);
                session.change_vt(vt);
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
            if !wm.any_expanded_status_segment(None) {
                return false;
            }
            log::debug!("Escape dismisses the open status menu");
            if let Some(ref sender) = wm.status_sender {
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

    for keyboard in (*group).keyboards() {
        ffi::wlr_keyboard_led_update((*keyboard).wlr_keyboard, leds);
    }
}

unsafe extern "C" fn handle_group_key(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let group = &mut *crate::listener_owner!(listener, KeyboardGroup, key_listener);
    let event = data as *mut ffi::wlr_keyboard_key_event;

    let xkb_state = group.wlr_keyboard.xkb_state;
    if xkb_state.is_null() {
        log::error!("no xkb_state available");
        return;
    }
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());

    // Keys are activity for the idle timeouts (and the idle-notify clients)
    // exactly as pointer events are; before 2026-09-16 only tablet, touch
    // and gestures counted, so idle-notify clients never saw a key.
    let sid = group.sid;
    if let Some(sid) = sid {
        crate::seat::Seat::handle_activity(wm, sid, sys);
    }

    // A real key press ends an emulated view drag (see `cursor::ViewDrag`):
    // the drag holds Space and a pointer button down on the client's behalf,
    // and a key pressed on top of that reaches the app as a chord nobody
    // asked for. The drag's own synthetic Space goes straight to the seat and
    // never passes through here.
    if let (Some(sid), true) = (sid, (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED) {
        if wm.seats.get(sid).is_some_and(|seat| seat.cursor.view_drag.is_some()) {
            crate::cursor::Cursor::end_view_drag(wm, sid, "key");
        }
        if let Some(seat) = wm.seats.get_mut(sid) {
            if seat.cursor.popup_wheel.is_some() {
                seat.cursor.end_popup_wheel("key");
            }
            if seat.cursor.hscroll_shift.is_some() {
                seat.cursor.end_hscroll_shift("key");
            }
        }
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
        // While the session is locked (or locking) a key is the lock
        // screen's: see `allowed_while_locked` for the few that still act.
        let locked = crate::shared::lock_state() != crate::lock_manager::LockState::Unlocked;
        
        let mut matched_builtin = false;
        let mut syms_ptr: *const ffi::xkb_keysym_t = std::ptr::null();
        let num_syms = ffi::xkb_state_key_get_syms(xkb_state, xkb_keycode, &mut syms_ptr);
        log::debug!("handle_group_key keycode={}, xkb_keycode={}, num_syms={}", (*event).keycode, xkb_keycode, num_syms);
        if num_syms > 0 && !syms_ptr.is_null() {
            let syms = std::slice::from_raw_parts(syms_ptr, num_syms as usize);
            for &sym in syms {
                log::debug!("  keysym={:#x}", sym);
                if locked && !is_vt_switch(sym) {
                    continue;
                }
                if handle_builtin_binding(wm, sym, modifiers) {
                    matched_builtin = true;
                    break;
                }
            }
        }

        if matched_builtin {
            KeyConsumer::Builtin
        } else if let Some(kb) = match_cce_keybind(&*wm, xkb_keycode, modifiers, group)
            .filter(|kb| !locked || allowed_while_locked(kb.keysym))
        {
            log::debug!("matched CCE monolithic keybind: {:?}", kb);
            KeyConsumer::CceBinding(kb)
        } else if let Some((session, id)) = (!locked)
            .then(|| match_portal_shortcut(&*wm, xkb_keycode, modifiers, group))
            .flatten()
        {
            log::debug!("matched portal shortcut {} {}", session, id);
            KeyConsumer::PortalShortcut { session, id }
        } else if !locked && !group.get_input_method_grab(wm).is_null() {
            // Never while locked: an input method would read the password.
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
        KeyConsumer::PortalShortcut { session, id } => {
            // Both edges go out: the portal has a Deactivated signal, and
            // the release comes back here through the consumer map with the
            // same variant the press recorded.
            let pressed = (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED;
            if let Some(ref sender) = wm.status_sender {
                sender.send_shortcut_event(
                    &cce_core::ipc::ctl::ShortcutEvent {
                        activated: pressed,
                        session: session.to_string(),
                        id: id.to_string(),
                        time_msec: (*event).time_msec,
                    }
                    .to_string(),
                );
            }
        }
        KeyConsumer::ImGrab => {
            let grab = group.get_input_method_grab(wm);
            if !grab.is_null() {
                ffi::wlr_input_method_keyboard_grab_v2_set_keyboard(grab, &mut group.wlr_keyboard);
                ffi::wlr_input_method_keyboard_grab_v2_send_key(grab, (*event).time_msec, (*event).keycode, (*event).state);
            }
        }
        KeyConsumer::Focus => {
            // Overlay AND Popup: desktop chrome (docks, the cce-cloud
            // launcher) stays keyboard-interactive in overview — only world
            // windows (spatial thumbnails) have their presses eaten.
            let seat = sid.and_then(|sid| wm.seats.get(sid));
            let is_overlay_mode = seat.is_some_and(|seat| seat.focus_is_chrome(wm));

            if let Some(seat) = seat.filter(|_| {
                crate::shared::mode() != crate::window_manager::WindowManagerMode::Overview
                    || is_overlay_mode
                    || (*event).state == ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED
            }) {
                ffi::wlr_seat_set_keyboard(seat.wlr_seat().ptr(), &mut group.wlr_keyboard);
                seat.wlr_seat().keyboard_notify_key((*event).time_msec, (*event).keycode, (*event).state);
            }
        }
    }

    group.send_state(crate::shared::server_ptr());
}

pub fn match_cce_keybind(
    wm: &crate::window_manager::WindowManager,
    keycode: u32,
    modifiers: u32,
    group: &KeyboardGroup,
) -> Option<crate::config::Keybind> {
    match_chord(keycode, modifiers, group, |mods, sym| {
        wm.keybinds.iter().find(|kb| kb.mods == mods && kb.keysym == sym).cloned()
    })
}

/// The portal-bound chords (`global_shortcuts`), matched exactly like the
/// config keybinds but consulted after them. Returns `(session, id)`.
pub fn match_portal_shortcut(
    wm: &crate::window_manager::WindowManager,
    keycode: u32,
    modifiers: u32,
    group: &KeyboardGroup,
) -> Option<(String, String)> {
    if wm.portal_shortcuts.is_empty() {
        return None;
    }
    match_chord(keycode, modifiers, group, |mods, sym| {
        wm.portal_shortcuts
            .iter()
            .find(|s| s.mods == mods && s.keysym == sym)
            .map(|s| (s.session.clone(), s.id.clone()))
    })
}

/// Chord lookup shared by every (mods, keysym) table. `probe` is asked
/// twice over: first with the keycode's level-0 keysyms against the raw
/// modifier mask (so `super+shift+h` matches on `h`, not `H`), then with the
/// keysyms of the level the modifiers actually select against the mask with
/// the consumed modifiers removed (so a bind on a shifted symbol like
/// `plus` still fires). The first hit wins.
fn match_chord<T>(
    keycode: u32,
    modifiers: u32,
    group: &KeyboardGroup,
    probe: impl Fn(u32, u32) -> Option<T>,
) -> Option<T> {
    let xkb_state = group.wlr_keyboard.xkb_state;
    if xkb_state.is_null() {
        return None;
    }
    // SAFETY: a non-null `xkb_state` is the live state wlroots made for the
    // group keyboard's keymap; only `wlr_keyboard_set_keymap` replaces it,
    // which needs the group mutably, and it is borrowed here. The keysym
    // slices live as long as that state's keymap, and `probe` is only handed
    // their values.
    unsafe {
        let keymap = ffi::xkb_state_get_keymap(xkb_state);
        if keymap.is_null() {
            return None;
        }
        let layout = ffi::xkb_state_key_get_layout(xkb_state, keycode);

        let mut syms_ptr: *const ffi::xkb_keysym_t = std::ptr::null();
        let num_syms = ffi::xkb_keymap_key_get_syms_by_level(keymap, keycode, layout, 0, &mut syms_ptr);
        if num_syms > 0 && !syms_ptr.is_null() {
            let syms = std::slice::from_raw_parts(syms_ptr, num_syms as usize);
            for &sym in syms {
                if let Some(hit) = probe(modifiers, sym) {
                    return Some(hit);
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
            for &sym in syms {
                if let Some(hit) = probe(modifiers_translated, sym) {
                    return Some(hit);
                }
            }
        }
    }

    None
}

unsafe extern "C" fn handle_group_modifiers(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let group = &mut *crate::listener_owner!(listener, KeyboardGroup, modifiers_listener);
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());

    group.modifiers_old = ffi::wlr_keyboard_get_modifiers(&mut group.wlr_keyboard);

    let grab = group.get_input_method_grab(wm);
    if !grab.is_null() {
        ffi::wlr_input_method_keyboard_grab_v2_set_keyboard(grab, &mut group.wlr_keyboard);
        ffi::wlr_input_method_keyboard_grab_v2_send_modifiers(grab, &mut group.wlr_keyboard.modifiers);
    } else if let Some(seat) = group.sid.and_then(|sid| wm.seats.get(sid)) {
        ffi::wlr_seat_set_keyboard(seat.wlr_seat().ptr(), &mut group.wlr_keyboard);
        seat.wlr_seat().keyboard_notify_modifiers(&group.wlr_keyboard.modifiers);
    }
    // Window-adjust mode (Super held) reads the seat keyboard's mask, which
    // is this group's — the DEVICE keyboard has no keymap on the DRM
    // backend (see keyboard::should_set_keymap), so its own modifiers
    // signal never fires there; this one does for every real key.
    wm.refresh_adjust_held();

    group.send_state(crate::shared::server_ptr());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locked_session_keeps_only_vt_switching_and_the_media_keys() {
        for sym in [
            ffi::XKB_KEY_XF86Switch_VT_1,
            ffi::XKB_KEY_XF86Switch_VT_12,
            ffi::XKB_KEY_XF86AudioRaiseVolume,
            ffi::XKB_KEY_XF86AudioMute,
            ffi::XKB_KEY_XF86MonBrightnessDown,
        ] {
            assert!(allowed_while_locked(sym), "{sym:#x} should act while locked");
        }
        // The keys a bind like super+q or super+d is made of, and media-range
        // keys that launch things.
        for sym in [
            ffi::XKB_KEY_q,
            ffi::XKB_KEY_d,
            ffi::XKB_KEY_Return,
            ffi::XKB_KEY_Escape,
            ffi::XKB_KEY_XF86Calculator,
            ffi::XKB_KEY_XF86WWW,
            ffi::XKB_KEY_XF86PowerOff,
        ] {
            assert!(!allowed_while_locked(sym), "{sym:#x} must go to the lock screen");
        }
    }
}
