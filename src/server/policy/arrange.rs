// Pure layout computation extracted from `WindowManager::arrange_views()`.
//
// Functions here take plain-data snapshots and return placement plans; the
// mechanism side (`window_manager.rs`) builds the snapshots from FFI state and
// applies the plans to the scene graph. Extraction happens section by section;
// currently covers the status-bar layout engine.

use super::api::{Rect, WindowRole};
use super::tiling::TilingMode;

/// Which screen edge/region a status-bar window docks to.
/// Set from the app_id suffix or config; `Unspecified` resolves to `TopLeft`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusEdge {
    Unspecified,
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    Left,
    Right,
}

/// Snapshot of one status-bar window, in `self.windows` iteration order.
/// Windows being interactively dragged are excluded before layout.
pub struct StatusBarItem {
    pub app_id: String,
    pub edge: StatusEdge,
    /// max(box_geom.width, box_geom.height) — the bar's previous major length.
    pub prev_len: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatusBarPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub struct StatusBarLayoutParams {
    /// The output's layout box.
    pub output: Rect,
    pub bar_height: u32,
    pub hide_mode: bool,
    /// Pixels of bar left peeking when hide_mode pushes top bars offscreen.
    pub hide_mode_preview: i32,
}

/// The usable (tileable) area of an output: the layout box, shrunk by the
/// layer-shell non-exclusive area and by one bar-height per screen edge that
/// has a status bar docked to it. Top bars reserve no space in hide mode.
///
/// `non_exclusive` is relative to the output box; a zero-sized rect means no
/// layer-shell exclusion. `status_edges` holds the raw edge of every live
/// status-bar window (`Unspecified` resolves to `TopLeft`).
pub fn compute_usable_area(
    output: Rect,
    non_exclusive: Rect,
    bar_height: i32,
    status_hide_mode: bool,
    status_edges: &[StatusEdge],
) -> Rect {
    let mut usable = output;

    if non_exclusive.width > 0 && non_exclusive.height > 0 {
        usable.x = output.x + non_exclusive.x;
        usable.y = output.y + non_exclusive.y;
        usable.width = non_exclusive.width;
        usable.height = non_exclusive.height;
    }

    let mut has_top = false;
    let mut has_bottom = false;
    let mut has_left = false;
    let mut has_right = false;

    for &edge in status_edges {
        let edge = if edge == StatusEdge::Unspecified { StatusEdge::TopLeft } else { edge };
        match edge {
            StatusEdge::Unspecified | StatusEdge::TopLeft | StatusEdge::TopCenter | StatusEdge::TopRight => {
                if !status_hide_mode {
                    has_top = true;
                }
            }
            StatusEdge::BottomLeft | StatusEdge::BottomCenter | StatusEdge::BottomRight => {
                has_bottom = true;
            }
            StatusEdge::Left => {
                has_left = true;
            }
            StatusEdge::Right => {
                has_right = true;
            }
        }
    }

    if has_top {
        usable.y += bar_height;
        usable.height -= bar_height;
    }
    if has_bottom {
        usable.height -= bar_height;
    }
    if has_left {
        usable.x += bar_height;
        usable.width -= bar_height;
    }
    if has_right {
        usable.width -= bar_height;
    }

    usable
}

/// How `arrange_views` treats a window this frame. `Background`/`StatusBar`
/// windows get fixed geometry regardless of visibility; `Hidden` windows are
/// disabled in the scene; `Overlay` windows get the overlay slot unless they
/// are mid-drag (then they arrange as `Normal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowClass {
    Background,
    StatusBar,
    Hidden,
    Overlay,
    Normal,
}

pub fn classify_window(
    role: WindowRole,
    minimized: bool,
    closing_or_init: bool,
    mode: TilingMode,
    is_moving: bool,
) -> WindowClass {
    match role {
        WindowRole::Background => WindowClass::Background,
        WindowRole::StatusBar => WindowClass::StatusBar,
        _ => {
            if minimized || closing_or_init {
                WindowClass::Hidden
            } else if mode == TilingMode::Overlay && !is_moving {
                WindowClass::Overlay
            } else {
                WindowClass::Normal
            }
        }
    }
}

/// Legacy border width, baked into the overlay geometry formulas.
const BW: i32 = 0;

/// Overlay windows keep 16px clear above for decorations.
const OVERLAY_DEC_H: i32 = 16;

/// Windows further than this many pixels outside the output are culled.
const OFFSCREEN_MARGIN: f64 = 50.0;

pub const OVERLAY_UNFOCUSED_OPACITY: f32 = 0.85;
pub const NORMAL_UNFOCUSED_OPACITY: f32 = 0.90;

pub fn window_opacity(is_focused: bool, opacity_enabled: bool, unfocused: f32) -> f32 {
    if is_focused || !opacity_enabled { 1.0 } else { unfocused }
}

