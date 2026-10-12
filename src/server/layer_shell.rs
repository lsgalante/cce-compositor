// SPDX-FileCopyrightText: © 2025 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use std::ffi::CStr;
use crate::ffi;
use crate::server::WlList;
use crate::slotmap::Key;
use crate::scene_node_data::{SceneNodeData, SceneNodeDataVal};
use crate::xdg_popup::XdgPopup;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LayerShellSeatFocus {
    Exclusive(Key),
    NonExclusive(Key),
    None,
}

pub struct LayerShell {
    pub wlr_shell: *mut ffi::wlr_layer_shell_v1,
    new_surface: crate::listener::Listener,
}

impl LayerShell {
    pub unsafe fn init(&mut self, wl_display: *mut ffi::wl_display) -> Result<(), ()> {
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

    /// Schedule keyboard focus for the topmost mapped exclusive layer
    /// surface on every seat, or clear an exclusive focus none claims.
    ///
    /// A surface queued for closing (`close_later`) is passed over, and a
    /// scheduled focus naming one is dropped: it is still mapped until the
    /// idle callback closes it, and before the close was deferred it went
    /// inside the arrange that found it too large, so it never held focus.
    /// Focused in between, its close would take the keyboard from the
    /// window and fire the app-exit camera reaction.
    pub fn check_exclusive_focus(seats: &mut crate::seat::Seats) {
        let layers = [
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_TOP,
        ];
        let exclusive = ffi::zwlr_layer_surface_v1_keyboard_interactivity_ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_EXCLUSIVE;
        let mut to_focus: Option<Key> = None;
        'outer: for &layer in &layers {
            for layer_surface in stacked(layer).into_iter().rev() {
                // SAFETY: just read from the scene (`stacked`), and nothing
                // has run since.
                let layer_surface = unsafe { &*layer_surface };
                let wlr = &layer_surface.wlr_layer_surface;
                if wlr.is_mapped() && wlr.keyboard_interactive() == exclusive && !closing(layer_surface.ref_key) {
                    to_focus = Some(layer_surface.ref_key);
                    break 'outer;
                }
            }
        }

        for seat in seats.values_mut() {
            if let Some(key) = to_focus {
                seat.layer_shell.scheduled_focus = LayerShellSeatFocus::Exclusive(key);
            } else if matches!(seat.layer_shell.scheduled_focus, LayerShellSeatFocus::Exclusive(_)) {
                seat.layer_shell.scheduled_focus = LayerShellSeatFocus::None;
            } else if matches!(seat.layer_shell.scheduled_focus, LayerShellSeatFocus::NonExclusive(key) if closing(key)) {
                seat.layer_shell.scheduled_focus = LayerShellSeatFocus::None;
            }
        }
    }
}

/// The layer surfaces drawn in scene layer `layer` (`zwlr_layer_shell_v1`),
/// bottom of the stack first, copied out. Each is live until something
/// destroys a layer surface: the scene helper destroys a surface's tree,
/// and with it the node data naming the surface, before the surface's own
/// destroy handler frees it, so a surface a child of the layer's tree names
/// is live now.
fn stacked(layer: u32) -> Vec<*mut LayerSurface> {
    let mut out = Vec::new();
    let tree = crate::shared::scene().layer_surface_tree(layer);
    if tree.is_null() {
        return out;
    }
    // SAFETY: the shared scene's layer trees live as long as the scene; the
    // walk only reads the children list and each child's node data.
    unsafe {
        let head = ffi::river_scene_tree_get_children(tree) as *mut WlList;
        let mut curr = (*head).next;
        while curr != head {
            let node = ffi::river_scene_node_from_children_link(curr as *mut ffi::wl_list);
            if let Some(node_data) = SceneNodeData::from_node(node) {
                if let SceneNodeDataVal::LayerSurface(layer_surface) = node_data.data {
                    if !layer_surface.is_null() {
                        out.push(layer_surface);
                    }
                }
            }
            curr = (*curr).next;
        }
    }
    out
}

