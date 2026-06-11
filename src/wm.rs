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
use crate::types::{TilingMode, Window, WindowManager, NUM_TAGS, ModeRule};
use crate::wayland::AppState;
use wayland_client::QueueHandle;

/// Determine the tiling mode for a window based on mode_rules, tag_layouts,
/// and global_layout (in priority order).
///
/// Returns the resolved TilingMode, or None if the window's mode is locked
/// (i.e., the user manually set it and it should not be overridden).
pub fn get_mode_for_window(wm: &WindowManager, win: &Window) -> Option<TilingMode> {
    if win.app_id.as_deref() == Some("cce-status-interface") {
        return Some(TilingMode::Fullscreen);
    }
    if win.app_id.as_deref() == Some("cce-notification-daemon") || win.app_id.as_deref() == Some("clear-notification-daemon") {
        return Some(TilingMode::Popup);
    }

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
        let has_app_id = win.app_id.as_deref().map_or(false, |s| !s.is_empty());
        let match_app = rule.app_id_pattern == "*"
            || win
                .app_id
                .as_deref()
                .map_or(false, |aid| aid.contains(&rule.app_id_pattern))
            // Fallback: if the window has no app_id (None or empty), try
            // matching the app_id_pattern against the window title. This
            // handles apps that never set a Wayland app_id (e.g. clear-colors).
            || (!has_app_id && win.title.as_deref().map_or(false, |t| {
                let normalize = |s: &str| -> String {
                    s.to_lowercase().replace(|c: char| c == '-' || c == '_', " ")
                };
                normalize(t).contains(&normalize(&rule.app_id_pattern))
            }));
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

fn matches_mode_rule(mode_rules: &[ModeRule], win: &Window) -> bool {
    for rule in mode_rules {
        let has_app_id = win.app_id.as_deref().map_or(false, |s| !s.is_empty());
        let match_app = rule.app_id_pattern == "*"
            || win
                .app_id
                .as_deref()
                .map_or(false, |aid| aid.contains(&rule.app_id_pattern))
            || (!has_app_id && win.title.as_deref().map_or(false, |t| {
                let normalize = |s: &str| -> String {
                    s.to_lowercase().replace(|c: char| c == '-' || c == '_', " ")
                };
                normalize(t).contains(&normalize(&rule.app_id_pattern))
            }));
        let match_title = rule.title_pattern.as_deref() == Some("*")
            || rule.title_pattern.is_none()
            || win.title.as_deref().map_or(false, |t| {
                t.contains(rule.title_pattern.as_deref().unwrap_or(""))
            });

        if match_app && match_title {
            return true;
        }
    }
    false
}

fn get_circular_for_window(mode_rules: &[ModeRule], win: &Window) -> bool {
    for rule in mode_rules {
        let has_app_id = win.app_id.as_deref().map_or(false, |s| !s.is_empty());
        let match_app = rule.app_id_pattern == "*"
            || win
                .app_id
                .as_deref()
                .map_or(false, |aid| aid.contains(&rule.app_id_pattern))
            || (!has_app_id && win.title.as_deref().map_or(false, |t| {
                let normalize = |s: &str| -> String {
                    s.to_lowercase().replace(|c: char| c == '-' || c == '_', " ")
                };
                normalize(t).contains(&normalize(&rule.app_id_pattern))
            }));
        let match_title = rule.title_pattern.as_deref() == Some("*")
            || rule.title_pattern.is_none()
            || win.title.as_deref().map_or(false, |t| {
                t.contains(rule.title_pattern.as_deref().unwrap_or(""))
            });

        if match_app && match_title {
            return rule.circular;
        }
    }
    false
}

/// Assign tiling modes to all windows that aren't mode_locked.
/// Should be called during ManageStart before compute_tiling.
pub fn assign_window_modes(wm: &mut WindowManager) {
    // Enforce that cce-status-interface is assigned all tags so that it is always visible
    // and that blank steam_proton helper windows are always untagged so they remain hidden.
    for win in &mut wm.windows {
        win.circular = get_circular_for_window(&wm.mode_rules, win);

        if win.app_id.as_deref() == Some("cce-status-interface") {
            win.tags = u32::MAX;
        }

        let is_proton = win.app_id.as_deref() == Some("steam_proton");
        let is_blank = win.title.is_none() || win.title.as_deref().map_or(true, |t| t.is_empty());
        if is_proton && is_blank {
            if win.tags != 0 {
                eprintln!("[mode] enforcing tags=0 for blank steam_proton helper window id={}", win.id);
                win.tags = 0;
            }
        }
    }

    // Have newly spawned windows adopt the window mode of the focused window, if there is one.
    let focused_mode = wm.focused_window().map(|w| w.tiling_mode);
    if let Some(f_mode) = focused_mode {
        let mode_rules = &wm.mode_rules;
        for win in &mut wm.windows {
            if win.is_new
                && win.app_id.as_deref() != Some("cce-status-interface")
                && win.app_id.as_deref() != Some("cce-notification-daemon")
                && win.app_id.as_deref() != Some("clear-notification-daemon")
                && !win.has_parent
                && !matches_mode_rule(mode_rules, win)
            {
                // Determine what normal fallback mode would be
                let mut normal_fallback = wm.global_layout;
                for tag_bit in 0..crate::types::NUM_TAGS {
                    let tag_mask = 1u32 << tag_bit;
                    if (win.tags & tag_mask) != 0 && wm.has_tag_layout[tag_bit] {
                        normal_fallback = wm.tag_layouts[tag_bit];
                        break;
                    }
                }

                if f_mode != normal_fallback {
                    win.mode_locked = true;
                }
                win.tiling_mode = f_mode;
                eprintln!(
                    "[mode] new window {} (app_id={:?}) inherits focus mode {:?}",
                    win.id, win.app_id, f_mode
                );
            }
        }
    }

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
                let old_mode = win.tiling_mode;
                win.tiling_mode = mode;
                if (mode == TilingMode::Floating || mode == TilingMode::Popup)
                    && old_mode != TilingMode::Floating
                    && old_mode != TilingMode::Popup
                {
                    win.width = 0;
                    win.height = 0;
                }
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
    let (screen_w, screen_h, phys_w, phys_h, phys_x, phys_y) = get_screen_geometry(&state.wm);

    /*
    eprintln!(
        "[manage] windows={} outputs={} screen={}x{} (phys={}x{} at {},{})",
        state.wm.windows.len(),
        state.wm.outputs.len(),
        screen_w,
        screen_h,
        phys_w,
        phys_h,
        phys_x,
        phys_y
    );
    */

    // Ensure each window has a river_node_v1 proxy for positioning
    ensure_window_nodes(state, qhandle);

    // Compute tiling
    let tile_results = compute_tiling(&state.wm, screen_w, screen_h, phys_w, phys_h, phys_x, phys_y);

    // Apply: set_position + propose_dimensions + update internal state
    apply_tiling(state, &tile_results);
}

/// Set border colors on all visible windows.
/// This modifies rendering state and is called during RenderStart.
/// Border colors are applied with the next render_finish.
pub fn render_borders(state: &mut AppState) {
    set_borders(state);
}

/// Get screen geometry (usable_w, usable_h, phys_w, phys_h, phys_x, phys_y) from the first output, with fallbacks.
fn get_screen_geometry(wm: &WindowManager) -> (i32, i32, i32, i32, i32, i32) {
    let output = match wm.outputs.first() {
        Some(o) if !o.removed => o,
        _ => return (800, 600, 800, 600, 0, 0),
    };

    let scale = if wm.output_scale > 0.0 { wm.output_scale } else { 1.0 };

    let mut uw = output.usable_width;
    let mut uh = output.usable_height;

    let log_width = if output.width > 0 {
        (output.width as f64 / scale).round() as i32
    } else {
        0
    };
    let log_height = if output.height > 0 {
        (output.height as f64 / scale).round() as i32
    } else {
        0
    };

    // Fall back to logical output dimensions if usable area not yet set
    if uw <= 0 && log_width > 0 {
        uw = log_width;
    }
    if uh <= 0 && log_height > 0 {
        uh = log_height;
    }

    if uw <= 0 {
        uw = 800;
    }
    if uh <= 0 {
        uh = 600;
    }

    let pw = if log_width > 0 { log_width } else { uw };
    let ph = if log_height > 0 { log_height } else { uh };
    let px = output.x;
    let py = output.y;

    (uw, uh, pw, ph, px, py)
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
fn compute_tiling(
    wm: &WindowManager,
    screen_w: i32,
    screen_h: i32,
    phys_w: i32,
    phys_h: i32,
    phys_x: i32,
    phys_y: i32,
) -> Vec<TileResult> {
    let gap = wm.layout.gap;
    let gap_top = wm.layout.gap_top;
    let gap_left = wm.layout.gap_left;
    let gap_right = wm.layout.gap_right;
    let gap_bottom = wm.layout.gap_bottom;
    let cascade_offset = wm.layout.cascade_offset;
    let bar_height = wm.layout.bar_height;

    if wm.expose_active {
        let mut results = Vec::new();
        // Collect all active non-status-bar, non-popup windows on current tags
        let mut expose_windows: Vec<&crate::types::Window> = wm.windows.iter()
            .filter(|w| !w.closed && !w.minimized && (w.tags & wm.active_tags) != 0 && w.app_id.as_deref() != Some("cce-status-interface") && w.tiling_mode != TilingMode::Popup)
            .collect();

        // Sort expose_windows by current visual location (y first, then x)
        expose_windows.sort_by(|a, b| {
            if a.y != b.y {
                a.y.cmp(&b.y)
            } else {
                a.x.cmp(&b.x)
            }
        });

        let n_expose = expose_windows.len() as i32;
        if n_expose > 0 {
            let cols = (n_expose as f64).sqrt().ceil() as i32;
            let rows = (n_expose + cols - 1) / cols;
            let bw = wm.layout.grid_border_width;
            let dec_h = std::cmp::max(bw, 16);

            for (idx, win) in expose_windows.iter().enumerate() {
                let idx = idx as i32;
                let row = idx / cols;
                let col = idx % cols;

                let width = (screen_w - gap - gap - (cols - 1) * gap) / cols - 2 * bw;
                let height = (screen_h - bar_height - gap - gap - (rows - 1) * gap) / rows - (dec_h + bw);
                let width = if width < 1 { 1 } else { width };
                let height = if height < 1 { 1 } else { height };

                let x = gap + bw + col * (width + 2 * bw + gap);
                let y = bar_height + gap + dec_h + row * (height + (dec_h + bw) + gap);

                results.push(TileResult {
                    wid: win.id,
                    x,
                    y,
                    w: width,
                    h: height,
                });
            }
        }

        // Still layout cce-status-interface as fullscreen/bar if present
        if let Some(win) = wm.windows.iter().find(|w| !w.closed && w.app_id.as_deref() == Some("cce-status-interface")) {
            let (tx, ty, tw, th) = tiling::tile_fullscreen(
                phys_w,
                phys_h,
                gap_top,
                gap_left,
                gap_right,
                gap_bottom,
                0,
                bar_height,
            );
            results.push(TileResult {
                wid: win.id,
                x: tx + phys_x,
                y: ty + phys_y,
                w: tw,
                h: th,
            });
        }

        // Layout any active popup windows normally
        for win in &wm.windows {
            if !win.closed && (win.tags & wm.active_tags) != 0 && win.tiling_mode == TilingMode::Popup {
                let fw = if win.width > 0 {
                    win.width
                } else if win.hint_min_width > 32 {
                    win.hint_min_width
                } else {
                    360
                };
                let fh = if win.height > 0 {
                    win.height
                } else if win.hint_min_height > 32 {
                    win.hint_min_height
                } else {
                    100
                };
                let fx = screen_w - fw - gap_right;
                let fy = bar_height + gap_top;
                results.push(TileResult {
                    wid: win.id,
                    x: fx,
                    y: fy,
                    w: fw,
                    h: fh,
                });
            }
        }

        return results;
    }

    // Collect and sort grid windows by ID to ensure stable tiling layout positions
    let mut grid_windows: Vec<&crate::types::Window> = wm
        .windows
        .iter()
        .filter(|w| {
            (w.tags & wm.active_tags) != 0
                && !w.closed
                && !w.minimized
                && w.tiling_mode == TilingMode::Grid
        })
        .collect();
    grid_windows.sort_by_key(|w| w.id);
    let n_grid = grid_windows.len() as i32;

    // Collect and reverse cascade windows (focused window gets index 0, next 1, etc.)
    let mut cascade_windows: Vec<&crate::types::Window> = wm
        .windows
        .iter()
        .filter(|w| {
            (w.tags & wm.active_tags) != 0
                && !w.closed
                && !w.minimized
                && w.tiling_mode == TilingMode::Cascade
        })
        .collect();
    cascade_windows.reverse();
    let n_cascade = cascade_windows.len() as i32;

    // Check for fullscreen window — prefer the focused window so FocusNext
    // cycles visible windows when fullscreen is used as a layout mode.
    let fullscreen_id = wm
        .seats
        .iter()
        .find(|s| !s.removed)
        .and_then(|s| s.focused_window_id)
        .filter(|&fid| {
            wm.get_window(fid).map_or(false, |w| {
                (w.tags & wm.active_tags) != 0 && !w.closed && !w.minimized && w.tiling_mode == TilingMode::Fullscreen && w.app_id.as_deref() != Some("cce-status-interface")
            })
        })
        .or_else(|| {
            // Fallback: first fullscreen window if no focused window qualifies
            wm.windows.iter().find(|w| {
                (w.tags & wm.active_tags) != 0 && !w.closed && !w.minimized && w.tiling_mode == TilingMode::Fullscreen && w.app_id.as_deref() != Some("cce-status-interface")
            }).map(|w| w.id)
        });

    // Check if there is a visible SidePanel window on the active tag
    let side_panel_win = wm.windows.iter().find(|w| {
        (w.tags & wm.active_tags) != 0
            && !w.closed
            && !w.minimized
            && w.tiling_mode == TilingMode::SidePanel
    });

    let shift_x = if let Some(panel_win) = side_panel_win {
        if wm.layout.side_panel_behavior == "above" {
            0
        } else if panel_win.hint_min_width > 32 {
            std::cmp::max(wm.layout.side_panel_width, panel_win.hint_min_width)
        } else {
            wm.layout.side_panel_width
        }
    } else {
        0
    };

    // Compute tiling
    let mut results = Vec::new();
    let mut idx_floating = 0i32;

    for win in &wm.windows {
        if (win.tags & wm.active_tags) == 0 || win.closed || win.minimized {
            continue;
        }

        let wid = win.id;
        let mode = win.tiling_mode;

        let (x, y, w, h) = match mode {
            TilingMode::SidePanel => {
                let target_w = if win.hint_min_width > 32 {
                    std::cmp::max(wm.layout.side_panel_width, win.hint_min_width)
                } else {
                    wm.layout.side_panel_width
                };
                let bw = wm.layout.cascade_border_width;
                let dec_h = std::cmp::max(bw, 16);
                (phys_x + bw, bar_height + phys_y + dec_h, target_w - bw * 2, screen_h - bar_height - (dec_h + bw))
            }
            TilingMode::Fullscreen => {
                if fullscreen_id == Some(wid) || win.app_id.as_deref() == Some("cce-status-interface") {
                    let border_w = if win.app_id.as_deref() == Some("cce-status-interface") {
                        0
                    } else {
                        wm.layout.fullscreen_border_width
                    };
                    let (tx, ty, tw, th) = tiling::tile_fullscreen(
                        phys_w,
                        phys_h,
                        gap_top,
                        gap_left,
                        gap_right,
                        gap_bottom,
                        border_w,
                        bar_height,
                    );
                    (tx + phys_x, ty + phys_y, tw, th)
                } else {
                    continue;
                }
            }
            TilingMode::Cascade => {
                let idx = cascade_windows
                    .iter()
                    .position(|w| w.id == wid)
                    .unwrap_or(0) as i32;
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
                    idx,
                );
                (x, y, w, h)
            }
            TilingMode::Grid => {
                 let idx = grid_windows
                     .iter()
                     .position(|w| w.id == wid)
                     .unwrap_or(0) as i32;
                 let (x, y, w, h) = tiling::tile_grid(
                     screen_w,
                     screen_h,
                     wm.layout.grid_gap,
                     gap_top,
                     gap_left,
                     gap_right,
                     gap_bottom,
                     wm.layout.grid_border_width,
                     bar_height,
                     n_grid,
                     idx,
                 );
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
                } else if win.hint_min_width > 32 {
                    win.hint_min_width
                } else {
                    screen_w * 2 / 3
                };
                let fh = if win.height > 0 {
                    win.height
                } else if win.hint_min_height > 32 {
                    win.hint_min_height
                } else {
                    screen_h * 2 / 3
                };
                let fx = if win.x != 0 || win.y != 0 {
                    win.x
                } else {
                    gap_left + fbw + cascade_offset * idx_floating
                };
                let fy = if win.x != 0 || win.y != 0 {
                    win.y
                } else {
                    gap_left + fbw + bar_height + gap_top + cascade_offset * idx_floating
                };
                idx_floating += 1;
                (fx, fy, fw, fh)
            }
            TilingMode::Popup => {
                let fw = if win.width > 0 {
                    win.width
                } else if win.hint_min_width > 32 {
                    win.hint_min_width
                } else {
                    360
                };
                let fh = if win.height > 0 {
                    win.height
                } else if win.hint_min_height > 32 {
                    win.hint_min_height
                } else {
                    100
                };
                let fx = screen_w - fw - gap_right;
                let fy = bar_height + gap_top;
                (fx, fy, fw, fh)
            }
        };

        let final_x = if mode != TilingMode::SidePanel
            && mode != TilingMode::Fullscreen
            && mode != TilingMode::Popup
            && win.app_id.as_deref() != Some("cce-status-interface")
        {
            x + shift_x
        } else {
            x
        };
        results.push(TileResult { wid, x: final_x, y, w, h });
    }

    // Layout minimized windows as bubbles stacked at the right edge
    let minimized_windows: Vec<&crate::types::Window> = wm.windows.iter()
        .filter(|w| !w.closed && w.minimized && (w.tags & wm.active_tags) != 0 && w.app_id.as_deref() != Some("cce-status-interface"))
        .collect();

    let bubble_width = 160;
    let border_w = wm.layout.grid_border_width;

    for (_idx, win) in minimized_windows.iter().enumerate() {
        let wid = win.id;
        let x = screen_w - wm.layout.gap_right - bubble_width - border_w;
        let y = screen_h - 2;
        let w = if win.width > 0 { win.width } else { bubble_width };
        let h = if win.height > 0 { win.height } else { 100 };
        results.push(TileResult {
            wid,
            x,
            y,
            w,
            h,
        });
    }

    results
}

