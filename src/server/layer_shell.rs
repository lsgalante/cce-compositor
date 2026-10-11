// SPDX-FileCopyrightText: © 2025 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use std::ffi::CStr;
use crate::ffi;
use crate::server::{Server, WlList};
use crate::slotmap::{SlotMap, Key};
use crate::output::Output;
use crate::seat::Seat;
use crate::scene_node_data::{SceneNodeData, SceneNodeDataVal};
use crate::xdg_popup::XdgPopup;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LayerShellSeatFocus {
    Exclusive(Key),
    NonExclusive(Key),
    None,
}

pub struct LayerShell {
    pub server: *mut Server,
    pub wlr_shell: *mut ffi::wlr_layer_shell_v1,
    pub surfaces: SlotMap<*mut LayerSurface>,
    pub new_surface: crate::listener::Listener,
}

impl LayerShell {
    pub unsafe fn init(&mut self, server: *mut Server, wl_display: *mut ffi::wl_display) -> Result<(), ()> {
        self.server = server;
        self.wlr_shell = ffi::wlr_layer_shell_v1_create(wl_display, 4);
        if self.wlr_shell.is_null() {
            return Err(());
        }

        self.new_surface.connect(&mut (*self.wlr_shell).events.new_surface, handle_new_surface);

        Ok(())
    }

    pub fn deinit(&mut self) {
        self.new_surface.disconnect();
    }

    pub unsafe fn check_exclusive_focus(&mut self) {
        let layers = [
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_TOP,
        ];
        let mut to_focus: *mut LayerSurface = std::ptr::null_mut();

        'outer: for &layer in &layers {
            let tree = (*self.server).scene.layer_surface_tree(layer);
            let children_head = ffi::river_scene_tree_get_children(tree) as *mut WlList;
            let mut curr = (*children_head).prev;
            while curr != children_head {
                let prev = (*curr).prev;
                let node = ffi::river_scene_node_from_children_link(curr as *mut ffi::wl_list);
                if let Some(node_data) = SceneNodeData::from_node(node) {
                    if let SceneNodeDataVal::LayerSurface(layer_surface) = node_data.data {
                        let wlr_layer_surface = (*layer_surface).wlr_layer_surface;
                        if ffi::river_wlr_surface_is_mapped((*wlr_layer_surface).surface) &&
                           (*wlr_layer_surface).current.keyboard_interactive == ffi::zwlr_layer_surface_v1_keyboard_interactivity_ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_EXCLUSIVE {
                            to_focus = layer_surface;
                            break 'outer;
                        }
                    }
                }
                curr = prev;
            }
        }

        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, Seat, link);
            if !to_focus.is_null() {
                (*seat).layer_shell.scheduled_focus = LayerShellSeatFocus::Exclusive((*to_focus).ref_key);
            } else if matches!((*seat).layer_shell.scheduled_focus, LayerShellSeatFocus::Exclusive(_)) {
                (*seat).layer_shell.scheduled_focus = LayerShellSeatFocus::None;
            }
            curr = next;
        }
    }
}