/// Per-output placement context: the physical box, the usable area from
/// `compute_usable_area`, and the desktop viewport (pan/zoom).
pub struct PlacementCtx {
    pub phys: Rect,
    pub usable: Rect,
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom: f64,
}

impl PlacementCtx {
    fn virtual_to_screen(&self, vx: f64, vy: f64) -> (i32, i32) {
        (
            self.phys.x + ((vx - self.pan_x) * self.zoom) as i32,
            self.phys.y + ((vy - self.pan_y) * self.zoom) as i32,
        )
    }

    fn is_offscreen(&self, x: i32, y: i32, scaled_w: f64, scaled_h: f64) -> bool {
        let viewport_w = self.phys.width as f64;
        let viewport_h = self.phys.height as f64;
        (x as f64 + scaled_w + OFFSCREEN_MARGIN) < self.phys.x as f64
            || (x as f64 - OFFSCREEN_MARGIN) > (self.phys.x as f64 + viewport_w)
            || (y as f64 + scaled_h + OFFSCREEN_MARGIN) < self.phys.y as f64
            || (y as f64 - OFFSCREEN_MARGIN) > (self.phys.y as f64 + viewport_h)
    }
}

pub struct OverlaySnapshot {
    pub box_geom: Rect,
    pub min_width: i32,
    pub is_cloud: bool,
    pub ssd: bool,
    pub decorations_size: (i32, i32),
}

pub struct OverlayParams {
    pub overlay_width: i32,
    pub border_gap: i32,
    pub position_right: bool,
    pub cloud_position_default: Option<[i32; 2]>,
}

pub struct OverlayPlacement {
    pub pos: (i32, i32),
    /// Persistent geometry to store back on the window, if placement chose it.
    pub box_geom_write: Option<Rect>,
    pub virtual_pos: (f64, f64),
    pub size: (u32, u32),
}

/// Place the primary overlay window: fresh windows get the configured overlay
/// slot (left or right edge, full usable height); cloud windows snap to their
/// configured default position; anything else keeps its stored geometry.
pub fn place_overlay_window(
    snap: &OverlaySnapshot,
    p: &OverlayParams,
    ctx: &PlacementCtx,
) -> OverlayPlacement {
    let bw = BW;
    let g = p.border_gap;
    let dec_h = std::cmp::max(bw, OVERLAY_DEC_H);

    let mut sp_x = snap.box_geom.x;
    let mut sp_y = snap.box_geom.y;
    let mut sp_w = snap.box_geom.width;
    let mut sp_h = snap.box_geom.height;
    let mut box_geom_write = None;

    if sp_w == 0 || sp_h == 0 {
        sp_w = if snap.min_width > 32 {
            std::cmp::max(p.overlay_width, snap.min_width)
        } else {
            p.overlay_width
        };
        sp_h = (ctx.usable.height - (dec_h + bw) - 2 * g).max(1);

        sp_x = if p.position_right {
            ctx.usable.x + ctx.usable.width - sp_w - g + bw
        } else {
            ctx.usable.x + g + bw
        };
        sp_y = ctx.usable.y + dec_h + g;

        box_geom_write = Some(Rect { x: sp_x, y: sp_y, width: sp_w, height: sp_h });
    } else if snap.is_cloud {
        if let Some(pos) = p.cloud_position_default {
            sp_x = ctx.usable.x + pos[0];
            sp_y = ctx.usable.y + pos[1];
            box_geom_write = Some(Rect { x: sp_x, y: sp_y, width: sp_w, height: sp_h });
        }
    }

    let vx = ctx.pan_x + (sp_x - ctx.phys.x) as f64 / ctx.zoom;
    let vy = ctx.pan_y + (sp_y - ctx.phys.y) as f64 / ctx.zoom;

    let mut target_w = sp_w;
    let mut target_h = sp_h;
    if !snap.ssd {
        let (dec_w, dec_h) = snap.decorations_size;
        target_w = (sp_w - dec_w).max(1);
        target_h = (sp_h - dec_h).max(1);
    }

    OverlayPlacement {
        pos: (sp_x, sp_y),
        box_geom_write,
        virtual_pos: (vx, vy),
        size: (target_w as u32, target_h as u32),
    }
}

/// State-machine step for entering/leaving Maximized mode. `Enter` tells the
/// mechanism to save the given restore size (plus the window's current virtual
/// position) and set `was_maximized`; `Exit` restores the saved geometry.
/// Leaving with an invalid saved size does nothing (`was_maximized` stays set).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaximizedTransition {
    Enter { width: i32, height: i32 },
    Exit { width: i32, height: i32, virtual_x: f64, virtual_y: f64 },
}

