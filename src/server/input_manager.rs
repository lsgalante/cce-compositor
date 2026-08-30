// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, wl_signal_add, WlListener, wl_listener_remove, WlList, wl_list_insert, wl_list_remove};
use crate::seat::Seat;

pub struct InputManager {
    pub server: *mut Server,
    pub default_seat: *mut Seat,
    pub seats: ffi::wl_list,
    pub devices: ffi::wl_list, // list of InputDevice
    pub objects: ffi::wl_list, // list of InputManagerObject
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

    pub new_input_listener: ffi::wl_listener,
    pub new_text_input: ffi::wl_listener,
    pub new_input_method: ffi::wl_listener,
    pub new_virtual_pointer_listener: ffi::wl_listener,
    pub new_virtual_keyboard_listener: ffi::wl_listener,

    /// Pending deferred pointer-focus re-evaluation (see
    /// [`InputManager::schedule_pointer_refresh`]); null when none. One idle
    /// source coalesces every scene-mapping change of a dispatch.
    pub pointer_refresh_idle: *mut ffi::wl_event_source,
}

pub struct InputManagerObject {
    pub manager: *mut InputManager,
    pub resource: *mut ffi::wl_resource,
    pub link: ffi::wl_list,
}

impl Default for InputManager {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl InputManager {
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        let wl_server = (*server).wl_server;

        ffi::wl_list_init(&mut self.seats);
        ffi::wl_list_init(&mut self.devices);
        ffi::wl_list_init(&mut self.objects);

        self.idle_notifier = ffi::wlr_idle_notifier_v1_create(wl_server);
        self.relative_pointer_manager = ffi::wlr_relative_pointer_manager_v1_create(wl_server);
        self.pointer_gestures = ffi::wlr_pointer_gestures_v1_create(wl_server);
        self.virtual_pointer_manager = ffi::wlr_virtual_pointer_manager_v1_create(wl_server);
        self.virtual_keyboard_manager = ffi::wlr_virtual_keyboard_manager_v1_create(wl_server);
        self.pointer_constraints = ffi::wlr_pointer_constraints_v1_create(wl_server);
        self.input_method_manager = ffi::wlr_input_method_manager_v2_create(wl_server);
        self.text_input_manager = ffi::wlr_text_input_manager_v3_create(wl_server);
        self.tablet_manager = ffi::wlr_tablet_v2_create(wl_server);

        // Create default seat
        self.default_seat = Seat::create(server, "default")?;

        if !(*server).xwayland.is_null() {
            ffi::wlr_xwayland_set_seat((*server).xwayland, (*self.default_seat).wlr_seat);
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

        let new_input_ptr = &mut self.new_input_listener as *mut ffi::wl_listener as *mut WlListener;
        (*new_input_ptr).notify = Some(handle_new_input);
        
        let signal = ffi::river_wlr_backend_get_new_input_signal((*server).backend);
        wl_signal_add(signal, &mut self.new_input_listener);

        // Connect new_text_input listener
        let new_text_input_ptr = &mut self.new_text_input as *mut ffi::wl_listener as *mut WlListener;
        (*new_text_input_ptr).notify = Some(handle_new_text_input);
        wl_signal_add(&mut (*self.text_input_manager).events.new_text_input, &mut self.new_text_input);

        // Connect new_input_method listener
        let new_input_method_ptr = &mut self.new_input_method as *mut ffi::wl_listener as *mut WlListener;
        (*new_input_method_ptr).notify = Some(handle_new_input_method);
        wl_signal_add(&mut (*self.input_method_manager).events.new_input_method, &mut self.new_input_method);

        // Connect new_virtual_pointer listener
        let new_virtual_pointer_ptr = &mut self.new_virtual_pointer_listener as *mut ffi::wl_listener as *mut WlListener;
        (*new_virtual_pointer_ptr).notify = Some(handle_new_virtual_pointer);
        wl_signal_add(&mut (*self.virtual_pointer_manager).events.new_virtual_pointer, &mut self.new_virtual_pointer_listener);

        // Connect new_virtual_keyboard listener
        let new_virtual_keyboard_ptr = &mut self.new_virtual_keyboard_listener as *mut ffi::wl_listener as *mut WlListener;
        (*new_virtual_keyboard_ptr).notify = Some(handle_new_virtual_keyboard);
        wl_signal_add(&mut (*self.virtual_keyboard_manager).events.new_virtual_keyboard, &mut self.new_virtual_keyboard_listener);

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
    pub unsafe fn schedule_pointer_refresh(&mut self) {
        if !self.pointer_refresh_idle.is_null() {
            return;
        }
        let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
        let idle = ffi::wl_event_loop_add_idle(
            event_loop,
            Some(handle_pointer_refresh_idle),
            self as *mut InputManager as *mut _,
        );
        if idle.is_null() {
            log::error!("Failed to schedule pointer-refresh idle source");
            return;
        }
        self.pointer_refresh_idle = idle;
    }

    pub unsafe fn deinit(&mut self) {
        log::info!("[deinit] InputManager::deinit started");
        if !self.pointer_refresh_idle.is_null() {
            ffi::wl_event_source_remove(self.pointer_refresh_idle);
            self.pointer_refresh_idle = std::ptr::null_mut();
        }
        if !self.global.is_null() {
            log::info!("[deinit] destroying input manager global");
            ffi::wl_global_destroy(self.global);
            self.global = std::ptr::null_mut();
        }

        let objects_head = &mut self.objects as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*objects_head).next;
        while curr != objects_head {
            let next = (*curr).next;
            let obj = crate::container_of!(curr, InputManagerObject, link);
            ffi::wl_resource_destroy((*obj).resource);
            curr = next;
        }

        // Detach all devices from their seats and set their seat pointer to null
        // so they do not attempt to access a freed seat during backend destruction.
        log::info!("[deinit] detaching devices");
        let devices_head = &mut self.devices as *mut ffi::wl_list as *mut WlList;
        let mut curr_dev = (*devices_head).next;
        while curr_dev != devices_head {
            let next_dev = (*curr_dev).next;
            let device = crate::container_of!(curr_dev, crate::input_device::InputDevice, link);
            if !(*device).seat.is_null() {
                log::info!("[deinit] detaching device from seat");
                (*(*device).seat).detach_device(device);
                (*device).seat = std::ptr::null_mut();
            }
            curr_dev = next_dev;
        }

        // Destroy all seats
        log::info!("[deinit] destroying seats");
        let seats_head = &mut self.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_head).next;
        while curr_seat != seats_head {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, Seat, link);
            log::info!("[deinit] calling Seat::destroy for {:?}", (*seat).wlr_seat);
            Seat::destroy(seat);
            curr_seat = next_seat;
        }
        self.default_seat = std::ptr::null_mut();
        log::info!("[deinit] seats destroyed");