unsafe extern "C" fn handle_new_surface(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let layer_shell = crate::container_of!(listener, LayerShell, new_surface);
    let wlr_layer_surface = data as *mut ffi::wlr_layer_surface_v1;

    log::debug!(
        "new layer surface: namespace {:?}, layer {}, anchor {}, size {}x{}, margin: top={}, right={}, bottom={}, left={}, exclusive_zone={}",
        CStr::from_ptr((*wlr_layer_surface).namespace),
        (*wlr_layer_surface).current.layer,
        (*wlr_layer_surface).current.anchor,
        (*wlr_layer_surface).current.desired_width,
        (*wlr_layer_surface).current.desired_height,
        (*wlr_layer_surface).current.margin.top,
        (*wlr_layer_surface).current.margin.right,
        (*wlr_layer_surface).current.margin.bottom,
        (*wlr_layer_surface).current.margin.left,
        (*wlr_layer_surface).current.exclusive_zone,
    );

    // A layer surface that names no output goes on the first one.
    if (*wlr_layer_surface).output.is_null() {
        let outputs = &mut (*(*layer_shell).server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let first_node = (*outputs).next;
        if first_node != outputs {
            let output = crate::container_of!(first_node, Output, link);
            log::info!("layer surface named no output, choosing the first");
            (*wlr_layer_surface).output = (*output).wlr_output;
        } else {
            log::error!("no output available for layer surface {:?}", CStr::from_ptr((*wlr_layer_surface).namespace));
            ffi::wlr_layer_surface_v1_destroy(wlr_layer_surface);
            return;
        }
    }

    if let Err(e) = LayerSurface::create(wlr_layer_surface, (*layer_shell).server) {
        log::error!("Failed to create layer surface: {}", e);
        ffi::wl_resource_post_no_memory((*wlr_layer_surface).resource);
    }
}

pub struct LayerSurface {
    pub ref_key: Key,
    pub server: *mut Server,
    pub wlr_layer_surface: *mut ffi::wlr_layer_surface_v1,
    pub scene_layer_surface: *mut ffi::wlr_scene_layer_surface_v1,
    pub popup_tree: crate::scene_handle::SceneTree,
    /// Where the open/close dissolve currently stands, 0.0 (invisible) to
    /// 1.0. Applied to the whole scene subtree, so the scenefx backdrop blur
    /// behind the surface fades with it (`river_scene_node_set_opacity`) —
    /// which is the thing a client fading its own pixels can never do.
    pub opacity: f32,
    /// Where `opacity` is easing to: 1.0 for an open fade, 0.0 for a close.
    pub opacity_target: f32,
    /// Linear per-tick step, from the configured duration at the moment the
    /// fade starts. Linear rather than exponential because a close fade has
    /// to actually reach zero before the client's exit deadline.
    pub opacity_step: f32,
    pub animation_timer: *mut ffi::wl_event_source,

    pub destroy: crate::listener::Listener,
    pub map: crate::listener::Listener,
    pub unmap: crate::listener::Listener,
    pub commit: crate::listener::Listener,
    pub new_popup: crate::listener::Listener,
    pub parent_offset_applied: bool,
}

impl LayerSurface {
    pub unsafe fn create(
        wlr_layer_surface: *mut ffi::wlr_layer_surface_v1,
        server: *mut Server,
    ) -> Result<*mut Self, &'static str> {
        let layer_tree = (*server).scene.layer_surface_tree((*wlr_layer_surface).current.layer);
        let scene_layer_surface = ffi::wlr_scene_layer_surface_v1_create(layer_tree, wlr_layer_surface);
        if scene_layer_surface.is_null() {
            return Err("Failed to create wlr_scene_layer_surface_v1");
        }

        let popup_tree = ffi::wlr_scene_tree_create((*server).scene.layers.popups.raw());
        if popup_tree.is_null() {
            ffi::wlr_scene_node_destroy((*scene_layer_surface).tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create popup_tree");
        }

        let layer_surface = Box::into_raw(Box::new(LayerSurface {
            ref_key: Key { generation: 0, index: 0 },
            server,
            wlr_layer_surface,
            scene_layer_surface,
            popup_tree: crate::scene_handle::SceneTree::adopt(popup_tree),
            opacity: 1.0,
            opacity_target: 1.0,
            opacity_step: 1.0,
            animation_timer: std::ptr::null_mut(),
            destroy: std::mem::zeroed(),
            map: std::mem::zeroed(),
            unmap: std::mem::zeroed(),
            commit: std::mem::zeroed(),
            new_popup: std::mem::zeroed(),
            parent_offset_applied: false,
        }));

        let key = (*server).layer_shell.surfaces.put(layer_surface);
        (*layer_surface).ref_key = key;

        SceneNodeData::attach((*scene_layer_surface).tree as *mut _, SceneNodeDataVal::LayerSurface(layer_surface));
        SceneNodeData::attach(popup_tree as *mut _, SceneNodeDataVal::LayerSurface(layer_surface));

        ffi::river_wlr_surface_set_data((*wlr_layer_surface).surface, (*scene_layer_surface).tree as *mut _);

        (*layer_surface).destroy.connect(&mut (*wlr_layer_surface).events.destroy, handle_layer_surface_destroy);

        (*layer_surface).map.connect(ffi::river_wlr_surface_get_map_signal((*wlr_layer_surface).surface), handle_layer_surface_map);

        (*layer_surface).unmap.connect(ffi::river_wlr_surface_get_unmap_signal((*wlr_layer_surface).surface), handle_layer_surface_unmap);

        (*layer_surface).commit.connect(ffi::river_wlr_surface_get_commit_signal((*wlr_layer_surface).surface), handle_layer_surface_commit);

        (*layer_surface).new_popup.connect(&mut (*wlr_layer_surface).events.new_popup, handle_layer_surface_new_popup);

        Ok(layer_surface)
    }

    pub unsafe fn destroy_popups(&mut self) {
        let popups_list = &mut (*self.wlr_layer_surface).popups as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*popups_list).next;
        while curr != popups_list {
            let next = (*curr).next;
            let wlr_xdg_popup = crate::container_of!(curr, ffi::wlr_xdg_popup, link);
            ffi::wlr_xdg_popup_destroy(wlr_xdg_popup);
            curr = next;
        }
    }
}

