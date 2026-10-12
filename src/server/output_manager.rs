// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::Server;
use crate::output::{Output, OutputStateValue, OutputMode};
use crate::scene_handle::{SceneRect, SceneTree};

#[repr(C)]
pub struct WlrOutputManagerV1Events {
    pub apply: ffi::wl_signal,
    pub test: ffi::wl_signal,
    pub destroy: ffi::wl_signal,
}

#[repr(C)]
pub struct WlrOutputManagerV1 {
    pub display: *mut ffi::wl_display,
    pub global: *mut ffi::wl_global,
    pub resources: ffi::wl_list,
    pub heads: ffi::wl_list,
    pub serial: u32,
    pub current_configuration_dirty: bool,
    pub events: WlrOutputManagerV1Events,
}

#[repr(C)]
pub struct WlrOutputPowerManagerV1Events {
    pub set_mode: ffi::wl_signal,
    pub destroy: ffi::wl_signal,
}

#[repr(C)]
pub struct WlrOutputPowerManagerV1 {
    pub global: *mut ffi::wl_global,
    pub output_powers: ffi::wl_list,
    pub events: WlrOutputPowerManagerV1Events,
}

pub struct OutputManager {
    pub first_modeset: bool,
    pub new_output: crate::listener::Listener,
    pub presentation: *mut ffi::wlr_presentation,
    pub xdg_output_manager: *mut ffi::wlr_xdg_output_manager_v1,
    pub wlr_output_manager: *mut ffi::wlr_output_manager_v1,
    pub manager_apply: crate::listener::Listener,
    pub manager_test: crate::listener::Listener,
    pub power_manager: *mut ffi::wlr_output_power_manager_v1,
    pub power_manager_set_mode: crate::listener::Listener,
    pub gamma_control_manager: *mut ffi::wlr_gamma_control_manager_v1,
}

/// The output at (or nearest to) a layout point, through the shared
/// output layout: for code with no output manager in hand. Null before the
/// layout exists or while it holds no output.
pub fn output_at(lx: f64, ly: f64) -> *mut ffi::wlr_output {
    crate::shared::handles().output_layout.closest_output(lx, ly)
}

impl OutputManager {
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.first_modeset = true;
        
        let output_layout = ffi::wlr_output_layout_create((*server).wl_server);
        if output_layout.is_null() {
            return Err("Failed to create wlr_output_layout");
        }
        crate::shared::handles().output_layout.set(output_layout);

        let gamma_control_manager = ffi::wlr_gamma_control_manager_v1_create((*server).wl_server);
        if gamma_control_manager.is_null() {
            ffi::wlr_output_layout_destroy(crate::shared::handles().output_layout.take());
            return Err("Failed to create wlr_gamma_control_manager_v1");
        }
        self.gamma_control_manager = gamma_control_manager;
        ffi::wlr_scene_set_gamma_control_manager_v1(crate::shared::scene().wlr_scene(), gamma_control_manager);

        let presentation = ffi::wlr_presentation_create((*server).wl_server, crate::shared::handles().backend.ptr(), 2);
        if presentation.is_null() {
            return Err("Failed to create wlr_presentation");
        }
        self.presentation = presentation;

        let xdg_output_manager = ffi::wlr_xdg_output_manager_v1_create((*server).wl_server, output_layout);
        if xdg_output_manager.is_null() {
            return Err("Failed to create wlr_xdg_output_manager_v1");
        }
        self.xdg_output_manager = xdg_output_manager;

        let wlr_output_manager = ffi::wlr_output_manager_v1_create((*server).wl_server);
        if wlr_output_manager.is_null() {
            return Err("Failed to create wlr_output_manager_v1");
        }
        self.wlr_output_manager = wlr_output_manager;

        let power_manager = ffi::wlr_output_power_manager_v1_create((*server).wl_server);
        if power_manager.is_null() {
            return Err("Failed to create wlr_output_power_manager_v1");
        }
        self.power_manager = power_manager;

        // Add backend new_output listener
        let backend_cast = crate::shared::handles().backend.ptr() as *mut crate::server::WlrBackend;
        self.new_output.connect(&mut (*backend_cast).events.new_output, handle_new_output);

