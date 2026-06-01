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
    if win.app_id.as_deref() == Some("clear-status-interface") {
        return Some(TilingMode::Fullscreen);
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

/// Assign tiling modes to all windows that aren't mode_locked.
/// Should be called during ManageStart before compute_tiling.
pub fn assign_window_modes(wm: &mut WindowManager) {
    // Enforce that clear-status-interface is assigned all tags so that it is always visible
    // and that blank steam_proton helper windows are always untagged so they remain hidden.
    for win in &mut wm.windows {
        if win.app_id.as_deref() == Some("clear-status-interface") {
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
    let (screen_w, screen_h, phys_w, phys_h, phys_x, phys_y) = get_screen_geometry(&state.wm);

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
        let expose_windows: Vec<&crate::types::Window> = wm.windows.iter()
            .filter(|w| !w.closed && (w.tags & wm.active_tags) != 0 && w.app_id.as_deref() != Some("clear-status-interface") && w.tiling_mode != TilingMode::Popup)
            .collect();

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

        // Still layout clear-status-interface as fullscreen/bar if present
        if let Some(win) = wm.windows.iter().find(|w| !w.closed && w.app_id.as_deref() == Some("clear-status-interface")) {
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

    // Count windows per tiling mode
    let mut n_cascade = 0i32;
    let mut n_grid = 0i32;
    for win in &wm.windows {
        if (win.tags & wm.active_tags) == 0 || win.closed {
            continue;
        }
        match win.tiling_mode {
            TilingMode::Cascade => n_cascade += 1,
            TilingMode::Grid => n_grid += 1,
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
                (w.tags & wm.active_tags) != 0 && !w.closed && w.tiling_mode == TilingMode::Fullscreen && w.app_id.as_deref() != Some("clear-status-interface")
            })
        })
        .or_else(|| {
            // Fallback: first fullscreen window if no focused window qualifies
            wm.windows.iter().find(|w| {
                (w.tags & wm.active_tags) != 0 && !w.closed && w.tiling_mode == TilingMode::Fullscreen && w.app_id.as_deref() != Some("clear-status-interface")
            }).map(|w| w.id)
        });

    // Compute tiling
    let mut results = Vec::new();
    let mut idx_cascade = 0i32;
    let mut idx_grid = 0i32;
    let mut idx_floating = 0i32;

    for win in &wm.windows {
        if (win.tags & wm.active_tags) == 0 || win.closed {
            continue;
        }

        let wid = win.id;
        let mode = win.tiling_mode;

        let (x, y, w, h) = match mode {
            TilingMode::Fullscreen => {
                if fullscreen_id == Some(wid) || win.app_id.as_deref() == Some("clear-status-interface") {
                    let border_w = if win.app_id.as_deref() == Some("clear-status-interface") {
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

        results.push(TileResult { wid, x, y, w, h });
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

    for tr in results {
        let mut final_x = tr.x;
        let mut final_y = tr.y;
        let mut final_w = tr.w;
        let mut final_h = tr.h;

        if let Some(win) = state.wm.get_window_mut(tr.wid) {
            let is_cascade = win.tiling_mode == TilingMode::Cascade;
            let is_exposed = expose_active && win.tiling_mode != TilingMode::Popup && win.app_id.as_deref() != Some("clear-status-interface");
            let was_animating = win.anim_x.is_some() || win.anim_y.is_some() || win.anim_w.is_some() || win.anim_h.is_some() || win.anim_opacity.is_some();
            let should_animate = is_cascade || is_exposed || was_animating;

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
                win.width = final_w;
                win.height = final_h;
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
            wp.river_window.propose_dimensions(final_w, final_h);
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
        if win.closed || win.app_id.as_deref() == Some("clear-status-interface") || win.tiling_mode == TilingMode::Popup {
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
            let is_exposed = state.wm.expose_visual_active && win.tiling_mode != TilingMode::Popup && win.app_id.as_deref() != Some("clear-status-interface");
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
