// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use std::ptr;

use crate::window_manager::WindowManager;
use crate::xkb_bindings::XkbBindings;
use crate::layer_shell::LayerShell;
use crate::scene::Scene;
use crate::output_manager::OutputManager;
use crate::input_manager::InputManager;
use crate::libinput_config::LibinputConfig;
use crate::xkb_config::XkbConfig;
use crate::idle_inhibit_manager::IdleInhibitManager;
use crate::lock_manager::LockManager;

// Activation-attention notifications ("<app> needs attention / has requested
// activation") are suppressed during the compositor's startup sequence: session
// restore respawns every previously-open window, and each one issues an
// xdg-activation request as it maps, which would otherwise spray a burst of
// "needs attention" popups the moment you log in. `begin_startup_activation_grace`
// is called once when startup begins; until the deadline passes, the handler
// swallows the notification. Outside the startup path the cell is never set, so
// activation notifications behave normally.
static STARTUP_ACTIVATION_GRACE_UNTIL: std::sync::OnceLock<std::time::Instant> =
    std::sync::OnceLock::new();

/// How long after startup begins to keep suppressing activation notifications.
/// Generous enough to cover restored heavyweight apps (e.g. a browser) that take
/// several seconds to launch and request focus.
const STARTUP_ACTIVATION_GRACE: std::time::Duration = std::time::Duration::from_secs(8);

/// Begin the startup grace window during which activation-attention
/// notifications are suppressed. Idempotent — only the first call takes effect.
pub fn begin_startup_activation_grace() {
    let _ = STARTUP_ACTIVATION_GRACE_UNTIL.set(std::time::Instant::now() + STARTUP_ACTIVATION_GRACE);
}

/// True while we're still inside the post-startup grace window.
fn in_startup_activation_grace() -> bool {
    STARTUP_ACTIVATION_GRACE_UNTIL
        .get()
        .is_some_and(|deadline| std::time::Instant::now() < *deadline)
}

// Helper macro equivalent to @fieldParentPtr in Zig
#[macro_export]
macro_rules! container_of {
    ($ptr:expr, $container:path, $field:ident) => {{
        let offset = {
            let dummy = std::mem::MaybeUninit::<$container>::uninit();
            let dummy_ptr = dummy.as_ptr();
            let field_ptr = std::ptr::addr_of!((*dummy_ptr).$field);
            (field_ptr as usize).wrapping_sub(dummy_ptr as usize)
        };
        ((($ptr as *const _) as usize).wrapping_sub(offset)) as *mut $container
    }};
}

// Custom non-opaque layouts for FFI casting
#[repr(C)]
pub struct WlList {
    pub prev: *mut WlList,
    pub next: *mut WlList,
}

#[repr(C)]
pub struct WlListener {
    pub link: WlList,
    pub notify: Option<unsafe extern "C" fn(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void)>,
}

#[repr(C)]
pub struct WlrRendererEvents {
    pub destroy: ffi::wl_signal,
    pub lost: ffi::wl_signal,
}

#[repr(C)]
pub struct WlrRendererFeatures {
    pub output_color_transform: bool,
    pub timeline: bool,
}

#[repr(C)]
pub struct WlrRenderer {
    pub render_buffer_caps: u32,
    pub events: WlrRendererEvents,
    pub features: WlrRendererFeatures,
}

#[repr(C)]
pub struct WlrBackendFeatures {
    pub timeline: bool,
}

#[repr(C)]
pub struct WlrBackendEvents {
    pub destroy: ffi::wl_signal,
    pub new_input: ffi::wl_signal,
    pub new_output: ffi::wl_signal,
}

#[repr(C)]
pub struct WlrBackend {
    pub impl_: *const std::ffi::c_void,
    pub buffer_caps: u32,
    pub features: WlrBackendFeatures,
    pub events: WlrBackendEvents,
}

#[repr(C)]
pub struct WlrXdgShellEvents {
    pub new_surface: ffi::wl_signal,
    pub new_toplevel: ffi::wl_signal,
    pub new_popup: ffi::wl_signal,
    pub destroy: ffi::wl_signal,
}

#[repr(C)]
pub struct WlrXdgShell {
    pub global: *mut ffi::wl_global,
    pub version: u32,
    pub clients: ffi::wl_list,
    pub popup_grabs: ffi::wl_list,
    pub ping_timeout: u32,
    pub events: WlrXdgShellEvents,
}

#[repr(C)]
pub struct WlrXdgDecorationManagerV1Events {
    pub new_toplevel_decoration: ffi::wl_signal,
    pub destroy: ffi::wl_signal,
}

#[repr(C)]
pub struct WlrXdgDecorationManagerV1 {
    pub global: *mut ffi::wl_global,
    pub decorations: ffi::wl_list,
    pub events: WlrXdgDecorationManagerV1Events,
}

