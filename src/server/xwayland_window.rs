// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, wl_signal_add};
use crate::window::{Window, WindowImpl, WindowState};
use crate::xwayland_override_redirect::XwaylandOverrideRedirect;

#[repr(C)]
pub struct XwaylandWindow {
    pub window: *mut Window,
    pub xsurface: *mut ffi::wlr_xwayland_surface,
    pub surface_tree: *mut ffi::wlr_scene_tree,

    pub destroy: ffi::wl_listener,
    pub request_configure: ffi::wl_listener,
    pub set_override_redirect: ffi::wl_listener,
    pub associate: ffi::wl_listener,
    pub dissociate: ffi::wl_listener,
    pub set_size_hints: ffi::wl_listener,
    pub set_title: ffi::wl_listener,
    pub set_class: ffi::wl_listener,
    pub set_parent: ffi::wl_listener,
    pub set_decorations: ffi::wl_listener,
    pub request_maximize: ffi::wl_listener,
    pub request_fullscreen: ffi::wl_listener,
    pub request_minimize: ffi::wl_listener,

    pub map: ffi::wl_listener,
    pub unmap: ffi::wl_listener,

    /// The last geometry this compositor handed to X through
    /// `send_configure`, physical pixels; `None` until the first one. See
    /// `needs_configure` for why this is kept apart from the wlroots mirror.
    pub sent_geom: Option<X11Geom>,
}

/// A window geometry in X11 root coordinates — physical pixels under
/// `xwayland_hidpi` (see `x11_scale`), the same units as the
/// `wlr_xwayland_surface` fields it is compared against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct X11Geom {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

/// Whether `wanted` has to be sent to X, given what wlroots reports the
/// window's geometry to be (`reported`) and the last geometry this
/// compositor sent (`sent`).
///
/// Comparing against the wlroots mirror alone was the bug: the saved-state
/// restore (`Window::try_restore`) pre-writes the mirror's width/height to
/// the saved size so the first frame renders at it, which makes the mirror
/// a statement of what the compositor wants X to have, not what X has. A
/// window saved fullscreen restored at exactly the fullscreen size then
/// looked already-configured, the configure was skipped, and the real X
/// window stayed at its natural size. So a geometry is also considered
/// unsent until this compositor has actually sent it once; after that the
/// mirror is what catches X changing the geometry on its own (a
/// ConfigureNotify updates it), which the sent record cannot see.
pub fn needs_configure(wanted: X11Geom, reported: X11Geom, sent: Option<X11Geom>) -> bool {
    wanted != reported || sent != Some(wanted)
}

unsafe fn connect_listener(
    signal: *mut ffi::wl_signal,
    listener: *mut ffi::wl_listener,
    callback: unsafe extern "C" fn(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void),
) {
    let wl_lis = listener as *mut WlListener;
    (*wl_lis).notify = Some(callback);
    wl_signal_add(signal, listener);
}

unsafe fn wl_listener_remove_safe(listener: *mut ffi::wl_listener) {
    let prev = (*listener).link.prev;
    let next = (*listener).link.next;
    if !prev.is_null() && !next.is_null() && prev != listener as *mut ffi::wl_list && next != listener as *mut ffi::wl_list {
        ffi::wl_list_remove(&mut (*listener).link);
        (*listener).link.prev = std::ptr::null_mut();
        (*listener).link.next = std::ptr::null_mut();
    }
}

/// Wine draws its own frame in a margin around the window; the compositor
/// hides it by oversizing and offsetting the X window. Logical pixels.
pub const WINE_MARGIN: i32 = 16;

/// The factor between X11 root coordinates and the logical layout.
///
/// With `xwayland_hidpi` on (the default) the xdg-output global is hidden
/// from Xwayland (`server.rs`), so it sizes its screen from the wl_output
/// MODE — the physical pixel grid — and X11 is a physical-pixel world: a
/// HiDPI-aware X11 app (Houdini, any Qt 6 app reading Xft.dpi) renders at
/// full resolution and its surfaces are drawn at 1/scale
/// (`Window::x11_buffer_scale`), sharp instead of upscaled from logical size.
/// Every position and size crossing into or out of X11 converts through
/// `to_x11` / `from_x11`; the window's own geometry stays logical.
///
/// Off, X11 is the logical layout (Xwayland reads xdg-output) and the
/// factor is 1. Multi-output with differing scales is not a case X11 can
/// express — the first output's scale stands for the screen. A single
/// window can opt out through `xwayland_hidpi_except` — see `x11_scale_for`,
/// which every per-window caller goes through; this screen-wide value is
/// only for what has no window, like the Xft.dpi pushed at Xwayland-ready.
///
/// The factor must survive the output going away. A lid-close suspend
/// destroys the DRM output and re-creates it on resume, and X11 windows
/// live on through it: any geometry read back from X while no output
/// exists (`Window::render_finish`, `XwaylandWindow::configure`) is still
/// in physical pixels, and dividing it by 1 instead of the panel's scale
/// records a window twice its logical size — which the first configure
/// after resume then multiplies by the real scale again, handing X a
/// window four times too big. So the last scale an output reported is
/// remembered and stands in while there is none.
pub unsafe fn x11_scale(server: *mut crate::server::Server) -> f32 {
    if server.is_null() || !(*server).wm.xwayland_hidpi {
        return 1.0;
    }
    let mut current = None;
    let link = (*server).om.outputs.next;
    if link != &mut (*server).om.outputs as *mut ffi::wl_list {
        let output = crate::container_of!(link, crate::output::Output, link);
        let scale = (*output).current.scale;
        if scale > 0.0 {
            current = Some(scale);
        }
    }
    resolve_x11_scale(current, &LAST_X11_SCALE)
}

