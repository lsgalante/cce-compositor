// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, WlList, wl_signal_add, wl_listener_remove, wl_list_insert, wl_list_remove};
use crate::layer_shell::LayerShellOutput;
use crate::lock_manager::LockState;
use crate::util;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputStateValue {
    Enabled,
    DisabledSoft,
    DisabledHard,
    Destroying,
}

#[derive(Debug, Clone, Copy)]
pub enum OutputMode {
    Standard(*mut ffi::wlr_output_mode),
    Custom {
        width: i32,
        height: i32,
        refresh: i32,
    },
    None,
}

#[derive(Debug, Clone, Copy)]
pub struct OutputState {
    pub state: OutputStateValue,
    pub x: i32,
    pub y: i32,
    pub mode: OutputMode,
    pub scale: f32,
    pub transform: ffi::wl_output_transform,
    pub adaptive_sync: bool,
    pub auto_layout: bool,
}

impl OutputState {
    pub fn mode_none(&self) -> bool {
        matches!(self.mode, OutputMode::None)
    }

    pub unsafe fn from_head_state(state: *const ffi::wlr_output_head_v1_state) -> Self {
        assert!((*state).enabled);
        let mode = if !(*state).mode.is_null() {
            OutputMode::Standard((*state).mode)
        } else {
            OutputMode::Custom {
                width: (*state).custom_mode.width,
                height: (*state).custom_mode.height,
                refresh: (*state).custom_mode.refresh,
            }
        };

        // Round to nearest 1/120 to ensure the scale is exactly represented
        // in the fractional-scale-v1 protocol.
        let scale = ((*state).scale * 120.0).round() / 120.0;

        Self {
            state: OutputStateValue::Enabled,
            mode,
            x: (*state).x,
            y: (*state).y,
            scale,
            transform: (*state).transform,
            adaptive_sync: (*state).adaptive_sync_enabled,
            auto_layout: false,
        }
    }

    pub unsafe fn dimensions(&self) -> (i32, i32) {
        let (mut w, mut h) = match self.mode {
            OutputMode::Standard(mode) => ((*mode).width, (*mode).height),
            OutputMode::Custom { width, height, .. } => (width, height),
            OutputMode::None => (0, 0),
        };
        if (self.transform as u32) % 2 != 0 {
            std::mem::swap(&mut w, &mut h);
        }
        (
            ((w as f32) / self.scale) as i32,
            ((h as f32) / self.scale) as i32,
        )
    }

    pub unsafe fn box_layout(&self) -> ffi::wlr_box {
        let (w, h) = self.dimensions();
        ffi::wlr_box {
            x: self.x,
            y: self.y,
            width: w,
            height: h,
        }
    }

    pub unsafe fn apply_no_modeset(&self, wlr_state: *mut ffi::wlr_output_state) {
        ffi::wlr_output_state_set_scale(wlr_state, self.scale);
        ffi::wlr_output_state_set_transform(wlr_state, self.transform);
    }

