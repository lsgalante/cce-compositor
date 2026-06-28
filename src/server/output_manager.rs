// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, WlList, wl_signal_add, wl_listener_remove, wl_list_remove};
use crate::output::{Output, OutputStateValue, OutputMode};

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
    pub new_output: ffi::wl_listener,
    pub output_layout: *mut ffi::wlr_output_layout,
    pub presentation: *mut ffi::wlr_presentation,
    pub xdg_output_manager: *mut ffi::wlr_xdg_output_manager_v1,
    pub wlr_output_manager: *mut ffi::wlr_output_manager_v1,
    pub manager_apply: ffi::wl_listener,
    pub manager_test: ffi::wl_listener,
    pub power_manager: *mut ffi::wlr_output_power_manager_v1,
    pub power_manager_set_mode: ffi::wl_listener,
    pub gamma_control_manager: *mut ffi::wlr_gamma_control_manager_v1,
    pub outputs: ffi::wl_list,
}

impl OutputManager {
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.first_modeset = true;
        
        let output_layout = ffi::wlr_output_layout_create((*server).wl_server);
        if output_layout.is_null() {
            return Err("Failed to create wlr_output_layout");
        }
        self.output_layout = output_layout;

        let gamma_control_manager = ffi::wlr_gamma_control_manager_v1_create((*server).wl_server);
        if gamma_control_manager.is_null() {
            ffi::wlr_output_layout_destroy(output_layout);
            return Err("Failed to create wlr_gamma_control_manager_v1");
        }
        self.gamma_control_manager = gamma_control_manager;
        ffi::wlr_scene_set_gamma_control_manager_v1((*server).scene.wlr_scene, gamma_control_manager);

        let presentation = ffi::wlr_presentation_create((*server).wl_server, (*server).backend, 2);
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

        ffi::wl_list_init(&mut self.outputs);

        // Add backend new_output listener
        let backend_cast = (*server).backend as *mut crate::server::WlrBackend;
        let new_output_ptr = &mut self.new_output as *mut ffi::wl_listener as *mut WlListener;
        (*new_output_ptr).notify = Some(handle_new_output);
        wl_signal_add(&mut (*backend_cast).events.new_output, &mut self.new_output);

        // Add apply/test listeners
        let manager_cast = self.wlr_output_manager as *mut WlrOutputManagerV1;
        
        let apply_ptr = &mut self.manager_apply as *mut ffi::wl_listener as *mut WlListener;
        (*apply_ptr).notify = Some(handle_manager_apply);
        wl_signal_add(&mut (*manager_cast).events.apply, &mut self.manager_apply);

        let test_ptr = &mut self.manager_test as *mut ffi::wl_listener as *mut WlListener;
        (*test_ptr).notify = Some(handle_manager_test);
        wl_signal_add(&mut (*manager_cast).events.test, &mut self.manager_test);

        // Add power manager set_mode listener
        let power_cast = self.power_manager as *mut WlrOutputPowerManagerV1;
        let set_mode_ptr = &mut self.power_manager_set_mode as *mut ffi::wl_listener as *mut WlListener;
        (*set_mode_ptr).notify = Some(handle_power_manager_set_mode);
        wl_signal_add(&mut (*power_cast).events.set_mode, &mut self.power_manager_set_mode);

        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        wl_listener_remove(&mut self.manager_apply);
        wl_listener_remove(&mut self.manager_test);
        wl_listener_remove(&mut self.power_manager_set_mode);
        wl_listener_remove(&mut self.new_output);

