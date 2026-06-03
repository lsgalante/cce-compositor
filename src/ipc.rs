// IPC command parser for ccec
// Ported from handle_ipc_command in ccec.c

use crate::config::{parse_keysym, spawn_command_bg};
use crate::types::{
    parse_action, parse_button, parse_hex_color, parse_modifiers, parse_tiling_mode, Action,
    ModeRule, PendingPointerBinding, PendingXkbBinding, WindowManager, NUM_TAGS, TilingMode,
};

fn get_window_under_pointer(state: &WindowManager) -> Option<u64> {
    // 1. Check seat.hovered_window_id first
    for seat in &state.seats {
        if !seat.removed {
            if let Some(wid) = seat.hovered_window_id {
                return Some(wid);
            }
        }
    }

    // 2. Fallback: find it using POINTER_X/POINTER_Y and window geometries
    let px = crate::input::POINTER_X.load(std::sync::atomic::Ordering::SeqCst) as f64;
    let py = crate::input::POINTER_Y.load(std::sync::atomic::Ordering::SeqCst) as f64;

    let active_tags = state.active_tags;
    let mut best_wid = None;

    for win in &state.windows {
        if win.closed
            || (win.tags & active_tags) == 0
            || win.app_id.as_deref() == Some("clear-status-interface")
            || win.tiling_mode == TilingMode::Popup
        {
            continue;
        }
        let wx = win.anim_x.unwrap_or(win.x as f64);
        let wy = win.anim_y.unwrap_or(win.y as f64);
        let ww = win.anim_w.unwrap_or(win.width as f64);
        let wh = win.anim_h.unwrap_or(win.height as f64);

        if px >= wx && px <= wx + ww && py >= wy && py <= wy + wh {
            best_wid = Some(win.id);
        }
    }

    best_wid
}