    pub unsafe fn apply_modeset(&self, wlr_state: *mut ffi::wlr_output_state) {
        let enabled = self.state == OutputStateValue::Enabled;
        ffi::wlr_output_state_set_enabled(wlr_state, enabled);
        if !enabled {
            return;
        }
        self.apply_no_modeset(wlr_state);
        match self.mode {
            OutputMode::Standard(mode) => {
                ffi::wlr_output_state_set_mode(wlr_state, mode);
            }
            OutputMode::Custom { width, height, refresh } => {
                ffi::wlr_output_state_set_custom_mode(wlr_state, width, height, refresh);
            }
            OutputMode::None => {}
        }
        ffi::wlr_output_state_set_adaptive_sync_enabled(wlr_state, self.adaptive_sync);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LockRenderState {
    PendingUnlock,
    Unlocked,
    PendingBlank,
    Blanked,
    PendingLockSurface,
    LockSurface,
}

#[derive(Debug, Clone, Copy)]
pub struct RenderingState {
    pub tearing: bool,
}

pub struct Output {
    pub server: *mut Server,
    pub wlr_output: *mut ffi::wlr_output,
    pub scene_output: *mut ffi::wlr_scene_output,
    pub background_rect: *mut ffi::wlr_scene_rect,
    pub grid_tree: *mut ffi::wlr_scene_tree,
    pub object: *mut ffi::wl_resource, // zcce_output_v1 resource
    pub layer_shell: LayerShellOutput,
    pub lock_render_state: LockRenderState,
    pub link: ffi::wl_list,
    pub link_sent: ffi::wl_list,
    pub scheduled: OutputState,
    pub sent: OutputState,
    pub current: OutputState,
    pub sent_wl_output: bool,
    pub rendering_requested: RenderingState,
    pub rendering_current: RenderingState,

    // Cached grid parameters to avoid redrawing when unchanged
    pub last_grid_viewport_w: i32,
    pub last_grid_viewport_h: i32,
    pub last_grid_zoom: f64,
    pub last_grid_pan_x: f64,
    pub last_grid_pan_y: f64,
    pub last_grid_spacing: f64,
    pub last_grid_gap_width: i32,
    pub last_grid_cell_color: [f32; 4],
    pub last_grid_cell_corner_radius: i32,
    pub last_grid_cell_fade_inset: i64,
    pub last_grid_gap_color: String,
    pub grid_is_low_res: bool,
    pub grid_rect_pool: Vec<*mut ffi::wlr_scene_rect>,

    pub destroy: ffi::wl_listener,
    pub request_state: ffi::wl_listener,
    pub frame: ffi::wl_listener,
    pub present: ffi::wl_listener,
}

unsafe extern "C" fn handle_destroy_resource(resource: *mut ffi::wl_resource) {
    let output = ffi::wl_resource_get_user_data(resource) as *mut Output;
    if !output.is_null() {
        if (*output).object != resource {
            return;
        }
        (*output).object = std::ptr::null_mut();
        (*output).sent_wl_output = false;
    }
}

unsafe extern "C" fn output_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn output_set_presentation_mode(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    mode: u32,
) {
    let output = ffi::wl_resource_get_user_data(resource) as *mut Output;
    if output.is_null() {
        return;
    }
    if !(*(*output).server).wm.ensure_rendering() {
        return;
    }
    match mode {
        ffi::zcce_output_v1_presentation_mode_ZCCE_OUTPUT_V1_PRESENTATION_MODE_VSYNC => {
            (*output).rendering_requested.tearing = false;
        }
        ffi::zcce_output_v1_presentation_mode_ZCCE_OUTPUT_V1_PRESENTATION_MODE_ASYNC => {
            (*output).rendering_requested.tearing = true;
        }
        _ => {
            ffi::wl_resource_post_error(
                resource,
                ffi::zcce_output_v1_error_ZCCE_OUTPUT_V1_ERROR_INVALID_PRESENTATION_MODE,
                b"invalid presentation mode enum value\0".as_ptr() as *const _,
            );
        }
    }
}

static OUTPUT_INTERFACE: ffi::zcce_output_v1_interface = ffi::zcce_output_v1_interface {
    destroy: Some(output_destroy),
    set_presentation_mode: Some(output_set_presentation_mode),
};

static INERT_OUTPUT_INTERFACE: ffi::zcce_output_v1_interface = ffi::zcce_output_v1_interface {
    destroy: Some(output_destroy),
    set_presentation_mode: None,
};

impl Output {
    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_post_event(self.object, 0); // zcce_output.removed
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_OUTPUT_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.layer_shell.make_inert();
            self.object = std::ptr::null_mut();
            self.sent_wl_output = false;
            self.grid_rect_pool.clear();
        }
    }

