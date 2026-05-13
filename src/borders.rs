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
pub fn compute_border_colors(state: &WindowManager) -> Vec<WindowBorders> {
    let mut results = Vec::new();
    let all_edges = 0b1111u32; // all edges

    // Count cascade windows
    let mut n_cascade = 0usize;
    for win in &state.windows {
        if (win.tags & state.active_tags) != 0 && win.tiling_mode == TilingMode::Cascade {
            n_cascade += 1;
        }
    }

    // Assign border colors
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
                r,
                g,
                b,
                a: CASCADE_ALPHA,
            });
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

    results
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
}
