#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct wl_listener {
    pub link: wl_list,
    pub notify: ::std::option::Option<
        unsafe extern "C" fn(listener: *mut wl_listener, data: *mut ::std::os::raw::c_void),
    >,
}

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

extern "C" {
    pub fn river_init_wlroots_log(importance: wlr_log_importance);
}