        // Add apply/test listeners
        let manager_cast = self.wlr_output_manager as *mut WlrOutputManagerV1;
        
        self.manager_apply.connect(&mut (*manager_cast).events.apply, handle_manager_apply);

        self.manager_test.connect(&mut (*manager_cast).events.test, handle_manager_test);

        // Add power manager set_mode listener
        let power_cast = self.power_manager as *mut WlrOutputPowerManagerV1;
        self.power_manager_set_mode.connect(&mut (*power_cast).events.set_mode, handle_power_manager_set_mode);

        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        self.manager_apply.disconnect();
        self.manager_test.disconnect();
        self.power_manager_set_mode.disconnect();
        self.new_output.disconnect();

        // Let go of the shared handle first: `Wlr::set`'s promise is that
        // the layout outlives the field's hold on it.
        let layout = crate::shared::handles().output_layout.take();
        if !layout.is_null() {
            ffi::wlr_output_layout_destroy(layout);
        }
    }

    pub fn output_at(&mut self, lx: f64, ly: f64) -> *mut ffi::wlr_output {
        output_at(lx, ly)
    }

    pub fn commit_output_state(&mut self, lock: &crate::lock_manager::LockManager, outputs: &mut crate::output::Outputs) {

        for id in crate::shared::sent().outputs.ids() {
            let Some(output) = outputs.get_mut(id) else { continue };
            assert!(output.sent.state != OutputStateValue::Destroying);
            output.rendering_current = output.rendering_requested;

            let wlr_output = output.wlr_output.ptr();
            if !wlr_output.is_null() {
                match output.sent.state {
                    OutputStateValue::Enabled | OutputStateValue::DisabledSoft => {
                        output.scene_output.set_position(output.sent.x, output.sent.y);
                        crate::shared::handles().output_layout.add(&output.wlr_output, output.sent.x, output.sent.y);
                        if let Some(lock_surface) = lock.lock_surface_from_output(output) {
                            // SAFETY: a lock surface `lock_surface_from_output`
                            // finds is live.
                            unsafe { (*lock_surface).tree.set_position(output.sent.x, output.sent.y) };
                        }

                        let (width, height) = output.sent.dimensions();
                        let color: [f32; 4] = [
                            (crate::shared::layout().background_r as f64 / u32::MAX as f64) as f32,
                            (crate::shared::layout().background_g as f64 / u32::MAX as f64) as f32,
                            (crate::shared::layout().background_b as f64 / u32::MAX as f64) as f32,
                            (crate::shared::layout().background_a as f64 / u32::MAX as f64) as f32,
                        ];
                        if output.background_rect.is_null() {
                            output.background_rect = SceneRect::create(
                                &crate::shared::scene().layers.background,
                                width,
                                height,
                                &color,
                            );
                        } else {
                            output.background_rect.set_size(width, height);
                            output.background_rect.set_color(&color);
                        }
                        output.background_rect.set_position(output.sent.x, output.sent.y);
                        if output.grid_tree.is_null() {
                            output.grid_tree = SceneTree::create(&crate::shared::scene().layers.background);
                        }
                        output.grid_tree.set_position(output.sent.x, output.sent.y);
                        output.grid_backdrop_tree.set_position(output.sent.x, output.sent.y);

                        if output.adjust_tree.is_null() {
                            output.adjust_tree = SceneTree::create(&crate::shared::scene().layers.top);
                        }
                        output.adjust_tree.set_position(output.sent.x, output.sent.y);
                    }
                    OutputStateValue::DisabledHard => {
                        crate::shared::handles().output_layout.remove(&output.wlr_output);
                        // All of them, rims and labels included: until
                        // 2026-10-10 this path left grid_bevel_tree and
                        // cell_label_tree naming children that died with
                        // grid_tree, and re-enabling the output raised a
                        // freed node.
                        output.destroy_scene_nodes();
                        // A fresh grid redraws its pools.
                        output.grid_force_redraw_frames = 3;
                    }
                    OutputStateValue::Destroying => unreachable!(),
                }
            }
        }

        let mut need_modeset = false;
        for id in crate::shared::sent().outputs.ids() {
            let Some(output) = outputs.get(id) else { continue };
            if output.wlr_output.is_null() {
                continue;
            }

            let wlr_enabled = output.wlr_output.enabled();
            match output.sent.state {
                OutputStateValue::Enabled => {
                    if !wlr_enabled {
                        need_modeset = true;
                        break;
                    }
                }
                OutputStateValue::DisabledSoft | OutputStateValue::DisabledHard => {
                    if wlr_enabled {
                        need_modeset = true;
                        break;
                    }
                }
                OutputStateValue::Destroying => unreachable!(),
            }

            match output.sent.mode {
                OutputMode::Standard { mode, .. } => {
                    if mode != output.wlr_output.current_mode() {
                        need_modeset = true;
                        break;
                    }
                }
                OutputMode::Custom { width, height, refresh } => {
                    if width != output.wlr_output.width() 
                        || height != output.wlr_output.height() 
                        || refresh != output.wlr_output.refresh() {
                        need_modeset = true;
                        break;
                    }
                }
                OutputMode::None => {
                    assert!(output.sent.state == OutputStateValue::DisabledHard);
                }
            }

            if output.current.mode_none() && output.sent.state == OutputStateValue::Enabled {
                need_modeset = true;
                break;
            }

            if output.sent.scale != output.current.scale {
                need_modeset = true;
                break;
            }

            if output.sent.adaptive_sync != output.wlr_output.adaptive_sync_enabled() {
                need_modeset = true;
                break;
            }

        }

        if need_modeset {
            log::debug!("committing output state requires modeset");

            // The outputs in the order their states are built, so each state
            // finds its output without going back through wlroots.
            let ids: Vec<crate::output::OutputId> = crate::shared::sent()
                .outputs
                .ids()
                .into_iter()
                .filter(|&id| outputs.get(id).is_some_and(|o| !o.wlr_output.is_null()))
                .collect();
            // SAFETY: each state's output is the live wlr_output its store
            // entry holds (`Wlr`), with that output's own modes in its sent
            // state (`OutputMode::standard`); every state is initialised
            // before use and finished on every path out, as is the
            // swapchain manager; the backend handle and each store output's
            // scene output are live.
            let committed = unsafe { self.modeset(outputs, &ids) };
            if !committed {
                self.modeset_failed(outputs);
                return;
            }
        }

        let config = crate::shared::sent().take_output_config();
        if !config.is_null() {
            // SAFETY: the configuration a client applied, ours to answer and
            // free (`Sent::set_output_config`).
            unsafe {
                ffi::wlr_output_configuration_v1_send_succeeded(config);
                ffi::wlr_output_configuration_v1_destroy(config);
            }
        }

        for id in crate::shared::sent().outputs.ids() {
            let Some(output) = outputs.get_mut(id) else { continue };
            if output.wlr_output.is_null() {
                continue;
            }

            match output.sent.state {
                OutputStateValue::Enabled => {
                    assert!(output.wlr_output.enabled());
                    output.wlr_output.schedule_frame();
                }
                OutputStateValue::DisabledSoft | OutputStateValue::DisabledHard => {
                    assert!(!output.wlr_output.enabled());
                    output.lock_render_state = crate::output::LockRenderState::Blanked;
                    if output.sent.state == OutputStateValue::DisabledHard {
                        crate::shared::sent().outputs.remove(output.id);
                    }
                }
                OutputStateValue::Destroying => unreachable!(),
            }

            output.current = output.sent;
        }
        crate::xwayland_window::note_output_scale(outputs);

        let _ = self.send_config(outputs);
    }

    /// The modeset half of `commit_output_state`: build each output's new
    /// state, give it a swapchain, render it, and commit them all at once.
    /// Whether the backend took it.
    ///
    /// # Safety
    /// Every id names a store output with a live `wlr_output`.
    unsafe fn modeset(&mut self, outputs: &crate::output::Outputs, ids: &[crate::output::OutputId]) -> bool {
        let mut states_vec: Vec<ffi::wlr_backend_output_state> = ids
            .iter()
            .filter_map(|&id| outputs.get(id))
            .map(|output| ffi::wlr_backend_output_state { output: output.wlr_output.ptr(), base: std::mem::zeroed() })
            .collect();

        for (state, &id) in states_vec.iter_mut().zip(ids) {
            ffi::wlr_output_state_init(&mut state.base);
            if let Some(output) = outputs.get(id) {
                output.sent.apply_modeset(&mut state.base);
            }
        }

        let mut swapchain_manager = std::mem::zeroed();
        ffi::wlr_output_swapchain_manager_init(&mut swapchain_manager, crate::shared::handles().backend.ptr());

        let finish = |states_vec: &mut Vec<ffi::wlr_backend_output_state>, swapchain_manager: &mut ffi::wlr_output_swapchain_manager| {
            for state in states_vec.iter_mut() {
                ffi::wlr_output_state_finish(&mut state.base);
            }
            ffi::wlr_output_swapchain_manager_finish(swapchain_manager);
        };

        if !ffi::wlr_output_swapchain_manager_prepare(&mut swapchain_manager, states_vec.as_ptr(), states_vec.len()) {
            log::error!("failed to prepare new output configuration");
            finish(&mut states_vec, &mut swapchain_manager);
            return false;
        }

        for (state, &id) in states_vec.iter_mut().zip(ids) {
            let Some(output) = outputs.get(id) else { continue };
            let sc = ffi::wlr_output_swapchain_manager_get_swapchain(&mut swapchain_manager, state.output);
            let options = ffi::wlr_scene_output_state_options {
                swapchain: sc,
                ..std::mem::zeroed()
            };
            if !ffi::wlr_scene_output_build_state(output.scene_output.ptr(), &mut state.base, &options) {
                log::error!("failed to render scene for {:?}", output.wlr_output.name());
            }
        }

        if !ffi::wlr_backend_commit(crate::shared::handles().backend.ptr(), states_vec.as_ptr(), states_vec.len()) {
            log::error!("failed to commit new output configuration");
            finish(&mut states_vec, &mut swapchain_manager);
            return false;
        }

        self.first_modeset = false;
        ffi::wlr_output_swapchain_manager_apply(&mut swapchain_manager);
        finish(&mut states_vec, &mut swapchain_manager);
        true
    }

    pub fn modeset_failed(&mut self, outputs: &mut crate::output::Outputs) {

        if self.first_modeset {
            log::error!("initial modeset failed, exiting river");
            crate::shared::shared().terminate_display();
            return;
        }

        let config = crate::shared::sent().take_output_config();
        if !config.is_null() {
            // SAFETY: the configuration a client applied, ours to answer and
            // free (`Sent::set_output_config`).
            unsafe {
                ffi::wlr_output_configuration_v1_send_failed(config);
                ffi::wlr_output_configuration_v1_destroy(config);
            }
        }

        for id in crate::shared::sent().outputs.ids() {
            let Some(output) = outputs.get_mut(id) else { continue };
            output.scheduled = output.current;
            output.sent = output.current;
        }
        crate::shared::pending().dirty_windowing();
    }

    pub fn send_config(&mut self, outputs: &crate::output::Outputs) -> Result<(), &'static str> {
        // SAFETY: the configuration is created here and either handed to
        // the live output manager (created in `init`) or destroyed; each
        // head is made for a store output's live wlr_output and filled in
        // before the handoff.
        unsafe {
            let config = ffi::wlr_output_configuration_v1_create();
            if config.is_null() {
                return Err("Failed to create configuration v1");
            }

            for output in outputs.values() {
                let wlr_output = output.wlr_output.ptr();
                if wlr_output.is_null() {
                    continue;
                }

                let head = ffi::wlr_output_configuration_head_v1_create(config, wlr_output);
                if head.is_null() {
                    ffi::wlr_output_configuration_v1_destroy(config);
                    return Err("Failed to create configuration head");
                }

                (*head).state.enabled = match output.current.state {
                    OutputStateValue::Enabled | OutputStateValue::DisabledSoft => true,
                    OutputStateValue::DisabledHard => false,
                    OutputStateValue::Destroying => unreachable!(),
                };
                (*head).state.scale = output.current.scale;
                (*head).state.transform = output.current.transform;
                (*head).state.x = output.current.x;
                (*head).state.y = output.current.y;

            }

            ffi::wlr_output_manager_v1_set_configuration(self.wlr_output_manager, config);
            Ok(())
        }
    }

}

