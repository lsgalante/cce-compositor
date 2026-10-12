// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::Server;
use crate::seat::Seat;

pub struct InputManager {
    /// The "default" seat, by id (look it up in `wm.seats`).
    pub default_seat: Option<crate::seat::SeatId>,
    /// Private: a registry another module could replace or swap would stop
    /// matching what the devices' destroy paths remove from it.
    devices: crate::registry::Registry<crate::input_device::InputDevice>,
    /// Its protocol objects (one per bound client). Private: walks trust
    /// that a registered object is live (its destroy handler unregisters it).
    objects: crate::registry::Registry<InputManagerObject>,
    pub global: *mut ffi::wl_global,
    
    pub idle_notifier: *mut ffi::wlr_idle_notifier_v1,
    pub relative_pointer_manager: *mut ffi::wlr_relative_pointer_manager_v1,
    pub pointer_gestures: *mut ffi::wlr_pointer_gestures_v1,
    pub virtual_pointer_manager: *mut ffi::wlr_virtual_pointer_manager_v1,
    pub virtual_keyboard_manager: *mut ffi::wlr_virtual_keyboard_manager_v1,
    pub pointer_constraints: *mut ffi::wlr_pointer_constraints_v1,
    pub input_method_manager: *mut ffi::wlr_input_method_manager_v2,
    pub text_input_manager: *mut ffi::wlr_text_input_manager_v3,
    pub tablet_manager: *mut ffi::wlr_tablet_manager_v2,

    new_input_listener: crate::listener::Listener,
    new_text_input: crate::listener::Listener,
    new_input_method: crate::listener::Listener,
    new_virtual_pointer_listener: crate::listener::Listener,
    new_virtual_keyboard_listener: crate::listener::Listener,
    new_pointer_constraint: crate::listener::Listener,

}

impl InputManager {
    /// Its protocol objects, copied out.
    pub(crate) fn objects(&self) -> Vec<*mut InputManagerObject> {
        self.objects.ptrs()
    }
}

// SAFETY: a field of the one `Server`, which only the unsafe `Server::zeroed`
// makes and `run_server` never frees. It has no safe constructor, so safe code
// holding the one `&mut` to it has nothing to swap it with or replace it by.
unsafe impl crate::listener::Anchored for InputManager {}

thread_local! {
    /// The pending deferred pointer-focus re-evaluation (see
    /// [`InputManager::schedule_pointer_refresh`]): one idle source
    /// coalesces every scene-mapping change of a dispatch. Module-level,
    /// not an `InputManager` field: its callback must forget the very
    /// `IdleOnce` that scheduled it (`IdleOnce::schedule`'s contract), and
    /// as a field that held only for the server's own input manager while
    /// any other one (a safe `Default`) could schedule too (fourth review,
    /// 2026-10-12).
    static POINTER_REFRESH: crate::event_source::IdleOnce = const { crate::event_source::IdleOnce::new() };
}

pub struct InputManagerObject {
    pub manager: *mut InputManager,
    pub resource: *mut ffi::wl_resource,
}

impl InputManager {
    /// Write empty registries into a zero-filled manager
    /// (`Server::zeroed`), whose registry bytes are not yet valid `Vec`s.
    ///
    /// # Safety
    /// `this` points at an `InputManager` whose `devices` and `objects`
    /// have not been initialised; they are overwritten without being
    /// dropped.
    pub unsafe fn init_registry_in_place(this: *mut Self) {
        std::ptr::write(std::ptr::addr_of_mut!((*this).devices), crate::registry::Registry::new());
        std::ptr::write(std::ptr::addr_of_mut!((*this).objects), crate::registry::Registry::new());
    }

    /// Every registered input device, copied out (a walk may destroy one).
    pub fn devices(&self) -> Vec<*mut crate::input_device::InputDevice> {
        self.devices.ptrs()
    }

