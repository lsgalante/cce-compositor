//! The manage / render transaction: when windowing and rendering are dirty, the
//! manage_start / manage_finish and render_start / render_finish passes that
//! apply a layout and present it, and the timers between them. Split out of
//! window_manager.rs on 2026-10-10.

use super::*;

impl WindowManager {
    /// Switch overview on or off, arming the handle fade on a real change.
    ///
    /// Every site that flips the mode goes through here. Resize handles only
    /// exist in overview (`window::draw_borders`), so the transition has to
    /// start the fade timer or they would pop in on the next unrelated
    /// redraw instead of easing — and the zoom paths that set the mode do it
    /// every frame, hence the equality guard.
    pub unsafe fn set_mode(&mut self, mode: WindowManagerMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        // Overview decides whether a fullscreen window owns the top of the
        // stack (`fullscreen_on_top`), and the stacking pass runs only on a
        // transaction.
        if self.windows.iter().any(|&w| !w.is_null() && !(*w).closed && (*w).is_fullscreen()) {
            self.dirty_windowing();
        }
        // The selection belongs to overview: it is made there, and the
        // group move it exists for is an overview drag.
        if mode != WindowManagerMode::Overview {
            self.selection_clear();
        }
        self.arm_border_fade();
        // The `adjust` status topic follows window_adjust_active().
        self.update_status();
    }

    /// Overview, or Super held: the focused window shows its frame and
    /// its body drags it. Every site that gates the handles — the hit
    /// test, the reveal, the catcher rects, hover-to-focus, the body
    /// grab — asks this, so the two ways in cannot drift apart.
    pub fn window_adjust_active(&self) -> bool {
        self.mode == WindowManagerMode::Overview || self.adjust_held
    }

