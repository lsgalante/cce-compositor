// SPDX-License-Identifier: GPL-3.0-only

//! [`Wlr<T>`]: a field holding a wlroots object that stays live while the
//! field holds it, so calls on it need no `unsafe` at the call site.
//!
//! The compositor kept its wlroots objects as raw `*mut ffi::wlr_*` fields,
//! and every call through one was an FFI call on a pointer the compiler
//! knew nothing about: a function that only flushed a pointer frame
//! (`wlr_seat_pointer_notify_frame`) was `unsafe` for that alone. `Wlr<T>`
//! puts the one fact that makes those calls sound behind the type, as
//! [`Timer`](crate::event_source::Timer) does for event sources:
//!
//! - [`Wlr::set`] is the `unsafe` step: whoever stores a pointer promises
//!   the object outlives the field's hold on it — until [`Wlr::take`]
//!   clears it, or the field is dropped. Each owner keeps that promise in
//!   the one place it already handles the object's end: its destroy path
//!   (the seat, the cursor) or the wlroots destroy signal that removes it
//!   from its store (the output).
//! - The methods are safe. Those whose other arguments are plain values (or
//!   another `Wlr`) are wrapped here; a call that also hands wlroots a raw
//!   surface or device pointer stays an FFI call through [`Wlr::ptr`],
//!   because that second pointer is the caller's to vouch for.
//! - An unset field (null) makes every method do nothing and return the
//!   zero value (null, `false`, `0`), so a field set after construction
//!   (the cursor's, in `Cursor::init`) needs no null check at each use.
//! - It is `repr(transparent)` over a `Cell<*mut T>`, so a zero-filled
//!   field (`std::mem::zeroed()`, `Server::default`) is a valid unset one,
//!   and setting through `&self` lets the shared, init-once handles
//!   ([`crate::shared::Handles`]) be `Wlr`s too.
//!
//! It is not `Clone`: a second copy of the pointer would be a second hold
//! the owner's destroy path does not know to clear.

use crate::ffi;
use std::cell::Cell;
use std::ffi::CStr;

#[repr(transparent)]
pub struct Wlr<T>(Cell<*mut T>);

impl<T> Default for Wlr<T> {
    fn default() -> Self {
        Self::null()
    }
}

impl<T> Wlr<T> {
    /// An unset field: every call on it does nothing.
    pub const fn null() -> Self {
        Wlr(Cell::new(std::ptr::null_mut()))
    }

    /// Hold `ptr`. Null leaves the field unset.
    ///
    /// # Safety
    /// A non-null `ptr` must be a live object of its type, and stay live
    /// until this field lets go of it: [`Self::take`], another `set`, or
    /// the field's drop.
    pub unsafe fn set(&self, ptr: *mut T) {
        self.0.set(ptr);
    }

    /// A field holding `ptr`.
    ///
    /// # Safety
    /// As [`Self::set`].
    pub unsafe fn from_raw(ptr: *mut T) -> Self {
        Wlr(Cell::new(ptr))
    }

    /// The object's address, for the FFI calls not wrapped here and for
    /// identity comparisons. Null when unset.
    pub fn ptr(&self) -> *mut T {
        self.0.get()
    }

    pub fn is_null(&self) -> bool {
        self.0.get().is_null()
    }

    /// Let go of the object, leaving the field unset, and return it: for
    /// the owner's destroy path, which then destroys it (or lets wlroots).
    pub fn take(&self) -> *mut T {
        self.0.replace(std::ptr::null_mut())
    }

    /// Whether this field holds `ptr` (and is set).
    pub fn is(&self, ptr: *const T) -> bool {
        !self.is_null() && std::ptr::eq(self.ptr(), ptr)
    }

    /// The pointer, if set.
    fn live(&self) -> Option<*mut T> {
        let p = self.0.get();
        (!p.is_null()).then_some(p)
    }
}

/// A NUL-terminated string an object owns, copied out.
///
/// # Safety
/// `s` is null or a valid C string.
unsafe fn owned_str(s: *const std::ffi::c_char) -> String {
    if s.is_null() {
        String::new()
    } else {
        CStr::from_ptr(s).to_string_lossy().into_owned()
    }
}

// Each call below passes a pointer the field holds, live by `set`'s
// contract, and otherwise only values; the SAFETY notes say only what is
// particular to a call.