    /// Register a new device, at the front (as the list's head inserts did).
    ///
    /// # Safety
    /// As `Registry::push`: `device` is live until `unregister_device`,
    /// which its destroy path runs before freeing it.
    pub unsafe fn register_device(&mut self, device: *mut crate::input_device::InputDevice) {
        self.devices.push_front(device);
    }

    pub fn unregister_device(&mut self, device: *mut crate::input_device::InputDevice) {
        self.devices.remove(device);
    }

    /// Take every device off seat `sid`, which a client is destroying
    /// (`river_input_manager_v1.destroy_seat`): the manage pass's
    /// `Seat::destroy` asserts the seat's keyboard groups empty (a client
    /// holding a virtual keyboard on a seat it then destroyed crashed the
    /// compositor there). The protocol gives a destroyed seat's devices to
    /// the default seat; a virtual device is only detached, since the one
    /// kind that can sit on another seat — a virtual keyboard bound to it —
    /// dies with that seat (wlroots' `seat_destroy` listener) and would only
    /// flash its keymap at the default seat's focused client on the way
    /// out. From then on the seat is gone to every attach
    /// (`Seat::destroying`).
    ///
    /// # Safety
    /// As `register_device`: every registered device is live.
    pub unsafe fn release_seat(&mut self, wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, sys: &mut crate::ctx::Sys) {
        if self.default_seat == Some(sid) {
            return;
        }
        for device in self.devices.ptrs() {
            if (*device).sid == Some(sid) {
                let to = if (*device).virtual_device { None } else { self.default_seat };
                (*device).assign_to_seat(wm, to, sys);
            }
        }
    }

    /// Drop every input mapping onto `wlr_output`, which is being destroyed
    /// — the device's own (`map_to_output`, which tablet tools re-apply on
    /// every event) and its seat cursor's, which wlroots never clears and
    /// would read on the next absolute motion or tablet event.
    ///
    /// # Safety
    /// `wlr_output` is still live (its destroy handler, before it lets go).
    pub unsafe fn forget_output(&mut self, wm: &crate::window_manager::WindowManager, wlr_output: *mut ffi::wlr_output) {
        if wlr_output.is_null() {
            return;
        }
        for dev in self.devices.ptrs() {
            if (*dev).config.map_to_output != wlr_output {
                continue;
            }
            (*dev).config.map_to_output = std::ptr::null_mut();
            if let Some(seat) = (*dev).sid.and_then(|sid| wm.seats.get(sid)) {
                ffi::wlr_cursor_map_input_to_output(seat.cursor.wlr_cursor().ptr(), (*dev).wlr_device, std::ptr::null_mut());
            }
        }
    }

    pub unsafe fn init(&mut self, server: *mut Server, seats: &mut crate::seat::Seats) -> Result<(), &'static str> {
        let wl_server = (*server).wl_server;

        self.idle_notifier = ffi::wlr_idle_notifier_v1_create(wl_server);
        crate::shared::handles().idle_notifier.set(self.idle_notifier);
        self.relative_pointer_manager = ffi::wlr_relative_pointer_manager_v1_create(wl_server);
        crate::shared::handles().relative_pointer.set(self.relative_pointer_manager);
        self.pointer_gestures = ffi::wlr_pointer_gestures_v1_create(wl_server);
        crate::shared::handles().pointer_gestures.set(self.pointer_gestures);
        self.virtual_pointer_manager = ffi::wlr_virtual_pointer_manager_v1_create(wl_server);
        self.virtual_keyboard_manager = ffi::wlr_virtual_keyboard_manager_v1_create(wl_server);
        self.pointer_constraints = ffi::wlr_pointer_constraints_v1_create(wl_server);
        crate::shared::handles().pointer_constraints.set(self.pointer_constraints);
        self.input_method_manager = ffi::wlr_input_method_manager_v2_create(wl_server);
        self.text_input_manager = ffi::wlr_text_input_manager_v3_create(wl_server);
        self.tablet_manager = ffi::wlr_tablet_v2_create(wl_server);
        crate::shared::handles().tablet_manager.set(self.tablet_manager);

