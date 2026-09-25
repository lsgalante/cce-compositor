// SPDX-FileCopyrightText: © 2025 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use std::ffi::CStr;
use crate::ffi;
use crate::server::{Server, WlListener, WlList, wl_list_insert, wl_list_remove, wl_signal_add, wl_listener_remove};
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

pub struct LayerShellObject {
    pub resource: *mut ffi::wl_resource,
    pub link: ffi::wl_list,
}

pub struct LayerShell {
    pub server: *mut Server,
    pub global: *mut ffi::wl_global,
    pub wlr_shell: *mut ffi::wlr_layer_shell_v1,
    pub objects: ffi::wl_list,
    pub surfaces: SlotMap<*mut LayerSurface>,
    pub new_surface: ffi::wl_listener,
}

impl LayerShell {
    pub unsafe fn init(&mut self, server: *mut Server, wl_display: *mut ffi::wl_display) -> Result<(), ()> {
        self.server = server;
        self.global = ffi::wl_global_create(
            wl_display,
            &ffi::river_layer_shell_v1_interface,
            1,
            self as *mut LayerShell as *mut _,
            Some(bind),
        );
        if self.global.is_null() {
            return Err(());
        }

        self.wlr_shell = ffi::wlr_layer_shell_v1_create(wl_display, 4);
        if self.wlr_shell.is_null() {
            ffi::wl_global_destroy(self.global);
            self.global = std::ptr::null_mut();
            return Err(());
        }

        ffi::wl_list_init(&mut self.objects);

        let new_surface_ptr = &mut self.new_surface as *mut ffi::wl_listener as *mut WlListener;
        (*new_surface_ptr).notify = Some(handle_new_surface);
        wl_signal_add(&mut (*self.wlr_shell).events.new_surface, &mut self.new_surface);

        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        if !self.global.is_null() {
            ffi::wl_global_destroy(self.global);
            self.global = std::ptr::null_mut();
        }
        if !self.new_surface.link.prev.is_null() {
            wl_listener_remove(&mut self.new_surface);
        }
    }

