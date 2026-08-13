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
    pub adjust_tree: *mut ffi::wlr_scene_tree,
    pub adjust_rects: Vec<*mut ffi::wlr_scene_rect>,
    pub last_adjust_mode: bool,
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
    /// Camera state at this output's last rendered frame; any difference
    /// forces a full-output repaint (see the frame chokepoint).
    pub last_rendered_pan_x: f64,
    pub last_rendered_pan_y: f64,
    pub last_rendered_zoom: f64,
    pub last_grid_viewport_w: i32,
    pub last_grid_viewport_h: i32,
    pub last_grid_zoom: f64,
    /// The spec the pool was last drawn from; a change (or viewport/zoom
    /// change) forces a redraw. Pan alone never redraws — it only moves the
    /// grid tree.
    pub last_grid_spec: Option<crate::policy::api::BackgroundSpec>,
    pub grid_rect_pool: Vec<*mut ffi::wlr_scene_rect>,
    pub grid_force_redraw_frames: u8,

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

                    if !self.adjust_tree.is_null() {
                        ffi::wlr_scene_node_destroy(self.adjust_tree as *mut ffi::wlr_scene_node);
                        self.adjust_tree = std::ptr::null_mut();
                    }
                    self.adjust_rects.clear();

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
            adjust_tree: std::ptr::null_mut(),
            adjust_rects: Vec::new(),
            last_adjust_mode: false,
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
            last_rendered_pan_x: f64::NAN,
            last_rendered_pan_y: f64::NAN,
            last_rendered_zoom: f64::NAN,
            last_grid_viewport_w: 0,
            last_grid_viewport_h: 0,
            last_grid_zoom: 0.0,
            last_grid_spec: None,
            grid_rect_pool: Vec::new(),
            grid_force_redraw_frames: 0,
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
        // Update grid node positions and parameters first, which marks the scene output as damaged if changed
        self.draw_grid();
        self.draw_adjust_overlay();

        // A parked `ccectl screenshot` targeting this output forces a render
        // even without damage so there is a fresh buffer to read back.
        let pending_shot = {
            let wm = &mut (*self.server).wm;
            if wm
                .pending_screenshot
                .as_ref()
                .map(|s| s.output == self as *mut Output)
                .unwrap_or(false)
            {
                wm.pending_screenshot.take()
            } else {
                None
            }
        };

        // One chokepoint for every camera-mutation path (wheel zoom, IPC,
        // keyed actions, edge-pan, the pan animation — whichever of
        // update_viewport_local or the manage transaction carried it): if
        // the camera changed since this output last rendered, per-node
        // damage under-reports the whole-screen relayout (stale slivers of
        // the previous zoom survive wherever idle content used to be), so
        // force a full repaint. Must run BEFORE the needs-frame early-out —
        // a camera change with no other pending damage would otherwise skip
        // the frame entirely.
        {
            let wm = &(*self.server).wm;
            let cam = (wm.desk_pan_x, wm.desk_pan_y, wm.desk_zoom);
            if cam != (self.last_rendered_pan_x, self.last_rendered_pan_y, self.last_rendered_zoom) {
                self.last_rendered_pan_x = cam.0;
                self.last_rendered_pan_y = cam.1;
                self.last_rendered_zoom = cam.2;
                if !self.scene_output.is_null() {
                    ffi::river_scene_output_damage_whole(self.scene_output);
                }
            }
        }

        if pending_shot.is_none() && !ffi::wlr_scene_output_needs_frame(self.scene_output) {
            return Ok(());
        }

        // Re-apply scale to all windows whose scale is not 1.0 right before rendering
        let wm = &(*self.server).wm;
        for &window in wm.windows.iter() {
            if !window.is_null() && (*window).scale != 1.0 {
                (*window).scale_only_render_finish();
            }
        }

        // Overview-delay debugging: while /tmp/cce-ovdbg exists (contents =
        // comma-separated app_id substrings), dump the scene-side truth for
        // matching windows every rendered frame. Toggle live with
        // `echo firefox,cce-calendar > /tmp/cce-ovdbg`; `rm` to stop.
        if let Ok(filter) = std::fs::read_to_string("/tmp/cce-ovdbg") {
            let pats: Vec<&str> = filter.trim().split(',').filter(|p| !p.is_empty()).collect();
            for &window in wm.windows.iter() {
                if window.is_null() || (*window).closed {
                    continue;
                }
                let app = (*window).get_app_id_string().unwrap_or_default();
                if !pats.iter().any(|p| app.contains(p)) {
                    continue;
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                eprintln!(
                    "[ovdbg] t={}.{:03} win {} scale={} req=({},{}) box=({},{}) state={:?} hidden={} saved={} tree_en={}",
                    now.as_secs(),
                    now.subsec_millis(),
                    app,
                    (*window).scale,
                    (*window).rendering_requested.x,
                    (*window).rendering_requested.y,
                    (*window).box_geom.x,
                    (*window).box_geom.y,
                    (*window).state,
                    (*window).rendering_requested.hidden,
                    (*window).surfaces.saved,
                    ffi::river_scene_node_get_enabled((*window).tree as *mut ffi::wlr_scene_node),
                );
                if let Ok(tag) = std::ffi::CString::new(app) {
                    ffi::river_scene_shadow_dbg((*window).shadow, tag.as_ptr());
                    ffi::river_scene_ovdbg_dump(
                        (*window).tree as *mut ffi::wlr_scene_node,
                        tag.as_ptr(),
                    );
                }
            }
        }

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

        // Read the just-committed frame back while the state's buffer is
        // still alive; encode/notify happen on a worker thread.
        if let Some(shot) = pending_shot {
            if state.buffer.is_null() {
                log::warn!("screenshot: output state has no buffer");
            } else {
                crate::screenshot::capture_state_buffer(
                    (*self.server).renderer,
                    state.buffer,
                    ffi::river_wlr_output_get_width(self.wlr_output),
                    ffi::river_wlr_output_get_height(self.wlr_output),
                    shot,
                );
            }
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

    pub unsafe fn draw_adjust_overlay(&mut self) {
        if self.adjust_tree.is_null() {
            return;
        }

        let wm = &(*self.server).wm;
        if wm.adjust_position_mode != self.last_adjust_mode {
            self.last_adjust_mode = wm.adjust_position_mode;
            ffi::wlr_output_schedule_frame(self.wlr_output);
        }

        if !wm.adjust_position_mode {
            ffi::wlr_scene_node_set_enabled(self.adjust_tree as *mut ffi::wlr_scene_node, false);
            return;
        }

        // Enable the overlay tree.
        ffi::wlr_scene_node_set_enabled(self.adjust_tree as *mut ffi::wlr_scene_node, true);
        ffi::wlr_scene_node_lower_to_bottom(self.adjust_tree as *mut ffi::wlr_scene_node);

        let (viewport_w, viewport_h) = self.current.dimensions();
        let w = viewport_w;
        let h = viewport_h;

        // Semicircles size: 120 x 120 (so radius = 60).
        let targets = [
            // TopLeft (nw): x = 0, y = -60
            (0, -60),
            // TopCenter (n): x = w/2 - 60, y = -60
            (w / 2 - 60, -60),
            // TopRight (ne): x = w - 120, y = -60
            (w - 120, -60),
            // BottomLeft (sw): x = 0, y = h - 60
            (0, h - 60),
            // BottomCenter (s): x = w/2 - 60, y = h - 60
            (w / 2 - 60, h - 60),
            // BottomRight (se): x = w - 120, y = h - 60
            (w - 120, h - 60),
            // Left (w): x = -60, y = h/2 - 60
            (-60, h / 2 - 60),
            // Right (e): x = w - 60, y = h/2 - 60
            (w - 60, h / 2 - 60),
        ];

        let color: [f32; 4] = [0.4, 0.6, 0.9, 0.5];
        let color_ptr = color.as_ptr();

        for (idx, &(tx, ty)) in targets.iter().enumerate() {
            let rect = if idx < self.adjust_rects.len() {
                let node = self.adjust_rects[idx];
                ffi::wlr_scene_node_set_enabled(node as *mut ffi::wlr_scene_node, true);
                ffi::wlr_scene_rect_set_size(node, 120, 120);
                ffi::wlr_scene_rect_set_color(node, color_ptr);
                node
            } else {
                let node = ffi::wlr_scene_rect_create(self.adjust_tree, 120, 120, color_ptr);
                if !node.is_null() {
                    self.adjust_rects.push(node);
                    // Make it circular!
                    ffi::river_scene_rect_set_corner_radius(node, 60);
                }
                node
            };

            if !rect.is_null() {
                ffi::wlr_scene_node_set_position(rect as *mut ffi::wlr_scene_node, tx, ty);
            }
        }

        // Disable any extra rects in the pool if we somehow have more
        for idx in targets.len()..self.adjust_rects.len() {
            let node = self.adjust_rects[idx];
            ffi::wlr_scene_node_set_enabled(node as *mut ffi::wlr_scene_node, false);
        }
    }

    /// Draw the desktop background from the policy crate's declarative
    /// spec: `Layout::background_spec()` says WHAT to show, and (for the
    /// grid) `policy::background::grid_frame` derives this frame's geometry
    /// — tree shift, backdrop extent, cell lattice, density fade. This side
    /// keeps the scene nodes, the rect reuse pool, and scenefx's fade-inset
    /// wire encoding.
    pub unsafe fn draw_grid(&mut self) {
        if self.grid_tree.is_null() {
            return;
        }

        let wm = &(*self.server).wm;

        // Enable the grid tree.
        ffi::wlr_scene_node_set_enabled(self.grid_tree as *mut ffi::wlr_scene_node, true);

        // Keep the grid tree at the top of the background layer to prevent wallpaper windows from overlapping it
        ffi::wlr_scene_node_raise_to_top(self.grid_tree as *mut ffi::wlr_scene_node);

        let (viewport_w, viewport_h) = self.current.dimensions();
        let spec = wm.layout.background_spec();
        let zoom = crate::policy::background::sanitized_zoom(wm.desk_zoom);

        // Spec/viewport/zoom changes force a redraw of the rect pool; pan
        // alone only moves the grid tree.
        let structure_changed = self.last_grid_viewport_w != viewport_w
            || self.last_grid_viewport_h != viewport_h
            || self.last_grid_zoom != zoom
            || self.last_grid_spec.as_ref() != Some(&spec);
        if structure_changed {
            self.grid_force_redraw_frames = 3;
        }
        let force = self.grid_force_redraw_frames > 0;
        if force {
            self.grid_force_redraw_frames -= 1;
            self.last_grid_viewport_w = viewport_w;
            self.last_grid_viewport_h = viewport_h;
            self.last_grid_zoom = zoom;
            self.last_grid_spec = Some(spec.clone());
        }

        let grid_tree = self.grid_tree;
        let pool = &mut self.grid_rect_pool;
        let mut pool_idx = 0;

        // Helper closure to manage/reuse the pool of wlr_scene_rect elements.
        let mut get_rect = |w: i32, h: i32, color_ptr: *const f32, x: i32, y: i32, corner_r: i32, fade_i: i32| -> *mut ffi::wlr_scene_rect {
            let rect = if pool_idx < pool.len() {
                let node = pool[pool_idx];
                ffi::wlr_scene_node_set_enabled(node as *mut ffi::wlr_scene_node, true);
                ffi::wlr_scene_rect_set_size(node, w, h);
                ffi::wlr_scene_rect_set_color(node, color_ptr);
                node
            } else {
                let node = ffi::wlr_scene_rect_create(grid_tree, w, h, color_ptr);
                if !node.is_null() {
                    pool.push(node);
                }
                node
            };

            if !rect.is_null() {
                ffi::wlr_scene_node_set_position(rect as *mut ffi::wlr_scene_node, x, y);
                ffi::river_scene_rect_set_corner_radius(rect, corner_r);
                ffi::wlr_scene_rect_set_fade_inset(rect, fade_i);
            }
            pool_idx += 1;
            rect
        };

        match &spec {
            crate::policy::api::BackgroundSpec::Grid(grid) => {
                let frame = crate::policy::background::grid_frame(
                    grid,
                    wm.camera(),
                    viewport_w,
                    viewport_h,
                    self.sent.x,
                    self.sent.y,
                );

                if let Some((x, y)) = frame.tree_pos {
                    ffi::river_scene_node_set_position_if_changed(
                        grid_tree as *mut ffi::wlr_scene_node,
                        x,
                        y,
                    );
                }

                if force {
                    // Backdrop in the gap color, then the cell lattice.
                    get_rect(frame.backdrop_w, frame.backdrop_h, grid.gap_color.0.as_ptr(), 0, 0, 0, 0);

                    if let Some(cells) = &frame.cells {
                        // scenefx fade-inset wire encoding: inset px * 1000
                        // + fade-mode index; 0 disables the fade.
                        use crate::policy::api::GridFadeMode;
                        let fade_mode = match grid.fade_mode {
                            GridFadeMode::Linear => 0,
                            GridFadeMode::Smoothstep => 1,
                            GridFadeMode::Quadratic => 2,
                            GridFadeMode::Cosine => 3,
                            GridFadeMode::Gaussian => 4,
                        };
                        let inset_scaled = if cells.fade_inset_px > 0 {
                            cells.fade_inset_px * 1000 + fade_mode
                        } else {
                            0
                        };
                        // Cell positions from the EXACT period, rounded per
                        // cell: a rounded-period spacing drifts from the
                        // world-anchored windows at fractional zooms (the
                        // grid visibly slides against window edges when
                        // panning).
                        for col in 0..=cells.cols {
                            let rel_x = (col as f64 * frame.period_px_exact).round() as i32;
                            for row in 0..=cells.rows {
                                let rel_y = (row as f64 * frame.period_px_exact).round() as i32;
                                get_rect(cells.cell_px, cells.cell_px, cells.color.0.as_ptr(), rel_x, rel_y, cells.corner_radius_px, inset_scaled);
                            }
                        }
                    }
                }
            }
            crate::policy::api::BackgroundSpec::Solid(color) => {
                ffi::river_scene_node_set_position_if_changed(
                    grid_tree as *mut ffi::wlr_scene_node,
                    self.sent.x,
                    self.sent.y,
                );
                if force {
                    get_rect(viewport_w, viewport_h, color.0.as_ptr(), 0, 0, 0, 0);
                }
            }
        }

        if force {
            // Disable unused rects in the pool to release GPU/scene resources.
            for i in pool_idx..pool.len() {
                ffi::wlr_scene_node_set_enabled(pool[i] as *mut ffi::wlr_scene_node, false);
            }
        }
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

    if !(*output).scene_output.is_null() {
        ffi::wlr_scene_output_destroy((*output).scene_output);
        (*output).scene_output = std::ptr::null_mut();
    }

    if !(*output).background_rect.is_null() {
        ffi::wlr_scene_node_destroy((*output).background_rect as *mut ffi::wlr_scene_node);
        (*output).background_rect = std::ptr::null_mut();
    }

    if !(*output).grid_tree.is_null() {
        ffi::wlr_scene_node_destroy((*output).grid_tree as *mut ffi::wlr_scene_node);
        (*output).grid_tree = std::ptr::null_mut();
    }

    if !(*output).adjust_tree.is_null() {
        ffi::wlr_scene_node_destroy((*output).adjust_tree as *mut ffi::wlr_scene_node);
        (*output).adjust_tree = std::ptr::null_mut();
    }
    (*output).adjust_rects.clear();

    if !(*output).wlr_output.is_null() {
        ffi::river_wlr_output_set_data((*output).wlr_output, std::ptr::null_mut());
    }

    (*output).wlr_output = std::ptr::null_mut();
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

/// `CCE_FRAME_DEBUG` (any value) also ticks every output frame, so a client's
/// frame-callback interval can be compared against the rate the output is
/// actually rendering at — the two diverging is the signature of a surface
/// being skipped by the scene's visible gate in `wlr_scene_buffer_send_frame_done`.
pub(crate) fn frame_debug() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCE_FRAME_DEBUG").is_some())
}

unsafe extern "C" fn handle_frame(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let output = &mut *crate::container_of!(listener, Output, frame);
    let render_start = if frame_debug() {
        Some(std::time::Instant::now())
    } else {
        None
    };
    if let Err(e) = output.render_and_commit() {
        log::error!("{}", e);
    }
    if let Some(start) = render_start {
        // Epoch ms mod 100000 — the shared tracer time base (see cce-ui's
        // CCE_PRESENT_DEBUG), so compositor and client logs interleave.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            % 100000;
        log::info!(
            "[cce-frame] t={} output frame (render_and_commit {}us)",
            t,
            start.elapsed().as_micros()
        );
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