    pub unsafe fn manage_start(&mut self) {
        match self.scheduled.state {
            OutputStateValue::Enabled | OutputStateValue::DisabledSoft => {
                assert!(!self.scheduled.mode_none());
                let wlr_output = self.wlr_output;

                let self_ptr = self as *mut Output;
                let layer_shell_ptr = &mut self.layer_shell as *mut LayerShellOutput;
                (*layer_shell_ptr).manage_start(self_ptr);

                let wm_v1 = (*self.server).wm.object;
                if !wm_v1.is_null() {
                    let new = self.object.is_null();
                    let output_v1 = if new {
                        let client = ffi::wl_resource_get_client(wm_v1);
                        let res = ffi::wl_resource_create(
                            client,
                            &ffi::zcce_output_v1_interface,
                            ffi::wl_resource_get_version(wm_v1),
                            0,
                        );
                        if res.is_null() {
                            log::error!("out of memory");
                            return;
                        }
                        self.object = res;
                        ffi::wl_resource_set_implementation(
                            res,
                            &OUTPUT_INTERFACE as *const _ as *const _,
                            self as *mut Output as *mut _,
                            Some(handle_destroy_resource),
                        );
                        ffi::wl_resource_post_event(wm_v1, ffi::ZCCE_WINDOW_MANAGER_V1_OUTPUT, res); // zcce_window_manager_v1.output
                        res
                    } else {
                        self.object
                    };

                    if !self.sent_wl_output {
                        let global = ffi::river_wlr_output_get_global(wlr_output);
                        if !global.is_null() {
                            let client = ffi::wl_resource_get_client(output_v1);
                            let wl_output_name = ffi::wl_global_get_name(global, client);
                            zcce_output_send_wl_output(output_v1, wl_output_name);
                            self.sent_wl_output = true;
                        }
                    }

                    let (scheduled_width, scheduled_height) = self.scheduled.dimensions();
                    let (sent_width, sent_height) = self.sent.dimensions();

                    if new || scheduled_width != sent_width || scheduled_height != sent_height {
                        zcce_output_send_dimensions(output_v1, scheduled_width, scheduled_height);
                    }
                    if new || self.scheduled.x != self.sent.x || self.scheduled.y != self.sent.y {
                        zcce_output_send_position(output_v1, self.scheduled.x, self.scheduled.y);
                    }
                }

                self.sent = self.scheduled;

                wl_list_remove(&mut self.link_sent as *mut ffi::wl_list as *mut WlList);
                let sent_outputs = &mut (*self.server).wm.sent.outputs as *mut ffi::wl_list as *mut WlList;
                wl_list_insert((*sent_outputs).prev, &mut self.link_sent as *mut ffi::wl_list as *mut WlList);
            }
            OutputStateValue::DisabledHard | OutputStateValue::Destroying => {
                self.make_inert();

                self.sent = self.scheduled;

                if self.scheduled.state == OutputStateValue::Destroying {
                    assert!(self.wlr_output.is_null());
                    
                    if !self.background_rect.is_null() {
                        ffi::wlr_scene_node_destroy(self.background_rect as *mut ffi::wlr_scene_node);
                        self.background_rect = std::ptr::null_mut();
                    }

                    if !self.grid_tree.is_null() {
                        ffi::wlr_scene_node_destroy(self.grid_tree as *mut ffi::wlr_scene_node);
                        self.grid_tree = std::ptr::null_mut();
                    }
                    self.grid_rect_pool.clear();

                    // remove output from windows fullscreen hint
                    for &window in (*self.server).wm.windows.iter() {
                        if let crate::window::FullscreenRequest::Fullscreen(out) = (*window).wm_scheduled.fullscreen_requested {
                            if out == self as *mut Output {
                                (*window).wm_scheduled.fullscreen_requested = crate::window::FullscreenRequest::Fullscreen(std::ptr::null_mut());
                            }
                        }
                        if (*window).wm_requested.fullscreen == self as *mut Output {
                            (*window).wm_requested.fullscreen = std::ptr::null_mut();
                        }
                    }

                    wl_list_remove(&mut self.link as *mut ffi::wl_list as *mut WlList);
                    wl_list_remove(&mut self.link_sent as *mut ffi::wl_list as *mut WlList);

                    let _ = Box::from_raw(self as *mut Output);
                }
            }
        }
    }

    pub unsafe fn create(server: *mut Server, wlr_output: *mut ffi::wlr_output) -> Result<(), &'static str> {
        let title = format!("river - {}\0", std::ffi::CStr::from_ptr(ffi::river_wlr_output_get_name(wlr_output)).to_string_lossy());
        
        // Check if output is Wayland/X11 and set application title/app_id
        if ffi::wlr_output_is_wl(wlr_output) {
            ffi::wlr_wl_output_set_app_id(wlr_output, "river\0".as_ptr() as *const _);
            ffi::wlr_wl_output_set_title(wlr_output, title.as_ptr() as *const _);
        } else if ffi::wlr_output_is_x11(wlr_output) {
            ffi::wlr_x11_output_set_title(wlr_output, title.as_ptr() as *const _);
        }

