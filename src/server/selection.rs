//! Drag-selection of windows in overview.
//!
//! A left press on the bare desktop in overview, dragged, stretches a
//! rectangle from the press point to the pointer; every window the
//! rectangle touches is selected, live, while the button is held. It is the
//! compositor's version of cce-designer's network cursor region, with one
//! difference that follows from what is being selected: a node sits on one
//! lattice cell, so the designer asks whether that cell is inside the
//! region, while a window spans many cells and in overview the background
//! between two of them is a narrow strip — so here touching is enough.
//!
//! What the selection is for: pressing the body of a selected window moves
//! every selected window together (`Seat::group_move`, filled at the grab).
//! Like the designer, a new drag replaces the selection, a press on a window
//! outside it drops it, and there are no modifiers.
//!
//! The desktop IMAGES select the same way. They are `cce-grid`'s (its
//! desktop items, pinned to the virtual canvas), and the compositor never
//! sees them as anything but the grid surface's input region — so the grid
//! reports them over the control socket (`grid-items <id>:<x>:<y>:<w>:<h>
//! ...`, virtual units, on every change) and this module keeps the list
//! (`Selection::desktop_items`). The band picks them up by the same touch
//! rule, the highlight is drawn here beside the windows', and a group move
//! carries them: the compositor moves the rects it holds and pushes the new
//! positions over the status socket's `selection` topic (`move
//! <id>:<x>:<y> ...`, then `drop` at the release, on which the grid saves
//! its sidecar and reports the list afresh). A press on a selected image
//! starts the group move from the image's side (`PointerOpType::GroupMove`
//! — no grabbed window, the delta is the pointer's); a press on an
//! unselected one drops the selection and goes to the grid as before.
//!
//! The press that starts the drag used to exit overview on the spot. It
//! still does, on RELEASE, when the pointer never travelled: the press
//! cannot know which of the two it is.
//!
//! The rectangle is anchored in VIRTUAL (desk) coordinates, so it stays put
//! on the desk while the edge auto-pan scrolls the camera under a held drag.
//! Selection exists only in overview and is dropped when the mode is left
//! (`WindowManager::set_mode`).
//!
//! Drawing is a pool of (fill rect, glint bevel) pairs in one tree that
//! hangs off the scene ROOT, outside `interactive_tree`: `Scene::at` stops
//! at the first node it meets, and a node with no `SceneNodeData` reads as
//! "nothing here", so a highlight inside the interactive tree would turn a
//! press on a selected window into a press on the background.

use crate::ffi;
use crate::server::WlList;
use crate::window::Window;
use crate::window_manager::{WindowManager, WindowManagerMode};

/// Pointer travel, in layout px on either axis, past which a background
/// press is a drag and not a click. The same figure the overview tap on a
/// window uses (`handle_button`'s release path).
pub const DRAG_THRESHOLD: i32 = 5;

/// Fill opacity of the rubber band, and of the wash over a selected window.
const MARQUEE_FILL: f32 = 0.14;
const SELECTED_FILL: f32 = 0.12;
/// Corner radius of the rubber band, screen px.
const MARQUEE_RADIUS: f64 = 6.0;

/// One of the grid's desktop images, as it last reported it: a
/// per-process id the grid assigns, and its rect in virtual units. The type
/// and its token format are cce-core's, shared with the grid that sends it.
pub use cce_core::ipc::ctl::DesktopItem;

/// Parse the grid's report: whitespace-separated `id:x:y:w:h` tokens, any
/// malformed one skipped. An empty report is an empty desk.
pub fn parse_desktop_items(tokens: &[&str]) -> Vec<DesktopItem> {
    tokens.iter().filter_map(|tok| DesktopItem::parse(tok)).collect()
}

/// The topmost item under a virtual point. The grid draws its list in
/// order, last on top, and reports it in that order.
pub fn item_at(items: &[DesktopItem], vx: f64, vy: f64) -> Option<u64> {
    items
        .iter()
        .rev()
        .find(|i| vx >= i.x && vx < i.x + i.w && vy >= i.y && vy < i.y + i.h)
        .map(|i| i.id)
}

/// The rubber band: the press point and the pointer, in virtual coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marquee {
    pub anchor: (f64, f64),
    pub far: (f64, f64),
}

impl Marquee {
    /// `(x, y, width, height)`, whichever way the drag went.
    pub fn rect(&self) -> (f64, f64, f64, f64) {
        let x = self.anchor.0.min(self.far.0);
        let y = self.anchor.1.min(self.far.1);
        (
            x,
            y,
            (self.anchor.0 - self.far.0).abs(),
            (self.anchor.1 - self.far.1).abs(),
        )
    }
}