/// The scale of the last output `x11_scale` saw, as `f32` bits; 0 until an
/// output has reported one.
static LAST_X11_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// `x11_scale` without the FFI: the live output's scale when there is one
/// (remembering it in `last`), else the remembered one, else 1.
pub fn resolve_x11_scale(current: Option<f32>, last: &std::sync::atomic::AtomicU32) -> f32 {
    use std::sync::atomic::Ordering;
    if let Some(scale) = current {
        last.store(scale.to_bits(), Ordering::Relaxed);
        return scale;
    }
    let remembered = f32::from_bits(last.load(Ordering::Relaxed));
    if remembered > 0.0 { remembered } else { 1.0 }
}

/// X11 clients answer an output coming or going — Xwayland re-creates its
/// screen and RandR tells them — by re-asserting a geometry of their own:
/// Houdini (Qt) asks for the whole panel after a resume from suspend, and a
/// floating window's unsolicited size request is otherwise honoured
/// verbatim (`handle_request_configure`). For a moment after any output
/// change those requests are answered with the window's own geometry
/// instead, the way a tiled window's always are, so the size the user set
/// survives the screen change.
static OUTPUT_CHANGE_GRACE_UNTIL: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// How long after an output is created or destroyed to hold floating X11
/// windows at their own size. Xwayland's RandR update and the client's
/// reaction land within the same second in practice; this leaves room for
/// a slow client.
const OUTPUT_CHANGE_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Called when an output is created or destroyed: (re)starts the grace
/// window during which floating X11 windows keep their size.
pub fn note_output_change() {
    if let Ok(mut deadline) = OUTPUT_CHANGE_GRACE_UNTIL.lock() {
        *deadline = Some(std::time::Instant::now() + OUTPUT_CHANGE_GRACE);
    }
}

/// True while inside the grace window begun by `note_output_change`.
pub fn in_output_change_grace() -> bool {
    OUTPUT_CHANGE_GRACE_UNTIL
        .lock()
        .ok()
        .and_then(|deadline| *deadline)
        .is_some_and(|deadline| std::time::Instant::now() < deadline)
}

/// `x11_scale` for one X11 surface: 1 when the window is named in
/// `window_manager { xwayland_hidpi_except }`, else the screen's factor.
///
/// The screen Xwayland shows is one thing for every client, so an exempt
/// window still SEES a physical-pixel root; what changes is what the
/// compositor does with it. Its configures go out in logical pixels and its
/// buffer is drawn at 1 (`Window::x11_buffer_scale`), the pre-`xwayland_hidpi`
/// arrangement — so a borderless-fullscreen game that asks for the whole
/// root is answered with the logical size and renders that many pixels,
/// not scale² as many. Matched by WM_CLASS class, WM_CLASS instance or
/// title (`hidpi_exempt`), re-evaluated on every use so a title that
/// arrives after the first configure still takes effect.
pub unsafe fn x11_scale_for(
    server: *mut crate::server::Server,
    xsurface: *const ffi::wlr_xwayland_surface,
) -> f32 {
    if server.is_null() || !(*server).wm.xwayland_hidpi {
        return 1.0;
    }
    if !xsurface.is_null() && !(*server).wm.xwayland_hidpi_except.is_empty() {
        let text = |p: *const libc::c_char| -> String {
            if p.is_null() { String::new() } else { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() }
        };
        let class = text((*xsurface).class);
        let instance = text((*xsurface).instance);
        let title = text((*xsurface).title);
        if hidpi_exempt(&(*server).wm.xwayland_hidpi_except, &class, &instance, &title) {
            return 1.0;
        }
    }
    x11_scale(server)
}

/// The factor an X11 client's CURSOR is drawn at: the screen's X11 factor
/// for any X11 surface, 1 for anything that is not X11.
///
/// What a cursor request needs. Deliberately NOT `x11_scale_for`: the
/// `xwayland_hidpi_except` exemption is about a game's window pixels — it
/// sizes itself to the root ignoring DPI, so its buffer is drawn at 1 —
/// but its cursor comes from the toolkit or Wine underneath, which follow
/// the DPI this compositor publishes (Xft.dpi / Xcursor.size at 96×scale
/// and 24×scale). Wine at LogPixels 192 hands Trackmania a 64px arrow;
/// drawn in the logical world with the window it was twice the desktop's
/// cursor. Every X11 cursor is a physical-pixel bitmap, exempt window or
/// not.
pub unsafe fn x11_scale_for_surface(
    server: *mut crate::server::Server,
    surface: *mut ffi::wlr_surface,
) -> f32 {
    if surface.is_null() {
        return 1.0;
    }
    let root = ffi::wlr_surface_get_root_surface(surface);
    if root.is_null() {
        return 1.0;
    }
    let xsurface = ffi::wlr_xwayland_surface_try_from_wlr_surface(root);
    if xsurface.is_null() {
        return 1.0;
    }
    x11_scale(server)
}

/// Whether any of `patterns` names this window: each is tried against the
/// WM_CLASS class, the WM_CLASS instance and the title with the
/// `app_id_matches` rules (case-insensitive, `*` wildcards). Empty fields
/// never match.
/// Whether `window` is an X11 window named in `xwayland_hidpi_except` — a
/// full-screen X11 game, by the key's definition. Such a window sizes and
/// places itself to the screen, and the compositor stays out of its way:
/// no saved-state restore (`Window::try_restore`); its own requests are
/// granted, clamped to the output's logical box since it sees a
/// physical-pixel root (`handle_request_configure`); a compositor
/// fullscreen overrides its size and survives Wine's withdrawal of the
/// state (`handle_request_fullscreen`). Its cursor is NOT exempt — see
/// `x11_scale_for_surface`.
pub unsafe fn window_is_hidpi_exempt(window: *const crate::window::Window) -> bool {
    if window.is_null() {
        return false;
    }
    let crate::window::WindowImpl::Xwayland(xwindow) = (*window).impl_type else {
        return false;
    };
    if xwindow.is_null() || (*xwindow).xsurface.is_null() {
        return false;
    }
    let server = (*window).server;
    if server.is_null() || !(*server).wm.xwayland_hidpi || (*server).wm.xwayland_hidpi_except.is_empty() {
        return false;
    }
    let xsurface = (*xwindow).xsurface;
    let text = |p: *const libc::c_char| -> String {
        if p.is_null() { String::new() } else { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() }
    };
    hidpi_exempt(
        &(*server).wm.xwayland_hidpi_except,
        &text((*xsurface).class),
        &text((*xsurface).instance),
        &text((*xsurface).title),
    )
}

