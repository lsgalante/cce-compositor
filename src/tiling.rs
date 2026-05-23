// Tiling formulas ported from clearwm.c

/// Cascade depth factor: each depth step multiplies channels by this
pub const CASCADE_DEPTH_FACTOR: f64 = 0.80;

/// Full alpha, byte-replicated for River's color format
pub const CASCADE_ALPHA: u32 = 0xFFFFFFFFu32;

/// Cascade base green (focused window) in fixed-point format
/// These come from the layout.border_r/g/b values, not hardcoded constants.
/// The C code uses #5c9060 as the default cascade base but it's actually
/// read from config. We just define the factor here.

/// Tile a window in cascade mode.
///
/// Returns (x, y, width, height) for the window at the given cascade index.
///
/// Screen edges use per-side gaps; inter-window offset is cascade_offset.
///   width  = screen_w - gap_left - gap_right - bw*2 - cascade_offset*(n_cascade - 1)
///   height = screen_h - gap_top - gap_bottom - bw*2 - cascade_offset*(n_cascade - 1)
///   x = gap_left + bw + idx * cascade_offset
///   y = bar_height + gap_top + bw + idx * cascade_offset
pub fn tile_cascade(
    screen_w: i32,
    screen_h: i32,
    _gap: i32,
    gap_top: i32,
    gap_left: i32,
    gap_right: i32,
    gap_bottom: i32,
    bw: i32,
    cascade_offset: i32,
    bar_height: i32,
    n_cascade: i32,
    idx: i32,
) -> (i32, i32, i32, i32) {
    let width = screen_w - gap_left - gap_right - bw * 2 - cascade_offset * (n_cascade - 1);
    let height = screen_h - gap_top - gap_bottom - bw * 2 - cascade_offset * (n_cascade - 1);
    let width = if width < 1 { 1 } else { width };
    let height = if height < 1 { 1 } else { height };
    let x = gap_left + bw + idx * cascade_offset;
    let y = bar_height + gap_top + bw + idx * cascade_offset;
    (x, y, width, height)
}

/// Tile a window in grid mode.
///
/// Uses a 2-column grid layout.
///
/// Screen edges use per-side gaps; inter-window spacing uses `gap`.
///   cols = 2
///   rows = (n_grid + cols - 1) / cols
///   width  = (screen_w - gap_left - gap_right - (cols - 1) * gap) / cols - 2 * bw
///   height = (screen_h - gap_top - gap_bottom - (rows - 1) * gap) / rows - 2 * bw
///   x = gap_left + bw + col * (width + 2 * bw + gap)
///   y = bar_height + gap_top + bw + row * (height + 2 * bw + gap)
pub fn tile_grid(
    screen_w: i32,
    screen_h: i32,
    gap: i32,
    gap_top: i32,
    gap_left: i32,
    gap_right: i32,
    gap_bottom: i32,
    bw: i32,
    bar_height: i32,
    n_grid: i32,
    idx: i32,
) -> (i32, i32, i32, i32) {
    let cols = 2i32;
    let row = idx / cols;
    let col = idx % cols;
    let rows = (n_grid + cols - 1) / cols;
    let width = (screen_w - gap_left - gap_right - (cols - 1) * gap) / cols - 2 * bw;
    let height = (screen_h - gap_top - gap_bottom - (rows - 1) * gap) / rows - 2 * bw;
    let width = if width < 1 { 1 } else { width };
    let height = if height < 1 { 1 } else { height };
    let x = gap_left + bw + col * (width + 2 * bw + gap);
    let y = bar_height + gap_top + bw + row * (height + 2 * bw + gap);
    (x, y, width, height)
}