        log::info!("[deinit] removing input manager listeners");
        wl_listener_remove(&mut self.new_input_listener);
        wl_listener_remove(&mut self.new_text_input);
        wl_listener_remove(&mut self.new_input_method);
        wl_listener_remove(&mut self.new_virtual_pointer_listener);
        wl_listener_remove(&mut self.new_virtual_keyboard_listener);
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
        link: std::mem::zeroed(),
    }));

    ffi::wl_resource_set_implementation(
        resource,
        &INPUT_MANAGER_INTERFACE as *const _ as *const _,
        obj as *mut _,
        Some(handle_manager_object_destroy),
    );

    let list_head = &mut (*im).objects as *mut ffi::wl_list as *mut WlList;
    wl_list_insert((*list_head).prev, &mut (*obj).link as *mut ffi::wl_list as *mut WlList);

    // Send existing devices to the client
    let devices_head = &mut (*im).devices as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*devices_head).next;
    while curr != devices_head {
        let next = (*curr).next;
        let device = crate::container_of!(curr, crate::input_device::InputDevice, link);
        if !(*device).virtual_device {
            (*device).create_object(resource);
        }
        curr = next;
    }
}

unsafe extern "C" fn handle_manager_object_destroy(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputManagerObject;
    if !obj.is_null() {
        wl_list_remove(&mut (*obj).link as *mut ffi::wl_list as *mut WlList);
        let _ = Box::from_raw(obj);
    }
}

static INPUT_MANAGER_INTERFACE: ffi::river_input_manager_v1_interface = ffi::river_input_manager_v1_interface {
    stop: Some(input_manager_stop),
    destroy: Some(input_manager_destroy),
    create_seat: Some(input_manager_create_seat),
    destroy_seat: Some(input_manager_destroy_seat),
};

static INPUT_MANAGER_INERT_INTERFACE: ffi::river_input_manager_v1_interface = ffi::river_input_manager_v1_interface {
    stop: None,
    destroy: Some(input_manager_inert_destroy),
    create_seat: None,
    destroy_seat: None,
};

