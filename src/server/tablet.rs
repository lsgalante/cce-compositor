use crate::ffi;
use crate::input_device::InputDevice;
use crate::seat::Seat;

pub struct Tablet {
    pub device: *mut InputDevice,
    pub wp_tablet: *mut ffi::wlr_tablet_v2_tablet,
}

impl Tablet {
    /// # Safety
    /// `server` must point to the live `Server`, `seat` to a live seat (not
    /// null: its `wlr_seat` is read), and `wlr_device` to a live tablet
    /// input device.
    pub unsafe fn create(
        server: *mut crate::server::Server,
        outputs: &crate::output::Outputs,
        seat: *mut Seat,
        wlr_device: *mut ffi::wlr_input_device,
        virtual_device: bool,
    ) -> Result<*mut Self, &'static str> {
        let tablet_manager = crate::shared::handles().tablet_manager.ptr();
        if tablet_manager.is_null() {
            return Err("Tablet manager is null");
        }

        let wp_tablet = ffi::wlr_tablet_create(tablet_manager, (*seat).wlr_seat().ptr(), wlr_device);
        if wp_tablet.is_null() {
            return Err("Failed to create wp_tablet");
        }

        let device = InputDevice::new(server, outputs, seat, wlr_device, virtual_device);
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

    /// # Safety
    /// `tablet` came from [`create`](Self::create) and is freed here, once;
    /// nothing uses it after (its device's destroy path is the caller).
    pub unsafe fn destroy(tablet: *mut Self) {
        let _boxed = Box::from_raw(tablet);
    }
}

unsafe extern "C" fn destroy_tablet_callback(data: *mut std::ffi::c_void) {
    Tablet::destroy(data as *mut Tablet);
}