        // Create default seat
        let default_seat = Seat::create(server, seats, "default")?;
        self.default_seat = Some((*default_seat).id);
        crate::shared::handles().default_seat.set(self.default_seat);

        if !(*server).xwayland.is_null() {
            ffi::wlr_xwayland_set_seat((*server).xwayland, (*default_seat).wlr_seat().ptr());
        }

        self.global = ffi::wl_global_create(
            wl_server,
            &ffi::river_input_manager_v1_interface,
            2,
            self as *mut InputManager as *mut _,
            Some(bind_input_manager),
        );
        if self.global.is_null() {
            return Err("Failed to create river_input_manager_v1 global");
        }

        
        let signal = ffi::river_wlr_backend_get_new_input_signal((*server).backend);
        self.new_input_listener.connect(signal, handle_new_input);

        // Pointer lock / confine (zwp_pointer_constraints_v1).
        if !self.pointer_constraints.is_null() {
            self.new_pointer_constraint.connect(
                ffi::river_wlr_pointer_constraints_get_new_constraint_signal(self.pointer_constraints),
                handle_new_pointer_constraint,
            );
        }

        // Connect new_text_input listener
        self.new_text_input.connect(&mut (*self.text_input_manager).events.new_text_input, handle_new_text_input);

        // Connect new_input_method listener
        self.new_input_method.connect(&mut (*self.input_method_manager).events.new_input_method, handle_new_input_method);

        // Connect new_virtual_pointer listener
        self.new_virtual_pointer_listener.connect(&mut (*self.virtual_pointer_manager).events.new_virtual_pointer, handle_new_virtual_pointer);

        // Connect new_virtual_keyboard listener
        self.new_virtual_keyboard_listener.connect(&mut (*self.virtual_keyboard_manager).events.new_virtual_keyboard, handle_new_virtual_keyboard);

        Ok(())
    }

    /// Schedule a pointer-focus re-evaluation for every seat, deferred to an
    /// idle callback and coalesced (one source no matter how many commits
    /// land in a dispatch). Used when a commit changes the surface↔frame
    /// mapping under a stationary cursor — geometry re-anchor or surface
    /// extent change. Deferred, NOT inline: wlroots' own scene commit
    /// listeners re-anchor the surface tree AFTER the compositor's toplevel
    /// commit handler runs, so an inline refresh would query the stale
    /// mapping.
    pub fn schedule_pointer_refresh(&self) {
        // SAFETY: `handle_pointer_refresh_idle` is an entry point that needs
        // no data, and calls `ran()` on this same `POINTER_REFRESH` first.
        let scheduled = POINTER_REFRESH.with(|r| unsafe { r.schedule(handle_pointer_refresh_idle, 0) });
        if !scheduled {
            log::error!("Failed to schedule pointer-refresh idle source");
        }
    }

    pub unsafe fn deinit(&mut self, wm: &mut crate::window_manager::WindowManager, sys: &mut crate::ctx::Sys) {
        log::info!("[deinit] InputManager::deinit started");
        POINTER_REFRESH.with(|r| r.cancel());
        if !self.global.is_null() {
            log::info!("[deinit] destroying input manager global");
            ffi::wl_global_destroy(self.global);
            self.global = std::ptr::null_mut();
        }

        for obj in self.objects.ptrs() {
            // Its destroy handler unregisters and frees it.
            ffi::wl_resource_destroy((*obj).resource);
        }

        // Detach all devices from their seats and set their seat pointer to null
        // so they do not attempt to access a freed seat during backend destruction.
        log::info!("[deinit] detaching devices");
        for device in self.devices.ptrs() {
            if let Some(sid) = (*device).sid {
                log::info!("[deinit] detaching device from seat");
                crate::seat::Seat::detach_device(wm, sid, device, sys);
                (*device).sid = None;
            }
        }

        // Destroy all seats
        log::info!("[deinit] destroying seats");
        for id in wm.seats.ids() {
            if let Some(seat) = wm.seats.get(id) {
                log::info!("[deinit] calling Seat::destroy for {:?}", seat.wlr_seat().ptr());
                Seat::destroy(wm, id);
            }
        }
        self.default_seat = None;
        crate::shared::handles().default_seat.set(None);
        log::info!("[deinit] seats destroyed");

        log::info!("[deinit] removing input manager listeners");
        self.new_input_listener.disconnect();
        self.new_text_input.disconnect();
        self.new_input_method.disconnect();
        self.new_virtual_pointer_listener.disconnect();
        self.new_virtual_keyboard_listener.disconnect();
        self.new_pointer_constraint.disconnect();
        log::info!("[deinit] InputManager::deinit finished");
    }
}

