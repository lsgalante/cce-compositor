use crate::ffi;
use crate::input_device::InputDevice;

pub struct Tablet {
    pub device: *mut InputDevice,
    pub wp_tablet: *mut ffi::wlr_tablet_v2_tablet,
}

impl Tablet {
    /// Wrap `device`, a tablet's `InputDevice`, as a keyboard's wraps its
    /// own (`Keyboard::create`), and offer it to clients on `wlr_seat`.
    /// Until 2026-10-13 this made a SECOND `InputDevice` for the same
    /// wlroots device: registered twice (clients of the input manager saw
    /// two devices), two destroy listeners, and the wlroots device's data
    /// overwritten to name the copy no seat ever attached.
    ///
    /// # Safety
    /// `device` is live and is a tablet; `wlr_seat` is null or live.
    pub unsafe fn create(device: *mut InputDevice, wlr_seat: *mut ffi::wlr_seat) -> Result<*mut Self, &'static str> {
        let tablet_manager = crate::shared::handles().tablet_manager.ptr();
        if tablet_manager.is_null() {
            return Err("Tablet manager is null");
        }
        if wlr_seat.is_null() {
            return Err("no wlr_seat for the tablet");
        }

        let wp_tablet = ffi::wlr_tablet_create(tablet_manager, wlr_seat, (*device).wlr_device());
        if wp_tablet.is_null() {
            return Err("Failed to create wp_tablet");
        }

        let tablet = Box::into_raw(Box::new(Self {
            device,
            wp_tablet,
        }));

        // The device's destroy handler frees it.
        (*device).set_tablet(tablet);

        Ok(tablet)
    }

    /// # Safety
    /// `tablet` came from [`create`](Self::create) and is freed here, once;
    /// nothing uses it after (its device's destroy path is the caller).
    pub unsafe fn destroy(tablet: *mut Self) {
        let _boxed = Box::from_raw(tablet);
    }
}