        if !self.output_layout.is_null() {
            ffi::wlr_output_layout_destroy(self.output_layout);
        }
    }

    pub unsafe fn output_at(&mut self, lx: f64, ly: f64) -> *mut ffi::wlr_output {
        let mut output_lx: f64 = 0.0;
        let mut output_ly: f64 = 0.0;
        ffi::wlr_output_layout_closest_point(
            self.output_layout,
            std::ptr::null_mut(),
            lx,
            ly,
            &mut output_lx,
            &mut output_ly,
        );
        ffi::wlr_output_layout_output_at(self.output_layout, output_lx, output_ly)
    }

    pub unsafe fn auto_layout(&mut self) {
        let mut rightmost_edge: i32 = 0;
        let mut row_y: i32 = 0;

        let mut link = self.outputs.next;
        while link != &mut self.outputs as *mut ffi::wl_list {
            let output = &mut *crate::container_of!(link, Output, link);
            if !output.scheduled.auto_layout {
                let (w, _) = output.scheduled.dimensions();
                let x = output.scheduled.x + w;
                if x > rightmost_edge {
                    rightmost_edge = x;
                    row_y = output.scheduled.y;
                }
            }
            link = (*link).next;
        }

        let mut link = self.outputs.next;
        while link != &mut self.outputs as *mut ffi::wl_list {
            let output = &mut *crate::container_of!(link, Output, link);
            if output.scheduled.auto_layout {
                output.scheduled.x = rightmost_edge;
                output.scheduled.y = row_y;
                let (w, _) = output.scheduled.dimensions();
                rightmost_edge += w;
            }
            link = (*link).next;
        }
    }

    pub unsafe fn commit_output_state(&mut self, server: *mut Server) {
        let wm = &mut (*server).wm;

        let mut link = wm.sent.outputs.next;
        while link != &mut wm.sent.outputs as *mut ffi::wl_list {
            let output = &mut *crate::container_of!(link, Output, link_sent);
            assert!(output.sent.state != OutputStateValue::Destroying);
            output.rendering_current = output.rendering_requested;

            let wlr_output = output.wlr_output;
            if !wlr_output.is_null() {
                match output.sent.state {
                    OutputStateValue::Enabled | OutputStateValue::DisabledSoft => {
                        ffi::wlr_scene_output_set_position(output.scene_output, output.sent.x, output.sent.y);
                        ffi::wlr_output_layout_add(self.output_layout, wlr_output, output.sent.x, output.sent.y);
                        if let Some(lock_surface) = (*server).lock_manager.lock_surface_from_output(output) {
                            ffi::wlr_scene_node_set_position(
                                (*lock_surface).tree as *mut ffi::wlr_scene_node,
                                output.sent.x,
                                output.sent.y,
                            );
                        }

                        let (width, height) = output.sent.dimensions();
                        let color: [f32; 4] = [
                            ((*server).wm.layout.background_r as f64 / u32::MAX as f64) as f32,
                            ((*server).wm.layout.background_g as f64 / u32::MAX as f64) as f32,
                            ((*server).wm.layout.background_b as f64 / u32::MAX as f64) as f32,
                            ((*server).wm.layout.background_a as f64 / u32::MAX as f64) as f32,
                        ];
                        if output.background_rect.is_null() {
                            output.background_rect = ffi::wlr_scene_rect_create(
                                (*server).scene.layers.background,
                                width,
                                height,
                                color.as_ptr(),
                            );
                        } else {
                            ffi::wlr_scene_rect_set_size(output.background_rect, width, height);
                            ffi::wlr_scene_rect_set_color(output.background_rect, color.as_ptr());
                        }
                        if !output.background_rect.is_null() {
                            ffi::wlr_scene_node_set_position(
                                output.background_rect as *mut ffi::wlr_scene_node,
                                output.sent.x,
                                output.sent.y,
                            );
                        }
                        if output.grid_tree.is_null() {
                            output.grid_tree = ffi::wlr_scene_tree_create((*server).scene.layers.background);
                        }
                        if !output.grid_tree.is_null() {
                            ffi::wlr_scene_node_set_position(
                                output.grid_tree as *mut ffi::wlr_scene_node,
                                output.sent.x,
                                output.sent.y,
                            );
                        }
                    }
                    OutputStateValue::DisabledHard => {
                        ffi::wlr_output_layout_remove(self.output_layout, wlr_output);
                        if !output.background_rect.is_null() {
                            ffi::wlr_scene_node_destroy(output.background_rect as *mut ffi::wlr_scene_node);
                            output.background_rect = std::ptr::null_mut();
                        }
                        if !output.grid_tree.is_null() {
                            ffi::wlr_scene_node_destroy(output.grid_tree as *mut ffi::wlr_scene_node);
                            output.grid_tree = std::ptr::null_mut();
                        }
                    }
                    OutputStateValue::Destroying => unreachable!(),
                }
            }
            link = (*link).next;
        }

        let mut need_modeset = false;
        let mut link = wm.sent.outputs.next;
        while link != &mut wm.sent.outputs as *mut ffi::wl_list {
            let output = &mut *crate::container_of!(link, Output, link_sent);
            let wlr_output = output.wlr_output;
            if wlr_output.is_null() {
                link = (*link).next;
                continue;
            }

            let wlr_enabled = ffi::river_wlr_output_get_enabled(wlr_output);
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
                OutputMode::Standard(mode) => {
                    if mode != ffi::river_wlr_output_get_current_mode(wlr_output) {
                        need_modeset = true;
                        break;
                    }
                }
                OutputMode::Custom { width, height, refresh } => {
                    if width != ffi::river_wlr_output_get_width(wlr_output) 
                        || height != ffi::river_wlr_output_get_height(wlr_output) 
                        || refresh != ffi::river_wlr_output_get_refresh(wlr_output) {
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

            let wlr_adaptive = ffi::river_wlr_output_get_adaptive_sync_status(wlr_output)
                == ffi::wlr_output_adaptive_sync_status_WLR_OUTPUT_ADAPTIVE_SYNC_ENABLED;
            if output.sent.adaptive_sync != wlr_adaptive {
                need_modeset = true;
                break;
            }

            link = (*link).next;
        }

        if need_modeset {
            log::debug!("committing output state requires modeset");

            let mut states_vec = Vec::new();
            let mut link = wm.sent.outputs.next;
            while link != &mut wm.sent.outputs as *mut ffi::wl_list {
                let output = &mut *crate::container_of!(link, Output, link_sent);
                let wlr_output = output.wlr_output;
                if wlr_output.is_null() {
                    link = (*link).next;
                    continue;
                }

                states_vec.push(ffi::wlr_backend_output_state {
                    output: wlr_output,
                    base: std::mem::zeroed(),
                });

                link = (*link).next;
            }

            for state in &mut states_vec {
                ffi::wlr_output_state_init(&mut state.base);
                let output = &mut *(ffi::river_wlr_output_get_data(state.output) as *mut Output);
                output.sent.apply_modeset(&mut state.base);
            }

            let mut swapchain_manager = std::mem::zeroed();
            ffi::wlr_output_swapchain_manager_init(&mut swapchain_manager, (*server).backend);

            if !ffi::wlr_output_swapchain_manager_prepare(
                &mut swapchain_manager,
                states_vec.as_ptr(),
                states_vec.len(),
            ) {
                log::error!("failed to prepare new output configuration");
                self.modeset_failed(server);
                for state in &mut states_vec {
                    ffi::wlr_output_state_finish(&mut state.base);
                }
                ffi::wlr_output_swapchain_manager_finish(&mut swapchain_manager);
                return;
            }

            for state in &mut states_vec {
                let output = &mut *(ffi::river_wlr_output_get_data(state.output) as *mut Output);
                let sc = ffi::wlr_output_swapchain_manager_get_swapchain(&mut swapchain_manager, state.output);
                let options = ffi::wlr_scene_output_state_options {
                    swapchain: sc,
                    ..std::mem::zeroed()
                };
                if !ffi::wlr_scene_output_build_state(output.scene_output, &mut state.base, &options) {
                    log::error!("failed to render scene for {:?}", std::ffi::CStr::from_ptr(ffi::river_wlr_output_get_name(state.output)));
                }
            }

            if !ffi::wlr_backend_commit((*server).backend, states_vec.as_ptr(), states_vec.len()) {
                log::error!("failed to commit new output configuration");
                self.modeset_failed(server);
                for state in &mut states_vec {
                    ffi::wlr_output_state_finish(&mut state.base);
                }
                ffi::wlr_output_swapchain_manager_finish(&mut swapchain_manager);
                return;
            }

            self.first_modeset = false;
            ffi::wlr_output_swapchain_manager_apply(&mut swapchain_manager);

            for state in &mut states_vec {
                ffi::wlr_output_state_finish(&mut state.base);
            }
            ffi::wlr_output_swapchain_manager_finish(&mut swapchain_manager);
        }

        if !wm.sent.output_config.is_null() {
            ffi::wlr_output_configuration_v1_send_succeeded(wm.sent.output_config);
            ffi::wlr_output_configuration_v1_destroy(wm.sent.output_config);
            wm.sent.output_config = std::ptr::null_mut();
        }

        let mut link = wm.sent.outputs.next;
        while link != &mut wm.sent.outputs as *mut ffi::wl_list {
            let next_link = (*link).next;
            let output = &mut *crate::container_of!(link, Output, link_sent);
            let wlr_output = output.wlr_output;
            if wlr_output.is_null() {
                link = next_link;
                continue;
            }

            if !output.sent_wl_output {
                let global = ffi::river_wlr_output_get_global(wlr_output);
                if !global.is_null() {
                    if !output.object.is_null() {
                        let name = ffi::wl_global_get_name(global, ffi::wl_resource_get_client(output.object));
                        crate::output::zcce_output_send_wl_output(output.object, name);
                        output.sent_wl_output = true;
                    }
                }
            }

            match output.sent.state {
                OutputStateValue::Enabled => {
                    assert!(ffi::river_wlr_output_get_enabled(wlr_output));
                    ffi::wlr_output_schedule_frame(wlr_output);
                }
                OutputStateValue::DisabledSoft | OutputStateValue::DisabledHard => {
                    assert!(!ffi::river_wlr_output_get_enabled(wlr_output));
                    output.lock_render_state = crate::output::LockRenderState::Blanked;
                    if output.sent.state == OutputStateValue::DisabledHard {
                        wl_list_remove(&mut output.link_sent as *mut ffi::wl_list as *mut WlList);
                        ffi::wl_list_init(&mut output.link_sent);
                    }
                }
                OutputStateValue::Destroying => unreachable!(),
            }

            output.current = output.sent;
            link = next_link;
        }

        let _ = self.send_config(server);
    }

    pub unsafe fn modeset_failed(&mut self, server: *mut Server) {
        let wm = &mut (*server).wm;

        if self.first_modeset {
            log::error!("initial modeset failed, exiting river");
            ffi::wl_display_terminate((*server).wl_server);
            return;
        }

        if !wm.sent.output_config.is_null() {
            ffi::wlr_output_configuration_v1_send_failed(wm.sent.output_config);
            ffi::wlr_output_configuration_v1_destroy(wm.sent.output_config);
            wm.sent.output_config = std::ptr::null_mut();
        }

        let mut link = wm.sent.outputs.next;
        while link != &mut wm.sent.outputs as *mut ffi::wl_list {
            let output = &mut *crate::container_of!(link, Output, link_sent);
            output.scheduled = output.current;
            output.sent = output.current;
            link = (*link).next;
        }
        wm.dirty_windowing();
    }

    pub unsafe fn send_config(&mut self, _server: *mut Server) -> Result<(), &'static str> {
        let config = ffi::wlr_output_configuration_v1_create();
        if config.is_null() {
            return Err("Failed to create configuration v1");
        }

        let mut link = self.outputs.next;
        while link != &mut self.outputs as *mut ffi::wl_list {
            let output = &mut *crate::container_of!(link, Output, link);
            let wlr_output = output.wlr_output;
            if wlr_output.is_null() {
                link = (*link).next;
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

            link = (*link).next;
        }

        ffi::wlr_output_manager_v1_set_configuration(self.wlr_output_manager, config);
        Ok(())
    }

    pub unsafe fn max_overlap_output(&mut self, box_: *const ffi::wlr_box) -> *mut ffi::wlr_output {
        let mut max_overlap_area = 0;
        let mut max_overlap_output = std::ptr::null_mut();

        let mut link = self.outputs.next;
        while link != &mut self.outputs as *mut ffi::wl_list {
            let output = &mut *crate::container_of!(link, Output, link);
            let wlr_output = output.wlr_output;
            if wlr_output.is_null() {
                link = (*link).next;
                continue;
            }

            let mut overlap: ffi::wlr_box = std::mem::zeroed();
            ffi::wlr_output_layout_get_box(self.output_layout, wlr_output, &mut overlap);
            
            if overlap.width <= 0 || overlap.height <= 0 {
                link = (*link).next;
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

            link = (*link).next;
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
    if let Err(e) = Output::create(server, wlr_output) {
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
    ffi::wlr_output_swapchain_manager_init(&mut swapchain_manager, (*server).backend);

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

    if !(*server).wm.scheduled.output_config.is_null() {
        ffi::wlr_output_configuration_v1_send_failed((*server).wm.scheduled.output_config);
        ffi::wlr_output_configuration_v1_destroy((*server).wm.scheduled.output_config);
    }
    (*server).wm.scheduled.output_config = config;

    (*server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_power_manager_set_mode(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let om = &mut *crate::container_of!(listener, OutputManager, power_manager_set_mode);
    let server = crate::container_of!(om as *mut OutputManager, Server, om);
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

    (*server).wm.dirty_windowing();
}