pub fn maximized_transition(
    is_maximized_mode: bool,
    was_maximized: bool,
    box_size: (i32, i32),
    min_size: (i32, i32),
    saved_size: (i32, i32),
    saved_virtual: (f64, f64),
) -> Option<MaximizedTransition> {
    if is_maximized_mode && !was_maximized {
        let mut w = box_size.0;
        let mut h = box_size.1;
        if w <= 0 {
            w = if min_size.0 > 32 { min_size.0 } else { 800 };
        }
        if h <= 0 {
            h = if min_size.1 > 32 { min_size.1 } else { 600 };
        }
        Some(MaximizedTransition::Enter { width: w, height: h })
    } else if !is_maximized_mode && was_maximized {
        if saved_size.0 > 0 && saved_size.1 > 0 {
            Some(MaximizedTransition::Exit {
                width: saved_size.0,
                height: saved_size.1,
                virtual_x: saved_virtual.0,
                virtual_y: saved_virtual.1,
            })
        } else {
            None
        }
    } else {
        None
    }
}

pub struct NormalSnapshot {
    pub mode: TilingMode,
    pub box_geom: Rect,
    pub min_size: (i32, i32),
    pub virtual_pos: (f64, f64),
    /// Live interactive-resize dimensions, if a resize op is in progress.
    pub active_resize: Option<(u32, u32)>,
    pub is_cloud: bool,
    pub saved_maximized_size: (i32, i32),
    pub saved_maximized_virtual: (f64, f64),
}

pub struct NormalParams {
    pub gap_right: i32,
    pub gap_top: i32,
    pub cloud_position_default: Option<[i32; 2]>,
    pub desktop_grid_scale: f64,
}

pub struct NormalPlacement {
    pub pos: (i32, i32),
    pub scale: f64,
    pub size: (u32, u32),
    /// Fullscreen windows report all edges tiled to the client.
    pub tiled_all_edges: bool,
    /// Offscreen-culling result; `None` leaves the window's flag untouched.
    pub hidden: Option<bool>,
    /// Maximized grid-snap moves the window's virtual position.
    pub virtual_write: Option<(f64, f64)>,
}

/// Place a non-overlay window according to its tiling mode: `Popup` docks to
/// the usable area's top-right (or the cloud default position), `Fullscreen`
/// covers the physical output, `Maximized` snaps to cover every desktop-grid
/// cell its saved geometry touches, and everything else pans on the virtual
/// surface under the current viewport.
pub fn place_normal_window(
    snap: &NormalSnapshot,
    p: &NormalParams,
    ctx: &PlacementCtx,
) -> NormalPlacement {
    match snap.mode {
        TilingMode::Popup => {
            let fw = if snap.box_geom.width > 0 {
                snap.box_geom.width
            } else if snap.min_size.0 > 32 {
                snap.min_size.0
            } else {
                360
            };
            let fh = if snap.box_geom.height > 0 {
                snap.box_geom.height
            } else if snap.min_size.1 > 32 {
                snap.min_size.1
            } else {
                100
            };

            let (fx, fy) = if snap.is_cloud && p.cloud_position_default.is_some() {
                let pos = p.cloud_position_default.unwrap();
                (ctx.usable.x + pos[0], ctx.usable.y + pos[1])
            } else {
                (ctx.usable.x + ctx.usable.width - fw - p.gap_right, ctx.usable.y + p.gap_top)
            };

            NormalPlacement {
                pos: (fx, fy),
                scale: 1.0,
                size: (fw as u32, fh as u32),
                tiled_all_edges: false,
                hidden: None,
                virtual_write: None,
            }
        }
        TilingMode::Fullscreen => NormalPlacement {
            pos: (ctx.phys.x, ctx.phys.y),
            scale: 1.0,
            size: (ctx.phys.width as u32, ctx.phys.height as u32),
            tiled_all_edges: true,
            hidden: None,
            virtual_write: None,
        },
        TilingMode::Maximized => {
            // Resize to fully fill all desktop-grid cells the saved geometry
            // is fully/partially inside of.
            let scale = p.desktop_grid_scale;

            let x1 = snap.saved_maximized_virtual.0;
            let y1 = snap.saved_maximized_virtual.1;
            let w = snap.saved_maximized_size.0 as f64;
            let h = snap.saved_maximized_size.1 as f64;
            let x2 = x1 + w;
            let y2 = y1 + h;

            let col_min = (x1 / scale).floor() as i32;
            let col_max = ((x2 / scale).ceil() as i32 - 1).max(col_min);
            let row_min = (y1 / scale).floor() as i32;
            let row_max = ((y2 / scale).ceil() as i32 - 1).max(row_min);

            let snapped_x1 = col_min as f64 * scale;
            let snapped_x2 = (col_max + 1) as f64 * scale;
            let snapped_y1 = row_min as f64 * scale;
            let snapped_y2 = (row_max + 1) as f64 * scale;

            let fw = snapped_x2 - snapped_x1;
            let fh = snapped_y2 - snapped_y1;

            let (final_x, final_y) = ctx.virtual_to_screen(snapped_x1, snapped_y1);

            NormalPlacement {
                pos: (final_x, final_y),
                scale: ctx.zoom,
                size: (fw as u32, fh as u32),
                tiled_all_edges: false,
                hidden: Some(ctx.is_offscreen(final_x, final_y, fw * ctx.zoom, fh * ctx.zoom)),
                virtual_write: Some((snapped_x1, snapped_y1)),
            }
        }
        _ => {
            // Regular pannable window on the virtual surface.
            let fw = if let Some(resize_size) = snap.active_resize {
                resize_size.0 as i32
            } else if snap.box_geom.width > 0 {
                snap.box_geom.width
            } else if snap.min_size.0 > 32 {
                snap.min_size.0
            } else {
                800
            };
            let fh = if let Some(resize_size) = snap.active_resize {
                resize_size.1 as i32
            } else if snap.box_geom.height > 0 {
                snap.box_geom.height
            } else if snap.min_size.1 > 32 {
                snap.min_size.1
            } else {
                600
            };

            let (final_x, final_y) = ctx.virtual_to_screen(snap.virtual_pos.0, snap.virtual_pos.1);

            NormalPlacement {
                pos: (final_x, final_y),
                scale: ctx.zoom,
                size: (fw as u32, fh as u32),
                tiled_all_edges: false,
                hidden: Some(ctx.is_offscreen(final_x, final_y, fw as f64 * ctx.zoom, fh as f64 * ctx.zoom)),
                virtual_write: None,
            }
        }
    }
}