unsafe extern "C" fn handle_layer_surface_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let layer_surface = crate::container_of!(listener, LayerSurface, destroy);

    log::debug!("layer surface {:?} destroyed", CStr::from_ptr((*(*layer_surface).wlr_layer_surface).namespace));

    if !(*layer_surface).animation_timer.is_null() {
        ffi::wl_event_source_remove((*layer_surface).animation_timer);
        (*layer_surface).animation_timer = std::ptr::null_mut();
    }

    (*layer_surface).destroy.disconnect();
    (*layer_surface).map.disconnect();
    (*layer_surface).unmap.disconnect();
    (*layer_surface).commit.disconnect();
    (*layer_surface).new_popup.disconnect();

    (*layer_surface).destroy_popups();

    (*layer_surface).popup_tree.destroy();

    ffi::river_wlr_surface_set_data((*(*layer_surface).wlr_layer_surface).surface, std::ptr::null_mut());

    let server = (*layer_surface).server;
    (*server).layer_shell.surfaces.remove((*layer_surface).ref_key);
    let _ = Box::from_raw(layer_surface);
}

/// Steps one layer surface's dissolve toward `opacity_target` and re-arms
/// itself until it lands. Unlike the window fade — which rides the window
/// manager's shared border-fade timer — each layer surface keeps its own,
/// because a layer surface is not in `wm.windows` and there is no list to
/// sweep.
unsafe extern "C" fn handle_animation_tick(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let layer_surface = data as *mut LayerSurface;

    let target = (*layer_surface).opacity_target;
    let delta = target - (*layer_surface).opacity;
    let settled = if delta.abs() <= (*layer_surface).opacity_step {
        (*layer_surface).opacity = target;
        true
    } else {
        (*layer_surface).opacity += (*layer_surface).opacity_step * delta.signum();
        false
    };

    ffi::river_scene_node_set_opacity(
        (*(*layer_surface).scene_layer_surface).tree as *mut ffi::wlr_scene_node,
        (*layer_surface).opacity,
    );

    if settled {
        if !(*layer_surface).animation_timer.is_null() {
            ffi::wl_event_source_remove((*layer_surface).animation_timer);
            (*layer_surface).animation_timer = std::ptr::null_mut();
        }
    } else {
        if !(*layer_surface).animation_timer.is_null() {
            ffi::wl_event_source_timer_update((*layer_surface).animation_timer, 16);
        }
    }

    0
}

impl LayerSurface {
    /// Begin a dissolve toward `target` (0.0 out, 1.0 in) over `ms`. A `ms`
    /// of 0 snaps, so callers can treat this as "put the surface at
    /// `target`" whether or not fading is configured on.
    pub unsafe fn start_fade(&mut self, target: f32, ms: u32) {
        self.opacity_target = target.clamp(0.0, 1.0);
        if !self.animation_timer.is_null() {
            ffi::wl_event_source_remove(self.animation_timer);
            self.animation_timer = std::ptr::null_mut();
        }
        if ms == 0 {
            self.opacity = self.opacity_target;
            ffi::river_scene_node_set_opacity(
                (*self.scene_layer_surface).tree as *mut ffi::wlr_scene_node,
                self.opacity,
            );
            return;
        }
        // Ticks at 16 ms; at least one step so a sub-frame duration still
        // lands rather than dividing by zero.
        let ticks = ((ms as f32) / 16.0).max(1.0);
        self.opacity_step = ((self.opacity_target - self.opacity).abs() / ticks).max(1.0e-4);
        ffi::river_scene_node_set_opacity(
            (*self.scene_layer_surface).tree as *mut ffi::wlr_scene_node,
            self.opacity,
        );

        let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
        let timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_animation_tick),
            self as *mut LayerSurface as *mut _,
        );
        if timer.is_null() {
            log::error!("Failed to create layer surface animation timer");
            // No timer means no ramp; land on the target rather than leave
            // the surface stranded at whatever it was mid-fade.
            self.opacity = self.opacity_target;
            ffi::river_scene_node_set_opacity(
                (*self.scene_layer_surface).tree as *mut ffi::wlr_scene_node,
                self.opacity,
            );
        } else {
            self.animation_timer = timer;
            ffi::wl_event_source_timer_update(timer, 16);
        }
    }

    /// PID of the client owning this layer surface, from its wl_resource.
    /// 0 when it cannot be read. Used to resolve a `fade-out` to the caller.
    pub unsafe fn client_pid(&self) -> i32 {
        let surface = (*self.wlr_layer_surface).surface;
        if surface.is_null() {
            return 0;
        }
        let res = ffi::river_wlr_surface_get_resource(surface);
        if res.is_null() {
            return 0;
        }
        let client = ffi::wl_resource_get_client(res);
        if client.is_null() {
            return 0;
        }
        let (mut pid, mut uid, mut gid) = (0, 0, 0);
        ffi::wl_client_get_credentials(client, &mut pid, &mut uid, &mut gid);
        pid
    }
}

