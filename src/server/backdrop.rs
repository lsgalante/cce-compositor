//! What is behind a status segment — computed, not sampled.
//!
//! The bar is a Wayland client: it draws into its own buffer and can never
//! see what it is composited over, so a translucent module box leaves its
//! text at the mercy of whatever the desktop happens to be showing there.
//! This module measures that backdrop compositor-side and the status socket
//! pushes the answer to each segment (`backdrop` topic), which is the only
//! way the bar can adapt its own contrast.
//!
//! The measurement is **geometry, not pixels**. The desktop background is
//! drawn by the compositor itself from a declarative spec
//! ([`crate::policy::background::grid_frame`]), so what sits under a segment
//! is known exactly: the fraction of its rect falling on a grid cell versus
//! on the gap between cells. That makes this a few rect intersections on the
//! CPU rather than a GPU readback — no pipeline stall, no frame-latency
//! feedback loop from sampling a frame the bar is already part of, and an
//! exact answer instead of a sampled one.
//!
//! Windows are the exception. The reserved strip keeps *tiled* windows out
//! from under the bar (`arrange.rs` shrinks the usable box by `bar_height`),
//! but a floating or fullscreen window — or a panned camera — can still slide
//! one beneath a segment, and a client's pixels are not knowable here. Any
//! such overlap reports maximum spread: "unknown, assume the worst", which
//! the bar answers with its outline treatment rather than a guess.

use crate::policy::api::Rgba;
use crate::policy::background::GridFrame;

/// One segment's backdrop, quantized to 0–100.
///
/// Quantized for two reasons: [`crate::status_server::StatusUpdate`] derives
/// `Eq` and its equality IS the resend gate, so a float would both break the
/// derive and defeat the gate — sub-percent wobble during a camera pan would
/// push a line every frame to every segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackdropSample {
    /// WCAG relative luminance of the backdrop, 0 (black) – 100 (white).
    pub luma: u8,
    /// How much the backdrop VARIES across the segment, 0 (uniform) – 100.
    /// High spread means no single text color works over the whole run and
    /// an outline is the only honest answer; it is also what an unknown
    /// backdrop (a window in the way) reports.
    pub spread: u8,
}

/// An axis-aligned rect in layout px — the same space `GridFrame::tree_pos`
/// and a window's `box_geom` are expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x + self.w
    }

    fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// Overlap area with `other`, in px².
    pub fn intersect_area(&self, other: &Rect) -> i64 {
        let w = (self.right().min(other.right()) - self.x.max(other.x)).max(0) as i64;
        let h = (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0) as i64;
        w * h
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.intersect_area(other) > 0
    }

    /// The overlapping rect, or None when they do not meet.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let w = self.right().min(other.right()) - x;
        let h = self.bottom().min(other.bottom()) - y;
        (w > 0 && h > 0).then_some(Rect { x, y, w, h })
    }
}

/// One sRGB channel to linear light (the WCAG transfer function).
fn to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance, 0–1. Perceptual weighting, not a channel mean:
/// the eye reads green as most of the brightness, and a naive average would
/// call the blue-gray gap color and a mid-gray equally bright.
fn relative_luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * to_linear(rgb[0]) + 0.7152 * to_linear(rgb[1]) + 0.0722 * to_linear(rgb[2])
}

/// `over` composited onto `under`, both PREMULTIPLIED (which is how
/// [`Rgba`] carries grid colors — see `GridSpec::gap_color`). Returns
/// straight rgb, since that is all luminance needs.
fn over(over_c: Rgba, under: [f32; 3]) -> [f32; 3] {
    let a = over_c.0[3].clamp(0.0, 1.0);
    [
        over_c.0[0] + under[0] * (1.0 - a),
        over_c.0[1] + under[1] * (1.0 - a),
        over_c.0[2] + under[2] * (1.0 - a),
    ]
}