const SPACING: i32 = 12;
const MARGIN: i32 = 12;

const LEFT_ORDER: &[&str] = &["viewport", "window"];
const RIGHT_ORDER: &[&str] = &["tray", "cpu", "memory", "brightness", "volume", "battery", "clock"];

fn left_sort_key(app_id: &str) -> usize {
    let name = app_id.strip_prefix("cce-status-interface-left-")
        .or_else(|| app_id.strip_prefix("cce-status-left-"))
        .unwrap_or(app_id);
    LEFT_ORDER.iter().position(|&m| m == name).unwrap_or(99)
}

fn right_sort_key(app_id: &str) -> usize {
    let name = app_id.strip_prefix("cce-status-interface-right-")
        .or_else(|| app_id.strip_prefix("cce-status-right-"))
        .unwrap_or(app_id);
    RIGHT_ORDER.iter().position(|&m| m == name).unwrap_or(99)
}

/// Bar length to use: previous major length, or 100 for a fresh bar.
fn bar_len(prev_len: i32) -> u32 {
    if prev_len > 0 { prev_len as u32 } else { 100 }
}

/// Lay out status-bar windows on one output: horizontal groups on the top and
/// bottom edges (left/center/right within each), vertical stacks on the left
/// and right edges, and full-width bars across the top.
///
/// Returns one placement per item, index-aligned with `items`.
pub fn layout_status_bars(
    items: &[StatusBarItem],
    p: &StatusBarLayoutParams,
) -> Vec<Option<StatusBarPlacement>> {
    let wlr_box = p.output;
    let bar_h = p.bar_height;
    let spacing = SPACING;
    let margin = MARGIN;

    let mut top_left: Vec<usize> = Vec::new();
    let mut top_center: Vec<usize> = Vec::new();
    let mut top_right: Vec<usize> = Vec::new();
    let mut bottom_left: Vec<usize> = Vec::new();
    let mut bottom_center: Vec<usize> = Vec::new();
    let mut bottom_right: Vec<usize> = Vec::new();
    let mut left_side: Vec<usize> = Vec::new();
    let mut right_side: Vec<usize> = Vec::new();
    let mut full_top: Vec<usize> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        let edge = if item.edge == StatusEdge::Unspecified {
            StatusEdge::TopLeft
        } else {
            item.edge
        };
        match edge {
            StatusEdge::TopLeft => top_left.push(idx),
            StatusEdge::TopCenter => top_center.push(idx),
            StatusEdge::TopRight => top_right.push(idx),
            StatusEdge::BottomLeft => bottom_left.push(idx),
            StatusEdge::BottomCenter => bottom_center.push(idx),
            StatusEdge::BottomRight => bottom_right.push(idx),
            StatusEdge::Left => left_side.push(idx),
            StatusEdge::Right => right_side.push(idx),
            _ => full_top.push(idx),
        }
    }

    log::info!("[ArrangeStatus] top_left_len={}, top_center_len={}, top_right_len={}, left_side_len={}", top_left.len(), top_center.len(), top_right.len(), left_side.len());

    let sort_left = |list: &mut Vec<usize>| {
        list.sort_by_key(|&i| left_sort_key(&items[i].app_id));
    };
    let sort_right = |list: &mut Vec<usize>| {
        list.sort_by_key(|&i| right_sort_key(&items[i].app_id));
    };

    sort_left(&mut top_left);
    sort_left(&mut top_center);
    sort_right(&mut top_right);
    sort_left(&mut bottom_left);
    sort_left(&mut bottom_center);
    sort_right(&mut bottom_right);

    let mut placements: Vec<Option<StatusBarPlacement>> = vec![None; items.len()];

    // 1. Top Edge
    let status_y_top = if p.hide_mode {
        wlr_box.y - (bar_h as i32 - p.hide_mode_preview)
    } else {
        wlr_box.y
    };

    let mut top_right_width_needed = 0;
    for &i in &top_right {
        let w = bar_len(items[i].prev_len);
        top_right_width_needed += w as i32 + spacing;
    }
    let top_right_boundary = wlr_box.x + wlr_box.width - margin - top_right_width_needed;

    // 1a. Left Group (TopLeft / nw)
    let mut cur_left_x = wlr_box.x + margin;
    for &i in &top_left {
        let mut w = bar_len(items[i].prev_len);
        let max_allowed_w = top_right_boundary - cur_left_x - spacing;
        if w as i32 > max_allowed_w {
            w = std::cmp::max(max_allowed_w, 20) as u32;
        }
        log::info!("[TopLeftLayout] app_id={} x={}, w={}", items[i].app_id, cur_left_x, w);
        placements[i] = Some(StatusBarPlacement { x: cur_left_x, y: status_y_top, width: w, height: bar_h });
        cur_left_x += w as i32 + spacing;
    }

    // 1b. Center Group (TopCenter / n)
    let mut top_center_width_needed = 0;
    for &i in &top_center {
        let w = bar_len(items[i].prev_len);
        top_center_width_needed += w as i32 + spacing;
    }
    if top_center_width_needed > 0 {
        top_center_width_needed -= spacing;
    }
    let center_start_x = wlr_box.x + (wlr_box.width - top_center_width_needed) / 2;
    let mut cur_center_x = std::cmp::max(center_start_x, cur_left_x + spacing);

    for &i in &top_center {
        let mut w = bar_len(items[i].prev_len);
        let max_allowed_w = top_right_boundary - cur_center_x - spacing;
        if w as i32 > max_allowed_w {
            w = std::cmp::max(max_allowed_w, 20) as u32;
        }
        log::info!("[TopCenterLayout] app_id={} x={}, w={}", items[i].app_id, cur_center_x, w);
        placements[i] = Some(StatusBarPlacement { x: cur_center_x, y: status_y_top, width: w, height: bar_h });
        cur_center_x += w as i32 + spacing;
    }

    // 1c. Right Group (TopRight / ne)
    let mut cur_right_x = wlr_box.x + wlr_box.width - margin;
    for &i in top_right.iter().rev() {
        let w = bar_len(items[i].prev_len);
        let x = cur_right_x - w as i32;
        placements[i] = Some(StatusBarPlacement { x, y: status_y_top, width: w, height: bar_h });
        cur_right_x = x - spacing;
    }

    for &i in &full_top {
        placements[i] = Some(StatusBarPlacement { x: wlr_box.x, y: status_y_top, width: wlr_box.width as u32, height: bar_h });
    }

    // 2. Bottom Edge
    let status_y_bottom = wlr_box.y + wlr_box.height - bar_h as i32;

    let mut bottom_right_width_needed = 0;
    for &i in &bottom_right {
        let w = bar_len(items[i].prev_len);
        bottom_right_width_needed += w as i32 + spacing;
    }
    let bottom_right_boundary = wlr_box.x + wlr_box.width - margin - bottom_right_width_needed;

    // 2a. Left Group (BottomLeft / sw)
    let mut cur_left_x = wlr_box.x + margin;
    for &i in &bottom_left {
        let mut w = bar_len(items[i].prev_len);
        let max_allowed_w = bottom_right_boundary - cur_left_x - spacing;
        if w as i32 > max_allowed_w {
            w = std::cmp::max(max_allowed_w, 20) as u32;
        }
        placements[i] = Some(StatusBarPlacement { x: cur_left_x, y: status_y_bottom, width: w, height: bar_h });
        cur_left_x += w as i32 + spacing;
    }

    // 2b. Center Group (BottomCenter / s)
    let mut bottom_center_width_needed = 0;
    for &i in &bottom_center {
        let w = bar_len(items[i].prev_len);
        bottom_center_width_needed += w as i32 + spacing;
    }
    if bottom_center_width_needed > 0 {
        bottom_center_width_needed -= spacing;
    }
    let center_start_x = wlr_box.x + (wlr_box.width - bottom_center_width_needed) / 2;
    let mut cur_center_x = std::cmp::max(center_start_x, cur_left_x + spacing);

    for &i in &bottom_center {
        let mut w = bar_len(items[i].prev_len);
        let max_allowed_w = bottom_right_boundary - cur_center_x - spacing;
        if w as i32 > max_allowed_w {
            w = std::cmp::max(max_allowed_w, 20) as u32;
        }
        placements[i] = Some(StatusBarPlacement { x: cur_center_x, y: status_y_bottom, width: w, height: bar_h });
        cur_center_x += w as i32 + spacing;
    }

    // 2c. Right Group (BottomRight / se)
    let mut cur_right_x = wlr_box.x + wlr_box.width - margin;
    for &i in bottom_right.iter().rev() {
        let w = bar_len(items[i].prev_len);
        let x = cur_right_x - w as i32;
        placements[i] = Some(StatusBarPlacement { x, y: status_y_bottom, width: w, height: bar_h });
        cur_right_x = x - spacing;
    }

    // 3. Left Edge (Vertical stacking)
    let mut left_total_height = 0;
    for &i in &left_side {
        let actual_h = bar_len(items[i].prev_len);
        left_total_height += actual_h as i32;
    }
    if !left_side.is_empty() {
        left_total_height += (left_side.len() as i32 - 1) * spacing;
    }
    let mut cur_left_y = wlr_box.y + (wlr_box.height - left_total_height) / 2;

    for &i in &left_side {
        let actual_h = bar_len(items[i].prev_len);
        placements[i] = Some(StatusBarPlacement { x: wlr_box.x, y: cur_left_y, width: bar_h, height: actual_h });
        cur_left_y += actual_h as i32 + spacing;
    }

    // 4. Right Edge (Vertical stacking)
    let mut right_total_height = 0;
    for &i in &right_side {
        let actual_h = bar_len(items[i].prev_len);
        right_total_height += actual_h as i32;
    }
    if !right_side.is_empty() {
        right_total_height += (right_side.len() as i32 - 1) * spacing;
    }
    let mut cur_right_y = wlr_box.y + (wlr_box.height - right_total_height) / 2;

    for &i in &right_side {
        let actual_h = bar_len(items[i].prev_len);
        placements[i] = Some(StatusBarPlacement { x: wlr_box.x + wlr_box.width - bar_h as i32, y: cur_right_y, width: bar_h, height: actual_h });
        cur_right_y += actual_h as i32 + spacing;
    }

    placements
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> StatusBarLayoutParams {
        StatusBarLayoutParams {
            output: Rect { x: 0, y: 0, width: 1920, height: 1080 },
            bar_height: 30,
            hide_mode: false,
            hide_mode_preview: 5,
        }
    }

    fn item(app_id: &str, edge: StatusEdge, prev_len: i32) -> StatusBarItem {
        StatusBarItem { app_id: app_id.to_string(), edge, prev_len }
    }

    #[test]
    fn top_groups_flow_from_edges() {
        let items = vec![
            item("cce-status-left-viewport", StatusEdge::TopLeft, 0),
            item("cce-status-left-window", StatusEdge::TopLeft, 0),
            item("cce-status-right-clock", StatusEdge::TopRight, 200),
        ];
        let p = layout_status_bars(&items, &params());
        // Left group flows right from the margin; fresh bars default to width 100.
        assert_eq!(p[0], Some(StatusBarPlacement { x: 12, y: 0, width: 100, height: 30 }));
        assert_eq!(p[1], Some(StatusBarPlacement { x: 124, y: 0, width: 100, height: 30 }));
        // Right group is placed from the right edge inward.
        assert_eq!(p[2], Some(StatusBarPlacement { x: 1908 - 200, y: 0, width: 200, height: 30 }));
    }

    #[test]
    fn right_group_sorted_by_module_order() {
        let items = vec![
            item("cce-status-right-clock", StatusEdge::TopRight, 100),
            item("cce-status-right-battery", StatusEdge::TopRight, 100),
        ];
        let p = layout_status_bars(&items, &params());
        // RIGHT_ORDER puts battery before clock left-to-right, so clock hugs the edge.
        assert_eq!(p[0].unwrap().x, 1808);
        assert_eq!(p[1].unwrap().x, 1808 - 12 - 100);
    }

    #[test]
    fn hide_mode_pushes_top_bars_offscreen_with_preview() {
        let mut prm = params();
        prm.hide_mode = true;
        let items = vec![item("cce-status-left-viewport", StatusEdge::Unspecified, 0)];
        let p = layout_status_bars(&items, &prm);
        // Unspecified resolves to TopLeft; y = 0 - (30 - 5).
        assert_eq!(p[0].unwrap().y, -25);
    }

    #[test]
    fn usable_area_reserves_bar_edges() {
        let output = Rect { x: 0, y: 0, width: 1920, height: 1080 };
        let no_excl = Rect { x: 0, y: 0, width: 0, height: 0 };

        // No bars, no exclusion: the full output.
        assert_eq!(compute_usable_area(output, no_excl, 30, false, &[]), output);

        // Top + left bars each reserve one bar-height.
        let edges = [StatusEdge::TopLeft, StatusEdge::Left];
        assert_eq!(
            compute_usable_area(output, no_excl, 30, false, &edges),
            Rect { x: 30, y: 30, width: 1890, height: 1050 }
        );

        // Hide mode releases the top reservation but not the others.
        assert_eq!(
            compute_usable_area(output, no_excl, 30, true, &edges),
            Rect { x: 30, y: 0, width: 1890, height: 1080 }
        );

        // Layer-shell non-exclusive area applies before bar reservations.
        let excl = Rect { x: 10, y: 20, width: 1900, height: 1040 };
        assert_eq!(
            compute_usable_area(output, excl, 30, false, &[StatusEdge::BottomCenter]),
            Rect { x: 10, y: 20, width: 1900, height: 1010 }
        );
    }

    #[test]
    fn window_roles_from_app_id() {
        assert_eq!(WindowRole::from_app_id(Some("cce-wallpaper")), WindowRole::Background);
        assert_eq!(WindowRole::from_app_id(Some("cce-status-interface-right-clock")), WindowRole::StatusBar);
        assert_eq!(WindowRole::from_app_id(Some("firefox")), WindowRole::Normal);
        assert_eq!(WindowRole::from_app_id(None), WindowRole::Normal);
    }

    #[test]
    fn classification_precedence() {
        // Role wins over visibility: a minimized wallpaper still arranges as background.
        assert_eq!(
            classify_window(WindowRole::Background, true, true, TilingMode::Floating, false),
            WindowClass::Background
        );
        assert_eq!(
            classify_window(WindowRole::StatusBar, false, false, TilingMode::Floating, false),
            WindowClass::StatusBar
        );
        // Minimized or closing/init normal windows are hidden.
        assert_eq!(
            classify_window(WindowRole::Normal, true, false, TilingMode::Grid, false),
            WindowClass::Hidden
        );
        // Overlay mode gets the overlay slot — unless mid-drag.
        assert_eq!(
            classify_window(WindowRole::Normal, false, false, TilingMode::Overlay, false),
            WindowClass::Overlay
        );
        assert_eq!(
            classify_window(WindowRole::Normal, false, false, TilingMode::Overlay, true),
            WindowClass::Normal
        );
        assert_eq!(
            classify_window(WindowRole::Normal, false, false, TilingMode::Cascade, false),
            WindowClass::Normal
        );
    }

    fn ctx() -> PlacementCtx {
        PlacementCtx {
            phys: Rect { x: 0, y: 0, width: 1920, height: 1080 },
            usable: Rect { x: 0, y: 30, width: 1920, height: 1050 },
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
        }
    }

    #[test]
    fn fresh_overlay_gets_configured_slot() {
        let placement = place_overlay_window(
            &OverlaySnapshot {
                box_geom: Rect { x: 0, y: 0, width: 0, height: 0 },
                min_width: 0,
                is_cloud: false,
                ssd: true,
                decorations_size: (0, 16),
            },
            &OverlayParams {
                overlay_width: 400,
                border_gap: 8,
                position_right: true,
                cloud_position_default: None,
            },
            &ctx(),
        );
        // Right slot: x = usable right edge - width - gap; height fills the
        // usable area minus the 16px decoration strip and both gaps.
        assert_eq!(placement.pos, (1512, 54));
        assert_eq!(placement.size, (400, 1018));
        assert_eq!(placement.box_geom_write, Some(Rect { x: 1512, y: 54, width: 400, height: 1018 }));
        assert_eq!(placement.virtual_pos, (1512.0, 54.0));
    }

    #[test]
    fn overlay_without_ssd_shrinks_by_decorations() {
        let placement = place_overlay_window(
            &OverlaySnapshot {
                box_geom: Rect { x: 100, y: 100, width: 400, height: 500 },
                min_width: 0,
                is_cloud: false,
                ssd: false,
                decorations_size: (2, 18),
            },
            &OverlayParams { overlay_width: 400, border_gap: 8, position_right: false, cloud_position_default: None },
            &ctx(),
        );
        // Existing geometry is kept; the client is sized minus decorations.
        assert_eq!(placement.pos, (100, 100));
        assert_eq!(placement.size, (398, 482));
        assert_eq!(placement.box_geom_write, None);
    }

    #[test]
    fn maximized_transitions() {
        // Entering with no usable geometry falls back to 800x600.
        assert_eq!(
            maximized_transition(true, false, (0, 0), (0, 0), (0, 0), (0.0, 0.0)),
            Some(MaximizedTransition::Enter { width: 800, height: 600 })
        );
        // Entering keeps real geometry.
        assert_eq!(
            maximized_transition(true, false, (640, 480), (0, 0), (0, 0), (0.0, 0.0)),
            Some(MaximizedTransition::Enter { width: 640, height: 480 })
        );
        // Steady states do nothing.
        assert_eq!(maximized_transition(true, true, (640, 480), (0, 0), (640, 480), (0.0, 0.0)), None);
        assert_eq!(maximized_transition(false, false, (640, 480), (0, 0), (0, 0), (0.0, 0.0)), None);
        // Exit restores the saved geometry; invalid saved size is a no-op.
        assert_eq!(
            maximized_transition(false, true, (0, 0), (0, 0), (640, 480), (10.0, 20.0)),
            Some(MaximizedTransition::Exit { width: 640, height: 480, virtual_x: 10.0, virtual_y: 20.0 })
        );
        assert_eq!(maximized_transition(false, true, (0, 0), (0, 0), (0, 480), (10.0, 20.0)), None);
    }

    #[test]
    fn maximized_snaps_to_grid_cells() {
        let snap = NormalSnapshot {
            mode: TilingMode::Maximized,
            box_geom: Rect { x: 0, y: 0, width: 100, height: 50 },
            min_size: (0, 0),
            virtual_pos: (150.0, 120.0),
            active_resize: None,
            is_cloud: false,
            saved_maximized_size: (100, 50),
            saved_maximized_virtual: (150.0, 120.0),
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, cloud_position_default: None, desktop_grid_scale: 100.0 };
        let placement = place_normal_window(&snap, &p, &ctx());
        // Saved geometry spans grid columns 1-2 and row 1 → snapped to
        // (100,100) with size 200x100.
        assert_eq!(placement.virtual_write, Some((100.0, 100.0)));
        assert_eq!(placement.pos, (100, 100));
        assert_eq!(placement.size, (200, 100));
        assert_eq!(placement.hidden, Some(false));
        assert_eq!(placement.scale, 1.0);
    }

    #[test]
    fn popup_docks_top_right_of_usable_area() {
        let snap = NormalSnapshot {
            mode: TilingMode::Popup,
            box_geom: Rect { x: 0, y: 0, width: 0, height: 0 },
            min_size: (0, 0),
            virtual_pos: (0.0, 0.0),
            active_resize: None,
            is_cloud: false,
            saved_maximized_size: (0, 0),
            saved_maximized_virtual: (0.0, 0.0),
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, cloud_position_default: None, desktop_grid_scale: 100.0 };
        let placement = place_normal_window(&snap, &p, &ctx());
        // Defaults to 360x100, docked inside the usable area (below the bar).
        assert_eq!(placement.pos, (1920 - 360 - 10, 30 + 6));
        assert_eq!(placement.size, (360, 100));
        assert_eq!(placement.hidden, None);
    }

    #[test]
    fn pannable_window_follows_viewport() {
        let snap = NormalSnapshot {
            mode: TilingMode::Cascade,
            box_geom: Rect { x: 0, y: 0, width: 640, height: 480 },
            min_size: (0, 0),
            virtual_pos: (100.0, 200.0),
            active_resize: None,
            is_cloud: false,
            saved_maximized_size: (0, 0),
            saved_maximized_virtual: (0.0, 0.0),
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, cloud_position_default: None, desktop_grid_scale: 100.0 };
        let mut c = ctx();
        c.pan_x = 50.0;
        c.pan_y = 100.0;
        c.zoom = 2.0;
        let placement = place_normal_window(&snap, &p, &c);
        assert_eq!(placement.pos, (100, 200));
        assert_eq!(placement.scale, 2.0);
        assert_eq!(placement.size, (640, 480));
        assert_eq!(placement.hidden, Some(false));

        // Pan far enough away and the window is culled.
        c.pan_x = 5000.0;
        let placement = place_normal_window(&snap, &p, &c);
        assert_eq!(placement.hidden, Some(true));

        // Interactive resize dimensions override stored geometry.
        let resizing = NormalSnapshot { active_resize: Some((800, 600)), ..snap };
        c.pan_x = 50.0;
        let placement = place_normal_window(&resizing, &p, &c);
        assert_eq!(placement.size, (800, 600));
    }

    #[test]
    fn opacity_policy() {
        assert_eq!(window_opacity(true, true, OVERLAY_UNFOCUSED_OPACITY), 1.0);
        assert_eq!(window_opacity(false, false, OVERLAY_UNFOCUSED_OPACITY), 1.0);
        assert_eq!(window_opacity(false, true, OVERLAY_UNFOCUSED_OPACITY), 0.85);
        assert_eq!(window_opacity(false, true, NORMAL_UNFOCUSED_OPACITY), 0.90);
    }

    #[test]
    fn side_stacks_center_vertically() {
        let items = vec![
            item("cce-status-a", StatusEdge::Left, 200),
            item("cce-status-b", StatusEdge::Left, 100),
        ];
        let p = layout_status_bars(&items, &params());
        // Total stack: 200 + 12 + 100 = 312, centered in 1080 → starts at 384.
        assert_eq!(p[0], Some(StatusBarPlacement { x: 0, y: 384, width: 30, height: 200 }));
        assert_eq!(p[1], Some(StatusBarPlacement { x: 0, y: 596, width: 30, height: 100 }));
    }
}
