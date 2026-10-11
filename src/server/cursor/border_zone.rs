//! Hit-testing a window's border and the grid layer: which edge or handle a point
//! is on, and the resize cursor for it. Split out of cursor.rs on 2026-10-10.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderZone {
    None,
    Move,
    Resize(crate::window::Edges),
    /// A window button disc (`BorderElement::is_button`).
    Button(crate::window::BorderElement),
}

pub use crate::window::HOVER_BAND_MIN;

/// The grid client's surface and the surface-local coordinates of a layout
/// point, ignoring the input region.
///
/// Hit-testing cannot be used for this: the grid's input region covers only
/// its desktop items, so a point over bare canvas — where drops mostly land —
/// misses it by design. A drop target does not need to be hit-testable, only
/// named, so the surface is resolved directly and the point is mapped through
/// the surface node's own layout origin and the window's display scale, which
/// is the same pair the renderer draws with.
pub unsafe fn grid_surface_at(
    server: *mut crate::server::Server,
    lx: f64,
    ly: f64,
) -> Option<(*mut ffi::wlr_surface, f64, f64)> {
    let (surface, nx, ny, scale, bw, bh) = grid_node_info(server)?;
    let (sx, sy) = ((lx - nx) / scale, (ly - ny) / scale);
    // Only claim points that actually fall on the grid's patch. box_geom
    // is the surface's own logical size, which is the space sx/sy are in.
    if sx < 0.0 || sy < 0.0 || (bw > 0.0 && sx >= bw) || (bh > 0.0 && sy >= bh) {
        return None;
    }
    Some((surface, sx, sy))
}

/// The grid window's surface plus the raw mapping ingredients: its surface
/// tree's layout origin, its display scale, and its logical size. This is
/// the node state a caller may need to FREEZE (the implicit-grab path
/// captures it at press time), which is why it is exposed separately from
/// the point mapping above.
pub unsafe fn grid_node_info(
    server: *mut crate::server::Server,
) -> Option<(*mut ffi::wlr_surface, f64, f64, f64, f64, f64)> {
    for &w in (*crate::reentry::wm(server)).windows.iter() {
        if w.is_null() || (*w).closed || !(*w).is_grid() {
            continue;
        }
        if !matches!((*w).state, crate::window::WindowState::Mapped) {
            continue;
        }
        let surface = (*w).root_surface();
        if surface.is_null() {
            continue;
        }
        let node = (*w).surfaces.tree.node();
        let (mut nx, mut ny) = (0, 0);
        if !ffi::wlr_scene_node_coords(node, &mut nx, &mut ny) {
            continue;
        }
        let scale = if (*w).scale > 0.0 { (*w).scale } else { 1.0 };
        return Some((
            surface,
            nx as f64,
            ny as f64,
            scale,
            (*w).box_geom.width as f64,
            (*w).box_geom.height as f64,
        ));
    }
    None
}

