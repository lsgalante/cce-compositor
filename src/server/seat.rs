use crate::ffi;
use crate::server::{Server, WlList, WlListener, wl_listener_remove, wl_signal_add};
use crate::cursor::Cursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatOpInput {
    Pointer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerOpType {
    Move,
    Resize { edges: crate::window::Edges },
}

#[derive(Clone, Copy, Debug)]
pub struct SeatOp {
    pub sent_release: bool,
    pub input: SeatOpInput,
    pub start_x: i32,
    pub start_y: i32,
    pub x: i32,
    pub y: i32,
    pub window_ptr: *mut crate::window::Window,
    pub op_type: PointerOpType,
    pub start_win_x: i32,
    pub start_win_y: i32,
    pub start_win_w: u32,
    pub start_win_h: u32,
    pub start_win_virtual_x: f64,
    pub start_win_virtual_y: f64,
    /// Desk pan at op start. The op tracks the cursor in virtual space as
    /// `start + cursor_delta/zoom + (pan - start_pan)`: the pan-delta term
    /// keeps the dragged window/edge pinned to the cursor when edge auto-pan
    /// (or anything else) scrolls the desktop mid-drag.
    pub start_pan_x: f64,
    pub start_pan_y: f64,
    pub start_tiling_mode: crate::tiling::TilingMode,
    /// Was the window Tiled when the drag was GRABBED? Every op site un-tiles
    /// a tiled window before building this struct (the drag needs it floating
    /// to follow the pointer), so `start_tiling_mode` already reads Floating
    /// and cannot answer this. A tiled window's move snaps hard to whole
    /// squares, so the motion handler has to know.
    pub start_was_tiled: bool,
    pub start_mode_locked: bool,
    pub started_in_overview: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Focus {
    None,
    LayerSurface(*mut ffi::wlr_surface),
    Window(*mut crate::window::Window),
    LockSurface(*mut crate::lock_manager::LockSurface),
    OverrideRedirect(*mut crate::xwayland_override_redirect::XwaylandOverrideRedirect),
    ShellSurface(*mut crate::shell_surface::ShellSurface),
}

impl Focus {
    pub unsafe fn surface(&self) -> *mut ffi::wlr_surface {
        match *self {
            Focus::None => std::ptr::null_mut(),
            Focus::LayerSurface(surface) => surface,
            Focus::Window(window) => if window.is_null() { std::ptr::null_mut() } else { (*window).root_surface() },
            Focus::LockSurface(lock_surface) => if lock_surface.is_null() { std::ptr::null_mut() } else { (*(*lock_surface).wlr_lock_surface).surface },
            Focus::OverrideRedirect(or) => if or.is_null() { std::ptr::null_mut() } else { (*(*or).xsurface).surface },
            Focus::ShellSurface(shell_surface) => if shell_surface.is_null() { std::ptr::null_mut() } else { (*shell_surface).surface },
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragState {
    None,
    Pointer,
    Touch,
}

pub struct Seat {
    pub server: *mut Server,
    pub wlr_seat: *mut ffi::wlr_seat,
    pub cursor: Cursor,
    pub focused: Focus,
    pub relay: crate::input_relay::InputRelay,
    pub layer_shell: crate::layer_shell::LayerShellSeat,
    pub xkb_bindings_seat: crate::xkb_bindings::XkbBindingsSeat,
    pub xkb_bindings: ffi::wl_list,
    pub pointer_bindings: ffi::wl_list,
    pub keyboard_groups: ffi::wl_list,
    /// A device-less keyboard group, created on demand by
    /// [`Seat::ensure_synthetic_keyboard`] when the backend supplies no
    /// keyboard at all. Null on any seat that has a real one.
    pub synthetic_keyboard: *mut crate::keyboard_group::KeyboardGroup,
    pub modifiers_old: u32,
    pub op: Option<SeatOp>,
    pub op_release: bool,
    /// One-shot focus-follow-pan suppression, set around refocuses caused
    /// by a window GOING AWAY (close/unmap/minimize): the camera stays
    /// where the user left it instead of chasing the fallback focus.
    /// Explicit focus changes (clicks, directional focus, the switcher)
    /// pan as always. Set-call-clear by the caller so a blocked focus
    /// can't leak the flag into a later, legitimate pan.
    pub suppress_focus_pan: bool,
    /// Overview-move displacement ledger: windows currently displaced out
    /// of the way of the active move op, with their pre-displacement
    /// virtual positions. While the button is held displacement is
    /// PROVISIONAL — every motion re-evaluates each entry at the spot it
    /// was pushed FROM, so a drag that moves away releases the window back
    /// home. Cleared (finalizing the positions) in `op_end`. Lives on the
    /// Seat because `SeatOp` is `Copy`.
    pub overview_displaced: Vec<(*mut crate::window::Window, f64, f64)>,
    pub wm_sent_x: i32,
    pub wm_sent_y: i32,

    pub request_set_cursor: ffi::wl_listener,
    /// The Xwayland cursor surface currently being shown at 1/`x11_cursor_scale`,
    /// null when the pointer image is anyone else's. See
    /// `handle_x11_cursor_commit` for why an X11 cursor needs shrinking at all.
    pub x11_cursor_surface: *mut ffi::wlr_surface,
    pub x11_cursor_scale: f32,
    pub x11_cursor_commit: ffi::wl_listener,
    pub x11_cursor_destroy: ffi::wl_listener,
    pub request_set_selection: ffi::wl_listener,
    pub request_start_drag: ffi::wl_listener,
    pub start_drag: ffi::wl_listener,
    pub request_set_primary_selection: ffi::wl_listener,

    pub drag: DragState,
    pub drag_destroy: ffi::wl_listener,

    pub link: ffi::wl_list,
    pub link_sent: ffi::wl_list,
    pub object: *mut ffi::wl_resource,
    pub destroying: bool,
    pub focus_requested: bool,
}

impl Seat {
    pub unsafe fn create(server: *mut Server, name: &str) -> Result<*mut Self, &'static str> {
        let name_c = std::ffi::CString::new(name).unwrap();
        let wlr_seat = ffi::wlr_seat_create((*server).wl_server, name_c.as_ptr());
        if wlr_seat.is_null() {
            return Err("Failed to create wlr_seat");
        }

        let seat = Box::into_raw(Box::new(Self {
            server,
            wlr_seat,
            cursor: Cursor::default(),
            focused: Focus::None,
            relay: std::mem::zeroed(),
            layer_shell: crate::layer_shell::LayerShellSeat::default(),
            xkb_bindings_seat: crate::xkb_bindings::XkbBindingsSeat::default(),
            xkb_bindings: std::mem::zeroed(),
            pointer_bindings: std::mem::zeroed(),
            keyboard_groups: std::mem::zeroed(),
            synthetic_keyboard: std::ptr::null_mut(),
            modifiers_old: 0,
            op: None,
            op_release: false,
            suppress_focus_pan: false,
            overview_displaced: Vec::new(),
            wm_sent_x: 0,
            wm_sent_y: 0,
            request_set_cursor: std::mem::zeroed(),
            x11_cursor_surface: std::ptr::null_mut(),
            x11_cursor_scale: 1.0,
            x11_cursor_commit: std::mem::zeroed(),
            x11_cursor_destroy: std::mem::zeroed(),
            request_set_selection: std::mem::zeroed(),
            request_start_drag: std::mem::zeroed(),
            start_drag: std::mem::zeroed(),
            request_set_primary_selection: std::mem::zeroed(),
            drag: DragState::None,
            drag_destroy: std::mem::zeroed(),
            link: std::mem::zeroed(),
            link_sent: std::mem::zeroed(),
            object: std::ptr::null_mut(),
            destroying: false,
            focus_requested: false,
        }));

        ffi::wl_list_init(&mut (*seat).link);
        ffi::wl_list_init(&mut (*seat).link_sent);
        ffi::wl_list_init(&mut (*seat).xkb_bindings);
        ffi::wl_list_init(&mut (*seat).pointer_bindings);
        ffi::wl_list_init(&mut (*seat).keyboard_groups);

        // Add to input_manager seats
        let seats_list = &mut (*server).input_manager.seats as *mut ffi::wl_list as *mut crate::server::WlList;
        crate::server::wl_list_insert((*seats_list).prev, &mut (*seat).link as *mut ffi::wl_list as *mut crate::server::WlList);

        ffi::river_wlr_seat_set_data(wlr_seat, seat as *mut _);
        (*seat).relay.init(seat);

        // Initialize Cursor
        let output_layout = (*server).om.output_layout;
        (*seat).cursor.init(seat, output_layout)?;

        // Setup listeners
        let set_cursor_ptr = &mut (*seat).request_set_cursor as *mut ffi::wl_listener as *mut WlListener;
        (*set_cursor_ptr).notify = Some(handle_request_set_cursor);
        wl_signal_add(
            ffi::river_wlr_seat_get_request_set_cursor_signal(wlr_seat),
            &mut (*seat).request_set_cursor,
        );

        let set_sel_ptr = &mut (*seat).request_set_selection as *mut ffi::wl_listener as *mut WlListener;
        (*set_sel_ptr).notify = Some(handle_request_set_selection);
        wl_signal_add(
            ffi::river_wlr_seat_get_request_set_selection_signal(wlr_seat),
            &mut (*seat).request_set_selection,
        );

        let start_drag_req_ptr = &mut (*seat).request_start_drag as *mut ffi::wl_listener as *mut WlListener;
        (*start_drag_req_ptr).notify = Some(handle_request_start_drag);
        wl_signal_add(
            ffi::river_wlr_seat_get_request_start_drag_signal(wlr_seat),
            &mut (*seat).request_start_drag,
        );

        let start_drag_ptr = &mut (*seat).start_drag as *mut ffi::wl_listener as *mut WlListener;
        (*start_drag_ptr).notify = Some(handle_start_drag);
        wl_signal_add(
            ffi::river_wlr_seat_get_start_drag_signal(wlr_seat),
            &mut (*seat).start_drag,
        );

        let set_prim_ptr = &mut (*seat).request_set_primary_selection as *mut ffi::wl_listener as *mut WlListener;
        (*set_prim_ptr).notify = Some(handle_request_set_primary_selection);
        wl_signal_add(
            ffi::river_wlr_seat_get_request_set_primary_selection_signal(wlr_seat),
            &mut (*seat).request_set_primary_selection,
        );

        (*seat).update_capabilities();

        Ok(seat)
    }

    pub unsafe fn destroy(seat: *mut Self) {
        (*seat).layer_shell.make_inert();
        (*seat).xkb_bindings_seat.make_inert();

        let bindings_head = &mut (*seat).xkb_bindings as *mut ffi::wl_list as *mut crate::server::WlList;
        let mut curr = (*bindings_head).next;
        while curr != bindings_head {
            let next = (*curr).next;
            let binding = crate::container_of!(curr, crate::xkb_bindings::XkbBinding, link);
            crate::xkb_bindings::XkbBinding::destroy(binding);
            curr = next;
        }

        let ptr_bindings_head = &mut (*seat).pointer_bindings as *mut ffi::wl_list as *mut crate::server::WlList;
        let mut curr_ptr = (*ptr_bindings_head).next;
        while curr_ptr != ptr_bindings_head {
            let next = (*curr_ptr).next;
            let binding = crate::container_of!(curr_ptr, crate::pointer_binding::PointerBinding, link);
            crate::pointer_binding::PointerBinding::destroy(binding);
            curr_ptr = next;
        }

        (*seat).cursor.deinit();

        // The synthetic keyboard has no device to outlive, so nothing else
        // will ever drop its reference — and the assert below requires the
        // group list to be empty by now.
        if !(*seat).synthetic_keyboard.is_null() {
            let group = (*seat).synthetic_keyboard;
            (*seat).synthetic_keyboard = std::ptr::null_mut();
            (*group).unref(&[]);
        }

        // Verify keyboard_groups is empty
        let groups_head = &mut (*seat).keyboard_groups as *mut ffi::wl_list as *mut crate::server::WlList;
        assert_eq!((*groups_head).next, groups_head);

        crate::server::wl_list_remove(&mut (*seat).link as *mut ffi::wl_list as *mut crate::server::WlList);
        crate::server::wl_list_remove(&mut (*seat).link_sent as *mut ffi::wl_list as *mut crate::server::WlList);

        (*seat).unwatch_x11_cursor();
        wl_listener_remove(&mut (*seat).request_set_cursor);
        wl_listener_remove(&mut (*seat).request_set_selection);
        wl_listener_remove(&mut (*seat).request_start_drag);
        wl_listener_remove(&mut (*seat).start_drag);
        wl_listener_remove(&mut (*seat).request_set_primary_selection);

        if (*seat).drag != DragState::None {
            wl_listener_remove(&mut (*seat).drag_destroy);
        }

        ffi::wlr_seat_destroy((*seat).wlr_seat);
        let _boxed = Box::from_raw(seat);
    }

    /// Start (or stop) shrinking an X11 client's cursor surface to 1/`scale`.
    ///
    /// `scale` of 1 — a Wayland client's cursor, or an X11 window exempted
    /// from `xwayland_hidpi` — just drops any surface being watched. The
    /// commit listener is added BEFORE the caller hands the surface to
    /// `wlr_cursor_set_surface`, so it sits ahead of wlroots' own commit
    /// listener in the signal and the size is already right when wlroots
    /// reads it.
    unsafe fn watch_x11_cursor(&mut self, surface: *mut ffi::wlr_surface, scale: f32) {
        if scale == 1.0 || surface.is_null() {
            self.unwatch_x11_cursor();
            return;
        }
        if self.x11_cursor_surface == surface {
            self.x11_cursor_scale = scale;
            return;
        }
        self.unwatch_x11_cursor();
        self.x11_cursor_surface = surface;
        self.x11_cursor_scale = scale;

        let commit = &mut self.x11_cursor_commit as *mut ffi::wl_listener as *mut WlListener;
        (*commit).notify = Some(handle_x11_cursor_commit);
        wl_signal_add(
            ffi::river_wlr_surface_get_commit_signal(surface),
            &mut self.x11_cursor_commit,
        );

        let destroy = &mut self.x11_cursor_destroy as *mut ffi::wl_listener as *mut WlListener;
        (*destroy).notify = Some(handle_x11_cursor_destroy);
        wl_signal_add(
            ffi::river_wlr_surface_get_destroy_signal(surface),
            &mut self.x11_cursor_destroy,
        );

        // Whatever is already committed on the surface is what wlroots reads
        // first; the fresh buffer only arrives on the commit that follows.
        ffi::river_wlr_surface_scale_logical_size(surface, scale);
    }

    unsafe fn unwatch_x11_cursor(&mut self) {
        if self.x11_cursor_surface.is_null() {
            return;
        }
        self.x11_cursor_surface = std::ptr::null_mut();
        self.x11_cursor_scale = 1.0;
        wl_listener_remove(&mut self.x11_cursor_commit);
        wl_listener_remove(&mut self.x11_cursor_destroy);
    }

    pub unsafe fn attach_device(&mut self, device: *mut crate::input_device::InputDevice) {
        (*device).seat = self;
        let dev_type = ffi::river_wlr_input_device_get_type((*device).wlr_device);
        match dev_type {
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD => {
                let keyboard = (*device).destroy_data as *mut crate::keyboard::Keyboard;
                if !keyboard.is_null() {
                    (*keyboard).set_group();
                    if !(*keyboard).group.is_null() {
                        ffi::wlr_seat_set_keyboard(self.wlr_seat, &mut (*(*keyboard).group).wlr_keyboard);
                        let focused_surface = ffi::river_wlr_seat_get_keyboard_focused_surface(self.wlr_seat);
                        if !focused_surface.is_null() {
                            self.keyboard_notify_enter(focused_surface);
                        }
                    }
                }
            }
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_POINTER => {
                ffi::wlr_cursor_attach_input_device(self.cursor.wlr_cursor, (*device).wlr_device);
            }
            ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TOUCH | ffi::wlr_input_device_type_WLR_INPUT_DEVICE_TABLET => {
                ffi::wlr_cursor_attach_input_device(self.cursor.wlr_cursor, (*device).wlr_device);
                if !(*device).config.map_to_output.is_null() {
                    ffi::wlr_cursor_map_input_to_output(self.cursor.wlr_cursor, (*device).wlr_device, (*device).config.map_to_output);
                }
                ffi::wlr_cursor_map_input_to_region(self.cursor.wlr_cursor, (*device).wlr_device, &mut (*device).config.map_to_rectangle);
            }
            _ => {}
        }
        self.update_capabilities();
    }

    pub unsafe fn detach_device(&mut self, device: *mut crate::input_device::InputDevice) {
        ffi::wlr_cursor_detach_input_device(self.cursor.wlr_cursor, (*device).wlr_device);

        let dev_type = ffi::river_wlr_input_device_get_type((*device).wlr_device);
        if dev_type == ffi::wlr_input_device_type_WLR_INPUT_DEVICE_KEYBOARD {
            let keyboard = (*device).destroy_data as *mut crate::keyboard::Keyboard;
            if !keyboard.is_null() {
                if !(*keyboard).group.is_null() {
                    let keys: Vec<u32> = (*keyboard).pressed.iter().cloned().collect();
                    crate::server::wl_list_remove(&mut (*keyboard).group_link as *mut ffi::wl_list as *mut crate::server::WlList);
                    (*(*keyboard).group).unref(&keys);
                    (*keyboard).group = std::ptr::null_mut();
                }
            }
        }
        self.update_capabilities();
    }

    pub unsafe fn handle_activity(&mut self) {
        (*self.server).idle.on_activity();
        let notifier = (*self.server).input_manager.idle_notifier;
        if !notifier.is_null() {
            ffi::wlr_idle_notifier_v1_notify_activity(notifier, self.wlr_seat);
        }
    }

    pub unsafe fn update_capabilities(&mut self) {
        let caps = ffi::wl_seat_capability_WL_SEAT_CAPABILITY_POINTER
            | ffi::wl_seat_capability_WL_SEAT_CAPABILITY_KEYBOARD;
        ffi::wlr_seat_set_capabilities(self.wlr_seat, caps);
    }

    pub unsafe fn focus(&mut self, new_focus: Focus) {
        if let Focus::Window(window) = new_focus {
            // The grid is the canvas, not a window: it must never take focus.
            // It became clickable when it started advertising an input region
            // for its desktop items, and the click path focuses whatever it
            // hits — which handed focus to a surface the size of the whole
            // patch and then let focus-follow pan the camera to "reveal" it,
            // so every press on an item dragged the desktop out from under
            // the pointer.
            if !window.is_null()
                && ((*window).is_status_bar() || (*window).is_wallpaper() || (*window).is_grid())
            {
                log::info!("[FocusDebug] Seat::focus blocking focus to status bar/wallpaper/grid window");
                return;
            }
        }

        if let Focus::Window(window) = new_focus {
            if !window.is_null() && (*window).tiling_mode == crate::tiling::TilingMode::Floating {
                (*self.server).wm.raise_window(window);
                (*self.server).wm.dirty_windowing();
            }
        }

        if self.focused == new_focus {
            // Re-focusing the already-focused window is still intent: a
            // click on the sliver of a mostly-hidden focused window (or on
            // a clipped one) should bring it over — its popups/menus open
            // relative to the window and land off-viewport otherwise. The
            // pan no-ops once the window is fully visible.
            if let Focus::Window(window) = new_focus {
                self.focus_follow_pan(window);
            }
            return;
        }

        if log::log_enabled!(log::Level::Debug) {
            let bt = std::backtrace::Backtrace::capture();
            log::debug!("[FocusDebug] Seat::focus changing from {:?} to {:?}. Backtrace:\n{}", self.focused, new_focus, bt);
        }

        // If an exclusive layer surface is active and scheduled for focus,
        // block any window manager or other client focus requests (via focus_requested)
        // from stealing focus back to a regular window or shell surface.
        if self.focus_requested {
            if let crate::layer_shell::LayerShellSeatFocus::Exclusive(key) = self.layer_shell.scheduled_focus {
                let server = self.server;
                if let Some(&layer_surface) = (*server).layer_shell.surfaces.get(key) {
                    let wlr_surf = (*(*layer_surface).wlr_layer_surface).surface;
                    if new_focus != Focus::LayerSurface(wlr_surf) {
                        if let Focus::Window(_) | Focus::ShellSurface(_) | Focus::OverrideRedirect(_) | Focus::None = new_focus {
                            log::info!("[FocusDebug] Blocking window manager focus request because Exclusive layer surface {:?} is active", key);
                            return;
                        }
                    }
                }
            }
        }

        match new_focus {
            Focus::None => log::info!("[FocusDebug] Seat::focus set to None"),
            Focus::LayerSurface(surface) => log::info!("[FocusDebug] Seat::focus set to LayerSurface {:?}", surface),
            Focus::Window(window) => {
                let title = if window.is_null() { "null".to_string() } else { (*window).get_title_string().unwrap_or_default() };
                let app_id = if window.is_null() { "null".to_string() } else { (*window).get_app_id_string().unwrap_or_default() };
                log::info!("[FocusDebug] Seat::focus set to Window {:?} (title={:?}, app_id={:?})", window, title, app_id);
            }
            Focus::LockSurface(lock) => log::info!("[FocusDebug] Seat::focus set to LockSurface {:?}", lock),
            Focus::OverrideRedirect(or) => log::info!("[FocusDebug] Seat::focus set to OverrideRedirect {:?}", or),
            Focus::ShellSurface(ss) => log::info!("[FocusDebug] Seat::focus set to ShellSurface {:?}", ss),
        }

        match self.focused {
            Focus::None => {}
            Focus::LayerSurface(_) | Focus::Window(_) | Focus::LockSurface(_) | Focus::OverrideRedirect(_) | Focus::ShellSurface(_) => {
                ffi::wlr_seat_keyboard_notify_clear_focus(self.wlr_seat);
                let focused_client = ffi::river_wlr_seat_get_pointer_focused_client(self.wlr_seat);
                // Keep pointer focus through an active implicit grab (held
                // client-notified button): clicking a window changes focus,
                // and dropping pointer focus here orphans the grab until the
                // next in-surface motion re-enters — a press-then-leave drag
                // (cursor straight out of the window) lost its target.
                if !focused_client.is_null() && self.cursor.notified_pressed.is_empty() {
                    ffi::wlr_seat_pointer_notify_clear_focus(self.wlr_seat);
                }
            }
        }

        self.focused = new_focus;
        // The overview resize ring is drawn on the focused window only and
        // eases in and out through the border fade — a focus change has to
        // arm that timer or the old ring lingers and the new one waits for an
        // unrelated redraw (the same reason WindowManager::set_mode arms it).
        (*self.server).wm.arm_border_fade();
        if let Focus::Window(window) = new_focus {
            if !window.is_null() {
                (*self.server).wm.record_focus(window);
            }
        }
        (*self.server).wm.update_status();

        match new_focus {
            Focus::None => {}
            Focus::LayerSurface(surface) => {
                if !surface.is_null() {
                    let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
                    if !kbd.is_null() {
                        let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            modifiers,
                        );
                    } else {
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                        );
                    }
                }

                let lx = self.cursor.x();
                let ly = self.cursor.y();
                let server = self.server;
                if let Some(result) = (*server).scene.at(lx, ly) {
                    if result.surface == surface {
                        ffi::wlr_seat_pointer_notify_enter(self.wlr_seat, surface, result.sx, result.sy);
                    }
                }
            }
            Focus::Window(window) => {
                let is_new = if !window.is_null() {
                    let was_new = (*window).is_new;
                    (*window).is_new = false;
                    was_new
                } else {
                    false
                };

                // A window newly on screen pulls the viewport over to it only when
                // `window_manager.center_on_spawn` allows it; one coming back from the
                // saved session at startup never did. Reopening an app mid-session is a
                // spawn even though `restored` is set — it only borrowed its old geometry
                // from `last_window_states`. Focus moving between windows that were
                // already up still pans either way; the key is about spawning.
                if !window.is_null() {
                    let spawn_pan = !(*window).session_restored
                        && !(*window).hint_placed
                        && (*self.server).wm.center_on_spawn;
                    // A restored window maps unfocused and keeps is_new until
                    // its first focus — which, after a session restart, is
                    // the user's first CLICK on it. Suppressing that pan made
                    // every window seem to ignore focus-follow right after
                    // login. Once real input has been seen the settling phase
                    // is over: a first focus is user intent and pans like any
                    // other, except for placement-hinted spawns (pickers that
                    // open at their control and must not yank the camera).
                    let user_focus = (*self.server).wm.startup_input_seen && !(*window).hint_placed;
                    if !is_new || spawn_pan || user_focus {
                        self.focus_follow_pan(window);
                    }
                }


                // Focus root surface of window
                let surface = (*window).root_surface();
                if !surface.is_null() {
                    let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
                    if !kbd.is_null() {
                        let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            modifiers,
                        );
                    } else {
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                        );
                    }

                    let lx = self.cursor.x();
                    let ly = self.cursor.y();
                    let server = self.server;
                    if let Some(result) = (*server).scene.at(lx, ly) {
                        if result.surface == surface {
                            ffi::wlr_seat_pointer_notify_enter(self.wlr_seat, surface, result.sx, result.sy);
                        }
                    }
                }
            }
            Focus::LockSurface(lock_surface) => {
                let surface = (*(*lock_surface).wlr_lock_surface).surface;
                if !surface.is_null() {
                    let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
                    if !kbd.is_null() {
                        let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            modifiers,
                        );
                    } else {
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                        );
                    }

                    let lx = self.cursor.x();
                    let ly = self.cursor.y();
                    let server = self.server;
                    if let Some(result) = (*server).scene.at(lx, ly) {
                        if result.surface == surface {
                            ffi::wlr_seat_pointer_notify_enter(self.wlr_seat, surface, result.sx, result.sy);
                        }
                    }
                }
            }
            Focus::OverrideRedirect(or) => {
                let surface = (*(*or).xsurface).surface;
                if !surface.is_null() {
                    let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
                    if !kbd.is_null() {
                        let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            modifiers,
                        );
                    } else {
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                        );
                    }
                }
            }
            Focus::ShellSurface(shell_surface) => {
                let surface = (*shell_surface).surface;
                if !surface.is_null() {
                    let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
                    if !kbd.is_null() {
                        let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            modifiers,
                        );
                    } else {
                        ffi::wlr_seat_keyboard_notify_enter(
                            self.wlr_seat,
                            surface,
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                        );
                    }
                }
            }
        }
        let target_surface = new_focus.surface();
        self.relay.focus(target_surface);
    }

    /// Give this seat a keyboard if the backend never supplied one, so that
    /// synthetic keys have somewhere to land. Returns false only when no
    /// keymap is configured, leaving nothing worth attaching.
    ///
    /// The seat advertises `WL_SEAT_CAPABILITY_KEYBOARD` unconditionally (see
    /// `update_capabilities`), so a client always binds `wl_keyboard`. But the
    /// keymap reaches that client from `wlr_seat_set_keyboard`, which only ever
    /// ran from `attach_device` — i.e. only once a real keyboard device
    /// existed. On the headless backend there is no keyboard device, so no
    /// keymap was ever sent, and a client with no keymap cannot turn a keycode
    /// into a keysym: `wlr_seat_keyboard_notify_key` delivered events that were
    /// silently dropped. That is why injected keys did nothing in a shadow
    /// session while injected pointer events worked.
    ///
    /// A real keyboard always wins — this is a no-op the moment the seat has
    /// one, so a normal session never reaches the creation path.
    pub unsafe fn ensure_synthetic_keyboard(&mut self) -> bool {
        if !ffi::river_wlr_seat_get_keyboard(self.wlr_seat).is_null() {
            return true;
        }

        let keymap = (*self.server).xkb_config.default_keymap;
        if keymap.is_null() {
            log::warn!("[seat] no keymap configured; synthetic keys cannot be delivered");
            return false;
        }

        // `KeyboardGroup::create` takes its own keymap reference and does the
        // `wlr_keyboard_init`/`set_keymap` wiring, so this borrows the group
        // machinery whole rather than hand-rolling a bare wlr_keyboard — which
        // would also leave `river_wlr_keyboard_get_data` null and cost
        // `keyboard_notify_enter` its pressed-key tracking.
        let config = crate::keyboard::KeyboardConfig {
            keymap,
            repeat_rate: 40,
            repeat_delay: 400,
        };
        match crate::keyboard_group::KeyboardGroup::create(self, config, true) {
            Ok(group) => {
                self.synthetic_keyboard = group;
                // set_keyboard is what pushes the keymap out to every bound
                // client; the enter re-announces focus with it in place.
                ffi::wlr_seat_set_keyboard(self.wlr_seat, &mut (*group).wlr_keyboard);
                let focused = ffi::river_wlr_seat_get_keyboard_focused_surface(self.wlr_seat);
                if !focused.is_null() {
                    self.keyboard_notify_enter(focused);
                }
                log::info!("[seat] no keyboard device on this backend — created a synthetic one so injected keys reach clients");
                true
            }
            Err(err) => {
                log::error!("[seat] failed to create synthetic keyboard: {}", err);
                false
            }
        }
    }

    pub unsafe fn keyboard_notify_enter(&mut self, wlr_surface: *mut ffi::wlr_surface) {
        if wlr_surface.is_null() {
            return;
        }
        let kbd = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
        if !kbd.is_null() {
            let group_ptr = ffi::river_wlr_keyboard_get_data(kbd) as *mut crate::keyboard_group::KeyboardGroup;
            if !group_ptr.is_null() {
                // Raw evdev keycodes, NOT xkb ones: `wl_keyboard.enter`'s key
                // array is the same space as `wl_keyboard.key`, and the client
                // is the one that adds 8 to reach an xkb keycode. Adding it
                // here shifted every held key up by 8 on the way out — and an
                // X11 popup that opens under a held key is exactly when this
                // array is sent, so opening Houdini's TAB menu (evdev 15)
                // handed Xwayland keycode 31 and typed an `i` into it, with no
                // release to follow, so X autorepeated it.
                let mut buffer = [0u32; 32];
                let mut count = 0;
                for &keycode in (*group_ptr).pressed.keys() {
                    if count >= 32 {
                        break;
                    }
                    buffer[count] = keycode;
                    count += 1;
                }
                let modifiers = ffi::river_wlr_keyboard_get_modifiers(kbd);
                ffi::wlr_seat_keyboard_notify_enter(
                    self.wlr_seat,
                    wlr_surface,
                    buffer.as_mut_ptr(),
                    count,
                    modifiers,
                );
                return;
            }
        }
        ffi::wlr_seat_keyboard_notify_enter(
            self.wlr_seat,
            wlr_surface,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
        );
    }

    pub unsafe fn keyboard_enter_or_leave(&mut self, target_surface: *mut ffi::wlr_surface) {
        if !target_surface.is_null() {
            self.keyboard_notify_enter(target_surface);
        } else {
            ffi::wlr_seat_keyboard_notify_clear_focus(self.wlr_seat);
        }
        self.relay.focus(target_surface);
    }

    pub unsafe fn manage_start(&mut self) {
        if self.destroying {
            Self::destroy(self);
            return;
        }

        self.focus_requested = false;
        self.layer_shell.manage_start();

        let wm_v1 = (*self.server).wm.object;
        if !wm_v1.is_null() {
            let new = self.object.is_null();
            if new {
                let client = ffi::wl_resource_get_client(wm_v1);
                let version = ffi::wl_resource_get_version(wm_v1);
                let seat_v1 = ffi::wl_resource_create(client, &ffi::zcce_seat_v1_interface, version, 0);
                if seat_v1.is_null() {
                    log::error!("out of memory creating zcce_seat_v1");
                    return;
                }
                self.object = seat_v1;
                
                ffi::wl_resource_set_implementation(
                    seat_v1,
                    &SEAT_INTERFACE as *const _ as *const _,
                    self as *mut Seat as *mut _,
                    Some(handle_destroy_resource),
                );
                
                ffi::wl_resource_post_event(wm_v1, ffi::ZCCE_WINDOW_MANAGER_V1_SEAT, seat_v1); // zcce_window_manager_v1.seat

                crate::server::wl_list_remove(&mut self.link_sent as *mut ffi::wl_list as *mut crate::server::WlList);
                let sent_seats = &mut (*self.server).wm.sent.seats as *mut ffi::wl_list as *mut crate::server::WlList;
                crate::server::wl_list_insert((*sent_seats).prev, &mut self.link_sent as *mut ffi::wl_list as *mut crate::server::WlList);
            }

            if new {
                let seat_v1 = self.object;
                let client = ffi::wl_resource_get_client(seat_v1);
                let wl_seat_name = ffi::wl_global_get_name(ffi::river_wlr_seat_get_global(self.wlr_seat), client);
                ffi::wl_resource_post_event(seat_v1, 1, wl_seat_name); // river_seat_v1.wl_seat
            }

            self.xkb_bindings_seat.manage_start();

            // Dispatch xkb binding events
            let bindings_head = &mut self.xkb_bindings as *mut ffi::wl_list as *mut crate::server::WlList;
            let mut curr = (*bindings_head).next;
            while curr != bindings_head {
                let next = (*curr).next;
                let binding = crate::container_of!(curr, crate::xkb_bindings::XkbBinding, link);
                for state in (*binding).wm_scheduled.state_changes.drain(..) {
                    match state {
                        crate::xkb_bindings::XkbBindingStateChange::None => {},
                        crate::xkb_bindings::XkbBindingStateChange::Pressed => {
                            if !(*binding).sent_pressed {
                                (*binding).sent_pressed = true;
                                ffi::wl_resource_post_event((*binding).object, 0);
                            }
                        },
                        crate::xkb_bindings::XkbBindingStateChange::StopRepeat => {
                            if (*binding).sent_pressed {
                                if ffi::wl_resource_get_version((*binding).object) >= 2 {
                                    ffi::wl_resource_post_event((*binding).object, 2);
                                }
                            }
                        },
                        crate::xkb_bindings::XkbBindingStateChange::Released => {
                            if (*binding).sent_pressed {
                                (*binding).sent_pressed = false;
                                ffi::wl_resource_post_event((*binding).object, 1);
                            }
                        },
                    }
                }
                curr = next;
            }

            // Dispatch pointer binding events
            let ptr_bindings_head = &mut self.pointer_bindings as *mut ffi::wl_list as *mut crate::server::WlList;
            let mut curr_ptr = (*ptr_bindings_head).next;
            while curr_ptr != ptr_bindings_head {
                let next = (*curr_ptr).next;
                let binding = crate::container_of!(curr_ptr, crate::pointer_binding::PointerBinding, link);
                for state in (*binding).wm_scheduled.state_changes.drain(..) {
                    match state {
                        crate::pointer_binding::PointerBindingStateChange::None => {}
                        crate::pointer_binding::PointerBindingStateChange::Pressed => {
                            if !(*binding).sent_pressed {
                                (*binding).sent_pressed = true;
                                ffi::wl_resource_post_event((*binding).object, 0); // pressed
                            }
                        }
                        crate::pointer_binding::PointerBindingStateChange::Released => {
                            if (*binding).sent_pressed {
                                (*binding).sent_pressed = false;
                                ffi::wl_resource_post_event((*binding).object, 1); // released
                            }
                        }
                    }
                }
                curr_ptr = next;
            }

            // Dispatch pointer operation events
            if let Some(ref mut op) = self.op {
                let dx = op.x - op.start_x;
                let dy = op.y - op.start_y;
                ffi::wl_resource_post_event(self.object, 6, dx, dy); // op_delta

                if self.op_release && !op.sent_release {
                    ffi::wl_resource_post_event(self.object, 7); // op_release
                    self.op_release = false;
                    op.sent_release = true;
                }
            }

            // Dispatch pointer position event
            if ffi::wl_resource_get_version(self.object) >= 2 {
                let x = (*self.cursor.wlr_cursor).x as i32;
                let y = (*self.cursor.wlr_cursor).y as i32;
                if x != self.wm_sent_x || y != self.wm_sent_y {
                    ffi::wl_resource_post_event(self.object, 8, x, y); // pointer_position
                    self.wm_sent_x = x;
                    self.wm_sent_y = y;
                }
            }
        } else {
            crate::server::wl_list_remove(&mut self.link_sent as *mut ffi::wl_list as *mut crate::server::WlList);
            let sent_seats = &mut (*self.server).wm.sent.seats as *mut ffi::wl_list as *mut crate::server::WlList;
            crate::server::wl_list_insert((*sent_seats).prev, &mut self.link_sent as *mut ffi::wl_list as *mut crate::server::WlList);
        }
    }

    pub unsafe fn manage_finish(&mut self) {
        self.xkb_bindings_seat.manage_finish();

        if (*self.server).lock_manager.state != crate::lock_manager::LockState::Unlocked {
            return;
        }

        match self.layer_shell.sent_focus {
            crate::layer_shell::LayerShellSeatFocus::Exclusive(key) => {
                let server = self.server;
                if let Some(&layer_surface) = (*server).layer_shell.surfaces.get(key) {
                    let wlr_surf = (*(*layer_surface).wlr_layer_surface).surface;
                    self.focus(Focus::LayerSurface(wlr_surf));
                }
            }
            crate::layer_shell::LayerShellSeatFocus::NonExclusive(key) => {
                if !self.focus_requested {
                    let server = self.server;
                    if let Some(&layer_surface) = (*server).layer_shell.surfaces.get(key) {
                        let wlr_surf = (*(*layer_surface).wlr_layer_surface).surface;
                        self.focus(Focus::LayerSurface(wlr_surf));
                    }
                } else {
                    self.layer_shell.scheduled_focus = crate::layer_shell::LayerShellSeatFocus::None;
                    (*self.server).wm.dirty_windowing();
                }
            }
            crate::layer_shell::LayerShellSeatFocus::None => {}
        }
    }


    /// Is this seat's keyboard focus desktop chrome — a Popup/Overlay window
    /// (the cce-cloud launcher, a dock) or a cce-cloud layer surface (a
    /// context menu)? Chrome stays keyboard-interactive in overview and is
    /// dismissed by using it (Escape, a pick, a click-away), so the
    /// overview-mode key and hover paths consult this before treating the
    /// focus as a world window's: keys are delivered rather than eaten, and
    /// hover-to-focus leaves the ring where it is instead of pulling the
    /// keyboard out from under the launcher.
    pub unsafe fn focus_is_chrome(&self) -> bool {
        match self.focused {
            Focus::Window(w) if !w.is_null() => matches!(
                (*w).tiling_mode,
                crate::tiling::TilingMode::Popup | crate::tiling::TilingMode::Overlay
            ),
            Focus::LayerSurface(s) if !s.is_null() => {
                let wlr_layer_surface = ffi::wlr_layer_surface_v1_try_from_wlr_surface(s);
                !wlr_layer_surface.is_null()
                    && !(*wlr_layer_surface).namespace.is_null()
                    && std::ffi::CStr::from_ptr((*wlr_layer_surface).namespace)
                        .to_string_lossy()
                        .starts_with("cce-cloud")
            }
            _ => false,
        }
    }

    /// Focus-follow: pan the camera to a focused Floating/Maximized window —
    /// centering when it is mostly hidden, nudging a clipped edge into view
    /// otherwise. Fullscreen is pinned to an output and popups/overlays are
    /// not desk citizens, so other modes no-op, as do cce-cloud and windows
    /// already fully visible.
    pub unsafe fn focus_follow_pan(&mut self, window: *mut crate::window::Window) {
        if self.suppress_focus_pan {
            return;
        }
        // While a camera ramp owns the camera (an overview enter/exit
        // flight), the current camera is a mid-flight sample — any pan
        // target computed from it is stale by construction. Never retarget
        // out from under the ramp.
        if (*self.server).wm.camera_ramp_anim.is_some() {
            return;
        }
        if window.is_null()
            || !matches!(
                (*window).tiling_mode,
                // Utility included: it pans on the virtual surface like any
                // floating window, so focusing one off-view should bring it in.
                crate::tiling::TilingMode::Floating
                    | crate::tiling::TilingMode::Tiled
                    | crate::tiling::TilingMode::Utility
            )
        {
            return;
        }
        let app_id = (*window).get_app_id_string();
        if app_id.as_ref().map(|id| id == "cce-cloud").unwrap_or(false) {
            return;
        }
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        let mut target_output: *mut crate::output::Output = std::ptr::null_mut();
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                if target_output.is_null() {
                    target_output = output;
                }
                let wlr_box = (*output).sent.box_layout();
                let wx = (*window).box_geom.x;
                let wy = (*window).box_geom.y;
                if wx >= wlr_box.x && wx < wlr_box.x + wlr_box.width
                    && wy >= wlr_box.y && wy < wlr_box.y + wlr_box.height
                {
                    target_output = output;
                    break;
                }
            }
            curr_out = (*curr_out).next;
        }

        if !target_output.is_null() {
            let wlr_box = (*target_output).sent.box_layout();
            let viewport_w = wlr_box.width as f64;
            let viewport_h = wlr_box.height as f64;

            let fw = if (*window).box_geom.width > 0 {
                (*window).box_geom.width as f64
            } else if (*window).wm_scheduled.dimensions_hint.min_width > 32 {
                (*window).wm_scheduled.dimensions_hint.min_width as f64
            } else {
                800.0
            };
            let fh = if (*window).box_geom.height > 0 {
                (*window).box_geom.height as f64
            } else if (*window).wm_scheduled.dimensions_hint.min_height > 32 {
                (*window).wm_scheduled.dimensions_hint.min_height as f64
            } else {
                600.0
            };

            let wm = &mut (*self.server).wm;
            let cam = wm.camera();
            // box_geom is already virtual units (its screen footprint is
            // box_geom * zoom) — dividing by zoom here inflated the window
            // whenever zoom != 1 and mistargeted the pan.
            let vw_w = fw;
            let vw_h = fh;
            // The camera moves as little as the focus demands: enough to
            // show the whole window with a margin, and no further. A window
            // half off the edge and one a screen away take the same rule —
            // `policy::camera::pan_into_view` carries the reasoning, and
            // `WindowManager::pan_to_virtual_rect` applies it to the
            // non-window rects (restore placeholders) from the same place.
            if let Some(target) = crate::policy::camera::pan_into_view(
                (*window).virtual_x,
                (*window).virtual_y,
                vw_w,
                vw_h,
                cam,
                viewport_w,
                viewport_h,
            ) {
                wm.target_desk_pan_x = Some(target.pan_x);
                wm.target_desk_pan_y = Some(target.pan_y);
                wm.start_panning_animation();
            }
        }
    }

    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_post_event(self.object, 0); // river_seat_v1.removed
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_SEAT_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
            (*self.server).wm.dirty_windowing();
        }
        self.layer_shell.make_inert();
        self.xkb_bindings_seat.make_inert();
    }

    pub unsafe fn match_xkb_binding(
        &self,
        keycode: u32,
        wlr_keyboard: *mut ffi::wlr_keyboard,
    ) -> Option<*mut crate::xkb_bindings::XkbBinding> {
        let xkb_state = (*wlr_keyboard).xkb_state;
        if xkb_state.is_null() {
            return None;
        }
        
        let modifiers = ffi::wlr_keyboard_get_modifiers(wlr_keyboard);
        
        let bindings_head = &self.xkb_bindings as *const ffi::wl_list as *mut crate::server::WlList;
        let mut curr = (*bindings_head).next;
        let mut found: Option<*mut crate::xkb_bindings::XkbBinding> = None;
        
        while curr != bindings_head {
            let next = (*curr).next;
            let binding = crate::container_of!(curr, crate::xkb_bindings::XkbBinding, link);
            if (*binding).match_keycode(keycode, modifiers, xkb_state, false) {
                if found.is_none() {
                    found = Some(binding);
                } else {
                    log::debug!("already found a matching xkb_binding, ignoring additional match");
                }
            }
            curr = next;
        }
        
        if found.is_some() {
            return found;
        }
        
        curr = (*bindings_head).next;
        while curr != bindings_head {
            let next = (*curr).next;
            let binding = crate::container_of!(curr, crate::xkb_bindings::XkbBinding, link);
            if (*binding).match_keycode(keycode, modifiers, xkb_state, true) {
                if found.is_none() {
                    found = Some(binding);
                } else {
                    log::debug!("already found a matching xkb_binding, ignoring additional match");
                }
            }
            curr = next;
        }
        
        found
    }

    pub unsafe fn match_pointer_binding(
        &self,
        button: u32,
    ) -> Option<*mut crate::pointer_binding::PointerBinding> {
        let wlr_keyboard = ffi::river_wlr_seat_get_keyboard(self.wlr_seat);
        if wlr_keyboard.is_null() {
            return None;
        }
        let modifiers = ffi::wlr_keyboard_get_modifiers(wlr_keyboard);

        let bindings_head = &self.pointer_bindings as *const ffi::wl_list as *mut crate::server::WlList;
        let mut curr = (*bindings_head).next;
        let mut found: Option<*mut crate::pointer_binding::PointerBinding> = None;

        while curr != bindings_head {
            let next = (*curr).next;
            let binding = crate::container_of!(curr, crate::pointer_binding::PointerBinding, link);
            if (*binding).match_binding(button, modifiers) {
                if found.is_none() {
                    found = Some(binding);
                } else {
                    log::debug!("already found a matching pointer binding, ignoring additional match");
                }
            }
            curr = next;
        }
        found
    }

    /// Snap parameters for interactive ops, from the current layout config.
    unsafe fn snap_params(&self) -> crate::policy::snap::SnapParams {
        // Zoom-aware: the felt grab distance stays constant in screen px.
        (*self.server).wm.layout.snap_params().for_zoom((*self.server).wm.desk_zoom)
    }

    pub unsafe fn op_update(&mut self, x: i32, y: i32) {
        let sp = self.snap_params();
        if let Some(ref mut op) = self.op {
            op.x = x;
            op.y = y;
            let dx = op.x - op.start_x;
            let dy = op.y - op.start_y;
            
            let win = op.window_ptr;
            if !win.is_null() && !(*win).closed {
                // Every drag step can bring a Floating window over the
                // adjust target or take it off: re-evaluate the overlap dim.
                (*self.server).wm.arm_border_fade();
                if (*win).tiling_mode != crate::tiling::TilingMode::Floating
                    && (*win).tiling_mode != crate::tiling::TilingMode::Overlay
                    // A drag moves a Utility window; it must not re-class it.
                    && (*win).tiling_mode != crate::tiling::TilingMode::Utility
                {
                    // Un-tile for the drag but KEEP the geometry (clearing
                    // was_tiled suppresses the arrange Exit restore);
                    // landing grid-aligned re-tiles it in op_end.
                    (*win).was_tiled = false;
                    (*win).tiling_mode = crate::tiling::TilingMode::Floating;
                    (*win).mode_locked = true;
                    (*self.server).wm.raise_window(win);
                }
                
                match op.op_type {
                    PointerOpType::Move => {
                        #[allow(unused_assignments)]
                        if (*win).is_status_bar() {
                            let final_x = op.start_win_x + dx;
                            let final_y = op.start_win_y + dy;
                            (*win).rendering_requested.x = final_x;
                            (*win).rendering_requested.y = final_y;
                            (*win).box_geom.x = final_x;
                            (*win).box_geom.y = final_y;

                            // Dynamically update orientation during drag
                            let lx = x as f64;
                            let ly = y as f64;
                            let mut closest_edge = crate::window::StatusEdge::TopLeft;
                            let mut min_dist = f64::MAX;

                            let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                            let mut curr_out = (*outputs_list).next;
                            let mut best_output: *mut crate::output::Output = std::ptr::null_mut();
                            let mut min_output_dist = f64::MAX;
                            
                            while curr_out != outputs_list {
                                let output = crate::container_of!(curr_out, crate::output::Output, link);
                                if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                                    let wlr_box = (*output).sent.box_layout();
                                    let ox = wlr_box.x as f64;
                                    let oy = wlr_box.y as f64;
                                    let ow = wlr_box.width as f64;
                                    let oh = wlr_box.height as f64;
                                    
                                    let clamp = |val: f64, min: f64, max: f64| {
                                        if val < min { min } else if val > max { max } else { val }
                                    };
                                    let cx = clamp(lx, ox, ox + ow);
                                    let cy = clamp(ly, oy, oy + oh);
                                    let dx = lx - cx;
                                    let dy = ly - cy;
                                    let dist = dx * dx + dy * dy;
                                    if dist < min_output_dist {
                                        min_output_dist = dist;
                                        best_output = output;
                                    }
                                }
                                curr_out = (*curr_out).next;
                            }

                            let mut found_out = false;
                            if !best_output.is_null() {
                                found_out = true;
                                let wlr_box = (*best_output).sent.box_layout();
                                let ox = wlr_box.x as f64;
                                let oy = wlr_box.y as f64;
                                let ow = wlr_box.width as f64;
                                let oh = wlr_box.height as f64;

                                let dt = ly - oy;
                                let db = (oy + oh) - ly;
                                let dl = lx - ox;
                                let dr = (ox + ow) - lx;

                                enum EdgeBasic { Top, Bottom, Left, Right }
                                let mut edge = EdgeBasic::Top;
                                if dt < min_dist { min_dist = dt; edge = EdgeBasic::Top; }
                                if db < min_dist { min_dist = db; edge = EdgeBasic::Bottom; }
                                if dl < min_dist { min_dist = dl; edge = EdgeBasic::Left; }
                                if dr < min_dist { min_dist = dr; edge = EdgeBasic::Right; }

                                let corner_threshold = 120.0;
                                let is_near_top = ly < oy + corner_threshold;
                                let is_near_bottom = ly > oy + oh - corner_threshold;
                                let is_near_left = lx < ox + corner_threshold;
                                let is_near_right = lx > ox + ow - corner_threshold;

                                let semicircle_centers = [
                                    (crate::window::StatusEdge::TopLeft, ox + 60.0, oy + 0.0),
                                    (crate::window::StatusEdge::TopCenter, ox + ow / 2.0, oy + 0.0),
                                    (crate::window::StatusEdge::TopRight, ox + ow - 60.0, oy + 0.0),
                                    (crate::window::StatusEdge::BottomLeft, ox + 60.0, oy + oh),
                                    (crate::window::StatusEdge::BottomCenter, ox + ow / 2.0, oy + oh),
                                    (crate::window::StatusEdge::BottomRight, ox + ow - 60.0, oy + oh),
                                    (crate::window::StatusEdge::Left, ox + 0.0, oy + oh / 2.0),
                                    (crate::window::StatusEdge::Right, ox + ow, oy + oh / 2.0),
                                ];

                                let mut snapped_to_semicircle = false;
                                for (edge_type, cx, cy) in semicircle_centers {
                                    let dx = lx - cx;
                                    let dy = ly - cy;
                                    if dx * dx + dy * dy <= 60.0 * 60.0 {
                                        closest_edge = edge_type;
                                        snapped_to_semicircle = true;
                                        break;
                                    }
                                }

                                if !snapped_to_semicircle {
                                    match edge {
                                        EdgeBasic::Top => {
                                            if is_near_left {
                                                closest_edge = crate::window::StatusEdge::TopLeft;
                                            } else if is_near_right {
                                                closest_edge = crate::window::StatusEdge::TopRight;
                                            } else {
                                                closest_edge = crate::window::StatusEdge::TopCenter;
                                            }
                                        }
                                        EdgeBasic::Bottom => {
                                            if is_near_left {
                                                closest_edge = crate::window::StatusEdge::BottomLeft;
                                            } else if is_near_right {
                                                closest_edge = crate::window::StatusEdge::BottomRight;
                                            } else {
                                                closest_edge = crate::window::StatusEdge::BottomCenter;
                                            }
                                        }
                                        EdgeBasic::Left => {
                                            if is_near_top {
                                                closest_edge = crate::window::StatusEdge::TopLeft;
                                            } else if is_near_bottom {
                                                closest_edge = crate::window::StatusEdge::BottomLeft;
                                            } else {
                                                closest_edge = crate::window::StatusEdge::Left;
                                            }
                                        }
                                        EdgeBasic::Right => {
                                            if is_near_top {
                                                closest_edge = crate::window::StatusEdge::TopRight;
                                            } else if is_near_bottom {
                                                closest_edge = crate::window::StatusEdge::BottomRight;
                                            } else {
                                                closest_edge = crate::window::StatusEdge::Right;
                                            }
                                        }
                                    }
                                }
                            }

                            if found_out {
                                let bar_h = (*self.server).wm.layout.bar_height as u32;
                                let original_length = std::cmp::max((*win).box_geom.width, (*win).box_geom.height) as u32;
                                let (target_w, target_h) = match closest_edge {
                                    crate::window::StatusEdge::Left | crate::window::StatusEdge::Right => (bar_h, original_length),
                                    _ => (original_length, bar_h),
                                };

                                if (*win).box_geom.width as u32 != target_w || (*win).box_geom.height as u32 != target_h {
                                    (*win).wm_requested.dimensions = Some(crate::window::Dimensions { width: target_w, height: target_h });
                                    (*win).wm_requested.bounds = crate::window::Dimensions { width: target_w, height: target_h };
                                    (*self.server).wm.dirty_windowing();
                                }
                            }
                        } else {
                            let scale = (*(*self.server).wm.server).wm.desk_zoom;
                            let pan_x = (*(*self.server).wm.server).wm.desk_pan_x;
                            let pan_y = (*(*self.server).wm.server).wm.desk_pan_y;
                            let virtual_dx = dx as f64 / scale + (pan_x - op.start_pan_x);
                            let virtual_dy = dy as f64 / scale + (pan_y - op.start_pan_y);

                            let vx = op.start_win_virtual_x + virtual_dx;
                            let vy = op.start_win_virtual_y + virtual_dy;
                            // A Tiled window only ever occupies whole squares,
                            // so its drag snaps hard to the nearest one. The
                            // magnetic snap below is for Floating windows,
                            // which use it to decide whether they land aligned
                            // (and so become Tiled) at op_end.
                            //
                            // This asks what the window was when GRABBED, not
                            // what it is now: op_update un-tiles a tiled window
                            // on the first motion event so the drag can follow
                            // the pointer, so the live mode is always Floating
                            // here and a test against it never fires.
                            let (vx, vy) = if op.start_was_tiled {
                                crate::policy::snap::snap_move_tiled(vx, vy, &sp)
                            } else {
                                crate::policy::snap::snap_move(
                                    vx,
                                    vy,
                                    (*win).box_geom.width as f64,
                                    (*win).box_geom.height as f64,
                                    &sp,
                                )
                            };
                            (*win).virtual_x = vx;
                            (*win).virtual_y = vy;

                            // Overview moves displace what they cover: any
                            // window the drag covers past the threshold
                            // scoots to the side the drag vacated.
                            if (*self.server).wm.mode
                                == crate::window_manager::WindowManagerMode::Overview
                            {
                                displace_covered(
                                    self.server,
                                    win,
                                    op.start_was_tiled,
                                    (virtual_dx, virtual_dy),
                                    &sp,
                                    &mut self.overview_displaced,
                                );
                            }

                            let (final_x, final_y) = (*win).virtual_to_screen(vx, vy);
                            (*win).rendering_requested.x = final_x;
                            (*win).rendering_requested.y = final_y;
                            (*win).box_geom.x = final_x;
                            (*win).box_geom.y = final_y;
                        }
                    }
                    PointerOpType::Resize { edges } => {
                        let scale = (*(*self.server).wm.server).wm.desk_zoom;
                        let pan_x = (*(*self.server).wm.server).wm.desk_pan_x;
                        let pan_y = (*(*self.server).wm.server).wm.desk_pan_y;
                        let virtual_dx = dx as f64 / scale + (pan_x - op.start_pan_x);
                        let virtual_dy = dy as f64 / scale + (pan_y - op.start_pan_y);

                        let mut vx = op.start_win_virtual_x;
                        let mut vy = op.start_win_virtual_y;

                        if edges.left {
                            vx = (*win).virtual_x;
                        }
                        if edges.top {
                            vy = (*win).virtual_y;
                        }

                        if (*win).resize_edges != Some(edges) {
                            (*win).resize_start_vx = op.start_win_virtual_x;
                            (*win).resize_start_vy = op.start_win_virtual_y;
                            (*win).resize_start_w = op.start_win_w;
                            (*win).resize_start_h = op.start_win_h;
                            (*win).resize_edges = Some(edges);
                        }

                        // A window grabbed Tiled snaps HARD: the dragged edge
                        // lands on a cell edge from any distance and the size
                        // stays whole cells, so it is still Tiled on release.
                        // A Floating one gets the magnetic pull onto the
                        // visible cell edges; the anchored edge is untouched
                        // either way. Must match get_active_resize_dimensions,
                        // which recomputes this for the arrange snapshot.
                        let (new_w, new_h) = if op.start_was_tiled {
                            (
                                crate::policy::snap::resize_axis_tiled(
                                    op.start_win_virtual_x, op.start_win_w as f64, virtual_dx,
                                    edges.left, edges.right, &sp.x(),
                                ) as u32,
                                crate::policy::snap::resize_axis_tiled(
                                    op.start_win_virtual_y, op.start_win_h as f64, virtual_dy,
                                    edges.top, edges.bottom, &sp.y(),
                                ) as u32,
                            )
                        } else {
                            (
                                crate::policy::snap::resize_axis(
                                    op.start_win_virtual_x, op.start_win_w as f64, virtual_dx,
                                    edges.left, edges.right, 50.0, &sp.x(),
                                ) as u32,
                                crate::policy::snap::resize_axis(
                                    op.start_win_virtual_y, op.start_win_h as f64, virtual_dy,
                                    edges.top, edges.bottom, 50.0, &sp.y(),
                                ) as u32,
                            )
                        };
                        // The client's xdg min/max size is a contract, not a
                        // suggestion: a configure below it is applied by
                        // cce-ui as-is, and a layout with less room than its
                        // fixed parts panicked cce-data-editor mid-drag.
                        let (new_w, new_h) = (*win).wm_scheduled.dimensions_hint.clamp(new_w, new_h);

                        (*win).virtual_x = vx;
                        (*win).virtual_y = vy;

                        let (final_x, final_y) = (*win).virtual_to_screen(vx, vy);

                        (*win).rendering_requested.x = final_x;
                        (*win).rendering_requested.y = final_y;
                        (*win).box_geom.x = final_x;
                        (*win).box_geom.y = final_y;

                        (*win).wm_requested.resizing = true;
                        (*win).wm_requested.dimensions = Some(crate::window::Dimensions {
                            width: new_w,
                            height: new_h,
                        });
                        (*win).wm_requested.bounds = crate::window::Dimensions {
                            width: new_w,
                            height: new_h,
                        };
                        (*win).set_dimensions(new_w, new_h);
                    }
                }
            }
            // The configure and the relayout go out once per output frame,
            // for wherever the pointer is by then (WindowManager::
            // step_op_frame), not once per motion event.
            (*self.server).wm.queue_op_frame();
        }
        self.update_edge_pan(x as f64, y as f64);
    }

    /// Edge auto-pan eligibility + velocity for the current op: while an
    /// interactive move/resize holds the cursor inside the band at an output
    /// edge, the desktop scrolls that way, ramping from 0 at the band's inner
    /// rim to full speed at the screen edge. Called on every op motion AND
    /// from the edge-pan tick's op_update, which is what re-arms the timer —
    /// so the scroll continues while the cursor rests pinned at the edge.
    unsafe fn update_edge_pan(&mut self, lx: f64, ly: f64) {
        let wm = &mut (*self.server).wm;
        let mut vx = 0.0;
        let mut vy = 0.0;
        let eligible = wm.layout.desktop_edge_pan
            && match self.op {
                Some(ref op) => {
                    !op.window_ptr.is_null()
                        && !(*op.window_ptr).closed
                        && !(*op.window_ptr).is_status_bar()
                }
                None => false,
            };
        if eligible {
            let wlr_output = (*self.server).om.output_at(lx, ly);
            if !wlr_output.is_null() {
                let mut ob = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
                ffi::wlr_output_layout_get_box((*self.server).om.output_layout, wlr_output, &mut ob);
                let band = wm.layout.desktop_edge_pan_band.max(1.0);
                let speed = wm.layout.desktop_edge_pan_speed.max(0.0);
                // 0 outside the band, 1 at (or past) the screen edge.
                let ramp = |dist_to_edge: f64| ((band - dist_to_edge) / band).clamp(0.0, 1.0);
                vx = speed
                    * (ramp((ob.x + ob.width) as f64 - lx) - ramp(lx - ob.x as f64));
                vy = speed
                    * (ramp((ob.y + ob.height) as f64 - ly) - ramp(ly - ob.y as f64));
            }
        }
        wm.set_edge_pan_velocity(vx, vy);
    }

    pub unsafe fn op_end(&mut self) {
        // Wherever everything sits now is final.
        self.overview_displaced.clear();
        if let Some(op) = self.op.take() {
            log::debug!("end seat op");
            let wm = &mut (*self.server).wm;
            wm.edge_pan_vx = 0.0;
            wm.edge_pan_vy = 0.0;
            let win = op.window_ptr;
            if !win.is_null() && !(*win).closed {
                if let PointerOpType::Resize { .. } = op.op_type {
                    (*win).wm_requested.resizing = false;
                    (*win).manage_finish();
                    (*self.server).wm.dirty_windowing();
                }
                if let PointerOpType::Move = op.op_type {
                    if (*win).tiling_mode == crate::tiling::TilingMode::Overlay {
                        (*self.server).wm.dirty_windowing();
                    }
                }
                // Geometric mode detection: a move/resize that lands every
                // content edge on a visible desktop-grid cell edge makes the
                // window Tiled (it then reports the maximized state to its
                // client); landing off-grid makes it Floating, in place.
                // Only windows resolving Floating/Tiled participate —
                // Popup/Overlay/Status/Fullscreen are untouched.
                let resolved = (*self.server).wm.get_mode_for_window(win);
                if matches!(
                    resolved,
                    crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Tiled
                ) {
                    // Unscaled params: alignment classifies the resting
                    // geometry, the zoom-aware grab distance is irrelevant.
                    let sp = (*self.server).wm.layout.snap_params();
                    let (w, h) = match (*win).wm_requested.dimensions {
                        // A just-finished resize may not be acked into
                        // box_geom yet; the requested size is what the
                        // window is about to become.
                        Some(d) => (d.width as f64, d.height as f64),
                        None => ((*win).box_geom.width as f64, (*win).box_geom.height as f64),
                    };
                    let aligned = crate::policy::snap::is_cell_aligned(
                        (*win).virtual_x,
                        (*win).virtual_y,
                        w,
                        h,
                        &sp,
                        1.0,
                    );
                    if aligned && resolved != crate::tiling::TilingMode::Tiled {
                        (*win).tiling_mode = crate::tiling::TilingMode::Tiled;
                        (*win).mode_locked = true;
                        (*self.server).wm.dirty_windowing();
                    } else if !aligned && resolved == crate::tiling::TilingMode::Tiled {
                        // Un-tile in place: clearing was_tiled keeps the
                        // arrange Exit transition from restoring the old
                        // floating geometry.
                        (*win).was_tiled = false;
                        (*win).tiling_mode = crate::tiling::TilingMode::Floating;
                        (*win).mode_locked = true;
                        (*self.server).wm.dirty_windowing();
                    }
                }
                // A TAP — press+release without meaningful motion — is a
                // click, not a drag. A drag never focuses the window it
                // moves or resizes (the press grabs without focusing), but
                // a click on a window chooses it as any click does, so the
                // tap focuses here. And it pans: the press killed any
                // focus-follow pan (drag protection), which left a
                // mostly-hidden window stranded — aiming at a thin content
                // sliver at the screen edge, it is easy to land on the
                // border band instead and see nothing happen. Real drags
                // (any actual motion) keep the camera still. An overview
                // tap is excluded: its release already launched the exit
                // flight centered on this window, and a second pan computed
                // from the still-overview camera drags that flight off
                // target (hover focused it there anyway).
                let dx = (op.x - op.start_x).abs();
                let dy = (op.y - op.start_y).abs();
                if dx < 4 && dy < 4 && !op.started_in_overview && !(*win).is_status_bar() {
                    self.focus(Focus::Window(win));
                    self.focus_follow_pan(win);
                }
            }
            match op.input {
                SeatOpInput::Pointer => {
                    self.cursor.op_end_pointer();
                }
            }
        }
    }
}