/// Handle an IPC command string, modifying the window manager state.
/// This is called from the Unix socket listener when clearctl sends a command.
pub fn handle_ipc_command(cmd: &str, state: &mut WindowManager) -> String {
    // Strip trailing newlines/spaces
    let cmd = cmd.trim_end_matches(|c| c == '\n' || c == '\r' || c == ' ');
    if cmd.is_empty() {
        return "\n".to_string();
    }

    // Split the command into tokens
    let tokens: Vec<&str> = cmd.splitn(2, ' ').collect();
    let tok = tokens[0];
    let rest = if tokens.len() > 1 { tokens[1] } else { "" };

    let mut reply = "ok\n".to_string();

    match tok {
        "spawn" => {
            if !rest.is_empty() {
                spawn_command_bg(rest);
            }
        }
        "close" => {
            // Close the focused window
            if let Some(seat) = state.seats.first() {
                if let Some(focused_id) = seat.focused_window_id {
                    if let Some(window) = state.get_window_mut(focused_id) {
                        window.closed = true;
                    }
                }
            }
            state.needs_render = true;
        }
        "minimize" => {
            if let Some(seat) = state.seats.first() {
                if let Some(focused_id) = seat.focused_window_id {
                    if let Some(window) = state.get_window_mut(focused_id) {
                        window.minimized = true;
                    }
                    // Shift focus to the next visible window
                    let active_tags = state.active_tags;
                    let visible_ids: Vec<u64> = state
                        .windows
                        .iter()
                        .filter(|w| (w.tags & active_tags) != 0 && !w.closed && !w.minimized && w.app_id.as_deref() != Some("clear-status-interface"))
                        .map(|w| w.id)
                        .collect();
                    let next_id = visible_ids.last().copied();
                    for s in &mut state.seats {
                        if !s.removed {
                            s.focused_window_id = next_id;
                        }
                    }
                }
            }
            state.needs_render = true;
            state.needs_focus = true;
            state.needs_status_update = true;
        }
        "focus-next" => {
            // Focus the next visible window (wrapping) and move it to the
            // front of the cascade stack (end of windows vector).
            if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                let focused_id = seat.focused_window_id;
                let active_tags = state.active_tags;
                let visible_ids: Vec<u64> = state
                    .windows
                    .iter()
                    .filter(|w| (w.tags & active_tags) != 0 && !w.closed && !w.minimized && w.app_id.as_deref() != Some("clear-status-interface"))
                    .map(|w| w.id)
                    .collect();
                if visible_ids.len() > 1 {
                    if let Some(fid) = focused_id {
                        if let Some(idx) = visible_ids.iter().position(|id| *id == fid) {
                            let next_idx = (idx + 1) % visible_ids.len();
                            let next_id = visible_ids[next_idx];
                            seat.focused_window_id = Some(next_id);
                            // Move newly focused window to front of cascade stack
                            state.move_window_to_end(next_id);
                        }
                    }
                }
            }
            state.needs_render = true;
            state.needs_focus = true;
            state.needs_status_update = true;
        }
        "focus-prev" => {
            // Focus the previous visible window (wrapping) and move it to the
            // front of the cascade stack (end of windows vector).
            if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                let focused_id = seat.focused_window_id;
                let active_tags = state.active_tags;
                let visible_ids: Vec<u64> = state
                    .windows
                    .iter()
                    .filter(|w| (w.tags & active_tags) != 0 && !w.closed && !w.minimized && w.app_id.as_deref() != Some("clear-status-interface"))
                    .map(|w| w.id)
                    .collect();
                if visible_ids.len() > 1 {
                    if let Some(fid) = focused_id {
                        if let Some(idx) = visible_ids.iter().position(|id| *id == fid) {
                            let prev_idx = if idx == 0 { visible_ids.len() - 1 } else { idx - 1 };
                            let prev_id = visible_ids[prev_idx];
                            seat.focused_window_id = Some(prev_id);
                            // Move newly focused window to front of cascade stack
                            state.move_window_to_end(prev_id);
                        }
                    }
                }
            }
            state.needs_render = true;
            state.needs_focus = true;
            state.needs_status_update = true;
        }
        "exit" => {
            // Signal exit request
        }
        "restart" => {
            crate::restart::wm_restart();
        }
        "reload" => {
            crate::restart::wm_reload(state);
        }
        "expose" => {
            let active = !state.expose_active;
            if !active {
                // Exiting expose mode! Focus the window under pointer.
                if let Some(wid) = get_window_under_pointer(state) {
                    for seat in &mut state.seats {
                        if !seat.removed {
                            seat.focused_window_id = Some(wid);
                        }
                    }
                    state.move_window_to_end(wid);
                    state.needs_focus = true;
                }
            }
            crate::wm::set_expose_active(state, active);
            state.needs_render = true;
            state.needs_status_update = true;
        }
        "expose-exit" => {
            if state.expose_active {
                let hovered_id = get_window_under_pointer(state);
                crate::wm::set_expose_active(state, false);
                if let Some(wid) = hovered_id {
                    for seat in &mut state.seats {
                        if !seat.removed {
                            seat.focused_window_id = Some(wid);
                        }
                    }
                    state.move_window_to_end(wid);
                    state.needs_focus = true;
                }
                state.needs_render = true;
                state.needs_status_update = true;
            }
        }
        "view-next" => {
            let current_tag_idx = (0..NUM_TAGS).find(|&i| (state.active_tags & (1 << i)) != 0).unwrap_or(0);
            let next_tag_idx = (current_tag_idx + 1) % NUM_TAGS;
            state.active_tags = 1 << next_tag_idx;

            if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                let visible_ids: Vec<u64> = state
                    .windows
                    .iter()
                    .filter(|w| (w.tags & state.active_tags) != 0 && !w.closed && w.app_id.as_deref() != Some("clear-status-interface"))
                    .map(|w| w.id)
                    .collect();
                seat.focused_window_id = visible_ids.last().copied();
            }
            state.needs_render = true;
            state.needs_focus = true;
            state.needs_status_update = true;
        }
        "view-prev" => {
            let current_tag_idx = (0..NUM_TAGS).find(|&i| (state.active_tags & (1 << i)) != 0).unwrap_or(0);
            let prev_tag_idx = (current_tag_idx + NUM_TAGS - 1) % NUM_TAGS;
            state.active_tags = 1 << prev_tag_idx;

            if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                let visible_ids: Vec<u64> = state
                    .windows
                    .iter()
                    .filter(|w| (w.tags & state.active_tags) != 0 && !w.closed && w.app_id.as_deref() != Some("clear-status-interface"))
                    .map(|w| w.id)
                    .collect();
                seat.focused_window_id = visible_ids.last().copied();
            }
            state.needs_render = true;
            state.needs_focus = true;
            state.needs_status_update = true;
        }
        "view" | _ if tok.starts_with("view") => {
            let tag = parse_tag_from_command(tok, "view", rest);
            if let Some(tag) = tag {
                if tag >= 1 && tag <= NUM_TAGS as i32 {
                    state.active_tags = 1 << (tag - 1);

                    // Reassign focus to a visible window on the new tag
                    if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                        let visible_ids: Vec<u64> = state
                            .windows
                            .iter()
                            .filter(|w| (w.tags & state.active_tags) != 0 && !w.closed && w.app_id.as_deref() != Some("clear-status-interface"))
                            .map(|w| w.id)
                            .collect();
                        seat.focused_window_id = visible_ids.last().copied();
                    }
                    state.needs_render = true;
                    state.needs_focus = true;
                    state.needs_status_update = true;
                }
            }
        }
        "toggle" | _ if tok.starts_with("toggle") => {
            let tag = parse_tag_from_command(tok, "toggle", rest);
            if let Some(tag) = tag {
                if tag >= 1 && tag <= NUM_TAGS as i32 {
                    state.active_tags ^= 1 << (tag - 1);

                    // If the focused window is no longer visible, reassign focus
                    let focused_id = state
                        .seats
                        .iter()
                        .find(|s| !s.removed)
                        .and_then(|s| s.focused_window_id);
                    let focused_still_visible = focused_id.map_or(false, |fid| {
                        state
                            .get_window(fid)
                            .map_or(false, |w| (w.tags & state.active_tags) != 0 && !w.closed)
                    });
                    if !focused_still_visible {
                        let visible_ids: Vec<u64> = state
                            .windows
                            .iter()
                            .filter(|w| (w.tags & state.active_tags) != 0 && !w.closed && w.app_id.as_deref() != Some("clear-status-interface"))
                            .map(|w| w.id)
                            .collect();
                        if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                            seat.focused_window_id = visible_ids.last().copied();
                        }
                        state.needs_focus = true;
                    }
                    state.needs_render = true;
                    state.needs_status_update = true;
                }
            }
        }
        "config-done" => {
            state.config_done = true;
        }
        "layout" => {
            handle_layout_command(rest, state);
        }
        "mode" => {
            handle_mode_command(rest, state);
        }
        "set-mode" => {
            handle_set_mode_command(rest, state);
        }
        "apply-mode-sharing" => {
            handle_apply_mode_sharing_command(rest, state);
        }
        "mode-next-shared" => {
            let cycle = [
                TilingMode::Cascade,
                TilingMode::Grid,
                TilingMode::Fullscreen,
                TilingMode::Floating,
            ];
            let focused_id = state
                .seats
                .iter()
                .find(|s| !s.removed)
                .and_then(|s| s.focused_window_id);
            if let Some(fid) = focused_id {
                let notifications_enable = state.notifications_enable;
                let current_mode = state.get_window(fid).map(|w| w.tiling_mode);
                if let Some(old_mode) = current_mode {
                    let next = cycle
                        .iter()
                        .position(|m| *m == old_mode)
                        .map(|i| cycle[(i + 1) % cycle.len()])
                        .unwrap_or(TilingMode::Cascade);

                    let active_tags = state.active_tags;

                    let mut updated_count = 0;
                    for win in &mut state.windows {
                        if !win.closed
                            && win.app_id.as_deref() != Some("clear-status-interface")
                            && (win.tags & active_tags) != 0
                            && win.tiling_mode == old_mode
                        {
                            win.tiling_mode = next;
                            win.mode_locked = true;
                            updated_count += 1;
                        }
                    }

                    if notifications_enable && updated_count > 0 {
                        crate::config::show_notification(
                            "ccec",
                            &format!(
                                "Tiling mode set to {} for all {} windows on active tag",
                                next.as_str(),
                                old_mode.as_str()
                            ),
                        );
                    }
                    state.needs_render = true;
                    state.needs_status_update = true;
                }
            }
        }
        "bind" => {
            handle_bind_command(rest, state);
        }
        "pbind" => {
            handle_pbind_command(rest, state);
        }
        "retile" => {
            // Retile is handled by the WM layer - just set a flag
            // The actual tile_windows() call happens in wm.rs
        }
        "tag-layout" => {
            handle_tag_layout_command(rest, state);
        }
        "set-tag" => {
            handle_set_tag_command(rest, state);
        }
        "repeat" => {
            handle_repeat_command(rest, state);
        }
        "input" => {
            handle_input_command(rest, state);
        }
        "notify" => {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() == 2 {
                if state.notifications_enable {
                    crate::config::show_notification(parts[0], parts[1]);
                }
            } else if parts.len() == 1 && !parts[0].is_empty() {
                if state.notifications_enable {
                    crate::config::show_notification("ccec", parts[0]);
                }
            }
        }
        "focus-window" => {
            let app_id_or_title = rest.trim();
            if !app_id_or_title.is_empty() {
                let mut found_id = None;
                for win in &state.windows {
                    if !win.closed && !win.minimized {
                        if let Some(ref app_id) = win.app_id {
                            if app_id.eq_ignore_ascii_case(app_id_or_title) {
                                found_id = Some(win.id);
                                break;
                            }
                        }
                    }
                }
                if found_id.is_none() {
                    for win in &state.windows {
                        if !win.closed && !win.minimized {
                            if let Some(ref title) = win.title {
                                if title.to_lowercase().contains(&app_id_or_title.to_lowercase()) {
                                    found_id = Some(win.id);
                                    break;
                                }
                            }
                        }
                    }
                }
                if let Some(wid) = found_id {
                    if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                        seat.focused_window_id = Some(wid);
                        state.move_window_to_end(wid);
                        state.needs_focus = true;
                        state.needs_render = true;
                        state.needs_status_update = true;
                        reply = format!("ok focused window {}\n", wid);
                    } else {
                        reply = "error: no active seat\n".to_string();
                    }
                } else {
                    reply = "error: window not found\n".to_string();
                }
            } else {
                reply = "error: usage: focus-window <app_id|title>\n".to_string();
            }
        }
        "pointer-location" => {
            let px = crate::input::POINTER_X.load(std::sync::atomic::Ordering::SeqCst);
            let py = crate::input::POINTER_Y.load(std::sync::atomic::Ordering::SeqCst);
            reply = format!("{} {}\n", px, py);
        }
        "pointer-move-to" => {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() == 2 {
                if let (Ok(target_x), Ok(target_y)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                    if let Some(ref controller) = state.input_controller {
                        let _ = controller.send(crate::input::InputDaemonMsg::SimulateMoveTo { x: target_x, y: target_y });
                    }
                } else {
                    reply = "error: invalid coordinates\n".to_string();
                }
            } else {
                reply = "error: usage: pointer-move-to <x> <y>\n".to_string();
            }
        }
        "pointer-move-by" => {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() == 2 {
                if let (Ok(dx), Ok(dy)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                    if let Some(ref controller) = state.input_controller {
                        let _ = controller.send(crate::input::InputDaemonMsg::SimulateMoveBy { dx, dy });
                    }
                } else {
                    reply = "error: invalid deltas\n".to_string();
                }
            } else {
                reply = "error: usage: pointer-move-by <dx> <dy>\n".to_string();
            }
        }
        "pointer-click" => {
            let btn = parse_button(rest) as u16;
            if btn != 0 {
                if let Some(ref controller) = state.input_controller {
                    let _ = controller.send(crate::input::InputDaemonMsg::SimulateClick { button: btn });
                }
            } else {
                reply = "error: invalid button\n".to_string();
            }
        }
        "pointer-press" => {
            let btn = parse_button(rest) as u16;
            if btn != 0 {
                if let Some(ref controller) = state.input_controller {
                    let _ = controller.send(crate::input::InputDaemonMsg::SimulateButton { button: btn, press: true });
                }
            } else {
                reply = "error: invalid button\n".to_string();
            }
        }
        "pointer-release" => {
            let btn = parse_button(rest) as u16;
            if btn != 0 {
                if let Some(ref controller) = state.input_controller {
                    let _ = controller.send(crate::input::InputDaemonMsg::SimulateButton { button: btn, press: false });
                }
            } else {
                reply = "error: invalid button\n".to_string();
            }
        }
        "keypress" => {
            if let Some(key) = parse_keycode(rest) {
                if let Some(ref controller) = state.input_controller {
                    let _ = controller.send(crate::input::InputDaemonMsg::SimulateKeyPress { keycode: key });
                }
            } else {
                reply = "error: invalid key\n".to_string();
            }
        }
        "key-press" => {
            if let Some(key) = parse_keycode(rest) {
                if let Some(ref controller) = state.input_controller {
                    let _ = controller.send(crate::input::InputDaemonMsg::SimulateKey { keycode: key, press: true });
                }
            } else {
                reply = "error: invalid key\n".to_string();
            }
        }
        "key-release" => {
            if let Some(key) = parse_keycode(rest) {
                if let Some(ref controller) = state.input_controller {
                    let _ = controller.send(crate::input::InputDaemonMsg::SimulateKey { keycode: key, press: false });
                }
            } else {
                reply = "error: invalid key\n".to_string();
            }
        }
        _ => {
            reply = "error: unknown command\n".to_string();
        }
    }

    reply
}

