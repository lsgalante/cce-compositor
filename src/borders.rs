// Border color computation for cascade depth gradient and normal windows

use crate::types::{TilingMode, WindowManager};

/// Interpolate a single 8-bit channel toward black by (factor ^ depth).
/// fp_channel is in fixed-point format (0xRR000000).
/// Returns the value in 32-bit fixed-point (channel in high byte).
pub fn interp_channel(fp_channel: u32, factor: f64, depth: i32) -> u32 {
    let base = (fp_channel >> 24) as u8;
    let mut f = 1.0_f64;
    for _ in 0..depth {
        f *= factor;
    }
    let val = (base as f64 * f) as u8;
    (val as u32) << 24
}

/// Write "#RRGGBB" for a given depth into a String.
pub fn cascade_hex_color(br: u32, bg: u32, bb: u32, depth: i32) -> String {
    let depth_factor = 0.80_f64;
    let r = interp_channel(br, depth_factor, depth) >> 24;
    let g = interp_channel(bg, depth_factor, depth) >> 24;
    let b = interp_channel(bb, depth_factor, depth) >> 24;
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

/// Normal border color in fixed-point format: dark gray (#3E3E3E)
pub const BORDER_COLOR_NORMAL_R: u32 = 0x3E000000;
pub const BORDER_COLOR_NORMAL_G: u32 = 0x3E000000;
pub const BORDER_COLOR_NORMAL_B: u32 = 0x3E000000;
pub const BORDER_COLOR_NORMAL_A: u32 = 0x000000FF;

/// Cascade alpha: full alpha in low byte
pub const CASCADE_ALPHA: u32 = 0x000000FF;

/// Cascade depth darkening factor
pub const CASCADE_DEPTH_FACTOR: f64 = 0.80;

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
/// Returns a list of WindowBorders and the background color string for swaybg.
pub fn compute_border_colors(state: &WindowManager) -> (Vec<WindowBorders>, Option<String>) {
    let mut results = Vec::new();
    let all_edges = 0b1111u32; // all edges

    // Count cascade windows
    let mut n_cascade = 0usize;
    for win in &state.windows {
        if (win.tags & state.active_tags) != 0 && win.tiling_mode == TilingMode::Cascade {
            n_cascade += 1;
        }
    }

    let mut max_cascade_depth = 0i32;
    let mut bg_color: Option<String> = None;

    // Second pass: assign border colors
    for (idx, win) in state.windows.iter().enumerate() {
        if (win.tags & state.active_tags) == 0 {
            continue;
        }

        if win.tiling_mode == TilingMode::Cascade && n_cascade > 0 {
            // Compute cascade depth: count how many cascade windows come before this one
            let mut cascade_idx = 0usize;
            for (i, w) in state.windows.iter().enumerate() {
                if i >= idx {
                    break;
                }
                if (w.tags & state.active_tags) != 0 && w.tiling_mode == TilingMode::Cascade {
                    cascade_idx += 1;
                }
            }
            // depth: 0 for front (focused/last), n_cascade-1 for back
            let depth = (n_cascade - 1 - cascade_idx) as i32;

            let r = interp_channel(state.layout.border_r, CASCADE_DEPTH_FACTOR, depth);
            let g = interp_channel(state.layout.border_g, CASCADE_DEPTH_FACTOR, depth);
            let b = interp_channel(state.layout.border_b, CASCADE_DEPTH_FACTOR, depth);

            results.push(WindowBorders {
                window_idx: idx,
                edges: all_edges,
                width: state.layout.border_width,
                r, g, b, a: CASCADE_ALPHA,
            });

            if depth > max_cascade_depth {
                max_cascade_depth = depth;
            }
        } else {
            results.push(WindowBorders {
                window_idx: idx,
                edges: all_edges,
                width: state.layout.border_width,
                r: BORDER_COLOR_NORMAL_R,
                g: BORDER_COLOR_NORMAL_G,
                b: BORDER_COLOR_NORMAL_B,
                a: BORDER_COLOR_NORMAL_A,
            });
        }
    }

    // Set desktop background to the darkest cascade color
    if n_cascade > 0 {
        bg_color = Some(cascade_hex_color(
            state.layout.border_r,
            state.layout.border_g,
            state.layout.border_b,
            max_cascade_depth,
        ));
    }

    (results, bg_color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interp_channel_depth0() {
        // Depth 0 should return the base color unchanged
        let result = interp_channel(0x5C000000, 0.80, 0);
        assert_eq!(result >> 24, 0x5C);
    }

    #[test]
    fn test_interp_channel_depth1() {
        let result = interp_channel(0x90000000, 0.80, 1);
        let val = result >> 24;
        assert_eq!(val, ((0x90 as f64 * 0.80) as u8) as u32);
    }

    #[test]
    fn test_cascade_hex_color() {
        let color = cascade_hex_color(0x5C000000, 0x90000000, 0x60000000, 0);
        assert_eq!(color, "#5c9060");
    }
}