unsafe extern "C" fn input_manager_stop(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut InputManagerObject;
    if obj.is_null() {
        return;
    }

    wl_list_remove(&mut (*obj).link as *mut ffi::wl_list as *mut WlList);
    ffi::wl_list_init(&mut (*obj).link);

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
    let im = (*obj).manager;
    let name_str = std::ffi::CStr::from_ptr(name).to_string_lossy();

    // Check if seat already exists
    let seats_head = &mut (*im).seats as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*seats_head).next;
    let mut exists = false;
    while curr != seats_head {
        let next = (*curr).next;
        let seat = crate::container_of!(curr, Seat, link);
        let seat_name = std::ffi::CStr::from_ptr(ffi::river_wlr_seat_get_name((*seat).wlr_seat)).to_string_lossy();
        if seat_name == name_str {
            exists = true;
            break;
        }
        curr = next;
    }

    if !exists {
        let server = (*im).server;
        if let Err(e) = Seat::create(server, &name_str) {
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
    let im = (*obj).manager;
    let name_str = std::ffi::CStr::from_ptr(name).to_string_lossy();

    if name_str == "default" {
        return;
    }

    let seats_head = &mut (*im).seats as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*seats_head).next;
    // Skip default seat (which is the first seat in the list)
    curr = (*curr).next;
    while curr != seats_head {
        let next = (*curr).next;
        let seat = crate::container_of!(curr, Seat, link);
        let seat_name = std::ffi::CStr::from_ptr(ffi::river_wlr_seat_get_name((*seat).wlr_seat)).to_string_lossy();
        if seat_name == name_str {
            (*seat).destroying = true;
            (*(*im).server).wm.dirty_windowing();
            break;
        }
        curr = next;
    }
}

unsafe extern "C" fn handle_new_input(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let im = &mut *crate::container_of!(listener, InputManager, new_input_listener);
    let wlr_device = data as *mut ffi::wlr_input_device;

    log::info!("new input device connected");

    let device = crate::input_device::InputDevice::new(im.default_seat, wlr_device, false);
    
    let name_ptr = ffi::river_wlr_input_device_get_name(wlr_device);
    if !name_ptr.is_null() {
        let name = std::ffi::CStr::from_ptr(name_ptr).to_string_lossy();
        let wm = &mut (*im.server).wm;
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
        let _ = crate::tablet::Tablet::create(im.default_seat, wlr_device, false);
    }

    // Attach device to the default seat
    (*im.default_seat).attach_device(device);
}

unsafe extern "C" fn handle_new_text_input(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let _im = crate::container_of!(listener, InputManager, new_text_input);
    let wlr_text_input = data as *mut ffi::wlr_text_input_v3;

    if let Err(e) = crate::text_input::TextInput::create(wlr_text_input) {
        log::error!("failed to create text input: {}", e);
        ffi::wl_resource_post_no_memory((*wlr_text_input).resource);
    }
}

unsafe extern "C" fn handle_new_input_method(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let im = &mut *crate::container_of!(listener, InputManager, new_input_method);
    let input_method = data as *mut ffi::wlr_input_method_v2;
    let seat = ffi::river_wlr_seat_get_data((*input_method).seat) as *mut Seat;
    if !seat.is_null() {
        (*seat).relay.new_input_method(input_method);
    } else {
        // Fallback to default seat if input_method.seat has no data set yet
        (*im.default_seat).relay.new_input_method(input_method);
    }
}

unsafe extern "C" fn handle_new_virtual_pointer(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let im = &mut *crate::container_of!(listener, InputManager, new_virtual_pointer_listener);
    let event = data as *mut ffi::wlr_virtual_pointer_v1_new_pointer_event;

    log::info!("new virtual pointer device connected");

    let virtual_pointer = (*event).new_pointer;
    let wlr_device = &mut (*virtual_pointer).pointer.base as *mut ffi::wlr_input_device;

    let device = crate::input_device::InputDevice::new(im.default_seat, wlr_device, true);

    // Attach device to the default seat
    (*im.default_seat).attach_device(device);
}

unsafe extern "C" fn handle_new_virtual_keyboard(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let im = &mut *crate::container_of!(listener, InputManager, new_virtual_keyboard_listener);
    let virtual_keyboard = data as *mut ffi::wlr_virtual_keyboard_v1;

    log::info!("new virtual keyboard device connected");

    let wlr_device = &mut (*virtual_keyboard).keyboard.base as *mut ffi::wlr_input_device;

    // Honor the seat the client bound the virtual keyboard to, like input
    // methods do; fall back to the default seat if it carries no Seat data.
    let mut seat = ffi::river_wlr_seat_get_data((*virtual_keyboard).seat) as *mut Seat;
    if seat.is_null() {
        seat = im.default_seat;
    }

    let device = crate::input_device::InputDevice::new(seat, wlr_device, true);
    crate::keyboard::Keyboard::create(device);

    // Same path a hardware keyboard takes: attach_device puts the keyboard in
    // a (private, virtual) KeyboardGroup, so compositor keybindings apply.
    (*seat).attach_device(device);
}

unsafe extern "C" fn handle_pointer_refresh_idle(data: *mut std::ffi::c_void) {
    let manager = data as *mut InputManager;
    (*manager).pointer_refresh_idle = std::ptr::null_mut();
    let seats_head = &mut (*manager).seats as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*seats_head).next;
    while curr != seats_head {
        let next = (*curr).next;
        let seat = crate::container_of!(curr, Seat, link);
        (*seat).cursor.refresh_after_scene_change();
        curr = next;
    }
}