thread_local! {
    /// Layer surfaces `send_configures` found too large — an exclusive
    /// zone that would leave less than half the output — to be closed by
    /// `close_oversized`. Closing one inside the walk ran its unmap
    /// handler, which arranged the output again from inside the walk: that
    /// inner walk could close a second surface the outer one was about to
    /// visit, and the outer one then read the freed node. Three top bars
    /// and an output scale change (any client can make one) took the
    /// compositor down that way (2026-10-12).
    /// Also those whose output went away (`output_gone`).
    static OVERSIZED: std::cell::RefCell<Vec<Key>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Whether layer surface `key` is queued for closing.
pub(crate) fn closing(key: Key) -> bool {
    OVERSIZED.with(|o| o.borrow().contains(&key))
}

/// Close layer surface `key` once the current dispatch is over.
fn close_later(key: Key) {
    let first = OVERSIZED.with(|o| {
        let mut o = o.borrow_mut();
        let first = o.is_empty();
        if !o.contains(&key) {
            o.push(key);
        }
        first
    });
    let event_loop = crate::shared::event_loop();
    if first && !event_loop.is_null() && crate::shared::pending().loop_live() {
        // SAFETY: the shared loop is live (`loop_live`), and the callback
        // takes no data; an idle source frees itself once it has run.
        unsafe { ffi::wl_event_loop_add_idle(event_loop, Some(close_oversized), std::ptr::null_mut()) };
    }
}

/// The idle callback `close_later` arms: close each surface still alive.
unsafe extern "C" fn close_oversized(_data: *mut std::ffi::c_void) {
    let keys = OVERSIZED.with(|o| std::mem::take(&mut *o.borrow_mut()));
    let server = crate::shared::server_ptr();
    if server.is_null() {
        return;
    }
    for key in keys {
        // Looked up afresh for each, with no borrow held across the close:
        // closing runs the surface's unmap and destroy handlers, which reach
        // the window manager themselves.
        let Some(layer_surface) = (*crate::reentry::wm(server)).layer_surfaces.get(key) else { continue };
        let wlr = (*layer_surface).wlr_layer_surface.ptr();
        if !wlr.is_null() {
            log::info!("closing a queued layer surface (too large for its output, or its output went away)");
            ffi::wlr_layer_surface_v1_destroy(wlr);
        }
    }
}

unsafe extern "C" fn handle_new_surface(_listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
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

    // A layer surface that names no output goes on the first one still
    // there (one being destroyed has let go of its `wlr_output`).
    if (*wlr_layer_surface).output.is_null() {
        let first = (*crate::reentry::wm(crate::shared::server_ptr())).outputs.values().map(|o| o.wlr_output().ptr()).find(|p| !p.is_null());
        if let Some(wlr_output) = first {
            log::info!("layer surface named no output, choosing the first");
            (*wlr_layer_surface).output = wlr_output;
        } else {
            log::error!("no output available for layer surface {:?}", CStr::from_ptr((*wlr_layer_surface).namespace));
            ffi::wlr_layer_surface_v1_destroy(wlr_layer_surface);
            return;
        }
    }

    if let Err(e) = LayerSurface::create(wlr_layer_surface, &mut (*crate::reentry::wm(crate::shared::server_ptr())).layer_surfaces) {
        log::error!("Failed to create layer surface: {}", e);
        ffi::wl_resource_post_no_memory((*wlr_layer_surface).resource);
    }
}

/// The window manager's layer surfaces by key: the address of each live
/// `LayerSurface`, put here by `LayerSurface::create` and taken out by its
/// destroy handler before the box is freed. A newtype so that only that pair
/// can change it: safe paths (`Seat::focus`) dereference what they find.
pub struct LayerSurfaces(crate::slotmap::SlotMap<*mut LayerSurface>);

impl LayerSurfaces {
    /// Write an empty store into a zero-filled window manager
    /// (`Server::zeroed`). The only way to make one: with no public
    /// constructor, safe code has no second store to swap in for the live
    /// one, so `create`/destroy stay its only writers.
    ///
    /// # Safety
    /// `this` points at an uninitialised store; it is overwritten without
    /// being dropped.
    pub unsafe fn init_in_place(this: *mut Self) {
        std::ptr::write(this, LayerSurfaces(crate::slotmap::SlotMap::new()));
    }

    /// The live layer surface `key` names, if any.
    pub fn get(&self, key: Key) -> Option<*mut LayerSurface> {
        self.0.get(key).copied()
    }

    /// Every live layer surface.
    pub fn iter(&self) -> impl Iterator<Item = *mut LayerSurface> + '_ {
        self.0.iter().copied()
    }

    /// The root wl_surface of the layer surface `key` names; null when it
    /// is gone.
    pub fn root_surface(&self, key: Key) -> *mut ffi::wlr_surface {
        // SAFETY: an entry is live (see the type).
        self.get(key).map_or(std::ptr::null_mut(), |l| unsafe { (*l).wlr_layer_surface.surface() })
    }
}

