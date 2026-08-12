use crate::ffi;
use crate::server::Server;
use crate::window::Window;
use crate::slotmap::Key as SlotMapKey;

pub struct CceWindowManagement {
    pub server: *mut Server,
    pub toplevels: Vec<*mut ffi::wl_resource>,
}

impl CceWindowManagement {
    pub fn new() -> Self {
        Self {
            server: std::ptr::null_mut(),
            toplevels: Vec::new(),
        }
    }

    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        self.toplevels = Vec::new();
        log::info!("CceWindowManagement initialized successfully");
        Ok(())
    }
}

struct CceToplevelData {
    server: *mut Server,
    window_key: SlotMapKey,
    _resource: *mut ffi::wl_resource,
}

pub unsafe extern "C" fn cce_wm_get_cce_toplevel(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    surface_resource: *mut ffi::wl_resource,
) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut crate::window_manager::WindowManager;
    if wm.is_null() {
        return;
    }
    let server = (*wm).server;
    if server.is_null() {
        return;
    }
    let cce_wm = &mut (*server).cce_window_management as *mut CceWindowManagement;

    let surface = ffi::wlr_surface_from_resource(surface_resource);
    if surface.is_null() {
        log::error!("cce_wm_get_cce_toplevel: surface is null");
        return;
    }

    // Find the Window corresponding to the surface
    let mut target_window = std::ptr::null_mut();
    for &window in (*server).wm.windows.iter() {
        if !window.is_null() && (*window).root_surface() == surface {
            target_window = window;
            break;
        }
    }

    if target_window.is_null() {
        log::error!("cce_wm_get_cce_toplevel: no Window structure found for surface");
        return;
    }

    let version = ffi::wl_resource_get_version(resource);
    let toplevel_res = ffi::wl_resource_create(client, &ffi::zcce_toplevel_v1_interface, version, id);
    if toplevel_res.is_null() {
        ffi::wl_client_post_no_memory(client);
        return;
    }

    let data = Box::into_raw(Box::new(CceToplevelData {
        server,
        window_key: (*target_window).ref_key,
        _resource: toplevel_res,
    }));

    ffi::wl_resource_set_implementation(
        toplevel_res,
        &CCE_TOPLEVEL_INTERFACE as *const _ as *const _,
        data as *mut _,
        Some(handle_destroy_toplevel_resource),
    );

    (*cce_wm).toplevels.push(toplevel_res);

    // Initial event: send current floating state
    let state = if (*target_window).tiling_mode == crate::tiling::TilingMode::Floating { 1 } else { 0 };
    // Event floating_state has index 0
    ffi::wl_resource_post_event(toplevel_res, 0, state as u32);
}

unsafe extern "C" fn handle_destroy_toplevel_resource(resource: *mut ffi::wl_resource) {
    let data_ptr = ffi::wl_resource_get_user_data(resource) as *mut CceToplevelData;
    if !data_ptr.is_null() {
        let _ = Box::from_raw(data_ptr);
    }
}

unsafe extern "C" fn toplevel_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn toplevel_set_floating(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let data = ffi::wl_resource_get_user_data(resource) as *mut CceToplevelData;
    if data.is_null() {
        return;
    }
    let server = (*data).server;
    let window_key = (*data).window_key;
    if let Some(window) = resolve_window(server, window_key) {
        (*window).tiling_mode = crate::tiling::TilingMode::Floating;
        (*window).mode_locked = true;
        if let Some(seat) = (*server).wm.first_seat() {
            if (*seat).focused == crate::seat::Focus::Window(window) {
                (*server).wm.raise_window(window);
            }
        }
        (*server).wm.dirty_windowing();

        // Send floating_state(1)
        ffi::wl_resource_post_event(resource, 0, 1u32);
    }
}

unsafe extern "C" fn toplevel_unset_floating(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let data = ffi::wl_resource_get_user_data(resource) as *mut CceToplevelData;
    if data.is_null() {
        return;
    }
    let server = (*data).server;
    let window_key = (*data).window_key;
    if let Some(window) = resolve_window(server, window_key) {
        (*window).mode_locked = false;
        (*server).wm.dirty_windowing();

        // Send floating_state(0)
        ffi::wl_resource_post_event(resource, 0, 0u32);
    }
}