/// The output manager's walks of the outputs, on the store the window
/// manager owns (`wm.outputs`).
impl crate::output::Outputs {
    /// The layout box `(x, y, w, h)` of every enabled output, as last sent.
    pub fn enabled_output_boxes(&self) -> Vec<(f64, f64, f64, f64)> {
        let mut boxes = Vec::new();
        for output in self.values() {
            if output.sent.state == crate::output::OutputStateValue::Enabled {
                let b = output.sent.box_layout();
                boxes.push((b.x as f64, b.y as f64, b.width as f64, b.height as f64));
            }
        }
        boxes
    }

    pub unsafe fn auto_layout(&self) {
        let mut rightmost_edge: i32 = 0;
        let mut row_y: i32 = 0;

        for output_ptr in self.ptrs() {
            let output = &mut *output_ptr;
            if !output.scheduled.auto_layout {
                let (w, _) = output.scheduled.dimensions();
                let x = output.scheduled.x + w;
                if x > rightmost_edge {
                    rightmost_edge = x;
                    row_y = output.scheduled.y;
                }
            }
        }

        for output_ptr in self.ptrs() {
            let output = &mut *output_ptr;
            if output.scheduled.auto_layout {
                output.scheduled.x = rightmost_edge;
                output.scheduled.y = row_y;
                let (w, _) = output.scheduled.dimensions();
                rightmost_edge += w;
            }
        }
    }