pub struct LayerSurface {
    /// Its key in `wm.layer_surfaces`; private, because the destroy handler
    /// takes out the entry it names.
    ref_key: Key,
    /// Private, as every `Wlr` field reachable through `&mut` is: one moved
    /// out of here would be a hold the destroy handler never clears.
    wlr_layer_surface: crate::wlr::Wlr<ffi::wlr_layer_surface_v1>,
    /// wlroots' scene helper for the surface, which lives as long as the
    /// layer surface does. Private: `send_configures` hands it to wlroots.
    scene_layer_surface: *mut ffi::wlr_scene_layer_surface_v1,
    /// The helper's tree, watched: the helper destroys it with the surface.
    tree: crate::scene_handle::SceneTree,
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
    pub animation_timer: crate::event_source::Timer,

    destroy: crate::listener::Listener,
    map: crate::listener::Listener,
    unmap: crate::listener::Listener,
    commit: crate::listener::Listener,
    new_popup: crate::listener::Listener,
    pub parent_offset_applied: bool,
}

// SAFETY: made by `Box::into_raw` and freed only by its own destroy path,
// after its listeners are gone, and reached only through raw pointers: no safe
// API returns a `&mut`, a `Box` or a value of it, so nothing safe can move it.
unsafe impl crate::listener::Anchored for LayerSurface {}

impl LayerSurface {
    /// `surfaces` is the window manager's layer-surface store.
    pub unsafe fn create(
        wlr_layer_surface: *mut ffi::wlr_layer_surface_v1,
        surfaces: &mut LayerSurfaces,
    ) -> Result<*mut Self, &'static str> {
        let layer_tree = crate::shared::scene().layer_surface_tree((*wlr_layer_surface).current.layer);
        let scene_layer_surface = ffi::wlr_scene_layer_surface_v1_create(layer_tree, wlr_layer_surface);
        if scene_layer_surface.is_null() {
            return Err("Failed to create wlr_scene_layer_surface_v1");
        }

        let popup_tree = ffi::wlr_scene_tree_create(crate::shared::scene().layers.popups.raw());
        if popup_tree.is_null() {
            ffi::wlr_scene_node_destroy((*scene_layer_surface).tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create popup_tree");
        }

        let layer_surface = Box::into_raw(Box::new(LayerSurface {
            ref_key: Key { generation: 0, index: 0 },
            wlr_layer_surface: crate::wlr::Wlr::from_raw(wlr_layer_surface),
            scene_layer_surface,
            tree: crate::scene_handle::SceneTree::watch((*scene_layer_surface).tree),
            popup_tree: crate::scene_handle::SceneTree::adopt(popup_tree),
            opacity: 1.0,
            opacity_target: 1.0,
            opacity_step: 1.0,
            animation_timer: crate::event_source::Timer::new(),
            destroy: std::mem::zeroed(),
            map: std::mem::zeroed(),
            unmap: std::mem::zeroed(),
            commit: std::mem::zeroed(),
            new_popup: std::mem::zeroed(),
            parent_offset_applied: false,
        }));

        let key = surfaces.0.put(layer_surface);
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

    /// Its key in `wm.layer_surfaces`.
    pub fn key(&self) -> Key {
        self.ref_key
    }

    /// The wlroots layer surface.
    pub fn wlr_layer_surface(&self) -> &crate::wlr::Wlr<ffi::wlr_layer_surface_v1> {
        &self.wlr_layer_surface
    }

    /// Whether its namespace marks it as cce-cloud chrome — the launcher
    /// and the desktop/app context menus. These are screen-anchored popups
    /// that stay interactive during overview and dismiss on click-away.
    pub fn is_cloud(&self) -> bool {
        self.wlr_layer_surface.with_namespace(|ns| ns.starts_with("cce-cloud"))
    }

    /// Whether a click or touch on it may give it keyboard focus. Not when
    /// it asked for none: per wlr-layer-shell such a surface is never given
    /// keyboard focus, and the one that relies on it — an on-screen
    /// keyboard (cce-keyboard), typing into the window it was clicked over
    /// — cannot work if a click on its keys takes focus off that window.
    pub fn takes_click_focus(&self) -> bool {
        !self.wlr_layer_surface.is_null()
            && self.wlr_layer_surface.keyboard_interactive()
                != ffi::zwlr_layer_surface_v1_keyboard_interactivity_ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE
    }
}

unsafe extern "C" fn handle_layer_surface_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let layer_surface = crate::listener_owner!(listener, LayerSurface, destroy);