/// Does `rect` touch `win`? Both `(x, y, width, height)`. Sharing only an
/// edge is not touching, and a window with no area is never touched. The
/// band itself may be a line: a drag straight down through a window has no
/// width and still crosses it.
pub fn touches(rect: (f64, f64, f64, f64), win: (f64, f64, f64, f64)) -> bool {
    let (rx, ry, rw, rh) = rect;
    let (wx, wy, ww, wh) = win;
    ww > 0.0 && wh > 0.0 && rx < wx + ww && wx < rx + rw && ry < wy + wh && wy < ry + rh
}

pub struct Selection {
    /// The selected windows, in `wm.windows` order. Entries are dropped in
    /// `Window::destroy`; a window that is merely unmapped or minimized
    /// stays listed and is skipped where it matters.
    pub windows: Vec<*mut Window>,
    /// The selected desktop images, by the grid's id, in report order.
    pub items: Vec<u64>,
    /// Every desktop image the grid has reported, in its draw order. Moved
    /// here during a group move so the highlight follows; the grid's next
    /// report replaces the lot.
    pub desktop_items: Vec<DesktopItem>,
    /// Where the armed background press landed (virtual). Set on press,
    /// cleared on release.
    pub anchor: Option<(f64, f64)>,
    /// The rubber band, once the press has travelled past the threshold.
    pub marquee: Option<Marquee>,
    tree: crate::scene_handle::SceneTree,
    boxes: Vec<(crate::scene_handle::SceneRect, crate::scene_handle::SceneBevel)>,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            windows: Vec::new(),
            items: Vec::new(),
            desktop_items: Vec::new(),
            anchor: None,
            marquee: None,
            tree: crate::scene_handle::SceneTree::none(),
            boxes: Vec::new(),
        }
    }
}

impl WindowManager {
    /// Layout box origin of the first enabled output, the one the camera is
    /// measured against (`Window::virtual_to_screen` uses the same).
    unsafe fn selection_output_origin(&self) -> (f64, f64) {
        let outputs_list = &(*self.server).om.outputs as *const ffi::wl_list as *mut WlList;
        let mut curr = (*outputs_list).next;
        while curr != outputs_list {
            let output = crate::container_of!(curr, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let b = (*output).sent.box_layout();
                return (b.x as f64, b.y as f64);
            }
            curr = (*curr).next;
        }
        (0.0, 0.0)
    }

    pub unsafe fn layout_to_virtual(&self, lx: f64, ly: f64) -> (f64, f64) {
        let (cam, _, _) = self.layout_camera();
        let zoom = cam.zoom.max(0.01);
        let (ox, oy) = self.selection_output_origin();
        (cam.pan_x + (lx - ox) / zoom, cam.pan_y + (ly - oy) / zoom)
    }

    pub unsafe fn virtual_to_layout(&self, vx: f64, vy: f64) -> (f64, f64) {
        let (cam, _, _) = self.layout_camera();
        let (ox, oy) = self.selection_output_origin();
        (ox + (vx - cam.pan_x) * cam.zoom, oy + (vy - cam.pan_y) * cam.zoom)
    }

    pub fn is_selected(&self, window: *mut Window) -> bool {
        self.selection.windows.iter().any(|&w| w == window)
    }

    pub fn is_item_selected(&self, id: u64) -> bool {
        self.selection.items.iter().any(|&i| i == id)
    }

    /// Anything selected at all — what a background click drops, and what
    /// a press on a selected image carries.
    pub fn has_selection(&self) -> bool {
        !self.selection.windows.is_empty() || !self.selection.items.is_empty()
    }

    /// The selected desktop image under a virtual point, if the topmost
    /// one there is selected.
    pub fn selected_item_at(&self, vx: f64, vy: f64) -> Option<u64> {
        item_at(&self.selection.desktop_items, vx, vy).filter(|&id| self.is_item_selected(id))
    }

    pub fn desktop_item(&self, id: u64) -> Option<&DesktopItem> {
        self.selection.desktop_items.iter().find(|i| i.id == id)
    }

    pub fn desktop_item_mut(&mut self, id: u64) -> Option<&mut DesktopItem> {
        self.selection.desktop_items.iter_mut().find(|i| i.id == id)
    }

    /// The grid reported its images (`grid-items`). The list is replaced
    /// wholesale; a selected id that is no longer in it was removed or
    /// belongs to a restarted grid, and is dropped.
    pub unsafe fn set_desktop_items(&mut self, items: Vec<DesktopItem>) {
        self.selection.items.retain(|id| items.iter().any(|i| i.id == *id));
        self.selection.desktop_items = items;
        self.schedule_frame_all_outputs();
    }