    /// Re-read whether Super is held (the seat keyboard's live mask, or an
    /// injected one) and, on a change, bring the desk into or out of
    /// adjust mode: fade the frame in/out and re-evaluate the pointer in
    /// place, so the ring lands on the window under a still pointer the
    /// moment the key goes down and the app gets its hover back when it
    /// comes up. A drag in progress is left alone — it ends on release.
    pub unsafe fn refresh_adjust_held(&mut self) {
        let mut held = self.injected_super_held;
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            let kb = ffi::river_wlr_seat_get_keyboard((*seat).wlr_seat);
            if !kb.is_null() && (ffi::wlr_keyboard_get_modifiers(kb) & ffi::wlr_keyboard_modifier_WLR_MODIFIER_LOGO) != 0 {
                held = true;
            }
            curr_seat = (*curr_seat).next;
        }
        if held == self.adjust_held {
            return;
        }
        self.adjust_held = held;
        self.arm_border_fade();
        // The `adjust` status topic: the desktop grid shows its image
        // handles in step with the windows'.
        self.update_status();
        let now = crate::util::msec_timestamp();
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if (*seat).op.is_none() {
                (*seat).cursor.passthrough(now);
            }
            curr_seat = (*curr_seat).next;
        }
    }

    /// Start the border hover fade if it isn't already running. Idempotent —
    /// re-arming mid-fade would restart the timer and step it twice as fast.
    pub unsafe fn arm_border_fade(&mut self) {
        if self.border_fade_running || self.border_fade_timer.is_null() {
            return;
        }
        self.border_fade_running = true;
        ffi::wl_event_source_timer_update(self.border_fade_timer, 16);
    }

    #[track_caller]
    pub unsafe fn dirty_windowing(&mut self) {
        // Capturing and symbolizing a backtrace costs far more than the event it
        // annotates, and this fires on routine commits — the session runs at
        // --log-level debug, so keying it on Debug meant ~160 log lines/second
        // and most of a 19MB session log. Behind its own switch now.
        if dirty_backtrace_debug() {
            let bt = std::backtrace::Backtrace::force_capture();
            log::debug!("dirty_windowing called from backtrace:\n{}", bt);
        }
        if dirty_trace() {
            log::debug!("dirty_windowing from {}", std::panic::Location::caller());
        }
        self.scheduled.dirty = true;
        self.add_dirty_idle();
    }
 
    pub unsafe fn clean_windowing(&mut self) {
        self.scheduled.dirty = false;
        self.scheduled.dirty_lazy = false;
        self.remove_dirty_idle();
    }
 
    #[track_caller]
    pub unsafe fn dirty_rendering(&mut self) {
        if dirty_trace() {
            log::debug!("dirty_rendering from {}", std::panic::Location::caller());
        }
        self.rendering_scheduled.dirty = true;
        self.add_dirty_idle();
    }

    pub unsafe fn clean_rendering(&mut self) {
        self.rendering_scheduled.dirty = false;
        self.remove_dirty_idle();
    }

    pub(crate) unsafe fn add_dirty_idle(&mut self) {
        if self.scheduled.dirty || self.scheduled.dirty_lazy || self.rendering_scheduled.dirty {
            if self.dirty_idle.is_null() {
                let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
                self.dirty_idle = ffi::wl_event_loop_add_idle(
                    event_loop,
                    Some(dirty_idle_callback),
                    self as *mut WindowManager as *mut _,
                );
            }
        }
    }

    pub(crate) unsafe fn remove_dirty_idle(&mut self) {
        if !self.scheduled.dirty && !self.scheduled.dirty_lazy && !self.rendering_scheduled.dirty {
            if !self.dirty_idle.is_null() {
                ffi::wl_event_source_remove(self.dirty_idle);
                self.dirty_idle = std::ptr::null_mut();
            }
        }
    }

    pub unsafe fn manage_start(&mut self) {
        assert!(matches!(self.state, WindowManagerState::Idle));
        assert!(self.scheduled.dirty);
        self.clean_windowing();
        self.state = WindowManagerState::Manage;

        log::debug!("manage sequence start");

        let mt0 = if manage_debug() { Some(std::time::Instant::now()) } else { None };

        (*self.server).om.auto_layout();
        let mt_auto = mt0.map(|s| s.elapsed().as_micros());

        let outputs = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs).next;
        while curr != outputs {
            let next = (*curr).next;
            let output = crate::container_of!(curr, crate::output::Output, link);
            (*output).manage_start();
            curr = next;
        }

        let mt_outputs = mt0.map(|s| s.elapsed().as_micros());

        if !self.sent.output_config.is_null() {
            log::warn!("sent.output_config was not null in manage_start, destroying old configuration");
            ffi::wlr_output_configuration_v1_send_failed(self.sent.output_config);
            ffi::wlr_output_configuration_v1_destroy(self.sent.output_config);
            self.sent.output_config = std::ptr::null_mut();
        }
        self.sent.output_config = self.scheduled.output_config;
        self.scheduled.output_config = std::ptr::null_mut();

        for &win_ptr in self.windows.iter() {
            (*win_ptr).manage_start();
        }
        let mt_windows = mt0.map(|s| s.elapsed().as_micros());

        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            (*seat).manage_start();
            curr = next;
        }

        let mt_seats = mt0.map(|s| s.elapsed().as_micros());

        self.arrange_views();
        self.debug_check_unlinked_status("manage_start end");

        if let (Some(s), Some(a), Some(o), Some(w), Some(t)) =
            (mt0, mt_auto, mt_outputs, mt_windows, mt_seats)
        {
            let total = s.elapsed().as_micros();
            log::info!(
                "[manage] start total={}us auto_layout={}us outputs={}us windows={}us(n={}) seats={}us arrange={}us",
                total, a, o - a, w - o, self.windows.count(), t - w, total - t
            );
        }

        self.manage_finish();
    }

    /// Wedge tracer: a Mapped status window outside the render list is
    /// invisible to configures and render_finish — exactly the tray
    /// mis-slot wedge. Silent unless one exists.
    pub(crate) unsafe fn debug_check_unlinked_status(&self, phase: &str) {
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed {
                continue;
            }
            if !matches!((*w).state, crate::window::WindowState::Mapped) {
                continue;
            }
            if (*w).is_linked() {
                continue;
            }
            // Borrowed (`is_status_bar`), not `get_app_id_string`: this runs
            // three times a transaction — once a vblank during a drag — and
            // allocated a String per window to prefix-match it.
            if (*w).is_status_bar() {
                log::info!("[LinkDbg] UNLINKED-MAPPED at {}: app={:?} link.prev_self={} link.prev_null={}",
                    phase,
                    (*w).get_app_id_string(),
                    (*w).node.link.prev == &(*w).node.link as *const ffi::wl_list as *mut ffi::wl_list,
                    (*w).node.link.prev.is_null());
            }
        }
        self.debug_check_render_list(phase);
    }

    /// Structural check of rendering_requested.list: every member's neighbor
    /// pointers must agree, and every Mapped status window must be reachable
    /// from the head. Silent when consistent.
    pub(crate) unsafe fn debug_check_render_list(&self, phase: &str) {
        let head = &self.rendering_requested.list as *const ffi::wl_list as *mut WlList;
        let mut members: Vec<*mut WlList> = Vec::new();
        let mut curr = (*head).next;
        let mut steps = 0;
        while curr != head {
            if curr.is_null() {
                log::info!("[LinkDbg] LIST BROKEN at {}: null next after {} steps", phase, steps);
                return;
            }
            if (*(*curr).next).prev != curr {
                log::info!("[LinkDbg] LIST INCONSISTENT at {}: member {:p} next.prev mismatch", phase, curr);
            }
            members.push(curr);
            curr = (*curr).next;
            steps += 1;
            if steps > 10000 {
                log::info!("[LinkDbg] LIST CYCLE at {}: >10000 members", phase);
                return;
            }
        }
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed {
                continue;
            }
            if !matches!((*w).state, crate::window::WindowState::Mapped) {
                continue;
            }
            if !(*w).is_status_bar() {
                continue;
            }
            let node = &(*w).node.link as *const ffi::wl_list as *mut WlList;
            let reachable = members.contains(&node);
            if (*w).is_linked() && !reachable {
                log::info!("[LinkDbg] ORPHAN-RING at {}: app={:?} is_linked=true but unreachable from head",
                    phase, (*w).get_app_id_string());
            }
        }
    }

    pub unsafe fn manage_finish(&mut self) {
        assert!(matches!(self.state, WindowManagerState::Manage));
        self.cancel_timeout_timer();

        log::debug!("manage sequence finish");

        let seats = &mut self.sent.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link_sent);
            (*seat).manage_finish();
            curr = next;
        }

        self.state = WindowManagerState::InflightConfigures(0);

        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            let window = (*node).window();
            if (*window).manage_finish() {
                if !(*window).wm_requested.resizing {
                    if let WindowManagerState::InflightConfigures(ref mut count) = self.state {
                        *count += 1;
                    }
                }
            }
            curr = next;
        }

        if let WindowManagerState::InflightConfigures(count) = self.state {
            log::debug!("sent {} tracked configure(s)", count);
            self.debug_check_unlinked_status("manage_finish end");
            if count > 0 {
                self.start_timeout_timer(100);
            } else {
                self.render_start();
            }
        }
    }

    pub(crate) unsafe fn start_timeout_timer(&mut self, ms: u32) {
        if !self.timeout.is_null() {
            ffi::wl_event_source_timer_update(self.timeout, ms as i32);
        }
    }

    pub(crate) unsafe fn cancel_timeout_timer(&mut self) {
        if !self.timeout.is_null() {
            ffi::wl_event_source_timer_update(self.timeout, 0);
        }
    }

    pub unsafe fn notify_configured(&mut self) {
        if let WindowManagerState::InflightConfigures(ref mut count) = self.state {
            *count -= 1;
            if *count == 0 {
                self.cancel_timeout_timer();
                self.render_start();
            }
        }
    }

    pub unsafe fn render_start(&mut self) {
        assert!(matches!(self.state, WindowManagerState::InflightConfigures(0)) ||
                (matches!(self.state, WindowManagerState::Idle) && self.rendering_scheduled.dirty));
        self.state = WindowManagerState::Render;
        self.clean_rendering();

        log::debug!("render sequence start");

        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            let window = (*node).window();
            (*window).render_start();
            curr = next;
        }

        self.render_finish();
    }

    pub unsafe fn render_finish(&mut self) {
        assert!(matches!(self.state, WindowManagerState::Render));
        self.state = WindowManagerState::Idle;
        self.cancel_timeout_timer();

        let rf0 = if manage_debug() { Some(std::time::Instant::now()) } else { None };

        log::debug!("render sequence finish");

        for &window in self.windows.iter() {
            if !matches!((*window).state, crate::window::WindowState::Closing) {
                (*window).surfaces.drop_saved();
            }
            if matches!((*window).state, crate::window::WindowState::Init) {
                ffi::wlr_scene_node_reparent((*window).tree as *mut ffi::wlr_scene_node, (*self.server).scene.hidden_tree);
            }
            if let crate::window::WindowImpl::Destroying = (*window).impl_type {
                Window::destroy(window);
            }
        }

        let has_wallpaper = self.windows.iter().any(|&w| !w.is_null() && !(*w).closed && (*w).is_wallpaper());
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let next_out = (*curr_out).next;
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                if !(*output).background_rect.is_null() {
                    ffi::wlr_scene_node_set_enabled((*output).background_rect as *mut ffi::wlr_scene_node, !has_wallpaper);
                }
            }
            curr_out = next_out;
        }

        self.keep_status_bar_on_top();

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.layout.overlay_behavior.hash(&mut hasher);
        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            let window = (*node).window();
            (*window).ref_key.hash(&mut hasher);
            rendered_fullscreen(window).hash(&mut hasher);
            fullscreen_on_top(window).hash(&mut hasher);
            (*window).rendering_requested.circular.hash(&mut hasher);
            (*window).rendering_requested.hidden.hash(&mut hasher);
            (*window).tiling_mode.hash(&mut hasher);
            // A status segment's layer flips between top and popups
            // on expand/contract (see the reorder pass below), so
            // expansion state must participate in the hash — without
            // it the restack waits for an unrelated reorder, and the
            // open menu sits UNDER its sibling segments (their text
            // stays unblurred over the menu) until one happens.
            if (*window).tiling_mode == crate::tiling::TilingMode::Status {
                let bg = (*window).box_geom;
                let thickness = match (*window).status_edge {
                    crate::policy::arrange::StatusEdge::Left
                    | crate::policy::arrange::StatusEdge::Right => bg.width,
                    _ => bg.height,
                };
                (thickness > self.layout.bar_height).hash(&mut hasher);
            }
            curr = next;
        }
        let new_order_hash = hasher.finish();
        let reorder = self.rendering_requested.order_hash != new_order_hash;
        self.rendering_requested.order_hash = new_order_hash;
        curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            let window = (*node).window();
            (*window).render_finish();
            if reorder {
                {
                    // Viewport-hidden windows are NOT parked under the
                    // disabled hidden_tree: they keep their normal layer
                    // parent and stacking slot, hidden purely by their
                    // disabled node (render_finish and
                    // render_viewport_update both own that flag).
                    // Un-hiding happens on camera-motion frames, which
                    // never run this reorder pass — a window parked here
                    // stayed invisible after scrolling into view until
                    // the next unrelated transaction reparented it (the
                    // off-screen reveal delay in overview/zoom).
                    let layer = if (*window).get_app_id_string().as_deref() == Some("cce-wallpaper") {
                        // Between the native backdrop and the fallback
                        // cells, like a layer-shell Background surface.
                        (*self.server).scene.layers.background_clients
                    } else if (*window).is_grid() {
                        // The grid client is a desktop fixture: above
                        // the native backdrop and fallback cells
                        // (layers.background) but under every window.
                        // Left to the generic wm arm it stacks by
                        // render-list order, burying whichever windows
                        // happened to map before it.
                        (*self.server).scene.layers.bottom
                    } else if fullscreen_on_top(window) {
                        (*self.server).scene.layers.fullscreen
                    } else if rendered_fullscreen(window) {
                        // Stepped aside for a focused window (alt-tab
                        // out), or in overview: behind every window
                        // but above the grid, which the pass below
                        // re-asserts. It
                        // sits on the desk there
                        // (`place_fullscreen_windows`), like the grid
                        // beneath it.
                        (*self.server).scene.layers.bottom
                    } else if (*window).tiling_mode == crate::tiling::TilingMode::Popup {
                        (*self.server).scene.layers.popups
                    } else if (*window).tiling_mode == crate::tiling::TilingMode::Status {
                        // An EXPANDED segment (in-surface menu open;
                        // thicker than the bar) stacks like a popup:
                        // this loop re-raises every window in
                        // render-list order each pass, so leaving it
                        // in the shared Status layer let whichever
                        // sibling rendered last cover the menu's
                        // strip band (the strip-band click routing
                        // bug — an arrange-time raise was clobbered
                        // here every frame).
                        let bg = (*window).box_geom;
                        let thickness = match (*window).status_edge {
                            crate::policy::arrange::StatusEdge::Left
                            | crate::policy::arrange::StatusEdge::Right => bg.width,
                            _ => bg.height,
                        };
                        if thickness > self.layout.bar_height {
                            (*self.server).scene.layers.popups
                        } else {
                            (*self.server).scene.layers.top
                        }
                    } else if (*window).rendering_requested.circular {
                        (*self.server).scene.layers.top
                    } else if (*window).tiling_mode == crate::tiling::TilingMode::Overlay && self.layout.overlay_behavior == "above" {
                        (*self.server).scene.layers.top
                    } else {
                        (*self.server).scene.layers.wm
                    };

                    ffi::wlr_scene_node_reparent((*window).tree as *mut _, layer);
                    if (*window).get_app_id_string().as_deref() == Some("cce-wallpaper") {
                        ffi::wlr_scene_node_lower_to_bottom((*window).tree as *mut _);
                    } else {
                        ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                    }
                    ffi::wlr_scene_node_reparent((*window).popup_tree as *mut _, layer);
                    ffi::wlr_scene_node_place_above((*window).popup_tree as *mut _, (*window).tree as *mut _);
                }
            }
            curr = next;
        }

        // Floating windows are a plane IN FRONT of the tiled ones: a tiled
        // window never covers a floating one, however recently it was raised.
        // The loop above stacks layers.wm in render-list order alone, so
        // clicking a tiled window buried every floating window it overlaps —
        // and the two modes are meant to be independent, not interleaved.
        //
        // Re-applied on every reorder pass, walking the render list again so
        // each plane keeps its OWN relative stacking: the floating windows
        // come out in the order they were raised, above the tiled ones in the
        // order they were raised. A rule in the stacking authority, like the
        // light_source raise below — `raise_window` cannot own it, because
        // every other path that reorders the list would then have to know it.
        //
        // Only the windows the loop actually parked in layers.wm take part,
        // tested through the parent it just set: a fullscreen, popup, status
        // or circular window lives in a layer of its own, where raising it
        // would reshuffle that layer's members for no reason. `Utility` is
        // floating furniture too (a client-declared tool window — it floats
        // and moves like any other), so it rides in the same plane; `Overlay`
        // keeps its own `overlay_behavior` rule and stays out of this.
        if reorder {
            let wm_layer = (*self.server).scene.layers.wm;
            let mut focused_popups: *mut Window = std::ptr::null_mut();
            curr = (*render_list).next;
            while curr != render_list {
                let next = (*curr).next;
                let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
                let window = (*node).window();
                let floats = matches!(
                    (*window).tiling_mode,
                    crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Utility
                );
                let in_wm_layer = !(*window).tree.is_null()
                    && !wm_layer.is_null()
                    && ffi::river_scene_node_get_parent((*window).tree as *mut _) == wm_layer;
                if floats && in_wm_layer {
                    ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                    ffi::wlr_scene_node_place_above(
                        (*window).popup_tree as *mut _,
                        (*window).tree as *mut _,
                    );
                }
                // Last, so an open menu clears the plane it was just
                // stacked behind (`raise_focused_popups`).
                focused_popups = if (*window).is_seat_focused() { window } else { focused_popups };
                curr = next;
            }
            if !focused_popups.is_null() {
                self.raise_focused_popups(focused_popups);
            }
        }

        // A fullscreen window that stepped aside shares layers.bottom with the
        // grid, which the loop raises in render-list order; keep the window
        // over it — the grid is the desk, and the window sits on the desk.
        if reorder {
            for &w in self.windows.iter() {
                if !w.is_null()
                    && !(*w).closed
                    && matches!((*w).state, crate::window::WindowState::Mapped)
                    && rendered_fullscreen(w)
                    && !fullscreen_on_top(w)
                {
                    ffi::wlr_scene_node_raise_to_top((*w).tree as *mut ffi::wlr_scene_node);
                    ffi::wlr_scene_node_place_above((*w).popup_tree as *mut _, (*w).tree as *mut _);
                }
            }
        }

        // The traveling light_source segment crosses over its sibling
        // segments; raise it after the loop so it stacks in front of them
        // within its layer regardless of render-list order. Re-applied on
        // every reorder pass — a rule in the stacking authority, not a
        // one-shot raise.
        if reorder {
            for &w in self.windows.iter() {
                if !w.is_null()
                    && !(*w).closed
                    && matches!((*w).state, crate::window::WindowState::Mapped)
                    && (*w).get_app_id_string().map_or(false, |id| id.ends_with("light_source"))
                {
                    ffi::wlr_scene_node_raise_to_top((*w).tree as *mut ffi::wlr_scene_node);
                    ffi::wlr_scene_node_place_above((*w).popup_tree as *mut _, (*w).tree as *mut _);
                }
            }
        }

        (*self.server).om.commit_output_state(self.server);

        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            (*seat).cursor.update_hovered();
            curr = next;
        }

        (*self.server).idle_inhibit_manager.check_active();

        log::debug!("finished committing transaction");
        self.debug_check_unlinked_status("render_finish end");

        if self.scheduled.dirty || self.scheduled.dirty_lazy || self.rendering_scheduled.dirty {
            self.add_dirty_idle();
        }
        self.schedule_save_state();
        if let Some(r) = rf0 {
            log::info!("[manage] render_finish total={}us", r.elapsed().as_micros());
        }
    }
}

/// The idle callback a dirty mark arms: runs the pass that was waiting.
unsafe extern "C" fn dirty_idle_callback(data: *mut std::ffi::c_void) {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    (*wm).dirty_idle = std::ptr::null_mut();
    
    if matches!((*wm).state, WindowManagerState::Idle) {
        if (*wm).scheduled.dirty || (*wm).scheduled.dirty_lazy {
            (*wm).scheduled.dirty = true;
            (*wm).scheduled.dirty_lazy = false;
            (*wm).manage_start();
        } else if (*wm).rendering_scheduled.dirty {
            (*wm).render_start();
        }
    }
}