    (*layer_surface).wlr_layer_surface.with_namespace(|ns| log::debug!("layer surface {ns:?} destroyed"));

    (*layer_surface).animation_timer.remove();

    (*layer_surface).destroy.disconnect();
    (*layer_surface).map.disconnect();
    (*layer_surface).unmap.disconnect();
    (*layer_surface).commit.disconnect();
    (*layer_surface).new_popup.disconnect();

    (*layer_surface).wlr_layer_surface.destroy_popups();

    (*layer_surface).popup_tree.destroy();

    ffi::river_wlr_surface_set_data((*layer_surface).wlr_layer_surface.surface(), std::ptr::null_mut());

    let server = crate::shared::server_ptr();
    (*crate::reentry::wm(server)).layer_surfaces.0.remove((*layer_surface).ref_key);
    (*layer_surface).wlr_layer_surface.take();
    let _ = Box::from_raw(layer_surface);
}

/// Steps one layer surface's dissolve toward `opacity_target` and re-arms
/// itself until it lands. Unlike the window fade — which rides the window
/// manager's shared border-fade timer — each layer surface keeps its own,
/// because a layer surface is not in `wm.windows` and there is no list to
/// sweep.
unsafe extern "C" fn handle_animation_tick(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    // The token is the surface's key in `wm.layer_surfaces`.
    let token = data as usize;
    let key = crate::slotmap::Key { generation: (token >> 32) as u32, index: token as u32 };
    let wm = &*crate::reentry::wm(crate::shared::server_ptr());
    let Some(layer_surface) = wm.layer_surfaces.get(key) else { return 0 };

    let target = (*layer_surface).opacity_target;
    let delta = target - (*layer_surface).opacity;
    let settled = if delta.abs() <= (*layer_surface).opacity_step {
        (*layer_surface).opacity = target;
        true
    } else {
        (*layer_surface).opacity += (*layer_surface).opacity_step * delta.signum();
        false
    };

    (*layer_surface).tree.set_opacity((*layer_surface).opacity);

    if settled {
        (*layer_surface).animation_timer.remove();
    } else {
        (*layer_surface).animation_timer.arm(16);
    }

    0
}

impl LayerSurface {
    /// Begin a dissolve toward `target` (0.0 out, 1.0 in) over `ms`. A `ms`
    /// of 0 snaps, so callers can treat this as "put the surface at
    /// `target`" whether or not fading is configured on.
    pub fn start_fade(&mut self, target: f32, ms: u32) {
        self.opacity_target = target.clamp(0.0, 1.0);
        self.animation_timer.remove();
        if ms == 0 {
            self.opacity = self.opacity_target;
            self.tree.set_opacity(self.opacity);
            return;
        }
        // Ticks at 16 ms; at least one step so a sub-frame duration still
        // lands rather than dividing by zero.
        let ticks = ((ms as f32) / 16.0).max(1.0);
        self.opacity_step = ((self.opacity_target - self.opacity).abs() / ticks).max(1.0e-4);
        self.tree.set_opacity(self.opacity);

        let display = crate::shared::shared().display();
        let timer = if display.is_null() {
            crate::event_source::Timer::new()
        } else {
            // The token is the surface's key in `wm.layer_surfaces`.
            let key = self.ref_key;
            // SAFETY: `handle_animation_tick` is an entry point that reads its
            // data as a store key and looks the surface up by it.
            unsafe {
                crate::event_source::Timer::on_shared_loop(handle_animation_tick, ((key.generation as usize) << 32) | key.index as usize)
            }
        };
        if !timer.is_set() {
            log::error!("Failed to create layer surface animation timer");
            // No timer means no ramp; land on the target rather than leave
            // the surface stranded at whatever it was mid-fade.
            self.opacity = self.opacity_target;
            self.tree.set_opacity(self.opacity);
        } else {
            self.animation_timer = timer;
            self.animation_timer.arm(16);
        }
    }