impl Wlr<ffi::wlr_seat> {
    pub fn name(&self) -> String {
        // SAFETY: the name is the seat's, valid while it lives.
        self.live().map_or_else(String::new, |s| unsafe { owned_str(ffi::river_wlr_seat_get_name(s)) })
    }

    pub fn set_capabilities(&self, caps: u32) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_seat_set_capabilities(s, caps) }
        }
    }

    /// The keyboard the seat sends keys from, or null. The seat clears it
    /// when that keyboard is destroyed.
    pub fn keyboard(&self) -> *mut ffi::wlr_keyboard {
        self.live().map_or(std::ptr::null_mut(), |s| unsafe { ffi::river_wlr_seat_get_keyboard(s) })
    }

    /// The modifiers held on the seat's keyboard; none without one.
    pub fn keyboard_modifiers(&self) -> u32 {
        let kbd = self.keyboard();
        if kbd.is_null() {
            return 0;
        }
        // SAFETY: the seat's keyboard is live while the seat holds it.
        unsafe { ffi::wlr_keyboard_get_modifiers(kbd) }
    }

    /// The seat's keyboard's key repeat `(rate, delay)`; none without one.
    pub fn keyboard_repeat_info(&self) -> Option<(i32, i32)> {
        let kbd = self.keyboard();
        // SAFETY: as `keyboard_modifiers`.
        (!kbd.is_null()).then(|| unsafe { ((*kbd).repeat_info.rate, (*kbd).repeat_info.delay) })
    }

    /// Set the seat's keyboard's key repeat (`0, 0` stops it); nothing
    /// without one.
    pub fn set_keyboard_repeat_info(&self, rate: i32, delay: i32) {
        let kbd = self.keyboard();
        if !kbd.is_null() {
            // SAFETY: as `keyboard_modifiers`.
            unsafe { ffi::wlr_keyboard_set_repeat_info(kbd, rate, delay) }
        }
    }

    pub fn pointer_focused_client(&self) -> *mut ffi::wlr_seat_client {
        self.live().map_or(std::ptr::null_mut(), |s| unsafe { ffi::river_wlr_seat_get_pointer_focused_client(s) })
    }

    pub fn pointer_focused_surface(&self) -> *mut ffi::wlr_surface {
        self.live().map_or(std::ptr::null_mut(), |s| unsafe { ffi::river_wlr_seat_get_pointer_focused_surface(s) })
    }

    pub fn keyboard_focused_surface(&self) -> *mut ffi::wlr_surface {
        self.live().map_or(std::ptr::null_mut(), |s| unsafe { ffi::river_wlr_seat_get_keyboard_focused_surface(s) })
    }

    /// Move the pointer within its focused surface to (`sx`, `sy`), telling
    /// the client (a pointer constraint's cursor hint).
    pub fn pointer_warp(&self, sx: f64, sy: f64) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_seat_pointer_warp(s, sx, sy) }
        }
    }

    pub fn pointer_notify_frame(&self) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_seat_pointer_notify_frame(s) }
        }
    }

    pub fn pointer_notify_clear_focus(&self) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_seat_pointer_notify_clear_focus(s) }
        }
    }

    pub fn pointer_notify_motion(&self, time_msec: u32, sx: f64, sy: f64) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_seat_pointer_notify_motion(s, time_msec, sx, sy) }
        }
    }

    /// Returns the serial, 0 when unset.
    pub fn pointer_notify_button(&self, time_msec: u32, button: u32, state: ffi::wl_pointer_button_state) -> u32 {
        self.live().map_or(0, |s| unsafe { ffi::wlr_seat_pointer_notify_button(s, time_msec, button, state) })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pointer_notify_axis(
        &self,
        time_msec: u32,
        orientation: ffi::wl_pointer_axis,
        value: f64,
        value_discrete: i32,
        source: ffi::wl_pointer_axis_source,
        relative_direction: ffi::wl_pointer_axis_relative_direction,
    ) {
        if let Some(s) = self.live() {
            unsafe {
                ffi::wlr_seat_pointer_notify_axis(s, time_msec, orientation, value, value_discrete, source, relative_direction)
            }
        }
    }

    pub fn keyboard_notify_clear_focus(&self) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_seat_keyboard_notify_clear_focus(s) }
        }
    }

    pub fn keyboard_notify_key(&self, time_msec: u32, key: u32, state: u32) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_seat_keyboard_notify_key(s, time_msec, key, state) }
        }
    }

    pub fn keyboard_notify_modifiers(&self, modifiers: &ffi::wlr_keyboard_modifiers) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_seat_keyboard_notify_modifiers(s, modifiers) }
        }
    }

    pub fn touch_notify_motion(&self, time_msec: u32, touch_id: i32, sx: f64, sy: f64) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_seat_touch_notify_motion(s, time_msec, touch_id, sx, sy) }
        }
    }

    /// Returns the serial, 0 when unset.
    pub fn touch_notify_up(&self, time_msec: u32, touch_id: i32) -> u32 {
        self.live().map_or(0, |s| unsafe { ffi::wlr_seat_touch_notify_up(s, time_msec, touch_id) })
    }

    /// How many touch points clients hold on this seat.
    pub fn touch_num_points(&self) -> i32 {
        self.live().map_or(0, |s| unsafe { ffi::wlr_seat_touch_num_points(s) })
    }

    pub fn touch_notify_frame(&self) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_seat_touch_notify_frame(s) }
        }
    }

    pub fn touch_cancel_point(&self, touch_id: i32) {
        if let Some(s) = self.live() {
            unsafe { ffi::river_wlr_seat_touch_cancel_point(s, touch_id) }
        }
    }

    /// Tell the idle notifier this seat saw user activity.
    pub fn notify_idle_activity(&self) {
        let notifier = crate::shared::handles().idle_notifier.ptr();
        if let (Some(s), false) = (self.live(), notifier.is_null()) {
            // SAFETY: the notifier handle is live while set (`Wlr::set`).
            unsafe { ffi::wlr_idle_notifier_v1_notify_activity(notifier, s) }
        }
    }
}