pub fn hidpi_exempt(patterns: &[String], class: &str, instance: &str, title: &str) -> bool {
    use crate::window_manager::app_id_matches;
    patterns.iter().any(|p| {
        [class, instance, title]
            .iter()
            .any(|field| !field.is_empty() && app_id_matches(p, field))
    })
}

pub fn to_x11(logical: i32, scale: f32) -> i32 {
    (logical as f32 * scale).round() as i32
}

pub fn from_x11(x11: i32, scale: f32) -> i32 {
    (x11 as f32 / scale).round() as i32
}

/// The nearest X11 value the LOGICAL grid can express — `to_x11` of
/// `from_x11`.
///
/// At scale 2 an odd X11 coordinate has no logical integer: 181 reads back as
/// 91 and goes out again as 182. The window's geometry is logical, so
/// granting a client's request verbatim and storing the rounded logical means
/// the next configure hands X a value a pixel off the one it asked for —
/// which the client reads as an unrequested move and answers with another
/// request, a pixel further along each time. Granting the snapped value makes
/// the geometry sent and the geometry the compositor's own model reproduces
/// the same number, so the round trip is stable however often it repeats.
///
/// At scale 1 this is the identity, so nothing outside `xwayland_hidpi`
/// changes.
pub fn snap_x11(x11: i32, scale: f32) -> i32 {
    to_x11(from_x11(x11, scale), scale)
}

impl XwaylandWindow {
    pub unsafe fn create(
        xsurface: *mut ffi::wlr_xwayland_surface,
        server: *mut Server,
    ) -> Result<(), &'static str> {
        let title_ptr = (*xsurface).title;
        let class_ptr = (*xsurface).class;
        log::debug!(
            "new xwayland window: title='{:?}', class='{:?}'",
            if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") },
            if class_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(class_ptr).to_str().unwrap_or("") }
        );

        let window = Window::create(WindowImpl::Xwayland(std::ptr::null_mut()), server)?;

        let xwindow = Box::new(XwaylandWindow {
            window,
            xsurface,
            surface_tree: std::ptr::null_mut(),
            destroy: std::mem::zeroed(),
            request_configure: std::mem::zeroed(),
            set_override_redirect: std::mem::zeroed(),
            associate: std::mem::zeroed(),
            dissociate: std::mem::zeroed(),
            set_size_hints: std::mem::zeroed(),
            set_title: std::mem::zeroed(),
            set_class: std::mem::zeroed(),
            set_parent: std::mem::zeroed(),
            set_decorations: std::mem::zeroed(),
            request_maximize: std::mem::zeroed(),
            request_fullscreen: std::mem::zeroed(),
            request_minimize: std::mem::zeroed(),
            map: std::mem::zeroed(),
            unmap: std::mem::zeroed(),
            sent_geom: None,
        });

        let raw = Box::into_raw(xwindow);
        (*window).set_impl(WindowImpl::Xwayland(raw));

        (*xsurface).data = raw as *mut std::ffi::c_void;

        connect_listener(&mut (*xsurface).events.destroy, &mut (*raw).destroy, handle_destroy);
        connect_listener(&mut (*xsurface).events.associate, &mut (*raw).associate, handle_associate);
        connect_listener(&mut (*xsurface).events.dissociate, &mut (*raw).dissociate, handle_dissociate);
        connect_listener(&mut (*xsurface).events.request_configure, &mut (*raw).request_configure, handle_request_configure);
        connect_listener(&mut (*xsurface).events.set_override_redirect, &mut (*raw).set_override_redirect, handle_set_override_redirect);
        // connect_listener(&mut (*xsurface).events.set_size_hints, &mut (*raw).set_size_hints, handle_set_size_hints);
        connect_listener(&mut (*xsurface).events.set_title, &mut (*raw).set_title, handle_set_title);
        connect_listener(&mut (*xsurface).events.set_class, &mut (*raw).set_class, handle_set_class);
        connect_listener(&mut (*xsurface).events.set_parent, &mut (*raw).set_parent, handle_set_parent);
        connect_listener(&mut (*xsurface).events.set_decorations, &mut (*raw).set_decorations, handle_set_decorations);
        connect_listener(&mut (*xsurface).events.request_maximize, &mut (*raw).request_maximize, handle_request_maximize);
        connect_listener(&mut (*xsurface).events.request_fullscreen, &mut (*raw).request_fullscreen, handle_request_fullscreen);
        connect_listener(&mut (*xsurface).events.request_minimize, &mut (*raw).request_minimize, handle_request_minimize);

        if !(*xsurface).surface.is_null() {
            handle_associate_impl(raw);
            if ffi::river_wlr_surface_is_mapped((*xsurface).surface) {
                handle_map_impl(raw);
            }
        }

        Ok(())
    }

    pub unsafe fn configure(&mut self) -> bool {
        let window = self.window;
        let scheduled = &mut (*window).configure_scheduled;
        let sent = &mut (*window).configure_sent;
        let s = x11_scale_for((*window).server, self.xsurface);

        if scheduled.width == Some(0) {
            scheduled.width = Some(from_x11((*self.xsurface).width as i32, s) as u32);
        }
        if scheduled.height == Some(0) {
            scheduled.height = Some(from_x11((*self.xsurface).height as i32, s) as u32);
        }

        let mut phys_width = if let Some(w) = scheduled.width {
            to_x11(w as i32, s) as u16
        } else {
            (*self.xsurface).width
        };

        let mut phys_height = if let Some(h) = scheduled.height {
            to_x11(h as i32, s) as u16
        } else {
            (*self.xsurface).height
        };

        // X11 root coordinates: see `x11_scale`. Everything sent to X goes
        // through `to_x11`, everything read back through `from_x11`; the
        // window's own geometry stays logical.
        let mut phys_x = to_x11((*window).box_geom.x, s) as i16;
        let mut phys_y = to_x11((*window).box_geom.y, s) as i16;

        let has_parent = !(*self.xsurface).parent.is_null();

        if (*window).is_wine() && !has_parent && !(*window).is_fullscreen() {
            if scheduled.width.is_some() {
                phys_width += to_x11(WINE_MARGIN * 2, s) as u16;
            }
            if scheduled.height.is_some() {
                phys_height += to_x11(WINE_MARGIN * 2, s) as u16;
            }
            phys_x -= to_x11(WINE_MARGIN, s) as i16;
            phys_y -= to_x11(WINE_MARGIN, s) as i16;
        }

        let wanted = X11Geom { x: phys_x, y: phys_y, width: phys_width, height: phys_height };
        if needs_configure(wanted, self.reported_geom(), self.sent_geom) {
            self.send_configure(wanted);
        }

        if scheduled.activated != sent.activated {
            self.set_activated(scheduled.activated);
        }
        if scheduled.maximized != sent.maximized {
            ffi::wlr_xwayland_surface_set_maximized(self.xsurface, scheduled.maximized, scheduled.maximized);
        }
        if scheduled.inform_fullscreen != sent.inform_fullscreen {
            ffi::wlr_xwayland_surface_set_fullscreen(self.xsurface, scheduled.inform_fullscreen);
        }

        let mut width = scheduled.width.unwrap_or(from_x11((*self.xsurface).width as i32, s) as u32);
        let mut height = scheduled.height.unwrap_or(from_x11((*self.xsurface).height as i32, s) as u32);

        if (*window).is_wine() && !has_parent && !(*window).is_fullscreen() {
            if scheduled.width.is_none() {
                width = width.saturating_sub((WINE_MARGIN * 2) as u32);
            }
            if scheduled.height.is_none() {
                height = height.saturating_sub((WINE_MARGIN * 2) as u32);
            }
        }

        (*window).configure_sent = (*window).configure_scheduled.clone();
        (*window).configure_sent.width = Some(width);
        (*window).configure_sent.height = Some(height);
        (*window).configure_scheduled.width = None;
        (*window).configure_scheduled.height = None;

        false
    }

    /// The geometry wlroots currently reports for the X window.
    pub unsafe fn reported_geom(&self) -> X11Geom {
        X11Geom {
            x: (*self.xsurface).x,
            y: (*self.xsurface).y,
            width: (*self.xsurface).width,
            height: (*self.xsurface).height,
        }
    }

    /// The one path to `wlr_xwayland_surface_configure`: every geometry
    /// handed to X is recorded in `sent_geom` so `needs_configure` can tell
    /// a geometry X has from one the compositor merely mirrored.
    pub unsafe fn send_configure(&mut self, g: X11Geom) {
        ffi::wlr_xwayland_surface_configure(self.xsurface, g.x, g.y, g.width, g.height);
        self.sent_geom = Some(g);
    }

    pub unsafe fn set_activated(&self, activated: bool) {
        if activated && (*self.xsurface).minimized {
            ffi::wlr_xwayland_surface_set_minimized(self.xsurface, false);
        }
        ffi::wlr_xwayland_surface_activate(self.xsurface, activated);
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, destroy);
    handle_destroy_impl(xwindow);
}

