// WM rendering logic — tiling, borders, and positioning
//
// Called from wayland.rs render_start handler. This module performs
// the same work as the C version's wm_handle_render_start():
//   1. Tile visible windows (propose dimensions + set position)
//   2. Set border colors (cascade depth gradient or normal gray)

use crate::borders::compute_border_colors;
use crate::protocol::river_window_management::client::river_node_v1::RiverNodeV1;
use crate::protocol::river_window_management::client::river_window_v1::Edges;
use crate::tiling;
use crate::types::{TilingMode, Window, WindowManager, NUM_TAGS};
use crate::wayland::AppState;
use wayland_client::QueueHandle;

/// Determine the tiling mode for a window based on mode_rules, tag_layouts,
/// and global_layout (in priority order).
///
/// Returns the resolved TilingMode, or None if the window's mode is locked
/// (i.e., the user manually set it and it should not be overridden).
pub fn get_mode_for_window(wm: &WindowManager, win: &Window) -> Option<TilingMode> {
    // If the user manually locked the mode (via set-mode, fullscreen toggle, etc.),
    // don't override it.
    if win.mode_locked {
        return None;
    }

    // 0. Windows with a parent (dialogs, file pickers, etc.) always float.
    if win.has_parent {
        return Some(TilingMode::Floating);
    }

    // 1. Check mode_rules for a match on app_id/title
    for rule in &wm.mode_rules {
        let match_app = rule.app_id_pattern == "*"
            || win
                .app_id
                .as_deref()
                .map_or(false, |aid| aid.contains(&rule.app_id_pattern));
        let match_title = rule.title_pattern.as_deref() == Some("*")
            || rule.title_pattern.is_none()
            || win.title.as_deref().map_or(false, |t| {
                t.contains(rule.title_pattern.as_deref().unwrap_or(""))
            });

        if match_app && match_title {
            return Some(rule.mode);
        }
    }

    // 2. Check tag_layouts for the window's active tags
    for tag_bit in 0..NUM_TAGS {
        let tag_mask = 1u32 << tag_bit;
        if (win.tags & tag_mask) != 0 && wm.has_tag_layout[tag_bit] {
            return Some(wm.tag_layouts[tag_bit]);
        }
    }

    // 3. Fall back to global_layout
    Some(wm.global_layout)
}

/// Assign tiling modes to all windows that aren't mode_locked.
/// Should be called during ManageStart before compute_tiling.
pub fn assign_window_modes(wm: &mut WindowManager) {
    // Collect assignments first (borrow checker: can't borrow wm mutably while iterating mode_rules)
    let assignments: Vec<(u64, TilingMode)> = wm
        .windows
        .iter()
        .filter(|w| !w.closed && !w.mode_locked)
        .filter_map(|win| get_mode_for_window(wm, win).map(|mode| (win.id, mode)))
        .collect();

    for (wid, mode) in assignments {
        if let Some(win) = wm.get_window_mut(wid) {
            if win.tiling_mode != mode {
                eprintln!(
                    "[mode] window {} (app_id={:?}): {:?} -> {:?}",
                    wid, win.app_id, win.tiling_mode, mode
                );
                win.tiling_mode = mode;
            }
        }
    }
}