unsafe fn update_scheduled_focus_and_dirty_windowing<F>(server: *mut Server, f: F)
where F: FnOnce() {
    let seats = &mut (*server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*seats).next;
    let mut old_focuses = Vec::new();
    while curr != seats {
        let seat = crate::container_of!(curr, Seat, link);
        old_focuses.push((*seat).layer_shell.scheduled_focus);
        curr = (*curr).next;
    }

    f();

    let mut changed = false;
    let mut curr = (*seats).next;
    let mut idx = 0;
    while curr != seats {
        let seat = crate::container_of!(curr, Seat, link);
        if (*seat).layer_shell.scheduled_focus != old_focuses[idx] {
            changed = true;
        }
        idx += 1;
        curr = (*curr).next;
    }

    if changed {
        (*server).wm.dirty_windowing();
    } else {
        (*server).wm.dirty_rendering();
    }
}


unsafe extern "C" fn handle_layer_surface_map(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let layer_surface = crate::container_of!(listener, LayerSurface, map);
    let wlr_layer_surface = (*layer_surface).wlr_layer_surface;

    log::debug!("layer surface {:?} mapped", CStr::from_ptr((*wlr_layer_surface).namespace));

    let server = (*layer_surface).server;

    // Overlay layer only: these are the transient surfaces the user opens
    // (the launcher, the notifier), so a dissolve reads as the thing
    // arriving. The Background/Bottom/Top layers are the desktop's own
    // furniture — wallpaper, status bar — and map once at login, where a
    // fade reads as the desktop failing to draw.
    if (*wlr_layer_surface).current.layer == ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY {
        let ms = if cce_core::motion::enabled() { (*server).wm.layout.fade_in_ms } else { 0 };
        if ms > 0 {
            (*layer_surface).opacity = 0.0;
        }
        (*layer_surface).start_fade(1.0, ms);
    }

    update_scheduled_focus_and_dirty_windowing(server, || {
        if (*wlr_layer_surface).current.keyboard_interactive == ffi::zwlr_layer_surface_v1_keyboard_interactivity_ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_ON_DEMAND {
            let seats = &mut (*server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*seats).next;
            while curr != seats {
                let next = (*curr).next;
                let seat = crate::container_of!(curr, Seat, link);
                if !matches!((*seat).layer_shell.scheduled_focus, LayerShellSeatFocus::Exclusive(_)) {
                    (*seat).layer_shell.scheduled_focus = LayerShellSeatFocus::NonExclusive((*layer_surface).ref_key);
                }
                curr = next;
            }
        }

        let wlr_output = (*wlr_layer_surface).output;
        if !wlr_output.is_null() {
            let output = ffi::river_wlr_output_get_data(wlr_output) as *mut Output;
            if !output.is_null() {
                (*output).layer_shell.arrange(output);
            }
        }
        (*server).layer_shell.check_exclusive_focus();
    });
}