unsafe fn handle_destroy_impl(xwindow: *mut XwaylandWindow) {
    wl_listener_remove_safe(&mut (*xwindow).destroy);
    wl_listener_remove_safe(&mut (*xwindow).associate);
    wl_listener_remove_safe(&mut (*xwindow).dissociate);
    wl_listener_remove_safe(&mut (*xwindow).request_configure);
    wl_listener_remove_safe(&mut (*xwindow).set_override_redirect);
    wl_listener_remove_safe(&mut (*xwindow).set_size_hints);
    wl_listener_remove_safe(&mut (*xwindow).set_title);
    wl_listener_remove_safe(&mut (*xwindow).set_class);
    wl_listener_remove_safe(&mut (*xwindow).set_parent);
    wl_listener_remove_safe(&mut (*xwindow).set_decorations);
    wl_listener_remove_safe(&mut (*xwindow).request_maximize);
    wl_listener_remove_safe(&mut (*xwindow).request_fullscreen);
    wl_listener_remove_safe(&mut (*xwindow).request_minimize);

    (*(*xwindow).xsurface).data = std::ptr::null_mut();

    let window = (*xwindow).window;
    (*window).impl_destroying();

    let _ = Box::from_raw(xwindow);
}

unsafe extern "C" fn handle_associate(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, associate);
    handle_associate_impl(xwindow);
}

unsafe fn handle_associate_impl(xwindow: *mut XwaylandWindow) {
    let surface = (*(*xwindow).xsurface).surface;
    if !surface.is_null() {
        connect_listener(
            ffi::river_wlr_surface_get_map_signal(surface),
            &mut (*xwindow).map,
            handle_map,
        );
        connect_listener(
            ffi::river_wlr_surface_get_unmap_signal(surface),
            &mut (*xwindow).unmap,
            handle_unmap,
        );
    }
}

unsafe extern "C" fn handle_dissociate(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, dissociate);
    handle_dissociate_impl(xwindow);
}

unsafe fn handle_dissociate_impl(xwindow: *mut XwaylandWindow) {
    wl_listener_remove_safe(&mut (*xwindow).map);
    wl_listener_remove_safe(&mut (*xwindow).unmap);
}

unsafe extern "C" fn handle_map(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, map);
    handle_map_impl(xwindow);
}