/// The fraction of `rect` covered by grid cells, 0–1.
///
/// Only the cell columns/rows that can reach `rect` are visited — derived
/// from the period rather than by scanning the whole lattice, which at a far
/// zoom-out is thousands of cells that a 27px-tall segment cannot touch.
///
/// Cell corner radius and the fade inset are deliberately ignored: both
/// soften a cell's edge by a few px, which moves the coverage fraction far
/// less than the quantization to whole percent does.
fn cell_coverage(frame: &GridFrame, rect: &Rect) -> f32 {
    let Some(cells) = &frame.cells else {
        return 0.0;
    };
    let Some((tx, ty)) = frame.tree_pos else {
        return 0.0;
    };
    let area = (rect.w as i64) * (rect.h as i64);
    if area <= 0 {
        return 0.0;
    }
    let px = frame.period_px_exact_x;
    let py = frame.period_px_exact_y;
    if !(px > 0.5) || !(py > 0.5) {
        return 0.0;
    }

    // Tree-local span the rect can touch, widened by one cell so a cell
    // whose origin sits before the rect but whose body reaches into it is
    // still visited.
    let lx0 = (rect.x - tx) as f64;
    let lx1 = (rect.right() - tx) as f64;
    let ly0 = (rect.y - ty) as f64;
    let ly1 = (rect.bottom() - ty) as f64;
    let col0 = (((lx0 - cells.cell_w_px as f64) / px).floor() as i64).clamp(0, cells.cols as i64);
    let col1 = ((lx1 / px).ceil() as i64).clamp(0, cells.cols as i64);
    let row0 = (((ly0 - cells.cell_h_px as f64) / py).floor() as i64).clamp(0, cells.rows as i64);
    let row1 = ((ly1 / py).ceil() as i64).clamp(0, cells.rows as i64);

    let mut covered: i64 = 0;
    for row in row0..=row1 {
        let cy = ty + (row as f64 * py).round() as i32;
        for col in col0..=col1 {
            let cx = tx + (col as f64 * px).round() as i32;
            let cell = Rect { x: cx, y: cy, w: cells.cell_w_px, h: cells.cell_h_px };
            covered += rect.intersect_area(&cell);
        }
    }
    (covered as f32 / area as f32).clamp(0.0, 1.0)
}

/// Measure a block of RGBA pixels — the window-content path, where the
/// backdrop is not derivable geometry and has to be looked at.
///
/// Spread comes from the 10th and 90th luminance percentiles rather than the
/// full range, so one stray highlight (a cursor, an icon, an anti-aliased
/// edge) does not report a whole terminal as high-variance. It is the same
/// quantity the grid path computes analytically: how far apart the light and
/// dark parts of this patch are.
pub fn measure_pixels(rgba: &[u8]) -> Option<BackdropSample> {
    let n = rgba.len() / 4;
    if n == 0 {
        return None;
    }
    let mut lumas: Vec<f32> = Vec::with_capacity(n);
    let mut sum = 0.0f32;
    for px in rgba.chunks_exact(4) {
        let l = relative_luminance([px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0]);
        sum += l;
        lumas.push(l);
    }
    lumas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p10 = lumas[n / 10];
    let p90 = lumas[n - 1 - n / 10];
    Some(BackdropSample {
        luma: ((sum / n as f32).clamp(0.0, 1.0) * 100.0).round() as u8,
        spread: ((p90 - p10).clamp(0.0, 1.0) * 100.0).round() as u8,
    })
}