unsafe extern "C" fn handle_layer_surface_unmap(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let layer_surface = crate::container_of!(listener, LayerSurface, unmap);
    let wlr_layer_surface = (*layer_surface).wlr_layer_surface;

    log::debug!("layer surface {:?} unmapped", CStr::from_ptr((*wlr_layer_surface).namespace));

    if !(*layer_surface).animation_timer.is_null() {
        ffi::wl_event_source_remove((*layer_surface).animation_timer);
        (*layer_surface).animation_timer = std::ptr::null_mut();
    }

    let server = (*layer_surface).server;

    update_scheduled_focus_and_dirty_windowing(server, || {
        let seats = &mut (*server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, Seat, link);
            if let crate::seat::Focus::LayerSurface(surface) = (*seat).focused {
                if surface == (*wlr_layer_surface).surface {
                    (*seat).focus(crate::seat::Focus::None);
                    // cce-cloud surfaces skip the focus_next fallback: the bare
                    // launcher is about to be replaced by whatever it spawned,
                    // and refocusing the old window first would fight the new
                    // map. But a PARENTED popup ("cce-cloud:<app-id>", e.g. the
                    // designer's add-node palette) is chrome OF that app —
                    // closing it must hand the keyboard straight back to its
                    // parent, not leave the seat focused on nothing.
                    let mut is_cce_cloud = false;
                    let mut cloud_parent: Option<String> = None;
                    if !(*wlr_layer_surface).namespace.is_null() {
                        let ns = std::ffi::CStr::from_ptr((*wlr_layer_surface).namespace).to_string_lossy();
                        if ns.starts_with("cce-cloud") {
                            is_cce_cloud = true;
                            cloud_parent = ns.strip_prefix("cce-cloud:").map(str::to_string);
                        }
                    }
                    if let Some(parent_app_id) = cloud_parent {
                        // Status modules parent their submenus too; the bar is
                        // never a keyboard-focus target, so those keep the old
                        // leave-it-unfocused behavior.
                        if !parent_app_id.starts_with("cce-status") {
                            for &win_ptr in (*server).wm.windows.iter() {
                                if win_ptr.is_null()
                                    || (*win_ptr).closed
                                    || (*win_ptr).minimized
                                    || !matches!((*win_ptr).state, crate::window::WindowState::Mapped)
                                {
                                    continue;
                                }
                                if (*win_ptr).get_app_id_string().as_deref() == Some(parent_app_id.as_str()) {
                                    // Dismissing chrome, not switching windows:
                                    // the camera stays where the user left it.
                                    (*seat).suppress_focus_pan = true;
                                    (*seat).focus(crate::seat::Focus::Window(win_ptr));
                                    (*seat).suppress_focus_pan = false;
                                    break;
                                }
                            }
                        }
                    } else if !is_cce_cloud {
                        (*server).wm.focus_next_visible_window(seat);
                    }
                }
            }
            if let LayerShellSeatFocus::NonExclusive(key) = (*seat).layer_shell.scheduled_focus {
                if key == (*layer_surface).ref_key {
                    (*seat).layer_shell.scheduled_focus = LayerShellSeatFocus::None;
                }
            }
            curr = next;
        }

        let wlr_output = (*wlr_layer_surface).output;
        if !wlr_output.is_null() {
            let output = ffi::river_wlr_output_get_data(wlr_output) as *mut Output;
            if !output.is_null() {
                (*output).layer_shell.arrange(output);
            }
        }
        (*server).layer_shell.check_exclusive_focus();
    });
}