#[repr(C)]
pub struct WlrXdgActivationV1Events {
    pub destroy: ffi::wl_signal,
    pub request_activate: ffi::wl_signal,
    pub new_token: ffi::wl_signal,
}

#[repr(C)]
pub struct WlrXdgActivationV1 {
    pub global: *mut ffi::wl_global,
    pub token_timeout_msec: u32,
    pub tokens: ffi::wl_list,
    pub events: WlrXdgActivationV1Events,
}

#[repr(C)]
pub struct WlrCursorShapeManagerV1Events {
    pub request_set_shape: ffi::wl_signal,
    pub destroy: ffi::wl_signal,
}

#[repr(C)]
pub struct WlrCursorShapeManagerV1 {
    pub global: *mut ffi::wl_global,
    pub events: WlrCursorShapeManagerV1Events,
}

#[repr(C)]
pub struct WlrExtForeignToplevelImageCaptureSourceManagerV1Events {
    pub destroy: ffi::wl_signal,
    pub new_request: ffi::wl_signal,
}

#[repr(C)]
pub struct WlrExtForeignToplevelImageCaptureSourceManagerV1 {
    pub global: *mut ffi::wl_global,
    pub events: WlrExtForeignToplevelImageCaptureSourceManagerV1Events,
}

#[repr(C)]
pub struct WlrXwaylandEvents {
    pub destroy: ffi::wl_signal,
    pub ready: ffi::wl_signal,
    pub new_surface: ffi::wl_signal,
    pub remove_startup_info: ffi::wl_signal,
}

#[repr(C)]
pub struct WlrXwayland {
    pub server: *mut std::ffi::c_void,
    pub own_server: bool,
    pub xwm: *mut std::ffi::c_void,
    pub shell_v1: *mut std::ffi::c_void,
    pub display_name: *const std::os::raw::c_char,
    pub wl_display: *mut ffi::wl_display,
    pub compositor: *mut ffi::wlr_compositor,
    pub seat: *mut std::ffi::c_void,
    pub events: WlrXwaylandEvents,
}

// Wayland list manipulation utilities
pub unsafe fn wl_list_insert(list: *mut WlList, elm: *mut WlList) {
    if list.is_null() {
        log::error!("wl_list_insert: list is null!");
        return;
    }
    if elm.is_null() {
        log::error!("wl_list_insert: elm is null!");
        return;
    }
    (*elm).prev = list;
    (*elm).next = (*list).next;
    (*(*list).next).prev = elm;
    (*list).next = elm;
}

pub unsafe fn wl_list_remove(elm: *mut WlList) {
    (*(*elm).next).prev = (*elm).prev;
    (*(*elm).prev).next = (*elm).next;
}

pub unsafe fn wl_list_remove_and_reinit(elm: *mut WlList) {
    wl_list_remove(elm);
    (*elm).prev = elm;
    (*elm).next = elm;
}

pub unsafe fn wl_signal_add(signal: *mut ffi::wl_signal, listener: *mut ffi::wl_listener) {
    log::info!("wl_signal_add: signal={:?}, listener={:?}", signal, listener);
    if signal.is_null() {
        log::error!("wl_signal_add: signal is null!");
        return;
    }
    let sig_list = &mut (*signal).listener_list as *mut ffi::wl_list as *mut WlList;
    log::info!("wl_signal_add: sig_list={:?}, prev={:?}, next={:?}", sig_list, (*sig_list).prev, (*sig_list).next);
    let listener_custom = listener as *mut WlListener;
    wl_list_insert((*sig_list).prev, &mut (*listener_custom).link);
}

pub unsafe fn wl_listener_remove(listener: *mut ffi::wl_listener) {
    let listener_custom = listener as *mut WlListener;
    wl_list_remove(&mut (*listener_custom).link);
}