    /// PID of the client owning this layer surface, from its wl_resource.
    /// 0 when it cannot be read. Used to resolve a `fade-out` to the caller.
    pub fn client_pid(&self) -> i32 {
        self.wlr_layer_surface.client_pid()
    }
}

/// Run `f`, then ask for a manage pass if it changed any seat's scheduled
/// layer focus, else a render pass. `f` is handed the window manager: it
/// may arrange outputs, which no longer closes surfaces inside the walk
/// (`close_later`), so nothing it runs reaches the window manager another
/// way.
fn update_scheduled_focus_and_dirty_windowing(
    wm: &mut crate::window_manager::WindowManager,
    f: impl FnOnce(&mut crate::window_manager::WindowManager),
) {
    let old_focuses: Vec<_> = wm.seats.values().map(|seat| seat.layer_shell.scheduled_focus).collect();

    f(wm);

    let changed = wm
        .seats
        .values()
        .zip(&old_focuses)
        .any(|(seat, old)| seat.layer_shell.scheduled_focus != *old);

    if changed {
        crate::shared::pending().dirty_windowing();
    } else {
        crate::shared::pending().dirty_rendering();
    }
}


/// Arrange the layer surfaces of the output that holds `wlr_output`;
/// nothing when none does.
fn arrange_output_of(outputs: &mut crate::output::Outputs, wlr_output: *mut ffi::wlr_output) {
    if let Some(output) = outputs.find(wlr_output).and_then(|id| outputs.get_mut(id)) {
        output.arrange_layers();
    }
}

/// `wlr_output` is being destroyed: forget it as the output of every layer
/// surface on it, and close them once this dispatch is over. wlroots does
/// not follow a layer surface's output, so each would keep the freed
/// pointer and read it on its next commit; a later output at the same
/// address would adopt it.
pub(crate) fn output_gone(surfaces: &LayerSurfaces, wlr_output: *mut ffi::wlr_output) {
    if wlr_output.is_null() {
        return;
    }
    for layer_surface in surfaces.iter() {
        // SAFETY: an entry is live (see `LayerSurfaces`).
        let (on_it, key) = unsafe { ((*layer_surface).wlr_layer_surface.output() == wlr_output, (*layer_surface).ref_key) };
        if on_it {
            log::info!("closing a layer surface whose output went away");
            // SAFETY: as above.
            unsafe { (*layer_surface).wlr_layer_surface.forget_output() };
            close_later(key);
        }
    }
}

unsafe extern "C" fn handle_layer_surface_map(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let layer_surface = crate::listener_owner!(listener, LayerSurface, map);

    (*layer_surface).wlr_layer_surface.with_namespace(|ns| log::debug!("layer surface {ns:?} mapped"));

    // Overlay layer only: these are the transient surfaces the user opens
    // (the launcher, the notifier), so a dissolve reads as the thing
    // arriving. The Background/Bottom/Top layers are the desktop's own
    // furniture — wallpaper, status bar — and map once at login, where a
    // fade reads as the desktop failing to draw.
    if (*layer_surface).wlr_layer_surface.layer() == ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY {
        let ms = if cce_core::motion::enabled() { crate::shared::layout().fade_in_ms } else { 0 };
        if ms > 0 {
            (*layer_surface).opacity = 0.0;
        }
        (*layer_surface).start_fade(1.0, ms);
    }

    let key = (*layer_surface).ref_key;
    let on_demand = (*layer_surface).wlr_layer_surface.keyboard_interactive()
        == ffi::zwlr_layer_surface_v1_keyboard_interactivity_ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_ON_DEMAND;
    let wlr_output = (*layer_surface).wlr_layer_surface.output();
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    update_scheduled_focus_and_dirty_windowing(wm, |wm| {
        if on_demand {
            for seat in wm.seats.values_mut() {
                if !matches!(seat.layer_shell.scheduled_focus, LayerShellSeatFocus::Exclusive(_)) {
                    seat.layer_shell.scheduled_focus = LayerShellSeatFocus::NonExclusive(key);
                }
            }
        }
        arrange_output_of(&mut wm.outputs, wlr_output);
        LayerShell::check_exclusive_focus(&mut wm.seats);
    });
}