unsafe extern "C" fn handle_layer_surface_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let layer_surface = crate::container_of!(listener, LayerSurface, commit);
    let wlr_layer_surface = (*layer_surface).wlr_layer_surface;

    if (*wlr_layer_surface).current.layer != ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND {
        let server = (*layer_surface).server;
        let mut blur_enabled = (*server).wm.layout.window_blur;
        let mut ignore_transparent = (*server).wm.layout.window_backdrop_blur_ignore_transparent;
        let mut is_status = false;
        if !(*wlr_layer_surface).namespace.is_null() {
            let ns = std::ffi::CStr::from_ptr((*wlr_layer_surface).namespace).to_string_lossy();
            if ns == "cce-status" || ns == "cce-status-interface" {
                blur_enabled = (*server).wm.layout.status_background_blur > 0.001;
                ignore_transparent = (*server).wm.layout.status_backdrop_blur_ignore_transparent;
                is_status = true;
            }
        }
        // Optimized (cached) blur is counterproductive for surfaces stacked ABOVE
        // windows (Top/Overlay): the scene graph re-dirties an optimized-blur node
        // whenever any node below it updates (scenefx wlr_scene.c:744), so window
        // content panning underneath forces a full re-bake every frame — a fixed-
        // position shimmer (e.g. the always-mapped cce-notifier overlay). Regular
        // blur is immune to that path and only re-bakes on real damage, so fall back
        // to it here, exactly as status surfaces already do. Bottom/Background layers
        // sit below windows and are unaffected, so they keep the cache.
        let layer = (*wlr_layer_surface).current.layer;
        let above_windows = layer == ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_TOP
            || layer == ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY;
        let use_optimized = if is_status || above_windows { false } else { (*server).wm.layout.scenefx_optimized_blur };
        let wlr_surface = (*wlr_layer_surface).surface;
        let geom_w = if !wlr_surface.is_null() {
            ffi::river_wlr_surface_get_width(wlr_surface)
        } else {
            (*wlr_layer_surface).current.actual_width as i32
        };
        let geom_h = if !wlr_surface.is_null() {
            ffi::river_wlr_surface_get_height(wlr_surface)
        } else {
            (*wlr_layer_surface).current.actual_height as i32
        };
        ffi::river_scene_node_enable_blur(
            (*(*layer_surface).scene_layer_surface).tree as *mut ffi::wlr_scene_node,
            blur_enabled,
            use_optimized,
            ignore_transparent,
            0,
            0,
            geom_w,
            geom_h,
            // 0 preserves existing behaviour: layer surfaces (status bar, etc.) never had a
            // blur radius applied, and their corner rounding is handled separately. Left
            // deliberately unchanged so this fix stays scoped to toplevels.
            0,
        );
    }

    if (*layer_surface).opacity < 1.0 {
        ffi::river_scene_node_set_opacity(
            (*(*layer_surface).scene_layer_surface).tree as *mut ffi::wlr_scene_node,
            (*layer_surface).opacity,
        );
    }

    let wlr_output = (*wlr_layer_surface).output;
    if wlr_output.is_null() {
        return;
    }
    let output = ffi::river_wlr_output_get_data(wlr_output) as *mut Output;
    if output.is_null() {
        return;
    }

    let server = (*layer_surface).server;

    // Position offset for cce-cloud sub-modules
    if !(*layer_surface).parent_offset_applied {
        if !(*wlr_layer_surface).namespace.is_null() {
            let ns = std::ffi::CStr::from_ptr((*wlr_layer_surface).namespace).to_string_lossy();
            if ns.starts_with("cce-cloud:") {
                let parent_app_id = &ns["cce-cloud:".len()..];
                let mut parent_x = None;
                let mut parent_y = None;
                let mut parent_w = 0;

                for &win_ptr in (*server).wm.windows.iter() {
                    if win_ptr.is_null() || (*win_ptr).closed {
                        continue;
                    }
                    if let Some(win_app_id) = (*win_ptr).get_app_id_string() {
                        if win_app_id == parent_app_id {
                            parent_x = Some((*win_ptr).rendering_requested.x);
                            parent_y = Some((*win_ptr).rendering_requested.y);
                            parent_w = (*win_ptr).box_geom.width;
                            break;
                        }
                    }
                }

                if let (Some(px), Some(py)) = (parent_x, parent_y) {
                    let wlr_output = (*wlr_layer_surface).output;
                    if !wlr_output.is_null() {
                        let mut output_x = 0;
                        let mut output_y = 0;
                        let mut output_w = 0;
                        let outputs_list = &mut (*server).om.outputs as *mut ffi::wl_list as *mut WlList;
                        let mut curr_out = (*outputs_list).next;
                        while curr_out != outputs_list {
                            let output = crate::container_of!(curr_out, crate::output::Output, link);
                            if (*output).wlr_output == wlr_output {
                                let wlr_box = (*output).sent.box_layout();
                                output_x = wlr_box.x;
                                output_y = wlr_box.y;
                                output_w = wlr_box.width;
                                break;
                            }
                            curr_out = (*curr_out).next;
                        }

                        let relative_parent_x = px - output_x;
                        let relative_parent_y = py - output_y;

                        let anchor = (*wlr_layer_surface).current.anchor;
                        let is_align_right = (anchor & ffi::zwlr_layer_surface_v1_anchor_ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT) != 0;

                        if is_align_right {
                            (*wlr_layer_surface).pending.margin.right = (output_w - relative_parent_x - parent_w) + (*wlr_layer_surface).pending.margin.right;
                            (*wlr_layer_surface).current.margin.right = (*wlr_layer_surface).pending.margin.right;
                        } else {
                            (*wlr_layer_surface).pending.margin.left = relative_parent_x + (*wlr_layer_surface).pending.margin.left;
                            (*wlr_layer_surface).current.margin.left = (*wlr_layer_surface).pending.margin.left;
                        }
                        (*wlr_layer_surface).pending.margin.top = relative_parent_y + (*wlr_layer_surface).pending.margin.top;
                        (*wlr_layer_surface).current.margin.top = (*wlr_layer_surface).pending.margin.top;

                        (*layer_surface).parent_offset_applied = true;
                    }
                }
            }
        }
    }

    // Check if layer was changed
    if (*wlr_layer_surface).current.committed & ffi::wlr_layer_surface_v1_state_field_WLR_LAYER_SURFACE_V1_STATE_LAYER != 0 {
        let tree = (*server).scene.layer_surface_tree((*wlr_layer_surface).current.layer);
        ffi::wlr_scene_node_reparent(
            (*(*layer_surface).scene_layer_surface).tree as *mut ffi::wlr_scene_node,
            tree,
        );
    }

    if (*wlr_layer_surface).initial_commit || ((*wlr_layer_surface).current.committed != 0) {
        update_scheduled_focus_and_dirty_windowing(server, || {
            (*output).layer_shell.arrange(output);
            (*server).layer_shell.check_exclusive_focus();
        });
    }
}

