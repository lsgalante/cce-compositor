//! Client-drawn decoration surfaces (`zcce_decoration_v1`): `Decoration`, its
//! surface role, and its requests. Split out of window.rs on 2026-10-10.

use super::*;

// zcce_decoration_v1 implementation
pub struct DecorationRenderingRequested {
    pub offset_x: i32,
    pub offset_y: i32,
    pub sync_next_commit: bool,
    pub blur: bool,
}

pub struct Decoration {
    pub object: *mut ffi::wl_resource, // zcce_decoration_v1
    pub surface: *mut ffi::wlr_surface,
    pub tree: *mut ffi::wlr_scene_tree,
    pub surfaces: crate::scene::SaveableSurfaces,
    pub link: ffi::wl_list,
    pub window: *mut Window,
    pub rendering_requested: DecorationRenderingRequested,
}

impl Decoration {
    pub unsafe fn create(
        client: *mut ffi::wl_client,
        version: u32,
        id: u32,
        surface: *mut ffi::wlr_surface,
        parent: *mut ffi::wlr_scene_tree,
        window: *mut Window,
    ) -> Result<*mut Self, &'static str> {
        let decoration_v1 = ffi::wl_resource_create(client, &ffi::zcce_decoration_v1_interface, version as i32, id);
        if decoration_v1.is_null() {
            ffi::wl_client_post_no_memory(client);
            return Err("wl_resource_create failed");
        }

        if !ffi::wlr_surface_set_role(
            surface,
            &raw const DECORATION_ROLE,
            decoration_v1,
            ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_ROLE,
        ) {
            return Err("wlr_surface_set_role failed");
        }
        ffi::river_wlr_surface_set_role_object(surface, decoration_v1);

        let tree = ffi::wlr_scene_tree_create(parent);
        if tree.is_null() {
            return Err("wlr_scene_tree_create failed");
        }