unsafe fn handle_map_impl(xwindow: *mut XwaylandWindow) {
    let surfaces_tree = (*(*xwindow).window).surfaces.tree;
    let surface = (*(*xwindow).xsurface).surface;
    let surface_tree = ffi::wlr_scene_subsurface_tree_create(surfaces_tree, surface);
    if surface_tree.is_null() {
        log::error!("out of memory creating subsurface tree");
        let surface_resource = ffi::river_wlr_surface_get_resource(surface);
        let client = ffi::wl_resource_get_client(surface_resource);
        ffi::wl_client_post_no_memory(client);
        return;
    }
    (*xwindow).surface_tree = surface_tree;

    let has_parent = !(*(*xwindow).xsurface).parent.is_null();

    if (*(*xwindow).window).is_wine() && !has_parent && !(*(*xwindow).window).is_fullscreen() {
        ffi::wlr_scene_node_set_position(surface_tree as *mut ffi::wlr_scene_node, -WINE_MARGIN, -WINE_MARGIN);
    }

    ffi::river_wlr_surface_set_data(surface, &mut (*(*xwindow).window).node as *mut crate::wm_node::WmNode as *mut _);

    let capture_tree = &mut (*(*(*xwindow).window).capture_scene).tree as *mut ffi::wlr_scene_tree;
    let capture_surface = ffi::wlr_scene_surface_create(capture_tree, surface);
    if capture_surface.is_null() {
        log::error!("out of memory creating capture surface");
        let surface_resource = ffi::river_wlr_surface_get_resource(surface);
        let client = ffi::wl_resource_get_client(surface_resource);
        ffi::wl_client_post_no_memory(client);
        return;
    }

    if (*(*xwindow).xsurface).fullscreen {
        (*(*xwindow).window).wm_scheduled.fullscreen_requested = crate::window::FullscreenRequest::Fullscreen(std::ptr::null_mut());
    }

    place_transient_where_it_asked(xwindow);

    (*(*xwindow).window).state = WindowState::Initialized;
    if let Err(e) = (*(*xwindow).window).map() {
        log::error!("out of memory mapping window: {}", e);
        let surface_resource = ffi::river_wlr_surface_get_resource(surface);
        let client = ffi::wl_resource_get_client(surface_resource);
        ffi::wl_client_post_no_memory(client);
    }
    (*(*(*xwindow).window).server).wm.dirty_windowing();
}

/// A transient that asked for a position before mapping maps there.
///
/// `handle_request_configure` grants a request that arrives before the
/// window is mapped verbatim, but records nothing: the window has no
/// geometry yet and the arrange pass has not placed it. The grant updates
/// the X surface's x/y, and nothing read them back at map, so a dialog that
/// positioned itself before showing -- Qt's `move()` before `show()`, which
/// is how Houdini's HC Panel centres itself on the pane it was opened over
/// -- mapped at the constructor's default origin instead, a hundred pixels
/// in from the desk corner. The client never asks again, since X told it
/// the request was granted, so the dialog sat there for good.
///
/// Only a window with a parent, and only when the client says the position
/// is its own: ICCCM's `USPosition` / `PPosition` flags in WM_NORMAL_HINTS
/// are what toolkits set for an explicit move before map. A transient
/// without them is at whatever the X server defaulted to, and stays on the
/// compositor's placement. Top-level windows keep theirs too: restore and
/// the placement hints own those, and a transient is the one kind of window
/// `try_restore` refuses to touch.
unsafe fn place_transient_where_it_asked(xwindow: *mut XwaylandWindow) {
    let xsurface = (*xwindow).xsurface;
    if (*xsurface).parent.is_null() {
        return;
    }
    let Some(asked) = (*xwindow).sent_geom else {
        return;
    };
    let hints = (*xsurface).size_hints;
    if hints.is_null() {
        return;
    }
    let position_flags = ffi::xcb_icccm_size_hints_flags_t_XCB_ICCCM_SIZE_HINT_US_POSITION
        | ffi::xcb_icccm_size_hints_flags_t_XCB_ICCCM_SIZE_HINT_P_POSITION;
    if (*hints).flags & position_flags == 0 {
        return;
    }

    let window = (*xwindow).window;
    let s = x11_scale_for((*window).server, xsurface);
    let log_x = from_x11(asked.x as i32, s);
    let log_y = from_x11(asked.y as i32, s);
    let (vx, vy) = (*window).screen_to_virtual(log_x, log_y);
    (*window).virtual_x = vx;
    (*window).virtual_y = vy;
    // Placed by the client, like a picker placed by its hint: the camera
    // must not pan to it on spawn or first focus.
    (*window).hint_placed = true;
    log::info!(
        "XWayland transient mapped where it asked: title='{}' x11=({}, {}) logical=({}, {}) virtual=({:.1}, {:.1})",
        (*window).get_title_string().unwrap_or_default(),
        asked.x, asked.y, log_x, log_y, vx, vy,
    );
}

unsafe extern "C" fn handle_unmap(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, unmap);
    handle_unmap_impl(xwindow);
}

unsafe fn handle_unmap_impl(xwindow: *mut XwaylandWindow) {
    let surface = (*(*xwindow).xsurface).surface;
    if !surface.is_null() {
        ffi::river_wlr_surface_set_data(surface, std::ptr::null_mut());
    }
    (*(*xwindow).window).unmap();
    if !(*xwindow).surface_tree.is_null() {
        ffi::wlr_scene_node_destroy((*xwindow).surface_tree as *mut ffi::wlr_scene_node);
        (*xwindow).surface_tree = std::ptr::null_mut();
    }
}