unsafe extern "C" fn handle_layer_surface_new_popup(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let layer_surface = crate::container_of!(listener, LayerSurface, new_popup);
    let wlr_xdg_popup = data as *mut ffi::wlr_xdg_popup;

    if let Err(e) = XdgPopup::create(
        wlr_xdg_popup,
        (*layer_surface).popup_tree.raw(),
        std::ptr::null_mut(),
        (*layer_surface).popup_tree.raw(),
    ) {
        log::error!("Failed to create layer surface popup: {}", e);
        ffi::wl_resource_post_no_memory((*wlr_xdg_popup).resource);
    }
}

#[derive(Clone, Copy)]
pub struct LayerShellOutputScheduled {
    pub non_exclusive_area: ffi::wlr_box,
}

#[derive(Clone, Copy)]
pub struct LayerShellOutputSent {
    pub non_exclusive_area: Option<ffi::wlr_box>,
}

pub struct LayerShellOutput {
    pub scheduled: LayerShellOutputScheduled,
    pub sent: LayerShellOutputSent,
}

impl Default for LayerShellOutput {
    fn default() -> Self {
        Self {
            scheduled: LayerShellOutputScheduled {
                non_exclusive_area: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
            },
            sent: LayerShellOutputSent {
                non_exclusive_area: None,
            },
        }
    }
}

impl LayerShellOutput {
    pub unsafe fn arrange(&mut self, output: *mut Output) {
        let (w, h) = (*output).scheduled.dimensions();
        let box_geom = ffi::wlr_box {
            x: (*output).scheduled.x,
            y: (*output).scheduled.y,
            width: w,
            height: h,
        };
        self.scheduled.non_exclusive_area = box_geom;
        self.send_configures(output, true);
        self.send_configures(output, false);

        let area_changed = match self.sent.non_exclusive_area {
            Some(sent_box) => {
                sent_box.x != self.scheduled.non_exclusive_area.x ||
                sent_box.y != self.scheduled.non_exclusive_area.y ||
                sent_box.width != self.scheduled.non_exclusive_area.width ||
                sent_box.height != self.scheduled.non_exclusive_area.height
            }
            None => true,
        };

        if area_changed {
            (*(*output).server).wm.dirty_windowing();
        }
    }