/// Live overview displacement for one motion step of a move op: every
/// mapped, visible Floating/Tiled window the drag covers past the policy
/// threshold relocates to the side the drag vacated
/// (`crate::policy::overview::displace`). Single-level on purpose — a
/// displaced window does not cascade into a third.
///
/// Displacement is provisional while the button is held: `ledger` remembers
/// every displaced window's pre-displacement position, and each motion
/// re-evaluates the window AT THAT SPOT — a drag that stops covering it
/// releases it back home. The ledger drops with the op on release, which
/// finalizes wherever everything currently sits.
///
/// `moved_tiled` is what the dragged window was when it was GRABBED (the op's
/// `start_was_tiled`), for the reason the snap call above gives: the drag
/// un-tiles it on the first motion event, so its live mode reads Floating
/// however it started. The policy skips candidates of the other kind.
unsafe fn displace_covered(
    server: *mut Server,
    win: *mut crate::window::Window,
    moved_tiled: bool,
    drag_delta: (f64, f64),
    sp: &crate::policy::snap::SnapParams,
    ledger: &mut Vec<(*mut crate::window::Window, f64, f64)>,
) {
    let wm = &mut (*server).wm;
    // A window can close mid-drag; drop its entry before any deref.
    ledger.retain(|&(w, _, _)| wm.windows.iter().any(|&p| p == w));
    let moved = (
        (*win).virtual_x,
        (*win).virtual_y,
        (*win).box_geom.width as f64,
        (*win).box_geom.height as f64,
    );
    let mut ptrs: Vec<*mut crate::window::Window> = Vec::new();
    let mut cands: Vec<crate::policy::overview::DisplaceCandidate> = Vec::new();
    for &w in wm.windows.iter() {
        if w.is_null() || w == win || (*w).closed || (*w).minimized {
            continue;
        }
        if !matches!((*w).state, crate::window::WindowState::Mapped) {
            continue;
        }
        if (*w).is_status_bar() || (*w).is_wallpaper() || (*w).is_grid() {
            continue;
        }
        let mode = wm.get_mode_for_window(w);
        if mode != crate::tiling::TilingMode::Floating
            && mode != crate::tiling::TilingMode::Tiled
        {
            continue;
        }
        // Judge an already-displaced window at its ORIGINAL spot, not
        // where it fled to.
        let (ox, oy) = ledger
            .iter()
            .find(|&&(p, _, _)| p == w)
            .map(|&(_, x, y)| (x, y))
            .unwrap_or(((*w).virtual_x, (*w).virtual_y));
        ptrs.push(w);
        cands.push(crate::policy::overview::DisplaceCandidate {
            x: ox,
            y: oy,
            w: (*w).box_geom.width as f64,
            h: (*w).box_geom.height as f64,
            tiled: mode == crate::tiling::TilingMode::Tiled,
        });
    }
    let moves = crate::policy::overview::displace(
        moved, moved_tiled, drag_delta, &cands, sp, sp.gap_width,
    );

    let mut changed = false;
    let mut displaced_now: Vec<*mut crate::window::Window> = Vec::new();
    for &(idx, (nx, ny)) in &moves {
        let w = ptrs[idx];
        displaced_now.push(w);
        if !ledger.iter().any(|&(p, _, _)| p == w) {
            ledger.push((w, cands[idx].x, cands[idx].y));
        }
        if (*w).virtual_x != nx || (*w).virtual_y != ny {
            (*w).virtual_x = nx;
            (*w).virtual_y = ny;
            changed = true;
        }
    }
    // No longer covered at its original spot: back home.
    ledger.retain(|&(w, ox, oy)| {
        if displaced_now.contains(&w) {
            return true;
        }
        if !(*w).closed && ((*w).virtual_x != ox || (*w).virtual_y != oy) {
            (*w).virtual_x = ox;
            (*w).virtual_y = oy;
            changed = true;
        }
        false
    });
    if changed {
        wm.dirty_windowing();
    }
}