impl Wlr<ffi::wlr_cursor> {
    /// Layout x; 0 when unset.
    pub fn x(&self) -> f64 {
        self.live().map_or(0.0, |c| unsafe { ffi::river_wlr_cursor_get_x(c) })
    }

    /// Layout y; 0 when unset.
    pub fn y(&self) -> f64 {
        self.live().map_or(0.0, |c| unsafe { ffi::river_wlr_cursor_get_y(c) })
    }

    pub fn unset_image(&self) {
        if let Some(c) = self.live() {
            unsafe { ffi::wlr_cursor_unset_image(c) }
        }
    }

    /// Show the theme's image `name` from `manager`. Nothing if either is
    /// unset.
    pub fn set_xcursor(&self, manager: &Wlr<ffi::wlr_xcursor_manager>, name: &CStr) {
        if let (Some(c), Some(m)) = (self.live(), manager.live()) {
            unsafe { ffi::wlr_cursor_set_xcursor(c, m, name.as_ptr()) }
        }
    }

    /// Warp to layout `(lx, ly)`, unconstrained by any input device's
    /// mapping. Whether it moved (false when unset).
    pub fn warp(&self, lx: f64, ly: f64) -> bool {
        self.live().is_some_and(|c| unsafe { ffi::wlr_cursor_warp(c, std::ptr::null_mut(), lx, ly) })
    }

    /// Move by `(dx, dy)` layout px, unconstrained by any device mapping.
    pub fn move_by(&self, dx: f64, dy: f64) {
        if let Some(c) = self.live() {
            unsafe { ffi::wlr_cursor_move(c, std::ptr::null_mut(), dx, dy) }
        }
    }

    /// Warp to `(x, y)` in [0, 1] across the whole layout.
    pub fn warp_absolute(&self, x: f64, y: f64) {
        if let Some(c) = self.live() {
            unsafe { ffi::wlr_cursor_warp_absolute(c, std::ptr::null_mut(), x, y) }
        }
    }
}

impl Wlr<ffi::wlr_output> {
    pub fn name(&self) -> String {
        // SAFETY: the name is the output's, valid while it lives.
        self.live().map_or_else(String::new, |o| unsafe { owned_str(ffi::river_wlr_output_get_name(o)) })
    }

    /// Whether the output is enabled (as last committed).
    pub fn enabled(&self) -> bool {
        self.live().is_some_and(|o| unsafe { ffi::river_wlr_output_get_enabled(o) })
    }

    /// The current mode (one of the output's own), or null: unset, or a
    /// custom mode.
    pub fn current_mode(&self) -> *mut ffi::wlr_output_mode {
        self.live().map_or(std::ptr::null_mut(), |o| unsafe { ffi::river_wlr_output_get_current_mode(o) })
    }