        if !ffi::wlr_output_init_render(wlr_output, (*server).allocator, (*server).renderer) {
            return Err("Failed to initialize renderer for output");
        }

        let scene_output = ffi::wlr_scene_output_create((*server).scene.wlr_scene, wlr_output);
        if scene_output.is_null() {
            return Err("Failed to create wlr_scene_output");
        }

        let name_raw = ffi::river_wlr_output_get_name(wlr_output);
        let name = std::ffi::CStr::from_ptr(name_raw).to_string_lossy();
        let scale_key = format!("scale_{}", name);
        let output_scale = (*server).wm.display.get(&scale_key)
            .map(|&s| s as f32)
            .unwrap_or((*server).wm.output_scale);

        let initial = OutputState {
            state: OutputStateValue::DisabledHard,
            x: 0,
            y: 0,
            mode: OutputMode::None,
            scale: output_scale,
            transform: ffi::wl_output_transform_WL_OUTPUT_TRANSFORM_NORMAL,
            adaptive_sync: ffi::river_wlr_output_get_adaptive_sync_status(wlr_output) == ffi::wlr_output_adaptive_sync_status_WLR_OUTPUT_ADAPTIVE_SYNC_ENABLED,
            auto_layout: true,
        };

        let output = Box::new(Output {
            server,
            wlr_output,
            scene_output,
            background_rect: std::ptr::null_mut(),
            grid_tree: std::ptr::null_mut(),
            object: std::ptr::null_mut(),
            layer_shell: LayerShellOutput::default(),
            lock_render_state: LockRenderState::Blanked,
            link: std::mem::zeroed(),
            link_sent: std::mem::zeroed(),
            scheduled: initial,
            sent: initial,
            current: initial,
            sent_wl_output: false,
            rendering_requested: RenderingState { tearing: false },
            rendering_current: RenderingState { tearing: false },
            last_grid_viewport_w: 0,
            last_grid_viewport_h: 0,
            last_grid_zoom: 0.0,
            last_grid_pan_x: 0.0,
            last_grid_pan_y: 0.0,
            last_grid_spacing: 0.0,
            last_grid_gap_width: 0,
            last_grid_cell_color: [0.0, 0.0, 0.0, 0.0],
            last_grid_cell_corner_radius: 0,
            last_grid_cell_fade_inset: 0,
            last_grid_gap_color: String::new(),
            grid_is_low_res: false,
            grid_rect_pool: Vec::new(),
            destroy: std::mem::zeroed(),
            request_state: std::mem::zeroed(),
            frame: std::mem::zeroed(),
            present: std::mem::zeroed(),
        });

        let raw = Box::into_raw(output);
        ffi::river_wlr_output_set_data(wlr_output, raw as *mut std::ffi::c_void);

        let list_head = &mut (*server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let link_custom = &mut (*raw).link as *mut ffi::wl_list as *mut WlList;
        wl_list_insert((*list_head).prev, link_custom);
        
        ffi::wl_list_init(&mut (*raw).link_sent);

        // Add event listeners
        let d_listener = &mut (*raw).destroy as *mut ffi::wl_listener as *mut WlListener;
        (*d_listener).notify = Some(handle_destroy);
        wl_signal_add(ffi::river_wlr_output_get_destroy_signal(wlr_output), &mut (*raw).destroy);

        let req_listener = &mut (*raw).request_state as *mut ffi::wl_listener as *mut WlListener;
        (*req_listener).notify = Some(handle_request_state);
        wl_signal_add(ffi::river_wlr_output_get_request_state_signal(wlr_output), &mut (*raw).request_state);

        let frame_listener = &mut (*raw).frame as *mut ffi::wl_listener as *mut WlListener;
        (*frame_listener).notify = Some(handle_frame);
        wl_signal_add(ffi::river_wlr_output_get_frame_signal(wlr_output), &mut (*raw).frame);

        let pres_listener = &mut (*raw).present as *mut ffi::wl_listener as *mut WlListener;
        (*pres_listener).notify = Some(handle_present);
        wl_signal_add(ffi::river_wlr_output_get_present_signal(wlr_output), &mut (*raw).present);

        (*raw).scheduled.state = OutputStateValue::Enabled;
        let preferred = ffi::wlr_output_preferred_mode(wlr_output);
        if !preferred.is_null() {
            (*raw).scheduled.mode = OutputMode::Standard(preferred);
        } else {
            (*raw).scheduled.mode = OutputMode::Custom { width: 1280, height: 720, refresh: 0 };
        }
        
        (*server).wm.dirty_windowing();
        Ok(())
    }