/// Parse a tag number from a command like "view-1" or "view 1"
fn parse_tag_from_command(tok: &str, prefix: &str, rest: &str) -> Option<i32> {
    let after_prefix = &tok[prefix.len()..];
    if after_prefix.starts_with('-') {
        after_prefix[1..].parse::<i32>().ok()
    } else if !rest.is_empty() {
        rest.trim()
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<i32>().ok())
    } else {
        None
    }
}

/// Handle "layout <param> <value>" command
fn handle_layout_command(rest: &str, state: &mut WindowManager) {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }
    let param = parts[0];
    let value_str = parts[1];

    match param {
        "gap" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.gap = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Gap set to {}px", value));
                }
            }
        }
        "gap_top" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.gap_top = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Top gap set to {}px", value));
                }
            }
        }
        "gap_left" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.gap_left = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Left gap set to {}px", value));
                }
            }
        }
        "gap_right" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.gap_right = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Right gap set to {}px", value));
                }
            }
        }
        "gap_bottom" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.gap_bottom = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Bottom gap set to {}px", value));
                }
            }
        }
        "cascade_offset" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.cascade_offset = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Cascade offset set to {}px", value));
                }
            }
        }
        "bar_height" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.bar_height = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Bar height set to {}px", value));
                }
            }
        }
        "border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Border width set to {}px", value));
                }
            }
        }
        "border_font_size" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.border_font_size = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Border font size set to {}px", value));
                }
            }
        }
        "transition_duration" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.transition_duration = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Transition duration set to {}ms", value));
                }
            }
        }
        "fullscreen_border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.fullscreen_border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Fullscreen border width set to {}px", value));
                }
            }
        }
        "cascade_border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.cascade_border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Cascade border width set to {}px", value));
                }
            }
        }
        "grid_gap" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.grid_gap = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Grid gap set to {}px", value));
                }
            }
        }
        "grid_border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.grid_border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Grid border width set to {}px", value));
                }
            }
        }
        "floating_border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.floating_border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Floating border width set to {}px", value));
                }
            }
        }
        "border_color" | "high_color" => {
            if let Some((r, g, b, a)) = parse_hex_color(value_str) {
                state.layout.border_r = r;
                state.layout.border_g = g;
                state.layout.border_b = b;
                state.layout.border_a = a;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Border color set to {}", value_str));
                }
            }
        }
        "background_color" | "low_color" => {
            if let Some((r, g, b, a)) = parse_hex_color(value_str) {
                state.layout.background_r = r;
                state.layout.background_g = g;
                state.layout.background_b = b;
                state.layout.background_a = a;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Background color set to {}", value_str));
                }
            }
        }
        _ => {}
    }
    state.needs_render = true;
}

