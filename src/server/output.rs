// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlList, wl_list_insert, wl_list_remove};
use crate::layer_shell::LayerShellOutput;
use crate::lock_manager::LockState;
use crate::util;
use crate::scene_handle::{SceneBevel, SceneBuffer, SceneRect, SceneTree};

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
    pub background_rect: SceneRect,
    pub grid_tree: SceneTree,
    /// The grid's backdrop (gap colour, or the Solid spec's colour), kept in
    /// its own tree under `scene.layers.background_clients` so a client
    /// background surface paints over it while the cells in `grid_tree` stay
    /// on top. Positioned in lockstep with `grid_tree`.
    pub grid_backdrop_tree: SceneTree,
    pub grid_backdrop_rect: SceneRect,
    pub adjust_tree: SceneTree,
    pub adjust_rects: Vec<SceneRect>,
    pub last_adjust_mode: bool,
    pub layer_shell: LayerShellOutput,
    pub lock_render_state: LockRenderState,
    pub link: ffi::wl_list,
    pub link_sent: ffi::wl_list,
    pub scheduled: OutputState,
    pub sent: OutputState,
    pub current: OutputState,
    pub rendering_requested: RenderingState,
    pub rendering_current: RenderingState,

    // Cached grid parameters to avoid redrawing when unchanged
    /// Camera state at this output's last rendered frame; any difference
    /// forces a full-output repaint (see the frame chokepoint).
    pub last_rendered_pan_x: f64,
    pub last_rendered_pan_y: f64,
    pub last_rendered_zoom: f64,
    /// Presentation clock, from the `present` event: when the last frame
    /// turned into light (CLOCK_MONOTONIC ns, 0 = never), its vblank
    /// sequence number (0 = the backend has none), and the refresh period.
    /// `predicted_present_ns` derives the camera's frame clock from these.
    pub present_when_ns: u64,
    pub present_seq: u32,
    pub present_refresh_ns: u64,
    /// Running count of vblanks skipped between consecutive presents —
    /// the dropped-frame counter `CCE_FRAME_DEBUG` reports.
    pub present_dropped: u64,
    /// Phase of the frame clock's vblank grid (ns within a refresh period),
    /// locked to the presentation times; `u64::MAX` until the first frame.
    pub frame_phase_ns: u64,
    /// A tearing page-flip test failed for the current tearing episode; do
    /// not repeat the atomic TEST_ONLY commit every frame. Cleared when the
    /// fullscreen client's tearing request goes away.
    pub tearing_test_failed: bool,
    /// Darkened by the idle timeout (`IdleManager::set_displays`), so the
    /// next activity wakes this one and leaves a client-darkened output alone.
    pub idle_off: bool,
    pub last_grid_viewport_w: i32,
    pub last_grid_viewport_h: i32,
    pub last_grid_zoom: f64,
    /// The spec the pool was last drawn from; a change (or viewport/zoom
    /// change) forces a redraw. Pan alone never redraws — it only moves the
    /// grid tree.
    pub last_grid_spec: Option<crate::policy::api::BackgroundSpec>,
    pub grid_rect_pool: Vec<SceneRect>,
    /// Lit-chamfer rims over the grid cells — the same scenefx bevel node
    /// the windows use, so the grid lines read as raised rails descending
    /// into each cell through a shaded fillet that wraps the corner arcs.
    /// Pooled like `grid_rect_pool`, but in their own subtree kept above
    /// the rects: pool reuse must never stack a rim beneath a
    /// later-created cell rect.
    pub grid_bevel_pool: Vec<SceneBevel>,
    pub grid_bevel_tree: SceneTree,
    /// The cell labels' own subtree inside `grid_tree`, raised above the
    /// cell rects and bevels on every draw: label nodes used to be direct
    /// children of the grid tree, so any cell rect the pool created after
    /// them stacked on top and hid them.
    pub cell_label_tree: SceneTree,
    /// Bevel params the rims were last drawn with (enabled, thickness,
    /// light x/y/intensity, shade, shoulder as bits) — the spec alone does
    /// not cover them, and a live config reload must redraw the rims too.
    pub last_grid_bevel: Option<[u32; 7]>,
    pub grid_force_redraw_frames: u8,
    /// Scene nodes for the per-square chess-style labels (overview only), and
    /// the rasterized glyph buffers behind them. Pooled exactly like
    /// `grid_rect_pool`: reused across frames, disabled past the live count.
    /// Each entry remembers the buffer it currently shows, because
    /// `wlr_scene_buffer_set_buffer` damages the node unconditionally — even
    /// when handed the buffer already on it — and these are re-walked every
    /// frame while the overview camera moves.
    pub cell_label_pool: Vec<(SceneBuffer, *mut ffi::wlr_buffer)>,
    pub cell_labels: crate::text::LabelCache,
    /// Label point size actually in use, so a zoom change can re-rasterize.
    pub last_label_px: u32,

    pub destroy: crate::listener::Listener,
    pub request_state: crate::listener::Listener,
    pub frame: crate::listener::Listener,
    pub present: crate::listener::Listener,
}