unsafe extern "C" fn handle_request_configure(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, request_configure);
    let event = data as *mut ffi::wlr_xwayland_surface_configure_event;

    let surface = (*(*xwindow).xsurface).surface;
    if surface.is_null() || !ffi::river_wlr_surface_is_mapped(surface) {
        (*xwindow).send_configure(X11Geom {
            x: (*event).x,
            y: (*event).y,
            width: (*event).width,
            height: (*event).height,
        });
        return;
    }

    let class_ptr = (*(*xwindow).xsurface).class;
    let class = if class_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(class_ptr).to_str().unwrap_or("") };
    let title_ptr = (*(*xwindow).xsurface).title;
    let title = if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") };
    let window = (*xwindow).window;
    let is_wine = (*window).is_wine();

    let has_parent = !(*(*xwindow).xsurface).parent.is_null();
    let s = x11_scale_for((*window).server, (*xwindow).xsurface);
    log::info!(
        "XWayland configure request: title='{}' class='{}' has_parent={} is_wine={} event=({}, {}, {}, {}) xsurface=({}, {}, {}, {})",
        title,
        class,
        has_parent,
        is_wine,
        (*event).x, (*event).y, (*event).width, (*event).height,
        (*(*xwindow).xsurface).x, (*(*xwindow).xsurface).y, (*(*xwindow).xsurface).width, (*(*xwindow).xsurface).height,
    );

    let is_tiled = unsafe {
        (*window).wm_requested.tiled != 0 || !matches!((*window).tiling_mode, crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Popup | crate::tiling::TilingMode::Utility)
    };
    let is_fullscreen = unsafe { (*window).is_fullscreen() };

    // A window named in `xwayland_hidpi_except` is a full-screen X11 game
    // that sizes AND places itself to the screen (Trackmania's
    // "windowedfull" asks for (0, 0) at the desktop size). Refusing the
    // position — answering every request with the compositor's placement —
    // had Wine re-asking ~170 times a second for as long as the window was
    // up. It gets the parented treatment: position and size granted, the
    // virtual origin moved with it. Not while the compositor has it
    // fullscreen or tiled: then the size is the compositor's (below).
    let exempt_self_placed = !has_parent && !is_fullscreen && !is_tiled && window_is_hidpi_exempt(window);

    if has_parent || exempt_self_placed {
        // Granted on the logical grid rather than verbatim — see `snap_x11`.
        // The logical values below are what the window's geometry becomes, so
        // handing X anything else is handing it a number this compositor
        // cannot reproduce.
        let (mut ex, mut ey, mut ew, mut eh) =
            ((*event).x as i32, (*event).y as i32, (*event).width as i32, (*event).height as i32);
        if exempt_self_placed {
            // The game SEES the physical-pixel root and asks for all of it
            // (Trackmania's windowedfull: 3840x2160 at (0, 0)), but its
            // pixels are logical here — granted verbatim that is a window
            // twice the screen, which the arrange pass then keeps pulling
            // back on-desk while the game keeps asking, ~60 requests a
            // second. So the request is answered with at most the output's
            // logical box, kept on that output: the whole root becomes the
            // whole screen, which is what the game meant.
            let out = (*window).fullscreen_output();
            if !out.is_null() {
                let ob = (*out).sent.box_layout();
                let (ox, oy, ow, oh) = (ob.x, ob.y, ob.width, ob.height);
                ew = from_x11(ew, s).min(ow).max(1);
                eh = from_x11(eh, s).min(oh).max(1);
                ex = from_x11(ex, s).clamp(ox, (ox + ow - ew).max(ox));
                ey = from_x11(ey, s).clamp(oy, (oy + oh - eh).max(oy));
                ex = to_x11(ex, s);
                ey = to_x11(ey, s);
                ew = to_x11(ew, s);
                eh = to_x11(eh, s);
                if (ex, ey, ew, eh) != ((*event).x as i32, (*event).y as i32, (*event).width as i32, (*event).height as i32) {
                    log::info!(
                        "XWayland configure request: '{}' is hidpi-exempt; ({}, {}, {}x{}) clamped to the output's logical box as ({}, {}, {}x{})",
                        title, (*event).x, (*event).y, (*event).width, (*event).height, ex, ey, ew, eh,
                    );
                }
            }
        }
        (*xwindow).send_configure(X11Geom {
            x: snap_x11(ex, s) as i16,
            y: snap_x11(ey, s) as i16,
            width: snap_x11(ew, s) as u16,
            height: snap_x11(eh, s) as u16,
        });
        let log_x = from_x11(ex, s);
        let log_y = from_x11(ey, s);
        let log_width = from_x11(ew, s) as u32;
        let log_height = from_x11(eh, s) as u32;
        
        (*window).box_geom.x = log_x;
        (*window).box_geom.y = log_y;
        (*window).box_geom.width = log_width as i32;
        (*window).box_geom.height = log_height as i32;
        (*window).rendering_requested.x = log_x;
        (*window).rendering_requested.y = log_y;
        (*window).rendering_sent.width = log_width;
        (*window).rendering_sent.height = log_height;
        // The screen origin above is only half the move: the arrange pass
        // places a floating window from its VIRTUAL origin, so leaving that
        // stale meant the very next transaction recomputed the window back
        // to where it was. An X11 client reads that as its move being
        // refused and asks again from the position it was pushed to, which
        // is a runaway: Houdini's Edit Theme dialog walked 270px left across
        // one tab switch, re-requesting 15 times in a second and never
        // converging on a size either.
        let (vx, vy) = (*window).screen_to_virtual(log_x, log_y);
        (*window).virtual_x = vx;
        (*window).virtual_y = vy;
        if exempt_self_placed {
            // Placed by the client: the camera must not pan to it on spawn.
            (*window).hint_placed = true;
        }
        (*window).set_dimensions(log_width, log_height);
        return;
    }

    // A floating window normally gets the size it asks for; not while an
    // output is coming or going (see `note_output_change`), and not while
    // the compositor has it FULLSCREEN: Wine syncs a window's
    // _NET_WM_STATE from its own idea of the window rect, so a game whose
    // fixed-size hints were granted here shrank the X window back the
    // instant the fullscreen configure went out, then withdrew the
    // fullscreen state Wine no longer saw as true — every Fullscreen press
    // on Trackmania undid itself within the same frame. Held at the
    // fullscreen size, Wine sees a screen-sized rect and keeps the state.
    let hold_size = is_tiled || is_fullscreen || in_output_change_grace();
    if hold_size && !is_tiled {
        log::info!(
            "XWayland configure request: holding floating '{}' at its own size during output-change grace",
            title,
        );
    }

    let (phys_width, phys_height) = if hold_size {
        let log_w = (*window).configure_sent.width.unwrap_or((*window).box_geom.width as u32);
        let log_h = (*window).configure_sent.height.unwrap_or((*window).box_geom.height as u32);
        if log_w > 0 && log_h > 0 {
            let mut w = log_w;
            let mut h = log_h;
            if is_wine && !has_parent && !is_fullscreen {
                w += (WINE_MARGIN * 2) as u32;
                h += (WINE_MARGIN * 2) as u32;
            }
            (to_x11(w as i32, s) as u16, to_x11(h as i32, s) as u16)
        } else {
            (snap_x11((*event).width as i32, s) as u16, snap_x11((*event).height as i32, s) as u16)
        }
    } else {
        // Snapped for the same reason as the parented branch above: the size
        // stored below is `from_x11` of what goes out here, and the next
        // configure sends `to_x11` of that back.
        (snap_x11((*event).width as i32, s) as u16, snap_x11((*event).height as i32, s) as u16)
    };

    let mut phys_x = to_x11((*window).box_geom.x, s) as i16;
    let mut phys_y = to_x11((*window).box_geom.y, s) as i16;

    if is_wine && !has_parent && !is_fullscreen {
        phys_x -= to_x11(WINE_MARGIN, s) as i16;
        phys_y -= to_x11(WINE_MARGIN, s) as i16;
    }

    (*xwindow).send_configure(X11Geom { x: phys_x, y: phys_y, width: phys_width, height: phys_height });
    let mut log_width = from_x11(phys_width as i32, s) as u32;
    let mut log_height = from_x11(phys_height as i32, s) as u32;
    if is_wine && !has_parent && !is_fullscreen {
        log_width = log_width.saturating_sub((WINE_MARGIN * 2) as u32);
        log_height = log_height.saturating_sub((WINE_MARGIN * 2) as u32);
    }
    (*window).set_dimensions(log_width, log_height);
}