/// Fold a window-content sample covering `coverage` (0-1) of a segment into
/// the desktop sample for the rest of it.
///
/// The third spread term is the one that is easy to miss: two patches can each
/// be perfectly uniform and still leave the text straddling a hard edge
/// between them — a black terminal ending halfway across a segment that sits
/// on a light gap. That boundary is exactly as unreadable as a busy texture,
/// and only the difference between the two means shows it.
pub fn blend(desktop: BackdropSample, window: BackdropSample, coverage: f32) -> BackdropSample {
    let c = coverage.clamp(0.0, 1.0);
    let dl = desktop.luma as f32 / 100.0;
    let wl = window.luma as f32 / 100.0;
    let luma = c * wl + (1.0 - c) * dl;
    let edge = 2.0 * c.min(1.0 - c) * (wl - dl).abs();
    let spread = (desktop.spread as f32 / 100.0)
        .max(window.spread as f32 / 100.0)
        .max(edge);
    BackdropSample {
        luma: (luma.clamp(0.0, 1.0) * 100.0).round() as u8,
        spread: (spread.clamp(0.0, 1.0) * 100.0).round() as u8,
    }
}

/// What a segment reports when its backdrop cannot be determined at all —
/// mid luminance, full spread, which drives the outline.
pub const UNKNOWN: BackdropSample = BackdropSample { luma: 50, spread: 100 };