unsafe extern "C" fn handle_request_set_cursor(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, request_set_cursor);
    let event = data as *mut ffi::wlr_seat_pointer_request_set_cursor_event;
    
    let focused_client = ffi::river_wlr_seat_get_pointer_focused_client(seat.wlr_seat);
    
    let event_client = ffi::river_wlr_seat_client_get_client((*event).seat_client);
    let wm_client = if !(*seat.server).wm.object.is_null() {
        ffi::wl_resource_get_client((*seat.server).wm.object)
    } else {
        std::ptr::null_mut()
    };
    let is_wm = !wm_client.is_null() && event_client == wm_client;

    if focused_client == (*event).seat_client || is_wm {
        // The client owns the cursor image from here; a compositor-driven
        // xcursor animation would paint over it on its next tick.
        seat.cursor.stop_xcursor_animation();
        // An X11 client's cursor is a physical-pixel bitmap like the rest of
        // its drawing, but Xwayland commits it at buffer scale 1, so wlroots
        // would show it at that many LOGICAL pixels — Houdini's 48px
        // crosshair came out 96 physical px against the desktop's 48. Show it
        // at 1/scale, the way the window's own buffer already is
        // (`Window::x11_buffer_scale`), and put the hotspot in the same units.
        let scale = if is_xwayland_client(seat.server, event_client) {
            crate::xwayland_window::x11_scale_for_surface(
                seat.server,
                ffi::river_wlr_seat_get_pointer_focused_surface(seat.wlr_seat),
            )
        } else {
            1.0
        };
        if scale != 1.0 {
            log::debug!(
                "X11 client cursor: drawn at 1/{scale} (hotspot {}, {})",
                (*event).hotspot_x, (*event).hotspot_y
            );
        }
        seat.watch_x11_cursor((*event).surface, scale);
        let (hotspot_x, hotspot_y) = if scale != 1.0 {
            (
                ((*event).hotspot_x as f32 / scale).round() as i32,
                ((*event).hotspot_y as f32 / scale).round() as i32,
            )
        } else {
            ((*event).hotspot_x, (*event).hotspot_y)
        };
        ffi::wlr_cursor_set_surface(
            seat.cursor.wlr_cursor,
            (*event).surface,
            hotspot_x,
            hotspot_y,
        );
    }
}