/// Handle "mode <mode> <app_id_pattern> [--single] [--tag N] [title_pattern]" command
fn handle_mode_command(rest: &str, state: &mut WindowManager) {
    let mut parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let mode_str = parts[0];
    let app_id_pattern = parts[1].to_string();

    // Remove the first two tokens
    parts.drain(0..2);

    let mut title_pattern: Option<String> = None;
    let mut single_instance = false;
    let mut tag = 0i32;

    let mut i = 0;
    while i < parts.len() {
        if parts[i] == "--single" {
            single_instance = true;
        } else if parts[i] == "--tag" {
            if i + 1 < parts.len() {
                tag = parts[i + 1].parse::<i32>().unwrap_or(0);
                i += 1;
            }
        } else if title_pattern.is_none() {
            title_pattern = Some(parts[i].to_string());
        }
        i += 1;
    }

    let mode = parse_tiling_mode(mode_str);
    if let Some(existing) = state.mode_rules.iter_mut().find(|r| {
        r.app_id_pattern == app_id_pattern && r.title_pattern == title_pattern
    }) {
        existing.mode = mode;
        existing.single_instance = single_instance;
        existing.tag = tag;
    } else {
        state.mode_rules.push(ModeRule {
            mode,
            app_id_pattern,
            title_pattern,
            single_instance,
            tag,
            circular: false,
        });
    }
}

/// Handle "set-mode <mode>" command — set the focused window's tiling mode
fn handle_set_mode_command(rest: &str, state: &mut WindowManager) {
    let mode_str = rest.trim();
    if mode_str.is_empty() {
        return;
    }
    let mode = parse_tiling_mode(mode_str);
    let notifications_enable = state.notifications_enable;
    if let Some(window) = state.focused_window_mut() {
        window.tiling_mode = mode;
        window.mode_locked = true;
        if notifications_enable {
            let win_title = window.title.as_deref().unwrap_or("Window");
            crate::config::show_notification("ccec", &format!("Tiling mode set to {} for: {}", mode.as_str(), win_title));
        }
        state.needs_render = true;
        state.needs_status_update = true;
    }
}

/// Handle "apply-mode-sharing <mode>" command — apply <mode> to all windows sharing a tiling mode with the focused window.
fn handle_apply_mode_sharing_command(rest: &str, state: &mut WindowManager) {
    let mode_str = rest.trim();
    if mode_str.is_empty() {
        return;
    }
    let new_mode = parse_tiling_mode(mode_str);

    // Find the focused window's current tiling mode
    let old_mode = if let Some(window) = state.focused_window() {
        window.tiling_mode
    } else {
        return;
    };

    let notifications_enable = state.notifications_enable;

    // Iterate over all windows and update tiling mode for matching windows
    for window in &mut state.windows {
        if !window.closed && window.tiling_mode == old_mode {
            window.tiling_mode = new_mode;
            window.mode_locked = true;
        }
    }

    if notifications_enable {
        crate::config::show_notification("ccec", &format!("Applied tiling mode {} to all windows sharing mode {}", new_mode.as_str(), old_mode.as_str()));
    }
    state.needs_render = true;
    state.needs_status_update = true;
}