pub struct Server {
    pub wl_server: *mut ffi::wl_display,
    pub sigint_source: *mut ffi::wl_event_source,
    pub sigterm_source: *mut ffi::wl_event_source,
    pub sigchld_source: *mut ffi::wl_event_source,
    // pub fixes: *mut ffi::wlr_fixes,
    pub backend: *mut ffi::wlr_backend,
    pub session: *mut ffi::wlr_session,
    pub renderer: *mut ffi::wlr_renderer,
    pub allocator: *mut ffi::wlr_allocator,
    pub gpu_reset_recover: *mut ffi::wl_event_source,
    pub security_context_manager: *mut ffi::wlr_security_context_manager_v1,
    pub shm: *mut ffi::wlr_shm,
    pub linux_dmabuf: *mut ffi::wlr_linux_dmabuf_v1,
    pub linux_drm_syncobj_manager: *mut ffi::wlr_linux_drm_syncobj_manager_v1,
    pub single_pixel_buffer_manager: *mut ffi::wlr_single_pixel_buffer_manager_v1,
    pub alpha_modifier: *mut ffi::wlr_alpha_modifier_v1,
    pub color_manager: *mut ffi::wlr_color_manager_v1,
    // pub color_representation_manager: *mut ffi::wlr_color_representation_manager_v1,
    pub viewporter: *mut ffi::wlr_viewporter,
    pub fractional_scale_manager: *mut ffi::wlr_fractional_scale_manager_v1,
    pub compositor: *mut ffi::wlr_compositor,
    pub subcompositor: *mut ffi::wlr_subcompositor,
    pub cursor_shape_manager: *mut ffi::wlr_cursor_shape_manager_v1,
    pub xdg_shell: *mut ffi::wlr_xdg_shell,
    pub xdg_decoration_manager: *mut ffi::wlr_xdg_decoration_manager_v1,
    pub xdg_activation: *mut ffi::wlr_xdg_activation_v1,
    pub xdg_foreign_registry: *mut ffi::wlr_xdg_foreign_registry,
    pub xdg_foreign_v2: *mut ffi::wlr_xdg_foreign_v2,
    pub data_device_manager: *mut ffi::wlr_data_device_manager,
    pub primary_selection_manager: *mut ffi::wlr_primary_selection_v1_device_manager,
    pub data_control_manager: *mut ffi::wlr_ext_data_control_manager_v1,
    pub wlr_data_control_manager: *mut ffi::wlr_data_control_manager_v1,
    pub export_dmabuf_manager: *mut ffi::wlr_export_dmabuf_manager_v1,
    pub screencopy_manager: *mut ffi::wlr_screencopy_manager_v1,
    pub image_copy_capture_manager: *mut ffi::wlr_ext_image_copy_capture_manager_v1,
    pub output_image_capture_source_manager: *mut ffi::wlr_ext_output_image_capture_source_manager_v1,
    pub wlr_foreign_toplevel_manager: *mut ffi::wlr_foreign_toplevel_manager_v1,
    pub foreign_toplevel_list: *mut ffi::wlr_ext_foreign_toplevel_list_v1,
    // pub toplevel_capture_source_manager: *mut ffi::wlr_ext_foreign_toplevel_image_capture_source_manager_v1,
    pub tearing_control_manager: *mut ffi::wlr_tearing_control_manager_v1,

    pub xwayland: *mut ffi::wlr_xwayland,

    // Subcomponents
    pub wm: WindowManager,
    pub xkb_bindings: XkbBindings,
    pub layer_shell: LayerShell,
    pub scene: Scene,
    pub om: OutputManager,
    pub input_manager: InputManager,
    pub libinput_config: LibinputConfig,
    pub xkb_config: XkbConfig,
    pub idle_inhibit_manager: IdleInhibitManager,
    pub lock_manager: LockManager,
    pub inspector: crate::inspector::Inspector,
    pub cce_window_management: crate::cce_window_management::CceWindowManagement,

    // Event listeners
    pub renderer_lost: ffi::wl_listener,
    pub new_xdg_toplevel: ffi::wl_listener,
    pub new_toplevel_decoration: ffi::wl_listener,
    pub request_activate: ffi::wl_listener,
    pub request_set_cursor_shape: ffi::wl_listener,
    // pub toplevel_capture_request: ffi::wl_listener,
    pub new_xsurface: ffi::wl_listener,
    pub xwayland_ready: ffi::wl_listener,
}

unsafe extern "C" fn terminate(_signum: std::os::raw::c_int, data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wl_server = data as *mut ffi::wl_display;
    ffi::wl_display_terminate(wl_server);
    0
}

unsafe extern "C" fn handle_sigchld(_signum: std::os::raw::c_int, _data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let mut status = 0;
    while libc::waitpid(-1, &mut status, libc::WNOHANG) > 0 {}
    0
}

unsafe extern "C" fn handle_renderer_lost(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let _server = container_of!(listener, Server, renderer_lost);
    log::info!("received GPU reset event");
}

unsafe extern "C" fn handle_new_xdg_toplevel(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let server = container_of!(listener, Server, new_xdg_toplevel);
    let xdg_toplevel = data as *mut ffi::wlr_xdg_toplevel;
    log::info!("new xdg toplevel surface");
    if let Err(e) = crate::xdg_toplevel::XdgToplevel::create(xdg_toplevel, server) {
        log::error!("Failed to create xdg toplevel: {}", e);
        let client = ffi::wl_resource_get_client((*xdg_toplevel).resource);
        ffi::wl_client_post_no_memory(client);
    }
}