/// Which handle disc, if any, a layout point falls on.
///
/// The handles are eight discs INSIDE the content rect — one on each side,
/// one on each corner — plus, for a Floating or Tiled window, the three
/// window buttons in the top row (`BorderZone::Button`), and exist only in
/// adjust mode (overview, or Super held). Two consequences worth stating, because
/// both are deliberate:
///
///   - Outside adjust mode there are no handles at all, so a window cannot
///     be resized with the pointer. The keyboard and IPC paths
///     (`move_window_*`, `ccectl move-window`, a client repositioning
///     itself) are untouched; this is only about dragging.
///   - Every disc RESIZES, the top one included. Moving is what dragging
///     the window's body does in adjust mode, so no handle has to be spent
///     on it; between two discs the pointer belongs to the body.
///
/// `window::handle_disc_layout` places the discs for the hit test here,
/// the catchers in `draw_borders`, and (mirrored in the frame shader) the
/// drawing, so the zones and the visuals cannot drift.
pub unsafe fn get_border_zone(window: *mut crate::window::Window, lx: f64, ly: f64) -> BorderZone {
    // Overview, or Super held (window-adjust mode): the same ring either way.
    if !crate::shared::window_adjust_active() {
        return BorderZone::None;
    }
    if !crate::window::window_takes_handles(window) {
        return BorderZone::None;
    }
    // The adjust target only — the window under the pointer — matching what
    // draw_borders draws. A window showing no ring has no band, and a grab
    // that is not drawn is the failure mode this file keeps warning about.
    // The pointer reaches a window's edge through its body, so the band is
    // live by the time it arrives.
    if !(*window).is_adjust_target() {
        return BorderZone::None;
    }

    let bw_unscaled = crate::window::border_band_width((*window).rendering_requested.border.width);
    if bw_unscaled <= 0.0 {
        return BorderZone::None;
    }

    // box_geom holds the UNSCALED content size; on screen the window covers
    // `size * scale`, and lx/ly are layout px — so the CONTENT extents scale
    // but the discs do NOT. The disc diameter is a screen size, matching
    // what draw_borders draws: overview is zoomed out, and a handle that
    // shrank with the window would be smallest exactly where it is the only
    // way to resize. Keep the two in step.
    let scale = if (*window).scale > 0.0 { (*window).scale } else { 1.0 };
    let geom = (*window).box_geom;
    let rx = lx - geom.x as f64;
    let ry = ly - geom.y as f64;
    let content_w = geom.width as f64 * scale;
    let content_h = geom.height as f64 * scale;

    let bw = (crate::shared::layout().border_handle_width as f64)
        .max(crate::window::HOVER_BAND_MIN)
        // The same fifth-of-the-short-side cap draw_borders applies, so the
        // grab zone never outgrows the disc the user can see.
        .min(content_w.min(content_h).max(1.0) * 0.2);

    // Outside the window entirely: not ours.
    if rx < 0.0 || rx >= content_w || ry < 0.0 || ry >= content_h {
        return BorderZone::None;
    }
    // A client popover (set_popover_region) owns its rect outright: the menu
    // reads as in front of the chrome, so nothing under it may grab. Checked
    // before the discs — it beats them.
    if let Some(r) = (*window).popover_region {
        let (ex, ey) = (r.x as f64 * scale, r.y as f64 * scale);
        let (ew, eh) = (r.width as f64 * scale, r.height as f64 * scale);
        if rx >= ex && rx < ex + ew && ry >= ey && ry < ey + eh {
            return BorderZone::None;
        }
    }

    // The discs, from the same on-screen size, silhouette radius and
    // diameter draw_borders hands the frame shader, so what is drawn is
    // what grabs. A pixel of slack covers the antialiased rim. The corner
    // discs place against the content radius: the widened root plate
    // radius the corner clip uses, on screen.
    let r_in = crate::window::widen_corner_radius(
        (*window).root_plate_radius_base(),
        geom.width,
        geom.height,
    ) as f64;
    let r_in = (r_in * scale) as i32 as f64;
    let (centres, r, live) = crate::window::handle_disc_layout(
        content_w,
        content_h,
        r_in,
        bw,
        crate::window::window_takes_buttons(window),
    );
    let reach = (r + 1.0) * (r + 1.0);
    for (i, &(cx, cy)) in centres[..live].iter().enumerate() {
        let (dx, dy) = (rx - cx, ry - cy);
        if dx * dx + dy * dy <= reach {
            let elem = crate::window::BorderElement::ALL[i];
            if elem.is_button() {
                return BorderZone::Button(elem);
            }
            return BorderZone::Resize(edges_for_border_element(elem));
        }
    }
    // In the body, between the discs: not ours. This is what leaves the
    // body drag-to-move working.
    BorderZone::None
}

/// The resize edges a handle disc stands for — the inverse of
/// `border_element_for_edges`.
pub fn edges_for_border_element(element: crate::window::BorderElement) -> crate::window::Edges {
    use crate::window::BorderElement::*;
    let (top, bottom, left, right) = match element {
        Top => (true, false, false, false),
        Bottom => (false, true, false, false),
        Left => (false, false, true, false),
        Right => (false, false, false, true),
        TopLeft => (true, false, true, false),
        TopRight => (true, false, false, true),
        BottomLeft => (false, true, true, false),
        BottomRight => (false, true, false, true),
        // A button resizes nothing.
        Minimize | Maximize | ToggleTile => (false, false, false, false),
    };
    crate::window::Edges { top, bottom, left, right }
}

/// Map a resize zone's edges to the border element that should highlight.
pub fn border_element_for_edges(edges: crate::window::Edges) -> crate::window::BorderElement {
    use crate::window::BorderElement::*;
    match (edges.top, edges.bottom, edges.left, edges.right) {
        (true, _, true, _) => TopLeft,
        (true, _, _, true) => TopRight,
        (_, true, true, _) => BottomLeft,
        (_, true, _, true) => BottomRight,
        (_, true, _, _) => Bottom,
        (_, _, true, _) => Left,
        (_, _, _, true) => Right,
        _ => Top,
    }
}

pub fn get_resize_cursor_name(edges: crate::window::Edges) -> &'static [u8] {
    if edges.top && edges.left {
        b"nw-resize\0"
    } else if edges.top && edges.right {
        b"ne-resize\0"
    } else if edges.bottom && edges.left {
        b"sw-resize\0"
    } else if edges.bottom && edges.right {
        b"se-resize\0"
    } else if edges.top {
        b"n-resize\0"
    } else if edges.bottom {
        b"s-resize\0"
    } else if edges.left {
        b"w-resize\0"
    } else if edges.right {
        b"e-resize\0"
    } else {
        b"default\0"
    }
}

pub unsafe fn get_closest_edges(window: *mut crate::window::Window, lx: f64, ly: f64) -> crate::window::Edges {
    let geom = (*window).box_geom;
    let rx = lx - geom.x as f64;
    let ry = ly - geom.y as f64;
    let w = geom.width as f64;
    let h = geom.height as f64;

    let left = rx < w / 2.0;
    let right = !left;
    let top = ry < h / 2.0;
    let bottom = !top;

    crate::window::Edges { top, bottom, left, right }
}