/// Tile a window in vsplit mode (vertical splits — windows side by side).
///
/// Each window gets an equal share of the horizontal space.
///
/// Screen edges use per-side gaps; inter-window spacing uses `gap`.
///   n = total windows in vsplit
///   width  = (screen_w - gap_left - gap_right - (n - 1) * gap) / n - 2 * bw
///   height = screen_h - gap_top - gap_bottom - bw * 2 - bar_height
///   x = gap_left + bw + idx * (width + 2 * bw + gap)
///   y = bar_height + gap_top + bw
pub fn tile_vsplit(
    screen_w: i32,
    screen_h: i32,
    gap: i32,
    gap_top: i32,
    gap_left: i32,
    gap_right: i32,
    gap_bottom: i32,
    bw: i32,
    bar_height: i32,
    n_vsplit: i32,
    idx: i32,
) -> (i32, i32, i32, i32) {
    let n = if n_vsplit < 1 { 1 } else { n_vsplit };
    let width = (screen_w - gap_left - gap_right - (n - 1) * gap) / n - 2 * bw;
    let height = screen_h - gap_top - gap_bottom - bw * 2 - bar_height;
    let width = if width < 1 { 1 } else { width };
    let height = if height < 1 { 1 } else { height };
    let x = gap_left + bw + idx * (width + 2 * bw + gap);
    let y = bar_height + gap_top + bw;
    (x, y, width, height)
}

/// Tile a window in hsplit mode (horizontal splits — windows stacked vertically).
///
/// Each window gets an equal share of the vertical space.
///
/// Screen edges use per-side gaps; inter-window spacing uses `gap`.
///   n = total windows in hsplit
///   width  = screen_w - gap_left - gap_right - bw * 2
///   height = (screen_h - bar_height - gap_top - gap_bottom - (n - 1) * gap) / n - 2 * bw
///   x = gap_left + bw
///   y = bar_height + gap_top + bw + idx * (height + 2 * bw + gap)
pub fn tile_hsplit(
    screen_w: i32,
    screen_h: i32,
    gap: i32,
    gap_top: i32,
    gap_left: i32,
    gap_right: i32,
    gap_bottom: i32,
    bw: i32,
    bar_height: i32,
    n_hsplit: i32,
    idx: i32,
) -> (i32, i32, i32, i32) {
    let n = if n_hsplit < 1 { 1 } else { n_hsplit };
    let width = screen_w - gap_left - gap_right - bw * 2;
    let height = (screen_h - bar_height - gap_top - gap_bottom - (n - 1) * gap) / n - 2 * bw;
    let width = if width < 1 { 1 } else { width };
    let height = if height < 1 { 1 } else { height };
    let x = gap_left + bw;
    let y = bar_height + gap_top + bw + idx * (height + 2 * bw + gap);
    (x, y, width, height)
}

/// Tile a window in fullscreen mode — fills the screen minus gaps, bar, and borders.
///
/// Only the focused window is visible; other fullscreen windows are skipped.
/// A fullscreen window occupies the entire screen (0, 0, screen_w, screen_h).
pub fn tile_fullscreen(
    screen_w: i32,
    screen_h: i32,
    _gap_top: i32,
    _gap_left: i32,
    _gap_right: i32,
    _gap_bottom: i32,
    _bw: i32,
    _bar_height: i32,
) -> (i32, i32, i32, i32) {
    (0, 0, screen_w, screen_h)
}

/// Interpolate a byte-replicated 32-bit channel (0xVVVVVVVV) by factor^depth.
///
/// depth 0 returns the base color unchanged; each step darkens by factor.
/// Returns byte-replicated 32-bit value for River's color format.
///
/// Ported from C: interp_channel()
pub fn interp_channel(fp_channel: u32, factor: f64, depth: i32) -> u32 {
    let base = (fp_channel & 0xFF) as u8;
    let f = factor.powi(depth);
    let val = ((base as f64) * f) as u8;
    val as u32 * 0x01010101
}