unsafe extern "C" fn handle_layer_surface_unmap(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let layer_surface = crate::listener_owner!(listener, LayerSurface, unmap);

    (*layer_surface).wlr_layer_surface.with_namespace(|ns| log::debug!("layer surface {ns:?} unmapped"));

    (*layer_surface).animation_timer.remove();

    let key = (*layer_surface).ref_key;
    let root = (*layer_surface).wlr_layer_surface.surface();
    let wlr_output = (*layer_surface).wlr_layer_surface.output();
    // cce-cloud surfaces skip the focus_next fallback: the bare launcher is
    // about to be replaced by whatever it spawned, and refocusing the old
    // window first would fight the new map. But a PARENTED popup
    // ("cce-cloud:<app-id>", e.g. the designer's add-node palette) is
    // chrome OF that app — closing it must hand the keyboard straight back
    // to its parent, not leave the seat focused on nothing.
    let (is_cce_cloud, cloud_parent) = (*layer_surface).wlr_layer_surface.with_namespace(|ns| {
        (ns.starts_with("cce-cloud"), ns.strip_prefix("cce-cloud:").map(str::to_string))
    });
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    update_scheduled_focus_and_dirty_windowing(wm, |wm| {
        for sid in wm.seats.ids() {
            if wm.seats.get(sid).is_some_and(|seat| seat.focused == crate::seat::Focus::LayerSurface(root)) {
                crate::seat::Seat::focus_window(wm, sid, None);
                if let Some(parent_app_id) = &cloud_parent {
                    // Status modules parent their submenus too; the bar is
                    // never a keyboard-focus target, so those keep the old
                    // leave-it-unfocused behavior.
                    if !parent_app_id.starts_with("cce-status") {
                        let parent = wm.windows.values().find(|w| {
                            !w.closed
                                && !w.minimized
                                && matches!(w.state, crate::window::WindowState::Mapped)
                                && w.get_app_id_string().as_deref() == Some(parent_app_id.as_str())
                        });
                        if let Some(id) = parent.map(|w| w.id()) {
                            // Dismissing chrome, not switching windows: the
                            // camera stays where the user left it.
                            if let Some(seat) = wm.seats.get_mut(sid) {
                                seat.suppress_focus_pan = true;
                            }
                            crate::seat::Seat::focus_window(wm, sid, Some(id));
                            if let Some(seat) = wm.seats.get_mut(sid) {
                                seat.suppress_focus_pan = false;
                            }
                        }
                    }
                } else if !is_cce_cloud {
                    wm.focus_next_visible_window(sid);
                }
            }
            if let Some(seat) = wm.seats.get_mut(sid) {
                if seat.layer_shell.scheduled_focus == LayerShellSeatFocus::NonExclusive(key) {
                    seat.layer_shell.scheduled_focus = LayerShellSeatFocus::None;
                }
            }
        }
        arrange_output_of(&mut wm.outputs, wlr_output);
        LayerShell::check_exclusive_focus(&mut wm.seats);
    });
}