    pub unsafe fn max_overlap_output(&self, box_: *const ffi::wlr_box) -> *mut ffi::wlr_output {
        let mut max_overlap_area = 0;
        let mut max_overlap_output = std::ptr::null_mut();

        for output_ptr in self.ptrs() {
            let output = &mut *output_ptr;
            let wlr_output = output.wlr_output.ptr();
            if wlr_output.is_null() {
                continue;
            }

            let overlap = output.wlr_output.layout_box();
            
            if overlap.width <= 0 || overlap.height <= 0 {
                continue;
            }

            let mut dest: ffi::wlr_box = std::mem::zeroed();
            if ffi::wlr_box_intersection(&mut dest, &overlap, box_) {
                let overlap_area = dest.width * dest.height;
                if overlap_area > max_overlap_area {
                    max_overlap_area = overlap_area;
                    max_overlap_output = wlr_output;
                }
            }

        }

        max_overlap_output
    }
}

fn validate_config_coordinates(server: *mut Server, config: *mut ffi::wlr_output_configuration_v1) -> bool {
    unsafe {
        let mut head_link = (*config).heads.next;
        while head_link != &mut (*config).heads {
            let head = crate::container_of!(head_link, ffi::wlr_output_configuration_head_v1, link);
            let _output_global = ffi::river_wlr_output_get_global((*head).state.output);
            
            if (*head).state.enabled {
                let proposed = crate::output::OutputState::from_head_state(&((*head).state));
                if !(*server).xwayland.is_null() {
                    if proposed.x < 0 || proposed.y < 0 {
                        log::error!(
                            "Attempted to set negative coordinates for output. Negative output coordinates are disallowed if Xwayland is active."
                        );
                        return false;
                    }
                    let (width, height) = proposed.dimensions();
                    if proposed.x + width > i16::MAX as i32 || proposed.y + height > i16::MAX as i32 {
                        log::error!(
                            "Attempted to set too-large coordinates for output. Coordinates greater than {} are disallowed if Xwayland is active.",
                            i16::MAX
                        );
                        return false;
                    }
                }
            }
            head_link = (*head_link).next;
        }
    }
    true
}