unsafe extern "C" fn handle_new_toplevel_decoration(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let _server = container_of!(listener, Server, new_toplevel_decoration);
    let decoration = data as *mut ffi::wlr_xdg_toplevel_decoration_v1;
    log::info!("new toplevel decoration");
    crate::xdg_toplevel::XdgDecoration::init(decoration);
}

unsafe extern "C" fn handle_request_activate(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let server = container_of!(listener, Server, request_activate);
    log::info!("xdg activation request received");
    if data.is_null() {
        return;
    }
    let event = data as *mut ffi::wlr_xdg_activation_v1_request_activate_event;
    let surface = (*event).surface;
    if surface.is_null() {
        return;
    }

    let default_seat = (*server).input_manager.default_seat;
    if !default_seat.is_null() {
        let focused_surf = ffi::river_wlr_seat_get_keyboard_focused_surface((*default_seat).wlr_seat);
        if focused_surf == surface {
            log::info!("xdg activation request ignored: window is already focused");
            return;
        }
    }

    for &win_ptr in (*server).wm.windows.iter() {
        if !win_ptr.is_null() && (*win_ptr).root_surface() == surface {
            let title_ptr = (*win_ptr).get_title();
            let title = if title_ptr.is_null() {
                "Window".to_string()
            } else {
                std::ffi::CStr::from_ptr(title_ptr).to_string_lossy().into_owned()
            };
            
            let app_id_ptr = (*win_ptr).get_app_id();
            let app_id = if app_id_ptr.is_null() {
                "unknown".to_string()
            } else {
                std::ffi::CStr::from_ptr(app_id_ptr).to_string_lossy().into_owned()
            };
            
            log::info!("xdg activation request for window '{}' ({})", title, app_id);

            if in_startup_activation_grace() {
                log::info!(
                    "activation notification suppressed during startup grace for '{}' ({})",
                    title,
                    app_id
                );
                break;
            }

            let uid = unsafe { libc::getuid() };
            let bus_address = format!("unix:path=/run/user/{}/bus", uid);
            
            std::process::Command::new("gdbus")
                .env("DBUS_SESSION_BUS_ADDRESS", &bus_address)
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.kde.StatusNotifierWatcher",
                    "--object-path",
                    "/StatusInterface",
                    "--method",
                    "org.clear.StatusInterface.NotifyAttention",
                    &app_id,
                    &title,
                ])
                .spawn()
                .ok();
            break;
        }
    }
}

unsafe extern "C" fn handle_request_set_cursor_shape(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let server = container_of!(listener, Server, request_set_cursor_shape);
    let event = data as *mut ffi::wlr_cursor_shape_manager_v1_request_set_shape_event;
    
    let wlr_seat = (*(*event).seat_client).seat;
    let focused_client = ffi::river_wlr_seat_get_pointer_focused_client(wlr_seat);
    
    let event_client = ffi::river_wlr_seat_client_get_client((*event).seat_client);
    let wm_client = if !(*server).wm.object.is_null() {
        ffi::wl_resource_get_client((*server).wm.object)
    } else {
        std::ptr::null_mut()
    };
    let is_wm = !wm_client.is_null() && event_client == wm_client;

    let shape_name = ffi::wlr_cursor_shape_v1_name((*event).shape);
    let shape_str = if shape_name.is_null() {
        "unknown".to_string()
    } else {
        std::ffi::CStr::from_ptr(shape_name).to_string_lossy().into_owned()
    };

    log::debug!(
        "set_cursor_shape: event_client={:?}, wm_client={:?}, is_wm={}, focused={:?}, shape={}",
        event_client,
        wm_client,
        is_wm,
        focused_client,
        shape_str
    );

    if focused_client == (*event).seat_client || is_wm {
        let seat = ffi::river_wlr_seat_get_data(wlr_seat) as *mut crate::seat::Seat;
        if !seat.is_null() {
            (*seat).cursor.set_xcursor(shape_name);
        }
    }
}

// unsafe extern "C" fn handle_toplevel_capture_request(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
//     let _server = container_of!(listener, Server, toplevel_capture_request);
//     log::info!("toplevel capture request");
// }

unsafe extern "C" fn handle_new_xwayland_surface(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let server = container_of!(listener, Server, new_xsurface);
    let xsurface = data as *mut ffi::wlr_xwayland_surface;
    log::info!("new xwayland surface");

    if (*xsurface).override_redirect {
        if let Err(e) = crate::xwayland_override_redirect::XwaylandOverrideRedirect::create(xsurface, server) {
            log::error!("Failed to create xwayland override redirect surface: {}", e);
        }
    } else {
        if let Err(e) = crate::xwayland_window::XwaylandWindow::create(xsurface, server) {
            log::error!("Failed to create xwayland window surface: {}", e);
        }
    }
}