/// Is this the Xwayland client itself? Every X11 window's requests arrive as
/// that one client, which is what separates an X11 cursor from a Wayland one.
unsafe fn is_xwayland_client(
    server: *mut crate::server::Server,
    client: *mut ffi::wl_client,
) -> bool {
    if server.is_null() || (*server).xwayland.is_null() || client.is_null() {
        return false;
    }
    let xwayland = (*server).xwayland as *mut crate::server::WlrXwayland;
    let xserver = (*xwayland).server as *mut ffi::wlr_xwayland_server;
    if xserver.is_null() {
        return false;
    }
    !(*xserver).client.is_null() && (*xserver).client == client
}

/// Keep an X11 cursor surface shown at 1/scale for as long as it is the
/// pointer image: `wlr_cursor` re-reads the surface's logical size on every
/// commit, so the shrink has to be re-applied there — before wlroots reads it,
/// which is why this listener is added ahead of `wlr_cursor_set_surface`.
unsafe extern "C" fn handle_x11_cursor_commit(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, x11_cursor_commit);
    ffi::river_wlr_surface_scale_logical_size(seat.x11_cursor_surface, seat.x11_cursor_scale);
}

unsafe extern "C" fn handle_x11_cursor_destroy(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, x11_cursor_destroy);
    seat.unwatch_x11_cursor();
}