    /// Whether adaptive sync is on (as last committed).
    pub fn adaptive_sync_enabled(&self) -> bool {
        self.live().is_some_and(|o| unsafe {
            ffi::river_wlr_output_get_adaptive_sync_status(o)
                == ffi::wlr_output_adaptive_sync_status_WLR_OUTPUT_ADAPTIVE_SYNC_ENABLED
        })
    }

    /// Refresh rate in mHz; 0 when unknown or unset.
    pub fn refresh(&self) -> i32 {
        self.live().map_or(0, |o| unsafe { ffi::river_wlr_output_get_refresh(o) })
    }

    /// Mode width in physical px.
    pub fn width(&self) -> i32 {
        self.live().map_or(0, |o| unsafe { ffi::river_wlr_output_get_width(o) })
    }

    /// Mode height in physical px.
    pub fn height(&self) -> i32 {
        self.live().map_or(0, |o| unsafe { ffi::river_wlr_output_get_height(o) })
    }

    pub fn schedule_frame(&self) {
        if let Some(o) = self.live() {
            unsafe { ffi::wlr_output_schedule_frame(o) }
        }
    }

    /// The output's box in the shared output layout; empty when it is not
    /// in the layout (or either is unset).
    pub fn layout_box(&self) -> ffi::wlr_box {
        let mut b = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
        let layout = crate::shared::handles().output_layout.ptr();
        if let (Some(o), false) = (self.live(), layout.is_null()) {
            // SAFETY: the layout handle is live while set; an output not in
            // it gets an empty box.
            unsafe { ffi::wlr_output_layout_get_box(layout, o, &mut b) }
        }
        b
    }
}

impl Wlr<ffi::wlr_xcursor_manager> {
    /// Load the theme at `scale`. Whether it loaded (false when unset).
    pub fn load(&self, scale: f32) -> bool {
        self.live().is_some_and(|m| unsafe { ffi::wlr_xcursor_manager_load(m, scale) })
    }

    /// The theme's cursor `name` at `scale`, or null (not in the theme, the
    /// scale not loaded, or unset). It lives as long as the manager does.
    pub fn xcursor(&self, name: &CStr, scale: f32) -> *mut ffi::wlr_xcursor {
        self.live().map_or(std::ptr::null_mut(), |m| unsafe { ffi::wlr_xcursor_manager_get_xcursor(m, name.as_ptr(), scale) })
    }
}

/// The X11 surface's fields, read through the held pointer. Strings come
/// back owned (empty when unset); pointers come back raw, for the caller to
/// vouch for. Writes (`data`, a restored size) stay FFI-side through
/// `ptr()`.
macro_rules! xsurface_fields {
    ($($name:ident: $ty:ty = $zero:expr;)*) => {$(
        pub fn $name(&self) -> $ty {
            // SAFETY: the field holds a live surface (`set`'s contract).
            self.live().map_or($zero, |x| unsafe { (*x).$name })
        }
    )*};
}

impl Wlr<ffi::wlr_xwayland_surface> {
    xsurface_fields! {
        window_id: ffi::xcb_window_t = 0;
        surface: *mut ffi::wlr_surface = std::ptr::null_mut();
        x: i16 = 0;
        y: i16 = 0;
        width: u16 = 0;
        height: u16 = 0;
        override_redirect: bool = false;
        pid: ffi::pid_t = 0;
        parent: *mut ffi::wlr_xwayland_surface = std::ptr::null_mut();
        decorations: u32 = 0;
        hints: *mut ffi::xcb_icccm_wm_hints_t = std::ptr::null_mut();
        size_hints: *mut ffi::xcb_size_hints_t = std::ptr::null_mut();
        fullscreen: bool = false;
        maximized_vert: bool = false;
        maximized_horz: bool = false;
        minimized: bool = false;
        data: *mut std::ffi::c_void = std::ptr::null_mut();
    }

    pub fn configure(&self, x: i16, y: i16, width: u16, height: u16) {
        if let Some(x11) = self.live() {
            unsafe { ffi::wlr_xwayland_surface_configure(x11, x, y, width, height) }
        }
    }

    pub fn set_maximized(&self, horizontal: bool, vertical: bool) {
        if let Some(x) = self.live() {
            unsafe { ffi::wlr_xwayland_surface_set_maximized(x, horizontal, vertical) }
        }
    }

    pub fn set_fullscreen(&self, fullscreen: bool) {
        if let Some(x) = self.live() {
            unsafe { ffi::wlr_xwayland_surface_set_fullscreen(x, fullscreen) }
        }
    }