/// Apply computed tiling results: set position and propose dimensions.
fn apply_tiling(state: &mut AppState, results: &[TileResult]) {
    let mut any_animating = false;

    let focused_id = state.wm.seats.iter()
        .find(|s| !s.removed)
        .and_then(|s| s.focused_window_id);

    let transition_duration = state.wm.layout.transition_duration;
    let easing = if transition_duration <= 16 {
        1.0
    } else {
        1.0 - 0.01f64.powf(16.0 / transition_duration as f64)
    };

    let expose_active = state.wm.expose_active;
    let (screen_w, screen_h, _, _, _, _) = get_screen_geometry(&state.wm);
    let fbw = state.wm.layout.floating_border_width;
    let gap_left = state.wm.layout.gap_left;
    let gap_top = state.wm.layout.gap_top;
    let bar_height = state.wm.layout.bar_height;

    let side_panel_win = state.wm.windows.iter().find(|w| {
        (w.tags & state.wm.active_tags) != 0
            && !w.closed
            && !w.minimized
            && w.tiling_mode == TilingMode::SidePanel
    });
    let is_side_panel_present = side_panel_win.is_some();

    for tr in results {
        let mut final_x = tr.x;
        let mut final_y = tr.y;
        let mut final_w = tr.w;
        let mut final_h = tr.h;

        if let Some(win) = state.wm.get_window_mut(tr.wid) {
            let is_cascade = win.tiling_mode == TilingMode::Cascade;
            let is_exposed = expose_active && win.tiling_mode != TilingMode::Popup && win.app_id.as_deref() != Some("cce-status-interface");
            let was_animating = win.anim_x.is_some() || win.anim_y.is_some() || win.anim_w.is_some() || win.anim_h.is_some() || win.anim_opacity.is_some();
            let should_animate = (is_cascade || is_exposed || is_side_panel_present || win.tiling_mode == TilingMode::SidePanel || was_animating) && !win.minimized;

            if should_animate {
                let curr_x = win.anim_x.unwrap_or(win.x as f64);
                let curr_y = win.anim_y.unwrap_or(win.y as f64);
                let curr_w = win.anim_w.unwrap_or(win.width as f64);
                let curr_h = win.anim_h.unwrap_or(win.height as f64);
                let curr_opacity = win.anim_opacity.unwrap_or(1.0);

                let target_opacity = if is_exposed {
                    if Some(win.id) == focused_id { 1.0 } else { 0.75 }
                } else if is_cascade {
                    if Some(win.id) == focused_id { 1.0 } else { 0.75 }
                } else {
                    1.0
                };

                if curr_w == 0.0 || win.is_new {
                    // New window: snap instantly
                    win.anim_x = Some(tr.x as f64);
                    win.anim_y = Some(tr.y as f64);
                    win.anim_w = Some(tr.w as f64);
                    win.anim_h = Some(tr.h as f64);
                    win.anim_opacity = Some(target_opacity);
                } else {
                    let target_x = tr.x as f64;
                    let target_y = tr.y as f64;
                    let target_w = tr.w as f64;
                    let target_h = tr.h as f64;

                    let dx = target_x - curr_x;
                    let dy = target_y - curr_y;
                    let dw = target_w - curr_w;
                    let dh = target_h - curr_h;
                    let d_opacity = target_opacity - curr_opacity;

                    if dx.abs() > 0.5 || dy.abs() > 0.5 || dw.abs() > 0.5 || dh.abs() > 0.5 || d_opacity.abs() > 0.01 {
                        let next_x = curr_x + dx * easing;
                        let next_y = curr_y + dy * easing;
                        let next_w = curr_w + dw * easing;
                        let next_h = curr_h + dh * easing;
                        let next_opacity = curr_opacity + d_opacity * easing;

                        win.anim_x = Some(next_x);
                        win.anim_y = Some(next_y);
                        win.anim_w = Some(next_w);
                        win.anim_h = Some(next_h);
                        win.anim_opacity = Some(next_opacity);

                        final_x = next_x.round() as i32;
                        final_y = next_y.round() as i32;
                        // Propose the target size instantly during transitions to avoid configure storms and flickering
                        final_w = tr.w;
                        final_h = tr.h;

                        any_animating = true;
                    } else {
                        if is_cascade || is_exposed {
                            win.anim_x = Some(target_x);
                            win.anim_y = Some(target_y);
                            win.anim_w = Some(target_w);
                            win.anim_h = Some(target_h);
                            win.anim_opacity = Some(target_opacity);
                        } else {
                            win.anim_x = None;
                            win.anim_y = None;
                            win.anim_w = None;
                            win.anim_h = None;
                            win.anim_opacity = None;
                        }
                    }
                }
            } else {
                win.anim_x = None;
                win.anim_y = None;
                win.anim_w = None;
                win.anim_h = None;
                win.anim_opacity = None;
            }

            // Update internal state
            let is_floating_and_expose = expose_active && win.tiling_mode == TilingMode::Floating;
            if !is_floating_and_expose {
                win.x = final_x;
                win.y = final_y;
                let is_floating_or_popup = win.tiling_mode == TilingMode::Floating || win.tiling_mode == TilingMode::Popup;
                if !(is_floating_or_popup && win.width == 0) {
                    win.width = final_w;
                    win.height = final_h;
                }
            } else {
                // If it's a new floating window created during expose mode,
                // initialize its position to the default floating position if unset.
                if win.x == 0 && win.y == 0 {
                    let fw = if win.hint_min_width > 32 { win.hint_min_width } else { screen_w * 2 / 3 };
                    let fh = if win.hint_min_height > 32 { win.hint_min_height } else { screen_h * 2 / 3 };
                    let fx = gap_left + fbw;
                    let fy = gap_left + fbw + bar_height + gap_top;
                    win.x = fx;
                    win.y = fy;
                    win.width = fw;
                    win.height = fh;
                }
            }
        }

        // Set position via river_node_v1
        if let Some(node) = state
            .window_nodes
            .iter()
            .find(|(id, _)| *id == tr.wid)
            .map(|(_, n)| n)
        {
            node.set_position(final_x, final_y);
        }

        // Propose dimensions via river_window_v1
        if let Some(wp) = state.get_window_proxy(tr.wid) {
            let is_floating_or_popup = if let Some(win) = state.wm.get_window(tr.wid) {
                (win.tiling_mode == TilingMode::Floating || win.tiling_mode == TilingMode::Popup) && win.width == 0
            } else {
                false
            };

            if is_floating_or_popup {
                wp.river_window.propose_dimensions(0, 0);
            } else {
                wp.river_window.propose_dimensions(final_w, final_h);
            }
            // Tell the client to use server-side decoration.
            // Per the River protocol, use_csd is the default when neither
            // use_csd nor use_ssd is called. Calling use_ssd here ensures
            // windows don't draw their own CSD titlebars/borders.
            // use_ssd has no effect if the client only supports CSD
            // (decoration_hint == only_supports_csd).
            wp.river_window.use_ssd();
        }
    }

    state.wm.animating = any_animating;
    if any_animating {
        state.wm.needs_render = true;
    }
    state.wm.expose_visual_active = state.wm.expose_active;
}