    /// Arm a drag-selection at a background press. Nothing is selected or
    /// drawn until the pointer travels (`selection_motion`).
    pub unsafe fn selection_press(&mut self, lx: f64, ly: f64) {
        self.selection.anchor = Some(self.layout_to_virtual(lx, ly));
        self.selection.marquee = None;
    }

    /// The pointer moved with the press held. `travelled` is whether it has
    /// left the click threshold; once it has, the band is up for the rest of
    /// the press even if the pointer comes back to where it started.
    pub unsafe fn selection_motion(&mut self, lx: f64, ly: f64, travelled: bool) {
        let Some(anchor) = self.selection.anchor else { return };
        if self.selection.marquee.is_none() && !travelled {
            return;
        }
        let marquee = Marquee { anchor, far: self.layout_to_virtual(lx, ly) };
        self.selection.marquee = Some(marquee);
        let rect = marquee.rect();
        let mut picked: Vec<*mut Window> = Vec::new();
        for &w in self.windows.iter() {
            if !self.selectable(w) {
                continue;
            }
            let win = (
                (*w).virtual_x,
                (*w).virtual_y,
                (*w).box_geom.width as f64,
                (*w).box_geom.height as f64,
            );
            if touches(rect, win) {
                picked.push(w);
            }
        }
        if picked != self.selection.windows {
            log::debug!("selection: {} window(s)", picked.len());
            self.selection.windows = picked;
        }
        let picked_items: Vec<u64> = self
            .selection
            .desktop_items
            .iter()
            .filter(|i| touches(rect, i.rect()))
            .map(|i| i.id)
            .collect();
        if picked_items != self.selection.items {
            log::debug!("selection: {} image(s)", picked_items.len());
            self.selection.items = picked_items;
        }
        self.schedule_frame_all_outputs();
    }

    /// The press ended. Returns whether it was a drag: a press that never
    /// travelled is a click on the background, which the caller answers by
    /// leaving overview.
    pub unsafe fn selection_release(&mut self) -> bool {
        let dragged = self.selection.marquee.is_some();
        self.selection.anchor = None;
        self.selection.marquee = None;
        self.schedule_frame_all_outputs();
        dragged
    }

    pub unsafe fn selection_clear(&mut self) {
        if !self.has_selection() && self.selection.marquee.is_none() {
            return;
        }
        self.selection.windows.clear();
        self.selection.items.clear();
        self.selection.marquee = None;
        self.schedule_frame_all_outputs();
    }

    /// `window` is being destroyed: it must not stay listed for a window
    /// allocated at the same address to inherit. The grid going away takes
    /// its images with it.
    pub unsafe fn selection_forget(&mut self, window: *mut Window) {
        let before = self.selection.windows.len();
        self.selection.windows.retain(|&w| w != window);
        let mut changed = self.selection.windows.len() != before;
        if (*window).is_grid() && !self.selection.desktop_items.is_empty() {
            self.selection.desktop_items.clear();
            self.selection.items.clear();
            changed = true;
        }
        if changed {
            self.schedule_frame_all_outputs();
        }
    }

    /// Windows the band can pick up, and a group move can carry: the ones
    /// with a place on the desk. The same set the overview displacement
    /// considers (`seat::displace_covered`).
    pub unsafe fn selectable(&self, w: *mut Window) -> bool {
        if w.is_null() || (*w).closed || (*w).minimized {
            return false;
        }
        if !matches!((*w).state, crate::window::WindowState::Mapped) {
            return false;
        }
        if (*w).is_status_bar() || (*w).is_wallpaper() || (*w).is_grid() {
            return false;
        }
        matches!(
            self.get_mode_for_window(w),
            crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Tiled
        )
    }

    /// `selectable`, for a window a group move is already carrying: the
    /// drag floats a Tiled one, which `selectable` would still accept, but
    /// the pointer may dangle if the window closed mid-drag — so this one
    /// checks it is still a window before looking at it.
    pub unsafe fn selectable_in_drag(&self, w: *mut Window) -> bool {
        self.windows.iter().any(|&p| p == w) && self.selectable(w)
    }