    pub fn set_minimized(&self, minimized: bool) {
        if let Some(x) = self.live() {
            unsafe { ffi::wlr_xwayland_surface_set_minimized(x, minimized) }
        }
    }

    pub fn activate(&self, activated: bool) {
        if let Some(x) = self.live() {
            unsafe { ffi::wlr_xwayland_surface_activate(x, activated) }
        }
    }

    /// Ask the client to close the window (WM_DELETE_WINDOW, or a kill).
    pub fn close(&self) {
        if let Some(x) = self.live() {
            unsafe { ffi::wlr_xwayland_surface_close(x) }
        }
    }

    /// The X11 scale this surface is drawn at (`xwayland_hidpi`, less the
    /// exempt apps).
    pub fn x11_scale(&self) -> f32 {
        // SAFETY: the pointer is null or the live surface the field holds.
        unsafe { crate::xwayland_window::x11_scale_for(self.ptr()) }
    }

    /// WM_CLASS class, owned; empty when unset.
    pub fn class(&self) -> String {
        // SAFETY: the string is the surface's, valid while it lives.
        self.live().map_or_else(String::new, |x| unsafe { owned_str((*x).class) })
    }

    /// WM_CLASS instance, owned; empty when unset.
    pub fn instance(&self) -> String {
        // SAFETY: as `class`.
        self.live().map_or_else(String::new, |x| unsafe { owned_str((*x).instance) })
    }

    /// The window title, owned; empty when unset.
    pub fn title(&self) -> String {
        // SAFETY: as `class`.
        self.live().map_or_else(String::new, |x| unsafe { owned_str((*x).title) })
    }
}

impl Wlr<ffi::wlr_output_layout> {
    /// Put `output` in the layout at (`x`, `y`), or move it there.
    pub fn add(&self, output: &Wlr<ffi::wlr_output>, x: i32, y: i32) {
        if let (Some(l), Some(o)) = (self.live(), output.live()) {
            unsafe { ffi::wlr_output_layout_add(l, o, x, y) };
        }
    }

    /// Take `output` out of the layout (nothing if it is not in it).
    pub fn remove(&self, output: &Wlr<ffi::wlr_output>) {
        if let (Some(l), Some(o)) = (self.live(), output.live()) {
            unsafe { ffi::wlr_output_layout_remove(l, o) };
        }
    }

    /// The output at the layout point closest to (`lx`, `ly`) — the point
    /// itself when it is on an output. Null when there is none (or unset).
    /// The pointer is for comparing, or for a call made before anything can
    /// destroy an output.
    pub fn closest_output(&self, lx: f64, ly: f64) -> *mut ffi::wlr_output {
        let Some(l) = self.live() else { return std::ptr::null_mut() };
        let (mut cx, mut cy) = (0.0, 0.0);
        // A null reference output means any output.
        unsafe {
            ffi::wlr_output_layout_closest_point(l, std::ptr::null_mut(), lx, ly, &mut cx, &mut cy);
            ffi::wlr_output_layout_output_at(l, cx, cy)
        }
    }

    /// The layout box of the output under (`lx`, `ly`), if one is.
    pub fn box_at(&self, lx: f64, ly: f64) -> Option<ffi::wlr_box> {
        let l = self.live()?;
        let mut b = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
        // The output comes from the layout, so it is live and in it.
        unsafe {
            let o = ffi::wlr_output_layout_output_at(l, lx, ly);
            if o.is_null() {
                return None;
            }
            ffi::wlr_output_layout_get_box(l, o, &mut b);
        }
        Some(b)
    }

    /// The layout box of the output closest to (`lx`, `ly`) (`closest_output`).
    pub fn closest_box(&self, lx: f64, ly: f64) -> Option<ffi::wlr_box> {
        let l = self.live()?;
        let o = self.closest_output(lx, ly);
        if o.is_null() {
            return None;
        }
        let mut b = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
        // As `box_at`.
        unsafe { ffi::wlr_output_layout_get_box(l, o, &mut b) };
        Some(b)
    }

    /// Whether an output covers (`lx`, `ly`).
    pub fn has_output_at(&self, lx: f64, ly: f64) -> bool {
        self.live().is_some_and(|l| unsafe { !ffi::wlr_output_layout_output_at(l, lx, ly).is_null() })
    }
}