/// Handle "bind <mods> <key> <action> [command]" command
fn handle_bind_command(rest: &str, state: &mut WindowManager) {
    // Format: bind <mods> <key> <action> [command...]
    let parts: Vec<&str> = rest.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return;
    }
    let mod_str = parts[0];
    let key_str = parts[1];
    let action_and_cmd = parts[2];

    let mods = parse_modifiers(mod_str);
    let keysym = parse_keysym(key_str);

    // Split action from command
    let (action_str, command) = if let Some(space_pos) = action_and_cmd.find(' ') {
        let (a, c) = action_and_cmd.split_at(space_pos);
        (a, Some(c.trim_start().to_string()))
    } else {
        (action_and_cmd, None)
    };

    let action = parse_action(action_str);
    let command = if action == Action::Spawn {
        command
    } else {
        None
    };

    state.pending_bindings.push(PendingXkbBinding {
        mods,
        keysym,
        action,
        command,
    });
}

/// Handle "pbind <mods> <button> <action>" command
fn handle_pbind_command(rest: &str, state: &mut WindowManager) {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 3 {
        return;
    }
    let mod_str = parts[0];
    let button_str = parts[1];
    let action_str = parts[2];

    let mods = parse_modifiers(mod_str);
    let button = parse_button(button_str);
    let action = parse_action(action_str);

    state.pending_pointer_bindings.push(PendingPointerBinding {
        mods,
        button,
        action,
    });
}

/// Handle "tag-layout <tag> <mode>" command
fn handle_tag_layout_command(rest: &str, state: &mut WindowManager) {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }
    if let Ok(tag) = parts[0].parse::<i32>() {
        if tag >= 1 && tag <= NUM_TAGS as i32 {
            let mode = parse_tiling_mode(parts[1]);
            state.tag_layouts[tag as usize - 1] = mode;
            state.has_tag_layout[tag as usize - 1] = true;
            if state.notifications_enable {
                crate::config::show_notification("ccec", &format!("Tag {} layout set to {}", tag, mode.as_str()));
            }
            state.needs_render = true;
            state.needs_status_update = true;
        }
    }
}

/// Handle "set-tag <tag>" command — set the focused window's tag
fn handle_set_tag_command(rest: &str, state: &mut WindowManager) {
    let tag_str = rest.trim();
    if let Ok(tag) = tag_str.parse::<i32>() {
        if tag >= 1 && tag <= NUM_TAGS as i32 {
            // Read focused_id before any mutable borrow
            let focused_id = state
                .seats
                .iter()
                .find(|s| !s.removed)
                .and_then(|s| s.focused_window_id);
            if let Some(focused_id) = focused_id {
                // Set the window's tag
                let active_tags = state.active_tags;
                let window_left_active_tag =
                    state.get_window_mut(focused_id).map_or(false, |window| {
                        window.tags = 1 << (tag - 1);
                        (window.tags & active_tags) == 0
                    });

                // If the window is no longer on an active tag, shift focus
                if window_left_active_tag {
                    let visible_ids: Vec<u64> = state
                        .windows
                        .iter()
                        .filter(|w| (w.tags & state.active_tags) != 0 && !w.closed && w.app_id.as_deref() != Some("clear-status-interface"))
                        .map(|w| w.id)
                        .collect();
                    if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                        seat.focused_window_id = visible_ids.last().copied();
                    }
                    state.needs_focus = true;
                }
                state.needs_render = true;
                state.needs_status_update = true;
            }
        }
    }
}

/// Handle "repeat <rate> <delay>" command
fn handle_repeat_command(_rest: &str, _state: &mut WindowManager) {
    // The actual repeat rate change is applied to input devices via the
    // Wayland protocol; this is a placeholder for the pure-logic layer.
    // The WM integration layer will read the rate/delay and call
    // river_input_device_v1_set_repeat_info()
}

