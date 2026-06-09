// Border color computation for cascade depth gradient and normal windows

use crate::types::{TilingMode, WindowManager};

/// Interpolate a single 8-bit channel toward black by (factor ^ depth).
/// Returns a 32-bit value with the byte replicated across all 4 bytes,
/// which is the format River expects (it divides by maxInt(u32) to get a float).
pub fn interp_channel(fp_channel: u32, factor: f64, depth: i32) -> u32 {
    let base = (fp_channel & 0xFF) as u8;
    let mut f = 1.0_f64;
    for _ in 0..depth {
        f *= factor;
    }
    let val = (base as f64 * f) as u8;
    // Replicate byte across all 4 bytes: 0xVV -> 0xVVVVVVVV
    val as u32 * 0x01010101
}

/// Blend between two byte-replicated 32-bit channel values.
/// factor=0.0 → pure bg, factor=1.0 → pure fg.
fn blend_channel(bg_channel: u32, fg_channel: u32, factor: f64) -> u32 {
    let bg = (bg_channel & 0xFF) as u8 as f64;
    let fg = (fg_channel & 0xFF) as u8 as f64;
    let val = (bg + (fg - bg) * factor) as u8;
    val as u32 * 0x01010101
}

/// Alpha for borders: fully opaque, byte-replicated
pub const ALPHA: u32 = 0xFFFFFFFF;

/// Unfocused depth factor: each step away from the focused window reduces
/// the blend factor by this multiplier, making the border color approach
/// the background color.
pub const UNFOCUSED_DEPTH_FACTOR: f64 = 0.70;

/// Result of border color computation for a single window
#[derive(Debug, Clone)]
pub struct WindowBorders {
    pub window_idx: usize,
    pub edges: u32, // all edges = top | bottom | left | right
    pub width: i32,
    pub r: u32,
    pub g: u32,
    pub b: u32,
    pub a: u32,
}

/// Compute border colors for all visible windows.
/// The focused window gets the configured border_color.
/// Unfocused windows get a color interpolated between the desktop
/// background_color and border_color based on stack depth.
pub fn compute_border_colors(state: &WindowManager) -> Vec<WindowBorders> {
    let mut results = Vec::new();
    let all_edges = 0b1111u32;

    // Find the focused window ID from the first non-removed seat
    let focused_id = state
        .seats
        .iter()
        .find(|s| !s.removed)
        .and_then(|s| s.focused_window_id);

    // Collect visible window indices (ordered from bottom to top of stack)
    let visible: Vec<usize> = state
        .windows
        .iter()
        .enumerate()
        .filter(|(_, w)| (w.tags & state.active_tags) != 0 && !w.closed)
        .map(|(i, _)| i)
        .collect();

    for (idx, win) in state.windows.iter().enumerate() {
        if (win.tags & state.active_tags) == 0 {
            continue;
        }

        let is_focused = focused_id.map_or(false, |fid| win.id == fid);

        eprintln!(
            "[borders] win={} app_id={:?} is_focused={} border_r=#{:08x} border_g=#{:08x} border_b=#{:08x}",
            win.id, win.app_id, is_focused,
            state.layout.border_r,
            state.layout.border_g,
            state.layout.border_b,
        );

        let (r, g, b, a) = if win.tiling_mode == TilingMode::Popup {
            // Popup windows have a transparent border
            (0, 0, 0, 0)
        } else if is_focused {
            // Focused window: pure border color
            (
                state.layout.border_r,
                state.layout.border_g,
                state.layout.border_b,
                state.layout.border_a,
            )
        } else {
            // Unfocused window: blend background → border based on depth
            let visible_mode: Vec<usize> = visible
                .iter()
                .cloned()
                .filter(|&i| state.expose_visual_active || state.windows[i].tiling_mode == win.tiling_mode)
                .collect();
            let n_visible_mode = visible_mode.len();
            let pos = visible_mode.iter().position(|&i| i == idx).unwrap_or(0);
            let depth = (n_visible_mode - 1 - pos) as i32;
            let mut factor = 1.0_f64;
            for _ in 0..depth {
                factor *= UNFOCUSED_DEPTH_FACTOR;
            }
            let r = blend_channel(state.layout.background_r, state.layout.border_r, factor);
            let g = blend_channel(state.layout.background_g, state.layout.border_g, factor);
            let b = blend_channel(state.layout.background_b, state.layout.border_b, factor);
            let a = blend_channel(state.layout.background_a, state.layout.border_a, factor);
            (r, g, b, a)
        };

        let mut width = if state.expose_visual_active && win.tiling_mode != TilingMode::Popup {
            state.layout.grid_border_width
        } else {
            match win.tiling_mode {
                TilingMode::Cascade => state.layout.cascade_border_width,
                TilingMode::Fullscreen => state.layout.fullscreen_border_width,
                TilingMode::Grid => state.layout.grid_border_width,
                TilingMode::Floating => state.layout.floating_border_width,
                TilingMode::Popup => 0,
                TilingMode::SidePanel => state.layout.cascade_border_width,
            }
        };

        if win.app_id.as_deref() == Some("cce-status-interface")
            || win.app_id.as_deref().map_or(false, |aid| aid.contains("noborder"))
        {
            width = 0;
        }

        let has_titlebar = !win.closed
            && !win.minimized
            && win.app_id.as_deref() != Some("cce-status-interface")
            && !win.app_id.as_deref().map_or(false, |aid| aid.contains("noborder"))
            && win.tiling_mode != TilingMode::Popup
            && win.tiling_mode != TilingMode::Fullscreen
            && !win.circular;

        let edges = if has_titlebar {
            // Disable compositor-drawn borders entirely; all borders are drawn by client decorations.
            0b0000u32
        } else {
            all_edges
        };

        eprintln!(
            "[borders]   -> r=#{:08x} g=#{:08x} b=#{:08x} a=#{:08x} width={}",
            r, g, b, a, width,
        );

        results.push(WindowBorders {
            window_idx: idx,
            edges,
            width,
            r,
            g,
            b,
            a,
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interp_channel_depth0() {
        // Depth 0 should return the base color unchanged
        let result = interp_channel(0x5C5C5C5C, 0.80, 0);
        assert_eq!(result, 0x5C5C5C5C);
    }

    #[test]
    fn test_interp_channel_depth1() {
        let result = interp_channel(0x90909090, 0.80, 1);
        let expected = ((0x90 as f64 * 0.80) as u8) as u32 * 0x01010101;
        assert_eq!(result, expected);
    }
}