unsafe extern "C" fn handle_layer_surface_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let layer_surface = crate::listener_owner!(listener, LayerSurface, commit);
    let wlr = &(*layer_surface).wlr_layer_surface;
    let layer = wlr.layer();

    if layer != ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND {
        let layout = crate::shared::layout();
        let is_status = wlr.with_namespace(|ns| ns == "cce-status" || ns == "cce-status-interface");
        let (blur_enabled, ignore_transparent) = if is_status {
            (layout.status_background_blur > 0.001, layout.status_backdrop_blur_ignore_transparent)
        } else {
            (layout.window_blur, layout.window_backdrop_blur_ignore_transparent)
        };
        // Optimized (cached) blur is counterproductive for surfaces stacked ABOVE
        // windows (Top/Overlay): the scene graph re-dirties an optimized-blur node
        // whenever any node below it updates (scenefx wlr_scene.c:744), so window
        // content panning underneath forces a full re-bake every frame — a fixed-
        // position shimmer (e.g. the always-mapped cce-notifier overlay). Regular
        // blur is immune to that path and only re-bakes on real damage, so fall back
        // to it here, exactly as status surfaces already do. Bottom/Background layers
        // sit below windows and are unaffected, so they keep the cache.
        let above_windows = layer == ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_TOP
            || layer == ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY;
        let use_optimized = if is_status || above_windows { false } else { layout.scenefx_optimized_blur };
        let wlr_surface = wlr.surface();
        let (geom_w, geom_h) = if !wlr_surface.is_null() {
            (ffi::river_wlr_surface_get_width(wlr_surface), ffi::river_wlr_surface_get_height(wlr_surface))
        } else {
            (wlr.actual_width() as i32, wlr.actual_height() as i32)
        };
        // A corner radius of 0 preserves existing behaviour: layer surfaces
        // (status bar, etc.) never had a blur radius applied, and their corner
        // rounding is handled separately.
        (*layer_surface).tree.enable_blur(blur_enabled, use_optimized, ignore_transparent, 0, 0, geom_w, geom_h, 0);
    }

    if (*layer_surface).opacity < 1.0 {
        (*layer_surface).tree.set_opacity((*layer_surface).opacity);
    }

    let wlr_output = wlr.output();
    let server = crate::shared::server_ptr();
    let Some(output_id) = (*crate::reentry::wm(server)).outputs.find(wlr_output) else { return };

    // Position offset for cce-cloud sub-modules
    if !(*layer_surface).parent_offset_applied {
        let parent_app_id = wlr.with_namespace(|ns| ns.strip_prefix("cce-cloud:").map(str::to_string));
        if let Some(parent_app_id) = parent_app_id {
            let wm = &*crate::reentry::wm(server);
            let parent = wm.windows.values().find(|w| !w.closed && w.get_app_id_string().as_deref() == Some(parent_app_id.as_str()));
            if let Some(parent) = parent {
                let (px, py, parent_w) = (parent.rendering_requested.x, parent.rendering_requested.y, parent.box_geom.width);
                let (mut output_x, mut output_y, mut output_w) = (0, 0, 0);
                if let Some(output) = wm.outputs.values().find(|o| o.wlr_output().ptr() == wlr_output) {
                    let wlr_box = output.sent.box_layout();
                    (output_x, output_y, output_w) = (wlr_box.x, wlr_box.y, wlr_box.width);
                }

                let relative_parent_x = px - output_x;
                let relative_parent_y = py - output_y;
                let is_align_right = (wlr.anchor() & ffi::zwlr_layer_surface_v1_anchor_ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT) != 0;

                // Written into wlroots' state directly: the client's margin,
                // shifted to sit on its parent window.
                let raw = wlr.ptr();
                if is_align_right {
                    (*raw).pending.margin.right = (output_w - relative_parent_x - parent_w) + (*raw).pending.margin.right;
                    (*raw).current.margin.right = (*raw).pending.margin.right;
                } else {
                    (*raw).pending.margin.left = relative_parent_x + (*raw).pending.margin.left;
                    (*raw).current.margin.left = (*raw).pending.margin.left;
                }
                (*raw).pending.margin.top = relative_parent_y + (*raw).pending.margin.top;
                (*raw).current.margin.top = (*raw).pending.margin.top;

                (*layer_surface).parent_offset_applied = true;
            }
        }
    }

    // Check if layer was changed
    if wlr.committed() & ffi::wlr_layer_surface_v1_state_field_WLR_LAYER_SURFACE_V1_STATE_LAYER != 0 {
        if let Some(tree) = crate::shared::scene().layer_tree(layer) {
            (*layer_surface).tree.reparent(tree);
        }
    }

    if wlr.initial_commit() || wlr.committed() != 0 {
        let wm = &mut *crate::reentry::wm(server);
        update_scheduled_focus_and_dirty_windowing(wm, |wm| {
            if let Some(output) = wm.outputs.get_mut(output_id) {
                output.arrange_layers();
            }
            LayerShell::check_exclusive_focus(&mut wm.seats);
        });
    }
}

