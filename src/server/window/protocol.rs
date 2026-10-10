//! The `zcce_window_v1` requests a window manager client sends about a window,
//! and the interface tables that route them. Split out of window.rs on
//! 2026-10-10.

use super::*;

pub(crate) unsafe fn clock_gettime(clk_id: libc::clockid_t, tp: &mut libc::timespec) -> libc::c_int {
    libc::clock_gettime(clk_id, tp)
}

unsafe extern "C" fn window_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn window_close(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.close = true;
}

unsafe extern "C" fn window_get_node(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    if !(*window).node.object.is_null() {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_NODE_EXISTS,
            b"window already has a node object\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).node.create_object(client, ffi::wl_resource_get_version(resource) as u32, id);
}

unsafe extern "C" fn window_propose_dimensions(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    width: i32,
    height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    if width < 0 || height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_DIMENSIONS,
            b"dimensions must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    if (*window).get_parent().is_null() {
        (*window).wm_requested.dimensions = Some(Dimensions {
            width: width as u32,
            height: height as u32,
        });
    }
}

unsafe extern "C" fn window_hide(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.hidden = true;
}

unsafe extern "C" fn window_show(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.hidden = false;
}

unsafe extern "C" fn window_use_csd(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.ssd = false;
    (*server).wm.dirty_windowing();
}

unsafe extern "C" fn window_use_ssd(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.ssd = true;
    (*server).wm.dirty_windowing();
}

unsafe extern "C" fn window_set_borders(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    edges: u32,
    width: i32,
    r: u32,
    g: u32,
    b: u32,
    a: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    if width < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_BORDER,
            b"border width must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    let alpha = (a as f64 / u32::MAX as f64) as f32;
    // Protocol channels are straight alpha; scene colors are premultiplied.
    let color = [
        (r as f64 / u32::MAX as f64) as f32 * alpha,
        (g as f64 / u32::MAX as f64) as f32 * alpha,
        (b as f64 / u32::MAX as f64) as f32 * alpha,
        alpha,
    ];
    (*window).rendering_requested.border = Border {
        edges: Edges::from_u32(edges),
        width: width as u32,
        color,
        // Protocol-set borders don't participate in hover highlighting.
        hover_color: color,
    };
}

unsafe extern "C" fn window_set_tiled(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    edges: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.tiled = edges;
}

unsafe extern "C" fn window_get_decoration_above(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    wl_surface: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let wlr_surface = ffi::wlr_surface_from_resource(wl_surface);
    let decoration = match Decoration::create(
        client,
        ffi::wl_resource_get_version(resource) as u32,
        id,
        wlr_surface,
        (*window).decorations_above_tree,
        window,
    ) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to create decoration: {}", e);
            ffi::wl_client_post_no_memory(client);
            return;
        }
    };
    let list_head = &mut (*window).decorations_above as *mut ffi::wl_list as *mut WlList;
    wl_list_insert((*list_head).prev, &mut (*decoration).link as *mut ffi::wl_list as *mut WlList);
}

unsafe extern "C" fn window_get_decoration_below(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    wl_surface: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let wlr_surface = ffi::wlr_surface_from_resource(wl_surface);
    let decoration = match Decoration::create(
        client,
        ffi::wl_resource_get_version(resource) as u32,
        id,
        wlr_surface,
        (*window).decorations_below_tree,
        window,
    ) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to create decoration: {}", e);
            ffi::wl_client_post_no_memory(client);
            return;
        }
    };
    let list_head = &mut (*window).decorations_below as *mut ffi::wl_list as *mut WlList;
    wl_list_insert((*list_head).prev, &mut (*decoration).link as *mut ffi::wl_list as *mut WlList);
}

unsafe extern "C" fn window_inform_resize_start(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.resizing = true;
}

unsafe extern "C" fn window_inform_resize_end(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.resizing = false;
}

unsafe extern "C" fn window_set_capabilities(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    caps: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.capabilities = caps;
}

unsafe extern "C" fn window_inform_maximized(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.maximized = true;
}

unsafe extern "C" fn window_inform_unmaximized(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.maximized = false;
}

unsafe extern "C" fn window_inform_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.inform_fullscreen = true;
}

unsafe extern "C" fn window_inform_not_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.inform_fullscreen = false;
}

unsafe extern "C" fn window_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    output: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    let out = if output.is_null() {
        std::ptr::null_mut()
    } else {
        let wlr_output = ffi::wlr_output_from_resource(output);
        if wlr_output.is_null() {
            std::ptr::null_mut()
        } else {
            ffi::river_wlr_output_get_data(wlr_output) as *mut crate::output::Output
        }
    };
    (*window).wm_requested.fullscreen = out;
}

