//! `handle_button`: a pointer button press or release, through every grab, op,
//! border, popup, layer and window it can land on. Split out of cursor.rs on
//! 2026-10-10.

use super::*;

pub(crate) unsafe extern "C" fn handle_button(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, button_listener);
    let event = data as *mut ffi::wlr_pointer_button_event;
    (*cursor.seat).handle_activity();
    if cursor.view_drag.is_some() {
        cursor.end_view_drag("button");
    }
    // Before the button reaches the client, so a click on the popup is not
    // a Ctrl-click (see `PopupWheel`).
    cursor.end_popup_wheel("button");
    cursor.end_hscroll_shift("button");
    
    let seat = &mut *cursor.seat;
    let lx = cursor.x();
    let ly = cursor.y();
    let server = seat.server;

    // First deliberate input ends the session-restore settling phase (see
    // the focus gate in Window::map).
    if (*event).state == ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED {
        (*server).wm.startup_input_seen = true;
    }

    let mut is_app_surface = false;
    let mut is_overlay_window = false;
    if let Some(result) = crate::shared::scene().at(lx, ly) {
        match result.data {
            SceneNodeDataVal::Window(window) => {
                // The grid does not block: hitting it means the press is on a
                // desktop item (its input region covers nothing else), and
                // the item drag needs the press delivered in overview like
                // any chrome click.
                if !(*window).is_status_bar()
                    && !(*window).is_wallpaper()
                    && !(*window).is_grid()
                {
                    is_app_surface = true;
                    // Popup counts as chrome like Overlay: the cce-cloud
                    // launcher must keep receiving clicks in overview.
                    if (*window).tiling_mode == crate::tiling::TilingMode::Overlay
                        || (*window).tiling_mode == crate::tiling::TilingMode::Popup
                    {
                        is_overlay_window = true;
                    }
                }
            }
            SceneNodeDataVal::LayerSurface(layer_surface) => {
                if !layer_surface.is_null() {
                    is_app_surface = true;
                    if is_cloud_layer(layer_surface) {
                        is_overlay_window = true;
                    }
                }
            }
            SceneNodeDataVal::OverrideRedirect(_) => {
                is_app_surface = true;
            }
            _ => {}
        }
    }
    let should_block_button = (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview && is_app_surface && !is_overlay_window;
    
    if (*event).state == ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED {
        if cursor.pressed.contains(&(*event).button) {
            log::error!("ignoring duplicate pointer button {} press", (*event).button);
            return;
        }

        press_dismissals(server, lx, ly);

        // Status-bar segments are dragged either in adjust-position mode or
        // directly with super+left-drag (0x40 = WLR_MODIFIER_LOGO).
        let super_held = {
            let wlr_keyboard = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
            !wlr_keyboard.is_null() && (ffi::wlr_keyboard_get_modifiers(wlr_keyboard) & 0x40) != 0
        };
        if (*event).button == 0x110 && ((*(*seat).server).wm.adjust_position_mode || super_held) {
            let mut clicked_status: *mut crate::window::Window = std::ptr::null_mut();
            if let Some(result) = crate::shared::scene().at(lx, ly) {
                if let SceneNodeDataVal::Window(window) = result.data {
                    if (*window).is_status_bar() {
                        clicked_status = window;
                    }
                }
            }
            // An EXPANDED segment (menu open) is never grabbed: its rows are
            // clicked, and a grab here would swallow the press the "Done"
            // row needs to leave adjust mode — the one control that ends the
            // mode from the bar would be unreachable while it is on.
            if !clicked_status.is_null() && !(*server).wm.is_expanded_status_segment(clicked_status) {
                (*server).wm.stop_panning_animation();
                let cursor_x = (*cursor.wlr_cursor).x;
                let cursor_y = (*cursor.wlr_cursor).y;
                seat.op = Some(crate::seat::SeatOp {
                    input: crate::seat::SeatOpInput::Pointer,
                    start_x: cursor_x as i32,
                    start_y: cursor_y as i32,
                    x: cursor_x as i32,
                    y: cursor_y as i32,
                    window_ptr: clicked_status,
                    op_type: crate::seat::PointerOpType::Move,
                    start_win_x: (*clicked_status).box_geom.x,
                    start_win_y: (*clicked_status).box_geom.y,
                    start_win_w: (*clicked_status).box_geom.width as u32,
                    start_win_h: (*clicked_status).box_geom.height as u32,
                    start_win_virtual_x: (*clicked_status).virtual_x,
                    start_win_virtual_y: (*clicked_status).virtual_y,
                    start_was_tiled: false,
                    start_pan_x: (*server).wm.desk_pan_x,
                    start_pan_y: (*server).wm.desk_pan_y,
                    start_tiling_mode: (*clicked_status).tiling_mode,
                    start_mode_locked: (*clicked_status).mode_locked,
                    started_in_overview: false,
                });
                cursor.op_start_pointer();
                cursor.pressed.insert((*event).button);
                cursor.set_xcursor(b"grab\0".as_ptr() as *const _);
                return;
            }
        }

        // --- ADJUST-MODE CLICK HANDLING (overview, or Super held) ---
        // A press on a window's body grabs the whole window to move it; a
        // press on its ring falls through to the border path. Only in
        // overview does a background press mean anything (it exits); with
        // Super held at zoom 1 it falls through to the normal desktop press.
        let in_overview = (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview;
        if (*event).button == 0x110 && (*(*seat).server).wm.window_adjust_active() {
            let mut clicked_win: *mut crate::window::Window = std::ptr::null_mut();
            let mut clicked_cloud_layer = false;
            if let Some(result) = crate::shared::scene().at(lx, ly) {
                match result.data {
                    SceneNodeDataVal::Window(window) => clicked_win = window,
                    SceneNodeDataVal::LayerSurface(layer_surface) => {
                        clicked_cloud_layer = is_cloud_layer(layer_surface);
                    }
                    _ => {}
                }
            }

            // A press on the border band falls through to the normal
            // border path below (move/resize by zone, zoom-aware) — the
            // resize controls work at any zoom. Content presses grab the
            // whole window; true background presses exit overview.
            //
            // Chrome windows (Overlay docks, Popup surfaces like the
            // cce-cloud launcher) and cce-cloud layer surfaces (launcher,
            // desktop/app context menus) are neither: they stay
            // interactive UI during overview, so their presses fall
            // through to the normal path (focus + delivery to the app)
            // and overview stays up.
            let overview_chrome = clicked_cloud_layer
                || (!clicked_win.is_null()
                    && matches!(
                        (*clicked_win).tiling_mode,
                        crate::tiling::TilingMode::Overlay | crate::tiling::TilingMode::Popup
                    ));
            // Hitting the grid means the press landed on a DESKTOP ITEM —
            // its input region covers the item rects and nothing else — so
            // it is chrome-like: fall through to normal delivery and the
            // grid client starts its item drag, in overview exactly as in
            // normal mode. Bare canvas misses the grid entirely (that is
            // the input region again) and still exits overview below.
            let clicked_grid = !clicked_win.is_null() && (*clicked_win).is_grid();
            // The bare canvas counts as background in overview: a press on
            // it must exit overview like any desktop press, never grab the
            // canvas itself as if it were a window.
            let overview_win_valid = !clicked_win.is_null()
                && !(*clicked_win).is_status_bar()
                && !(*clicked_win).is_wallpaper()
                && !clicked_grid;
            let overview_border_zone = if overview_win_valid {
                get_border_zone(clicked_win, lx, ly)
            } else {
                BorderZone::None
            };
            // A window button is a click: nothing is grabbed, the client
            // never sees the press, and the release decides
            // (`Cursor::button_press`, answered in the release path).
            if let BorderZone::Button(elem) = overview_border_zone {
                cursor.button_press = Some((clicked_win, elem));
                cursor.pressed.insert((*event).button);
                return;
            }
            // A press on a SELECTED desktop image grabs the whole selection
            // — windows and images — from the image's side: the grid never
            // sees the press, the compositor moves everything and tells the
            // grid where its images went (`crate::selection`). A press on an
            // unselected image drops the selection, as one on an unselected
            // window does, and goes to the grid as before.
            if clicked_grid && in_overview {
                let (vx, vy) = (*server).wm.layout_to_virtual(lx, ly);
                if (*server).wm.selected_item_at(vx, vy).is_some() {
                    (*server).wm.stop_panning_animation();
                    seat.group_move.clear();
                    seat.group_items.clear();
                    // The first Tiled window carried is the snap anchor.
                    let mut tiled_anchor: Option<(f64, f64)> = None;
                    let carried: Vec<*mut crate::window::Window> = (*server)
                        .wm
                        .selection
                        .windows
                        .iter()
                        .copied()
                        .filter(|&w| (*server).wm.selectable(w))
                        .collect();
                    for w in carried {
                        seat.group_move.push((w, (*w).virtual_x, (*w).virtual_y));
                        if (*w).tiling_mode == crate::tiling::TilingMode::Tiled {
                            if tiled_anchor.is_none() {
                                tiled_anchor = Some(((*w).virtual_x, (*w).virtual_y));
                            }
                        } else if (*w).tiling_mode == crate::tiling::TilingMode::Floating {
                            (*server).wm.raise_window(w);
                        }
                    }
                    let ids: Vec<u64> = (*server).wm.selection.items.clone();
                    for id in ids {
                        if let Some(item) = (*server).wm.desktop_item(id) {
                            seat.group_items.push((id, item.x, item.y));
                        }
                    }
                    let cursor_x = (*cursor.wlr_cursor).x;
                    let cursor_y = (*cursor.wlr_cursor).y;
                    let (anchor_x, anchor_y) = tiled_anchor.unwrap_or((0.0, 0.0));
                    seat.op = Some(crate::seat::SeatOp {
                        input: crate::seat::SeatOpInput::Pointer,
                        start_x: cursor_x as i32,
                        start_y: cursor_y as i32,
                        x: cursor_x as i32,
                        y: cursor_y as i32,
                        window_ptr: std::ptr::null_mut(),
                        op_type: crate::seat::PointerOpType::GroupMove,
                        start_win_x: 0,
                        start_win_y: 0,
                        start_win_w: 0,
                        start_win_h: 0,
                        start_win_virtual_x: anchor_x,
                        start_win_virtual_y: anchor_y,
                        start_was_tiled: tiled_anchor.is_some(),
                        start_pan_x: (*server).wm.desk_pan_x,
                        start_pan_y: (*server).wm.desk_pan_y,
                        start_tiling_mode: crate::tiling::TilingMode::Floating,
                        start_mode_locked: false,
                        started_in_overview: true,
                    });
                    cursor.op_start_pointer();
                    cursor.pressed.insert((*event).button);
                    cursor.set_xcursor(b"grab\0".as_ptr() as *const _);
                    return;
                }
                (*server).wm.selection_clear();
            }
            if overview_chrome || clicked_grid {
                // fall through
            } else if overview_win_valid && matches!(overview_border_zone, BorderZone::None) {
                // The grab does NOT focus the window: moving a window is
                // not choosing it, and the ring already sits on it through
                // `adjust_hover`. A tap — press and release without motion
                // — is a click and focuses in `op_end`; a Floating window
                // is raised for the drag in `op_start_pointer`. (In
                // overview hover already focused it.)
                (*server).wm.stop_panning_animation();
                // A selected window carries the rest of the selection with
                // it; a press on any other window drops the selection, the
                // way a press on a node outside the region does in
                // cce-designer. The carried Floating windows are raised
                // here, in stacking order, and the grabbed one over them in
                // `op_start_pointer`.
                seat.group_move.clear();
                seat.group_items.clear();
                if (*server).wm.is_selected(clicked_win) {
                    let carried: Vec<*mut crate::window::Window> = (*server)
                        .wm
                        .selection
                        .windows
                        .iter()
                        .copied()
                        .filter(|&w| w != clicked_win && (*server).wm.selectable(w))
                        .collect();
                    for w in carried {
                        seat.group_move.push((w, (*w).virtual_x, (*w).virtual_y));
                        if (*w).tiling_mode == crate::tiling::TilingMode::Floating {
                            (*server).wm.raise_window(w);
                        }
                    }
                    // And the selected desktop images, by the same offset.
                    let ids: Vec<u64> = (*server).wm.selection.items.clone();
                    for id in ids {
                        if let Some(item) = (*server).wm.desktop_item(id) {
                            seat.group_items.push((id, item.x, item.y));
                        }
                    }
                } else {
                    (*server).wm.selection_clear();
                }
                let cursor_x = (*cursor.wlr_cursor).x;
                let cursor_y = (*cursor.wlr_cursor).y;
                seat.op = Some(crate::seat::SeatOp {
                    input: crate::seat::SeatOpInput::Pointer,
                    start_x: cursor_x as i32,
                    start_y: cursor_y as i32,
                    x: cursor_x as i32,
                    y: cursor_y as i32,
                    window_ptr: clicked_win,
                    op_type: crate::seat::PointerOpType::Move,
                    start_win_x: (*clicked_win).box_geom.x,
                    start_win_y: (*clicked_win).box_geom.y,
                    start_win_w: (*clicked_win).box_geom.width as u32,
                    start_win_h: (*clicked_win).box_geom.height as u32,
                    start_win_virtual_x: (*clicked_win).virtual_x,
                    start_win_virtual_y: (*clicked_win).virtual_y,
                    start_was_tiled: (*clicked_win).tiling_mode == crate::tiling::TilingMode::Tiled,
                    start_pan_x: (*server).wm.desk_pan_x,
                    start_pan_y: (*server).wm.desk_pan_y,
                    start_tiling_mode: (*clicked_win).tiling_mode,
                    start_mode_locked: (*clicked_win).mode_locked,
                    started_in_overview: in_overview,
                });
                cursor.op_start_pointer();
                cursor.pressed.insert((*event).button);
                cursor.set_xcursor(b"grab\0".as_ptr() as *const _);
                return;
            } else if !overview_win_valid && in_overview {
                // Click-away with a cce-cloud popup open (desktop context
                // menu, launcher): the press dismisses the popup and does
                // nothing else — overview stays up. Dropping keyboard focus
                // IS the dismissal: cce-cloud closes itself on keyboard
                // leave, the same signal a normal-mode click-away produces
                // through its focus change.
                if let crate::layer_shell::LayerShellSeatFocus::Exclusive(key) =
                    seat.layer_shell.scheduled_focus
                {
                    if let Some(&layer_surface) = (*server).layer_shell.surfaces.get(key) {
                        if is_cloud_layer(layer_surface) {
                            seat.focus(Focus::None);
                            cursor.pressed.insert((*event).button);
                            return;
                        }
                    }
                }
                // A press on the bare desktop is a click (which leaves
                // overview) or the start of a drag-selection, and only the
                // release can say which: it arms the selection, and the
                // release path answers a press that never travelled.
                (*server).wm.stop_panning_animation();
                (*server).wm.selection_press(lx, ly);
                seat.group_move.clear();
                let cursor_x = (*cursor.wlr_cursor).x;
                let cursor_y = (*cursor.wlr_cursor).y;
                seat.op = Some(crate::seat::SeatOp {
                    input: crate::seat::SeatOpInput::Pointer,
                    start_x: cursor_x as i32,
                    start_y: cursor_y as i32,
                    x: cursor_x as i32,
                    y: cursor_y as i32,
                    window_ptr: std::ptr::null_mut(),
                    op_type: crate::seat::PointerOpType::Select,
                    start_win_x: 0,
                    start_win_y: 0,
                    start_win_w: 0,
                    start_win_h: 0,
                    start_win_virtual_x: 0.0,
                    start_win_virtual_y: 0.0,
                    start_was_tiled: false,
                    start_pan_x: (*server).wm.desk_pan_x,
                    start_pan_y: (*server).wm.desk_pan_y,
                    start_tiling_mode: crate::tiling::TilingMode::Floating,
                    start_mode_locked: false,
                    started_in_overview: true,
                });
                cursor.op_start_pointer();
                cursor.pressed.insert((*event).button);
                cursor.set_xcursor(b"crosshair\0".as_ptr() as *const _);
                return;
            }
        }

        let wlr_keyboard = ffi::river_wlr_seat_get_keyboard(seat.wlr_seat);
        let modifiers = if !wlr_keyboard.is_null() {
            ffi::wlr_keyboard_get_modifiers(wlr_keyboard)
        } else {
            0
        };

        if (*event).button == 0x111 && modifiers == 0 {
            let mut clicked_interactive = false;
            if let Some(result) = crate::shared::scene().at(lx, ly) {
                match result.data {
                    SceneNodeDataVal::Window(window) => {
                        if !(*window).is_wallpaper() {
                            clicked_interactive = true;
                        }
                    }
                    SceneNodeDataVal::LayerSurface(_) | SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                        clicked_interactive = true;
                    }
                }
            }
            if !clicked_interactive {
                cursor.right_click_on_bg = true;
                let x = cursor.x() as i32;
                let y = cursor.y() as i32;
                let home = std::env::var("HOME").unwrap_or_default();
                let mut cmd = format!("{}/.local/bin/cce-desktop-menu -x {} -y {}", home, x, y);
                // The menu's "Window Mode" page acts on the FOCUSED window,
                // and focus is dropped right below (so the popup takes the
                // keyboard and a click-away dismisses it) — by the time the
                // script asks `ccectl windows` nothing is focused. Hand it
                // the window here instead. Only a window whose mode
                // `set-mode` accepts is worth naming.
                if let Focus::Window(w) = seat.focused {
                    if !w.is_null()
                        && matches!(
                            (*w).tiling_mode,
                            crate::tiling::TilingMode::Floating
                                | crate::tiling::TilingMode::Tiled
                                | crate::tiling::TilingMode::Fullscreen
                        )
                    {
                        let app_id = (*w).get_app_id_string().unwrap_or_else(|| "unknown".to_string());
                        cmd.push_str(&format!(
                            " -i {} -a '{}'",
                            (*w).ref_key.index,
                            app_id.replace('\'', "'\\''")
                        ));
                    }
                }
                (*server).wm.execute_action(&crate::config::Action::Spawn, Some(&cmd));

                seat.focus(Focus::None);
                crate::shared::pending().dirty_windowing();

                cursor.pressed.insert((*event).button);
                return;
            }
        }
        
        let mut matched_pb: Option<crate::config::PointerBind> = None;
        for pb in &(*(*seat).server).wm.pointer_binds {
            if pb.button == (*event).button && pb.mods == modifiers {
                matched_pb = Some(pb.clone());
                break;
            }
        }
        
        if let Some(pb) = matched_pb {
            let lx = cursor.x();
            let ly = cursor.y();
            let server = seat.server;
            let mut target_win: *mut crate::window::Window = std::ptr::null_mut();
            if let Some(result) = crate::shared::scene().at(lx, ly) {
                if let SceneNodeDataVal::Window(window) = result.data {
                    target_win = window;
                }
            }
            
            if !target_win.is_null() && !(*target_win).is_status_bar() && !(*target_win).is_wallpaper() {
                // Before the un-tile below: a tiled window's drag snaps hard
                // to whole squares, and by op_update the mode reads Floating.
                let grabbed_tiled =
                    (*target_win).tiling_mode == crate::tiling::TilingMode::Tiled;
                if (*target_win).tiling_mode != crate::tiling::TilingMode::Floating
                    && (*target_win).tiling_mode != crate::tiling::TilingMode::Popup
                    && (*target_win).tiling_mode != crate::tiling::TilingMode::Fullscreen
                    && (*target_win).tiling_mode != crate::tiling::TilingMode::Overlay
                    // A drag moves a Utility window; it must not re-class it.
                    && (*target_win).tiling_mode != crate::tiling::TilingMode::Utility
                {
                    // A tiled window un-tiles for the drag but keeps its
                    // cell-quantized geometry; landing grid-aligned re-tiles
                    // it (op_end geometric detection).
                    (*target_win).was_tiled = false;
                    (*target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                    (*target_win).mode_locked = true;
                }
                // No focus on the grab: a bound move/resize drag acts on
                // the window under the pointer without choosing it (a tap
                // focuses in `op_end`, the raise is in `op_start_pointer`).
                let op_type = match pb.action {
                    crate::config::Action::Move => Some(crate::seat::PointerOpType::Move),
                    // The modifier binding is a resize path the border zones
                    // never see, so it carries its own Utility rejection.
                    crate::config::Action::Resize
                        if (*target_win).tiling_mode == crate::tiling::TilingMode::Utility =>
                    {
                        None
                    }
                    crate::config::Action::Resize => {
                        let edges = get_closest_edges(target_win, lx, ly);
                        Some(crate::seat::PointerOpType::Resize { edges })
                    }
                    _ => None,
                };
                
                if let Some(ot) = op_type {
                    (*server).wm.stop_panning_animation();
                    let cursor_x = (*cursor.wlr_cursor).x;
                    let cursor_y = (*cursor.wlr_cursor).y;
                    seat.op = Some(crate::seat::SeatOp {
                        input: crate::seat::SeatOpInput::Pointer,
                        start_x: cursor_x as i32,
                        start_y: cursor_y as i32,
                        x: cursor_x as i32,
                        y: cursor_y as i32,
                        window_ptr: target_win,
                        op_type: ot,
                        start_win_x: (*target_win).box_geom.x,
                        start_win_y: (*target_win).box_geom.y,
                        start_win_w: (*target_win).box_geom.width as u32,
                        start_win_h: (*target_win).box_geom.height as u32,
                        start_win_virtual_x: (*target_win).virtual_x,
                        start_win_virtual_y: (*target_win).virtual_y,
                        start_tiling_mode: (*target_win).tiling_mode,
                        start_was_tiled: grabbed_tiled,
                        start_mode_locked: (*target_win).mode_locked,
                        start_pan_x: (*(*seat).server).wm.desk_pan_x,
                        start_pan_y: (*(*seat).server).wm.desk_pan_y,
                        started_in_overview: (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview,
                    });
                    cursor.op_start_pointer();
                    cursor.pressed.insert((*event).button);

                    match ot {
                        crate::seat::PointerOpType::Resize { edges } => {
                            let cursor_name = get_resize_cursor_name(edges);
                            cursor.set_xcursor(cursor_name.as_ptr() as *const _);
                        }
                        crate::seat::PointerOpType::Move => {
                            cursor.set_xcursor(b"grab\0".as_ptr() as *const _);
                        }
                        // A pointer bind is a move or a resize.
                        crate::seat::PointerOpType::Select
                        | crate::seat::PointerOpType::GroupMove => {}
                    }
                    return;
                }
            }
        }

        let lx = cursor.x();
        let ly = cursor.y();
        let server = seat.server;
        let mut border_target_win: *mut crate::window::Window = std::ptr::null_mut();
        if let Some(result) = crate::shared::scene().at(lx, ly) {
            if let SceneNodeDataVal::Window(window) = result.data {
                border_target_win = window;
            }
        }

        if !border_target_win.is_null() && !(*border_target_win).is_status_bar() && !(*border_target_win).is_wallpaper() && !(*border_target_win).is_grid() && (
            (*border_target_win).tiling_mode != crate::tiling::TilingMode::Popup
            && (*border_target_win).tiling_mode != crate::tiling::TilingMode::Fullscreen
        ) {
            let initial_mode = (*border_target_win).tiling_mode;
            let zone = get_border_zone(border_target_win, lx, ly);
            // The window context menu (`scripts/cce-app-menu`, a cce-cloud
            // popup like the desktop menu and cce-grid's item menu). A
            // right-click on a handle disc opens it in either adjust mode;
            // in OVERVIEW a right-click anywhere on the window does: the
            // client never sees buttons there (`should_block_button`), so
            // the press is the compositor's to spend, and the menu is how a
            // window's mode is set from the overview. Overlay docks are
            // chrome and keep their clicks (`overview_chrome` above);
            // Utility windows take no handles and have no mode to set.
            let menu_on_body = in_overview
                && !matches!(
                    initial_mode,
                    crate::tiling::TilingMode::Overlay | crate::tiling::TilingMode::Utility
                );
            if (*event).button == 0x111
                && modifiers == 0
                && (!matches!(zone, BorderZone::None) || menu_on_body)
            {
                cursor.right_click_on_border = true;
                let x = cursor.x() as i32;
                let y = cursor.y() as i32;
                let index = (*border_target_win).ref_key.index;
                let app_id = (*border_target_win).get_app_id_string().unwrap_or_else(|| "unknown".to_string());
                // The command runs under `sh -c`: quote the app_id, which
                // is client-chosen text.
                let app_id = format!("'{}'", app_id.replace('\'', "'\\''"));
                let home = std::env::var("HOME").unwrap_or_default();
                let cmd = format!("{}/.local/bin/cce-app-menu -x {} -y {} -i {} -a {}", home, x, y, index, app_id);
                (*server).wm.execute_action(&crate::config::Action::Spawn, Some(&cmd));

                cursor.pressed.insert((*event).button);
                return;
            }

            match zone {
                BorderZone::Resize(edges) => {
                    if (*event).button == 0x110 { // BTN_LEFT
                        // Unreachable for Utility (get_border_zone maps its
                        // whole band to Move), and gated here regardless.
                        if initial_mode == crate::tiling::TilingMode::Utility {
                            return;
                        }
                        if initial_mode != crate::tiling::TilingMode::Floating {
                            // A tiled window un-tiles for the drag but keeps its
                            // cell-quantized geometry; landing grid-aligned re-tiles
                            // it (op_end geometric detection).
                            (*border_target_win).was_tiled = false;
                            (*border_target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                            (*border_target_win).mode_locked = true;
                        }

                        // No focus on the grab (see the body grab above).
                        (*server).wm.stop_panning_animation();
                        let cursor_x = (*cursor.wlr_cursor).x;
                        let cursor_y = (*cursor.wlr_cursor).y;
                        seat.op = Some(crate::seat::SeatOp {
                            input: crate::seat::SeatOpInput::Pointer,
                            start_x: cursor_x as i32,
                            start_y: cursor_y as i32,
                            x: cursor_x as i32,
                            y: cursor_y as i32,
                            window_ptr: border_target_win,
                            op_type: crate::seat::PointerOpType::Resize { edges },
                            start_win_x: (*border_target_win).box_geom.x,
                            start_win_y: (*border_target_win).box_geom.y,
                            start_win_w: (*border_target_win).box_geom.width as u32,
                            start_win_h: (*border_target_win).box_geom.height as u32,
                            start_win_virtual_x: (*border_target_win).virtual_x,
                            start_win_virtual_y: (*border_target_win).virtual_y,
                            start_tiling_mode: (*border_target_win).tiling_mode,
                            start_was_tiled: initial_mode == crate::tiling::TilingMode::Tiled,
                            start_mode_locked: (*border_target_win).mode_locked,
                            start_pan_x: (*(*seat).server).wm.desk_pan_x,
                        start_pan_y: (*(*seat).server).wm.desk_pan_y,
                        started_in_overview: (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview,
                        });
                        cursor.op_start_pointer();
                        cursor.pressed.insert((*event).button);

                        let cursor_name = get_resize_cursor_name(edges);
                        cursor.set_xcursor(cursor_name.as_ptr() as *const _);
                        return;
                    }
                }
                BorderZone::Move => {
                    if (*event).button == 0x110 { // BTN_LEFT
                        let current_time = (*event).time_msec;
                        let is_titlebar = ly < (*border_target_win).box_geom.y as f64;
                        let is_double_click = is_titlebar
                            && border_target_win == cursor.last_click_window
                            && current_time.saturating_sub(cursor.last_click_time) < 300;

                        cursor.last_click_time = current_time;
                        cursor.last_click_window = border_target_win;

                        // A Utility window has no Tiled state to toggle;
                        // the double-click falls through to an ordinary move.
                        if is_double_click && initial_mode != crate::tiling::TilingMode::Utility {
                            if initial_mode == crate::tiling::TilingMode::Tiled {
                                (*border_target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                                (*border_target_win).mode_locked = true;
                            } else {
                                (*border_target_win).tiling_mode = crate::tiling::TilingMode::Tiled;
                                (*border_target_win).mode_locked = true;
                            }
                            seat.focus(Focus::Window(border_target_win));
                            crate::shared::pending().dirty_windowing();
                            cursor.last_click_time = 0;
                            cursor.last_click_window = std::ptr::null_mut();
                            return;
                        }

                        if initial_mode != crate::tiling::TilingMode::Floating
                            && initial_mode != crate::tiling::TilingMode::Overlay
                            // A drag moves a Utility window; it must not
                            // re-class it.
                            && initial_mode != crate::tiling::TilingMode::Utility
                        {
                            // A tiled window un-tiles for the drag but keeps its
                            // cell-quantized geometry; landing grid-aligned re-tiles
                            // it (op_end geometric detection).
                            (*border_target_win).was_tiled = false;
                            (*border_target_win).tiling_mode = crate::tiling::TilingMode::Floating;
                            (*border_target_win).mode_locked = true;
                        }

                        // No focus on the grab (see the body grab above).
                        (*server).wm.stop_panning_animation();
                        let cursor_x = (*cursor.wlr_cursor).x;
                        let cursor_y = (*cursor.wlr_cursor).y;
                        seat.op = Some(crate::seat::SeatOp {
                            input: crate::seat::SeatOpInput::Pointer,
                            start_x: cursor_x as i32,
                            start_y: cursor_y as i32,
                            x: cursor_x as i32,
                            y: cursor_y as i32,
                            window_ptr: border_target_win,
                            op_type: crate::seat::PointerOpType::Move,
                            start_win_x: (*border_target_win).box_geom.x,
                            start_win_y: (*border_target_win).box_geom.y,
                            start_win_w: (*border_target_win).box_geom.width as u32,
                            start_win_h: (*border_target_win).box_geom.height as u32,
                            start_win_virtual_x: (*border_target_win).virtual_x,
                            start_win_virtual_y: (*border_target_win).virtual_y,
                            start_tiling_mode: (*border_target_win).tiling_mode,
                            start_was_tiled: initial_mode == crate::tiling::TilingMode::Tiled,
                            start_mode_locked: (*border_target_win).mode_locked,
                            start_pan_x: (*(*seat).server).wm.desk_pan_x,
                        start_pan_y: (*(*seat).server).wm.desk_pan_y,
                        started_in_overview: (*(*seat).server).wm.mode == crate::window_manager::WindowManagerMode::Overview,
                        });
                        cursor.op_start_pointer();
                        cursor.pressed.insert((*event).button);

                        cursor.set_xcursor(b"grab\0".as_ptr() as *const _);
                        return;
                    }
                }
                // Answered in the adjust-mode block above, which a button
                // zone always passes through (get_border_zone gives none
                // outside adjust mode).
                BorderZone::Button(_) => {}
                BorderZone::None => {}
            }
        }

        cursor.pressed.insert((*event).button);

        if !should_block_button {
            let first_grab_button = cursor.notified_pressed.is_empty();
            ffi::wlr_seat_pointer_notify_button(
                seat.wlr_seat,
                (*event).time_msec,
                (*event).button,
                (*event).state,
            );
            cursor.notified_pressed.insert((*event).button);
            // First grab button: record the pressed surface's layout origin
            // as the implicit grab's frame of reference (passthrough keeps
            // motion surface-relative through it while the button is held).
            if first_grab_button {
                let glx = cursor.x();
                let gly = cursor.y();
                if let Some(result) = crate::shared::scene().at(glx, gly) {
                    // A surface node's scene buffer begins with its node.
                    let mut ratio = 1.0;
                    if !result.surface.is_null() && !result.node.is_null() {
                        let dest_w = ffi::river_scene_buffer_get_dest_width(result.node as *mut ffi::wlr_scene_buffer);
                        let surf_w = ffi::river_wlr_surface_get_width(result.surface);
                        if dest_w > 0 && surf_w > 0 {
                            ratio = surf_w as f64 / dest_w as f64;
                        }
                    }
                    cursor.grab_scale = ratio;
                    cursor.grab_origin = (glx - result.sx / ratio, gly - result.sy / ratio);
                }
                // A grab that starts on the grid freezes its node mapping
                // here instead of using grab_origin: passthrough maps motion
                // against these values for the whole grab, so camera motion
                // (which moves the node every frame) reads as nothing rather
                // than as pointer motion. See the grab branch in passthrough.
                cursor.grab_grid = None;
                let grab_focused =
                    ffi::river_wlr_seat_get_pointer_focused_surface(seat.wlr_seat);
                if !grab_focused.is_null() {
                    if let Some((gsurf, nx, ny, scale, _, _)) = grid_node_info(seat.server) {
                        if gsurf == grab_focused {
                            cursor.grab_grid = Some((nx, ny, scale));
                        }
                    }
                }
            }
        }

        // If pressed, update focus to window under cursor
        let lx = cursor.x();
        let ly = cursor.y();
        let server = seat.server;
        let mut clicked_something = false;
        if let Some(result) = crate::shared::scene().at(lx, ly) {
            match result.data {
                SceneNodeDataVal::Window(window) => {
                    clicked_something = true;
                    if !(*window).is_status_bar() && !(*window).is_wallpaper() {
                        seat.focus(Focus::Window(window));
                    }
                }
                SceneNodeDataVal::LayerSurface(layer_surface) => {
                    clicked_something = true;
                    if layer_takes_click_focus(layer_surface) {
                        seat.focus(Focus::LayerSurface(result.surface));
                    }
                }
                SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                    clicked_something = true;
                }
            }
        }

        if !clicked_something && (*event).button == 0x110 {
            // Restore placeholders are bare scene rects the hit-test can't
            // see, but they stand in for restored windows — clicking one
            // gets the same camera rules as clicking the real window.
            let wm = &mut (*server).wm;
            if let Some((pvx, pvy, pw, ph)) = wm.placeholder_at(lx, ly) {
                wm.pan_to_virtual_rect(pvx, pvy, pw, ph);
            } else {
                seat.focus(Focus::None);
                crate::shared::pending().dirty_windowing();
            }
        }
    } else {
        assert_eq!((*event).state, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_RELEASED);
        // Pair the release for the client BEFORE any compositor-side
        // consumption below: if the press was forwarded, the release always
        // is too, or the client is left with an orphaned press.
        if cursor.notified_pressed.remove(&(*event).button) {
            ffi::wlr_seat_pointer_notify_button(
                seat.wlr_seat,
                (*event).time_msec,
                (*event).button,
                (*event).state,
            );
        }
        // The implicit grab ends with its last button; the frozen grid
        // mapping must not outlive it into the next grab.
        if cursor.notified_pressed.is_empty() {
            cursor.grab_grid = None;
        }
        if seat.op.is_some() {
            let cursor_x = (*cursor.wlr_cursor).x;
            let cursor_y = (*cursor.wlr_cursor).y;
            seat.op_update(cursor_x as i32, cursor_y as i32);
            
            let op = seat.op.unwrap();

            if op.op_type == crate::seat::PointerOpType::Select {
                let dragged = (*server).wm.selection_release();
                // Released before the op ends, so its end re-evaluates the
                // pointer and the crosshair does not outlast the drag.
                cursor.pressed.remove(&(*event).button);
                seat.op_end();
                // A press that never travelled is a click on the desktop.
                // With windows selected it drops the selection, as a click
                // on empty grid does in cce-designer; with none it leaves
                // overview, as it always has.
                if !dragged && (*event).button == 0x110 {
                    if (*server).wm.has_selection() {
                        (*server).wm.selection_clear();
                    } else {
                        (*server).wm.execute_action(&crate::config::Action::Overview, None);
                    }
                }
                return;
            }

            #[allow(unused_assignments)]
            if !op.window_ptr.is_null()
                && (*op.window_ptr).is_status_bar()
                && op.op_type == crate::seat::PointerOpType::Move
                && (*event).button == 0x110
            {
                let win = op.window_ptr;
                let app_id = (*win).get_app_id_string().unwrap_or_default();

                // A press that never travelled is a CLICK, not a drag: end
                // the grab without snapping — snapping classifies the
                // release point alone, so a still click on a top-edge
                // segment away from the corners re-homed it to top-center
                // — and replay press+release to the segment, which never
                // saw the press. The grab is taken on press so drag
                // feedback is immediate; this is where the two are told
                // apart.
                const STATUS_CLICK_TRAVEL: f64 = 6.0;
                let travel = (lx - op.start_x as f64).hypot(ly - op.start_y as f64);
                if travel < STATUS_CLICK_TRAVEL {
                    log::info!("[StatusRelease] click (travel {:.1}px) on app_id={} — replayed, not snapped", travel, app_id);
                    seat.op_end();
                    cursor.pressed.remove(&(*event).button);
                    let time = (*event).time_msec;
                    // Re-evaluate pointer focus onto the surface under the
                    // pointer (nothing was notified during the grab), then
                    // deliver the click.
                    cursor.passthrough(time);
                    ffi::wlr_seat_pointer_notify_button(seat.wlr_seat, time, (*event).button, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED);
                    ffi::wlr_seat_pointer_notify_frame(seat.wlr_seat);
                    ffi::wlr_seat_pointer_notify_button(seat.wlr_seat, time, (*event).button, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_RELEASED);
                    ffi::wlr_seat_pointer_notify_frame(seat.wlr_seat);
                    crate::shared::pending().dirty_windowing();
                    return;
                }

                let mut closest_edge = crate::window::StatusEdge::TopLeft;
                let mut min_dist = f64::MAX;
                log::info!("[StatusRelease] Released status window: app_id={}, lx={}, ly={}", app_id, lx, ly);
                
                let outputs_list = &mut (*server).om.outputs as *mut ffi::wl_list as *mut WlList;
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

                    log::info!("[StatusRelease] Checking best output: box_geom=({}, {}, {}, {})", ox, oy, ow, oh);
                    
                    let dt = ly - oy;
                    let db = (oy + oh) - ly;
                    let dl = lx - ox;
                    let dr = (ox + ow) - lx;
                    
                    log::info!("[StatusRelease] Distances: top={}, bottom={}, left={}, right={}", dt, db, dl, dr);
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
                
                log::info!("[StatusRelease] Snapping app_id={} closest_edge={:?}, found_out={}", app_id, closest_edge, found_out);
                (*win).status_edge = closest_edge;
                
                let name = if let Some(stripped) = app_id.strip_prefix("cce-status-interface-left-").or_else(|| app_id.strip_prefix("cce-status-left-")) {
                    stripped
                } else if let Some(stripped) = app_id.strip_prefix("cce-status-interface-right-").or_else(|| app_id.strip_prefix("cce-status-right-")) {
                    stripped
                } else {
                    &app_id
                };
                let edge_str = match closest_edge {
                    crate::window::StatusEdge::Left => "left",
                    crate::window::StatusEdge::Right => "right",
                    crate::window::StatusEdge::TopLeft => "top-left",
                    crate::window::StatusEdge::TopCenter => "top-center",
                    crate::window::StatusEdge::TopRight => "top-right",
                    crate::window::StatusEdge::BottomLeft => "bottom-left",
                    crate::window::StatusEdge::BottomCenter => "bottom-center",
                    crate::window::StatusEdge::BottomRight => "bottom-right",
                    _ => "top-left",
                };
                let key_path = format!("layout.status_bar.{}", name);
                cce_core::config::write_config_value(
                    &cce_core::config::get_config_path().to_string_lossy(),
                    &key_path,
                    &format!("\"{}\"", edge_str),
                    "layout"
                );

                seat.op_end();
                cursor.pressed.remove(&(*event).button);
                crate::shared::pending().dirty_windowing();
                return;
            }
            if op.started_in_overview && (*event).button == 0x110 {
                let moved = (cursor_x as i32 - op.start_x).abs() > 5 || (cursor_y as i32 - op.start_y).abs() > 5;
                if !moved && !op.window_ptr.is_null() {
                    let win_ptr = op.window_ptr;
                    // Restore original tiling mode & lock status
                    (*win_ptr).tiling_mode = op.start_tiling_mode;
                    (*win_ptr).mode_locked = op.start_mode_locked;

                    let server = seat.server;
                    if !(*win_ptr).closed && !(*win_ptr).is_status_bar() && !(*win_ptr).is_wallpaper() {
                        // The Overview toggle's exit path: it centers on the
                        // HOVERED window (the cursor is on the clicked one),
                        // focuses it, and ANIMATES the camera home along the
                        // configured overview ramp — the same flight the
                        // background-click exit takes, instead of the
                        // instant cut this block used to hand-roll.
                        (*server).wm.execute_action(&crate::config::Action::Overview, None);
                    }
                }
            }
            
            seat.op_end();
            cursor.pressed.remove(&(*event).button);
            return;
        }
        if cursor.pressed.remove(&(*event).button) {
            if (*event).button == 0x110 {
                if let Some((win, elem)) = cursor.button_press.take() {
                    let alive = (*server).wm.windows.iter().any(|&w| w == win) && !(*win).closed;
                    if alive && get_border_zone(win, lx, ly) == BorderZone::Button(elem) {
                        (*server).wm.press_window_button(win, elem);
                    }
                    return;
                }
            }

            if (*event).button == 0x111 && (cursor.right_click_on_bg || cursor.right_click_on_border) {
                cursor.right_click_on_bg = false;
                cursor.right_click_on_border = false;
                if cursor.pressed.is_empty() && seat.op.is_some() {
                    crate::shared::pending().dirty_windowing();
                }
                return;
            }

            // The client-facing release (when the press was forwarded) is
            // already paired at the top of the release path.
            if cursor.pressed.is_empty() && seat.op.is_some() {
                crate::shared::pending().dirty_windowing();
            }
        } else {
            log::error!("ignoring duplicate pointer button {} release", (*event).button);
        }
    }
}
