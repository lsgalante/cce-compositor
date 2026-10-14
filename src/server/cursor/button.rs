//! `handle_button`: a pointer button press or release, through every grab, op,
//! border, popup, layer and window it can land on. Split out of cursor.rs on
//! 2026-10-10.

use super::*;

pub(crate) unsafe extern "C" fn handle_button(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let sid = listener_seat!(listener, cursor.button_listener);
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    on_button(wm, sid, &*(data as *const ffi::wlr_pointer_button_event), sys);
}

/// `handle_button` with the window manager passed in; `ccectl`'s injection calls this from inside `process_ipc_command`, with its `self`.
pub(crate) fn on_button(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, event: &ffi::wlr_pointer_button_event, sys: &mut crate::ctx::Sys) {
    if wm.seats.get(sid).is_none() {
        return;
    }
    crate::seat::Seat::handle_activity(wm, sid, sys);
    let cursor = cursor_mut!(wm, sid);
    if cursor.view_drag.is_some() {
        crate::cursor::Cursor::end_view_drag(wm, sid, "button");
    }
    // Before the button reaches the client, so a click on the popup is not
    // a Ctrl-click (see `PopupWheel`).
    let cursor = cursor_mut!(wm, sid);
    cursor.end_popup_wheel("button");
    cursor.end_hscroll_shift("button");
    
    let cursor = cursor_mut!(wm, sid);
    let lx = cursor.x();
    let ly = cursor.y();

    // First deliberate input ends the session-restore settling phase (see
    // the focus gate in Window::map).
    if event.state == ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED {
        wm.startup_input_seen = true;
    }

    let mut is_app_surface = false;
    let mut is_overlay_window = false;
    if let Some(result) = crate::shared::scene().at(wm, lx, ly) {
        match result.data {
            SceneNodeDataVal::Window(window_id) => {
                if let Some(window) = wm.windows.get(window_id) {
                    // The grid does not block: hitting it means the press is on a
                    // desktop item (its input region covers nothing else), and
                    // the item drag needs the press delivered in overview like
                    // any chrome click.
                    if !window.is_status_bar()
                        && !window.is_wallpaper()
                        && !window.is_grid()
                    {
                        is_app_surface = true;
                        // Popup counts as chrome like Overlay: the cce-cloud
                        // launcher must keep receiving clicks in overview.
                        if window.tiling_mode == crate::tiling::TilingMode::Overlay
                            || window.tiling_mode == crate::tiling::TilingMode::Popup
                        {
                            is_overlay_window = true;
                        }
                    }
                }
            }
            SceneNodeDataVal::LayerSurface(layer_surface) => {
                if !layer_surface.is_null() {
                    is_app_surface = true;
                    // SAFETY: a layer surface the hit test just found is live
                    // (its scene data dies with its tree).
                    if unsafe { is_cloud_layer(layer_surface) } {
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
    let should_block_button = crate::shared::mode() == crate::window_manager::WindowManagerMode::Overview && is_app_surface && !is_overlay_window;
    
    if event.state == ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED {
        let cursor = cursor_mut!(wm, sid);
        if cursor.pressed.contains(&event.button) {
            log::error!("ignoring duplicate pointer button {} press", event.button);
            return;
        }

        press_dismissals(wm, lx, ly);

        // Status-bar segments are dragged either in adjust-position mode or
        // directly with super+left-drag (0x40 = WLR_MODIFIER_LOGO).
        let super_held = seat_mut!(wm, sid).wlr_seat().keyboard_modifiers() & 0x40 != 0;
        if event.button == 0x110 && (wm.adjust_position_mode || super_held) {
            let mut clicked_status: Option<crate::window::WindowId> = None;
            if let Some(result) = crate::shared::scene().at(wm, lx, ly) {
                if let SceneNodeDataVal::Window(window_id) = result.data {
                    if wm.windows.get(window_id).is_some_and(|w| w.is_status_bar()) {
                        clicked_status = Some(window_id);
                    }
                }
            }
            // An EXPANDED segment (menu open) is never grabbed: its rows are
            // clicked, and a grab here would swallow the press the "Done"
            // row needs to leave adjust mode — the one control that ends the
            // mode from the bar would be unreachable while it is on.
            let grab = clicked_status
                .and_then(|id| wm.windows.get(id))
                .filter(|w| !wm.is_expanded_status_segment(w))
                .map(|w| (w.id(), w.box_geom, w.virtual_x, w.virtual_y, w.tiling_mode, w.mode_locked));
            if let Some((status_id, bg, vx, vy, mode, locked)) = grab {
                wm.stop_panning_animation();
                let cursor = cursor_mut!(wm, sid);
                let cursor_x = cursor.wlr_cursor().x();
                let cursor_y = cursor.wlr_cursor().y();
                let seat = seat_mut!(wm, sid);
                seat.op = Some(crate::seat::SeatOp {
                    input: crate::seat::SeatOpInput::Pointer,
                    start_x: cursor_x as i32,
                    start_y: cursor_y as i32,
                    x: cursor_x as i32,
                    y: cursor_y as i32,
                    window: Some(status_id),
                    op_type: crate::seat::PointerOpType::Move,
                    start_win_x: bg.x,
                    start_win_y: bg.y,
                    start_win_w: bg.width as u32,
                    start_win_h: bg.height as u32,
                    start_win_virtual_x: vx,
                    start_win_virtual_y: vy,
                    start_was_tiled: false,
                    start_pan_x: wm.desk_pan_x,
                    start_pan_y: wm.desk_pan_y,
                    start_tiling_mode: mode,
                    start_mode_locked: locked,
                    started_in_overview: false,
                });
                crate::cursor::Cursor::op_start_pointer(wm, sid);
                let cursor = cursor_mut!(wm, sid);
                cursor.pressed.insert(event.button);
                cursor.set_xcursor(c"grab");
                return;
            }
        }

        // --- ADJUST-MODE CLICK HANDLING (overview, or Super held) ---
        // A press on a window's body grabs the whole window to move it; a
        // press on its ring falls through to the border path. Only in
        // overview does a background press mean anything (it exits); with
        // Super held at zoom 1 it falls through to the normal desktop press.
        let in_overview = crate::shared::mode() == crate::window_manager::WindowManagerMode::Overview;
        if event.button == 0x110 && crate::shared::window_adjust_active() {
            let mut clicked_id: Option<crate::window::WindowId> = None;
            let mut clicked_cloud_layer = false;
            if let Some(result) = crate::shared::scene().at(wm, lx, ly) {
                match result.data {
                    SceneNodeDataVal::Window(window_id) => clicked_id = Some(window_id),
                    SceneNodeDataVal::LayerSurface(layer_surface) => {
                        // SAFETY: as above, a fresh hit.
                        clicked_cloud_layer = unsafe { is_cloud_layer(layer_surface) };
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
            let clicked = clicked_id.and_then(|id| wm.windows.get(id));
            let overview_chrome = clicked_cloud_layer
                || clicked.is_some_and(|w| {
                    matches!(
                        w.tiling_mode,
                        crate::tiling::TilingMode::Overlay | crate::tiling::TilingMode::Popup
                    )
                });
            // Hitting the grid means the press landed on a DESKTOP ITEM —
            // its input region covers the item rects and nothing else — so
            // it is chrome-like: fall through to normal delivery and the
            // grid client starts its item drag, in overview exactly as in
            // normal mode. Bare canvas misses the grid entirely (that is
            // the input region again) and still exits overview below.
            let clicked_grid = clicked.is_some_and(|w| w.is_grid());
            // The bare canvas counts as background in overview: a press on
            // it must exit overview like any desktop press, never grab the
            // canvas itself as if it were a window.
            let overview_win_valid = clicked.is_some_and(|w| !w.is_status_bar() && !w.is_wallpaper())
                && !clicked_grid;
            let stores = stores!(wm);
            let overview_border_zone = match clicked {
                Some(w) if overview_win_valid => get_border_zone(w, stores, lx, ly),
                _ => BorderZone::None,
            };
            // A window button is a click: nothing is grabbed, the client
            // never sees the press, and the release decides
            // (`Cursor::button_press`, answered in the release path).
            if let (BorderZone::Button(elem), Some(id)) = (overview_border_zone, clicked_id) {
                let cursor = cursor_mut!(wm, sid);
                cursor.button_press = Some((id, elem));
                cursor.pressed.insert(event.button);
                return;
            }
            // A press on a SELECTED desktop image grabs the whole selection
            // — windows and images — from the image's side: the grid never
            // sees the press, the compositor moves everything and tells the
            // grid where its images went (`crate::selection`). A press on an
            // unselected image drops the selection, as one on an unselected
            // window does, and goes to the grid as before.
            if clicked_grid && in_overview {
                let (vx, vy) = wm.layout_to_virtual(lx, ly);
                if wm.selected_item_at(vx, vy).is_some() {
                    wm.stop_panning_animation();
                    let seat = seat_mut!(wm, sid);
                    seat.group_move.clear();
                    seat.group_items.clear();
                    // The first Tiled window carried is the snap anchor.
                    let mut tiled_anchor: Option<(f64, f64)> = None;
                    let carried: Vec<crate::window::WindowId> = wm
                        .selection
                        .windows
                        .iter()
                        .copied()
                        .filter(|&id| wm.windows.get(id).is_some_and(|w| wm.selectable(w)))
                        .collect();
                    for id in carried {
                        let Some(w) = wm.windows.get(id) else { continue };
                        let (vx, vy, mode) = (w.virtual_x, w.virtual_y, w.tiling_mode);
                        let seat = seat_mut!(wm, sid);
                        seat.group_move.push((id, vx, vy));
                        if mode == crate::tiling::TilingMode::Tiled {
                            if tiled_anchor.is_none() {
                                tiled_anchor = Some((vx, vy));
                            }
                        } else if mode == crate::tiling::TilingMode::Floating {
                            wm.raise_window(id);
                        }
                    }
                    let ids: Vec<u64> = wm.selection.items.clone();
                    for id in ids {
                        if let Some((x, y)) = wm.desktop_item(id).map(|item| (item.x, item.y)) {
                            seat_mut!(wm, sid).group_items.push((id, x, y));
                        }
                    }
                    let cursor = cursor_mut!(wm, sid);
                    let cursor_x = cursor.wlr_cursor().x();
                    let cursor_y = cursor.wlr_cursor().y();
                    let (anchor_x, anchor_y) = tiled_anchor.unwrap_or((0.0, 0.0));
                    let seat = seat_mut!(wm, sid);
                    seat.op = Some(crate::seat::SeatOp {
                        input: crate::seat::SeatOpInput::Pointer,
                        start_x: cursor_x as i32,
                        start_y: cursor_y as i32,
                        x: cursor_x as i32,
                        y: cursor_y as i32,
                        window: None,
                        op_type: crate::seat::PointerOpType::GroupMove,
                        start_win_x: 0,
                        start_win_y: 0,
                        start_win_w: 0,
                        start_win_h: 0,
                        start_win_virtual_x: anchor_x,
                        start_win_virtual_y: anchor_y,
                        start_was_tiled: tiled_anchor.is_some(),
                        start_pan_x: wm.desk_pan_x,
                        start_pan_y: wm.desk_pan_y,
                        start_tiling_mode: crate::tiling::TilingMode::Floating,
                        start_mode_locked: false,
                        started_in_overview: true,
                    });
                    crate::cursor::Cursor::op_start_pointer(wm, sid);
                    let cursor = cursor_mut!(wm, sid);
                    cursor.pressed.insert(event.button);
                    cursor.set_xcursor(c"grab");
                    return;
                }
                wm.selection_clear();
            }
            if overview_chrome || clicked_grid {
                // fall through
            } else if overview_win_valid && matches!(overview_border_zone, BorderZone::None) {
                // `overview_win_valid` means the press hit a window.
                let Some(clicked_id) = clicked_id else { return };
                // The grab does NOT focus the window: moving a window is
                // not choosing it, and the ring already sits on it through
                // `adjust_hover`. A tap — press and release without motion
                // — is a click and focuses in `op_end`; a Floating window
                // is raised for the drag in `op_start_pointer`. (In
                // overview hover already focused it.)
                wm.stop_panning_animation();
                // A selected window carries the rest of the selection with
                // it; a press on any other window drops the selection, the
                // way a press on a node outside the region does in
                // cce-designer. The carried Floating windows are raised
                // here, in stacking order, and the grabbed one over them in
                // `op_start_pointer`.
                let seat = seat_mut!(wm, sid);
                seat.group_move.clear();
                seat.group_items.clear();
                if wm.is_selected(clicked_id) {
                    let carried: Vec<crate::window::WindowId> = wm
                        .selection
                        .windows
                        .iter()
                        .copied()
                        .filter(|&id| id != clicked_id && wm.windows.get(id).is_some_and(|w| wm.selectable(w)))
                        .collect();
                    for id in carried {
                        let Some(w) = wm.windows.get(id) else { continue };
                        let floating = w.tiling_mode == crate::tiling::TilingMode::Floating;
                        let seat = seat_mut!(wm, sid);
                        seat.group_move.push((id, w.virtual_x, w.virtual_y));
                        if floating {
                            wm.raise_window(id);
                        }
                    }
                    // And the selected desktop images, by the same offset.
                    let ids: Vec<u64> = wm.selection.items.clone();
                    for id in ids {
                        if let Some((x, y)) = wm.desktop_item(id).map(|item| (item.x, item.y)) {
                            seat_mut!(wm, sid).group_items.push((id, x, y));
                        }
                    }
                } else {
                    wm.selection_clear();
                }
                let cursor = cursor_mut!(wm, sid);
                let cursor_x = cursor.wlr_cursor().x();
                let cursor_y = cursor.wlr_cursor().y();
                let Some(w) = wm.windows.get(clicked_id) else { return };
                let (bg, vx, vy, mode, locked) = (w.box_geom, w.virtual_x, w.virtual_y, w.tiling_mode, w.mode_locked);
                let seat = seat_mut!(wm, sid);
                seat.op = Some(crate::seat::SeatOp {
                    input: crate::seat::SeatOpInput::Pointer,
                    start_x: cursor_x as i32,
                    start_y: cursor_y as i32,
                    x: cursor_x as i32,
                    y: cursor_y as i32,
                    window: Some(clicked_id),
                    op_type: crate::seat::PointerOpType::Move,
                    start_win_x: bg.x,
                    start_win_y: bg.y,
                    start_win_w: bg.width as u32,
                    start_win_h: bg.height as u32,
                    start_win_virtual_x: vx,
                    start_win_virtual_y: vy,
                    start_was_tiled: mode == crate::tiling::TilingMode::Tiled,
                    start_pan_x: wm.desk_pan_x,
                    start_pan_y: wm.desk_pan_y,
                    start_tiling_mode: mode,
                    start_mode_locked: locked,
                    started_in_overview: in_overview,
                });
                crate::cursor::Cursor::op_start_pointer(wm, sid);
                let cursor = cursor_mut!(wm, sid);
                cursor.pressed.insert(event.button);
                cursor.set_xcursor(c"grab");
                return;
            } else if !overview_win_valid && in_overview {
                // Click-away with a cce-cloud popup open (desktop context
                // menu, launcher): the press dismisses the popup and does
                // nothing else — overview stays up. Dropping keyboard focus
                // IS the dismissal: cce-cloud closes itself on keyboard
                // leave, the same signal a normal-mode click-away produces
                // through its focus change.
                let seat = seat_mut!(wm, sid);
                if let crate::layer_shell::LayerShellSeatFocus::Exclusive(key) =
                    seat.layer_shell.scheduled_focus
                {
                    if let Some(layer_surface) = wm.layer_surfaces.get(key) {
                        // SAFETY: an entry of the layer-surface store is live
                        // (`LayerSurfaces`).
                        if unsafe { is_cloud_layer(layer_surface) } {
                            crate::seat::Seat::focus_window(wm, sid, None);
                            let cursor = cursor_mut!(wm, sid);
                            cursor.pressed.insert(event.button);
                            return;
                        }
                    }
                }
                // A press on the bare desktop is a click (which leaves
                // overview) or the start of a drag-selection, and only the
                // release can say which: it arms the selection, and the
                // release path answers a press that never travelled.
                wm.stop_panning_animation();
                wm.selection_press(lx, ly);
                let seat = seat_mut!(wm, sid);
                seat.group_move.clear();
                let cursor = cursor_mut!(wm, sid);
                let cursor_x = cursor.wlr_cursor().x();
                let cursor_y = cursor.wlr_cursor().y();
                let seat = seat_mut!(wm, sid);
                seat.op = Some(crate::seat::SeatOp {
                    input: crate::seat::SeatOpInput::Pointer,
                    start_x: cursor_x as i32,
                    start_y: cursor_y as i32,
                    x: cursor_x as i32,
                    y: cursor_y as i32,
                    window: None,
                    op_type: crate::seat::PointerOpType::Select,
                    start_win_x: 0,
                    start_win_y: 0,
                    start_win_w: 0,
                    start_win_h: 0,
                    start_win_virtual_x: 0.0,
                    start_win_virtual_y: 0.0,
                    start_was_tiled: false,
                    start_pan_x: wm.desk_pan_x,
                    start_pan_y: wm.desk_pan_y,
                    start_tiling_mode: crate::tiling::TilingMode::Floating,
                    start_mode_locked: false,
                    started_in_overview: true,
                });
                crate::cursor::Cursor::op_start_pointer(wm, sid);
                let cursor = cursor_mut!(wm, sid);
                cursor.pressed.insert(event.button);
                cursor.set_xcursor(c"crosshair");
                return;
            }
        }

        let seat = seat_mut!(wm, sid);
        let modifiers = seat.wlr_seat().keyboard_modifiers();

        if event.button == 0x111 && modifiers == 0 {
            let mut clicked_interactive = false;
            if let Some(result) = crate::shared::scene().at(wm, lx, ly) {
                match result.data {
                    SceneNodeDataVal::Window(window_id) => {
                        if let Some(window) = wm.windows.get(window_id) {
                            if !window.is_wallpaper() {
                                clicked_interactive = true;
                            }
                        }
                    }
                    SceneNodeDataVal::LayerSurface(_) | SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                        clicked_interactive = true;
                    }
                }
            }
            if !clicked_interactive {
                let cursor = cursor_mut!(wm, sid);
                cursor.right_click_on_bg = true;
                let (cx, cy) = (cursor.x(), cursor.y());
                // The menu's "Window Mode" page acts on the FOCUSED window,
                // and focus is dropped right below (so the menu takes the
                // keyboard and a click-away dismisses it) — by the time the
                // menu could ask `ccectl windows` nothing is focused. Hand it
                // the window here instead. Only a window whose mode
                // `set-mode` accepts is worth naming.
                let seat = seat_mut!(wm, sid);
                let focused = match seat.focused {
                    crate::seat::Focus::Window(id) => wm.windows.get(id),
                    _ => None,
                };
                let menu_window = focused.and_then(|w| {
                    let mode = match w.tiling_mode {
                        crate::tiling::TilingMode::Floating => "floating",
                        crate::tiling::TilingMode::Tiled => "tiled",
                        crate::tiling::TilingMode::Fullscreen => "fullscreen",
                        _ => return None,
                    };
                    Some(cce_core::ipc::ctl::MenuWindow {
                        id: w.id().ipc(),
                        mode: mode.to_string(),
                        app_id: w.get_app_id_string().unwrap_or_else(|| "unknown".to_string()),
                    })
                });
                // The desktop client (cce-grid) opens the menu itself when it
                // listens: in process, at the press, on the press's output.
                // Without it, the `cce-desktop-menu` script does, through
                // cce-cloud.
                let topic = crate::status_server::MenuTopic::Desktop;
                if let Some(sender) = wm.status_sender.as_ref().filter(|s| s.has_menu(topic)) {
                    sender.send_menu(topic, &menu_request(wm, cx, cy, menu_window).to_string());
                } else {
                    let home = std::env::var("HOME").unwrap_or_default();
                    let mut cmd = format!("{}/.local/bin/cce-desktop-menu -x {} -y {}", home, cx as i32, cy as i32);
                    if let Some(w) = menu_window {
                        cmd.push_str(&format!(" -i {} -a '{}'", w.id, w.app_id.replace('\'', "'\\''")));
                    }
                    wm.execute_action(&crate::config::Action::Spawn, Some(&cmd));
                }

                crate::seat::Seat::focus_window(wm, sid, None);
                crate::shared::pending().dirty_windowing();

                let cursor = cursor_mut!(wm, sid);
                cursor.pressed.insert(event.button);
                return;
            }
        }
        
        let mut matched_pb: Option<crate::config::PointerBind> = None;
        for pb in &crate::shared::layout().pointer_binds {
            if pb.button == event.button && pb.mods == modifiers {
                matched_pb = Some(pb.clone());
                break;
            }
        }
        
        if let Some(pb) = matched_pb {
            let cursor = cursor_mut!(wm, sid);
            let lx = cursor.x();
            let ly = cursor.y();
            let mut target_id: Option<crate::window::WindowId> = None;
            if let Some(result) = crate::shared::scene().at(wm, lx, ly) {
                if let SceneNodeDataVal::Window(window_id) = result.data {
                    target_id = Some(window_id);
                }
            }

            let target_id = target_id.filter(|&id| wm.windows.get(id).is_some_and(|w| !w.is_status_bar() && !w.is_wallpaper()));
            if let Some(target_id) = target_id {
                let id = target_id;
                let this = window_mut!(wm, id);
                // Before the un-tile below: a tiled window's drag snaps hard
                // to whole squares, and by op_update the mode reads Floating.
                let grabbed_tiled =
                    this.tiling_mode == crate::tiling::TilingMode::Tiled;
                if this.tiling_mode != crate::tiling::TilingMode::Floating
                    && this.tiling_mode != crate::tiling::TilingMode::Popup
                    && this.tiling_mode != crate::tiling::TilingMode::Fullscreen
                    && this.tiling_mode != crate::tiling::TilingMode::Overlay
                    // A drag moves a Utility window; it must not re-class it.
                    && this.tiling_mode != crate::tiling::TilingMode::Utility
                {
                    // A tiled window un-tiles for the drag but keeps its
                    // cell-quantized geometry; landing grid-aligned re-tiles
                    // it (op_end geometric detection).
                    this.was_tiled = false;
                    this.tiling_mode = crate::tiling::TilingMode::Floating;
                    this.mode_locked = true;
                }
                // No focus on the grab: a bound move/resize drag acts on
                // the window under the pointer without choosing it (a tap
                // focuses in `op_end`, the raise is in `op_start_pointer`).
                let op_type = match pb.action {
                    crate::config::Action::Move => Some(crate::seat::PointerOpType::Move),
                    // The modifier binding is a resize path the border zones
                    // never see, so it carries its own Utility rejection.
                    crate::config::Action::Resize
                        if this.tiling_mode == crate::tiling::TilingMode::Utility =>
                    {
                        None
                    }
                    crate::config::Action::Resize => {
                        let edges = get_closest_edges(window_ref!(wm, id), lx, ly);
                        Some(crate::seat::PointerOpType::Resize { edges })
                    }
                    _ => None,
                };
                
                if let Some(ot) = op_type {
                    wm.stop_panning_animation();
                    let cursor = cursor_mut!(wm, sid);
                    let cursor_x = cursor.wlr_cursor().x();
                    let cursor_y = cursor.wlr_cursor().y();
                    let this = window_mut!(wm, id);
                    let seat = seat_mut!(wm, sid);
                    seat.op = Some(crate::seat::SeatOp {
                        input: crate::seat::SeatOpInput::Pointer,
                        start_x: cursor_x as i32,
                        start_y: cursor_y as i32,
                        x: cursor_x as i32,
                        y: cursor_y as i32,
                        window: Some(id),
                        op_type: ot,
                        start_win_x: this.box_geom.x,
                        start_win_y: this.box_geom.y,
                        start_win_w: this.box_geom.width as u32,
                        start_win_h: this.box_geom.height as u32,
                        start_win_virtual_x: this.virtual_x,
                        start_win_virtual_y: this.virtual_y,
                        start_tiling_mode: this.tiling_mode,
                        start_was_tiled: grabbed_tiled,
                        start_mode_locked: this.mode_locked,
                        start_pan_x: wm.desk_pan_x,
                        start_pan_y: wm.desk_pan_y,
                        started_in_overview: crate::shared::mode() == crate::window_manager::WindowManagerMode::Overview,
                    });
                    crate::cursor::Cursor::op_start_pointer(wm, sid);
                    let cursor = cursor_mut!(wm, sid);
                    cursor.pressed.insert(event.button);

                    match ot {
                        crate::seat::PointerOpType::Resize { edges } => {
                            let cursor_name = get_resize_cursor_name(edges);
                            cursor.set_xcursor(cursor_name);
                        }
                        crate::seat::PointerOpType::Move => {
                            cursor.set_xcursor(c"grab");
                        }
                        // A pointer bind is a move or a resize.
                        crate::seat::PointerOpType::Select
                        | crate::seat::PointerOpType::GroupMove => {}
                    }
                    return;
                }
            }
        }

        let cursor = cursor_mut!(wm, sid);
        let lx = cursor.x();
        let ly = cursor.y();
        let mut border_id: Option<crate::window::WindowId> = None;
        if let Some(result) = crate::shared::scene().at(wm, lx, ly) {
            if let SceneNodeDataVal::Window(window_id) = result.data {
                border_id = Some(window_id);
            }
        }

        let border_id = border_id.filter(|&id| {
            wm.windows.get(id).is_some_and(|w| {
                !w.is_status_bar()
                    && !w.is_wallpaper()
                    && !w.is_grid()
                    && w.tiling_mode != crate::tiling::TilingMode::Popup
                    && w.tiling_mode != crate::tiling::TilingMode::Fullscreen
            })
        });
        if let Some(border_id) = border_id {
            let id = border_id;
            let this = window_mut!(wm, id);
            let initial_mode = this.tiling_mode;
            let stores = stores!(wm);
            let zone = get_border_zone(window_ref!(wm, id), stores, lx, ly);
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
            if event.button == 0x111
                && modifiers == 0
                && (!matches!(zone, BorderZone::None) || menu_on_body)
            {
                let cursor = cursor_mut!(wm, sid);
                cursor.right_click_on_border = true;
                let (cx, cy) = (cursor.x(), cursor.y());
                let this = window_mut!(wm, id);
                let clicked = cce_core::ipc::ctl::MenuWindow {
                    id: this.id().ipc(),
                    mode: this.tiling_mode.as_str().to_lowercase(),
                    app_id: this.get_app_id_string().unwrap_or_else(|| "unknown".to_string()),
                };
                // cce-grid draws the window's menu in process when it
                // listens (`window-menu`); the cce-app-menu script, a
                // cce-cloud popup, otherwise.
                let topic = crate::status_server::MenuTopic::Window;
                if let Some(sender) = wm.status_sender.as_ref().filter(|s| s.has_menu(topic)) {
                    sender.send_menu(topic, &menu_request(wm, cx, cy, Some(clicked)).to_string());
                } else {
                    // The command runs under `sh -c`: quote the app_id, which
                    // is client-chosen text.
                    let app_id = format!("'{}'", clicked.app_id.replace('\'', "'\\''"));
                    let home = std::env::var("HOME").unwrap_or_default();
                    let cmd = format!(
                        "{}/.local/bin/cce-app-menu -x {} -y {} -i {} -a {}",
                        home, cx as i32, cy as i32, clicked.id, app_id
                    );
                    wm.execute_action(&crate::config::Action::Spawn, Some(&cmd));
                }

                let cursor = cursor_mut!(wm, sid);
                cursor.pressed.insert(event.button);
                return;
            }

            match zone {
                BorderZone::Resize(edges) => {
                    if event.button == 0x110 { // BTN_LEFT
                        // Unreachable for Utility (get_border_zone maps its
                        // whole band to Move), and gated here regardless.
                        if initial_mode == crate::tiling::TilingMode::Utility {
                            return;
                        }
                        if initial_mode != crate::tiling::TilingMode::Floating {
                            // A tiled window un-tiles for the drag but keeps its
                            // cell-quantized geometry; landing grid-aligned re-tiles
                            // it (op_end geometric detection).
                            let this = window_mut!(wm, id);
                            this.was_tiled = false;
                            this.tiling_mode = crate::tiling::TilingMode::Floating;
                            this.mode_locked = true;
                        }

                        // No focus on the grab (see the body grab above).
                        wm.stop_panning_animation();
                        let cursor = cursor_mut!(wm, sid);
                        let cursor_x = cursor.wlr_cursor().x();
                        let cursor_y = cursor.wlr_cursor().y();
                        let this = window_mut!(wm, id);
                        let seat = seat_mut!(wm, sid);
                        seat.op = Some(crate::seat::SeatOp {
                            input: crate::seat::SeatOpInput::Pointer,
                            start_x: cursor_x as i32,
                            start_y: cursor_y as i32,
                            x: cursor_x as i32,
                            y: cursor_y as i32,
                            window: Some(id),
                            op_type: crate::seat::PointerOpType::Resize { edges },
                            start_win_x: this.box_geom.x,
                            start_win_y: this.box_geom.y,
                            start_win_w: this.box_geom.width as u32,
                            start_win_h: this.box_geom.height as u32,
                            start_win_virtual_x: this.virtual_x,
                            start_win_virtual_y: this.virtual_y,
                            start_tiling_mode: this.tiling_mode,
                            start_was_tiled: initial_mode == crate::tiling::TilingMode::Tiled,
                            start_mode_locked: this.mode_locked,
                            start_pan_x: wm.desk_pan_x,
                        start_pan_y: wm.desk_pan_y,
                        started_in_overview: crate::shared::mode() == crate::window_manager::WindowManagerMode::Overview,
                        });
                        crate::cursor::Cursor::op_start_pointer(wm, sid);
                        let cursor = cursor_mut!(wm, sid);
                        cursor.pressed.insert(event.button);

                        let cursor_name = get_resize_cursor_name(edges);
                        cursor.set_xcursor(cursor_name);
                        return;
                    }
                }
                BorderZone::Move => {
                    if event.button == 0x110 { // BTN_LEFT
                        let current_time = event.time_msec;
                        let this = window_mut!(wm, id);
                        let is_titlebar = ly < this.box_geom.y as f64;
                        let cursor = cursor_mut!(wm, sid);
                        let is_double_click = is_titlebar
                            && Some(id) == cursor.last_click_window
                            && current_time.saturating_sub(cursor.last_click_time) < 300;

                        let cursor = cursor_mut!(wm, sid);
                        cursor.last_click_time = current_time;
                        cursor.last_click_window = Some(id);

                        // A Utility window has no Tiled state to toggle;
                        // the double-click falls through to an ordinary move.
                        if is_double_click && initial_mode != crate::tiling::TilingMode::Utility {
                            if initial_mode == crate::tiling::TilingMode::Tiled {
                                this.tiling_mode = crate::tiling::TilingMode::Floating;
                                this.mode_locked = true;
                            } else {
                                this.tiling_mode = crate::tiling::TilingMode::Tiled;
                                this.mode_locked = true;
                            }
                            crate::seat::Seat::focus_window(wm, sid, Some(id));
                            crate::shared::pending().dirty_windowing();
                            let cursor = cursor_mut!(wm, sid);
                            cursor.last_click_time = 0;
                            cursor.last_click_window = None;
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
                            this.was_tiled = false;
                            this.tiling_mode = crate::tiling::TilingMode::Floating;
                            this.mode_locked = true;
                        }

                        // No focus on the grab (see the body grab above).
                        wm.stop_panning_animation();
                        let cursor = cursor_mut!(wm, sid);
                        let cursor_x = cursor.wlr_cursor().x();
                        let cursor_y = cursor.wlr_cursor().y();
                        let this = window_mut!(wm, id);
                        let seat = seat_mut!(wm, sid);
                        seat.op = Some(crate::seat::SeatOp {
                            input: crate::seat::SeatOpInput::Pointer,
                            start_x: cursor_x as i32,
                            start_y: cursor_y as i32,
                            x: cursor_x as i32,
                            y: cursor_y as i32,
                            window: Some(id),
                            op_type: crate::seat::PointerOpType::Move,
                            start_win_x: this.box_geom.x,
                            start_win_y: this.box_geom.y,
                            start_win_w: this.box_geom.width as u32,
                            start_win_h: this.box_geom.height as u32,
                            start_win_virtual_x: this.virtual_x,
                            start_win_virtual_y: this.virtual_y,
                            start_tiling_mode: this.tiling_mode,
                            start_was_tiled: initial_mode == crate::tiling::TilingMode::Tiled,
                            start_mode_locked: this.mode_locked,
                            start_pan_x: wm.desk_pan_x,
                        start_pan_y: wm.desk_pan_y,
                        started_in_overview: crate::shared::mode() == crate::window_manager::WindowManagerMode::Overview,
                        });
                        crate::cursor::Cursor::op_start_pointer(wm, sid);
                        let cursor = cursor_mut!(wm, sid);
                        cursor.pressed.insert(event.button);

                        cursor.set_xcursor(c"grab");
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

        let cursor = cursor_mut!(wm, sid);
        cursor.pressed.insert(event.button);

        if !should_block_button {
            let first_grab_button = cursor.notified_pressed.is_empty();
            let seat = seat_mut!(wm, sid);
            seat.wlr_seat().pointer_notify_button(event.time_msec, event.button, event.state);
            let cursor = cursor_mut!(wm, sid);
            cursor.notified_pressed.insert(event.button);
            // First grab button: record the pressed surface's layout origin
            // as the implicit grab's frame of reference (passthrough keeps
            // motion surface-relative through it while the button is held).
            if first_grab_button {
                let glx = cursor.x();
                let gly = cursor.y();
                if let Some(result) = crate::shared::scene().at(wm, glx, gly) {
                    let ratio = result.surface_scale;
                    let cursor = cursor_mut!(wm, sid);
                    cursor.grab_scale = ratio;
                    cursor.grab_origin = (glx - result.sx / ratio, gly - result.sy / ratio);
                }
                // A grab that starts on the grid freezes its node mapping
                // here instead of using grab_origin: passthrough maps motion
                // against these values for the whole grab, so camera motion
                // (which moves the node every frame) reads as nothing rather
                // than as pointer motion. See the grab branch in passthrough.
                let cursor = cursor_mut!(wm, sid);
                cursor.grab_grid = None;
                let seat = seat_mut!(wm, sid);
                let grab_focused =
                    seat.wlr_seat().pointer_focused_surface();
                if !grab_focused.is_null() {
                    if let Some((gsurf, nx, ny, scale, _, _)) = grid_node_info(wm) {
                        if gsurf == grab_focused {
                            let cursor = cursor_mut!(wm, sid);
                            cursor.grab_grid = Some((nx, ny, scale));
                        }
                    }
                }
            }
        }

        // If pressed, update focus to window under cursor
        let cursor = cursor_mut!(wm, sid);
        let lx = cursor.x();
        let ly = cursor.y();
        let mut clicked_something = false;
        if let Some(result) = crate::shared::scene().at(wm, lx, ly) {
            match result.data {
                SceneNodeDataVal::Window(window_id) => {
                    if let Some(window) = wm.windows.get(window_id) {
                        clicked_something = true;
                        if !window.is_status_bar() && !window.is_wallpaper() {
                            crate::seat::Seat::focus_window(wm, sid, Some(window_id));
                        }
                    }
                }
                SceneNodeDataVal::LayerSurface(layer_surface) => {
                    clicked_something = true;
                    // SAFETY: a layer surface the hit test just found is live,
                    // and so is its root wl_surface; focusing it is
                    // `Seat::focus`'s contract (a live layer root).
                    unsafe {
                        if !layer_surface.is_null() && (*layer_surface).takes_click_focus() {
                            // The layer's ROOT surface, not the hit one: a click
                            // on its popup or subsurface would otherwise leave the
                            // seat naming a surface the layer's unmap never clears
                            // (it compares the root), freed with the popup.
                            let root = (*layer_surface).wlr_layer_surface().surface();
                            crate::seat::Seat::focus(wm, sid, Focus::LayerSurface(root));
                        }
                    }
                }
                SceneNodeDataVal::LockSurface(_) | SceneNodeDataVal::OverrideRedirect(_) => {
                    clicked_something = true;
                }
            }
        }

        if !clicked_something && event.button == 0x110 {
            // Restore placeholders are bare scene rects the hit-test can't
            // see, but they stand in for restored windows — clicking one
            // gets the same camera rules as clicking the real window.
            if let Some((pvx, pvy, pw, ph)) = wm.placeholder_at(lx, ly) {
                wm.pan_to_virtual_rect(pvx, pvy, pw, ph);
            } else {
                crate::seat::Seat::focus_window(wm, sid, None);
                crate::shared::pending().dirty_windowing();
            }
        }
    } else {
        assert_eq!(event.state, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_RELEASED);
        // Pair the release for the client BEFORE any compositor-side
        // consumption below: if the press was forwarded, the release always
        // is too, or the client is left with an orphaned press.
        let cursor = cursor_mut!(wm, sid);
        if cursor.notified_pressed.remove(&event.button) {
            let seat = seat_mut!(wm, sid);
            seat.wlr_seat().pointer_notify_button(event.time_msec, event.button, event.state);
        }
        // The implicit grab ends with its last button; the frozen grid
        // mapping must not outlive it into the next grab.
        let cursor = cursor_mut!(wm, sid);
        if cursor.notified_pressed.is_empty() {
            cursor.grab_grid = None;
        }
        let seat = seat_mut!(wm, sid);
        if seat.op.is_some() {
            let cursor = cursor_mut!(wm, sid);
            let cursor_x = cursor.wlr_cursor().x();
            let cursor_y = cursor.wlr_cursor().y();
            crate::seat::Seat::op_update(wm, sid, cursor_x as i32, cursor_y as i32);
            
            let seat = seat_mut!(wm, sid);
            let op = seat.op.unwrap();

            if op.op_type == crate::seat::PointerOpType::Select {
                let dragged = wm.selection_release();
                // Released before the op ends, so its end re-evaluates the
                // pointer and the crosshair does not outlast the drag.
                let cursor = cursor_mut!(wm, sid);
                cursor.pressed.remove(&event.button);
                crate::seat::Seat::op_end(wm, sid);
                // A press that never travelled is a click on the desktop.
                // With windows selected it drops the selection, as a click
                // on empty grid does in cce-designer; with none it leaves
                // overview, as it always has.
                if !dragged && event.button == 0x110 {
                    if wm.has_selection() {
                        wm.selection_clear();
                    } else {
                        wm.execute_action(&crate::config::Action::Overview, None);
                    }
                }
                return;
            }

            let status_id = op
                .window
                .filter(|&id| wm.windows.get(id).is_some_and(|w| w.is_status_bar()))
                .filter(|_| op.op_type == crate::seat::PointerOpType::Move && event.button == 0x110);
            #[allow(unused_assignments)]
            if let Some(status_id) = status_id {
                let id = status_id;
                let this = window_mut!(wm, id);
                let app_id = this.get_app_id_string().unwrap_or_default();

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
                    crate::seat::Seat::op_end(wm, sid);
                    let cursor = cursor_mut!(wm, sid);
                    cursor.pressed.remove(&event.button);
                    let time = event.time_msec;
                    // Re-evaluate pointer focus onto the surface under the
                    // pointer (nothing was notified during the grab), then
                    // deliver the click.
                    crate::cursor::Cursor::passthrough(wm, sid, time);
                    let seat = seat_mut!(wm, sid);
                    seat.wlr_seat().pointer_notify_button(time, event.button, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_PRESSED);
                    seat.wlr_seat().pointer_notify_frame();
                    seat.wlr_seat().pointer_notify_button(time, event.button, ffi::wl_pointer_button_state_WL_POINTER_BUTTON_STATE_RELEASED);
                    seat.wlr_seat().pointer_notify_frame();
                    crate::shared::pending().dirty_windowing();
                    return;
                }

                let mut closest_edge = crate::window::StatusEdge::TopLeft;
                let mut min_dist = f64::MAX;
                log::info!("[StatusRelease] Released status window: app_id={}, lx={}, ly={}", app_id, lx, ly);
                
                let mut best_box: Option<ffi::wlr_box> = None;
                let mut min_output_dist = f64::MAX;
                
                for output in wm.outputs.values() {
                    if output.sent.state == crate::output::OutputStateValue::Enabled {
                        let wlr_box = output.sent.box_layout();
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
                            best_box = Some(wlr_box);
                        }
                    }
                }

                let mut found_out = false;
                if let Some(wlr_box) = best_box {
                    found_out = true;
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
                this.status_edge = closest_edge;
                
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

                crate::seat::Seat::op_end(wm, sid);
                let cursor = cursor_mut!(wm, sid);
                cursor.pressed.remove(&event.button);
                crate::shared::pending().dirty_windowing();
                return;
            }
            if op.started_in_overview && event.button == 0x110 {
                let moved = (cursor_x as i32 - op.start_x).abs() > 5 || (cursor_y as i32 - op.start_y).abs() > 5;
                let op_win = op.window.and_then(|id| wm.windows.get_mut(id));
                if let Some(win) = op_win.filter(|_| !moved) {
                    // Restore original tiling mode & lock status
                    win.tiling_mode = op.start_tiling_mode;
                    win.mode_locked = op.start_mode_locked;

                    if !win.closed && !win.is_status_bar() && !win.is_wallpaper() {
                        // The Overview toggle's exit path: it centers on the
                        // HOVERED window (the cursor is on the clicked one),
                        // focuses it, and ANIMATES the camera home along the
                        // configured overview ramp — the same flight the
                        // background-click exit takes, instead of the
                        // instant cut this block used to hand-roll.
                        wm.execute_action(&crate::config::Action::Overview, None);
                    }
                }
            }
            
            crate::seat::Seat::op_end(wm, sid);
            let cursor = cursor_mut!(wm, sid);
            cursor.pressed.remove(&event.button);
            return;
        }
        let cursor = cursor_mut!(wm, sid);
        if cursor.pressed.remove(&event.button) {
            if event.button == 0x110 {
                if let Some((id, elem)) = cursor.button_press.take() {
                    let stores = stores!(wm);
                    let still_on_it = wm
                        .windows
                        .get(id)
                        .is_some_and(|w| !w.closed && get_border_zone(w, stores, lx, ly) == BorderZone::Button(elem));
                    if still_on_it {
                        wm.press_window_button(id, elem);
                    }
                    return;
                }
            }

            if event.button == 0x111 && (cursor.right_click_on_bg || cursor.right_click_on_border) {
                cursor.right_click_on_bg = false;
                cursor.right_click_on_border = false;
                if seat_mut!(wm, sid).cursor.pressed.is_empty() && seat_mut!(wm, sid).op.is_some() {
                    crate::shared::pending().dirty_windowing();
                }
                return;
            }

            // The client-facing release (when the press was forwarded) is
            // already paired at the top of the release path.
            if seat_mut!(wm, sid).cursor.pressed.is_empty() && seat_mut!(wm, sid).op.is_some() {
                crate::shared::pending().dirty_windowing();
            }
        } else {
            log::error!("ignoring duplicate pointer button {} release", event.button);
        }
    }
}

/// The request a right-click at layout `(lx, ly)` hands the desktop client
/// (`desktop-menu` / `window-menu`): the point on its output — where a menu
/// on that output's surface opens — in layout coordinates and on the desk,
/// and the window it is about.
fn menu_request(
    wm: &crate::window_manager::WindowManager,
    lx: f64,
    ly: f64,
    window: Option<cce_core::ipc::ctl::MenuWindow>,
) -> cce_core::ipc::ctl::DesktopMenuRequest {
    let probe = ffi::wlr_box { x: lx.floor() as i32, y: ly.floor() as i32, width: 1, height: 1 };
    let (ox, oy) = wm.outputs.max_overlap_output(&probe).map_or((0, 0), |o| {
        let b = o.wlr_output().layout_box();
        (b.x, b.y)
    });
    let (vx, vy) = wm.layout_to_virtual(lx, ly);
    cce_core::ipc::ctl::DesktopMenuRequest { x: lx - ox as f64, y: ly - oy as f64, lx, ly, vx, vy, window }
}