unsafe extern "C" fn bind_input_manager(
    client: *mut ffi::wl_client,
    data: *mut std::ffi::c_void,
    version: u32,
    id: u32,
) {
    let im = data as *mut InputManager;
    let resource = ffi::wl_resource_create(client, &ffi::river_input_manager_v1_interface, version as i32, id);
    if resource.is_null() {
        ffi::wl_client_post_no_memory(client);
        return;
    }

    let obj = Box::into_raw(Box::new(InputManagerObject {
        manager: im,
        resource,
    }));

    ffi::wl_resource_set_implementation(
        resource,
        &INPUT_MANAGER_INTERFACE as *const _ as *const _,
        obj as *mut _,
        Some(handle_manager_object_destroy),
    );

    // Unregistered by its destroy handler before it is freed.
    (*im).objects.push(obj);

    // Send existing devices to the client
    for device in (*im).devices.ptrs() {
        if !(*device).virtual_device {
            (*device).create_object(crate::shared::server_ptr(), resource);
        }
    }
}

unsafe extern "C" fn handle_manager_object_destroy(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputManagerObject;
    if !obj.is_null() {
        // The manager is a server field, live for the server's life.
        (*(*obj).manager).objects.remove(obj);
        let _ = Box::from_raw(obj);
    }
}

static INPUT_MANAGER_INTERFACE: ffi::river_input_manager_v1_interface = ffi::river_input_manager_v1_interface {
    stop: Some(input_manager_stop),
    destroy: Some(input_manager_destroy),
    create_seat: Some(input_manager_create_seat),
    destroy_seat: Some(input_manager_destroy_seat),
};

// Every request has a handler: libwayland aborts the whole compositor on a
// null entry, and a client can send any request on an inert object (one it
// sent before it saw the event that made the object inert, or a repeat).
static INPUT_MANAGER_INERT_INTERFACE: ffi::river_input_manager_v1_interface = ffi::river_input_manager_v1_interface {
    stop: Some(input_manager_inert_stop),
    destroy: Some(input_manager_inert_destroy),
    create_seat: Some(input_manager_inert_seat_request),
    destroy_seat: Some(input_manager_inert_seat_request),
};

unsafe extern "C" fn input_manager_inert_stop(_client: *mut ffi::wl_client, _resource: *mut ffi::wl_resource) {}

unsafe extern "C" fn input_manager_inert_seat_request(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _name: *const std::os::raw::c_char,
) {
}

unsafe extern "C" fn input_manager_stop(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputManagerObject;
    if obj.is_null() {
        return;
    }

    (*(*obj).manager).objects.remove(obj);

    ffi::wl_resource_post_event(resource, 0); // finished event

    ffi::wl_resource_set_implementation(
        resource,
        &INPUT_MANAGER_INERT_INTERFACE as *const _ as *const _,
        obj as *mut _,
        Some(handle_manager_object_destroy),
    );
}

unsafe extern "C" fn input_manager_destroy(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    ffi::wl_resource_post_error(
        resource,
        ffi::river_input_manager_v1_error_RIVER_INPUT_MANAGER_V1_ERROR_INVALID_DESTROY,
        b"destroy before finished event sent\0".as_ptr() as *const _,
    );
}