unsafe extern "C" fn handle_request_set_selection(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, request_set_selection);
    let event = data as *mut ffi::wlr_seat_request_set_selection_event;
    ffi::wlr_seat_set_selection(seat.wlr_seat, (*event).source, (*event).serial);
}

unsafe extern "C" fn handle_request_start_drag(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, request_start_drag);
    let event = data as *mut ffi::wlr_seat_request_start_drag_event;

    assert!(seat.drag == DragState::None);

    if ffi::wlr_seat_validate_pointer_grab_serial(seat.wlr_seat, (*event).origin, (*event).serial) {
        ffi::wlr_seat_start_pointer_drag(seat.wlr_seat, (*event).drag, (*event).serial);
        return;
    }

    let mut point: *mut ffi::wlr_touch_point = std::ptr::null_mut();
    if ffi::wlr_seat_validate_touch_grab_serial(seat.wlr_seat, (*event).origin, (*event).serial, &mut point) {
        ffi::wlr_seat_start_touch_drag(seat.wlr_seat, (*event).drag, (*event).serial, point);
        return;
    }

    let source = ffi::river_wlr_drag_get_source((*event).drag);
    if !source.is_null() {
        ffi::wlr_data_source_destroy(source);
    }
}

unsafe extern "C" fn handle_start_drag(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, start_drag);
    let wlr_drag = data as *mut ffi::wlr_drag;

    assert!(seat.drag == DragState::None);
    let grab_type = ffi::river_wlr_drag_get_grab_type(wlr_drag);
    log::debug!("[drag] started (grab type {grab_type})");
    match grab_type {
        ffi::wlr_drag_grab_type_WLR_DRAG_GRAB_KEYBOARD_POINTER => {
            seat.drag = DragState::Pointer;
        }
        ffi::wlr_drag_grab_type_WLR_DRAG_GRAB_KEYBOARD_TOUCH => {
            seat.drag = DragState::Touch;
        }
        _ => {}
    }

    let drag_destroy_ptr = &mut seat.drag_destroy as *mut ffi::wl_listener as *mut WlListener;
    (*drag_destroy_ptr).notify = Some(handle_drag_destroy);
    wl_signal_add(
        ffi::river_wlr_drag_get_destroy_signal(wlr_drag),
        &mut seat.drag_destroy,
    );

    let wlr_drag_icon = ffi::river_wlr_drag_get_icon(wlr_drag);
    if !wlr_drag_icon.is_null() {
        if let Err(err) = crate::drag_icon::DragIcon::create(wlr_drag_icon, &mut seat.cursor) {
            log::error!("Failed to create drag icon: {}", err);
            let seat_client = ffi::river_wlr_drag_get_seat_client(wlr_drag);
            if !seat_client.is_null() {
                let client = ffi::river_wlr_seat_client_get_client(seat_client);
                if !client.is_null() {
                    ffi::wl_client_post_no_memory(client);
                }
            }
        }
    }
}

