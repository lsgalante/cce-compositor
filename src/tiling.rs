// Tiling formulas ported from clearwm.c

/// Cascade depth factor: each depth step multiplies channels by this
pub const CASCADE_DEPTH_FACTOR: f64 = 0.80;

/// Full alpha in high-byte-first fixed-point
pub const CASCADE_ALPHA: u32 = 0x000000FFu32;

/// Cascade base green (focused window) in fixed-point format
/// These come from the layout.border_r/g/b values, not hardcoded constants.
/// The C code uses #5c9060 as the default cascade base but it's actually
/// read from config. We just define the factor here.

/// Tile a window in cascade mode.
///
/// Returns (x, y, width, height) for the window at the given cascade index.
///
/// The cascade formula from C:
///   width  = screen_w - (gap + bw)*2 - offset*(n_cascade - 1)
///   height = screen_h - (gap + bw)*2 - offset*(n_cascade - 1)
///   x = gap + bw + idx * offset
///   y = bar_height + gap + bw + idx * offset
pub fn tile_cascade(
    screen_w: i32,
    screen_h: i32,
    gap: i32,
    bw: i32,
    offset: i32,
    bar_height: i32,
    n_cascade: i32,
    idx: i32,
) -> (i32, i32, i32, i32) {
    let width = screen_w - (gap + bw) * 2 - offset * (n_cascade - 1);
    let height = screen_h - (gap + bw) * 2 - offset * (n_cascade - 1);
    let width = if width < 1 { 1 } else { width };
    let height = if height < 1 { 1 } else { height };
    let x = gap + bw + idx * offset;
    let y = bar_height + gap + bw + idx * offset;
    (x, y, width, height)
}

/// Tile a window in grid mode.
///
/// Uses a 2-column grid layout.
///
/// The grid formula from C:
///   cols = 2
///   rows = (n_grid + cols - 1) / cols
///   width  = (screen_w - (cols + 1) * gap) / cols - 2 * bw
///   height = (screen_h - (rows + 1) * gap) / rows - 2 * bw
///   x = gap + bw + col * (width + 2 * bw + gap)
///   y = bar_height + gap + bw + row * (height + 2 * bw + gap)
pub fn tile_grid(
    screen_w: i32,
    screen_h: i32,
    gap: i32,
    bw: i32,
    bar_height: i32,
    n_grid: i32,
    idx: i32,
) -> (i32, i32, i32, i32) {
    let cols = 2i32;
    let row = idx / cols;
    let col = idx % cols;
    let rows = (n_grid + cols - 1) / cols;
    let width = (screen_w - (cols + 1) * gap) / cols - 2 * bw;
    let height = (screen_h - (rows + 1) * gap) / rows - 2 * bw;
    let width = if width < 1 { 1 } else { width };
    let height = if height < 1 { 1 } else { height };
    let x = gap + bw + col * (width + 2 * bw + gap);
    let y = bar_height + gap + bw + row * (height + 2 * bw + gap);
    (x, y, width, height)
}

/// Interpolate a fixed-point channel (0xRR000000) by factor^depth.
///
/// depth 0 returns the base color unchanged; each step darkens by factor.
/// Returns the value in 32-bit fixed-point (channel in high byte).
///
/// Ported from C: interp_channel()
pub fn interp_channel(fp_channel: u32, factor: f64, depth: i32) -> u32 {
    let base = (fp_channel >> 24) as u8;
    let f = factor.powi(depth);
    let val = ((base as f64) * f) as u8;
    (val as u32) << 24
}

