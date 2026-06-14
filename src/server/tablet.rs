use crate::ffi;
use crate::input_device::InputDevice;
use crate::seat::Seat;

pub struct Tablet {
    pub device: *mut InputDevice,
    pub wp_tablet: *mut ffi::wlr_tablet_v2_tablet,
}

impl Tablet {
    pub unsafe fn create(
        seat: *mut Seat,
        wlr_device: *mut ffi::wlr_input_device,
        virtual_device: bool,
    ) -> Result<*mut Self, &'static str> {
        let server = (*seat).server;
        let tablet_manager = (*server).input_manager.tablet_manager;
        if tablet_manager.is_null() {
            return Err("Tablet manager is null");
        }

        let wp_tablet = ffi::wlr_tablet_create(tablet_manager, (*seat).wlr_seat, wlr_device);
        if wp_tablet.is_null() {
            return Err("Failed to create wp_tablet");
        }

        let device = InputDevice::new(seat, wlr_device, virtual_device);
        if device.is_null() {
            return Err("Failed to create input device");
        }

        let tablet = Box::into_raw(Box::new(Self {
            device,
            wp_tablet,
        }));

        // Set up the custom destroy callback on the input device
        (*device).destroy_fn = Some(destroy_tablet_callback);
        (*device).destroy_data = tablet as *mut _;

        Ok(tablet)
    }

    pub unsafe fn destroy(tablet: *mut Self) {
        let _boxed = Box::from_raw(tablet);
    }
}

unsafe extern "C" fn destroy_tablet_callback(data: *mut std::ffi::c_void) {
    Tablet::destroy(data as *mut Tablet);
}