unsafe extern "C" fn handle_drag_destroy(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, drag_destroy);
    wl_listener_remove(&mut seat.drag_destroy);

    match seat.drag {
        DragState::None => unreachable!(),
        DragState::Pointer => {
            seat.cursor.update_state();
        }
        DragState::Touch => {}
    }
    seat.drag = DragState::None;
}

unsafe extern "C" fn handle_request_set_primary_selection(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let seat = &mut *crate::container_of!(listener, Seat, request_set_primary_selection);
    let event = data as *mut ffi::wlr_seat_request_set_primary_selection_event;
    ffi::wlr_seat_set_primary_selection(seat.wlr_seat, (*event).source, (*event).serial);
}

unsafe extern "C" fn seat_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn seat_focus_window(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    window_resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    (*seat).focus_requested = true;
    if window_resource.is_null() {
        (*seat).focus(Focus::None);
        return;
    }
    let window = ffi::wl_resource_get_user_data(window_resource) as *mut crate::window::Window;
    if !window.is_null() {
        (*seat).focus(Focus::Window(window));
    }
}

unsafe extern "C" fn seat_focus_shell_surface(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    shell_surface_resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    (*seat).focus_requested = true;
    if shell_surface_resource.is_null() {
        (*seat).focus(Focus::None);
        return;
    }
    let shell_surface = ffi::wl_resource_get_user_data(shell_surface_resource) as *mut crate::shell_surface::ShellSurface;
    if !shell_surface.is_null() {
        (*seat).focus(Focus::ShellSurface(shell_surface));
    }
}