/// Measure the backdrop under `rect`.
///
/// `base` is the opaque desktop background color the grid is drawn onto (the
/// output's background rect), so a gap or cell color carrying alpha resolves
/// against the same thing the screen shows.
///
/// `occluded` says a window overlaps the rect; its content is not knowable
/// here, so the sample degrades to "unknown" — mid luminance and full
/// spread — rather than confidently reporting the desktop that is no longer
/// what the text sits on.
pub fn measure(frame: &GridFrame, spec_gap: Rgba, base: [f32; 3], rect: Rect, occluded: bool) -> BackdropSample {
    if occluded {
        return UNKNOWN;
    }

    let gap_rgb = over(spec_gap, base);
    let gap_luma = relative_luminance(gap_rgb);

    let (cell_luma, f) = match &frame.cells {
        // The cell color carries the density fade in its alpha, so a
        // faded-out lattice correctly resolves toward the gap color.
        Some(cells) => (relative_luminance(over(cells.color, gap_rgb)), cell_coverage(frame, &rect)),
        None => (gap_luma, 0.0),
    };

    let luma = f * cell_luma + (1.0 - f) * gap_luma;

    // Spread is the area split WEIGHTED by how different the two colors
    // actually are: a rect straddling cell and gap is only a problem for the
    // text when the two read as different brightnesses. A lattice drawn in
    // two similar tones is uniform as far as legibility is concerned, however
    // the area happens to divide.
    let split = 2.0 * f.min(1.0 - f);
    let spread = split * (cell_luma - gap_luma).abs();

    BackdropSample {
        luma: (luma.clamp(0.0, 1.0) * 100.0).round() as u8,
        spread: (spread.clamp(0.0, 1.0) * 100.0).round() as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::api::{GridFadeMode, GridSpec};
    use crate::policy::background::{grid_frame, GridCells};
    use crate::policy::camera::Camera;

    const BLACK: Rgba = Rgba([0.0, 0.0, 0.0, 1.0]);
    const WHITE: Rgba = Rgba([1.0, 1.0, 1.0, 1.0]);

    fn frame_with(cells: Option<GridCells>, period: f64) -> GridFrame {
        GridFrame {
            tree_pos: Some((0, 0)),
            period_px_x: period as i32,
            period_px_y: period as i32,
            period_px_exact_x: period,
            period_px_exact_y: period,
            backdrop_w: 1000,
            backdrop_h: 1000,
            cells,
            first_col: 0,
            first_row: 0,
        }
    }

    fn cells(w: i32, h: i32, color: Rgba) -> GridCells {
        GridCells {
            cell_w_px: w,
            cell_h_px: h,
            cols: 8,
            rows: 8,
            color,
            corner_radius_px: 0,
            fade_inset_px: 0,
        }
    }

    #[test]
    fn a_rect_wholly_on_a_cell_is_uniform_at_the_cell_color() {
        // The legibility case that motivated all this: a segment sitting
        // entirely on a black cell must report black AND report it
        // confidently, or the bar has no reason to change anything.
        let frame = frame_with(Some(cells(500, 500, BLACK)), 516.0);
        let s = measure(&frame, WHITE, [1.0, 1.0, 1.0], Rect { x: 100, y: 100, w: 200, h: 27 }, false);
        assert_eq!(s.luma, 0);
        assert_eq!(s.spread, 0);
    }

    #[test]
    fn a_rect_wholly_in_the_gap_is_uniform_at_the_gap_color() {
        let frame = frame_with(Some(cells(100, 100, BLACK)), 200.0);
        // x 120..180 falls between the cell at 0..100 and the one at 200.
        let s = measure(&frame, WHITE, [1.0, 1.0, 1.0], Rect { x: 120, y: 120, w: 60, h: 27 }, false);
        assert_eq!(s.luma, 100);
        assert_eq!(s.spread, 0);
    }

    #[test]
    fn straddling_a_cell_edge_reports_spread() {
        // Half on a black cell, half on a white gap: no single text color
        // works, which is exactly what a high spread tells the bar.
        let frame = frame_with(Some(cells(100, 100, BLACK)), 200.0);
        let s = measure(&frame, WHITE, [1.0, 1.0, 1.0], Rect { x: 50, y: 20, w: 100, h: 27 }, false);
        assert!(s.spread > 90, "spread was {}", s.spread);
        assert!((40..=60).contains(&s.luma), "luma was {}", s.luma);
    }

    #[test]
    fn a_two_tone_lattice_of_similar_colors_is_not_spread() {
        // Same 50/50 area split as above, but the two tones are close, so
        // there is no legibility problem to report.
        let near_white = Rgba([0.97, 0.97, 0.97, 1.0]);
        let frame = frame_with(Some(cells(100, 100, near_white)), 200.0);
        let rect = Rect { x: 50, y: 20, w: 100, h: 27 };
        let s = measure(&frame, WHITE, [1.0, 1.0, 1.0], rect, false);
        // The same area split black-on-white reports ~100 (above), so the
        // weighting — not the geometry — is what separates these two.
        assert!(s.spread < 10, "spread was {}", s.spread);
    }

    #[test]
    fn an_occluding_window_reports_unknown_rather_than_the_desktop() {
        let frame = frame_with(Some(cells(500, 500, BLACK)), 516.0);
        let rect = Rect { x: 100, y: 100, w: 200, h: 27 };
        let clear = measure(&frame, WHITE, [1.0, 1.0, 1.0], rect, false);
        let hidden = measure(&frame, WHITE, [1.0, 1.0, 1.0], rect, true);
        assert_eq!(clear.spread, 0);
        assert_eq!(hidden.spread, 100);
        assert_ne!(clear.luma, hidden.luma);
    }

    #[test]
    fn no_cells_is_the_flat_gap_color() {
        let frame = frame_with(None, 200.0);
        let s = measure(&frame, BLACK, [0.0, 0.0, 0.0], Rect { x: 0, y: 0, w: 100, h: 27 }, false);
        assert_eq!(s.luma, 0);
        assert_eq!(s.spread, 0);
    }

    #[test]
    fn luminance_is_perceptual_not_a_channel_mean() {
        // Pure green and pure blue have the same channel mean; the eye does
        // not see them as remotely the same brightness.
        let green = relative_luminance([0.0, 1.0, 0.0]);
        let blue = relative_luminance([0.0, 0.0, 1.0]);
        assert!(green > blue * 5.0, "green {} blue {}", green, blue);
    }

    #[test]
    fn coverage_visits_only_the_cells_that_can_reach_the_rect() {
        // A far zoom-out puts thousands of cells on screen; a bar segment
        // touches a handful. The result must still be right when the rect
        // sits deep inside the lattice rather than at its origin.
        let frame = frame_with(Some(cells(10, 10, BLACK)), 20.0);
        let s = measure(&frame, WHITE, [1.0, 1.0, 1.0], Rect { x: 1000, y: 1000, w: 40, h: 27 }, false);
        // Beyond cols/rows (8), so no cell reaches it — pure gap.
        assert_eq!(s.luma, 100);
    }

    fn solid(luma_byte: u8, n: usize) -> Vec<u8> {
        std::iter::repeat([luma_byte, luma_byte, luma_byte, 255]).take(n).flatten().collect()
    }

    #[test]
    fn a_flat_patch_of_pixels_has_no_spread() {
        let s = measure_pixels(&solid(0, 1000)).unwrap();
        assert_eq!(s.luma, 0);
        assert_eq!(s.spread, 0);
        let s = measure_pixels(&solid(255, 1000)).unwrap();
        assert_eq!(s.luma, 100);
        assert_eq!(s.spread, 0);
    }

    #[test]
    fn half_black_half_white_pixels_report_full_spread() {
        let mut px = solid(0, 500);
        px.extend(solid(255, 500));
        let s = measure_pixels(&px).unwrap();
        assert!(s.spread > 95, "spread was {}", s.spread);
        assert!((45..=55).contains(&s.luma), "luma was {}", s.luma);
    }

    #[test]
    fn a_lone_highlight_does_not_read_as_a_busy_backdrop() {
        // A cursor or an icon on an otherwise flat terminal. The percentile
        // spread is what keeps a handful of bright pixels from pinning the
        // outline on over content the text reads fine against.
        let mut px = solid(0, 990);
        px.extend(solid(255, 10));
        let s = measure_pixels(&px).unwrap();
        assert_eq!(s.spread, 0, "spread was {}", s.spread);
    }

    #[test]
    fn measure_pixels_rejects_an_empty_read() {
        assert!(measure_pixels(&[]).is_none());
    }

    #[test]
    fn blending_a_window_over_part_of_a_segment_moves_the_luma() {
        let desktop = BackdropSample { luma: 0, spread: 0 };
        let window = BackdropSample { luma: 100, spread: 0 };
        assert_eq!(blend(desktop, window, 0.0).luma, 0);
        assert_eq!(blend(desktop, window, 1.0).luma, 100);
        assert_eq!(blend(desktop, window, 0.5).luma, 50);
    }

    #[test]
    fn a_hard_edge_between_two_flat_patches_is_itself_spread() {
        // A black terminal ending halfway across a segment that sits on a
        // light gap: both halves uniform, the text across the seam is not.
        let desktop = BackdropSample { luma: 100, spread: 0 };
        let window = BackdropSample { luma: 0, spread: 0 };
        assert_eq!(blend(desktop, window, 0.5).spread, 100);
        // ...and at the edges of coverage there is no seam to worry about.
        assert_eq!(blend(desktop, window, 0.02).spread, 4);
    }

    #[test]
    fn blending_keeps_the_worse_of_the_two_spreads() {
        let desktop = BackdropSample { luma: 50, spread: 10 };
        let window = BackdropSample { luma: 50, spread: 80 };
        assert_eq!(blend(desktop, window, 0.5).spread, 80);
    }

    #[test]
    fn a_real_grid_frame_measures_without_panicking() {
        // Exercises the real grid_frame output rather than a hand-built one,
        // so a field-meaning drift in the policy crate surfaces here.
        let spec = GridSpec {
            gap_color: Rgba([0.686, 0.796, 0.867, 1.0]),
            cell_color: BLACK,
            cell_w: 512.0,
            cell_h: 512.0,
            gap_width: 16.0,
            cell_corner_radius: 0,
            cell_fade_inset: 4,
            fade_mode: GridFadeMode::Quadratic,
        };
        let cam = Camera { pan_x: 0.0, pan_y: 0.0, zoom: 1.0 };
        let frame = grid_frame(&spec, cam, 1920, 1080, 0, 0);
        let s = measure(&frame, spec.gap_color, [0.0, 0.0, 0.0], Rect { x: 40, y: 0, w: 200, h: 27 }, false);
        assert!(s.luma <= 100);
        assert!(s.spread <= 100);
    }
}