    pub unsafe fn render_and_commit(&mut self) -> Result<(), &'static str> {
        if !ffi::wlr_scene_output_needs_frame(self.scene_output) {
            return Ok(());
        }

        // Re-apply scale to all windows whose scale is not 1.0 right before rendering
        let wm = &(*self.server).wm;
        for &window in wm.windows.iter() {
            if !window.is_null() && (*window).scale != 1.0 {
                (*window).scale_only_render_finish();
            }
        }

        self.draw_grid();

        let mut state = std::mem::zeroed();
        ffi::wlr_output_state_init(&mut state);
        
        self.current.apply_no_modeset(&mut state);

        if !ffi::wlr_scene_output_build_state(self.scene_output, &mut state, std::ptr::null()) {
            ffi::wlr_output_state_finish(&mut state);
            return Err("Failed to build scene state");
        }

        if self.rendering_current.tearing {
            state.tearing_page_flip = true;
            if !ffi::wlr_output_test_state(self.wlr_output, &state) {
                state.tearing_page_flip = false;
            }
        }

        if !ffi::wlr_output_commit_state(self.wlr_output, &state) {
            ffi::wlr_output_state_finish(&mut state);
            return Err("Failed to commit state");
        }

        ffi::wlr_output_state_finish(&mut state);

        match (*self.server).lock_manager.state {
            LockState::Unlocked => {
                if self.lock_render_state != LockRenderState::Unlocked {
                    self.lock_render_state = LockRenderState::PendingUnlock;
                }
            }
            LockState::Locked => {
                // Assert normal tree disabled, lock surface rendered
            }
            LockState::WaitingForBlank => {
                if self.lock_render_state != LockRenderState::Blanked {
                    self.lock_render_state = LockRenderState::PendingBlank;
                }
            }
            LockState::WaitingForLockSurfaces => {
                if let Some(_lock_surf) = (*self.server).lock_manager.lock_surface_from_output(self) {
                    if self.lock_render_state != LockRenderState::LockSurface {
                        self.lock_render_state = LockRenderState::PendingLockSurface;
                    }
                } else {
                    if self.lock_render_state != LockRenderState::Unlocked {
                        self.lock_render_state = LockRenderState::PendingUnlock;
                    }
                }
            }
        }