unsafe extern "C" fn toplevel_set_maximized(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let data = ffi::wl_resource_get_user_data(resource) as *mut CceToplevelData;
    if data.is_null() {
        return;
    }
    let server = (*data).server;
    let window_key = (*data).window_key;
    if let Some(window) = resolve_window(server, window_key) {
        (*window).tiling_mode = crate::tiling::TilingMode::Tiled;
        (*window).mode_locked = true;
        (*window).wm_scheduled.maximize_requested = crate::window::MaximizeRequest::Maximize;
        (*server).wm.dirty_windowing();
    }
}

unsafe extern "C" fn toplevel_unset_maximized(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let data = ffi::wl_resource_get_user_data(resource) as *mut CceToplevelData;
    if data.is_null() {
        return;
    }
    let server = (*data).server;
    let window_key = (*data).window_key;
    if let Some(window) = resolve_window(server, window_key) {
        (*window).mode_locked = false;
        (*window).wm_scheduled.maximize_requested = crate::window::MaximizeRequest::Unmaximize;
        (*server).wm.dirty_windowing();
    }
}

unsafe extern "C" fn toplevel_set_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let data = ffi::wl_resource_get_user_data(resource) as *mut CceToplevelData;
    if data.is_null() {
        return;
    }
    let server = (*data).server;
    let window_key = (*data).window_key;
    if let Some(window) = resolve_window(server, window_key) {
        (*window).wm_scheduled.fullscreen_requested = crate::window::FullscreenRequest::Fullscreen(std::ptr::null_mut());
        (*server).wm.dirty_windowing();
    }
}

unsafe extern "C" fn toplevel_unset_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let data = ffi::wl_resource_get_user_data(resource) as *mut CceToplevelData;
    if data.is_null() {
        return;
    }
    let server = (*data).server;
    let window_key = (*data).window_key;
    if let Some(window) = resolve_window(server, window_key) {
        (*window).wm_scheduled.fullscreen_requested = crate::window::FullscreenRequest::Exit;
        (*server).wm.dirty_windowing();
    }
}

unsafe extern "C" fn toplevel_set_minimized(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let data = ffi::wl_resource_get_user_data(resource) as *mut CceToplevelData;
    if data.is_null() {
        return;
    }
    let server = (*data).server;
    let window_key = (*data).window_key;
    if let Some(window) = resolve_window(server, window_key) {
        (*window).wm_scheduled.minimize_requested = true;
        (*server).wm.dirty_windowing();
    }
}

unsafe extern "C" fn toplevel_set_popup(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let data = ffi::wl_resource_get_user_data(resource) as *mut CceToplevelData;
    if data.is_null() {
        return;
    }
    let server = (*data).server;
    let window_key = (*data).window_key;
    if let Some(window) = resolve_window(server, window_key) {
        (*window).tiling_mode = crate::tiling::TilingMode::Popup;
        (*window).mode_locked = true;
        (*server).wm.dirty_windowing();
    }
}

unsafe extern "C" fn toplevel_unset_popup(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let data = ffi::wl_resource_get_user_data(resource) as *mut CceToplevelData;
    if data.is_null() {
        return;
    }
    let server = (*data).server;
    let window_key = (*data).window_key;
    if let Some(window) = resolve_window(server, window_key) {
        (*window).mode_locked = false;
        (*server).wm.dirty_windowing();
    }
}

static CCE_TOPLEVEL_INTERFACE: ffi::zcce_toplevel_v1_interface = ffi::zcce_toplevel_v1_interface {
    destroy: Some(toplevel_destroy),
    set_floating: Some(toplevel_set_floating),
    unset_floating: Some(toplevel_unset_floating),
    set_maximized: Some(toplevel_set_maximized),
    unset_maximized: Some(toplevel_unset_maximized),
    set_fullscreen: Some(toplevel_set_fullscreen),
    unset_fullscreen: Some(toplevel_unset_fullscreen),
    set_minimized: Some(toplevel_set_minimized),
    set_popup: Some(toplevel_set_popup),
    unset_popup: Some(toplevel_unset_popup),
};

unsafe fn resolve_window(server: *mut Server, key: SlotMapKey) -> Option<*mut Window> {
    if server.is_null() {
        return None;
    }
    let windows_map = &(*server).wm.windows;
    windows_map.get(key).copied().filter(|&w| !w.is_null())
}