unsafe extern "C" fn window_exit_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.fullscreen = std::ptr::null_mut();
}

unsafe extern "C" fn window_set_clip_box(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    if width < 0 || height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_CLIP_BOX,
            b"width/height must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).rendering_requested.clip = ffi::wlr_box {
        x,
        y,
        width,
        height,
    };
}

unsafe extern "C" fn window_set_content_clip_box(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    if width < 0 || height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_CLIP_BOX,
            b"width/height must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).rendering_requested.content_clip = ffi::wlr_box {
        x,
        y,
        width,
        height,
    };
}

unsafe extern "C" fn window_set_dimension_bounds(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    max_width: i32,
    max_height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    if max_width < 0 || max_height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_DIMENSIONS,
            b"dimensions must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).wm_requested.bounds = Dimensions {
        width: max_width as u32,
        height: max_height as u32,
    };
}

unsafe extern "C" fn window_set_opacity(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    opacity: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    let opacity_f32 = opacity as f32 / u32::MAX as f32;
    (*window).rendering_requested.opacity = opacity_f32;
}

unsafe extern "C" fn window_set_circular(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    circular: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.circular = circular != 0;
}

unsafe extern "C" fn window_set_blur(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    blur: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.blur = blur != 0;
}

// zcce_window_v1 implementation
pub(crate) static WINDOW_INTERFACE: ffi::zcce_window_v1_interface = ffi::zcce_window_v1_interface {
    destroy: Some(window_destroy),
    close: Some(window_close),
    get_node: Some(window_get_node),
    propose_dimensions: Some(window_propose_dimensions),
    hide: Some(window_hide),
    show: Some(window_show),
    use_csd: Some(window_use_csd),
    use_ssd: Some(window_use_ssd),
    set_borders: Some(window_set_borders),
    set_tiled: Some(window_set_tiled),
    get_decoration_above: Some(window_get_decoration_above),
    get_decoration_below: Some(window_get_decoration_below),
    inform_resize_start: Some(window_inform_resize_start),
    inform_resize_end: Some(window_inform_resize_end),
    set_capabilities: Some(window_set_capabilities),
    inform_maximized: Some(window_inform_maximized),
    inform_unmaximized: Some(window_inform_unmaximized),
    inform_fullscreen: Some(window_inform_fullscreen),
    inform_not_fullscreen: Some(window_inform_not_fullscreen),
    fullscreen: Some(window_fullscreen),
    exit_fullscreen: Some(window_exit_fullscreen),
    set_clip_box: Some(window_set_clip_box),
    set_content_clip_box: Some(window_set_content_clip_box),
    set_dimension_bounds: Some(window_set_dimension_bounds),
    set_opacity: Some(window_set_opacity),
    set_circular: Some(window_set_circular),
    set_blur: Some(window_set_blur),
};

pub(crate) static INERT_WINDOW_INTERFACE: ffi::zcce_window_v1_interface = ffi::zcce_window_v1_interface {
    destroy: Some(window_destroy),
    close: None,
    get_node: None,
    propose_dimensions: None,
    hide: None,
    show: None,
    use_csd: None,
    use_ssd: None,
    set_borders: None,
    set_tiled: None,
    get_decoration_above: None,
    get_decoration_below: None,
    inform_resize_start: None,
    inform_resize_end: None,
    set_capabilities: None,
    inform_maximized: None,
    inform_unmaximized: None,
    inform_fullscreen: None,
    inform_not_fullscreen: None,
    fullscreen: None,
    exit_fullscreen: None,
    set_clip_box: None,
    set_content_clip_box: None,
    set_dimension_bounds: None,
    set_opacity: None,
    set_circular: None,
    set_blur: None,
};

pub(crate) unsafe extern "C" fn handle_destroy_resource(resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if !window.is_null() {
        if (*window).object != resource {
            return;
        }
        (*window).object = std::ptr::null_mut();
        (*window).node.make_inert();
        
        for decorations in [&mut (*window).decorations_above as *mut ffi::wl_list, &mut (*window).decorations_below as *mut ffi::wl_list] {
            let list_head = decorations as *mut WlList;
            let mut curr = (*list_head).next;
            while curr != list_head {
                let next = (*curr).next;
                let dec = crate::container_of!(curr, Decoration, link);
                (*dec).make_inert();
                curr = next;
            }
        }
    }
}