        Ok(())
    }

    pub unsafe fn update_background_color(&mut self) {
        if !self.background_rect.is_null() {
            let wm = &(*self.server).wm;
            let color: [f32; 4] = [
                (wm.layout.background_r as f64 / u32::MAX as f64) as f32,
                (wm.layout.background_g as f64 / u32::MAX as f64) as f32,
                (wm.layout.background_b as f64 / u32::MAX as f64) as f32,
                (wm.layout.background_a as f64 / u32::MAX as f64) as f32,
            ];
            ffi::wlr_scene_rect_set_color(self.background_rect, color.as_ptr());
        }
    }

    pub unsafe fn draw_grid(&mut self) {
        if self.grid_tree.is_null() {
            return;
        }

        let wm = &(*self.server).wm;
        let (viewport_w, viewport_h) = self.current.dimensions();
        let zoom = wm.desk_zoom;

        // LOD calculations for cells and gaps
        let mut cell_size = wm.layout.desktop_grid_scale.max(5.0);
        let mut gap_size = (wm.layout.desktop_gap_width as f64).max(0.0);
        let mut period = cell_size + gap_size;

        while period * zoom < 40.0 {
            cell_size *= 2.0;
            gap_size *= 2.0;
            period = cell_size + gap_size;
        }

        let cell_color: [f32; 4] = wm.layout.desktop_cell_color;
        let cell_corner_radius = wm.layout.desktop_cell_corner_radius;
        let cell_fade_inset = wm.layout.desktop_cell_fade_inset;
        let gap_color = wm.layout.desktop_gap_color.clone();

        let structure_changed = self.last_grid_viewport_w != viewport_w
            || self.last_grid_viewport_h != viewport_h
            || self.last_grid_zoom != zoom
            || self.last_grid_spacing != cell_size
            || self.last_grid_gap_width != gap_size as i32
            || self.last_grid_cell_color != cell_color
            || self.last_grid_cell_corner_radius != cell_corner_radius
            || self.last_grid_cell_fade_inset != cell_fade_inset
            || self.last_grid_gap_color != gap_color;

        let force_high_res = !structure_changed && self.grid_is_low_res;

        let period_pixels = period * zoom;

        if structure_changed || force_high_res {
            let draw_low_res = self.last_grid_zoom != zoom;

            // Cache current parameters (excluding pan coordinates)
            self.last_grid_viewport_w = viewport_w;
            self.last_grid_viewport_h = viewport_h;
            self.last_grid_zoom = zoom;
            self.last_grid_spacing = cell_size;
            self.last_grid_gap_width = gap_size as i32;
            self.last_grid_cell_color = cell_color;
            self.last_grid_cell_corner_radius = cell_corner_radius;
            self.last_grid_cell_fade_inset = cell_fade_inset;
            self.last_grid_gap_color = gap_color;
            self.grid_is_low_res = draw_low_res;

            ffi::wlr_scene_node_raise_to_top(self.grid_tree as *mut ffi::wlr_scene_node);

            let grid_tree = self.grid_tree;
            let pool = &mut self.grid_rect_pool;
            let mut pool_idx = 0;

            let mut get_rect = |w: i32, h: i32, color: *const f32, x: i32, y: i32, r: i32| -> *mut ffi::wlr_scene_rect {
                let rect = if pool_idx < pool.len() {
                    let node = pool[pool_idx];
                    ffi::wlr_scene_node_set_enabled(node as *mut ffi::wlr_scene_node, true);
                    ffi::wlr_scene_rect_set_size(node, w, h);
                    ffi::wlr_scene_rect_set_color(node, color);
                    node
                } else {
                    let node = ffi::wlr_scene_rect_create(grid_tree, w, h, color);
                    if !node.is_null() {
                        pool.push(node);
                    }
                    node
                };
                if !rect.is_null() {
                    ffi::wlr_scene_node_set_position(rect as *mut ffi::wlr_scene_node, x, y);
                    ffi::river_scene_rect_set_corner_radius(rect, r);
                }
                pool_idx += 1;
                rect
            };

            // Draw the base level background rect (gap color)
            // Sized larger to allow shifting by up to one period
            let bg_w = viewport_w + period_pixels.ceil() as i32;
            let bg_h = viewport_h + period_pixels.ceil() as i32;
            let gap_color_arr = crate::config::parse_hex_color_rgba(&self.last_grid_gap_color);
            let _bg_rect = get_rect(bg_w, bg_h, gap_color_arr.as_ptr(), 0, 0, 0);

            // Determine grid range needed to cover viewport + shifting buffer
            let cols = (viewport_w as f64 / period_pixels).ceil() as i32 + 1;
            let rows = (viewport_h as f64 / period_pixels).ceil() as i32 + 1;

            for col in 0..=cols {
                let rel_x = (col as f64 * period_pixels) as i32;
                let rw = (cell_size * zoom) as i32;
                if rw <= 0 {
                    continue;
                }

                for row in 0..=rows {
                    let rel_y = (row as f64 * period_pixels) as i32;
                    let rh = (cell_size * zoom) as i32;
                    if rh <= 0 {
                        continue;
                    }
                    let draw_fade = cell_fade_inset > 0 && !draw_low_res;
                    let cell_rect = get_rect(rw, rh, cell_color.as_ptr(), rel_x, rel_y, cell_corner_radius);
                    if !cell_rect.is_null() {
                        let inset_scaled = if draw_fade {
                            (cell_fade_inset as f64 * zoom) as i32
                        } else {
                            0
                        };
                        ffi::wlr_scene_rect_set_fade_inset(cell_rect, inset_scaled);
                    }
                }
            }

            // Disable any remaining rects in the pool
            for i in pool_idx..pool.len() {
                ffi::wlr_scene_node_set_enabled(pool[i] as *mut ffi::wlr_scene_node, false);
            }
        }

        // Apply modulo translation to the grid tree based on panning
        let shift_x = -((wm.desk_pan_x * zoom).rem_euclid(period_pixels));
        let shift_y = -((wm.desk_pan_y * zoom).rem_euclid(period_pixels));

        ffi::river_scene_node_set_position_if_changed(
            self.grid_tree as *mut ffi::wlr_scene_node,
            shift_x as i32,
            shift_y as i32,
        );

        self.last_grid_pan_x = wm.desk_pan_x;
        self.last_grid_pan_y = wm.desk_pan_y;
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let output = crate::container_of!(listener, Output, destroy);

    log::debug!("Output destroyed");

    // Remove listeners
    wl_listener_remove(&mut (*output).destroy);
    wl_listener_remove(&mut (*output).request_state);
    wl_listener_remove(&mut (*output).frame);
    wl_listener_remove(&mut (*output).present);

    if !(*output).background_rect.is_null() {
        ffi::wlr_scene_node_destroy((*output).background_rect as *mut ffi::wlr_scene_node);
        (*output).background_rect = std::ptr::null_mut();
    }

    if !(*output).grid_tree.is_null() {
        ffi::wlr_scene_node_destroy((*output).grid_tree as *mut ffi::wlr_scene_node);
        (*output).grid_tree = std::ptr::null_mut();
    }

    if !(*output).wlr_output.is_null() {
        ffi::river_wlr_output_set_data((*output).wlr_output, std::ptr::null_mut());
    }

    (*output).wlr_output = std::ptr::null_mut();
    (*output).scene_output = std::ptr::null_mut();
    (*output).scheduled.mode = OutputMode::None;
    (*output).sent.mode = OutputMode::None;
    (*output).current.mode = OutputMode::None;
    (*output).scheduled.state = OutputStateValue::Destroying;

    (*(*output).server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_request_state(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let output = &mut *crate::container_of!(listener, Output, request_state);
    let event = data as *mut ffi::wlr_output_event_request_state;

    let committed: u32 = std::mem::transmute((*(*event).state).committed);
    // mode field is bit 1 (mode)
    if committed & 1 != 0 {
        if !(*(*event).state).mode.is_null() {
            output.scheduled.mode = OutputMode::Standard((*(*event).state).mode);
        } else {
            output.scheduled.mode = OutputMode::Custom {
                width: (*(*event).state).custom_mode.width,
                height: (*(*event).state).custom_mode.height,
                refresh: (*(*event).state).custom_mode.refresh,
            };
        }
    }

    (*output.server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_frame(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let output = &mut *crate::container_of!(listener, Output, frame);
    if let Err(e) = output.render_and_commit() {
        log::error!("{}", e);
    }
    let now = util::timestamp();
    let mut ffi_now = ffi::timespec {
        tv_sec: now.tv_sec,
        tv_nsec: now.tv_nsec,
    };
    ffi::wlr_scene_output_send_frame_done(output.scene_output, &mut ffi_now);
}

unsafe extern "C" fn handle_present(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let output = &mut *crate::container_of!(listener, Output, present);
    let event = data as *mut ffi::wlr_output_event_present;
    if !(*event).presented {
        return;
    }
    match output.lock_render_state {
        LockRenderState::PendingUnlock => {
            output.lock_render_state = LockRenderState::Unlocked;
        }
        LockRenderState::PendingBlank => {
            output.lock_render_state = LockRenderState::Blanked;
            (*output.server).lock_manager.maybe_lock();
        }
        LockRenderState::PendingLockSurface => {
            output.lock_render_state = LockRenderState::LockSurface;
            (*output.server).lock_manager.maybe_lock();
        }
        _ => {}
    }
}

// Helpers for raw Wayland FFI protocol events
pub unsafe fn zcce_output_send_removed(resource: *mut ffi::wl_resource) {
    ffi::wl_resource_post_event(resource, 0);
}

pub unsafe fn zcce_output_send_wl_output(resource: *mut ffi::wl_resource, name: u32) {
    ffi::wl_resource_post_event(resource, 1, name);
}

pub unsafe fn zcce_output_send_position(resource: *mut ffi::wl_resource, x: i32, y: i32) {
    ffi::wl_resource_post_event(resource, 2, x, y);
}

pub unsafe fn zcce_output_send_dimensions(resource: *mut ffi::wl_resource, width: i32, height: i32) {
    ffi::wl_resource_post_event(resource, 3, width, height);
}