unsafe extern "C" fn handle_set_override_redirect(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_override_redirect);
    let xsurface = (*xwindow).xsurface;
    log::info!("xwayland surface set override redirect: val={}", (*xsurface).override_redirect);
    assert!((*xsurface).override_redirect);

    let surface = (*xsurface).surface;
    if !surface.is_null() {
        if ffi::river_wlr_surface_is_mapped(surface) {
            handle_unmap_impl(xwindow);
        }
        handle_dissociate_impl(xwindow);
    }
    let server = (*(*xwindow).window).server;
    handle_destroy_impl(xwindow);

    if let Err(e) = XwaylandOverrideRedirect::create(xsurface, server) {
        log::error!("Failed to create XwaylandOverrideRedirect: {}", e);
    }
}

#[allow(dead_code)]
unsafe extern "C" fn handle_set_size_hints(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_size_hints);
    let size_hints = (*(*xwindow).xsurface).size_hints;
    if !size_hints.is_null() {
        let min_width = std::cmp::max(0, (*size_hints).min_width) as u32;
        let min_height = std::cmp::max(0, (*size_hints).min_height) as u32;
        let max_width = if (*size_hints).max_width <= 0 {
            0
        } else {
            std::cmp::max(min_width, (*size_hints).max_width as u32)
        };
        let max_height = if (*size_hints).max_height <= 0 {
            0
        } else {
            std::cmp::max(min_height, (*size_hints).max_height as u32)
        };
        let hint = crate::window::DimensionsHint {
            min_width,
            max_width,
            min_height,
            max_height,
        };
        (*(*xwindow).window).set_dimensions_hint(hint);
    }
}

unsafe extern "C" fn handle_set_title(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_title);
    (*(*xwindow).window).notify_title();
}

unsafe extern "C" fn handle_set_class(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_class);
    (*(*xwindow).window).notify_app_id();
}