/// Set the expose_active state and initialize transition animation states to prevent flickering and enable smooth animations.
pub fn set_expose_active(wm: &mut WindowManager, active: bool) {
    if wm.expose_active == active {
        return;
    }
    let prior_expose_visual_active = wm.expose_visual_active;

    let focused_id = wm.seats.iter()
        .find(|s| !s.removed)
        .and_then(|s| s.focused_window_id);

    for win in &mut wm.windows {
        if win.closed || win.app_id.as_deref() == Some("cce-status-interface") || win.tiling_mode == TilingMode::Popup {
            continue;
        }
        let is_focused = Some(win.id) == focused_id;

        // Ensure current geometry animation states are initialized
        if win.anim_x.is_none() { win.anim_x = Some(win.x as f64); }
        if win.anim_y.is_none() { win.anim_y = Some(win.y as f64); }
        if win.anim_w.is_none() { win.anim_w = Some(win.width as f64); }
        if win.anim_h.is_none() { win.anim_h = Some(win.height as f64); }

        // Ensure opacity animation state is initialized to its current visual value
        if win.anim_opacity.is_none() {
            let current_opacity = if win.tiling_mode == TilingMode::Cascade && !is_focused {
                0.75
            } else if prior_expose_visual_active && !is_focused {
                0.75
            } else {
                1.0
            };
            win.anim_opacity = Some(current_opacity);
        }
    }

    wm.expose_active = active;
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

/// Set opacity on all visible windows.
/// This modifies rendering state and is called during RenderStart.
/// Opacity is applied with the next render_finish.
pub fn render_opacity(state: &mut AppState) {
    let focused_id = state.wm.seats.iter()
        .find(|s| !s.removed)
        .and_then(|s| s.focused_window_id);

    for win in &state.wm.windows {
        if win.closed {
            continue;
        }
        if let Some(wp) = state.get_window_proxy(win.id) {
            let is_cascade = win.tiling_mode == TilingMode::Cascade;
            let is_exposed = state.wm.expose_visual_active && win.tiling_mode != TilingMode::Popup && win.app_id.as_deref() != Some("cce-status-interface");
            let was_animating = win.anim_opacity.is_some();
            let should_fade = is_cascade || is_exposed || was_animating;

            let opacity = if should_fade {
                win.anim_opacity.unwrap_or_else(|| {
                    if is_exposed || is_cascade {
                        if Some(win.id) == focused_id {
                            1.0
                        } else {
                            0.75
                        }
                    } else {
                        1.0
                    }
                })
            } else {
                1.0
            };
            let op_u32 = (opacity * u32::MAX as f64).round() as u32;
            wp.river_window.set_opacity(op_u32);
        }
    }
}

/// Apply whether windows are circular.
/// This modifies rendering state and is called during RenderStart.
pub fn render_circular(state: &mut AppState) {
    for win in &state.wm.windows {
        if win.closed {
            continue;
        }
        if let Some(wp) = state.get_window_proxy(win.id) {
            let val = if win.circular { 1 } else { 0 };
            wp.river_window.set_circular(val);
        }
    }
}

/// Apply window backdrop blur.
/// This modifies rendering state and is called during RenderStart.
pub fn render_blur(state: &mut AppState) {
    for win in &state.wm.windows {
        if win.closed {
            continue;
        }
        if let Some(wp) = state.get_window_proxy(win.id) {
            let val = if state.wm.layout.window_blur { 1 } else { 0 };
            wp.river_window.set_blur(val);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Window, Seat, WindowManager, TilingMode, ModeRule};

    #[test]
    fn test_assign_window_modes_inherit_focus_mode_locked() {
        let mut wm = WindowManager::default();
        wm.global_layout = TilingMode::Cascade;
        wm.tag_layouts[0] = TilingMode::Cascade;

        // Spawn a focused window that is in Grid mode
        wm.windows.push(Window {
            id: 1,
            tiling_mode: TilingMode::Grid,
            is_new: false,
            ..Default::default()
        });
        wm.seats.push(Seat {
            id: 1,
            focused_window_id: Some(1),
            ..Default::default()
        });

        // Spawn a new window
        wm.windows.push(Window {
            id: 2,
            is_new: true,
            app_id: Some("kitty".to_string()),
            ..Default::default()
        });

        assign_window_modes(&mut wm);

        // Window 2 should inherit Grid mode and be locked because Grid != Cascade
        let win2 = wm.get_window(2).unwrap();
        assert_eq!(win2.tiling_mode, TilingMode::Grid);
        assert!(win2.mode_locked);
    }

    #[test]
    fn test_assign_window_modes_inherit_focus_mode_not_locked() {
        let mut wm = WindowManager::default();
        wm.global_layout = TilingMode::Grid;
        wm.tag_layouts[0] = TilingMode::Grid;

        // Spawn a focused window that is in Grid mode
        wm.windows.push(Window {
            id: 1,
            tiling_mode: TilingMode::Grid,
            is_new: false,
            ..Default::default()
        });
        wm.seats.push(Seat {
            id: 1,
            focused_window_id: Some(1),
            ..Default::default()
        });

        // Spawn a new window
        wm.windows.push(Window {
            id: 2,
            is_new: true,
            app_id: Some("kitty".to_string()),
            ..Default::default()
        });

        assign_window_modes(&mut wm);

        // Window 2 should inherit Grid mode but NOT be locked because Grid == Grid (normal fallback)
        let win2 = wm.get_window(2).unwrap();
        assert_eq!(win2.tiling_mode, TilingMode::Grid);
        assert!(!win2.mode_locked);
    }

    #[test]
    fn test_assign_window_modes_inherit_focus_has_parent() {
        let mut wm = WindowManager::default();
        wm.global_layout = TilingMode::Grid;
        wm.tag_layouts[0] = TilingMode::Grid;

        // Spawn a focused window that is in Grid mode
        wm.windows.push(Window {
            id: 1,
            tiling_mode: TilingMode::Grid,
            is_new: false,
            ..Default::default()
        });
        wm.seats.push(Seat {
            id: 1,
            focused_window_id: Some(1),
            ..Default::default()
        });

        // Spawn a new parented window
        wm.windows.push(Window {
            id: 2,
            is_new: true,
            app_id: Some("kitty".to_string()),
            has_parent: true,
            ..Default::default()
        });

        assign_window_modes(&mut wm);

        // Window 2 should get Floating mode (parent fallback) and not inherit Grid
        let win2 = wm.get_window(2).unwrap();
        assert_eq!(win2.tiling_mode, TilingMode::Floating);
    }

    #[test]
    fn test_assign_window_modes_inherit_focus_matches_rule() {
        let mut wm = WindowManager::default();
        wm.global_layout = TilingMode::Cascade;
        wm.tag_layouts[0] = TilingMode::Cascade;
        wm.mode_rules.push(ModeRule {
            mode: TilingMode::Fullscreen,
            app_id_pattern: "firefox".to_string(),
            title_pattern: None,
            single_instance: false,
            tag: 0,
            circular: false,
        });

        // Spawn a focused window that is in Grid mode
        wm.windows.push(Window {
            id: 1,
            tiling_mode: TilingMode::Grid,
            is_new: false,
            ..Default::default()
        });
        wm.seats.push(Seat {
            id: 1,
            focused_window_id: Some(1),
            ..Default::default()
        });

        // Spawn a new firefox window matching the mode rule
        wm.windows.push(Window {
            id: 2,
            is_new: true,
            app_id: Some("firefox".to_string()),
            ..Default::default()
        });

        assign_window_modes(&mut wm);

        // Window 2 should get Fullscreen mode from rule and not inherit Grid
        let win2 = wm.get_window(2).unwrap();
        assert_eq!(win2.tiling_mode, TilingMode::Fullscreen);
    }

    #[test]
    fn test_assign_window_modes_notification_daemon_no_inherit() {
        let mut wm = WindowManager::default();
        wm.global_layout = TilingMode::Grid;
        wm.tag_layouts[0] = TilingMode::Grid;

        // Spawn a focused window that is in Grid mode
        wm.windows.push(Window {
            id: 1,
            tiling_mode: TilingMode::Grid,
            is_new: false,
            ..Default::default()
        });
        wm.seats.push(Seat {
            id: 1,
            focused_window_id: Some(1),
            ..Default::default()
        });

        // Spawn a new notification daemon window
        wm.windows.push(Window {
            id: 2,
            is_new: true,
            app_id: Some("cce-notification-daemon".to_string()),
            ..Default::default()
        });

        assign_window_modes(&mut wm);

        // Window 2 should get Popup mode (hardcoded default for notification daemon) and not inherit Grid
        let win2 = wm.get_window(2).unwrap();
        assert_eq!(win2.tiling_mode, TilingMode::Popup);
        assert!(!win2.mode_locked);
    }

    #[test]
    fn test_expose_mode_sorting() {
        let mut wm = WindowManager::default();
        wm.expose_active = true;
        wm.active_tags = 1;

        // Push windows in unordered spatial positions, representing focus ordering
        wm.windows.push(Window {
            id: 10,
            x: 1000,
            y: 500,
            tags: 1,
            ..Default::default()
        });
        wm.windows.push(Window {
            id: 20,
            x: 0,
            y: 500,
            tags: 1,
            ..Default::default()
        });
        wm.windows.push(Window {
            id: 30,
            x: 1000,
            y: 0,
            tags: 1,
            ..Default::default()
        });
        wm.windows.push(Window {
            id: 40,
            x: 0,
            y: 0,
            tags: 1,
            ..Default::default()
        });

        // Compute tiling in expose mode
        let results = compute_tiling(&wm, 1920, 1080, 1920, 1080, 0, 0);

        // Expected sorted order:
        // 1. (0, 0) -> id 40
        // 2. (1000, 0) -> id 30
        // 3. (0, 500) -> id 20
        // 4. (1000, 500) -> id 10
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].wid, 40);
        assert_eq!(results[1].wid, 30);
        assert_eq!(results[2].wid, 20);
        assert_eq!(results[3].wid, 10);
    }
}