unsafe extern "C" fn handle_new_output(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let om = &mut *crate::container_of!(listener, OutputManager, new_output);
    let server = crate::container_of!(om as *mut OutputManager, Server, om);
    let wlr_output = data as *mut ffi::wlr_output;
    log::debug!("new output {:?}", std::ffi::CStr::from_ptr(ffi::river_wlr_output_get_name(wlr_output)));
    crate::xwayland_window::note_output_change();
    if let Err(e) = Output::create(&mut *crate::reentry::wm(server), server, wlr_output) {
        log::error!("failed to create output: {}", e);
        ffi::wlr_output_destroy(wlr_output);
    }
}

unsafe extern "C" fn handle_manager_test(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let om = &mut *crate::container_of!(listener, OutputManager, manager_test);
    let server = crate::container_of!(om as *mut OutputManager, Server, om);
    let config = data as *mut ffi::wlr_output_configuration_v1;

    if !validate_config_coordinates(server, config) {
        ffi::wlr_output_configuration_v1_send_failed(config);
        ffi::wlr_output_configuration_v1_destroy(config);
        return;
    }

    let mut states_len: usize = 0;
    let states = ffi::wlr_output_configuration_v1_build_state(config, &mut states_len);
    if states.is_null() {
        log::error!("out of memory");
        ffi::wlr_output_configuration_v1_send_failed(config);
        ffi::wlr_output_configuration_v1_destroy(config);
        return;
    }

    let mut swapchain_manager = std::mem::zeroed();
    ffi::wlr_output_swapchain_manager_init(&mut swapchain_manager, crate::shared::handles().backend.ptr());

    if ffi::wlr_output_swapchain_manager_prepare(&mut swapchain_manager, states, states_len) {
        ffi::wlr_output_configuration_v1_send_succeeded(config);
    } else {
        ffi::wlr_output_configuration_v1_send_failed(config);
    }

    ffi::wlr_output_swapchain_manager_finish(&mut swapchain_manager);
    libc::free(states as *mut std::ffi::c_void);
    ffi::wlr_output_configuration_v1_destroy(config);
}