/// Compute a "#RRGGBB" hex color string for a given cascade depth.
///
/// Takes the base border color channels in byte-replicated format and
/// interpolates them toward black by CASCADE_DEPTH_FACTOR^depth.
///
/// Ported from C: cascade_hex_color()
pub fn cascade_hex_color(r: u32, g: u32, b: u32, depth: i32) -> String {
    let ri = interp_channel(r, CASCADE_DEPTH_FACTOR, depth);
    let gi = interp_channel(g, CASCADE_DEPTH_FACTOR, depth);
    let bi = interp_channel(b, CASCADE_DEPTH_FACTOR, depth);
    // Extract the low byte for display
    let rv = ri & 0xFF;
    let gv = gi & 0xFF;
    let bv = bi & 0xFF;
    format!("#{:02x}{:02x}{:02x}", rv, gv, bv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_cascade_single() {
        // Single cascade window should fill the screen minus gaps/borders
        let (x, y, w, h) = tile_cascade(1920, 1080, 18, 18, 18, 18, 18, 18, 32, 28, 1, 0);
        assert_eq!(x, 36); // gap_left + bw
        assert_eq!(y, 64); // bar_height + gap_top + bw
                           // w = 1920 - 18 - 18 - 18*2 - 32*0 = 1920 - 72 = 1848
        assert_eq!(w, 1848);
        // h = 1080 - 18 - 18 - 18*2 - 32*0 = 1080 - 72 = 1008
        assert_eq!(h, 1008);
    }

    #[test]
    fn test_tile_cascade_multiple() {
        // 3 cascade windows, idx=2 (the back one)
        let (x, y, _w, _h) = tile_cascade(1920, 1080, 18, 18, 18, 18, 18, 18, 32, 28, 3, 2);
        assert_eq!(x, 36 + 2 * 32); // gap_left + bw + idx * offset
        assert_eq!(y, 64 + 2 * 32); // bar + gap_top + bw + idx * offset
    }

    #[test]
    fn test_tile_cascade_minimum_size() {
        // Very small screen, many windows — should clamp to 1
        let (_, _, w, h) = tile_cascade(100, 100, 18, 18, 18, 18, 18, 18, 32, 28, 10, 5);
        assert!(w >= 1);
        assert!(h >= 1);
    }

    #[test]
    fn test_tile_cascade_asymmetric_gaps() {
        // Asymmetric screen gaps: top=10, left=20, right=30, bottom=40
        let (x, y, w, h) = tile_cascade(1920, 1080, 12, 10, 20, 30, 40, 6, 24, 28, 1, 0);
        assert_eq!(x, 20 + 6);        // gap_left + bw = 26
        assert_eq!(y, 28 + 10 + 6);   // bar_height + gap_top + bw = 44
                                       // w = 1920 - 20 - 30 - 6*2 = 1920 - 62 = 1858
        assert_eq!(w, 1858);
        // h = 1080 - 10 - 40 - 6*2 = 1080 - 62 = 1018
        assert_eq!(h, 1018);
    }

    #[test]
    fn test_tile_grid_two_windows() {
        let (x0, y0, w0, h0) = tile_grid(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 2, 0);
        let (x1, y1, w1, h1) = tile_grid(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 2, 1);

        // Both windows should have same dimensions
        assert_eq!(w0, w1);
        assert_eq!(h0, h1);

        // Window 1 is to the right of window 0
        assert!(x1 > x0);
        assert_eq!(y0, y1); // Same row

        // Width = (1920 - 18 - 18 - 1*18) / 2 - 2*18 = (1920-54)/2 - 36 = 933 - 36 = 897
        assert_eq!(w0, 897);
        // rows=1, Height = (1080 - 18 - 18 - 0*18) / 1 - 2*18 = 1044 - 36 = 1008
        assert_eq!(h0, 1008);
    }

    #[test]
    fn test_tile_grid_four_windows() {
        // 4 windows in 2x2 grid
        let (x0, y0, _, _) = tile_grid(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 4, 0);
        let (x1, y1, _, _) = tile_grid(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 4, 1);
        let (x2, y2, _, _) = tile_grid(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 4, 2);
        let (x3, y3, _, _) = tile_grid(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 4, 3);

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
    fn test_tile_grid_asymmetric_gaps() {
        // Asymmetric screen gaps with 2 windows in 1 row
        let (x0, y0, w0, h0) = tile_grid(1920, 1080, 12, 10, 20, 30, 40, 6, 28, 2, 0);
        let (x1, _y1, w1, _h1) = tile_grid(1920, 1080, 12, 10, 20, 30, 40, 6, 28, 2, 1);

        // Width = (1920 - 20 - 30 - 1*12) / 2 - 2*6 = (1920-62)/2 - 12 = 929 - 12 = 917
        assert_eq!(w0, 917);
        assert_eq!(w0, w1);

        // x0 = gap_left + bw = 20 + 6 = 26
        assert_eq!(x0, 26);
        // y0 = bar_height + gap_top + bw = 28 + 10 + 6 = 44
        assert_eq!(y0, 44);
        // rows=1, Height = (1080 - 10 - 40 - 0*12) / 1 - 2*6 = 1030 - 12 = 1018
        assert_eq!(h0, 1018);

        // x1 = gap_left + bw + 1*(917 + 2*6 + 12) = 26 + 941 = 967
        assert_eq!(x1, 967);
    }

    #[test]
    fn test_interp_channel_depth_zero() {
        // depth 0 should return the base color unchanged
        let result = interp_channel(0x5C5C5C5C, CASCADE_DEPTH_FACTOR, 0);
        assert_eq!(result, 0x5C5C5C5C);
    }

    #[test]
    fn test_interp_channel_depth_one() {
        let result = interp_channel(0x90909090, CASCADE_DEPTH_FACTOR, 1);
        // 0x90 * 0.80 = 144 * 0.80 = 115.2 → 115 = 0x73
        assert_eq!(result & 0xFF, 0x73);
    }

    #[test]
    fn test_interp_channel_depth_two() {
        let result = interp_channel(0x60606060, CASCADE_DEPTH_FACTOR, 2);
        // 0x60 * 0.80^2 = 96 * 0.64 = 61.44 → 61 = 0x3D
        assert_eq!(result & 0xFF, 0x3D);
    }

    #[test]
    fn test_cascade_hex_color_depth_zero() {
        let color = cascade_hex_color(0x5C5C5C5C, 0x90909090, 0x60606060, 0);
        assert_eq!(color, "#5c9060");
    }

    #[test]
    fn test_cascade_hex_color_depth_one() {
        let color = cascade_hex_color(0x5C5C5C5C, 0x90909090, 0x60606060, 1);
        // 0x5C*0.80=0x49, 0x90*0.80=0x73, 0x60*0.80=0x4C
        assert_eq!(color, "#49734c");
    }

    #[test]
    fn test_cascade_hex_color_dark_gray() {
        // Using #3e3e3e as base (normal border color)
        let color = cascade_hex_color(0x3E3E3E3E, 0x3E3E3E3E, 0x3E3E3E3E, 0);
        assert_eq!(color, "#3e3e3e");
    }

    #[test]
    fn test_tile_vsplit_single() {
        // Single vsplit window fills screen minus gaps/borders/bar
        let (x, y, w, h) = tile_vsplit(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 1, 0);
        assert_eq!(x, 36); // gap_left + bw
        assert_eq!(y, 64); // bar_height + gap_top + bw
                           // w = (1920 - 18 - 18 - 0*18) / 1 - 2*18 = 1884 - 36 = 1848
        assert_eq!(w, 1848);
        // h = 1080 - 18 - 18 - 18*2 - 28 = 1080 - 100 = 980
        assert_eq!(h, 980);
    }

    #[test]
    fn test_tile_vsplit_two() {
        // Two vsplit windows side by side
        let (x0, y0, w0, h0) = tile_vsplit(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 2, 0);
        let (x1, y1, w1, h1) = tile_vsplit(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 2, 1);

        // Same dimensions
        assert_eq!(w0, w1);
        assert_eq!(h0, h1);
        // Same y (same row)
        assert_eq!(y0, y1);
        // Window 1 is to the right
        assert!(x1 > x0);

        // w = (1920 - 18 - 18 - 1*18) / 2 - 2*18 = (1920-54)/2 - 36 = 933 - 36 = 897
        assert_eq!(w0, 897);
    }

    #[test]
    fn test_tile_vsplit_minimum_size() {
        let (_, _, w, h) = tile_vsplit(100, 100, 18, 18, 18, 18, 18, 18, 28, 10, 5);
        assert!(w >= 1);
        assert!(h >= 1);
    }

    #[test]
    fn test_tile_vsplit_asymmetric_gaps() {
        // Asymmetric screen gaps with 2 vsplit windows
        let (x0, y0, w0, h0) = tile_vsplit(1920, 1080, 12, 10, 20, 30, 40, 6, 28, 2, 0);
        let (x1, _y1, w1, _h1) = tile_vsplit(1920, 1080, 12, 10, 20, 30, 40, 6, 28, 2, 1);

        // w = (1920 - 20 - 30 - 1*12) / 2 - 2*6 = (1920-62)/2 - 12 = 929 - 12 = 917
        assert_eq!(w0, 917);
        assert_eq!(w0, w1);

        // x0 = gap_left + bw = 20 + 6 = 26
        assert_eq!(x0, 26);
        // y0 = bar_height + gap_top + bw = 28 + 10 + 6 = 44
        assert_eq!(y0, 44);
        // h = 1080 - 10 - 40 - 6*2 - 28 = 1080 - 90 = 990
        assert_eq!(h0, 990);

        // x1 = gap_left + bw + 1*(917 + 2*6 + 12) = 26 + 941 = 967
        assert_eq!(x1, 967);
    }

    #[test]
    fn test_tile_hsplit_single() {
        // Single hsplit window fills screen minus gaps/borders/bar
        let (x, y, w, h) = tile_hsplit(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 1, 0);
        assert_eq!(x, 36); // gap_left + bw
        assert_eq!(y, 64); // bar_height + gap_top + bw
                           // w = 1920 - 18 - 18 - 18*2 = 1920 - 72 = 1848
        assert_eq!(w, 1848);
        // h = (1080 - 28 - 18 - 18 - 0*18) / 1 - 2*18 = 1016 - 36 = 980
        assert_eq!(h, 980);
    }

    #[test]
    fn test_tile_hsplit_two() {
        // Two hsplit windows stacked vertically
        let (x0, y0, w0, h0) = tile_hsplit(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 2, 0);
        let (x1, y1, w1, h1) = tile_hsplit(1920, 1080, 18, 18, 18, 18, 18, 18, 28, 2, 1);

        // Same dimensions
        assert_eq!(w0, w1);
        assert_eq!(h0, h1);
        // Same x (same column)
        assert_eq!(x0, x1);
        // Window 1 is below window 0
        assert!(y1 > y0);

        // h = (1080 - 28 - 18 - 18 - 1*18) / 2 - 2*18 = (1080-82)/2 - 36 = 499 - 36 = 463
        assert_eq!(h0, 463);
    }

    #[test]
    fn test_tile_hsplit_minimum_size() {
        let (_, _, w, h) = tile_hsplit(100, 100, 18, 18, 18, 18, 18, 18, 28, 10, 5);
        assert!(w >= 1);
        assert!(h >= 1);
    }

    #[test]
    fn test_tile_hsplit_asymmetric_gaps() {
        // Asymmetric screen gaps with 2 hsplit windows
        let (x0, y0, w0, h0) = tile_hsplit(1920, 1080, 12, 10, 20, 30, 40, 6, 28, 2, 0);
        let (_x1, y1, w1, h1) = tile_hsplit(1920, 1080, 12, 10, 20, 30, 40, 6, 28, 2, 1);

        // w = 1920 - 20 - 30 - 6*2 = 1920 - 62 = 1858
        assert_eq!(w0, 1858);
        assert_eq!(w0, w1);

        // x0 = gap_left + bw = 20 + 6 = 26
        assert_eq!(x0, 26);
        // y0 = bar_height + gap_top + bw = 28 + 10 + 6 = 44
        assert_eq!(y0, 44);

        // h = (1080 - 28 - 10 - 40 - 1*12) / 2 - 2*6 = (1080-90)/2 - 12 = 495 - 12 = 483
        assert_eq!(h0, 483);
        assert_eq!(h0, h1);

        // y1 = bar_height + gap_top + bw + 1*(483 + 2*6 + 12) = 44 + 507 = 551
        assert_eq!(y1, 551);
    }

    #[test]
    fn test_tile_fullscreen_basic() {
        let (x, y, w, h) = tile_fullscreen(1920, 1080, 18, 18, 18, 18, 6, 28);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
        assert_eq!(w, 1920);
        assert_eq!(h, 1080);
    }

    #[test]
    fn test_tile_fullscreen_asymmetric_gaps() {
        let (x, y, w, h) = tile_fullscreen(1920, 1080, 10, 20, 30, 40, 6, 28);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
        assert_eq!(w, 1920);
        assert_eq!(h, 1080);
    }

    #[test]
    fn test_tile_fullscreen_minimum_size() {
        let (x, y, w, h) = tile_fullscreen(50, 50, 18, 18, 18, 18, 6, 28);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
        assert_eq!(w, 50);
        assert_eq!(h, 50);
    }
}