unsafe extern "C" fn input_manager_inert_destroy(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn input_manager_create_seat(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    name: *const std::os::raw::c_char,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputManagerObject;
    if obj.is_null() || name.is_null() {
        return;
    }
    let name_str = std::ffi::CStr::from_ptr(name).to_string_lossy();

    // Check if seat already exists
    let exists = (*crate::reentry::wm(crate::shared::server_ptr())).seats.values().any(|seat| {
        std::ffi::CStr::from_ptr(ffi::river_wlr_seat_get_name(seat.wlr_seat().ptr())).to_string_lossy() == name_str
    });

    if !exists {
        let server = crate::shared::server_ptr();
        if let Err(e) = Seat::create(server, &mut (*crate::reentry::wm(server)).seats, &name_str) {
            log::error!("failed to create seat '{}': {}", name_str, e);
            ffi::wl_resource_post_no_memory(resource);
        }
    }
}

unsafe extern "C" fn input_manager_destroy_seat(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    name: *const std::os::raw::c_char,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputManagerObject;
    if obj.is_null() || name.is_null() {
        return;
    }
    let name_str = std::ffi::CStr::from_ptr(name).to_string_lossy();

    if name_str == "default" {
        return;
    }

    let server = crate::shared::server_ptr();
    let sys = &mut crate::ctx::Sys::of(server);
    let wm = &mut *crate::reentry::wm(server);
    let input = &mut *std::ptr::addr_of_mut!((*server).input_manager);
    // Skip default seat (which is the first seat in the list); a seat
    // already on its way out is gone to a request too.
    let Some(sid) = wm.seats.ids().into_iter().skip(1).find(|&sid| {
        wm.seats.get(sid).is_some_and(|seat| !seat.destroying && seat.wlr_seat().name() == name_str)
    }) else {
        return;
    };
    // Its devices leave it now, from this entry point, rather than in the
    // manage pass that destroys it: `wlr_seat_destroy` there destroys a
    // virtual keyboard bound to the seat, whose destroy handler reaches the
    // input manager, which the pass must not hold borrowed across it.
    input.release_seat(wm, sid, sys);
    if let Some(seat) = wm.seats.get_mut(sid) {
        seat.destroying = true;
    }
    crate::shared::pending().dirty_windowing();
}

unsafe extern "C" fn handle_new_input(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let im = &mut *crate::listener_owner!(listener, InputManager, new_input_listener);
    let wlr_device = data as *mut ffi::wlr_input_device;

    log::info!("new input device connected");

    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    let default_seat = im.default_seat.and_then(|sid| wm.seats.ptr(sid)).unwrap_or(std::ptr::null_mut());
    let device = crate::input_device::InputDevice::new(crate::shared::server_ptr(), &wm.outputs, default_seat, wlr_device, false);
    
    let name_ptr = ffi::river_wlr_input_device_get_name(wlr_device);
    if !name_ptr.is_null() {
        let name = std::ffi::CStr::from_ptr(name_ptr).to_string_lossy();
        for rule in &wm.input_rules {
            if rule.name == "*" || name.contains(&rule.name) {
                if let Some(factor) = rule.scroll_factor {
                    (*device).config.scroll_factor = factor;
                }
            }
        }
    }
    
    let dev_type = ffi::river_wlr_input_device_get_type(wlr_device);
    if dev_type == ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD {
        crate::keyboard::Keyboard::create(device);
    } else if dev_type == ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TABLET {
        let _ = crate::tablet::Tablet::create(crate::shared::server_ptr(), &wm.outputs, default_seat, wlr_device, false);
    }

    // Attach device to the default seat
    if let Some(seat) = im.default_seat.and_then(|sid| wm.seats.get_mut(sid)) {
        seat.attach_device(device);
    }
}

unsafe extern "C" fn handle_new_pointer_constraint(_listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    crate::pointer_constraint::PointerConstraint::create(wm, data as *mut ffi::wlr_pointer_constraint_v1);
}

unsafe extern "C" fn handle_new_text_input(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let _im = crate::listener_owner!(listener, InputManager, new_text_input);
    let wlr_text_input = data as *mut ffi::wlr_text_input_v3;

    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    if let Err(e) = crate::text_input::TextInput::create(wm, wlr_text_input) {
        log::error!("failed to create text input: {}", e);
        ffi::wl_resource_post_no_memory((*wlr_text_input).resource);
    }
}

unsafe extern "C" fn handle_new_input_method(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let im = &mut *crate::listener_owner!(listener, InputManager, new_input_method);
    let input_method = data as *mut ffi::wlr_input_method_v2;
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    // The seat holding the input method's `wlr_seat`, else the default seat.
    let Some(sid) = wm.seats.find((*input_method).seat).or(im.default_seat) else { return };
    crate::input_relay::InputRelay::new_input_method(wm, sid, input_method);
}

unsafe extern "C" fn handle_new_virtual_pointer(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let im = &mut *crate::listener_owner!(listener, InputManager, new_virtual_pointer_listener);
    let event = data as *mut ffi::wlr_virtual_pointer_v1_new_pointer_event;

    log::info!("new virtual pointer device connected");

    let virtual_pointer = (*event).new_pointer;
    let wlr_device = &mut (*virtual_pointer).pointer.base as *mut ffi::wlr_input_device;

    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    let default_seat = im.default_seat.and_then(|sid| wm.seats.ptr(sid)).unwrap_or(std::ptr::null_mut());
    let device = crate::input_device::InputDevice::new(crate::shared::server_ptr(), &wm.outputs, default_seat, wlr_device, true);

    // Attach device to the default seat
    if let Some(seat) = im.default_seat.and_then(|sid| wm.seats.get_mut(sid)) {
        seat.attach_device(device);
    }
}

unsafe extern "C" fn handle_new_virtual_keyboard(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let im = &mut *crate::listener_owner!(listener, InputManager, new_virtual_keyboard_listener);
    let virtual_keyboard = data as *mut ffi::wlr_virtual_keyboard_v1;

    log::info!("new virtual keyboard device connected");

    let wlr_device = &mut (*virtual_keyboard).keyboard.base as *mut ffi::wlr_input_device;

    // Honor the seat the client bound the virtual keyboard to, like input
    // methods do; fall back to the default seat if no seat holds it.
    let wm = &*crate::reentry::wm(crate::shared::server_ptr());
    let bound = wm.seats.find((*virtual_keyboard).seat);
    let seat = bound.or(im.default_seat).and_then(|sid| wm.seats.ptr(sid)).unwrap_or(std::ptr::null_mut());
    if seat.is_null() {
        return;
    }

    // Bound to a seat a client is destroying: wlroots destroys the keyboard
    // with that seat, so it joins none (`InputManager::release_seat`).
    if (*seat).destroying {
        let device = crate::input_device::InputDevice::new(crate::shared::server_ptr(), &wm.outputs, std::ptr::null_mut(), wlr_device, true);
        crate::keyboard::Keyboard::create(device);
        return;
    }

    let device = crate::input_device::InputDevice::new(crate::shared::server_ptr(), &wm.outputs, seat, wlr_device, true);
    crate::keyboard::Keyboard::create(device);

    // Same path a hardware keyboard takes: attach_device puts the keyboard in
    // a (private, virtual) KeyboardGroup, so compositor keybindings apply.
    (*seat).attach_device(device);
}

unsafe extern "C" fn handle_pointer_refresh_idle(_data: *mut std::ffi::c_void) {
    POINTER_REFRESH.with(|r| r.ran());
    let server = crate::shared::server_ptr();
    if server.is_null() {
        return;
    }
    let wm = &mut *crate::reentry::wm(server);
    for sid in wm.seats.ids() {
        crate::cursor::Cursor::refresh_after_scene_change(wm, sid);
    }
}