unsafe extern "C" fn seat_clear_focus(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if !seat.is_null() {
        if (*(*seat).server).wm.ensure_windowing() {
            (*seat).focus_requested = true;
            (*seat).focus(Focus::None);
        }
    }
}

unsafe extern "C" fn seat_op_start_pointer(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    if (*seat).op.is_none() {
        log::debug!("start seat op pointer");
        let cursor_x = (*(*seat).cursor.wlr_cursor).x;
        let cursor_y = (*(*seat).cursor.wlr_cursor).y;
        (*seat).op = Some(SeatOp {
            sent_release: false,
            input: SeatOpInput::Pointer,
            start_x: cursor_x as i32,
            start_y: cursor_y as i32,
            x: cursor_x as i32,
            y: cursor_y as i32,
            window_ptr: std::ptr::null_mut(),
            op_type: PointerOpType::Move,
            start_win_x: 0,
            start_win_y: 0,
            start_win_w: 0,
            start_win_h: 0,
            start_win_virtual_x: 0.0,
            start_win_virtual_y: 0.0,
            start_pan_x: (*(*seat).server).wm.desk_pan_x,
            start_pan_y: (*(*seat).server).wm.desk_pan_y,
            start_tiling_mode: crate::tiling::TilingMode::Floating,
            start_was_tiled: false,
            start_mode_locked: false,
            started_in_overview: false,
        });
        (*(*seat).server).wm.stop_panning_animation();
        (*seat).cursor.op_start_pointer();
    }
}