    unsafe fn send_configures(&mut self, output: *mut Output, exclusive: bool) {
        let (output_width, output_height) = (*output).scheduled.dimensions();
        let output_box = ffi::wlr_box {
            x: (*output).scheduled.x,
            y: (*output).scheduled.y,
            width: output_width,
            height: output_height,
        };

        let layers = [
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_TOP,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY,
        ];

        for &layer in &layers {
            let tree = (*(*output).server).scene.layer_surface_tree(layer);
            let children_head = ffi::river_scene_tree_get_children(tree) as *mut WlList;
            let mut curr = (*children_head).next;
            while curr != children_head {
                let next = (*curr).next;
                let node = ffi::river_scene_node_from_children_link(curr as *mut ffi::wl_list);
                if let Some(node_data) = SceneNodeData::from_node(node) {
                    if let SceneNodeDataVal::LayerSurface(layer_surface) = node_data.data {
                        let wlr_layer_surface = (*layer_surface).wlr_layer_surface;
                        if !ffi::river_wlr_surface_is_mapped((*wlr_layer_surface).surface) && !(*wlr_layer_surface).initial_commit {
                            curr = next;
                            continue;
                        }
                        if (*wlr_layer_surface).output != (*output).wlr_output {
                            curr = next;
                            continue;
                        }
                        let current_exclusive = (*wlr_layer_surface).current.exclusive_zone > 0;
                        if current_exclusive != exclusive {
                            curr = next;
                            continue;
                        }

                        let mut new_area = self.scheduled.non_exclusive_area;
                        ffi::wlr_scene_layer_surface_v1_configure(
                            (*layer_surface).scene_layer_surface,
                            &output_box,
                            &mut new_area,
                        );

                        if new_area.width < (output_width / 2) || new_area.height < (output_height / 2) {
                            ffi::wlr_layer_surface_v1_destroy(wlr_layer_surface);
                            curr = next;
                            continue;
                        }
                        self.scheduled.non_exclusive_area = new_area;

                        let x = ffi::river_scene_node_get_x((*(*layer_surface).scene_layer_surface).tree as *mut ffi::wlr_scene_node);
                        let y = ffi::river_scene_node_get_y((*(*layer_surface).scene_layer_surface).tree as *mut ffi::wlr_scene_node);
                        (*layer_surface).popup_tree.set_position(x, y);

                        let clip = ffi::wlr_box {
                            x: -(x - (*output).scheduled.x),
                            y: -(y - (*output).scheduled.y),
                            width: output_width,
                            height: output_height,
                        };
                        ffi::wlr_scene_subsurface_tree_set_clip(
                            (*(*layer_surface).scene_layer_surface).tree as *mut ffi::wlr_scene_node,
                            &clip,
                        );
                    }
                }
                curr = next;
            }
        }
    }

    pub unsafe fn manage_start(&mut self, output: *mut Output) {
        let state = (*output).scheduled.state;
        assert!(
            matches!(state, crate::output::OutputStateValue::Enabled)
                || matches!(state, crate::output::OutputStateValue::DisabledSoft)
        );

        let (w, h) = (*output).scheduled.dimensions();
        let scheduled_box = ffi::wlr_box {
            x: (*output).scheduled.x,
            y: (*output).scheduled.y,
            width: w,
            height: h,
        };

        let (w_sent, h_sent) = (*output).sent.dimensions();
        let sent_box = ffi::wlr_box {
            x: (*output).sent.x,
            y: (*output).sent.y,
            width: w_sent,
            height: h_sent,
        };

        let box_changed = scheduled_box.x != sent_box.x ||
                          scheduled_box.y != sent_box.y ||
                          scheduled_box.width != sent_box.width ||
                          scheduled_box.height != sent_box.height;

        if box_changed {
            self.scheduled.non_exclusive_area = scheduled_box;
            self.send_configures(output, true);
            self.send_configures(output, false);
        }

        let area_changed = match self.sent.non_exclusive_area {
            Some(sent_box) => {
                sent_box.x != self.scheduled.non_exclusive_area.x ||
                sent_box.y != self.scheduled.non_exclusive_area.y ||
                sent_box.width != self.scheduled.non_exclusive_area.width ||
                sent_box.height != self.scheduled.non_exclusive_area.height
            }
            None => true,
        };

        if area_changed {
            self.sent.non_exclusive_area = Some(self.scheduled.non_exclusive_area);
        }
    }
}

pub struct LayerShellSeat {
    pub scheduled_focus: LayerShellSeatFocus,
    pub sent_focus: LayerShellSeatFocus,
}

impl Default for LayerShellSeat {
    fn default() -> Self {
        Self {
            scheduled_focus: LayerShellSeatFocus::None,
            sent_focus: LayerShellSeatFocus::None,
        }
    }
}

impl LayerShellSeat {
    pub fn manage_start(&mut self) {
        if self.scheduled_focus != self.sent_focus {
            self.sent_focus = self.scheduled_focus;
        }
    }
}
