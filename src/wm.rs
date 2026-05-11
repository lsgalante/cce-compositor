// WM rendering logic — tiling, borders, and positioning
//
// Called from wayland.rs render_start handler. This module performs
// the same work as the C version's wm_handle_render_start():
//   1. Tile visible windows (propose dimensions + set position)
//   2. Set border colors (cascade depth gradient or normal gray)
//   3. Update desktop background via swaybg

use crate::borders::compute_border_colors;
use crate::protocol::river_window_management::client::river_node_v1::RiverNodeV1;
use crate::protocol::river_window_management::client::river_window_v1::Edges;
use crate::tiling;
use crate::types::{TilingMode, WindowManager};
use crate::wayland::AppState;
use wayland_client::QueueHandle;

/// A tiling result for a single window
struct TileResult {
    wid: u64,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// Perform a full render cycle: tile windows and set borders.
///
/// This is called from the `render_start` handler only when `needs_render` is true.
/// After this, the caller always calls `render_finish()`.
pub fn render_windows(state: &mut AppState, qhandle: &QueueHandle<AppState>) {
    let screen_dims = get_screen_dimensions(&state.wm);
    let (screen_w, screen_h) = screen_dims;

    // Ensure each window that needs positioning has a river_node_v1
    ensure_window_nodes(state, qhandle);

    // Compute tiling (read-only pass over windows)
    let tile_results = compute_tiling(&state.wm, screen_w, screen_h);

    // Apply tiling results (mutations)
    apply_tiling(state, &tile_results);

    // Set border colors
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
    let bw = wm.layout.border_width;
    let offset = wm.layout.offset;
    let bar_height = wm.layout.bar_height;

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

    // Check for fullscreen window
    let fullscreen_id = wm
        .windows
        .iter()
        .find(|w| {
            (w.tags & wm.active_tags) != 0
                && !w.closed
                && w.tiling_mode == TilingMode::Fullscreen
        })
        .map(|w| w.id);

    // Compute tiling
    let mut results = Vec::new();
    let mut idx_cascade = 0i32;
    let mut idx_grid = 0i32;

    for win in &wm.windows {
        if (win.tags & wm.active_tags) == 0 || win.closed {
            continue;
        }

        let wid = win.id;
        let mode = win.tiling_mode;

        let (x, y, w, h) = match mode {
            TilingMode::Fullscreen => {
                if fullscreen_id == Some(wid) {
                    (0, 0, screen_w, screen_h)
                } else {
                    continue;
                }
            }
            TilingMode::Cascade => {
                let (x, y, w, h) = tiling::tile_cascade(
                    screen_w,
                    screen_h,
                    gap,
                    bw,
                    offset,
                    bar_height,
                    n_cascade,
                    idx_cascade,
                );
                idx_cascade += 1;
                (x, y, w, h)
            }
            TilingMode::Grid => {
                let (x, y, w, h) =
                    tiling::tile_grid(screen_w, screen_h, gap, bw, bar_height, n_grid, idx_grid);
                idx_grid += 1;
                (x, y, w, h)
            }
            TilingMode::Vsplit | TilingMode::Hsplit => {
                // TODO: implement vsplit/hsplit
                continue;
            }
            TilingMode::Floating => {
                continue;
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
    let (border_colors, bg_color) = compute_border_colors(&state.wm);

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

    // Update desktop background if cascade color changed
    if let Some(ref color) = bg_color {
        if *color != state.wm.last_bg_color {
            state.wm.last_bg_color = color.clone();
            spawn_swaybg(color);
        }
    }
}

/// Spawn swaybg with the given color (fire-and-forget).
fn spawn_swaybg(color: &str) {
    let cmd = format!("pkill -f swaybg 2>/dev/null; swaybg -c '{}'", color);
    unsafe {
        match nix::unistd::fork() {
            Ok(nix::unistd::ForkResult::Child) => {
                nix::unistd::close(nix::libc::STDIN_FILENO).ok();
                nix::unistd::close(nix::libc::STDOUT_FILENO).ok();
                nix::unistd::close(nix::libc::STDERR_FILENO).ok();
                nix::unistd::execvp(
                    &std::ffi::CString::new("/bin/sh").unwrap(),
                    &[
                        std::ffi::CString::new("sh").unwrap(),
                        std::ffi::CString::new("-c").unwrap(),
                        std::ffi::CString::new(cmd).unwrap(),
                    ],
                )
                .ok();
                // If exec fails, exit child
                libc::_exit(127);
            }
            Ok(nix::unistd::ForkResult::Parent { .. }) => {}
            Err(_) => {}
        }
    }
}