fn handle_input_command(rest: &str, state: &mut WindowManager) {
    let tokens: Vec<&str> = rest.splitn(2, ' ').collect();
    let param = tokens[0];
    let value_str = if tokens.len() > 1 { tokens[1] } else { "" };

    match param {
        "tap-to-click" | "tap_to_click" => {
            let old_val = state.tap_to_click;
            match value_str {
                "true" | "1" | "enabled" => {
                    state.tap_to_click = true;
                    state.tap_config_applied = false; // re-apply
                }
                "false" | "0" | "disabled" => {
                    state.tap_to_click = false;
                    state.tap_config_applied = false; // re-apply
                }
                "toggle" => {
                    state.tap_to_click = !state.tap_to_click;
                    state.tap_config_applied = false; // re-apply
                }
                _ => {}
            }
            if state.tap_to_click != old_val && state.notifications_enable {
                crate::config::show_notification(
                    "ccec",
                    &format!(
                        "Tap-to-click {}",
                        if state.tap_to_click { "enabled" } else { "disabled" }
                    ),
                );
            }
        }
        "accel-speed" | "accel_speed" => {
            if let Ok(val) = value_str.parse::<f64>() {
                state.accel_speed = Some(val);
                state.tap_config_applied = false;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Acceleration speed set to {}", val));
                }
            }
        }
        "accel-profile" | "accel_profile" => {
            let val = value_str.trim().to_string();
            if !val.is_empty() {
                state.accel_profile = Some(val.clone());
                state.tap_config_applied = false;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Acceleration profile set to {}", val));
                }
            }
        }
        "natural-scroll" | "natural_scroll" => {
            let old_val = state.natural_scroll;
            match value_str {
                "true" | "1" | "enabled" => {
                    state.natural_scroll = Some(true);
                }
                "false" | "0" | "disabled" => {
                    state.natural_scroll = Some(false);
                }
                "toggle" => {
                    state.natural_scroll = Some(!state.natural_scroll.unwrap_or(false));
                }
                _ => {}
            }
            if state.natural_scroll != old_val {
                state.tap_config_applied = false;
                if state.notifications_enable {
                    crate::config::show_notification(
                        "ccec",
                        &format!(
                            "Natural scroll {}",
                            if state.natural_scroll.unwrap_or(false) { "enabled" } else { "disabled" }
                        ),
                    );
                }
            }
        }
        "dwt" => {
            let old_val = state.dwt;
            match value_str {
                "true" | "1" | "enabled" => {
                    state.dwt = Some(true);
                }
                "false" | "0" | "disabled" => {
                    state.dwt = Some(false);
                }
                "toggle" => {
                    state.dwt = Some(!state.dwt.unwrap_or(false));
                }
                _ => {}
            }
            if state.dwt != old_val {
                state.tap_config_applied = false;
                if state.notifications_enable {
                    crate::config::show_notification(
                        "ccec",
                        &format!(
                            "Disable-while-typing {}",
                            if state.dwt.unwrap_or(false) { "enabled" } else { "disabled" }
                        ),
                    );
                }
            }
        }
        "dwtp" => {
            let old_val = state.dwtp;
            match value_str {
                "true" | "1" | "enabled" => {
                    state.dwtp = Some(true);
                }
                "false" | "0" | "disabled" => {
                    state.dwtp = Some(false);
                }
                "toggle" => {
                    state.dwtp = Some(!state.dwtp.unwrap_or(false));
                }
                _ => {}
            }
            if state.dwtp != old_val {
                state.tap_config_applied = false;
                if state.notifications_enable {
                    crate::config::show_notification(
                        "ccec",
                        &format!(
                            "Disable-while-trackpointing {}",
                            if state.dwtp.unwrap_or(false) { "enabled" } else { "disabled" }
                        ),
                    );
                }
            }
        }
        "trackpad-disabled" | "trackpad_disabled" => {
            let old_val = state.trackpad_disabled;
            match value_str {
                "true" | "1" | "enabled" => {
                    state.trackpad_disabled = true;
                }
                "false" | "0" | "disabled" => {
                    state.trackpad_disabled = false;
                }
                _ => {}
            }
            if state.trackpad_disabled != old_val {
                state.tap_config_applied = false;
            }
        }
        "trackpoint-accel-speed" | "trackpoint_accel_speed" => {
            if let Ok(val) = value_str.parse::<f64>() {
                state.trackpoint_accel_speed = Some(val);
                state.tap_config_applied = false;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Trackpoint acceleration speed set to {}", val));
                }
            }
        }
        "trackpoint-accel-profile" | "trackpoint_accel_profile" => {
            let val = value_str.trim().to_string();
            if !val.is_empty() {
                state.trackpoint_accel_profile = Some(val.clone());
                state.tap_config_applied = false;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Trackpoint acceleration profile set to {}", val));
                }
            }
        }
        "cursor-theme" | "cursor_theme" => {
            let val = value_str.trim().to_string();
            if !val.is_empty() {
                state.cursor_theme = Some(val.clone());
                state.cursor_theme_applied = false;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Cursor theme set to {}", val));
                }
            }
        }
        "cursor-size" | "cursor_size" => {
            if let Ok(val) = value_str.parse::<u32>() {
                state.cursor_size = Some(val);
                state.cursor_theme_applied = false;
                if state.notifications_enable {
                    crate::config::show_notification("ccec", &format!("Cursor size set to {}", val));
                }
            }
        }
        _ => {}
    }
}