unsafe extern "C" fn handle_xwayland_ready(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let server = container_of!(listener, Server, xwayland_ready);
    let xwayland_cast = (*server).xwayland as *mut WlrXwayland;
    if !xwayland_cast.is_null() && !(*xwayland_cast).display_name.is_null() {
        let display_name = std::ffi::CStr::from_ptr((*xwayland_cast).display_name)
            .to_string_lossy()
            .into_owned();
        log::info!("Xwayland is ready on display {}", display_name);
        std::env::set_var("DISPLAY", &display_name);
    }
}

impl Server {
    pub fn init(&mut self, runtime_xwayland: bool) -> Result<(), &'static str> {
        unsafe {
            let wl_server = ffi::wl_display_create();
            if wl_server.is_null() {
                return Err("Failed to create wayland server");
            }
            self.wl_server = wl_server;

            let loop_ = ffi::wl_display_get_event_loop(wl_server);
            if loop_.is_null() {
                return Err("Failed to get event loop");
            }

            self.sigint_source = ffi::wl_event_loop_add_signal(loop_, libc::SIGINT, Some(terminate), wl_server as *mut _);
            self.sigterm_source = ffi::wl_event_loop_add_signal(loop_, libc::SIGTERM, Some(terminate), wl_server as *mut _);
            self.sigchld_source = ffi::wl_event_loop_add_signal(loop_, libc::SIGCHLD, Some(handle_sigchld), wl_server as *mut _);

            let mut session: *mut ffi::wlr_session = ptr::null_mut();
            let backend = ffi::wlr_backend_autocreate(loop_, &mut session);
            if backend.is_null() {
                return Err("Failed to autocreate wlr_backend");
            }
            self.backend = backend;
            self.session = session;

            let renderer = ffi::fx_renderer_create(backend);
            if renderer.is_null() {
                return Err("Failed to create fx_renderer");
            }
            self.renderer = renderer;

            let compositor = ffi::wlr_compositor_create(wl_server, 6, renderer);
            if compositor.is_null() {
                return Err("Failed to create wlr_compositor");
            }
            self.compositor = compositor;

            let xdg_foreign_registry = ffi::wlr_xdg_foreign_registry_create(wl_server);
            if xdg_foreign_registry.is_null() {
                return Err("Failed to create xdg_foreign_registry");
            }
            self.xdg_foreign_registry = xdg_foreign_registry;

            // let fixes = ffi::wlr_fixes_create(wl_server, 1);
            // if fixes.is_null() {
            //     return Err("Failed to create wlr_fixes");
            // }
            // self.fixes = fixes;

            let allocator = ffi::wlr_allocator_autocreate(backend, renderer);
            if allocator.is_null() {
                return Err("Failed to autocreate allocator");
            }
            self.allocator = allocator;

            let security_context_manager = ffi::wlr_security_context_manager_v1_create(wl_server);
            if security_context_manager.is_null() {
                return Err("Failed to create security context manager");
            }
            self.security_context_manager = security_context_manager;

            let shm = ffi::wlr_shm_create_with_renderer(wl_server, 2, renderer);
            if shm.is_null() {
                return Err("Failed to create shm");
            }
            self.shm = shm;

            let single_pixel_buffer_manager = ffi::wlr_single_pixel_buffer_manager_v1_create(wl_server);
            if single_pixel_buffer_manager.is_null() {
                return Err("Failed to create single pixel buffer manager");
            }
            self.single_pixel_buffer_manager = single_pixel_buffer_manager;

            let alpha_modifier = ffi::wlr_alpha_modifier_v1_create(wl_server);
            if alpha_modifier.is_null() {
                return Err("Failed to create alpha modifier");
            }
            self.alpha_modifier = alpha_modifier;

            // let color_representation_manager = ffi::wlr_color_representation_manager_v1_create_with_renderer(wl_server, 1, renderer);
            // if color_representation_manager.is_null() {
            //     return Err("Failed to create color representation manager");
            // }
            // self.color_representation_manager = color_representation_manager;

            let viewporter = ffi::wlr_viewporter_create(wl_server);
            if viewporter.is_null() {
                return Err("Failed to create viewporter");
            }
            self.viewporter = viewporter;

            let fractional_scale_manager = ffi::wlr_fractional_scale_manager_v1_create(wl_server, 1);
            if fractional_scale_manager.is_null() {
                return Err("Failed to create fractional scale manager");
            }
            self.fractional_scale_manager = fractional_scale_manager;

            let subcompositor = ffi::wlr_subcompositor_create(wl_server);
            if subcompositor.is_null() {
                return Err("Failed to create subcompositor");
            }
            self.subcompositor = subcompositor;

            let cursor_shape_manager = ffi::wlr_cursor_shape_manager_v1_create(wl_server, 1);
            if cursor_shape_manager.is_null() {
                return Err("Failed to create cursor shape manager");
            }
            self.cursor_shape_manager = cursor_shape_manager;

            let xdg_shell = ffi::wlr_xdg_shell_create(wl_server, 5);
            if xdg_shell.is_null() {
                return Err("Failed to create xdg shell");
            }
            self.xdg_shell = xdg_shell;

            let xdg_decoration_manager = ffi::wlr_xdg_decoration_manager_v1_create(wl_server);
            if xdg_decoration_manager.is_null() {
                return Err("Failed to create xdg decoration manager");
            }
            self.xdg_decoration_manager = xdg_decoration_manager;

            let xdg_activation = ffi::wlr_xdg_activation_v1_create(wl_server);
            if xdg_activation.is_null() {
                return Err("Failed to create xdg activation");
            }
            self.xdg_activation = xdg_activation;

            let xdg_foreign_v2 = ffi::wlr_xdg_foreign_v2_create(wl_server, xdg_foreign_registry);
            if xdg_foreign_v2.is_null() {
                return Err("Failed to create xdg foreign v2");
            }
            self.xdg_foreign_v2 = xdg_foreign_v2;

            let data_device_manager = ffi::wlr_data_device_manager_create(wl_server);
            if data_device_manager.is_null() {
                return Err("Failed to create data device manager");
            }
            self.data_device_manager = data_device_manager;

            let primary_selection_manager = ffi::wlr_primary_selection_v1_device_manager_create(wl_server);
            if primary_selection_manager.is_null() {
                return Err("Failed to create primary selection manager");
            }
            self.primary_selection_manager = primary_selection_manager;

            let data_control_manager = ffi::wlr_ext_data_control_manager_v1_create(wl_server, 1);
            if data_control_manager.is_null() {
                return Err("Failed to create ext data control manager");
            }
            self.data_control_manager = data_control_manager;

            let wlr_data_control_manager = ffi::wlr_data_control_manager_v1_create(wl_server);
            if wlr_data_control_manager.is_null() {
                return Err("Failed to create data control manager");
            }
            self.wlr_data_control_manager = wlr_data_control_manager;

            let export_dmabuf_manager = ffi::wlr_export_dmabuf_manager_v1_create(wl_server);
            if export_dmabuf_manager.is_null() {
                return Err("Failed to create export dmabuf manager");
            }
            self.export_dmabuf_manager = export_dmabuf_manager;

            let screencopy_manager = ffi::wlr_screencopy_manager_v1_create(wl_server);
            if screencopy_manager.is_null() {
                return Err("Failed to create screencopy manager");
            }
            self.screencopy_manager = screencopy_manager;

            let image_copy_capture_manager = ffi::wlr_ext_image_copy_capture_manager_v1_create(wl_server, 1);
            if image_copy_capture_manager.is_null() {
                return Err("Failed to create image copy capture manager");
            }
            self.image_copy_capture_manager = image_copy_capture_manager;

            let output_image_capture_source_manager = ffi::wlr_ext_output_image_capture_source_manager_v1_create(wl_server, 1);
            if output_image_capture_source_manager.is_null() {
                return Err("Failed to create output image capture source manager");
            }
            self.output_image_capture_source_manager = output_image_capture_source_manager;

            let wlr_foreign_toplevel_manager = ffi::wlr_foreign_toplevel_manager_v1_create(wl_server);
            if wlr_foreign_toplevel_manager.is_null() {
                return Err("Failed to create foreign toplevel manager");
            }
            self.wlr_foreign_toplevel_manager = wlr_foreign_toplevel_manager;

            let foreign_toplevel_list = ffi::wlr_ext_foreign_toplevel_list_v1_create(wl_server, 1);
            if foreign_toplevel_list.is_null() {
                return Err("Failed to create foreign toplevel list");
            }
            self.foreign_toplevel_list = foreign_toplevel_list;

            // let toplevel_capture_source_manager = ffi::wlr_ext_foreign_toplevel_image_capture_source_manager_v1_create(wl_server, 1);
            // if toplevel_capture_source_manager.is_null() {
            //     return Err("Failed to create toplevel capture source manager");
            // }
            // self.toplevel_capture_source_manager = toplevel_capture_source_manager;

            let tearing_control_manager = ffi::wlr_tearing_control_manager_v1_create(wl_server, 1);
            if tearing_control_manager.is_null() {
                return Err("Failed to create tearing control manager");
            }
            self.tearing_control_manager = tearing_control_manager;

            // Setup Xwayland if runtime requested
            if runtime_xwayland {
                let xwayland = ffi::wlr_xwayland_create(wl_server, compositor, false);
                if xwayland.is_null() {
                    return Err("Failed to create xwayland server");
                }
                self.xwayland = xwayland;
            }

            // Setup linux dmabuf if supported
            if !ffi::wlr_renderer_get_texture_formats(renderer, ffi::wlr_buffer_cap_WLR_BUFFER_CAP_DMABUF).is_null() {
                self.linux_dmabuf = ffi::wlr_linux_dmabuf_v1_create_with_renderer(wl_server, 5, renderer);
            }

            // Setup linux drm syncobj if supported
            let renderer_cast = renderer as *mut WlrRenderer;
            let backend_cast = backend as *mut WlrBackend;
            if (*renderer_cast).features.timeline && (*backend_cast).features.timeline {
                let drm_fd = ffi::wlr_renderer_get_drm_fd(renderer);
                if drm_fd >= 0 {
                    self.linux_drm_syncobj_manager = ffi::wlr_linux_drm_syncobj_manager_v1_create(wl_server, 1, drm_fd);
                }
            }

            // Setup color manager if supported
            // (Commented out for wlroots 0.19 compatibility)
            self.color_manager = std::ptr::null_mut();

            // Setup subcomponents stubs
            let server_ptr = self as *mut Server;
            self.wm.init_with_server(server_ptr).map_err(|_| "Failed to init wm")?;
            self.xkb_bindings.init(server_ptr, self.wl_server).map_err(|_| "Failed to init xkb_bindings")?;
            self.layer_shell.init(server_ptr, self.wl_server).map_err(|_| "Failed to init layer_shell")?;
            self.scene.init(self.linux_dmabuf, self.color_manager).map_err(|_| "Failed to init scene")?;
            self.om.init(server_ptr).map_err(|_| "Failed to init om")?;
            self.input_manager.init(server_ptr).map_err(|_| "Failed to init input_manager")?;
            self.libinput_config.init(server_ptr).map_err(|_| "Failed to init libinput_config")?;
            self.xkb_config.init(server_ptr).map_err(|_| "Failed to init xkb_config")?;
            self.idle_inhibit_manager.init(server_ptr).map_err(|_| "Failed to init idle_inhibit_manager")?;
            self.lock_manager.init(server_ptr).map_err(|_| "Failed to init lock_manager")?;
            self.inspector.init(server_ptr).map_err(|_| "Failed to init inspector")?;
            self.cce_window_management.init(server_ptr).map_err(|_| "Failed to init cce_window_management")?;

            // Setup listeners
            let r_lost = &mut self.renderer_lost as *mut ffi::wl_listener as *mut WlListener;
            (*r_lost).notify = Some(handle_renderer_lost);

            let new_xdg = &mut self.new_xdg_toplevel as *mut ffi::wl_listener as *mut WlListener;
            (*new_xdg).notify = Some(handle_new_xdg_toplevel);

            let new_dec = &mut self.new_toplevel_decoration as *mut ffi::wl_listener as *mut WlListener;
            (*new_dec).notify = Some(handle_new_toplevel_decoration);

            let req_act = &mut self.request_activate as *mut ffi::wl_listener as *mut WlListener;
            (*req_act).notify = Some(handle_request_activate);

            let req_cursor = &mut self.request_set_cursor_shape as *mut ffi::wl_listener as *mut WlListener;
            (*req_cursor).notify = Some(handle_request_set_cursor_shape);

            // let cap_req = &mut self.toplevel_capture_request as *mut ffi::wl_listener as *mut WlListener;
            // (*cap_req).notify = Some(handle_toplevel_capture_request);

            let xdg_shell_cast = self.xdg_shell as *mut WlrXdgShell;
            let xdg_decoration_manager_cast = self.xdg_decoration_manager as *mut WlrXdgDecorationManagerV1;
            let xdg_activation_cast = self.xdg_activation as *mut WlrXdgActivationV1;
            let cursor_shape_manager_cast = self.cursor_shape_manager as *mut WlrCursorShapeManagerV1;
            // let toplevel_capture_source_manager_cast = self.toplevel_capture_source_manager as *mut WlrExtForeignToplevelImageCaptureSourceManagerV1;

            wl_signal_add(&mut (*renderer_cast).events.lost, &mut self.renderer_lost);
            wl_signal_add(&mut (*xdg_shell_cast).events.new_toplevel, &mut self.new_xdg_toplevel);
            wl_signal_add(&mut (*xdg_decoration_manager_cast).events.new_toplevel_decoration, &mut self.new_toplevel_decoration);
            wl_signal_add(&mut (*xdg_activation_cast).events.request_activate, &mut self.request_activate);
            wl_signal_add(&mut (*cursor_shape_manager_cast).events.request_set_shape, &mut self.request_set_cursor_shape);
            // wl_signal_add(&mut (*toplevel_capture_source_manager_cast).events.new_request, &mut self.toplevel_capture_request);

            // Register Xwayland surface listener if active
            if !self.xwayland.is_null() {
                let new_x = &mut self.new_xsurface as *mut ffi::wl_listener as *mut WlListener;
                (*new_x).notify = Some(handle_new_xwayland_surface);

                let xwayland_cast = self.xwayland as *mut WlrXwayland;
                wl_signal_add(&mut (*xwayland_cast).events.new_surface, &mut self.new_xsurface);

                let ready_x = &mut self.xwayland_ready as *mut ffi::wl_listener as *mut WlListener;
                (*ready_x).notify = Some(handle_xwayland_ready);
                wl_signal_add(&mut (*xwayland_cast).events.ready, &mut self.xwayland_ready);
            }
        }