impl Wlr<ffi::wlr_scene_output> {
    /// Damage the whole output, so its next frame repaints everything.
    pub fn damage_whole(&self) {
        if let Some(s) = self.live() {
            unsafe { ffi::river_scene_output_damage_whole(s) };
        }
    }

    /// Place the scene output at (`x`, `y`) in the scene.
    pub fn set_position(&self, x: i32, y: i32) {
        if let Some(s) = self.live() {
            unsafe { ffi::wlr_scene_output_set_position(s, x, y) };
        }
    }
}

impl Wlr<ffi::wlr_scene> {
    /// The scene's root tree; null when unset.
    pub fn tree(&self) -> *mut ffi::wlr_scene_tree {
        self.live().map_or(std::ptr::null_mut(), |s| unsafe { std::ptr::addr_of_mut!((*s).tree) })
    }

    /// Destroy the scene (its root node, and everything under it) and
    /// leave the field unset.
    pub fn destroy(&self) {
        let s = self.take();
        if !s.is_null() {
            // The root tree starts with its node (`wlr_scene_tree.node`).
            unsafe { ffi::wlr_scene_node_destroy(std::ptr::addr_of_mut!((*s).tree) as *mut ffi::wlr_scene_node) }
        }
    }
}

impl Wlr<ffi::wlr_idle_notifier_v1> {
    /// Tell idle clients whether something inhibits idling.
    pub fn set_inhibited(&self, inhibited: bool) {
        if let Some(n) = self.live() {
            unsafe { ffi::wlr_idle_notifier_v1_set_inhibited(n, inhibited) }
        }
    }
}

impl Wlr<ffi::wlr_xwayland> {
    /// The X display Xwayland serves (`:1`), owned; empty when unset.
    pub fn display_name(&self) -> String {
        // SAFETY: the field holds a live xwayland, whose layout
        // `server::WlrXwayland` mirrors (checked by layout_probe); the name
        // is a C string it owns, or null.
        self.live().map_or_else(String::new, |x| unsafe {
            owned_str((*(x as *mut crate::server::WlrXwayland)).display_name)
        })
    }
}

impl Wlr<ffi::wlr_xdg_toplevel> {
    /// Ask the client to close the toplevel.
    pub fn send_close(&self) {
        if let Some(t) = self.live() {
            unsafe { ffi::wlr_xdg_toplevel_send_close(t) }
        }
    }

    /// The toplevel's xdg surface; null when unset.
    pub fn base(&self) -> *mut ffi::wlr_xdg_surface {
        self.live().map_or(std::ptr::null_mut(), |t| unsafe { ffi::river_wlr_xdg_toplevel_get_base(t) })
    }
}

/// The xdg toplevel's configure setters: each takes values only and returns
/// the configure serial (0 when unset).
macro_rules! toplevel_setters {
    ($($name:ident($($arg:ident: $ty:ty),*) => $ffi:ident;)*) => {
        impl Wlr<ffi::wlr_xdg_toplevel> {$(
            pub fn $name(&self, $($arg: $ty),*) -> u32 {
                self.live().map_or(0, |t| unsafe { ffi::$ffi(t, $($arg),*) })
            }
        )*}
    };
}

toplevel_setters! {
    set_activated(activated: bool) => wlr_xdg_toplevel_set_activated;
    set_tiled(edges: u32) => wlr_xdg_toplevel_set_tiled;
    set_wm_capabilities(caps: u32) => wlr_xdg_toplevel_set_wm_capabilities;
    set_maximized(maximized: bool) => wlr_xdg_toplevel_set_maximized;
    set_fullscreen(fullscreen: bool) => wlr_xdg_toplevel_set_fullscreen;
    set_resizing(resizing: bool) => wlr_xdg_toplevel_set_resizing;
    set_bounds(width: i32, height: i32) => wlr_xdg_toplevel_set_bounds;
    set_size(width: i32, height: i32) => wlr_xdg_toplevel_set_size;
}


/// The intersection of two boxes, or `None` when it is empty (as is any
/// intersection with an empty box): `wlr_box_intersection`, by value.
pub fn box_intersection(a: &ffi::wlr_box, b: &ffi::wlr_box) -> Option<ffi::wlr_box> {
    let mut dest = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
    // SAFETY: all three pointers come from references; wlroots only reads
    // `a` and `b` and writes `dest`.
    unsafe { ffi::wlr_box_intersection(&mut dest, a, b) }.then_some(dest)
}