fn parse_keycode(key: &str) -> Option<u16> {
    let key = key.trim();
    if let Ok(val) = key.parse::<u16>() {
        return Some(val);
    }
    match key.to_lowercase().as_str() {
        "esc" | "escape" => Some(1),
        "1" => Some(2), "2" => Some(3), "3" => Some(4), "4" => Some(5),
        "5" => Some(6), "6" => Some(7), "7" => Some(8), "8" => Some(9),
        "9" => Some(10), "0" => Some(11),
        "minus" => Some(12), "equal" => Some(13), "backspace" => Some(14),
        "tab" => Some(15),
        "q" => Some(16), "w" => Some(17), "e" => Some(18), "r" => Some(19),
        "t" => Some(20), "y" => Some(21), "u" => Some(22), "i" => Some(23),
        "o" => Some(24), "p" => Some(25),
        "leftbrace" | "[" => Some(26), "rightbrace" | "]" => Some(27),
        "enter" | "return" => Some(28),
        "ctrl" | "leftctrl" => Some(29),
        "a" => Some(30), "s" => Some(31), "d" => Some(32), "f" => Some(33),
        "g" => Some(34), "h" => Some(35), "j" => Some(36), "k" => Some(37),
        "l" => Some(38), "semicolon" | ";" => Some(39), "apostrophe" | "'" => Some(40),
        "grave" | "`" => Some(41), "shift" | "leftshift" => Some(42),
        "backslash" | "\\" => Some(43),
        "z" => Some(44), "x" => Some(45), "c" => Some(46), "v" => Some(47),
        "b" => Some(48), "n" => Some(49), "m" => Some(50),
        "comma" | "," => Some(51), "dot" | "." => Some(52), "slash" | "/" => Some(53),
        "rightshift" => Some(54),
        "alt" | "leftalt" => Some(56), "space" => Some(57), "capslock" => Some(58),
        "f1" => Some(59), "f2" => Some(60), "f3" => Some(61), "f4" => Some(62),
        "f5" => Some(63), "f6" => Some(64), "f7" => Some(65), "f8" => Some(66),
        "f9" => Some(67), "f10" => Some(68),
        "f11" => Some(87), "f12" => Some(88),
        "rightctrl" => Some(97), "rightalt" => Some(100),
        "home" => Some(102), "up" => Some(103), "pageup" => Some(104),
        "left" => Some(105), "right" => Some(106), "end" => Some(107),
        "down" => Some(108), "pagedown" => Some(109), "insert" => Some(110),
        "delete" => Some(111),
        "super" | "hyper" | "meta" | "logo" | "leftmeta" | "leftsuper" => Some(125),
        "rightmeta" | "rightsuper" => Some(126),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TilingMode, WindowManager};

    #[test]
    fn test_ipc_view() {
        let mut state = WindowManager::default();
        assert_eq!(state.active_tags, 1);

        handle_ipc_command("view-2", &mut state);
        assert_eq!(state.active_tags, 2);

        handle_ipc_command("view 3", &mut state);
        assert_eq!(state.active_tags, 4);
    }

    #[test]
    fn test_ipc_toggle() {
        let mut state = WindowManager::default();
        assert_eq!(state.active_tags, 1);

        handle_ipc_command("toggle-2", &mut state);
        assert_eq!(state.active_tags, 3); // tag 1 + tag 2

        handle_ipc_command("toggle-1", &mut state);
        assert_eq!(state.active_tags, 2); // tag 2 only
    }

    #[test]
    fn test_ipc_config_done() {
        let mut state = WindowManager::default();
        assert!(!state.config_done);

        handle_ipc_command("config-done", &mut state);
        assert!(state.config_done);
    }

    #[test]
    fn test_ipc_layout_gap() {
        let mut state = WindowManager::default();
        assert_eq!(state.layout.gap, 48);

        handle_ipc_command("layout gap 18", &mut state);
        assert_eq!(state.layout.gap, 18);
    }

    #[test]
    fn test_ipc_layout_grid_gap() {
        let mut state = WindowManager::default();
        assert_eq!(state.layout.grid_gap, 18);

        handle_ipc_command("layout grid_gap 24", &mut state);
        assert_eq!(state.layout.grid_gap, 24);
    }

    #[test]
    fn test_ipc_layout_gap_sides() {
        let mut state = WindowManager::default();
        assert_eq!(state.layout.gap_top, 48);
        assert_eq!(state.layout.gap_left, 48);
        assert_eq!(state.layout.gap_right, 48);
        assert_eq!(state.layout.gap_bottom, 48);

        handle_ipc_command("layout gap_top 10", &mut state);
        handle_ipc_command("layout gap_left 20", &mut state);
        handle_ipc_command("layout gap_right 30", &mut state);
        handle_ipc_command("layout gap_bottom 40", &mut state);
        assert_eq!(state.layout.gap_top, 10);
        assert_eq!(state.layout.gap_left, 20);
        assert_eq!(state.layout.gap_right, 30);
        assert_eq!(state.layout.gap_bottom, 40);
    }

    #[test]
    fn test_ipc_layout_border_color() {
        let mut state = WindowManager::default();
        handle_ipc_command("layout border_color #5c9060", &mut state);
        assert_eq!(state.layout.border_r, 0x5C5C5C5C);
        assert_eq!(state.layout.border_g, 0x90909090);
        assert_eq!(state.layout.border_b, 0x60606060);
    }

    #[test]
    fn test_ipc_mode_rule() {
        let mut state = WindowManager::default();
        handle_ipc_command("mode cascade ghostty", &mut state);
        assert_eq!(state.mode_rules.len(), 1);
        assert_eq!(state.mode_rules[0].mode, TilingMode::Cascade);
        assert_eq!(state.mode_rules[0].app_id_pattern, "ghostty");
    }

    #[test]
    fn test_ipc_mode_rule_update() {
        let mut state = WindowManager::default();
        handle_ipc_command("mode fullscreen clear-system-interface", &mut state);
        assert_eq!(state.mode_rules.len(), 1);
        assert_eq!(state.mode_rules[0].mode, TilingMode::Fullscreen);

        handle_ipc_command("mode cascade clear-system-interface", &mut state);
        assert_eq!(state.mode_rules.len(), 1);
        assert_eq!(state.mode_rules[0].mode, TilingMode::Cascade);
    }

    #[test]
    fn test_ipc_mode_rule_with_single() {
        let mut state = WindowManager::default();
        handle_ipc_command("mode cascade qutebrowser --single", &mut state);
        assert!(state.mode_rules[0].single_instance);
    }

    #[test]
    fn test_ipc_mode_rule_with_tag() {
        let mut state = WindowManager::default();
        handle_ipc_command("mode cascade ghostty --tag 2", &mut state);
        assert_eq!(state.mode_rules[0].tag, 2);
    }

    #[test]
    fn test_ipc_bind() {
        let mut state = WindowManager::default();
        handle_ipc_command("bind alt Return spawn ghostty", &mut state);
        assert_eq!(state.pending_bindings.len(), 1);
        assert_eq!(state.pending_bindings[0].action, Action::Spawn);
        assert_eq!(
            state.pending_bindings[0].command,
            Some("ghostty".to_string())
        );
    }

    #[test]
    fn test_ipc_pbind() {
        let mut state = WindowManager::default();
        handle_ipc_command("pbind alt left move", &mut state);
        assert_eq!(state.pending_pointer_bindings.len(), 1);
        assert_eq!(state.pending_pointer_bindings[0].action, Action::Move);
        assert_eq!(state.pending_pointer_bindings[0].button, 0x110); // BTN_LEFT
    }

    #[test]
    fn test_ipc_tag_layout() {
        let mut state = WindowManager::default();
        handle_ipc_command("tag-layout 2 grid", &mut state);
        assert!(state.has_tag_layout[1]);
        assert_eq!(state.tag_layouts[1], TilingMode::Grid);
    }

    #[test]
    fn test_ipc_set_tag() {
        let mut state = WindowManager::default();
        // Add a window and a seat with focus
        state.windows.push(crate::types::Window {
            id: 1,
            tags: 1,
            ..Default::default()
        });
        state.seats.push(crate::types::Seat {
            id: 1,
            focused_window_id: Some(1),
            ..Default::default()
        });

        handle_ipc_command("set-tag 3", &mut state);
        assert_eq!(state.windows[0].tags, 4); // 1 << 2
    }

    #[test]
    fn test_ipc_empty_command() {
        let mut state = WindowManager::default();
        handle_ipc_command("", &mut state);
        handle_ipc_command("  \n", &mut state);
        // Should not crash
    }

    #[test]
    fn test_ipc_layout_cascade_offset() {
        let mut state = WindowManager::default();
        handle_ipc_command("layout cascade_offset 32", &mut state);
        assert_eq!(state.layout.cascade_offset, 32);
    }

    #[test]
    fn test_ipc_layout_bar_height() {
        let mut state = WindowManager::default();
        handle_ipc_command("layout bar_height 28", &mut state);
        assert_eq!(state.layout.bar_height, 28);
    }

    #[test]
    fn test_ipc_layout_border_width() {
        let mut state = WindowManager::default();
        handle_ipc_command("layout border_width 18", &mut state);
        assert_eq!(state.layout.border_width, 18);
    }

    #[test]
    fn test_ipc_expose() {
        let mut state = WindowManager::default();
        assert!(!state.expose_active);
        handle_ipc_command("expose", &mut state);
        assert!(state.expose_active);
        handle_ipc_command("expose", &mut state);
        assert!(!state.expose_active);
    }

    #[test]
    fn test_ipc_expose_exit() {
        let mut state = WindowManager::default();
        state.seats.push(crate::types::Seat {
            id: 1,
            hovered_window_id: Some(42),
            focused_window_id: Some(10),
            ..Default::default()
        });

        // 1. When expose is not active, "expose-exit" should do nothing.
        assert!(!state.expose_active);
        handle_ipc_command("expose-exit", &mut state);
        assert!(!state.expose_active);
        assert_eq!(state.seats[0].focused_window_id, Some(10));

        // 2. When expose is active, "expose-exit" should deactivate it and focus the hovered window.
        state.expose_active = true;
        handle_ipc_command("expose-exit", &mut state);
        assert!(!state.expose_active);
        assert_eq!(state.seats[0].focused_window_id, Some(42));
        assert!(state.needs_focus);
        assert!(state.needs_render);
    }

    #[test]
    fn test_ipc_view_next_prev() {
        let mut state = WindowManager::default();
        state.active_tags = 1; // Tag 1 (1 << 0)

        // view-next should go: Tag 1 -> Tag 2 -> Tag 3 -> Tag 4 -> Tag 1
        handle_ipc_command("view-next", &mut state);
        assert_eq!(state.active_tags, 2); // Tag 2

        handle_ipc_command("view-next", &mut state);
        assert_eq!(state.active_tags, 4); // Tag 3

        handle_ipc_command("view-next", &mut state);
        assert_eq!(state.active_tags, 8); // Tag 4

        handle_ipc_command("view-next", &mut state);
        assert_eq!(state.active_tags, 1); // Tag 1 (wrap around)

        // view-prev should go: Tag 1 -> Tag 4 -> Tag 3 -> Tag 2 -> Tag 1
        handle_ipc_command("view-prev", &mut state);
        assert_eq!(state.active_tags, 8); // Tag 4 (wrap around)

        handle_ipc_command("view-prev", &mut state);
        assert_eq!(state.active_tags, 4); // Tag 3

        handle_ipc_command("view-prev", &mut state);
        assert_eq!(state.active_tags, 2); // Tag 2

        handle_ipc_command("view-prev", &mut state);
        assert_eq!(state.active_tags, 1); // Tag 1
    }

    #[test]
    fn test_ipc_focus_next_prev() {
        let mut state = WindowManager::default();
        // Setup 3 windows
        state.windows.push(crate::types::Window { id: 1, tags: 1, ..Default::default() });
        state.windows.push(crate::types::Window { id: 2, tags: 1, ..Default::default() });
        state.windows.push(crate::types::Window { id: 3, tags: 1, ..Default::default() });
        state.seats.push(crate::types::Seat { id: 1, focused_window_id: Some(1), ..Default::default() });

        handle_ipc_command("focus-next", &mut state);
        assert_eq!(state.seats[0].focused_window_id, Some(2));

        handle_ipc_command("focus-next", &mut state);
        assert_eq!(state.seats[0].focused_window_id, Some(1));

        handle_ipc_command("focus-next", &mut state);
        assert_eq!(state.seats[0].focused_window_id, Some(3));

        handle_ipc_command("focus-prev", &mut state);
        assert_eq!(state.seats[0].focused_window_id, Some(1));

        handle_ipc_command("focus-prev", &mut state);
        assert_eq!(state.seats[0].focused_window_id, Some(3));
    }

    #[test]
    fn test_ipc_pointer_and_keys() {
        let mut state = WindowManager::default();
        
        // Test pointer location query
        let location = handle_ipc_command("pointer-location", &mut state);
        assert!(location.ends_with("\n"));
        let coords: Vec<&str> = location.trim().split_whitespace().collect();
        assert_eq!(coords.len(), 2);
        assert_eq!(coords[0].parse::<i32>().is_ok(), true);
        assert_eq!(coords[1].parse::<i32>().is_ok(), true);

        // Test button parsing helpers
        assert_eq!(parse_button("left"), 272);
        assert_eq!(parse_button("right"), 273);
        assert_eq!(parse_button("middle"), 274);
        assert_eq!(parse_button("side"), 275);
        assert_eq!(parse_button("extra"), 276);
        assert_eq!(parse_button("280"), 280);
        assert_eq!(parse_button("invalid"), 0);

        // Test keycode parsing helpers
        assert_eq!(parse_keycode("escape"), Some(1));
        assert_eq!(parse_keycode("enter"), Some(28));
        assert_eq!(parse_keycode("a"), Some(30));
        assert_eq!(parse_keycode("30"), Some(30));
        assert_eq!(parse_keycode("invalid"), None);

        // Test some simulation commands return error if invalid or ok (without input controller it shouldn't crash)
        let r1 = handle_ipc_command("pointer-move-to invalid", &mut state);
        assert!(r1.starts_with("error:"));
        let r2 = handle_ipc_command("pointer-move-by 10", &mut state);
        assert!(r2.starts_with("error:"));
        let r3 = handle_ipc_command("pointer-click invalid", &mut state);
        assert!(r3.starts_with("error:"));
        let r4 = handle_ipc_command("keypress invalid", &mut state);
        assert!(r4.starts_with("error:"));
    }
}
