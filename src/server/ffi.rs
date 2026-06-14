#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::all)]
#![allow(clippy::approx_constant)]

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct wl_listener {
    pub link: wl_list,
    pub notify: ::std::option::Option<
        unsafe extern "C" fn(listener: *mut wl_listener, data: *mut ::std::os::raw::c_void),
    >,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct pixman_region32_data {
    _unused: [u8; 0],
}

pub type pixman_region32_data_t = pixman_region32_data;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct pixman_box32 {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

pub type pixman_box32_t = pixman_box32;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct pixman_region32 {
    pub extents: pixman_box32,
    pub data: *mut pixman_region32_data,
}

pub type pixman_region32_t = pixman_region32;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct pixman_rectangle32 {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub type pixman_rectangle32_t = pixman_rectangle32;

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct wlr_input_device {
    pub type_: wlr_input_device_type,
    pub name: *mut ::std::os::raw::c_char,
    pub events: wlr_input_device_events,
    pub data: *mut ::std::os::raw::c_void,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct wlr_input_device_events {
    pub destroy: wl_signal,
}

extern "C" {
    pub fn river_init_wlroots_log(importance: wlr_log_importance);
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct wlr_addon {
    pub impl_: *const wlr_addon_interface,
    pub owner: *const ::std::os::raw::c_void,
    pub link: wl_list,
}