unsafe extern "C" fn handle_layer_surface_new_popup(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let layer_surface = crate::listener_owner!(listener, LayerSurface, new_popup);
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

/// What arranging the layer surfaces reads of their output, copied out so
/// that the output's `layer_shell` can be borrowed mutably beside it (this
/// replaced a raw `*mut Output` the methods below dereferenced while a
/// `&mut` to its `layer_shell` was live).
#[derive(Clone, Copy)]
pub struct LayerOutput {
    /// The scheduled box, layout coordinates.
    pub scheduled: ffi::wlr_box,
    /// The box the last manage pass applied.
    pub sent: ffi::wlr_box,
    pub state: crate::output::OutputStateValue,
    /// Which output this is, to pick its surfaces: compared, never
    /// dereferenced.
    pub wlr_output: *mut ffi::wlr_output,
}

fn same_box(a: &ffi::wlr_box, b: &ffi::wlr_box) -> bool {
    a.x == b.x && a.y == b.y && a.width == b.width && a.height == b.height
}

impl LayerShellOutput {
    pub fn arrange(&mut self, output: LayerOutput) {
        self.scheduled.non_exclusive_area = output.scheduled;
        self.send_configures(&output, true);
        self.send_configures(&output, false);

        let area_changed = match self.sent.non_exclusive_area {
            Some(sent_box) => !same_box(&sent_box, &self.scheduled.non_exclusive_area),
            None => true,
        };

        if area_changed {
            crate::shared::pending().dirty_windowing();
        }
    }

    fn send_configures(&mut self, output: &LayerOutput, exclusive: bool) {
        let output_box = output.scheduled;
        let (output_width, output_height) = (output_box.width, output_box.height);

        let layers = [
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_TOP,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY,
        ];

        // The stacking order is copied out first, and a surface found too
        // large is closed after the walk (`close_later`): closing it here
        // ran its unmap handler, which arranged the output again from inside
        // this walk.
        for &layer in &layers {
            for layer_surface in stacked(layer) {
                // SAFETY: from `stacked`, and nothing below destroys a layer
                // surface: configuring one only queues an event and moves its
                // node.
                let layer_surface = unsafe { &*layer_surface };
                let wlr = &layer_surface.wlr_layer_surface;
                if !wlr.is_mapped() && !wlr.initial_commit() {
                    continue;
                }
                if wlr.output() != output.wlr_output {
                    continue;
                }
                if (wlr.exclusive_zone() > 0) != exclusive {
                    continue;
                }

                let mut new_area = self.scheduled.non_exclusive_area;
                // SAFETY: the scene helper lives as long as its layer surface
                // (live, above); both boxes are locals.
                unsafe {
                    ffi::wlr_scene_layer_surface_v1_configure(layer_surface.scene_layer_surface, &output_box, &mut new_area);
                }

                if new_area.width < (output_width / 2) || new_area.height < (output_height / 2) {
                    close_later(layer_surface.ref_key);
                    continue;
                }
                self.scheduled.non_exclusive_area = new_area;

                let (x, y) = layer_surface.tree.position().unwrap_or((0, 0));
                layer_surface.popup_tree.set_position(x, y);

                let clip = ffi::wlr_box {
                    x: -(x - output_box.x),
                    y: -(y - output_box.y),
                    width: output_width,
                    height: output_height,
                };
                layer_surface.tree.set_subsurface_clip(Some(&clip));
            }
        }
    }

    pub fn manage_start(&mut self, output: LayerOutput) {
        assert!(
            matches!(output.state, crate::output::OutputStateValue::Enabled)
                || matches!(output.state, crate::output::OutputStateValue::DisabledSoft)
        );

        if !same_box(&output.scheduled, &output.sent) {
            self.scheduled.non_exclusive_area = output.scheduled;
            self.send_configures(&output, true);
            self.send_configures(&output, false);
        }

        let area_changed = match self.sent.non_exclusive_area {
            Some(sent_box) => !same_box(&sent_box, &self.scheduled.non_exclusive_area),
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
