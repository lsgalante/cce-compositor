// Status file writing and waybar signaling

use crate::types::{TilingMode, WindowManager};
use std::fs;
use std::process::Command;

pub const NUM_TAGS: u32 = 4;

/// Write status files only (no pkill signaling). Safe to call inside
/// Dispatch callbacks — no fork, no blocking, just file I/O.
pub fn write_status_files(state: &WindowManager) {
    use std::io::Write;

    // /tmp/clearwm-tags: active_tags focused_tags num_tags
    if let Ok(mut f) = fs::File::create("/tmp/clearwm-tags") {
        let _ = writeln!(
            f,
            "{} {} {}",
            state.active_tags, state.focused_tags, NUM_TAGS
        );
    }

    // /tmp/clearwm-layout: focused window's tiling mode
    let mode_str = state
        .focused_window()
        .map(|w| tiling_mode_str(w.tiling_mode))
        .unwrap_or("none");

    if let Ok(mut f) = fs::File::create("/tmp/clearwm-layout") {
        let _ = writeln!(f, "{}", mode_str);
    }

    // /tmp/clearwm-windows: one line per window
    if let Ok(mut f) = fs::File::create("/tmp/clearwm-windows") {
        let focused_title = state.focused_window().map(|w| w.title.clone());

        for win in &state.windows {
            let mode_str = tiling_mode_str(win.tiling_mode);
            let decoration_str = match win.decoration_hint {
                0 => "only_csd",
                1 => "prefers_csd",
                2 => "prefers_ssd",
                3 => "no_preference",
                _ => "unknown",
            };
            let presentation_str = match win.presentation_hint {
                0 => "vsync",
                1 => "async",
                _ => "unknown",
            };
            let _ = writeln!(
                f,
                "window app_id={} title={} mode={} decoration={} presentation={} tags={} x={} y={} w={} h={} has_parent={}",
                win.app_id.as_deref().unwrap_or("(null)"),
                win.title.as_deref().unwrap_or("(null)"),
                mode_str,
                decoration_str,
                presentation_str,
                win.tags, win.x, win.y, win.width, win.height,
                win.has_parent,
            );
        }

        // /tmp/clearwm-title
        if let Some(title) = focused_title {
            if let Ok(mut tf) = fs::File::create("/tmp/clearwm-title") {
                let _ = writeln!(tf, "{}", title.as_deref().unwrap_or("(null)"));
            }
        }
    }
}

/// Write all status files and signal waybar.
/// WARNING: The pkill calls block the event loop. Do NOT call this
/// inside a Dispatch callback. Use write_status_files() instead.
pub fn update_status_files(state: &WindowManager) {
    write_status_files(state);

    // Signal waybar
    let _ = Command::new("pkill").args(["-RTMIN+8", "waybar"]).output();
    let _ = Command::new("pkill").args(["-RTMIN+9", "waybar"]).output();
    let _ = Command::new("pkill").args(["-RTMIN+10", "waybar"]).output();
}

fn tiling_mode_str(mode: TilingMode) -> &'static str {
    match mode {
        TilingMode::Floating => "Floating",
        TilingMode::Cascade => "Cascade",
        TilingMode::Grid => "Grid",
        TilingMode::Vsplit => "Vsplit",
        TilingMode::Hsplit => "Hsplit",
        TilingMode::Fullscreen => "Fullscreen",
        TilingMode::Popup => "Popup",
    }
}