unsafe extern "C" fn handle_manager_apply(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let om = &mut *crate::container_of!(listener, OutputManager, manager_apply);
    let server = crate::container_of!(om as *mut OutputManager, Server, om);
    let config = data as *mut ffi::wlr_output_configuration_v1;

    log::info!("applying output configuration");

    if !validate_config_coordinates(server, config) {
        ffi::wlr_output_configuration_v1_send_failed(config);
        ffi::wlr_output_configuration_v1_destroy(config);
        return;
    }

    let mut head_link = (*config).heads.next;
    while head_link != &mut (*config).heads {
        let head = crate::container_of!(head_link, ffi::wlr_output_configuration_head_v1, link);
        let output = &mut *(ffi::river_wlr_output_get_data((*head).state.output) as *mut Output);
        
        if (*head).state.enabled {
            let previous = output.scheduled.state;
            output.scheduled = crate::output::OutputState::from_head_state(&((*head).state));
            if previous == crate::output::OutputStateValue::DisabledSoft {
                output.scheduled.state = crate::output::OutputStateValue::DisabledSoft;
            } else {
                assert!(output.scheduled.state == crate::output::OutputStateValue::Enabled);
            }
        } else {
            output.scheduled.state = crate::output::OutputStateValue::DisabledHard;
        }

        head_link = (*head_link).next;
    }

    let wm = &mut *crate::reentry::wm(server);
    if !wm.scheduled.output_config.is_null() {
        ffi::wlr_output_configuration_v1_send_failed(wm.scheduled.output_config);
        ffi::wlr_output_configuration_v1_destroy(wm.scheduled.output_config);
    }
    wm.scheduled.output_config = config;

    crate::shared::pending().dirty_windowing();
}

unsafe extern "C" fn handle_power_manager_set_mode(_listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let event = data as *mut ffi::wlr_output_power_v1_set_mode_event;

    let output_data = ffi::river_wlr_output_get_data((*event).output);
    if output_data.is_null() {
        return;
    }
    let output = &mut *(output_data as *mut Output);

    log::debug!("client requested dpms mode {:?} for output {:?}", (*event).mode, std::ffi::CStr::from_ptr(ffi::river_wlr_output_get_name((*event).output)));

    match output.scheduled.state {
        OutputStateValue::Enabled => {
            if (*event).mode == ffi::zwlr_output_power_v1_mode_ZWLR_OUTPUT_POWER_V1_MODE_OFF {
                output.scheduled.state = OutputStateValue::DisabledSoft;
            } else {
                return;
            }
        }
        OutputStateValue::DisabledSoft => {
            if (*event).mode == ffi::zwlr_output_power_v1_mode_ZWLR_OUTPUT_POWER_V1_MODE_ON {
                output.scheduled.state = OutputStateValue::Enabled;
            } else {
                return;
            }
        }
        OutputStateValue::DisabledHard | OutputStateValue::Destroying => return,
    }

    crate::shared::pending().dirty_windowing();
}