        let surfaces = crate::scene::SaveableSurfaces::init(tree)?;
        let subsurface_tree = ffi::wlr_scene_subsurface_tree_create(surfaces.tree, surface);
        if subsurface_tree.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            return Err("wlr_scene_subsurface_tree_create failed");
        }

        let dec = Box::new(Decoration {
            object: decoration_v1,
            surface,
            tree,
            surfaces,
            link: std::mem::zeroed(),
            window,
            rendering_requested: DecorationRenderingRequested {
                offset_x: 0,
                offset_y: 0,
                sync_next_commit: false,
                blur: false,
            },
        });
        let raw = Box::into_raw(dec);

        ffi::wl_resource_set_implementation(
            decoration_v1,
            &DECORATION_INTERFACE as *const _ as *const _,
            raw as *mut _,
            Some(handle_dec_destroy_resource),
        );

        Ok(raw)
    }

    pub unsafe fn destroy(&mut self) {
        assert!(self.object.is_null());
        ffi::wlr_scene_node_destroy(self.tree as *mut ffi::wlr_scene_node);
        wl_list_remove(&mut self.link as *mut ffi::wl_list as *mut WlList);
        let _ = Box::from_raw(self);
    }

    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_DECORATION_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
        }
        if !self.surface.is_null() {
            ffi::river_wlr_surface_set_role_object(self.surface, std::ptr::null_mut());
        }
        self.surfaces.save();
    }

    pub unsafe fn render_finish(&mut self, _window_clip: *const ffi::wlr_box) {
        if self.rendering_requested.sync_next_commit {
            self.rendering_requested.sync_next_commit = false;

            if !self.surfaces.saved {
                if !self.object.is_null() {
                    ffi::wl_resource_post_error(
                        self.object,
                        ffi::zcce_decoration_v1_error_ZCCE_DECORATION_V1_ERROR_NO_COMMIT,
                        b"no wl_surface.commit after sync_next_commit and before update_rendering_finish\0".as_ptr() as *const _,
                    );
                }
            }
        }

        self.surfaces.drop_saved();

        let server = (*self.window).server;
        let app_id = (*self.window).get_app_id_string().unwrap_or_default();
        let mut ignore_transparent = (*server).wm.layout.window_backdrop_blur_ignore_transparent;
        let is_status = (*self.window).tiling_mode == crate::tiling::TilingMode::Status ||
                        app_id.starts_with("cce-status");
        if is_status {
            ignore_transparent = (*server).wm.layout.status_backdrop_blur_ignore_transparent;
        }
        let is_decorated = (*server).wm.is_decorated_app(&app_id);
        let blur_enabled = self.rendering_requested.blur && ((*self.window).wm_requested.ssd || is_decorated || is_status) && !(*self.window).droplet_backdrop_on();
        // Radius 0 preserves existing behaviour on the layer-surface path (see layer_shell.rs)
        // — it never had a blur radius applied, and this fix is scoped to toplevels.
        ffi::river_scene_node_enable_blur(self.surfaces.tree as *mut ffi::wlr_scene_node, blur_enabled, (*server).wm.layout.scenefx_optimized_blur, ignore_transparent, 0, 0, 0, 0, 0);

        let scale = (*self.window).scale;
        let scaled_x = (self.rendering_requested.offset_x as f64 * scale) as i32;
        let scaled_y = (self.rendering_requested.offset_y as f64 * scale) as i32;
        ffi::river_scene_node_set_position_if_changed(self.tree as *mut ffi::wlr_scene_node, scaled_x, scaled_y);

        struct ScaleData {
            scale: f64,
            ancestor: *mut ffi::wlr_scene_node,
        }

        unsafe extern "C" fn set_overview_scale_iterator(
            buffer: *mut ffi::wlr_scene_buffer,
            sx: i32,
            sy: i32,
            user_data: *mut std::ffi::c_void,
        ) {
            let data = &*(user_data as *const ScaleData);
            let node = buffer as *mut ffi::wlr_scene_node;

            let surface = ffi::river_scene_node_get_surface(node);
            if !surface.is_null() {
                let w = ffi::river_wlr_surface_get_width(surface);
                let h = ffi::river_wlr_surface_get_height(surface);
                if data.scale == 1.0 {
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                    ffi::river_scene_node_set_position_if_changed(node, 0, 0);
                } else {
                    let dest_w = (w as f64 * data.scale).round() as i32;
                    let dest_h = (h as f64 * data.scale).round() as i32;
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                    let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                    let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                    let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                    ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                }
                // Keep the opaque region in step with the dest scale —
                // unscaled it covers the shrunken node's translucent CSD
                // margins and occlusion culling stops repainting behind
                // the client shadow (stale pixels show through it).
                ffi::river_scene_buffer_set_scaled_opaque_region(buffer, surface, data.scale);
            }
            // Non-surface buffers are frozen SAVED copies (see
            // save_surface_tree_iter): their natural buffer size is
            // meaningless for geometry — HiDPI clients commit scale-N
            // buffers and Chromium pads buffers beyond the surface,
            // cropping via viewport src — so rescaling from it ballooned
            // ghosts around the window at any zoom change. A frozen copy
            // keeps its save-time dest/position; a zoom mid-transaction
            // leaves it briefly at the old zoom, which restore corrects.
        }

        let scale_data = ScaleData { scale: scale * (*self.window).x11_buffer_scale(), ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.surfaces.tree as *mut ffi::wlr_scene_node,
            Some(set_overview_scale_iterator),
            &scale_data as *const ScaleData as *mut std::ffi::c_void,
        );

        if self.surfaces.saved {
            let scale_data_saved = ScaleData { scale: scale * (*self.window).x11_buffer_scale(), ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                Some(set_overview_scale_iterator),
                &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
            );
        }

        let children_head = ffi::river_scene_tree_get_children(self.surfaces.tree) as *mut WlList;
        if (*children_head).next != children_head {
            ffi::wlr_scene_subsurface_tree_set_clip(self.surfaces.tree as *mut ffi::wlr_scene_node, std::ptr::null());
        }
    }

    /// Driven by the window's own pass, which decides when to run it
    /// (including the one reset pass back at `eff_scale` 1.0).
    pub unsafe fn scale_only_render_finish(&mut self, eff_scale: f64) {

        struct ScaleData {
            scale: f64,
            ancestor: *mut ffi::wlr_scene_node,
        }

        unsafe extern "C" fn set_overview_scale_iterator(
            buffer: *mut ffi::wlr_scene_buffer,
            sx: i32,
            sy: i32,
            user_data: *mut std::ffi::c_void,
        ) {
            let data = &*(user_data as *const ScaleData);
            let node = buffer as *mut ffi::wlr_scene_node;

            let surface = ffi::river_scene_node_get_surface(node);
            if !surface.is_null() {
                let w = ffi::river_wlr_surface_get_width(surface);
                let h = ffi::river_wlr_surface_get_height(surface);
                if data.scale == 1.0 {
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                    ffi::river_scene_node_set_position_if_changed(node, 0, 0);
                } else {
                    let dest_w = (w as f64 * data.scale).round() as i32;
                    let dest_h = (h as f64 * data.scale).round() as i32;
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                    let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                    let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                    let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                    ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                }
                // Keep the opaque region in step with the dest scale —
                // unscaled it covers the shrunken node's translucent CSD
                // margins and occlusion culling stops repainting behind
                // the client shadow (stale pixels show through it).
                ffi::river_scene_buffer_set_scaled_opaque_region(buffer, surface, data.scale);
            }
            // Non-surface buffers are frozen SAVED copies (see
            // save_surface_tree_iter): their natural buffer size is
            // meaningless for geometry — HiDPI clients commit scale-N
            // buffers and Chromium pads buffers beyond the surface,
            // cropping via viewport src — so rescaling from it ballooned
            // ghosts around the window at any zoom change. A frozen copy
            // keeps its save-time dest/position; a zoom mid-transaction
            // leaves it briefly at the old zoom, which restore corrects.
        }

        let scale_data = ScaleData { scale: eff_scale, ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.surfaces.tree as *mut ffi::wlr_scene_node,
            Some(set_overview_scale_iterator),
            &scale_data as *const ScaleData as *mut std::ffi::c_void,
        );

        if self.surfaces.saved {
            let scale_data_saved = ScaleData { scale: eff_scale, ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                Some(set_overview_scale_iterator),
                &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
            );
        }
    }
}

