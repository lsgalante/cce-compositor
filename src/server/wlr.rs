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
//!   field (`std::mem::zeroed()`, `Server::zeroed`) is a valid unset one,
//!   and setting through `&self` lets the shared, init-once handles
//!   ([`crate::shared::Handles`]) be `Wlr`s too.
//!
//! It is not `Clone`: a second copy of the pointer would be a second hold
//! the owner's destroy path does not know to clear. For the same reason a
//! `Wlr` field reachable through `&mut` is PRIVATE, read through a
//! same-named accessor (`seat.wlr_seat()`): a `mem::swap` or `replace` from
//! another module would move the hold where the destroy path cannot see it.
//! The shared handles are only ever reached through `&'static`, so they
//! stay public fields.

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

    /// The xkb index of modifier `name` (`c"Control"`) in the seat's
    /// keyboard's keymap; `None` without a keyboard, a keymap or the name.
    pub fn keyboard_mod_index(&self, name: &CStr) -> Option<u32> {
        let kbd = self.keyboard();
        if kbd.is_null() {
            return None;
        }
        // SAFETY: as `keyboard_modifiers`; the keymap is the keyboard's own.
        let idx = unsafe {
            let keymap = (*kbd).keymap;
            if keymap.is_null() {
                return None;
            }
            ffi::xkb_keymap_mod_get_index(keymap, name.as_ptr())
        };
        (idx != ffi::XKB_MOD_INVALID).then_some(idx)
    }

    /// The seat's keyboard's full modifier state; `None` without one.
    pub fn keyboard_modifier_state(&self) -> Option<ffi::wlr_keyboard_modifiers> {
        let kbd = self.keyboard();
        // SAFETY: as `keyboard_modifiers`.
        (!kbd.is_null()).then(|| unsafe { (*kbd).modifiers })
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

    /// Give the pointer to `surface` at surface (`sx`, `sy`).
    ///
    /// # Safety
    /// `surface` is a live wl_surface.
    pub unsafe fn pointer_notify_enter(&self, surface: *mut ffi::wlr_surface, sx: f64, sy: f64) {
        if let Some(s) = self.live() {
            ffi::wlr_seat_pointer_notify_enter(s, surface, sx, sy);
        }
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

    /// A finger lands on `surface` at surface (`sx`, `sy`).
    ///
    /// # Safety
    /// `surface` is a live wl_surface.
    pub unsafe fn touch_notify_down(&self, surface: *mut ffi::wlr_surface, time_msec: u32, touch_id: i32, sx: f64, sy: f64) -> u32 {
        match self.live() {
            Some(s) => ffi::wlr_seat_touch_notify_down(s, surface, time_msec, touch_id, sx, sy),
            None => 0,
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
    ///
    /// # Safety
    /// `manager` must outlive this cursor's hold on it: wlroots keeps it in
    /// the cursor's state, and reloads from it on an output scale change or
    /// a new output, until the image is next reset (`unset_image`, a
    /// surface, another xcursor) or the cursor is destroyed.
    pub unsafe fn set_xcursor(&self, manager: &Wlr<ffi::wlr_xcursor_manager>, name: &CStr) {
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
    /// The panel's physical size in mm, as the output reports it (EDID, or
    /// a configured `size_mm`); (0, 0) when unset or unknown.
    pub fn phys_size(&self) -> (i32, i32) {
        let (mut w, mut h) = (0, 0);
        if let Some(o) = self.live() {
            unsafe { ffi::river_wlr_output_get_phys_size(o, &mut w, &mut h) };
        }
        (w, h)
    }

    /// Whether committing `state` would succeed (a TEST_ONLY commit);
    /// false when unset.
    pub fn test_state(&self, state: &OutputStateBuf) -> bool {
        self.live().is_some_and(|o| unsafe { ffi::wlr_output_test_state(o, &state.0) })
    }

    /// Commit `state`; false when unset or refused.
    pub fn commit_state(&self, state: &OutputStateBuf) -> bool {
        self.live().is_some_and(|o| unsafe { ffi::wlr_output_commit_state(o, &state.0) })
    }

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

    /// Whether `mode` is one of this output's own modes (compared only, never
    /// read); false when unset.
    pub fn has_mode(&self, mode: *const ffi::wlr_output_mode) -> bool {
        self.live().is_some_and(|o| unsafe { ffi::river_wlr_output_has_mode(o, mode) })
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

    /// `f` handed the WM_CLASS class, the WM_CLASS instance and the title,
    /// borrowed (copied only when not valid UTF-8); empty when unset.
    pub fn with_names<R>(&self, f: impl FnOnce(&str, &str, &str) -> R) -> R {
        let text = |p: *const std::ffi::c_char| -> std::borrow::Cow<'_, str> {
            // SAFETY: a string the live surface holds, which the XWM
            // replaces only from its event handlers — never inside `f`.
            if p.is_null() { std::borrow::Cow::Borrowed("") } else { unsafe { CStr::from_ptr(p) }.to_string_lossy() }
        };
        match self.live() {
            // SAFETY: the field holds a live surface (`set`'s contract).
            Some(x) => unsafe { f(&text((*x).class), &text((*x).instance), &text((*x).title)) },
            None => f("", "", ""),
        }
    }

    /// Whether `window_manager { xwayland_hidpi_except }` names this
    /// window (`xwayland_window::hidpi_exempt`). Never while unset, or
    /// while `xwayland_hidpi` is off.
    pub fn hidpi_exempt(&self) -> bool {
        let layout = crate::shared::layout();
        if self.is_null() || !layout.xwayland_hidpi || layout.xwayland_hidpi_except.is_empty() {
            return false;
        }
        self.with_names(|class, instance, title| {
            crate::xwayland_window::hidpi_exempt(&layout.xwayland_hidpi_except, class, instance, title)
        })
    }

    /// The X11 scale this surface is drawn at: 1 when the window is named
    /// in `window_manager { xwayland_hidpi_except }`, else the screen's
    /// factor (`xwayland_window::x11_scale`).
    ///
    /// The screen Xwayland shows is one thing for every client, so an
    /// exempt window still SEES a physical-pixel root; what changes is what
    /// the compositor does with it. Its configures go out in logical pixels
    /// and its buffer is drawn at 1 (`Window::x11_buffer_scale`), the
    /// pre-`xwayland_hidpi` arrangement — so a borderless-fullscreen game
    /// that asks for the whole root is answered with the logical size and
    /// renders that many pixels, not scale² as many. Re-evaluated on every
    /// use, so a title that arrives after the first configure still takes
    /// effect; it runs every frame for every X11 window, hence the borrowed
    /// names.
    pub fn x11_scale(&self) -> f32 {
        if self.hidpi_exempt() {
            return 1.0;
        }
        crate::xwayland_window::x11_scale()
    }

    /// The WM_NORMAL_HINTS flags (`USPosition`, `PMinSize`, …); `None`
    /// when unset or the client set no hints.
    pub fn size_hints_flags(&self) -> Option<u32> {
        let x = self.live()?;
        // SAFETY: a live surface's `size_hints` is null or the XWM's own
        // copy, replaced only from its event handlers.
        unsafe {
            let hints = (*x).size_hints;
            (!hints.is_null()).then(|| (*hints).flags)
        }
    }

    /// Whether an override-redirect window is of a type that wants the
    /// keyboard (`wlr_xwayland_surface_override_redirect_wants_focus`).
    pub fn override_redirect_wants_focus(&self) -> bool {
        self.live().is_some_and(|x| unsafe { ffi::wlr_xwayland_surface_override_redirect_wants_focus(x) })
    }

    /// The ICCCM input model (WM_HINTS input and WM_TAKE_FOCUS); `NONE`
    /// when unset.
    pub fn icccm_input_model(&self) -> ffi::wlr_xwayland_icccm_input_model {
        self.live().map_or(ffi::wlr_xwayland_icccm_input_model_WLR_ICCCM_INPUT_MODEL_NONE, |x| unsafe {
            ffi::wlr_xwayland_surface_icccm_input_model(x)
        })
    }

    /// Map the associated surface if it already holds a buffer; a no-op
    /// when it is mapped, has none, or is unassociated.
    ///
    /// wlroots maps an X11 surface on the first commit AFTER the XWM pairs
    /// it with its X window (`xwayland_surface_handle_commit`). The Wayland
    /// and X connections are separate, so Xwayland's first commit can land
    /// before the pairing; a window that draws once and never commits again
    /// (a static dialog, a Wine helper) then stays unmapped for good while
    /// X reports it mapped — about one X11 window in ten created while
    /// another was closing (2026-10-12). Called once the map listener is
    /// connected, this does what that missed commit would have.
    ///
    /// # Safety
    /// Mapping runs the surface's map listeners, and the compositor's take
    /// the window manager themselves (`reentry::wm`): no window-manager
    /// borrow may be live, unless no such listener is connected yet.
    pub unsafe fn map_if_buffered(&self) {
        let surface = self.surface();
        if surface.is_null() {
            return;
        }
        // SAFETY: a live surface's `surface` is null or the wl_surface it is
        // associated with, which carries the Xwayland role that maps on a
        // buffered commit.
        unsafe {
            if ffi::wlr_surface_has_buffer(surface) && !ffi::river_wlr_surface_is_mapped(surface) {
                log::info!("xwayland surface {} held a buffer before it was paired; mapping it", self.window_id());
                ffi::wlr_surface_map(surface);
            }
        }
    }

    /// Forget the associated `wlr_surface`'s data pointer (nothing while
    /// unassociated).
    pub fn clear_surface_data(&self) {
        let surface = self.surface();
        if !surface.is_null() {
            // SAFETY: a live surface's `surface` is null or the wlr_surface
            // it is associated with, live until the dissociate clears it.
            unsafe { ffi::river_wlr_surface_set_data(surface, std::ptr::null_mut()) }
        }
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

impl Wlr<ffi::wlr_pointer_constraint_v1> {
    /// The constrained surface, for comparing (or handing to wlroots while
    /// the constraint lives); null when unset.
    pub fn surface(&self) -> *mut ffi::wlr_surface {
        self.live().map_or(std::ptr::null_mut(), |c| unsafe { (*c).surface })
    }

    /// The `wlr_seat` the constraint is for (null once let go), to find its
    /// seat by (`Seats::find`, `seat::place_holding`).
    pub fn seat(&self) -> *mut ffi::wlr_seat {
        self.live().map_or(std::ptr::null_mut(), |c| unsafe { (*c).seat })
    }

    /// Whether it locks the pointer (rather than confining it).
    pub fn is_lock(&self) -> bool {
        self.live().is_some_and(|c| unsafe { (*c).type_ == ffi::wlr_pointer_constraint_v1_type_WLR_POINTER_CONSTRAINT_V1_LOCKED })
    }

    /// Whether surface point (`sx`, `sy`) is inside the constraint's region.
    pub fn region_contains(&self, sx: f64, sy: f64) -> bool {
        self.live().is_some_and(|c| unsafe {
            ffi::pixman_region32_contains_point(&mut (*c).region, sx as i32, sy as i32, std::ptr::null_mut()) != 0
        })
    }

    /// Where a motion from surface (`sx`, `sy`) to (`tx`, `ty`) stops when
    /// confined to the region (`wlr_region_confine`); `None` when it cannot.
    pub fn region_confine(&self, sx: f64, sy: f64, tx: f64, ty: f64) -> Option<(f64, f64)> {
        let c = self.live()?;
        let (mut nx, mut ny) = (0.0, 0.0);
        unsafe { ffi::wlr_region_confine(&mut (*c).region, sx, sy, tx, ty, &mut nx, &mut ny) }.then_some((nx, ny))
    }

    /// The client's cursor hint (surface coordinates), when it set one.
    pub fn cursor_hint(&self) -> Option<(f64, f64)> {
        let c = self.live()?;
        unsafe { (*c).current.cursor_hint.enabled.then(|| ((*c).current.cursor_hint.x, (*c).current.cursor_hint.y)) }
    }

    pub fn send_activated(&self) {
        if let Some(c) = self.live() {
            unsafe { ffi::wlr_pointer_constraint_v1_send_activated(c) }
        }
    }
}

fn opt_ptr(s: Option<&CStr>) -> *const std::ffi::c_char {
    s.map_or(std::ptr::null(), CStr::as_ptr)
}

impl Wlr<ffi::wlr_ext_foreign_toplevel_handle_v1> {
    /// Publish the toplevel's title and app id (wlroots copies both).
    pub fn update_state(&self, title: Option<&CStr>, app_id: Option<&CStr>) {
        if let Some(h) = self.live() {
            let state = ffi::wlr_ext_foreign_toplevel_handle_v1_state { title: opt_ptr(title), app_id: opt_ptr(app_id) };
            unsafe { ffi::wlr_ext_foreign_toplevel_handle_v1_update_state(h, &state) }
        }
    }

    /// Destroy the handle and leave the field unset.
    pub fn destroy(&self) {
        let h = self.take();
        if !h.is_null() {
            unsafe { ffi::wlr_ext_foreign_toplevel_handle_v1_destroy(h) }
        }
    }
}

impl Wlr<ffi::wlr_foreign_toplevel_handle_v1> {
    pub fn set_title(&self, title: &CStr) {
        if let Some(h) = self.live() {
            unsafe { ffi::wlr_foreign_toplevel_handle_v1_set_title(h, title.as_ptr()) }
        }
    }

    pub fn set_app_id(&self, app_id: &CStr) {
        if let Some(h) = self.live() {
            unsafe { ffi::wlr_foreign_toplevel_handle_v1_set_app_id(h, app_id.as_ptr()) }
        }
    }

    pub fn set_activated(&self, activated: bool) {
        if let Some(h) = self.live() {
            unsafe { ffi::wlr_foreign_toplevel_handle_v1_set_activated(h, activated) }
        }
    }

    /// Destroy the handle and leave the field unset.
    pub fn destroy(&self) {
        let h = self.take();
        if !h.is_null() {
            unsafe { ffi::wlr_foreign_toplevel_handle_v1_destroy(h) }
        }
    }
}

impl Wlr<ffi::wlr_drag_icon> {
    /// How its drag is driven (`wlr_drag_grab_type`); keyboard when unset.
    pub fn grab_type(&self) -> ffi::wlr_drag_grab_type {
        // SAFETY: a live icon's drag outlives it (wlroots destroys the icon
        // inside the drag's destroy).
        self.live().map_or(ffi::wlr_drag_grab_type_WLR_DRAG_GRAB_KEYBOARD, |i| unsafe {
            ffi::river_wlr_drag_get_grab_type((*i).drag)
        })
    }

    /// The touch point driving a touch drag.
    pub fn touch_id(&self) -> i32 {
        // SAFETY: as `grab_type`.
        self.live().map_or(0, |i| unsafe { ffi::river_wlr_drag_get_touch_id((*i).drag) })
    }
}

/// The pointer-gestures sends: each takes the seat by its `Wlr` and values
/// only, and does nothing when either is unset.
macro_rules! gesture_sends {
    ($($name:ident($($arg:ident: $ty:ty),*) => $ffi:ident;)*) => {
        impl Wlr<ffi::wlr_pointer_gestures_v1> {$(
            pub fn $name(&self, seat: &Wlr<ffi::wlr_seat>, time_msec: u32, $($arg: $ty),*) {
                if let (Some(g), Some(s)) = (self.live(), seat.live()) {
                    unsafe { ffi::$ffi(g, s, time_msec, $($arg),*) }
                }
            }
        )*}
    };
}

gesture_sends! {
    send_swipe_begin(fingers: u32) => wlr_pointer_gestures_v1_send_swipe_begin;
    send_swipe_update(dx: f64, dy: f64) => wlr_pointer_gestures_v1_send_swipe_update;
    send_swipe_end(cancelled: bool) => wlr_pointer_gestures_v1_send_swipe_end;
    send_pinch_begin(fingers: u32) => wlr_pointer_gestures_v1_send_pinch_begin;
    send_pinch_update(dx: f64, dy: f64, scale: f64, rotation: f64) => wlr_pointer_gestures_v1_send_pinch_update;
    send_pinch_end(cancelled: bool) => wlr_pointer_gestures_v1_send_pinch_end;
}

impl Wlr<ffi::wlr_relative_pointer_manager_v1> {
    /// Relative motion to every relative-pointer client of `seat` (deltas
    /// in layout px, accelerated and raw).
    pub fn send_relative_motion(&self, seat: &Wlr<ffi::wlr_seat>, time_usec: u64, dx: f64, dy: f64, unaccel_dx: f64, unaccel_dy: f64) {
        if let (Some(m), Some(s)) = (self.live(), seat.live()) {
            unsafe { ffi::wlr_relative_pointer_manager_v1_send_relative_motion(m, s, time_usec, dx, dy, unaccel_dx, unaccel_dy) }
        }
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

/// A `wlr_output_state` that is initialised when made and finished when
/// dropped. Its state is private: only the setters below and
/// `Wlr<wlr_scene_output>::build_state` fill it, so the buffer and the rest
/// a commit reads are wlroots' own, never a pointer safe code forged.
pub struct OutputStateBuf(ffi::wlr_output_state);

impl OutputStateBuf {
    pub fn new() -> Self {
        // SAFETY: an all-zero state is what `wlr_output_state_init` starts
        // from (it zeroes it, then sets up the damage region); the state
        // holds no pointer to itself, so it may move once made.
        unsafe {
            let mut state: ffi::wlr_output_state = std::mem::zeroed();
            ffi::wlr_output_state_init(&mut state);
            OutputStateBuf(state)
        }
    }

    pub fn set_scale(&mut self, scale: f32) {
        // SAFETY: an initialised state; the setter only writes its fields.
        unsafe { ffi::wlr_output_state_set_scale(&mut self.0, scale) }
    }

    pub fn set_transform(&mut self, transform: ffi::wl_output_transform) {
        // SAFETY: as `set_scale`.
        unsafe { ffi::wlr_output_state_set_transform(&mut self.0, transform) }
    }

    /// Ask for a tearing (async) page flip.
    pub fn set_tearing_page_flip(&mut self, tearing: bool) {
        self.0.tearing_page_flip = tearing;
    }

    /// The buffer `build_state` attached; null before. Live until this
    /// state is dropped.
    pub fn buffer(&self) -> *mut ffi::wlr_buffer {
        self.0.buffer
    }
}

impl Default for OutputStateBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for OutputStateBuf {
    fn drop(&mut self) {
        // SAFETY: initialised in `new`, finished once here.
        unsafe { ffi::wlr_output_state_finish(&mut self.0) }
    }
}

impl Wlr<ffi::wlr_scene_output> {
    /// Whether the output has damage (or a frame was asked for).
    pub fn needs_frame(&self) -> bool {
        self.live().is_some_and(|s| unsafe { ffi::wlr_scene_output_needs_frame(s) })
    }

    /// Render the scene into `state` for this output; false when unset or
    /// it failed.
    pub fn build_state(&self, state: &mut OutputStateBuf) -> bool {
        self.live().is_some_and(|s| unsafe { ffi::wlr_scene_output_build_state(s, &mut state.0, std::ptr::null()) })
    }

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

impl Wlr<ffi::wlr_output_manager_v1> {
    /// Make `config` the current output configuration clients see; the
    /// manager takes it. Destroyed instead when the field is unset.
    ///
    /// # Safety
    /// `config` is a live configuration that nothing else owns.
    pub unsafe fn set_configuration(&self, config: *mut ffi::wlr_output_configuration_v1) {
        match self.live() {
            Some(m) => ffi::wlr_output_manager_v1_set_configuration(m, config),
            None => ffi::wlr_output_configuration_v1_destroy(config),
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
    /// Hand the XWM its seat again, which re-announces the seat's clipboard
    /// and primary selections to X (a fresh SetSelectionOwner each).
    ///
    /// wlroots refuses an X client's selection request while no Xwayland
    /// surface has keyboard focus. Copying in a Wayland app makes the XWM the
    /// X owner at once, and Wine's clipboard manager reads the new owner's
    /// data right then — refused, so Wine holds an empty clipboard, and
    /// focusing the Wine window changes no owner, so it never asks again:
    /// nothing copied in a Wayland app could be pasted into Ubisoft Connect
    /// (2026-10-12). Re-announced once an X11 window is activated, Wine reads
    /// it while it may. A no-op while unset or before the XWM is up.
    pub fn reassert_selections(&self) {
        if let Some(xwayland) = self.live() {
            // SAFETY: the field holds a live xwayland; its `seat` is the one
            // the input manager handed it (`wlr_xwayland_set_seat`, which
            // wlroots clears when that seat is destroyed) or null.
            unsafe {
                let seat = (*xwayland).seat;
                if !seat.is_null() && !(*xwayland).xwm.is_null() {
                    ffi::wlr_xwayland_set_seat(xwayland, seat);
                }
            }
        }
    }

    /// A CARDINAL-array property of X window `window` (format 32), at most
    /// `max_words` long, read synchronously over the XWM's own connection;
    /// `None` when unset, before the XWM is up, or when the window, atom or
    /// property is missing (an X error is a null reply, not a crash).
    pub fn cardinal_property(&self, window: ffi::xcb_window_t, name: &[u8], max_words: u32) -> Option<Vec<u32>> {
        let xwayland = self.live()?;
        // SAFETY: the field holds a live xwayland, so its XWM connection is
        // null or open; every request passes values and a name of the
        // stated length; each reply is checked for null, read within its
        // own value length, and freed once.
        unsafe {
            let conn = ffi::wlr_xwayland_get_xwm_connection(xwayland);
            if conn.is_null() {
                return None;
            }
            let cookie = ffi::xcb_intern_atom(conn, 1, name.len() as u16, name.as_ptr() as *const libc::c_char);
            let atom_reply = ffi::xcb_intern_atom_reply(conn, cookie, std::ptr::null_mut());
            if atom_reply.is_null() {
                return None;
            }
            let atom = (*atom_reply).atom;
            libc::free(atom_reply as *mut libc::c_void);
            if atom == 0 {
                return None;
            }
            // AnyPropertyType.
            let cookie = ffi::xcb_get_property(conn, 0, window, atom, 0, 0, max_words);
            let reply = ffi::xcb_get_property_reply(conn, cookie, std::ptr::null_mut());
            if reply.is_null() {
                return None;
            }
            let words = if (*reply).format == 32 {
                let len = ffi::xcb_get_property_value_length(reply) as usize / 4;
                let data = ffi::xcb_get_property_value(reply) as *const u32;
                Some(std::slice::from_raw_parts(data, len).to_vec())
            } else {
                None
            };
            libc::free(reply as *mut libc::c_void);
            words
        }
    }

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

impl Wlr<ffi::wl_global> {
    /// Destroy the global and let go of it; nothing when unset.
    pub fn destroy(&self) {
        let global = self.take();
        if !global.is_null() {
            // SAFETY: the field held it live (`set`'s contract) and has let
            // go, so this is its only destruction.
            unsafe { ffi::wl_global_destroy(global) };
        }
    }
}

impl Wlr<ffi::wlr_session> {
    /// Switch to virtual terminal `vt`; false when unset or it failed.
    pub fn change_vt(&self, vt: u32) -> bool {
        self.live().is_some_and(|s| unsafe { ffi::wlr_session_change_vt(s, vt) })
    }
}

impl Wlr<ffi::wlr_backend> {
    /// Whether it is the Wayland (nested) backend; false when unset.
    pub fn is_wl(&self) -> bool {
        self.live().is_some_and(|b| unsafe { ffi::wlr_backend_is_wl(b) })
    }

    /// Whether it is the X11 (nested) backend; false when unset.
    pub fn is_x11(&self) -> bool {
        self.live().is_some_and(|b| unsafe { ffi::wlr_backend_is_x11(b) })
    }
}

/// The layer surface's committed (`current`) state, by value: each reads
/// zero when unset.
macro_rules! layer_current {
    ($($name:ident: $ty:ty = $field:ident;)*) => {$(
        pub fn $name(&self) -> $ty {
            // SAFETY: the field holds a live layer surface (`set`'s contract).
            self.live().map_or(Default::default(), |l| unsafe { (*l).current.$field })
        }
    )*};
}

impl Wlr<ffi::wlr_layer_surface_v1> {
    layer_current! {
        layer: u32 = layer;
        keyboard_interactive: u32 = keyboard_interactive;
        exclusive_zone: i32 = exclusive_zone;
        anchor: u32 = anchor;
        committed: u32 = committed;
        actual_width: u32 = actual_width;
        actual_height: u32 = actual_height;
    }

    /// The layer surface's wl_surface; null when unset.
    pub fn surface(&self) -> *mut ffi::wlr_surface {
        self.live().map_or(std::ptr::null_mut(), |l| unsafe { (*l).surface })
    }

    /// The output it is on (it never moves); null when unset, or once that
    /// output went away (`forget_output`).
    pub fn output(&self) -> *mut ffi::wlr_output {
        self.live().map_or(std::ptr::null_mut(), |l| unsafe { (*l).output })
    }

    /// Forget the output it is on, which is being destroyed: wlroots does
    /// not, and every reader of `output` treats null as "none".
    pub fn forget_output(&self) {
        if let Some(l) = self.live() {
            // SAFETY: a live layer surface; `output` is ours to set (wlroots
            // reads it only when the surface is created).
            unsafe { (*l).output = std::ptr::null_mut() };
        }
    }

    /// Whether this commit is the client's first (wlroots' `initial_commit`).
    pub fn initial_commit(&self) -> bool {
        self.live().is_some_and(|l| unsafe { (*l).initial_commit })
    }

    /// Whether its wl_surface is mapped.
    pub fn is_mapped(&self) -> bool {
        let surface = self.surface();
        // SAFETY: a live layer surface's wl_surface is live.
        !surface.is_null() && unsafe { ffi::river_wlr_surface_is_mapped(surface) }
    }

    /// `f` handed the namespace, borrowed (copied only when not valid
    /// UTF-8); empty when unset. A namespace is fixed at creation.
    pub fn with_namespace<R>(&self, f: impl FnOnce(&str) -> R) -> R {
        let ns = self.live().map_or(std::ptr::null(), |l| unsafe { (*l).namespace });
        if ns.is_null() {
            return f("");
        }
        // SAFETY: the live layer surface's own string, never replaced.
        f(&unsafe { CStr::from_ptr(ns) }.to_string_lossy())
    }

    /// The pid of the client that owns it; 0 when it cannot be read.
    pub fn client_pid(&self) -> i32 {
        let surface = self.surface();
        if surface.is_null() {
            return 0;
        }
        // SAFETY: a live layer surface's wl_surface is live, and so are its
        // resource and that resource's client.
        unsafe {
            let res = ffi::river_wlr_surface_get_resource(surface);
            if res.is_null() {
                return 0;
            }
            let client = ffi::wl_resource_get_client(res);
            if client.is_null() {
                return 0;
            }
            let (mut pid, mut uid, mut gid) = (0, 0, 0);
            ffi::wl_client_get_credentials(client, &mut pid, &mut uid, &mut gid);
            pid
        }
    }

    /// Destroy every xdg popup it has open (each popup's own destroy
    /// handler then tears its side down).
    pub fn destroy_popups(&self) {
        let Some(l) = self.live() else { return };
        // SAFETY: `popups` is the live layer surface's list of its live
        // popups; destroying one unlinks only that one, and the next link
        // is read before the visit.
        unsafe {
            let head = &mut (*l).popups as *mut ffi::wl_list as *mut crate::server::WlList;
            let mut curr = (*head).next;
            while curr != head {
                let next = (*curr).next;
                let popup = crate::container_of!(curr, ffi::wlr_xdg_popup, link);
                ffi::wlr_xdg_popup_destroy(popup);
                curr = next;
            }
        }
    }
}

impl Wlr<ffi::wlr_xdg_popup> {
    /// The popup's xdg surface; null when unset.
    pub fn base(&self) -> *mut ffi::wlr_xdg_surface {
        self.live().map_or(std::ptr::null_mut(), |p| unsafe { ffi::river_wlr_xdg_popup_get_base(p) })
    }

    /// The popup's wl_surface; null when unset.
    pub fn surface(&self) -> *mut ffi::wlr_surface {
        let base = self.base();
        // SAFETY: a live popup's xdg surface is live.
        if base.is_null() { std::ptr::null_mut() } else { unsafe { ffi::river_wlr_xdg_surface_get_surface(base) } }
    }

    /// The wl_surface's size; (0, 0) when unset.
    pub fn surface_size(&self) -> (i32, i32) {
        let surface = self.surface();
        if surface.is_null() {
            return (0, 0);
        }
        // SAFETY: a live popup's wl_surface is live.
        unsafe { (ffi::river_wlr_surface_get_width(surface), ffi::river_wlr_surface_get_height(surface)) }
    }

    /// Whether this commit is the xdg surface's first.
    pub fn initial_commit(&self) -> bool {
        let base = self.base();
        // SAFETY: as `surface`.
        !base.is_null() && unsafe { ffi::river_wlr_xdg_surface_get_initial_commit(base) }
    }

    /// The anchor rectangle, relative to the parent surface; zero when
    /// unset.
    pub fn anchor_rect(&self) -> ffi::wlr_box {
        let mut anchor = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
        if let Some(p) = self.live() {
            unsafe { ffi::river_wlr_xdg_popup_get_anchor_rect(p, &mut anchor) };
        }
        anchor
    }

    /// Fit the popup into `toplevel_box` (the root surface's coordinates)
    /// and schedule the configure that tells the client.
    pub fn unconstrain_and_configure(&self, toplevel_box: &ffi::wlr_box) {
        if let Some(p) = self.live() {
            let mut constraint = *toplevel_box;
            // SAFETY: `constraint` is a local the call only reads; the popup's
            // xdg surface is live with it.
            unsafe {
                ffi::wlr_xdg_popup_unconstrain_from_box(p, &mut constraint);
                ffi::wlr_xdg_surface_schedule_configure(ffi::river_wlr_xdg_popup_get_base(p));
            }
        }
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
