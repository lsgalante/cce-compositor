use crate::ffi;
use crate::seat::Seat;
use crate::server::{WlListener, wl_listener_remove};

pub struct InputDeviceConfig {
    pub scroll_factor: f64,
    pub map_to_output: *mut ffi::wlr_output,
    pub map_to_rectangle: ffi::wlr_box,
}

pub struct InputDevice {
    pub seat: *mut Seat,
    pub wlr_device: *mut ffi::wlr_input_device,
    pub virtual_device: bool,
    pub destroy_listener: ffi::wl_listener,
    pub config: InputDeviceConfig,
    pub destroy_fn: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    pub destroy_data: *mut std::ffi::c_void,
}

impl InputDevice {
    pub unsafe fn new(
        seat: *mut Seat,
        wlr_device: *mut ffi::wlr_input_device,
        virtual_device: bool,
    ) -> *mut Self {
        let device = Box::into_raw(Box::new(Self {
            seat,
            wlr_device,
            virtual_device,
            destroy_listener: std::mem::zeroed(),
            config: InputDeviceConfig {
                scroll_factor: 1.0,
                map_to_output: std::ptr::null_mut(),
                map_to_rectangle: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
            },
            destroy_fn: None,
            destroy_data: std::ptr::null_mut(),
        }));

        ffi::river_wlr_input_device_set_data(wlr_device, device as *mut _);

        let destroy_listener_ptr = &mut (*device).destroy_listener as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_listener_ptr).notify = Some(handle_device_destroy);
        
        let destroy_signal = ffi::river_wlr_input_device_get_destroy_signal(wlr_device);
        crate::server::wl_signal_add(destroy_signal, &mut (*device).destroy_listener);

        device
    }

    pub unsafe fn assign_to_seat(&mut self, new_seat: *mut Seat) {
        if self.seat == new_seat {
            return;
        }
        if !self.seat.is_null() {
            (*self.seat).detach_device(self);
        }
        self.seat = new_seat;
        if !self.seat.is_null() {
            (*self.seat).attach_device(self);
        }
    }

    pub unsafe fn active_mapping(&self) -> ffi::wlr_box {
        let mut mapping = self.config.map_to_rectangle;
        if !ffi::wlr_box_empty(&mapping) {
            return mapping;
        }
        if !self.config.map_to_output.is_null() {
            let server = (*self.seat).server;
            ffi::wlr_output_layout_get_box((*server).om.output_layout, self.config.map_to_output, &mut mapping);
        }
        mapping
    }
}

unsafe extern "C" fn handle_device_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let device_ptr = crate::container_of!(listener, InputDevice, destroy_listener) as *mut InputDevice;
    let device = &mut *device_ptr;
    
    log::debug!(
        "removed input device: {:?}",
        ffi::river_wlr_input_device_get_type(device.wlr_device)
    );

    // Call custom destroy callback if set (e.g. to clean up Tablet/Keyboard wrappers)
    if let Some(destroy_fn) = device.destroy_fn {
        destroy_fn(device.destroy_data);
    }

    // Remove destroy listener
    wl_listener_remove(&mut device.destroy_listener);

    // Detach from seat if attached
    if !device.seat.is_null() {
        (*device.seat).detach_device(device);
    }

    // Free wrapper memory
    let _boxed = Box::from_raw(device_ptr);
}