impl Output {
    pub unsafe fn manage_start(&mut self) {
        match self.scheduled.state {
            OutputStateValue::Enabled | OutputStateValue::DisabledSoft => {
                assert!(!self.scheduled.mode_none());

                let self_ptr = self as *mut Output;
                let layer_shell_ptr = &mut self.layer_shell as *mut LayerShellOutput;
                (*layer_shell_ptr).manage_start(self_ptr);

                self.sent = self.scheduled;

                wl_list_remove(&mut self.link_sent as *mut ffi::wl_list as *mut WlList);
                let sent_outputs = &mut (*self.server).wm.sent.outputs as *mut ffi::wl_list as *mut WlList;
                wl_list_insert((*sent_outputs).prev, &mut self.link_sent as *mut ffi::wl_list as *mut WlList);
            }
            OutputStateValue::DisabledHard | OutputStateValue::Destroying => {
                self.sent = self.scheduled;

                if self.scheduled.state == OutputStateValue::Destroying {
                    assert!(self.wlr_output.is_null());
                    
                    self.destroy_scene_nodes();

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

        // The physical size every client's wl_output geometry will carry,
        // which cce-ui's `units::Metric` measures logical px per mm against.
        // A configured `size_mm` replaces the EDID figure before the global
        // exists (wlr_output_layout_add creates it), so no client ever sees
        // the lie. Logged either way: an output with no size at all leaves
        // clients on the assumed 96 ppi, and that is worth knowing.
        let (mut edid_w, mut edid_h) = (0i32, 0i32);
        ffi::river_wlr_output_get_phys_size(wlr_output, &mut edid_w, &mut edid_h);
        let configured = (*server).wm.display.get(&format!("mm_w_{}", name))
            .zip((*server).wm.display.get(&format!("mm_h_{}", name)))
            .map(|(&w, &h)| (w.round() as i32, h.round() as i32));
        match configured {
            Some((w, h)) => {
                ffi::river_wlr_output_set_phys_size(wlr_output, w, h);
                log::info!("output {}: physical size {}x{} mm (configured size_mm; EDID said {}x{})", name, w, h, edid_w, edid_h);
            }
            None if edid_w > 0 && edid_h > 0 => {
                log::info!("output {}: physical size {}x{} mm (EDID)", name, edid_w, edid_h);
            }
            None => {
                log::info!("output {}: no physical size — clients assume 96 ppi; set `output {{ {} size_mm=\"WxH\" }}` to measure", name, name);
            }
        }

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
            background_rect: SceneRect::none(),
            grid_tree: SceneTree::none(),
            grid_backdrop_tree: SceneTree::none(),
            grid_backdrop_rect: SceneRect::none(),
            adjust_tree: SceneTree::none(),
            adjust_rects: Vec::new(),
            last_adjust_mode: false,
            layer_shell: LayerShellOutput::default(),
            lock_render_state: LockRenderState::Blanked,
            link: std::mem::zeroed(),
            link_sent: std::mem::zeroed(),
            scheduled: initial,
            sent: initial,
            current: initial,
            rendering_requested: RenderingState { tearing: false },
            rendering_current: RenderingState { tearing: false },
            last_rendered_pan_x: f64::NAN,
            last_rendered_pan_y: f64::NAN,
            last_rendered_zoom: f64::NAN,
            present_when_ns: 0,
            present_seq: 0,
            present_refresh_ns: 0,
            present_dropped: 0,
            frame_phase_ns: u64::MAX,
            tearing_test_failed: false,
            idle_off: false,
            last_grid_viewport_w: 0,
            last_grid_viewport_h: 0,
            last_grid_zoom: 0.0,
            last_grid_spec: None,
            grid_rect_pool: Vec::new(),
            grid_bevel_pool: Vec::new(),
            grid_bevel_tree: SceneTree::none(),
            cell_label_tree: SceneTree::none(),
            last_grid_bevel: None,
            grid_force_redraw_frames: 0,
            cell_label_pool: Vec::new(),
            cell_labels: Default::default(),
            last_label_px: 0,
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
        (*raw).destroy.connect(ffi::river_wlr_output_get_destroy_signal(wlr_output), handle_destroy);

        (*raw).request_state.connect(ffi::river_wlr_output_get_request_state_signal(wlr_output), handle_request_state);

        (*raw).frame.connect(ffi::river_wlr_output_get_frame_signal(wlr_output), handle_frame);

        (*raw).present.connect(ffi::river_wlr_output_get_present_signal(wlr_output), handle_present);

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
        (*self.server).wm.draw_selection();
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
            // Quantized to screen pixels: a sub-pixel pan moves no node
            // (see `update_viewport_local`), so it is not a reason to paint.
            let wm = &(*self.server).wm;
            let q = wm.desk_zoom * self.current.scale as f64;
            let cam = ((wm.desk_pan_x * q).round(), (wm.desk_pan_y * q).round(), wm.desk_zoom);
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

        // Re-apply scale to all windows whose scale is not 1.0 right before
        // rendering — and to any whose buffers are still shrunk from a
        // scale that has just returned to 1.0, which this pass resets.
        let wm = &(*self.server).wm;
        for &window in wm.windows.iter() {
            if !window.is_null() && ((*window).scale != 1.0 || (*window).x11_buffer_scale() != 1.0 || (*window).buffers_scaled) {
                (*window).scale_only_render_finish();
            }
        }
        for &or in wm.override_redirects.iter() {
            if !or.is_null() {
                (*or).apply_x11_scale();
            }
        }

        // Overview-delay debugging: with `CCE_OVDBG=1` in the compositor's
        // environment, while /tmp/cce-ovdbg exists (contents = comma-separated
        // app_id substrings), dump the scene-side truth for matching windows
        // every rendered frame. Toggle live with
        // `echo firefox,cce-calendar > /tmp/cce-ovdbg`; `rm` to stop. The env
        // gate is what keeps a release build from doing an open()+read() of
        // that path on every frame it ever renders.
        if ovdbg_enabled() {
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
        }

        let mut state = std::mem::zeroed();
        ffi::wlr_output_state_init(&mut state);
        
        self.current.apply_no_modeset(&mut state);

        if !ffi::wlr_scene_output_build_state(self.scene_output, &mut state, std::ptr::null()) {
            ffi::wlr_output_state_finish(&mut state);
            return Err("Failed to build scene state");
        }

        if self.rendering_current.tearing {
            // The test is an atomic TEST_ONLY commit; once it has said no for
            // this episode, asking again every frame just taxes the game.
            if !self.tearing_test_failed {
                state.tearing_page_flip = true;
                if !ffi::wlr_output_test_state(self.wlr_output, &state) {
                    state.tearing_page_flip = false;
                    self.tearing_test_failed = true;
                }
            }
        } else {
            self.tearing_test_failed = false;
        }

        if !ffi::wlr_output_commit_state(self.wlr_output, &state) {
            ffi::wlr_output_state_finish(&mut state);
            // The damage this frame carried is still pending (only a
            // successful commit clears it), but nothing else asks for
            // another frame: a refused commit (the panel's EBUSY bursts)
            // left the screen stale until something unrelated moved. Retry
            // at the next vblank.
            ffi::wlr_output_schedule_frame(self.wlr_output);
            return Err("Failed to commit state");
        }

        // Read the just-committed frame back while the state's buffer is
        // still alive; encode/notify happen on a worker thread.
        if let Some(mut shot) = pending_shot {
            if state.buffer.is_null() {
                log::warn!("screenshot: output state has no buffer");
                shot.reply_err("screenshot: output state has no buffer");
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

    /// Destroy this output's scene nodes: the base rect, the grid (whose
    /// pools, rims and labels go with it), its backdrop and the adjust
    /// overlay. The pools' handles read null once their parent tree is
    /// gone, so clearing them afterwards destroys nothing twice.
    pub fn destroy_scene_nodes(&mut self) {
        self.background_rect.destroy();
        self.grid_tree.destroy();
        self.grid_backdrop_tree.destroy();
        self.grid_backdrop_rect.destroy();
        self.grid_rect_pool.clear();
        self.grid_bevel_pool.clear();
        self.grid_bevel_tree.destroy();
        self.cell_label_pool.clear();
        self.cell_label_tree.destroy();
        self.adjust_tree.destroy();
        self.adjust_rects.clear();
    }

    pub unsafe fn update_background_color(&mut self) {
        let wm = &(*self.server).wm;
        let color: [f32; 4] = [
            (wm.layout.background_r as f64 / u32::MAX as f64) as f32,
            (wm.layout.background_g as f64 / u32::MAX as f64) as f32,
            (wm.layout.background_b as f64 / u32::MAX as f64) as f32,
            (wm.layout.background_a as f64 / u32::MAX as f64) as f32,
        ];
        self.background_rect.set_color(&color);
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
            self.adjust_tree.set_enabled(false);
            return;
        }

        // Enable the overlay tree.
        self.adjust_tree.set_enabled(true);
        self.adjust_tree.lower_to_bottom();

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

        for (idx, &(tx, ty)) in targets.iter().enumerate() {
            if idx < self.adjust_rects.len() {
                let rect = &self.adjust_rects[idx];
                rect.set_enabled(true);
                rect.set_size(120, 120);
                rect.set_color(&color);
                rect.set_position(tx, ty);
            } else {
                let rect = SceneRect::create(&self.adjust_tree, 120, 120, &color);
                if rect.is_null() {
                    continue;
                }
                // Make it circular!
                ffi::river_scene_rect_set_corner_radius(rect.raw(), 60);
                rect.set_position(tx, ty);
                self.adjust_rects.push(rect);
            }
        }

        // Disable any extra rects in the pool if we somehow have more
        for rect in &self.adjust_rects[targets.len().min(self.adjust_rects.len())..] {
            rect.set_enabled(false);
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
        self.grid_tree.set_enabled(true);

        // Keep the grid tree (cells + rims) at the top of the background layer,
        // above the client backgrounds in layers.background_clients.
        self.grid_tree.raise_to_top();

        // The backdrop goes BELOW the client backgrounds: its own tree, placed
        // just above this output's base rect (or at the very bottom), so a
        // layer-shell Background surface or a wallpaper window replaces the flat
        // colour and keeps the cell lattice.
        if self.grid_backdrop_tree.is_null() {
            self.grid_backdrop_tree = SceneTree::create_in((*self.server).scene.layers.background);
            if self.grid_backdrop_tree.is_null() {
                return;
            }
            ffi::river_scene_tree_set_desk_offset(self.grid_backdrop_tree.raw(), true);
            if !self.background_rect.is_null() {
                self.background_rect.lower_to_bottom();
                self.grid_backdrop_tree.place_above(&self.background_rect);
            } else {
                self.grid_backdrop_tree.lower_to_bottom();
            }
        }
        self.grid_backdrop_tree.set_enabled(true);
        let backdrop_tree = self.grid_backdrop_tree.raw();
        let backdrop_rect = &mut self.grid_backdrop_rect;
        let mut set_backdrop = |w: i32, h: i32, color_ptr: *const f32| {
            if backdrop_rect.is_null() {
                *backdrop_rect = SceneRect::adopt(ffi::wlr_scene_rect_create(backdrop_tree, w, h, color_ptr));
            } else {
                ffi::wlr_scene_rect_set_size(backdrop_rect.raw(), w, h);
                ffi::wlr_scene_rect_set_color(backdrop_rect.raw(), color_ptr);
            }
        };

        let (viewport_w, viewport_h) = self.current.dimensions();
        let spec = wm.layout.background_spec();
        let zoom = crate::policy::background::sanitized_zoom(wm.desk_zoom);

        // Spec/viewport/zoom/bevel changes force a redraw of the pools;
        // pan alone only moves the grid tree.
        let bevel_key = [
            wm.layout.bevel_enabled as u32,
            wm.layout.bevel_thickness.to_bits(),
            wm.layout.bevel_light_x.to_bits(),
            wm.layout.bevel_light_y.to_bits(),
            wm.layout.bevel_light_intensity.to_bits(),
            wm.layout.bevel_shade_intensity.to_bits(),
            wm.layout.bevel_shoulder.to_bits(),
        ];
        let structure_changed = self.last_grid_viewport_w != viewport_w
            || self.last_grid_viewport_h != viewport_h
            || self.last_grid_zoom != zoom
            || self.last_grid_spec.as_ref() != Some(&spec)
            || self.last_grid_bevel != Some(bevel_key);
        if structure_changed {
            self.grid_force_redraw_frames = 3;
            // Fallback-grid structure (spec/zoom/viewport) is backdrop
            // content in the optimized-blur capture set — same staleness
            // rule as the client-grid latch. Not while a camera gesture
            // holds the bakes frozen: the zoom restructures the grid every
            // frame, and the settle re-bakes once at the end.
            if !wm.viewport_is_active {
                ffi::river_scene_mark_optimized_blur_dirty((*self.server).scene.wlr_scene);
            }
        }
        let force = self.grid_force_redraw_frames > 0;
        if force {
            self.grid_force_redraw_frames -= 1;
            self.last_grid_viewport_w = viewport_w;
            self.last_grid_viewport_h = viewport_h;
            self.last_grid_zoom = zoom;
            self.last_grid_spec = Some(spec.clone());
            self.last_grid_bevel = Some(bevel_key);
        }

        // The cell rims live in their own subtree kept above every pooled
        // rect (incl. the backdrop), so reuse order can never bury one.
        if self.grid_bevel_tree.is_null() {
            self.grid_bevel_tree = SceneTree::create(&self.grid_tree);
        }
        self.grid_bevel_tree.raise_to_top();

        let grid_tree = self.grid_tree.raw();
        if !grid_tree.is_null() {
            // Desk content: rendered with the camera's sub-pixel offset.
            ffi::river_scene_tree_set_desk_offset(grid_tree, true);
        }
        let pool = &mut self.grid_rect_pool;
        let mut pool_idx = 0;

        let layout = &wm.layout;
        // The relief lives on the LINES, never the cells (mirroring the
        // cce-grid client, which must be able to latch without swapping the
        // grid's material): each cell's chamfer box is expanded by the
        // half-gap, so the lit wall occupies exactly the half-rail around
        // the cell — a crest at the rail centerline descending to the cell
        // edge — and neighboring rings abut without overlap. Cell floors
        // stay flat.
        let bevel_on = layout.bevel_enabled;
        // Light normalized exactly like the window bevels — the grid is lit
        // by the same lamp.
        let (bevel_lx, bevel_ly) = {
            let (lx, ly) = (layout.bevel_light_x, layout.bevel_light_y);
            let len = (lx * lx + ly * ly).sqrt();
            if len > 1e-6 { (lx / len, ly / len) } else { (-0.7071, -0.7071) }
        };
        let bevel_tree = self.grid_bevel_tree.raw();
        let bevel_pool = &mut self.grid_bevel_pool;
        let mut bevel_idx = 0;

        let mut get_bevel = |w: i32, h: i32, x: i32, y: i32, radius: i32, thickness: f32| {
            // An entry whose node went with its tree reads null: refill it.
            let bevel = if bevel_idx < bevel_pool.len() && !bevel_pool[bevel_idx].is_null() {
                let node = bevel_pool[bevel_idx].raw();
                ffi::wlr_scene_node_set_enabled(&mut (*node).node as *mut ffi::wlr_scene_node, true);
                ffi::wlr_scene_bevel_set_size(node, w, h);
                node
            } else {
                let handle = SceneBevel::create_in(bevel_tree, w, h, 0, 0.0, &layout.bevel_color);
                let node = handle.raw();
                if !node.is_null() {
                    if bevel_idx < bevel_pool.len() {
                        bevel_pool[bevel_idx] = handle;
                    } else {
                        bevel_pool.push(handle);
                    }
                }
                node
            };
            if !bevel.is_null() {
                ffi::wlr_scene_node_set_position(&mut (*bevel).node as *mut ffi::wlr_scene_node, x, y);
                ffi::wlr_scene_bevel_set_corner_radius(bevel, radius);
                ffi::wlr_scene_bevel_set_thickness(bevel, thickness.max(1.0));
                ffi::wlr_scene_bevel_set_light(
                    bevel,
                    bevel_lx,
                    bevel_ly,
                    layout.bevel_light_intensity,
                    layout.bevel_shade_intensity,
                );
                ffi::wlr_scene_bevel_set_shoulder(bevel, layout.bevel_shoulder);
                ffi::wlr_scene_bevel_set_color(bevel, layout.bevel_color.as_ptr());
            }
            bevel_idx += 1;
        };

        // Helper closure to manage/reuse the pool of wlr_scene_rect elements.
        let mut get_rect = |w: i32, h: i32, color_ptr: *const f32, x: i32, y: i32, corner_r: i32, fade_i: i32| -> *mut ffi::wlr_scene_rect {
            let rect = if pool_idx < pool.len() && !pool[pool_idx].is_null() {
                let node = pool[pool_idx].raw();
                ffi::wlr_scene_node_set_enabled(node as *mut ffi::wlr_scene_node, true);
                ffi::wlr_scene_rect_set_size(node, w, h);
                ffi::wlr_scene_rect_set_color(node, color_ptr);
                node
            } else {
                let handle = SceneRect::adopt(ffi::wlr_scene_rect_create(grid_tree, w, h, color_ptr));
                let node = handle.raw();
                if !node.is_null() {
                    if pool_idx < pool.len() {
                        pool[pool_idx] = handle;
                    } else {
                        pool.push(handle);
                    }
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

        // A forced rebuild resizes, recolours and moves every pooled cell
        // rect and rim — hundreds at overview zoom, every frame of a zoom
        // flight. Each setter on a live node re-walks the scene for the
        // region it touched; under a disabled ancestor it returns at once
        // (scene_node_update). So the tree is off while the pools are
        // redrawn and on again after: two walks for the whole rebuild.
        let suspended = force && !grid_tree.is_null();
        if suspended {
            ffi::wlr_scene_node_set_enabled(grid_tree as *mut ffi::wlr_scene_node, false);
        }

        match &spec {
            crate::policy::api::BackgroundSpec::Grid(grid) => {
                let frame = crate::policy::background::grid_frame(
                    grid,
                    wm.layout_camera().0,
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
                    ffi::river_scene_node_set_position_if_changed(
                        backdrop_tree as *mut ffi::wlr_scene_node,
                        x,
                        y,
                    );
                }

                if force {
                    // Backdrop in the gap color, then the cell lattice. The
                    // backdrop always draws — while a grid client is live it
                    // is the safety net beyond the patch edges during fast
                    // pans; the CELLS yield to the client's rendering.
                    set_backdrop(frame.backdrop_w, frame.backdrop_h, grid.gap_color.0.as_ptr());

                    if let Some(cells) = frame.cells.as_ref().filter(|_| wm.grid_cells_enabled) {
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
                        // Widened exactly like the windows' corner clip:
                        // at corner_shape > 2 the superellipse hugs the
                        // corner, so the raw radius reads nearly square —
                        // and a tiled window's (widened) arc must land on
                        // the cell's arc.
                        let cell_radius = crate::window::widen_corner_radius(
                            cells.corner_radius_px, cells.cell_w_px, cells.cell_h_px,
                        );
                        // The root plate-edge roll (mirroring cce-grid): the
                        // bevel-width knob clamped to a fraction of the
                        // rail, pre-scaled by zoom like every cell metric,
                        // so the rail reads as a flat face with a narrow
                        // lip at each sunken cell — not a full-ramp grout.
                        // style.surface.desktop.line_relief overrides the
                        // width outright (0 = no lip). The ring expands the
                        // cell box by the roll, inner edge concentric with
                        // the cell arc.
                        let gap_px = (frame.period_px_exact_x - cells.cell_w_px as f64)
                            .min(frame.period_px_exact_y - cells.cell_h_px as f64)
                            .max(0.0);
                        let roll = layout
                            .desktop_line_relief
                            .map(|v| v * zoom)
                            .unwrap_or_else(|| (layout.bevel_thickness as f64 * zoom).min(gap_px * 0.25))
                            .max(0.0);
                        let hg = roll.round() as i32;
                        let ring_w_px = cells.cell_w_px + 2 * hg;
                        let ring_h_px = cells.cell_h_px + 2 * hg;
                        let ring_radius = cell_radius + hg;
                        // Cell positions from the EXACT period, rounded per
                        // cell: a rounded-period spacing drifts from the
                        // world-anchored windows at fractional zooms (the
                        // grid visibly slides against window edges when
                        // panning).
                        for col in 0..=cells.cols {
                            let rel_x = (col as f64 * frame.period_px_exact_x).round() as i32;
                            for row in 0..=cells.rows {
                                let rel_y = (row as f64 * frame.period_px_exact_y).round() as i32;
                                get_rect(cells.cell_w_px, cells.cell_h_px, cells.color.0.as_ptr(), rel_x, rel_y, cell_radius, inset_scaled);
                                if bevel_on && hg > 0 {
                                    get_bevel(ring_w_px, ring_h_px, rel_x - hg, rel_y - hg, ring_radius, roll as f32);
                                }
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
                ffi::river_scene_node_set_position_if_changed(
                    backdrop_tree as *mut ffi::wlr_scene_node,
                    self.sent.x,
                    self.sent.y,
                );
                if force {
                    set_backdrop(viewport_w, viewport_h, color.0.as_ptr());
                }
            }
        }

        if force {
            // Disable unused rects in the pool to release GPU/scene resources.
            for rect in pool.iter().skip(pool_idx) {
                rect.set_enabled(false);
            }
            for bevel in bevel_pool.iter().skip(bevel_idx) {
                bevel.set_enabled(false);
            }
        }

        self.draw_cell_labels();
        if suspended {
            ffi::wlr_scene_node_set_enabled(grid_tree as *mut ffi::wlr_scene_node, true);
        }
    }

    /// Name every visible desktop square, chess style, while overview is open.
    ///
    /// The labels live in the grid tree, so they inherit its modulo shift and
    /// ride along with a pan for free; only the world index of the first drawn
    /// cell (`GridFrame::first_col/row`, computed in policy) is needed to know
    /// what to write. Outside overview every node is disabled — this is a
    /// navigation aid, not desktop furniture.
    unsafe fn draw_cell_labels(&mut self) {
        let wm = &(*self.server).wm;
        let overview = wm.mode == crate::window_manager::WindowManagerMode::Overview
            && wm.layout.desktop_cell_labels;

        if !overview {
            self.disable_cell_labels();
            return;
        }
        if self.grid_tree.is_null() {
            return;
        }
        if self.cell_label_tree.is_null() {
            self.cell_label_tree = SceneTree::create(&self.grid_tree);
            if self.cell_label_tree.is_null() {
                return;
            }
        }
        // Above the cell rects and the bevel subtree, which draw_grid raised
        // just before this.
        self.cell_label_tree.raise_to_top();

        let (viewport_w, viewport_h) = self.current.dimensions();
        let spec = wm.layout.background_spec();
        let crate::policy::api::BackgroundSpec::Grid(grid) = &spec else {
            self.disable_cell_labels();
            return;
        };
        let frame = crate::policy::background::grid_frame(
            grid,
            wm.camera(),
            viewport_w,
            viewport_h,
            self.sent.x,
            self.sent.y,
        );
        let Some(cells) = &frame.cells else {
            self.disable_cell_labels();
            return;
        };

        // A fixed fraction of the on-screen cell, clamped so labels stay
        // readable when zoomed far out and don't swell into billboards when
        // near. Below the floor there is no room for glyphs at all.
        let min_cell_px = cells.cell_w_px.min(cells.cell_h_px);
        let px = ((min_cell_px as f32) * 0.16).clamp(9.0, 40.0);
        if px * 3.0 > min_cell_px as f32 {
            self.disable_cell_labels();
            return;
        }
        let px_key = px.round() as u32;
        if px_key != self.last_label_px || self.cell_labels.len() > 512 {
            self.cell_labels.clear();
            self.last_label_px = px_key;
            // The freed buffers' addresses can be handed straight back to the
            // next rasterization, so a stale pointer here would compare equal
            // to a different label and skip the update.
            for entry in self.cell_label_pool.iter_mut() {
                entry.1 = std::ptr::null_mut();
            }
        }

        let inset = (min_cell_px as f64 * 0.06).round() as i32;
        let mut idx = 0usize;
        for col in 0..=cells.cols {
            let rel_x = (col as f64 * frame.period_px_exact_x).round() as i32;
            for row in 0..=cells.rows {
                let rel_y = (row as f64 * frame.period_px_exact_y).round() as i32;
                let Some(label) = self.cell_labels.get_square(frame.first_col + col, frame.first_row + row, px) else {
                    continue;
                };
                let (buf, lw, lh) = (label.buffer, label.width, label.height);

                let node = if idx < self.cell_label_pool.len() && !self.cell_label_pool[idx].0.is_null() {
                    let node = self.cell_label_pool[idx].0.raw();
                    if self.cell_label_pool[idx].1 != buf {
                        ffi::wlr_scene_buffer_set_buffer(node, buf);
                        self.cell_label_pool[idx].1 = buf;
                    }
                    ffi::wlr_scene_node_set_enabled(node as *mut ffi::wlr_scene_node, true);
                    node
                } else {
                    let handle = SceneBuffer::adopt(ffi::wlr_scene_buffer_create(self.cell_label_tree.raw(), buf));
                    let node = handle.raw();
                    if node.is_null() {
                        continue;
                    }
                    if idx < self.cell_label_pool.len() {
                        self.cell_label_pool[idx] = (handle, buf);
                    } else {
                        self.cell_label_pool.push((handle, buf));
                    }
                    node
                };
                ffi::wlr_scene_buffer_set_dest_size(node, lw, lh);
                // Top-left corner of the cell, inside the fade inset.
                ffi::river_scene_node_set_position_if_changed(
                    node as *mut ffi::wlr_scene_node,
                    rel_x + inset,
                    rel_y + inset,
                );
                let _ = lh;
                idx += 1;
            }
        }

        for (node, _) in self.cell_label_pool.iter().skip(idx) {
            node.set_enabled(false);
        }
    }

    fn disable_cell_labels(&self) {
        for (node, _) in &self.cell_label_pool {
            node.set_enabled(false);
        }
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let output = crate::container_of!(listener, Output, destroy);

    log::debug!("Output destroyed");
    crate::xwayland_window::note_output_change();

    // Remove listeners
    (*output).destroy.disconnect();
    (*output).request_state.disconnect();
    (*output).frame.disconnect();
    (*output).present.disconnect();

    if !(*output).scene_output.is_null() {
        ffi::wlr_scene_output_destroy((*output).scene_output);
        (*output).scene_output = std::ptr::null_mut();
    }

    (*output).destroy_scene_nodes();

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

/// `CCE_OVDBG=1` arms the `/tmp/cce-ovdbg` per-frame scene dump (see
/// `render_and_commit`); without it the file is never even looked for.
fn ovdbg_enabled() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCE_OVDBG").is_some())
}

impl Output {
    /// The refresh period to plan frames by: what the last `present`
    /// reported, else the current mode's rate, else 60 Hz.
    unsafe fn refresh_period_ns(&self) -> u64 {
        if self.present_refresh_ns > 0 {
            return self.present_refresh_ns;
        }
        let mhz = if self.wlr_output.is_null() { 0 } else { ffi::river_wlr_output_get_refresh(self.wlr_output) };
        if mhz > 0 {
            1_000_000_000_000 / mhz as u64
        } else {
            16_666_667
        }
    }

    /// When the frame rendered now is expected to reach the screen: the
    /// first point of a vblank grid after now (plus a small render lead).
    /// The camera animates to this instant, so its step is an exact whole
    /// number of refresh periods whether the frame callback ran early or
    /// late, a missed vblank is a double step rather than a stumble, and a
    /// second frame inside one period gets the same target (a zero step).
    ///
    /// The grid's phase locks to the hardware presentation timestamps and
    /// re-anchors only when they drift by more than a quarter period, so
    /// the per-present jitter of the timestamps themselves (and the
    /// headless backend's commit-time stamps) never reaches the camera.
    pub unsafe fn predicted_present_ns(&mut self) -> u64 {
        let now = util::timestamp_ns();
        let period = self.refresh_period_ns().max(1);
        if self.present_when_ns != 0 {
            let phase = self.present_when_ns % period;
            let drift = if self.frame_phase_ns == u64::MAX {
                u64::MAX
            } else {
                let d = phase.abs_diff(self.frame_phase_ns);
                d.min(period - d)
            };
            if drift > period / 4 {
                self.frame_phase_ns = phase;
            }
        } else if self.frame_phase_ns == u64::MAX {
            self.frame_phase_ns = now % period;
        }
        let lead = period / 8;
        let base = (now + lead).saturating_sub(self.frame_phase_ns);
        self.frame_phase_ns + (base / period + 1) * period
    }
}

/// Log a failed frame, at most once per 10 s with a count: a refused commit
/// can repeat at frame rate for minutes (38,784 lines in one session), and
/// wlroots logs each one itself as well.
fn log_render_error(e: &str) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    static SUPPRESSED: AtomicU64 = AtomicU64::new(0);
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    if last.is_some_and(|t| t.elapsed().as_secs() < 10) {
        SUPPRESSED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    *last = Some(std::time::Instant::now());
    match SUPPRESSED.swap(0, Ordering::Relaxed) {
        0 => log::error!("{}", e),
        n => log::error!("{} (and {} more in the last 10 s)", e, n),
    }
}

unsafe extern "C" fn handle_frame(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let output = &mut *crate::container_of!(listener, Output, frame);
    // The camera steps here, on the vblank, to where it should be at the
    // instant THIS frame is presented (see WindowManager::step_camera_frame).
    let frame_target_ns = output.predicted_present_ns();
    (*output.server).wm.step_camera_frame(frame_target_ns);
    // Likewise the interactive move/resize: one configure + relayout per
    // vblank, for the pointer's latest position.
    (*output.server).wm.step_op_frame();
    let render_start = if frame_debug() {
        Some(std::time::Instant::now())
    } else {
        None
    };
    if let Err(e) = output.render_and_commit() {
        log_render_error(e);
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
    // The scene's frame-done pass is gated on a node being visible, and the
    // desktop grid spends most of its life behind opaque windows — so the
    // one client that MUST repaint on demand is the one whose callbacks dry
    // up. cce-ui's runner waits on a frame callback before it renders, and
    // only a 250ms starvation fallback unblocks it: every patch took a
    // quarter second to come back, which is longer than the overview ramp
    // and is why an exit's replacement patch used to land after the
    // animation. While a patch is in the air, drive the client directly.
    (*output.server).wm.send_frame_done_to_grid_clients_awaiting_patch();
}

unsafe extern "C" fn handle_present(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let output = &mut *crate::container_of!(listener, Output, present);
    let event = data as *mut ffi::wlr_output_event_present;
    if !(*event).presented {
        return;
    }
    // Presentation clock bookkeeping, and the dropped-frame count: a vblank
    // sequence that advanced by more than one since the last present means
    // frames were skipped. Backends without a counter (headless) report
    // seq 0; there the gap is inferred from time, but only while the camera
    // is animating — a still desktop legitimately presents nothing for ages.
    {
        let when_ns = (*event).when.tv_sec as u64 * 1_000_000_000 + (*event).when.tv_nsec as u64;
        if (*event).refresh > 0 {
            output.present_refresh_ns = (*event).refresh as u64;
        }
        let period = output.refresh_period_ns();
        let seq = (*event).seq as u32;
        let mut dropped = 0u64;
        if seq != 0 && output.present_seq != 0 && seq > output.present_seq + 1 {
            dropped = (seq - output.present_seq - 1) as u64;
        } else if seq == 0 && output.present_when_ns != 0 && (*output.server).wm.camera_anim_active {
            let gap = when_ns.saturating_sub(output.present_when_ns);
            let periods = (gap + period / 2) / period;
            dropped = periods.saturating_sub(1);
        }
        if dropped > 0 {
            output.present_dropped += dropped;
            if frame_debug() {
                log::info!(
                    "[cce-frame] present seq={} dropped={} (total {}) refresh={}us",
                    seq,
                    dropped,
                    output.present_dropped,
                    period / 1000
                );
            }
        }
        output.present_when_ns = when_ns;
        output.present_seq = seq;
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