/// A tiling result for a single window
struct TileResult {
    wid: u64,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// Perform window management during a manage sequence.
/// This calls set_position and propose_dimensions on visible windows.
/// These modify window management state and can ONLY be called during
/// a manage sequence (between ManageStart and ManageFinish).
pub fn manage_windows(state: &mut AppState, qhandle: &QueueHandle<AppState>) {
    let screen_dims = get_screen_dimensions(&state.wm);
    let (screen_w, screen_h) = screen_dims;

    eprintln!(
        "[manage] windows={} outputs={} screen={}x{}",
        state.wm.windows.len(),
        state.wm.outputs.len(),
        screen_w,
        screen_h
    );

    // Ensure each window has a river_node_v1 proxy for positioning
    ensure_window_nodes(state, qhandle);

    // Compute tiling
    let tile_results = compute_tiling(&state.wm, screen_w, screen_h);

    // Apply: set_position + propose_dimensions + update internal state
    apply_tiling(state, &tile_results);
}

/// Set border colors on all visible windows.
/// This modifies rendering state and is called during RenderStart.
/// Border colors are applied with the next render_finish.
pub fn render_borders(state: &mut AppState) {
    set_borders(state);
}

/// Get screen dimensions from the first output, with fallbacks.
fn get_screen_dimensions(wm: &WindowManager) -> (i32, i32) {
    let output = match wm.outputs.first() {
        Some(o) if !o.removed => o,
        _ => return (800, 600),
    };

    let mut w = output.usable_width;
    let mut h = output.usable_height;

    // Fall back to raw output dimensions if usable area not yet set
    if w <= 0 && output.width > 0 {
        w = output.width;
    }
    if h <= 0 && output.height > 0 {
        h = output.height;
    }

    if w <= 0 {
        w = 800;
    }
    if h <= 0 {
        h = 600;
    }

    (w, h)
}

/// Ensure each window has a river_node_v1 proxy for positioning.
/// The node is created via river_window_v1.get_node() — can only be called once.
fn ensure_window_nodes(state: &mut AppState, qhandle: &QueueHandle<AppState>) {
    let windows_needing_nodes: Vec<u64> = state
        .wm
        .windows
        .iter()
        .filter(|w| !w.closed)
        .filter(|w| !state.window_nodes.iter().any(|(id, _)| *id == w.id))
        .map(|w| w.id)
        .collect();

    if windows_needing_nodes.is_empty() {
        return;
    }

    for wid in windows_needing_nodes {
        if let Some(wp) = state.get_window_proxy(wid) {
            let node: RiverNodeV1 = wp.river_window.get_node(qhandle, ());
            state.window_nodes.push((wid, node));
        }
    }
}

/// Compute tiling for all visible windows (read-only, returns results).
fn compute_tiling(wm: &WindowManager, screen_w: i32, screen_h: i32) -> Vec<TileResult> {
    let gap = wm.layout.gap;
    let gap_top = wm.layout.gap_top;
    let gap_left = wm.layout.gap_left;
    let gap_right = wm.layout.gap_right;
    let gap_bottom = wm.layout.gap_bottom;
    let cascade_offset = wm.layout.cascade_offset;
    let bar_height = wm.layout.bar_height;

    // Count windows per tiling mode
    let mut n_cascade = 0i32;
    let mut n_grid = 0i32;
    let mut n_vsplit = 0i32;
    let mut n_hsplit = 0i32;
    for win in &wm.windows {
        if (win.tags & wm.active_tags) == 0 || win.closed {
            continue;
        }
        match win.tiling_mode {
            TilingMode::Cascade => n_cascade += 1,
            TilingMode::Grid => n_grid += 1,
            TilingMode::Vsplit => n_vsplit += 1,
            TilingMode::Hsplit => n_hsplit += 1,
            _ => {}
        }
    }

    // Check for fullscreen window — prefer the focused window so FocusNext
    // cycles visible windows when fullscreen is used as a layout mode.
    let fullscreen_id = wm
        .seats
        .iter()
        .find(|s| !s.removed)
        .and_then(|s| s.focused_window_id)
        .filter(|&fid| {
            wm.get_window(fid).map_or(false, |w| {
                (w.tags & wm.active_tags) != 0 && !w.closed && w.tiling_mode == TilingMode::Fullscreen
            })
        })
        .or_else(|| {
            // Fallback: first fullscreen window if no focused window qualifies
            wm.windows.iter().find(|w| {
                (w.tags & wm.active_tags) != 0 && !w.closed && w.tiling_mode == TilingMode::Fullscreen
            }).map(|w| w.id)
        });

    // Compute tiling
    let mut results = Vec::new();
    let mut idx_cascade = 0i32;
    let mut idx_grid = 0i32;
    let mut idx_vsplit = 0i32;
    let mut idx_hsplit = 0i32;

    for win in &wm.windows {
        if (win.tags & wm.active_tags) == 0 || win.closed {
            continue;
        }

        let wid = win.id;
        let mode = win.tiling_mode;

        let (x, y, w, h) = match mode {
            TilingMode::Fullscreen => {
                if fullscreen_id == Some(wid) {
                    tiling::tile_fullscreen(
                        screen_w,
                        screen_h,
                        gap_top,
                        gap_left,
                        gap_right,
                        gap_bottom,
                        wm.layout.fullscreen_border_width,
                        bar_height,
                    )
                } else {
                    continue;
                }
            }
            TilingMode::Cascade => {
                let (x, y, w, h) = tiling::tile_cascade(
                    screen_w,
                    screen_h,
                    gap,
                    gap_top,
                    gap_left,
                    gap_right,
                    gap_bottom,
                    wm.layout.cascade_border_width,
                    cascade_offset,
                    bar_height,
                    n_cascade,
                    idx_cascade,
                );
                idx_cascade += 1;
                (x, y, w, h)
            }
            TilingMode::Grid => {
                let (x, y, w, h) =
                    tiling::tile_grid(screen_w, screen_h, gap, gap_top, gap_left, gap_right, gap_bottom, wm.layout.grid_border_width, bar_height, n_grid, idx_grid);
                idx_grid += 1;
                (x, y, w, h)
            }
            TilingMode::Vsplit => {
                let (x, y, w, h) = tiling::tile_vsplit(
                    screen_w, screen_h, gap, gap_top, gap_left, gap_right, gap_bottom, wm.layout.vsplit_border_width, bar_height, n_vsplit, idx_vsplit,
                );
                idx_vsplit += 1;
                (x, y, w, h)
            }
            TilingMode::Hsplit => {
                let (x, y, w, h) = tiling::tile_hsplit(
                    screen_w, screen_h, gap, gap_top, gap_left, gap_right, gap_bottom, wm.layout.hsplit_border_width, bar_height, n_hsplit, idx_hsplit,
                );
                idx_hsplit += 1;
                (x, y, w, h)
            }
            TilingMode::Floating => {
                // Floating windows: don't tile them, but still propose
                // dimensions so they don't end up at w=0 h=0 (which
                // triggers River's unresponsive-client detection).
                // Use the window's existing dimensions, or a reasonable
                // default if unset.
                let fbw = wm.layout.floating_border_width;
                let fw = if win.width > 0 {
                    win.width
                } else {
                    screen_w * 2 / 3
                };
                let fh = if win.height > 0 {
                    win.height
                } else {
                    screen_h * 2 / 3
                };
                let fx = if win.x != 0 || win.y != 0 {
                    win.x
                } else {
                    gap_left + fbw + cascade_offset * idx_cascade
                };
                let fy = if win.x != 0 || win.y != 0 {
                    win.y
                } else {
                    gap_left + fbw + bar_height + gap_top + cascade_offset * idx_cascade
                };
                idx_cascade += 1;
                (fx, fy, fw, fh)
            }
        };

        results.push(TileResult { wid, x, y, w, h });
    }

    results
}

/// Apply computed tiling results: set position and propose dimensions.
fn apply_tiling(state: &mut AppState, results: &[TileResult]) {
    for tr in results {
        // Set position via river_node_v1
        if let Some(node) = state
            .window_nodes
            .iter()
            .find(|(id, _)| *id == tr.wid)
            .map(|(_, n)| n)
        {
            node.set_position(tr.x, tr.y);
        }

        // Propose dimensions via river_window_v1
        if let Some(wp) = state.get_window_proxy(tr.wid) {
            wp.river_window.propose_dimensions(tr.w, tr.h);
            // Tell the client to use server-side decoration.
            // Per the River protocol, use_csd is the default when neither
            // use_csd nor use_ssd is called. Calling use_ssd here ensures
            // windows don't draw their own CSD titlebars/borders.
            // use_ssd has no effect if the client only supports CSD
            // (decoration_hint == only_supports_csd).
            wp.river_window.use_ssd();
        }

        // Update internal state
        if let Some(win) = state.wm.get_window_mut(tr.wid) {
            win.x = tr.x;
            win.y = tr.y;
            win.width = tr.w;
            win.height = tr.h;
        }
    }
}

/// Set border colors on all visible windows.
fn set_borders(state: &mut AppState) {
    let border_colors = compute_border_colors(&state.wm);

    for bc in &border_colors {
        let wid = match state.wm.windows.get(bc.window_idx) {
            Some(w) => w.id,
            None => continue,
        };

        if let Some(wp) = state.get_window_proxy(wid) {
            let edges = Edges::from_bits_truncate(bc.edges);
            wp.river_window
                .set_borders(edges, bc.width, bc.r, bc.g, bc.b, bc.a);
        }
    }
}