unsafe extern "C" fn seat_op_end(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    (*seat).op_end();
}

unsafe extern "C" fn seat_get_pointer_binding(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    button: u32,
    modifiers: u32,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    let version = ffi::wl_resource_get_version(resource) as u32;
    if let Err(err) = crate::pointer_binding::PointerBinding::create(
        seat,
        client,
        version,
        id,
        button,
        modifiers,
    ) {
        log::error!("failed to create pointer binding: {}", err);
        ffi::wl_client_post_no_memory(client);
    }
}

unsafe extern "C" fn seat_set_xcursor_theme(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    name: *const ::std::os::raw::c_char,
    size: u32,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if let Err(err) = (*seat).cursor.set_theme(name, size) {
        log::error!("failed to set xcursor theme: {}", err);
        ffi::wl_client_post_no_memory(client);
    }
}

unsafe extern "C" fn seat_pointer_warp(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if seat.is_null() {
        return;
    }
    if !(*(*seat).server).wm.ensure_windowing() {
        return;
    }
    let cursor = &mut (*seat).cursor;
    ffi::wlr_cursor_warp_absolute(cursor.wlr_cursor, std::ptr::null_mut(), x as f64, y as f64);
}

static SEAT_INTERFACE: ffi::zcce_seat_v1_interface = ffi::zcce_seat_v1_interface {
    destroy: Some(seat_destroy),
    focus_window: Some(seat_focus_window),
    focus_shell_surface: Some(seat_focus_shell_surface),
    clear_focus: Some(seat_clear_focus),
    op_start_pointer: Some(seat_op_start_pointer),
    op_end: Some(seat_op_end),
    get_pointer_binding: Some(seat_get_pointer_binding),
    set_xcursor_theme: Some(seat_set_xcursor_theme),
    pointer_warp: Some(seat_pointer_warp),
};

unsafe extern "C" fn seat_inert_focus_window(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _window_resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn seat_inert_focus_shell_surface(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _shell_surface_resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn seat_inert_clear_focus(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn seat_inert_op_start_pointer(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn seat_inert_op_end(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
) {}

unsafe extern "C" fn seat_inert_get_pointer_binding(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _id: u32,
    _button: u32,
    _modifiers: u32,
) {}

unsafe extern "C" fn seat_inert_set_xcursor_theme(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _name: *const ::std::os::raw::c_char,
    _size: u32,
) {}

unsafe extern "C" fn seat_inert_pointer_warp(
    _client: *mut ffi::wl_client,
    _resource: *mut ffi::wl_resource,
    _x: i32,
    _y: i32,
) {}

static INERT_SEAT_INTERFACE: ffi::zcce_seat_v1_interface = ffi::zcce_seat_v1_interface {
    destroy: Some(seat_destroy),
    focus_window: Some(seat_inert_focus_window),
    focus_shell_surface: Some(seat_inert_focus_shell_surface),
    clear_focus: Some(seat_inert_clear_focus),
    op_start_pointer: Some(seat_inert_op_start_pointer),
    op_end: Some(seat_inert_op_end),
    get_pointer_binding: Some(seat_inert_get_pointer_binding),
    set_xcursor_theme: Some(seat_inert_set_xcursor_theme),
    pointer_warp: Some(seat_inert_pointer_warp),
};

unsafe extern "C" fn handle_destroy_resource(resource: *mut ffi::wl_resource) {
    let seat = ffi::wl_resource_get_user_data(resource) as *mut Seat;
    if !seat.is_null() {
        if (*seat).object != resource {
            return;
        }
        (*seat).object = std::ptr::null_mut();
    }
}