unsafe extern "C" fn handle_set_parent(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_parent);
    (*(*(*xwindow).window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_set_decorations(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, set_decorations);
    let prefers_csd = ((*(*xwindow).xsurface).decorations
        & (ffi::wlr_xwayland_surface_decorations_WLR_XWAYLAND_SURFACE_DECORATIONS_NO_BORDER
            | ffi::wlr_xwayland_surface_decorations_WLR_XWAYLAND_SURFACE_DECORATIONS_NO_TITLE) as u32)
        != 0;

    let hint = if prefers_csd {
        ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_PREFERS_CSD
    } else {
        ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_PREFERS_SSD
    };
    (*(*xwindow).window).set_decoration_hint(hint);
}

unsafe extern "C" fn handle_request_maximize(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, request_maximize);
    let maximized = (*(*xwindow).xsurface).maximized_vert || (*(*xwindow).xsurface).maximized_horz;
    let window = (*xwindow).window;
    if maximized {
        (*window).tiling_mode = crate::tiling::TilingMode::Tiled;
        (*window).mode_locked = true;
    } else {
        (*window).tiling_mode = crate::tiling::TilingMode::Floating;
        (*window).mode_locked = true;
    }
    (*window).wm_scheduled.maximize_requested = if maximized {
        crate::window::MaximizeRequest::Maximize
    } else {
        crate::window::MaximizeRequest::Unmaximize
    };
    (*(*window).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_request_fullscreen(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, request_fullscreen);
    let fullscreen = (*(*xwindow).xsurface).fullscreen;
    log::info!(
        "XWayland fullscreen request: title='{}' fullscreen={}",
        (*(*xwindow).window).get_title_string().unwrap_or_default(),
        fullscreen,
    );
    // An exempt game's pixels are logical, so the compositor's fullscreen
    // is a 1920x1200 X window on a 3840x2400 root. Wine syncs
    // _NET_WM_STATE from its own idea of the screen and withdraws
    // FULLSCREEN the moment it sees a window that does not cover its
    // root — every Fullscreen press on Trackmania was undone by this
    // request within the frame. The user's fullscreen stands; the key that
    // set it clears it.
    if !fullscreen
        && (*(*xwindow).window).is_fullscreen()
        && window_is_hidpi_exempt((*xwindow).window)
    {
        log::info!("XWayland fullscreen request: ignored — the window is hidpi-exempt and fullscreen by the compositor");
        return;
    }
    (*(*xwindow).window).wm_scheduled.fullscreen_requested = if fullscreen {
        crate::window::FullscreenRequest::Fullscreen(std::ptr::null_mut())
    } else {
        crate::window::FullscreenRequest::Exit
    };
    (*(*(*xwindow).window).server).wm.dirty_windowing();
    (*(*(*xwindow).window).server).wm.apply_client_fullscreen((*xwindow).window, fullscreen);
}

unsafe extern "C" fn handle_request_minimize(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let xwindow = crate::container_of!(listener, XwaylandWindow, request_minimize);
    let event = data as *mut ffi::wlr_xwayland_minimize_event;
    ffi::wlr_xwayland_surface_set_minimized((*xwindow).xsurface, (*event).minimize);
    (*(*xwindow).window).wm_scheduled.minimize_requested = true;
    (*(*(*xwindow).window).server).wm.dirty_windowing();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn scale_from_live_output_is_remembered() {
        let last = AtomicU32::new(0);
        assert_eq!(resolve_x11_scale(Some(2.0), &last), 2.0);
        // Output gone (suspend): the remembered scale stands in, not 1.
        assert_eq!(resolve_x11_scale(None, &last), 2.0);
        // A new output with another scale takes over and is remembered.
        assert_eq!(resolve_x11_scale(Some(1.5), &last), 1.5);
        assert_eq!(resolve_x11_scale(None, &last), 1.5);
    }

    fn pats(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn exempt_matches_class_instance_or_title() {
        // Proton: every window is class steam_proton, so the game is told
        // apart by its instance (the exe) or its title.
        let p = pats(&["Trackmania"]);
        assert!(hidpi_exempt(&p, "steam_proton", "trackmania.exe", "Trackmania"));
        assert!(hidpi_exempt(&p, "steam_proton", "", "Trackmania"));
        assert!(hidpi_exempt(&p, "Trackmania", "", ""));
        assert!(!hidpi_exempt(&p, "steam_proton", "upc.exe", "Ubisoft Connect"));
        // Wildcards and case follow app_id_matches.
        let p = pats(&["trackmania*"]);
        assert!(hidpi_exempt(&p, "steam_proton", "Trackmania.exe", ""));
        assert!(!hidpi_exempt(&p, "steam_proton", "", "My Trackmania"));
    }

    #[test]
    fn exempt_ignores_empty_fields_and_lists() {
        assert!(!hidpi_exempt(&[], "steam_proton", "trackmania.exe", "Trackmania"));
        // An empty field must not match a pattern that is itself empty-ish.
        assert!(!hidpi_exempt(&pats(&["*"]), "", "", ""));
        assert!(hidpi_exempt(&pats(&["*"]), "x", "", ""));
    }

    #[test]
    fn scale_before_any_output_is_one() {
        let last = AtomicU32::new(0);
        assert_eq!(resolve_x11_scale(None, &last), 1.0);
    }

    #[test]
    fn x11_round_trip_holds_at_remembered_scale() {
        // The suspend case: physical 3712 read back while no output exists
        // must come back as logical 1856, and go out again as 3712.
        let last = AtomicU32::new(0);
        let _ = resolve_x11_scale(Some(2.0), &last);
        let s = resolve_x11_scale(None, &last);
        let logical = from_x11(3712, s);
        assert_eq!(logical, 1856);
        assert_eq!(to_x11(logical, resolve_x11_scale(Some(2.0), &last)), 3712);
    }

    #[test]
    fn snap_x11_is_what_the_logical_grid_can_express() {
        // An odd X11 coordinate at scale 2 has no logical integer, so it
        // moves by one; the point is that it then STAYS there. Granting the
        // raw value instead is what let Houdini's dialog gain a pixel per
        // request: 181 -> 91 -> 182 -> 91 -> 182 ...
        assert_eq!(from_x11(181, 2.0), 91);
        assert_eq!(to_x11(91, 2.0), 182);
        assert_eq!(snap_x11(181, 2.0), 182);

        // Idempotent: snapping a snapped value is a no-op, which is what
        // makes repeated configures converge instead of drifting.
        for x in [-91, -90, -1, 0, 1, 180, 181, 757, 1300, 3712] {
            let once = snap_x11(x, 2.0);
            assert_eq!(snap_x11(once, 2.0), once, "not idempotent at x={x}");
        }

        // Even values — and every value at scale 1 — are untouched, so
        // nothing outside xwayland_hidpi changes.
        for x in [-90, 0, 180, 720, 1360, 3712] {
            assert_eq!(snap_x11(x, 2.0), x, "even value moved at x={x}");
        }
        for x in [-91, -1, 0, 1, 181, 757, 1301] {
            assert_eq!(snap_x11(x, 1.0), x, "scale 1 moved at x={x}");
        }
    }

    #[test]
    fn configure_is_sent_until_it_has_actually_been_sent_once() {
        let g = |x, y, w, h| X11Geom { x, y, width: w, height: h };
        let fullscreen = g(0, 0, 1280, 720);
        // The restore case: the mirror already says 1280x720 (pre-written by
        // try_restore) but nothing was ever sent — X is at its natural size.
        assert!(needs_configure(fullscreen, fullscreen, None));
        // Something else was sent (the client's own pre-map request).
        assert!(needs_configure(fullscreen, fullscreen, Some(g(0, 0, 103, 36))));
        // Sent once and X reports it: nothing to do.
        assert!(!needs_configure(fullscreen, fullscreen, Some(fullscreen)));
        // X moved or resized itself since: the mirror disagrees, resend.
        assert!(needs_configure(fullscreen, g(10, 10, 1280, 720), Some(fullscreen)));
        assert!(needs_configure(fullscreen, g(0, 0, 640, 360), Some(fullscreen)));
        // A different wanted geometry always goes out.
        assert!(needs_configure(g(0, 0, 640, 360), fullscreen, Some(fullscreen)));
    }

    #[test]
    fn output_change_grace_begins_and_is_re_armed() {
        note_output_change();
        assert!(in_output_change_grace());
        // Expire it by hand, then a second change re-arms it.
        *OUTPUT_CHANGE_GRACE_UNTIL.lock().unwrap() =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        assert!(!in_output_change_grace());
        note_output_change();
        assert!(in_output_change_grace());
    }
}
