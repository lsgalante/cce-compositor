// IPC command parser for clearwm
// Ported from handle_ipc_command in clearwm.c

use crate::config::{parse_keysym, spawn_command_bg};
use crate::types::{
    parse_action, parse_button, parse_hex_color, parse_modifiers, parse_tiling_mode, Action,
    ModeRule, PendingPointerBinding, PendingXkbBinding, WindowManager, NUM_TAGS,
};

/// Handle an IPC command string, modifying the window manager state.
/// This is called from the Unix socket listener when clearctl sends a command.
pub fn handle_ipc_command(cmd: &str, state: &mut WindowManager) {
    // Strip trailing newlines/spaces
    let cmd = cmd.trim_end_matches(|c| c == '\n' || c == '\r' || c == ' ');
    if cmd.is_empty() {
        return;
    }

    // Split the command into tokens
    let tokens: Vec<&str> = cmd.splitn(2, ' ').collect();
    let tok = tokens[0];
    let rest = if tokens.len() > 1 { tokens[1] } else { "" };

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
        "focus-next" => {
            // Focus the next visible window (wrapping) and move it to the
            // front of the cascade stack (end of windows vector).
            if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                let focused_id = seat.focused_window_id;
                let active_tags = state.active_tags;
                let visible_ids: Vec<u64> = state
                    .windows
                    .iter()
                    .filter(|w| (w.tags & active_tags) != 0 && !w.closed)
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
        "exit" => {
            // Signal exit request
        }
        "restart" => {
            crate::restart::wm_restart();
        }
        "reload" => {
            crate::restart::wm_reload(state);
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
                            .filter(|w| (w.tags & state.active_tags) != 0 && !w.closed)
                            .map(|w| w.id)
                            .collect();
                        seat.focused_window_id = visible_ids.last().copied();
                    }
                    state.needs_focus = true;
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
                            .filter(|w| (w.tags & state.active_tags) != 0 && !w.closed)
                            .map(|w| w.id)
                            .collect();
                        if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                            seat.focused_window_id = visible_ids.last().copied();
                        }
                        state.needs_focus = true;
                    }
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
                    crate::config::show_notification("clearwm", parts[0]);
                }
            }
        }
        _ => {
            // Unknown command, ignore
        }
    }
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
                    crate::config::show_notification("clearwm", &format!("Gap set to {}px", value));
                }
            }
        }
        "gap_top" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.gap_top = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Top gap set to {}px", value));
                }
            }
        }
        "gap_left" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.gap_left = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Left gap set to {}px", value));
                }
            }
        }
        "gap_right" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.gap_right = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Right gap set to {}px", value));
                }
            }
        }
        "gap_bottom" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.gap_bottom = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Bottom gap set to {}px", value));
                }
            }
        }
        "cascade_offset" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.cascade_offset = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Cascade offset set to {}px", value));
                }
            }
        }
        "bar_height" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.bar_height = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Bar height set to {}px", value));
                }
            }
        }
        "border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Border width set to {}px", value));
                }
            }
        }
        "fullscreen_border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.fullscreen_border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Fullscreen border width set to {}px", value));
                }
            }
        }
        "cascade_border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.cascade_border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Cascade border width set to {}px", value));
                }
            }
        }
        "grid_border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.grid_border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Grid border width set to {}px", value));
                }
            }
        }
        "vsplit_border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.vsplit_border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Vsplit border width set to {}px", value));
                }
            }
        }
        "hsplit_border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.hsplit_border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Hsplit border width set to {}px", value));
                }
            }
        }
        "floating_border_width" => {
            if let Ok(value) = value_str.parse::<i32>() {
                state.layout.floating_border_width = value;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Floating border width set to {}px", value));
                }
            }
        }
        "border_color" => {
            if let Some((r, g, b, a)) = parse_hex_color(value_str) {
                state.layout.border_r = r;
                state.layout.border_g = g;
                state.layout.border_b = b;
                state.layout.border_a = a;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Border color set to {}", value_str));
                }
            }
        }
        "background_color" => {
            if let Some((r, g, b, a)) = parse_hex_color(value_str) {
                state.layout.background_r = r;
                state.layout.background_g = g;
                state.layout.background_b = b;
                state.layout.background_a = a;
                if state.notifications_enable {
                    crate::config::show_notification("clearwm", &format!("Background color set to {}", value_str));
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
    state.mode_rules.push(ModeRule {
        mode,
        app_id_pattern,
        title_pattern,
        single_instance,
        tag,
    });
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
            crate::config::show_notification("clearwm", &format!("Tiling mode set to {} for: {}", mode.as_str(), win_title));
        }
    }
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
                crate::config::show_notification("clearwm", &format!("Tag {} layout set to {}", tag, mode.as_str()));
            }
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
                        .filter(|w| (w.tags & state.active_tags) != 0 && !w.closed)
                        .map(|w| w.id)
                        .collect();
                    if let Some(seat) = state.seats.iter_mut().find(|s| !s.removed) {
                        seat.focused_window_id = visible_ids.last().copied();
                    }
                    state.needs_focus = true;
                }
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
                    "clearwm",
                    &format!(
                        "Tap-to-click {}",
                        if state.tap_to_click { "enabled" } else { "disabled" }
                    ),
                );
            }
        }
        _ => {}
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
}