    pub unsafe fn supported(&self) -> bool {
        let wm_v1 = (*self.server).wm.object;
        if wm_v1.is_null() {
            return true;
        }
        let wm_client = ffi::wl_resource_get_client(wm_v1);

        let objects_list = &self.objects as *const ffi::wl_list as *mut WlList;
        let mut curr = (*objects_list).next;
        while curr != objects_list {
            let next = (*curr).next;
            let obj = crate::container_of!(curr, LayerShellObject, link);
            let obj_client = ffi::wl_resource_get_client((*obj).resource);
            if obj_client == wm_client {
                return true;
            }
            curr = next;
        }
        false
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

unsafe extern "C" fn bind(
    client: *mut ffi::wl_client,
    data: *mut std::ffi::c_void,
    version: u32,
    id: u32,
) {
    let layer_shell = data as *mut LayerShell;
    if layer_shell.is_null() {
        return;
    }

    let resource = ffi::wl_resource_create(client, &ffi::river_layer_shell_v1_interface, version as i32, id);
    if resource.is_null() {
        ffi::wl_client_post_no_memory(client);
        log::error!("out of memory binding river_layer_shell_v1");
        return;
    }

    let obj = Box::into_raw(Box::new(LayerShellObject {
        resource,
        link: std::mem::zeroed(),
    }));
    ffi::wl_list_init(&mut (*obj).link);
    let objects_list = &mut (*layer_shell).objects as *mut ffi::wl_list as *mut WlList;
    let link_custom = &mut (*obj).link as *mut ffi::wl_list as *mut WlList;
    wl_list_insert(objects_list, link_custom);

    ffi::wl_resource_set_implementation(
        resource,
        &LAYER_SHELL_INTERFACE as *const _ as *const _,
        obj as *mut _,
        Some(handle_destroy_resource),
    );
}

unsafe extern "C" fn handle_destroy_resource(resource: *mut ffi::wl_resource) {
    let obj = ffi::wl_resource_get_user_data(resource) as *mut LayerShellObject;
    if !obj.is_null() {
        wl_list_remove(&mut (*obj).link as *mut ffi::wl_list as *mut WlList);
        let _ = Box::from_raw(obj);
    }
}

unsafe extern "C" fn layer_shell_destroy(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let _ = client;
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn layer_shell_get_output(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    output_resource: *mut ffi::wl_resource,
) {
    let output = ffi::wl_resource_get_user_data(output_resource) as *mut Output;
    if output.is_null() {
        return;
    }
    if !(*output).layer_shell.object.is_null() {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_layer_shell_v1_error_RIVER_LAYER_SHELL_V1_ERROR_OBJECT_ALREADY_CREATED,
            b"river_layer_shell_output_v1 already created\0".as_ptr() as *const _,
        );
        return;
    }
    let version = ffi::wl_resource_get_version(resource);
    (*output).layer_shell.create_object(client, version as u32, id, output);
}

unsafe extern "C" fn layer_shell_get_seat(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    seat_resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(seat_resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*seat).layer_shell.object.is_null() {
        ffi::wl_resource_post_error(
            resource,
            ffi::river_layer_shell_v1_error_RIVER_LAYER_SHELL_V1_ERROR_OBJECT_ALREADY_CREATED,
            b"river_layer_shell_seat_v1 already created\0".as_ptr() as *const _,
        );
        return;
    }
    let version = ffi::wl_resource_get_version(resource);
    (*seat).layer_shell.create_object(client, version as u32, id, seat);
}

static LAYER_SHELL_INTERFACE: ffi::river_layer_shell_v1_interface = ffi::river_layer_shell_v1_interface {
    destroy: Some(layer_shell_destroy),
    get_output: Some(layer_shell_get_output),
    get_seat: Some(layer_shell_get_seat),
};

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

    if !(*layer_shell).supported() {
        log::info!("window manager did not bind river_layer_shell_v1, closing layer surface");
        ffi::wlr_layer_surface_v1_destroy(wlr_layer_surface);
        return;
    }

    if (*wlr_layer_surface).output.is_null() {
        let outputs = &mut (*(*layer_shell).server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs).next;
        while curr != outputs {
            let next = (*curr).next;
            let output = crate::container_of!(curr, Output, link);
            if (*output).layer_shell.requested.default {
                (*wlr_layer_surface).output = (*output).wlr_output;
                break;
            }
            curr = next;
        }

        if (*wlr_layer_surface).output.is_null() {
            let first_node = (*outputs).next;
            if first_node != outputs {
                let output = crate::container_of!(first_node, Output, link);
                log::info!("window manager did not set default layer surface output, choosing arbitrary output");
                (*wlr_layer_surface).output = (*output).wlr_output;
            } else {
                log::error!("no output available for layer surface {:?}", CStr::from_ptr((*wlr_layer_surface).namespace));
                ffi::wlr_layer_surface_v1_destroy(wlr_layer_surface);
                return;
            }
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
    pub popup_tree: *mut ffi::wlr_scene_tree,
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

    pub destroy: ffi::wl_listener,
    pub map: ffi::wl_listener,
    pub unmap: ffi::wl_listener,
    pub commit: ffi::wl_listener,
    pub new_popup: ffi::wl_listener,
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

        let popup_tree = ffi::wlr_scene_tree_create((*server).scene.layers.popups);
        if popup_tree.is_null() {
            ffi::wlr_scene_node_destroy((*scene_layer_surface).tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create popup_tree");
        }

        let layer_surface = Box::into_raw(Box::new(LayerSurface {
            ref_key: Key { generation: 0, index: 0 },
            server,
            wlr_layer_surface,
            scene_layer_surface,
            popup_tree,
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

        let destroy_ptr = &mut (*layer_surface).destroy as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_ptr).notify = Some(handle_layer_surface_destroy);
        wl_signal_add(&mut (*wlr_layer_surface).events.destroy, &mut (*layer_surface).destroy);

        let map_ptr = &mut (*layer_surface).map as *mut ffi::wl_listener as *mut WlListener;
        (*map_ptr).notify = Some(handle_layer_surface_map);
        wl_signal_add(ffi::river_wlr_surface_get_map_signal((*wlr_layer_surface).surface), &mut (*layer_surface).map);

        let unmap_ptr = &mut (*layer_surface).unmap as *mut ffi::wl_listener as *mut WlListener;
        (*unmap_ptr).notify = Some(handle_layer_surface_unmap);
        wl_signal_add(ffi::river_wlr_surface_get_unmap_signal((*wlr_layer_surface).surface), &mut (*layer_surface).unmap);

        let commit_ptr = &mut (*layer_surface).commit as *mut ffi::wl_listener as *mut WlListener;
        (*commit_ptr).notify = Some(handle_layer_surface_commit);
        wl_signal_add(ffi::river_wlr_surface_get_commit_signal((*wlr_layer_surface).surface), &mut (*layer_surface).commit);

        let new_popup_ptr = &mut (*layer_surface).new_popup as *mut ffi::wl_listener as *mut WlListener;
        (*new_popup_ptr).notify = Some(handle_layer_surface_new_popup);
        wl_signal_add(&mut (*wlr_layer_surface).events.new_popup, &mut (*layer_surface).new_popup);

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

    wl_listener_remove(&mut (*layer_surface).destroy);
    wl_listener_remove(&mut (*layer_surface).map);
    wl_listener_remove(&mut (*layer_surface).unmap);
    wl_listener_remove(&mut (*layer_surface).commit);
    wl_listener_remove(&mut (*layer_surface).new_popup);

    (*layer_surface).destroy_popups();

    ffi::wlr_scene_node_destroy((*layer_surface).popup_tree as *mut ffi::wlr_scene_node);

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
        let ms = if cce_ui::motion::enabled() { (*server).wm.layout.fade_in_ms } else { 0 };
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

    if let Err(e) = XdgPopup::create(wlr_xdg_popup, (*layer_surface).popup_tree, std::ptr::null_mut()) {
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

#[derive(Clone, Copy)]
pub struct LayerShellOutputRequested {
    pub default: bool,
}

pub struct LayerShellOutput {
    pub object: *mut ffi::wl_resource, // river_layer_shell_output_v1
    pub scheduled: LayerShellOutputScheduled,
    pub sent: LayerShellOutputSent,
    pub requested: LayerShellOutputRequested,
}

impl Default for LayerShellOutput {
    fn default() -> Self {
        Self {
            object: std::ptr::null_mut(),
            scheduled: LayerShellOutputScheduled {
                non_exclusive_area: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
            },
            sent: LayerShellOutputSent {
                non_exclusive_area: None,
            },
            requested: LayerShellOutputRequested {
                default: false,
            },
        }
    }
}

impl LayerShellOutput {
    pub unsafe fn create_object(&mut self, client: *mut ffi::wl_client, version: u32, id: u32, output: *mut Output) {
        assert!(self.object.is_null());
        let resource = ffi::wl_resource_create(client, &ffi::river_layer_shell_output_v1_interface, version as i32, id);
        if resource.is_null() {
            ffi::wl_client_post_no_memory(client);
            log::error!("out of memory creating river_layer_shell_output_v1");
            return;
        }

        ffi::wl_resource_set_implementation(
            resource,
            &LAYER_SHELL_OUTPUT_INTERFACE as *const _ as *const _,
            self as *mut LayerShellOutput as *mut _,
            Some(handle_layer_shell_output_destroy),
        );
        self.object = resource;
        (*(*output).server).wm.dirty_windowing();
    }

    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_LAYER_SHELL_OUTPUT_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
        }
    }

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
                        ffi::wlr_scene_node_set_position((*layer_surface).popup_tree as *mut ffi::wlr_scene_node, x, y);

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
            if !self.object.is_null() {
                ffi::wl_resource_post_event(
                    self.object,
                    ffi::RIVER_LAYER_SHELL_OUTPUT_V1_NON_EXCLUSIVE_AREA,
                    self.scheduled.non_exclusive_area.x,
                    self.scheduled.non_exclusive_area.y,
                    self.scheduled.non_exclusive_area.width,
                    self.scheduled.non_exclusive_area.height,
                );
            }
            self.sent.non_exclusive_area = Some(self.scheduled.non_exclusive_area);
        }
    }
}

unsafe extern "C" fn handle_layer_shell_output_destroy(resource: *mut ffi::wl_resource) {
    let layer_shell_output = ffi::wl_resource_get_user_data(resource) as *mut LayerShellOutput;
    if !layer_shell_output.is_null() {
        (*layer_shell_output).object = std::ptr::null_mut();
        (*layer_shell_output).sent.non_exclusive_area = None;
        (*layer_shell_output).requested.default = false;
    }
}

unsafe extern "C" fn layer_shell_output_destroy(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let _ = client;
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn layer_shell_output_set_default(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let _ = client;
    let layer_shell_output = ffi::wl_resource_get_user_data(resource) as *mut LayerShellOutput;
    if layer_shell_output.is_null() {
        return;
    }
    let server = if !(*layer_shell_output).object.is_null() {
        // Find server. We can get it via finding Output from the parent link.
        // Let's traverse the outputs to set requested.default to false on all outputs
        let output = container_of_output(layer_shell_output);
        (*output).server
    } else {
        std::ptr::null_mut()
    };

    if !server.is_null() {
        let outputs = &mut (*server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs).next;
        while curr != outputs {
            let next = (*curr).next;
            let output = crate::container_of!(curr, Output, link);
            (*output).layer_shell.requested.default = false;
            curr = next;
        }
        (*layer_shell_output).requested.default = true;
    }
}

static LAYER_SHELL_OUTPUT_INTERFACE: ffi::river_layer_shell_output_v1_interface = ffi::river_layer_shell_output_v1_interface {
    destroy: Some(layer_shell_output_destroy),
    set_default: Some(layer_shell_output_set_default),
};

unsafe extern "C" fn layer_shell_output_inert_set_default(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
) {}

static INERT_LAYER_SHELL_OUTPUT_INTERFACE: ffi::river_layer_shell_output_v1_interface = ffi::river_layer_shell_output_v1_interface {
    destroy: Some(layer_shell_output_destroy),
    set_default: Some(layer_shell_output_inert_set_default),
};

unsafe fn container_of_output(layer_shell_output: *mut LayerShellOutput) -> *mut Output {
    crate::container_of!(layer_shell_output, Output, layer_shell)
}

pub struct LayerShellSeat {
    pub object: *mut ffi::wl_resource, // river_layer_shell_seat_v1
    pub scheduled_focus: LayerShellSeatFocus,
    pub sent_focus: LayerShellSeatFocus,
}

impl Default for LayerShellSeat {
    fn default() -> Self {
        Self {
            object: std::ptr::null_mut(),
            scheduled_focus: LayerShellSeatFocus::None,
            sent_focus: LayerShellSeatFocus::None,
        }
    }
}

impl LayerShellSeat {
    pub unsafe fn create_object(&mut self, client: *mut ffi::wl_client, version: u32, id: u32, seat: *mut Seat) {
        assert!(self.object.is_null());
        let resource = ffi::wl_resource_create(client, &ffi::river_layer_shell_seat_v1_interface, version as i32, id);
        if resource.is_null() {
            ffi::wl_client_post_no_memory(client);
            log::error!("out of memory creating river_layer_shell_seat_v1");
            return;
        }

        ffi::wl_resource_set_implementation(
            resource,
            &LAYER_SHELL_SEAT_INTERFACE as *const _ as *const _,
            self as *mut LayerShellSeat as *mut _,
            Some(handle_layer_shell_seat_destroy),
        );
        self.object = resource;
        (*(*seat).server).wm.dirty_windowing();
    }

    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_LAYER_SHELL_SEAT_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
        }
    }

    pub unsafe fn manage_start(&mut self) {
        if self.scheduled_focus != self.sent_focus {
            if !self.object.is_null() {
                match self.scheduled_focus {
                    LayerShellSeatFocus::Exclusive(_) => {
                        ffi::wl_resource_post_event(self.object, ffi::RIVER_LAYER_SHELL_SEAT_V1_FOCUS_EXCLUSIVE);
                    }
                    LayerShellSeatFocus::NonExclusive(_) => {
                        ffi::wl_resource_post_event(self.object, ffi::RIVER_LAYER_SHELL_SEAT_V1_FOCUS_NON_EXCLUSIVE);
                    }
                    LayerShellSeatFocus::None => {
                        ffi::wl_resource_post_event(self.object, ffi::RIVER_LAYER_SHELL_SEAT_V1_FOCUS_NONE);
                    }
                }
            }
            self.sent_focus = self.scheduled_focus;
        }
    }
}

unsafe extern "C" fn handle_layer_shell_seat_destroy(resource: *mut ffi::wl_resource) {
    let layer_shell_seat = ffi::wl_resource_get_user_data(resource) as *mut LayerShellSeat;
    if !layer_shell_seat.is_null() {
        (*layer_shell_seat).object = std::ptr::null_mut();
    }
}

unsafe extern "C" fn layer_shell_seat_destroy(client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let _ = client;
    ffi::wl_resource_destroy(resource);
}

static LAYER_SHELL_SEAT_INTERFACE: ffi::river_layer_shell_seat_v1_interface = ffi::river_layer_shell_seat_v1_interface {
    destroy: Some(layer_shell_seat_destroy),
};

static INERT_LAYER_SHELL_SEAT_INTERFACE: ffi::river_layer_shell_seat_v1_interface = ffi::river_layer_shell_seat_v1_interface {
    destroy: Some(layer_shell_seat_destroy),
};
