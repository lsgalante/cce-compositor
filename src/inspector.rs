use crate::ffi;
use crate::server::Server;
use std::collections::HashMap;

pub struct Inspector {
    pub server: *mut Server,
    pub global: *mut ffi::wl_global,
    pub client_states: HashMap<*mut ffi::wlr_surface, String>,
    pub resources: Vec<*mut ffi::wl_resource>,
}

impl Inspector {
    pub fn new() -> Self {
        Self {
            server: std::ptr::null_mut(),
            global: std::ptr::null_mut(),
            client_states: HashMap::new(),
            resources: Vec::new(),
        }
    }

    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        self.client_states = HashMap::new();
        self.resources = Vec::new();

        self.global = ffi::wl_global_create(
            (*server).wl_server,
            &ffi::zclear_inspector_v1_interface,
            1,
            self as *mut Inspector as *mut _,
            Some(bind),
        );

        if self.global.is_null() {
            return Err("Failed to create zclear_inspector_v1 global");
        }

        log::info!("zclear_inspector_v1 protocol global initialized successfully");
        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        if !self.global.is_null() {
            ffi::wl_global_destroy(self.global);
            self.global = std::ptr::null_mut();
        }
    }
}

unsafe extern "C" fn handle_destroy_resource(resource: *mut ffi::wl_resource) {
    let inspector = ffi::wl_resource_get_user_data(resource) as *mut Inspector;
    if !inspector.is_null() {
        (*inspector).resources.retain(|&r| r != resource);
    }
}

unsafe extern "C" fn bind(
    client: *mut ffi::wl_client,
    data: *mut std::ffi::c_void,
    version: u32,
    id: u32,
) {
    let inspector = data as *mut Inspector;
    if inspector.is_null() {
        return;
    }

    let resource = ffi::wl_resource_create(client, &ffi::zclear_inspector_v1_interface, version as i32, id);
    if resource.is_null() {
        ffi::wl_client_post_no_memory(client);
        return;
    }

    ffi::wl_resource_set_implementation(
        resource,
        &INSPECTOR_INTERFACE as *const _ as *const _,
        inspector as *mut _,
        Some(handle_destroy_resource),
    );

    (*inspector).resources.push(resource);
}

unsafe extern "C" fn inspector_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn inspector_register_client(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    surface_resource: *mut ffi::wl_resource,
) {
    let inspector = ffi::wl_resource_get_user_data(resource) as *mut Inspector;
    if inspector.is_null() {
        return;
    }
    let surface = ffi::wlr_surface_from_resource(surface_resource);
    if surface.is_null() {
        return;
    }
    (*inspector).client_states.entry(surface).or_insert_with(String::new);
}

unsafe extern "C" fn inspector_update_state(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    surface_resource: *mut ffi::wl_resource,
    state: *const std::os::raw::c_char,
) {
    let inspector = ffi::wl_resource_get_user_data(resource) as *mut Inspector;
    if inspector.is_null() {
        return;
    }
    let surface = ffi::wlr_surface_from_resource(surface_resource);
    if surface.is_null() {
        return;
    }
    if !state.is_null() {
        let state_str = std::ffi::CStr::from_ptr(state).to_string_lossy().into_owned();
        (*inspector).client_states.insert(surface, state_str);
    }
}

unsafe extern "C" fn inspector_get_inspected_surfaces(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let inspector = ffi::wl_resource_get_user_data(resource) as *mut Inspector;
    if inspector.is_null() {
        return;
    }
    let server = (*inspector).server;
    let windows = &(*server).wm.windows;

    for &window in windows.iter() {
        if window.is_null() {
            continue;
        }
        let root_surface = (*window).root_surface();
        if root_surface.is_null() {
            continue;
        }

        // Retrieve registered UI state JSON or default to empty
        let state = (*inspector)
            .client_states
            .get(&root_surface)
            .cloned()
            .unwrap_or_else(|| "{}".to_string());

        let title_ptr = (*window).get_title();
        let app_id_ptr = (*window).get_app_id();

        let title = if title_ptr.is_null() {
            std::ffi::CString::new("").unwrap()
        } else {
            std::ffi::CStr::from_ptr(title_ptr).to_owned()
        };

        let app_id = if app_id_ptr.is_null() {
            std::ffi::CString::new("").unwrap()
        } else {
            std::ffi::CStr::from_ptr(app_id_ptr).to_owned()
        };

        let state_c = std::ffi::CString::new(state).unwrap();

        // Send: inspected_surface(title, app_id, x, y, width, height, state)
        // Event inspected_surface has index 0
        ffi::wl_resource_post_event(
            resource,
            0,
            title.as_ptr(),
            app_id.as_ptr(),
            (*window).box_geom.x,
            (*window).box_geom.y,
            (*window).box_geom.width,
            (*window).box_geom.height,
            state_c.as_ptr(),
        );
    }

    // Send: inspected_surface_done()
    // Event inspected_surface_done has index 1
    ffi::wl_resource_post_event(resource, 1);
}

static INSPECTOR_INTERFACE: ffi::zclear_inspector_v1_interface = ffi::zclear_inspector_v1_interface {
    destroy: Some(inspector_destroy),
    register_client: Some(inspector_register_client),
    update_state: Some(inspector_update_state),
    get_inspected_surfaces: Some(inspector_get_inspected_surfaces),
};