        Ok(())
    }

    pub fn deinit(&mut self) {
        unsafe {
            log::info!("[deinit] Server::deinit started");
            // 1. Terminate all client connections first
            log::info!("[deinit] wl_display_destroy_clients started");
            ffi::wl_display_destroy_clients(self.wl_server);
            log::info!("[deinit] wl_display_destroy_clients finished");

            // 2. Deinitialize subcomponents while backend, renderer, allocator, and display are valid
            log::info!("[deinit] self.om.deinit started");
            self.om.deinit();
            log::info!("[deinit] self.om.deinit finished");

            log::info!("[deinit] self.input_manager.deinit started");
            self.input_manager.deinit();
            log::info!("[deinit] self.input_manager.deinit finished");

            log::info!("[deinit] deinitializing other subcomponents");
            self.idle_inhibit_manager.deinit();
            self.lock_manager.deinit();
            self.layer_shell.deinit();
            self.inspector.deinit();
            self.xkb_bindings.deinit();
            self.libinput_config.deinit();
            self.xkb_config.deinit();
            log::info!("[deinit] other subcomponents deinitialized");

            // 3. Remove signal listeners registered directly by the server
            log::info!("[deinit] removing server listeners");
            ffi::wl_event_source_remove(self.sigint_source);
            ffi::wl_event_source_remove(self.sigterm_source);
            if !self.sigchld_source.is_null() {
                ffi::wl_event_source_remove(self.sigchld_source);
            }

            wl_listener_remove(&mut self.renderer_lost);
            wl_listener_remove(&mut self.new_xdg_toplevel);
            wl_listener_remove(&mut self.new_toplevel_decoration);
            wl_listener_remove(&mut self.request_activate);
            wl_listener_remove(&mut self.request_set_cursor_shape);

            // 4. Destroy Xwayland if active
            if !self.xwayland.is_null() {
                wl_listener_remove(&mut self.new_xsurface);
                wl_listener_remove(&mut self.xwayland_ready);
                ffi::wlr_xwayland_destroy(self.xwayland);
            }
            log::info!("[deinit] server listeners removed");

            // 5. Destroy wlroots core hardware interfaces
            log::info!("[deinit] destroying backend");
            ffi::wlr_backend_destroy(self.backend);
            if !self.session.is_null() {
                log::info!("[deinit] destroying session");
                ffi::wlr_session_destroy(self.session);
            }
            log::info!("[deinit] destroying renderer");
            ffi::wlr_renderer_destroy(self.renderer);
            log::info!("[deinit] destroying allocator");
            ffi::wlr_allocator_destroy(self.allocator);

            // 6. Finally, destroy the display
            log::info!("[deinit] destroying display");
            ffi::wl_display_destroy(self.wl_server);
            log::info!("[deinit] Server::deinit finished successfully");
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        let mut server = std::mem::MaybeUninit::<Server>::uninit();
        unsafe {
            // Zero-initialize the memory (C structures and primitive fields)
            std::ptr::write_bytes(server.as_mut_ptr(), 0, 1);
            // Overwrite collections and SlotMap with valid instances to avoid UB/segfaults from null pointers
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.windows, crate::slotmap::SlotMap::new());
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.focus_history, Vec::new());
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.mode_rules, Vec::new());
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.keybinds, Vec::new());
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.pointer_binds, Vec::new());
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.gesture_binds, Vec::new());
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.ipc_rx, None);
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.startup, Vec::new());
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.startup_pids, Vec::new());
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.status_sender, None);
            std::ptr::write(&mut (*server.as_mut_ptr()).wm.last_saved_state_json, None);
            std::ptr::write(&mut (*server.as_mut_ptr()).layer_shell.surfaces, crate::slotmap::SlotMap::new());
            std::ptr::write(&mut (*server.as_mut_ptr()).inspector, crate::inspector::Inspector::new());
            std::ptr::write(&mut (*server.as_mut_ptr()).cce_window_management, crate::cce_window_management::CceWindowManagement::new());
            server.assume_init()
        }
    }
}