    /// Place the band and the selected windows' highlights for the frame
    /// about to render. Called from `Output::render_and_commit` beside the
    /// grid, so the highlights follow a window through a drag and the camera
    /// through a pan; with nothing selected it is one disabled tree.
    pub unsafe fn draw_selection(&mut self) {
        let showing = self.mode == WindowManagerMode::Overview
            && (*self.server).lock_manager.state == crate::lock_manager::LockState::Unlocked
            && (self.selection.marquee.is_some() || self.has_selection());
        if !showing {
            if !self.selection.tree.is_null() {
                ffi::wlr_scene_node_set_enabled(
                    self.selection.tree.node(),
                    false,
                );
            }
            return;
        }

        // (x, y, width, height, corner radius, fill opacity), layout px.
        let mut wanted: Vec<(i32, i32, i32, i32, i32, f32)> = Vec::new();
        for &w in self.selection.windows.iter() {
            if !self.selectable(w) || (*w).rendering_requested.hidden {
                continue;
            }
            let sc = if (*w).scale > 0.0 { (*w).scale } else { 1.0 };
            let g = (*w).box_geom;
            let radius = crate::window::widen_corner_radius(
                (*w).root_plate_radius_base(),
                g.width,
                g.height,
            );
            wanted.push((
                g.x,
                g.y,
                (g.width as f64 * sc) as i32,
                (g.height as f64 * sc) as i32,
                (radius as f64 * sc) as i32,
                SELECTED_FILL,
            ));
        }
        // The images are square-cornered quads on the grid surface, so
        // their wash is too.
        for &id in self.selection.items.iter() {
            let Some(item) = self.desktop_item(id) else { continue };
            let (x0, y0) = self.virtual_to_layout(item.x, item.y);
            let (x1, y1) = self.virtual_to_layout(item.x + item.w, item.y + item.h);
            wanted.push((
                x0.round() as i32,
                y0.round() as i32,
                (x1 - x0).round() as i32,
                (y1 - y0).round() as i32,
                0,
                SELECTED_FILL,
            ));
        }
        if let Some(marquee) = self.selection.marquee {
            let (vx, vy, vw, vh) = marquee.rect();
            let (x0, y0) = self.virtual_to_layout(vx, vy);
            let (x1, y1) = self.virtual_to_layout(vx + vw, vy + vh);
            let (w, h) = ((x1 - x0).round() as i32, (y1 - y0).round() as i32);
            let radius = MARQUEE_RADIUS.min(w.min(h) as f64 / 2.0) as i32;
            wanted.push((x0.round() as i32, y0.round() as i32, w, h, radius, MARQUEE_FILL));
        }

        if self.selection.tree.is_null() {
            let scene = &(*self.server).scene;
            let tree = crate::scene_handle::SceneTree::create_in(&mut (*scene.wlr_scene).tree);
            if tree.is_null() {
                return;
            }
            // Over everything interactive, under a drag icon.
            tree.place_above(&scene.interactive_tree);
            // Desk content, like the windows it marks: rendered with the
            // camera's sub-pixel offset.
            ffi::river_scene_tree_set_desk_offset(tree.raw(), true);
            self.selection.tree = tree;
        }
        self.selection.tree.set_enabled(true);
        let tree = self.selection.tree.raw();

        let layout = &self.layout;
        let accent = layout.bevel_focus_color;
        let (light_x, light_y) = {
            let (lx, ly) = (layout.bevel_light_x, layout.bevel_light_y);
            let len = (lx * lx + ly * ly).sqrt();
            if len > 1e-6 { (lx / len, ly / len) } else { (-0.7071, -0.7071) }
        };
        // The glint has to read whatever the window bevels are set to,
        // including off: it is the selection's outline, not a bevel.
        let thickness = layout.bevel_thickness.max(2.0);
        let light = layout.bevel_light_intensity.max(0.6);

        let mut used = 0;
        for &(x, y, w, h, radius, fill) in wanted.iter() {
            if w < 1 || h < 1 {
                continue;
            }
            // wlr_scene_rect colours are premultiplied.
            let color = [accent[0] * fill, accent[1] * fill, accent[2] * fill, fill];
            if used == self.selection.boxes.len() {
                let rect = crate::scene_handle::SceneRect::adopt(ffi::wlr_scene_rect_create(tree, w, h, color.as_ptr()));
                let bevel = crate::scene_handle::SceneBevel::create_in(tree, w, h, radius, thickness, &layout.bevel_color);
                if rect.is_null() || bevel.is_null() {
                    // Dropping them destroys whichever was made.
                    break;
                }
                self.selection.boxes.push((rect, bevel));
            }
            let (rect, bevel) = (self.selection.boxes[used].0.raw(), self.selection.boxes[used].1.raw());
            used += 1;

            let rect_node = rect as *mut ffi::wlr_scene_node;
            ffi::wlr_scene_node_set_enabled(rect_node, true);
            ffi::river_scene_node_set_position_if_changed(rect_node, x, y);
            ffi::river_scene_rect_set_size_if_changed(rect, w, h);
            ffi::river_scene_rect_set_corner_radius(rect, radius);
            ffi::wlr_scene_rect_set_color(rect, color.as_ptr());

            let bevel_node = &mut (*bevel).node as *mut ffi::wlr_scene_node;
            ffi::wlr_scene_node_set_enabled(bevel_node, true);
            ffi::river_scene_node_set_position_if_changed(bevel_node, x, y);
            ffi::wlr_scene_bevel_set_size(bevel, w, h);
            ffi::wlr_scene_bevel_set_corner_radius(bevel, radius);
            ffi::wlr_scene_bevel_set_thickness(bevel, thickness);
            ffi::wlr_scene_bevel_set_light(
                bevel,
                light_x,
                light_y,
                light,
                layout.bevel_shade_intensity,
            );
            ffi::wlr_scene_bevel_set_shoulder(bevel, layout.bevel_shoulder);
            ffi::wlr_scene_bevel_set_color(bevel, layout.bevel_color.as_ptr());
            // Focus 1 is the shader's glint-only branch: the accent on the
            // rim and nothing else, the tint the designer marks a region
            // and its nodes with.
            ffi::wlr_scene_bevel_set_focus(
                bevel,
                1.0,
                layout.bevel_focus_sharpness,
                accent.as_ptr(),
            );
        }
        for (rect, bevel) in self.selection.boxes.iter().skip(used) {
            rect.set_enabled(false);
            bevel.set_enabled(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_is_the_same_whichever_way_the_drag_went() {
        let down_right = Marquee { anchor: (10.0, 20.0), far: (110.0, 70.0) };
        let up_left = Marquee { anchor: (110.0, 70.0), far: (10.0, 20.0) };
        let mixed = Marquee { anchor: (110.0, 20.0), far: (10.0, 70.0) };
        let want = (10.0, 20.0, 100.0, 50.0);
        assert_eq!(down_right.rect(), want);
        assert_eq!(up_left.rect(), want);
        assert_eq!(mixed.rect(), want);
    }

    #[test]
    fn a_band_that_clips_a_corner_selects_the_window() {
        let win = (100.0, 100.0, 400.0, 300.0);
        assert!(touches((50.0, 50.0, 60.0, 60.0), win));
        // Entirely inside the window's bounds is unreachable from a
        // background press, but it is still a touch.
        assert!(touches((200.0, 200.0, 10.0, 10.0), win));
        // And the window entirely inside the band.
        assert!(touches((0.0, 0.0, 1000.0, 1000.0), win));
    }

    #[test]
    fn a_band_in_the_gap_beside_a_window_selects_nothing() {
        let win = (100.0, 100.0, 400.0, 300.0);
        assert!(!touches((0.0, 100.0, 90.0, 300.0), win));
        // Sharing an edge is not touching.
        assert!(!touches((0.0, 100.0, 100.0, 300.0), win));
        assert!(!touches((100.0, 400.0, 400.0, 50.0), win));
    }

    #[test]
    fn a_straight_drag_through_a_window_selects_it() {
        let win = (100.0, 100.0, 400.0, 300.0);
        // Pressed above the window, dragged straight down through it.
        assert!(touches((200.0, 50.0, 0.0, 200.0), win));
        // The same line beside the window.
        assert!(!touches((50.0, 50.0, 0.0, 200.0), win));
    }

    #[test]
    fn a_window_with_no_area_is_never_touched() {
        assert!(!touches((0.0, 0.0, 1000.0, 1000.0), (100.0, 100.0, 0.0, 300.0)));
    }

    #[test]
    fn the_grid_report_parses_and_skips_a_bad_token() {
        let items = parse_desktop_items(&["3:10.5:20:300:200", "junk", "4:0:0:1:1:extra", "x:1:2:3:4"]);
        assert_eq!(
            items,
            vec![
                DesktopItem { id: 3, x: 10.5, y: 20.0, w: 300.0, h: 200.0 },
                DesktopItem { id: 4, x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
            ]
        );
        assert!(parse_desktop_items(&[]).is_empty());
    }

    #[test]
    fn the_last_reported_image_is_on_top() {
        let items = vec![
            DesktopItem { id: 1, x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            DesktopItem { id: 2, x: 50.0, y: 50.0, w: 100.0, h: 100.0 },
        ];
        assert_eq!(item_at(&items, 75.0, 75.0), Some(2));
        assert_eq!(item_at(&items, 10.0, 10.0), Some(1));
        assert_eq!(item_at(&items, 200.0, 200.0), None);
        // Half-open: the far edge belongs to nothing.
        assert_eq!(item_at(&items, 150.0, 150.0), None);
    }
}