/// Compute a "#RRGGBB" hex color string for a given cascade depth.
///
/// Takes the base border color channels in fixed-point format and
/// interpolates them toward black by CASCADE_DEPTH_FACTOR^depth.
///
/// Ported from C: cascade_hex_color()
pub fn cascade_hex_color(r: u32, g: u32, b: u32, depth: i32) -> String {
    let ri = interp_channel(r, CASCADE_DEPTH_FACTOR, depth);
    let gi = interp_channel(g, CASCADE_DEPTH_FACTOR, depth);
    let bi = interp_channel(b, CASCADE_DEPTH_FACTOR, depth);
    // Extract the high byte for display
    let rv = ri >> 24;
    let gv = gi >> 24;
    let bv = bi >> 24;
    format!("#{:02x}{:02x}{:02x}", rv, gv, bv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_cascade_single() {
        // Single cascade window should fill the screen minus gaps/borders
        let (x, y, w, h) = tile_cascade(1920, 1080, 18, 18, 32, 28, 1, 0);
        assert_eq!(x, 36); // gap + bw
        assert_eq!(y, 64); // bar_height + gap + bw
        // w = 1920 - (18+18)*2 - 32*0 = 1920 - 72 = 1848
        assert_eq!(w, 1848);
        // h = 1080 - (18+18)*2 - 32*0 = 1080 - 72 = 1008
        assert_eq!(h, 1008);
    }

    #[test]
    fn test_tile_cascade_multiple() {
        // 3 cascade windows, idx=2 (the back one)
        let (x, y, _w, _h) = tile_cascade(1920, 1080, 18, 18, 32, 28, 3, 2);
        assert_eq!(x, 36 + 2 * 32); // gap + bw + idx * offset
        assert_eq!(y, 64 + 2 * 32); // bar + gap + bw + idx * offset
    }

    #[test]
    fn test_tile_cascade_minimum_size() {
        // Very small screen, many windows — should clamp to 1
        let (_, _, w, h) = tile_cascade(100, 100, 18, 18, 32, 28, 10, 5);
        assert!(w >= 1);
        assert!(h >= 1);
    }

    #[test]
    fn test_tile_grid_two_windows() {
        let (x0, y0, w0, h0) = tile_grid(1920, 1080, 18, 18, 28, 2, 0);
        let (x1, y1, w1, h1) = tile_grid(1920, 1080, 18, 18, 28, 2, 1);

        // Both windows should have same dimensions
        assert_eq!(w0, w1);
        assert_eq!(h0, h1);

        // Window 1 is to the right of window 0
        assert!(x1 > x0);
        assert_eq!(y0, y1); // Same row

        // Width = (1920 - 3*18) / 2 - 2*18 = (1920-54)/2 - 36 = 933 - 36 = 897
        assert_eq!(w0, 897);
        // Height = (1080 - 2*18) / 1 - 2*18 = 1044 - 36 = 1008
        // Wait: rows = (2+2-1)/2 = 1, so height = (1080 - 2*18)/1 - 36 = 1044 - 36 = 1008
        assert_eq!(h0, 1008);
    }

    #[test]
    fn test_tile_grid_four_windows() {
        // 4 windows in 2x2 grid
        let (x0, y0, _, _) = tile_grid(1920, 1080, 18, 18, 28, 4, 0);
        let (x1, y1, _, _) = tile_grid(1920, 1080, 18, 18, 28, 4, 1);
        let (x2, y2, _, _) = tile_grid(1920, 1080, 18, 18, 28, 4, 2);
        let (x3, y3, _, _) = tile_grid(1920, 1080, 18, 18, 28, 4, 3);

        // Row 0: windows 0,1
        assert!(y0 == y1);
        assert!(x0 < x1);
        // Row 1: windows 2,3
        assert!(y2 == y3);
        assert!(x2 < x3);
        // Row 1 below row 0
        assert!(y2 > y0);
    }

    #[test]
    fn test_interp_channel_depth_zero() {
        // depth 0 should return the base color unchanged
        let result = interp_channel(0x5C000000, CASCADE_DEPTH_FACTOR, 0);
        assert_eq!(result >> 24, 0x5C);
    }

    #[test]
    fn test_interp_channel_depth_one() {
        let result = interp_channel(0x90000000, CASCADE_DEPTH_FACTOR, 1);
        // 0x90 * 0.80 = 0x90 * 0.80 = 144 * 0.80 = 115.2 → 115 = 0x73
        assert_eq!(result >> 24, 0x73);
    }

    #[test]
    fn test_interp_channel_depth_two() {
        let result = interp_channel(0x60000000, CASCADE_DEPTH_FACTOR, 2);
        // 0x60 * 0.80^2 = 96 * 0.64 = 61.44 → 61 = 0x3D
        assert_eq!(result >> 24, 0x3D);
    }

    #[test]
    fn test_cascade_hex_color_depth_zero() {
        let color = cascade_hex_color(0x5C000000, 0x90000000, 0x60000000, 0);
        assert_eq!(color, "#5c9060");
    }

    #[test]
    fn test_cascade_hex_color_depth_one() {
        let color = cascade_hex_color(0x5C000000, 0x90000000, 0x60000000, 1);
        // 0x5C*0.80=0x49, 0x90*0.80=0x73, 0x60*0.80=0x4C
        assert_eq!(color, "#49734c");
    }

    #[test]
    fn test_cascade_hex_color_dark_gray() {
        // Using #3e3e3e as base (normal border color)
        let color = cascade_hex_color(0x3E000000, 0x3E000000, 0x3E000000, 0);
        assert_eq!(color, "#3e3e3e");
    }
}