pub unsafe fn decoration_from_wlr_surface(surface: *mut ffi::wlr_surface) -> *mut Decoration {
    if surface.is_null() {
        return std::ptr::null_mut();
    }
    let role_ptr = ffi::river_wlr_surface_get_role(surface);
    if role_ptr != &raw const DECORATION_ROLE {
        return std::ptr::null_mut();
    }
    let resource = ffi::river_wlr_surface_get_role_resource(surface);
    if resource.is_null() {
        return std::ptr::null_mut();
    }
    ffi::wl_resource_get_user_data(resource) as *mut Decoration
}

unsafe extern "C" fn dec_client_commit(surface: *mut ffi::wlr_surface) {
    let dec = decoration_from_wlr_surface(surface);
    if dec.is_null() {
        return;
    }
    if (*dec).rendering_requested.sync_next_commit {
        (*dec).surfaces.save();
    }
}

unsafe extern "C" fn dec_commit(surface: *mut ffi::wlr_surface) {
    if ffi::wlr_surface_has_buffer(surface) {
        ffi::wlr_surface_map(surface);
    }
}

unsafe extern "C" fn dec_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn dec_set_offset(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if dec.is_null() {
        return;
    }
    let server = (*(*dec).window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*dec).rendering_requested.offset_x = x;
    (*dec).rendering_requested.offset_y = y;
}

unsafe extern "C" fn dec_sync_next_commit(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if dec.is_null() {
        return;
    }
    let server = (*(*dec).window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*dec).rendering_requested.sync_next_commit = true;
}

unsafe extern "C" fn dec_set_blur(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    blur: u32,
) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if dec.is_null() {
        return;
    }
    let server = (*(*dec).window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*dec).rendering_requested.blur = blur != 0;
}

pub(crate) static DECORATION_INTERFACE: ffi::zcce_decoration_v1_interface = ffi::zcce_decoration_v1_interface {
    destroy: Some(dec_destroy),
    set_offset: Some(dec_set_offset),
    sync_next_commit: Some(dec_sync_next_commit),
    set_blur: Some(dec_set_blur),
};

pub(crate) static INERT_DECORATION_INTERFACE: ffi::zcce_decoration_v1_interface = ffi::zcce_decoration_v1_interface {
    destroy: Some(dec_destroy),
    set_offset: None,
    sync_next_commit: None,
    set_blur: None,
};

unsafe extern "C" fn handle_dec_destroy_resource(resource: *mut ffi::wl_resource) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if !dec.is_null() {
        ffi::river_wlr_surface_set_role_object((*dec).surface, std::ptr::null_mut());
        (*dec).object = std::ptr::null_mut();
        (*dec).destroy();
    }
}

unsafe extern "C" fn dec_role_destroy(surface: *mut ffi::wlr_surface) {
    let dec = decoration_from_wlr_surface(surface);
    if dec.is_null() {
        return;
    }
    ffi::river_wlr_surface_set_role_object(surface, std::ptr::null_mut());
    if !(*dec).object.is_null() {
        ffi::wl_resource_set_user_data((*dec).object, std::ptr::null_mut());
        ffi::wl_resource_destroy((*dec).object);
        (*dec).object = std::ptr::null_mut();
    }
    (*dec).destroy();
}

#[no_mangle]
pub static mut DECORATION_ROLE: ffi::wlr_surface_role = ffi::wlr_surface_role {
    name: b"zcce_decoration_v1\0".as_ptr() as *const _,
    no_object: false,
    client_commit: Some(dec_client_commit),
    commit: Some(dec_commit),
    map: None,
    unmap: None,
    destroy: Some(dec_role_destroy),
};
