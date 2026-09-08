// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlList, wl_list_insert, wl_list_remove, wl_list_remove_and_reinit, WlListener, wl_signal_add};
use crate::wm_node::WmNode;
use crate::xdg_toplevel::ConfigureState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    Init,
    Ready,
    Initialized,
    Mapped,
    Closing,
}

#[derive(Clone, Copy)]
pub enum WindowImpl {
    Toplevel(*mut crate::xdg_toplevel::XdgToplevel),
    Xwayland(*mut crate::xwayland_window::XwaylandWindow),
    Destroying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullscreenRequest {
    NoRequest,
    Fullscreen(*mut crate::output::Output),
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaximizeRequest {
    NoRequest,
    Maximize,
    Unmaximize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DimensionsHint {
    pub min_width: u32,
    pub min_height: u32,
    pub max_width: u32,
    pub max_height: u32,
}

impl DimensionsHint {
    /// Clamp a requested content size to the client's declared range; a
    /// zero bound is "unset" (xdg-shell's convention) and leaves that side
    /// alone. A max below the min is the client's own contradiction and
    /// the min wins.
    pub fn clamp(&self, width: u32, height: u32) -> (u32, u32) {
        let mut w = width;
        let mut h = height;
        if self.max_width > 0 {
            w = w.min(self.max_width);
        }
        if self.max_height > 0 {
            h = h.min(self.max_height);
        }
        if self.min_width > 0 {
            w = w.max(self.min_width);
        }
        if self.min_height > 0 {
            h = h.max(self.min_height);
        }
        (w, h)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Edges {
    pub fn new() -> Self {
        Self { top: false, bottom: false, left: false, right: false }
    }
    pub fn from_u32(val: u32) -> Self {
        Self {
            top: (val & 1) != 0,
            bottom: (val & 2) != 0,
            left: (val & 4) != 0,
            right: (val & 8) != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub edges: Edges,
    pub width: u32,
    /// Premultiplied-alpha RGBA, 0.0–1.0 per channel (scenefx convention).
    pub color: [f32; 4],
    /// Color while the pointer hovers the border (the grab surface).
    pub hover_color: [f32; 4],
    pub corner_radius: i32,
}

impl Border {
    pub fn none() -> Self {
        Self { edges: Edges::new(), width: 0, color: [0.0; 4], hover_color: [0.0; 4], corner_radius: 0 }
    }
}

/// A window-scale corner radius as scenefx should consume it: the configured
/// nominal (circle-equivalent) radius widened by the curvature-match span
/// factor, capped at half the smaller content extent so opposite corners
/// can't overlap — the exact counterpart of cce-ui's
/// `VkRenderer::clip_corner_radius`, which widens the clients' plate/clip
/// corners the same way. `width`/`height` and the returned radius are in
/// logical px; callers scale to device px where they already do.
pub fn widen_corner_radius(nominal: i32, width: i32, height: i32) -> i32 {
    if nominal <= 0 {
        return nominal;
    }
    let widened = (nominal as f64 * crate::config::corner_span_factor()).round() as i32;
    widened.min(width.min(height) / 2)
}

/// One of the 8 interactive border zones. Each draws as its own visual
/// element (corners as two-rect Ls) and highlights independently on hover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderElement {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl BorderElement {
    /// Every zone, in `index()` order.
    pub const ALL: [BorderElement; 8] = [
        BorderElement::Top,
        BorderElement::Bottom,
        BorderElement::Left,
        BorderElement::Right,
        BorderElement::TopLeft,
        BorderElement::TopRight,
        BorderElement::BottomLeft,
        BorderElement::BottomRight,
    ];

    /// Index into `Window::border_reveal`. Declaration order; kept in one
    /// place so the reveal array and the enum can't drift apart.
    pub fn index(self) -> usize {
        match self {
            BorderElement::Top => 0,
            BorderElement::Bottom => 1,
            BorderElement::Left => 2,
            BorderElement::Right => 3,
            BorderElement::TopLeft => 4,
            BorderElement::TopRight => 5,
            BorderElement::BottomLeft => 6,
            BorderElement::BottomRight => 7,
        }
    }
}

/// Per-frame step of the hover fade, as a fraction of the remaining distance
/// to the target (the same exponential-approach shape the viewport pan uses).
pub const BORDER_FADE_STEP: f32 = 0.25;
/// Below this the fade is treated as finished and snapped to its target.
pub const BORDER_FADE_EPSILON: f32 = 0.004;

/// Per-tick step of the fullscreen-toggle animation, as a fraction of the
/// remaining distance to the target rect (the pan/border-fade shape).
pub const FS_ANIM_STEP: f64 = 0.22;
/// A channel within this many screen px of its target counts as settled.
pub const FS_ANIM_EPSILON: f64 = 0.5;
/// Hard cap on animation lifetime (~3s at 16ms) so a client that never
/// commits its new size can't leave the window stuck mid-stretch.
pub const FS_ANIM_MAX_TICKS: u32 = 180;

/// State of an in-flight fullscreen-toggle animation, in screen px.
#[derive(Clone, Copy)]
pub struct FsAnim {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// The target only becomes real once the next arrange/configure lands;
    /// until the target has moved off the start rect the animation must not
    /// declare itself settled (start == target on the first ticks).
    pub moved: bool,
    pub ticks: u32,
}

/// Length of a corner zone, measured from the outer corner along each band.
/// Shared by the visual segments (draw_borders) and the pointer zones
/// (cursor.rs get_border_zone) so they always agree. `configured` comes from
/// `border { corner_length= }`; 0 picks the auto formula. Never shorter than
/// the band width, so a corner is at least its diagonal square. `r_out` is
/// the corner ring's OUTER arc radius (the window silhouette radius plus the
/// band; 0 for square windows): the zone must reach past the arc plus half a
/// band of straight arm, or the widened window corners (~35 logical px)
/// overflow the corner piece and the arc gets truncated mid-sweep.
pub fn border_corner_len(bw: f64, configured: i32, r_out: f64) -> f64 {
    let cl = if configured > 0 {
        // Configured lengths predate the band doubling — scale them the same
        // way, and keep at least half a band of straight arm (arm = cl − bw)
        // so a corner can never collapse to a bare square. (corner_length=16
        // with the doubled 16px band used to yield arm = 0: corners vanished.)
        (configured as f64 * 2.0).max(1.5 * bw)
    } else {
        // 3× band: the corner arms reach well down each edge (they also
        // carry the rounded-corner arc, which eats into the straight run).
        (3.0 * bw).max(24.0)
    };
    cl.max(r_out + 0.5 * bw)
}

/// Floor on the border grab/reveal band, in unscaled layout pixels. Borders
/// rest invisible until hovered, so the band is the only thing to aim at; a
/// narrow target would be unusable.
pub const HOVER_BAND_MIN: f64 = 16.0;

/// Effective interactive border band width (unscaled): the configured border
/// width DOUBLED — the hover/grab band runs twice the classic border — with
/// the HOVER_BAND_MIN floor. Shared by the visual segments (draw_borders),
/// the hit catchers, and the pointer zones (cursor::get_border_zone) so they
/// can never drift apart.
pub fn border_band_width(configured_width: u32) -> f64 {
    (configured_width as f64 * 2.0).max(HOVER_BAND_MIN)
}

// Indices into BorderRects.segments: 4 edge bars + 2 L-arm rects per corner.
const SEG_TOP: usize = 0;
const SEG_BOTTOM: usize = 1;
const SEG_LEFT: usize = 2;
const SEG_RIGHT: usize = 3;
const SEG_TL_H: usize = 4;
const SEG_TL_V: usize = 5;
const SEG_TR_H: usize = 6;
const SEG_TR_V: usize = 7;
const SEG_BL_H: usize = 8;
const SEG_BL_V: usize = 9;
const SEG_BR_H: usize = 10;
const SEG_BR_V: usize = 11;

pub struct BorderRects {
    /// Invisible full-band rects kept as scene hit-test catchers, so the
    /// pointer never falls through the visual gaps between segments.
    pub left: *mut ffi::wlr_scene_rect,
    pub right: *mut ffi::wlr_scene_rect,
    pub top: *mut ffi::wlr_scene_rect,
    pub bottom: *mut ffi::wlr_scene_rect,
    /// The visible zone segments, indexed by the SEG_* constants. Retired by
    /// the frame node below and kept disabled; see the creation site.
    pub segments: [*mut ffi::wlr_scene_rect; 12],
    /// The resize-handle ring: all eight zones in one shader-drawn node.
    pub frame: *mut ffi::wlr_scene_frame,
    /// Parent of `segments`, living in the global border overlay layer rather
    /// than in the window tree. Tracks the window tree's position so the
    /// segments keep their window-local coordinates.
    pub tree: *mut ffi::wlr_scene_tree,
}

pub struct ShowWindowMenuRequest {
    pub x: i32,
    pub y: i32,
}

pub struct PointerResizeRequest {
    pub seat: *mut crate::seat::Seat,
    pub edges: u32,
}

pub struct WmScheduledState {
    pub dimensions_hint: DimensionsHint,
    pub decoration_hint: ffi::zcce_window_v1_decoration_hint,
    pub show_window_menu_requested: Option<ShowWindowMenuRequest>,
    pub fullscreen_requested: FullscreenRequest,
    pub maximize_requested: MaximizeRequest,
    pub minimize_requested: bool,
    pub dirty_app_id: bool,
    pub dirty_title: bool,
    pub pointer_move_requested: *mut crate::seat::Seat,
    pub pointer_resize_requested: Option<PointerResizeRequest>,
}

pub struct WmSentState {
    pub dimensions_hint: DimensionsHint,
    pub decoration_hint: ffi::zcce_window_v1_decoration_hint,
    pub parent: Option<crate::slotmap::Key>,
}

pub struct WmRequestedState {
    pub dimensions: Option<Dimensions>,
    pub bounds: Dimensions,
    pub ssd: bool,
    pub tiled: u32,
    pub capabilities: u32,
    pub resizing: bool,
    pub maximized: bool,
    pub fullscreen: *mut crate::output::Output,
    pub inform_fullscreen: bool,
    pub close: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configure {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bounds: Dimensions,
    pub activated: bool,
    pub ssd: bool,
    pub tiled: u32,
    pub capabilities: u32,
    pub maximized: bool,
    pub inform_fullscreen: bool,
    pub resizing: bool,
}

impl Configure {
    pub fn new() -> Self {
        Self {
            width: None,
            height: None,
            bounds: Dimensions { width: 0, height: 0 },
            activated: false,
            ssd: false,
            tiled: 0,
            capabilities: 0,
            maximized: false,
            inform_fullscreen: false,
            resizing: false,
        }
    }
}

pub struct WindowRenderingScheduled {
    pub width: u32,
    pub height: u32,
    pub resend_dimensions: bool,
}

pub struct WindowRenderingSent {
    pub width: u32,
    pub height: u32,
    pub presentation_hint: ffi::zcce_output_v1_presentation_mode,
}

pub struct WindowRenderingRequested {
    pub x: i32,
    pub y: i32,
    pub hidden: bool,
    pub border: Border,
    pub clip: ffi::wlr_box,
    pub content_clip: ffi::wlr_box,
    pub opacity: f32,
    pub circular: bool,
    pub blur: bool,
}

pub struct Window {
    pub ref_key: crate::slotmap::Key,
    pub server: *mut Server,
    pub object: *mut ffi::wl_resource, // zcce_window_v1
    pub node: WmNode,
    pub state: WindowState,
    pub impl_type: WindowImpl,

    pub tree: *mut ffi::wlr_scene_tree,
    pub fullscreen_background: *mut ffi::wlr_scene_rect,
    pub window_background: *mut ffi::wlr_scene_rect,
    /// scenefx drop shadow, first child of `tree` so it renders beneath
    /// everything else in the window; null if creation failed (shadow skipped).
    pub shadow: *mut ffi::wlr_scene_shadow,
    /// scenefx bevel node: the lit chamfer around the inside of the window's
    /// edge. Created LAST in the window tree so it draws over the surface —
    /// the rim overlays the client's outermost pixels. Null if creation
    /// failed (the effect is then simply absent).
    pub bevel: *mut ffi::wlr_scene_bevel,
    /// scenefx droplet node: for droplet-styled status segments, the
    /// backdrop refracted through the drop's lens. Created BEFORE the
    /// surfaces so it draws beneath the client's translucent drop. Null if
    /// creation failed (the effect is then simply absent).
    pub droplet: *mut ffi::wlr_scene_droplet,
    pub decorations_below: ffi::wl_list,
    pub decorations_below_tree: *mut ffi::wlr_scene_tree,
    pub surfaces: crate::scene::SaveableSurfaces,
    pub border: BorderRects,
    /// The border zone the pointer is over (set by cursor.rs); that segment
    /// draws in `hover_color` while set.
    pub hovered_border_element: Option<BorderElement>,
    /// The zone the ring was last DRAWN with, so `step_border_fade` can tell
    /// a hover change from a settled ring. In overview every zone already
    /// sits at full reveal, so a hover swap moves no reveal value at all —
    /// and a step keyed on reveal alone never repainted, leaving the shader
    /// on whatever zone the last unrelated commit happened to push. The
    /// highlight lagged one hover behind: "the wrong handle lights up".
    pub border_hover_drawn: Option<BorderElement>,
    /// Per-zone reveal factor, 0.0 (fully hidden) to 1.0 (fully drawn),
    /// indexed by `BorderElement::index`. Borders rest invisible and only the
    /// zone under the pointer fades in. Deliberately NOT part of
    /// `rendering_requested.border`, which the arrange pass rewrites wholesale
    /// every pass and would otherwise clobber.
    pub border_reveal: [f32; 8],
    pub decorations_above: ffi::wl_list,
    pub decorations_above_tree: *mut ffi::wlr_scene_tree,
    pub popup_tree: *mut ffi::wlr_scene_tree,
    pub capture_scene: *mut ffi::wlr_scene,
    pub capture_source: *mut ffi::wlr_ext_image_capture_source_v1,
    pub tiling_mode: crate::tiling::TilingMode,
    pub mode_locked: bool,
    pub is_new: bool,
    pub restored: bool,
    /// Position was decided at map time rather than by history: a one-shot
    /// `place-next` hint (widget-spawned picker opening at its control) or a
    /// view-centered session modal. Either way it suppresses the spawn
    /// viewport pan — the window is already where the user is looking.
    pub hint_placed: bool,
    /// A view-centering that ran before the window's real size was known and
    /// must be redone once it lands. Only self-sizing modals set it: their
    /// geometry arrives on a commit, well after `map()`, so the centering at
    /// map sees `mapped_size_hint`'s fallback and misses by half the
    /// difference between that and the truth.
    pub pending_view_center: bool,
    /// True only when the restored geometry came out of the startup restore queue
    /// (`state.json`'s window list). A window reopened later in the session matches
    /// `last_window_states` instead and leaves this false, so it still counts as a
    /// fresh spawn for `center_on_spawn`.
    pub session_restored: bool,
    pub restored_focused: bool,
    pub closed: bool,
    /// Set when the compositor asks this window to close, so `unmap` can tell
    /// a departure someone requested from a client that simply vanished.
    pub close_requested: bool,
    pub has_parent: bool,
    pub minimized: bool,
    /// While Some, the window is mid fullscreen-toggle: its on-screen rect is
    /// this box, eased toward the arranged geometry by `step_fs_anim` on the
    /// border-fade tick. `render_finish` draws at this rect (position, buffer
    /// stretch, backdrop, clip) instead of the settled geometry.
    pub fs_anim: Option<FsAnim>,
    /// Mode and lock this window had when a `SetWindowMode` made it
    /// Fullscreen; the policy's fullscreen toggle restores both on exit.
    /// Cleared by any `SetWindowMode` to another mode.
    pub pre_fullscreen: Option<(crate::tiling::TilingMode, bool)>,
    pub circular: bool,
    pub blur: bool,
    pub scale: f64,
    pub last_applied_scale: f64,
    pub virtual_x: f64,
    pub virtual_y: f64,
    pub resize_start_vx: f64,
    pub resize_start_vy: f64,
    pub resize_start_w: u32,
    pub resize_start_h: u32,
    pub resize_edges: Option<Edges>,
    /// Client hint: an in-surface popover (menu/dropdown) covers this rect,
    /// surface-local logical px (zcce set_popover_region). The overview
    /// resize ring is clipped away beneath it and its band does not grab
    /// there — the menu reads as in front of the chrome.
    pub popover_region: Option<ffi::wlr_box>,
    /// The client resized itself and the new-size buffer is already on screen, so
    /// `render_finish` must take the size from the live commit rather than the
    /// render-start snapshot (`rendering_sent`), which still holds the previous
    /// size and would snap the border back. Cleared once consumed.
    pub self_resized: bool,
    /// Status segments: the along-bar length last seen while the segment was
    /// at bar thickness. Feeds WindowSnapshot::status_collapsed_len so an
    /// EXPANDED segment (surface grown into an in-surface menu) keeps its
    /// frozen slot in the arrange pass.
    pub status_collapsed_len: i32,
    /// Set by the commit listener, cleared by the window-manager stream
    /// timer after a capture: the damage gate for `stream_server` frames.
    /// Starts true so a fresh subscriber gets an immediate first frame.
    pub stream_dirty: bool,
    pub commit: ffi::wl_listener,
    pub was_fullscreen: bool,
    pub saved_width: i32,
    pub saved_height: i32,
    pub saved_virtual_x: f64,
    pub saved_virtual_y: f64,
    pub was_tiled: bool,
    /// Declared the desktop-grid layer via zcce_toplevel_v1.set_grid (the
    /// app_id "cce-grid" convention also maps the role; the flag makes the
    /// declaration explicit and app_id-independent).
    pub grid_declared: bool,
    /// Grid windows: patch sent to the client, awaiting ack_grid_patch.
    pub grid_patch_pending: Option<(u32, crate::policy::api::GridPatch)>,
    /// Acked patch awaiting the client's next commit (the rendered buffer).
    pub grid_patch_acked: Option<(u32, crate::policy::api::GridPatch)>,
    /// The patch the CURRENT buffer covers — what arrange anchors to.
    pub grid_patch_current: Option<crate::policy::api::GridPatch>,
    pub grid_patch_serial: u32,
    /// The current patch was rendered under a style config that has since
    /// changed (reload, or a `layout` change to the desktop keys): re-issue
    /// it on the next arrange even though its coverage is still fine. See
    /// `WindowManager::invalidate_grid_patches`.
    pub grid_patch_stale: bool,
    pub saved_floating_width: i32,
    pub saved_floating_height: i32,
    pub saved_floating_virtual_x: f64,
    pub saved_floating_virtual_y: f64,

    pub wm_scheduled: WmScheduledState,
    pub wm_sent: WmSentState,
    pub wm_requested: WmRequestedState,
    pub configure_scheduled: Configure,
    pub configure_sent: Configure,
    pub rendering_scheduled: WindowRenderingScheduled,
    pub rendering_sent: WindowRenderingSent,
    pub rendering_requested: WindowRenderingRequested,
    pub box_geom: ffi::wlr_box,
    pub margin_x: i32,
    pub margin_y: i32,
    pub last_decor_w: i32,
    pub last_decor_h: i32,
    pub foreign_toplevel_handle: *mut ffi::wlr_ext_foreign_toplevel_handle_v1,
    pub wlr_toplevel_handle: *mut ffi::wlr_foreign_toplevel_handle_v1,
    pub csd_buffer_size_bug: bool,
    pub status_edge: StatusEdge,
}

pub use crate::policy::arrange::StatusEdge;

impl Window {
    pub unsafe fn is_wine(&self) -> bool {
        false
    }

    /// The `surface { shadow tiled=false }` switch: a Tiled window (which is
    /// also what Maximized resolves to) casts no drop shadow when it is off.
    /// Floating, popup and every other mode are unaffected. Evaluated on both
    /// render paths, so a float/tile toggle restyles on the next arrange.
    pub unsafe fn wants_tiled_shadow(&self) -> bool {
        (*self.server).wm.layout.shadow_tiled
            || self.tiling_mode != crate::tiling::TilingMode::Tiled
    }

    pub unsafe fn is_fullscreen(&self) -> bool {
        self.tiling_mode == crate::tiling::TilingMode::Fullscreen
            || !self.wm_requested.fullscreen.is_null()
    }

    pub unsafe fn role(&self) -> crate::policy::api::WindowRole {
        if self.grid_declared {
            return crate::policy::api::WindowRole::Grid;
        }
        crate::policy::api::WindowRole::from_app_id(self.get_app_id_string().as_deref())
    }

    pub unsafe fn is_grid(&self) -> bool {
        self.role() == crate::policy::api::WindowRole::Grid
    }

    pub unsafe fn is_status_bar(&self) -> bool {
        self.role() == crate::policy::api::WindowRole::StatusBar
    }

    pub unsafe fn is_wallpaper(&self) -> bool {
        self.role() == crate::policy::api::WindowRole::Background
    }

    pub unsafe fn is_linked(&self) -> bool {
        let prev = self.node.link.prev;
        let next = self.node.link.next;
        if prev.is_null() || next.is_null() {
            return false;
        }
        let self_ptr = &self.node.link as *const ffi::wl_list as *mut ffi::wl_list;
        prev != self_ptr
    }


    pub unsafe fn create(impl_type: WindowImpl, server: *mut Server) -> Result<*mut Self, &'static str> {
        let hidden_tree = (*server).scene.hidden_tree;
        let tree = ffi::wlr_scene_tree_create(hidden_tree);
        if tree.is_null() {
            return Err("Failed to create tree");
        }

        let popup_tree = ffi::wlr_scene_tree_create(hidden_tree);
        if popup_tree.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create popup_tree");
        }

        let capture_scene = ffi::wlr_scene_create();
        if capture_scene.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create capture_scene");
        }
        // SceneFX 0.4 does not support restack_xwayland_surfaces
        // (*capture_scene).restack_xwayland_surfaces = false;

        // Created first so it is the bottom-most child: the cast shadow must render
        // beneath the (translucent) window content and its backgrounds. Geometry and
        // color are synced per-frame in update_shadow; a null pointer just disables
        // the effect rather than failing window creation.
        let shadow_color = [0.0f32, 0.0f32, 0.0f32, 0.55f32];
        let shadow = ffi::wlr_scene_shadow_create(tree, 0, 0, 0, 22.0, shadow_color.as_ptr());
        if !shadow.is_null() {
            ffi::wlr_scene_node_set_enabled(&mut (*shadow).node, false);
        }

        // Beneath the surfaces like the shadow: the refracted backdrop must
        // render under the client's translucent drop, not over it. Synced in
        // update_droplet; enabled only for droplet-styled status segments.
        let droplet = ffi::wlr_scene_droplet_create(tree, 0, 0);
        if !droplet.is_null() {
            ffi::wlr_scene_node_set_enabled(&mut (*droplet).node, false);
        }

        let black_color = [0.0f32, 0.0f32, 0.0f32, 1.0f32];
        let fullscreen_background = ffi::wlr_scene_rect_create(tree, 0, 0, black_color.as_ptr());
        if fullscreen_background.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(&mut (*capture_scene).tree as *mut ffi::wlr_scene_tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create fullscreen rect");
        }

        let decorations_below_tree = ffi::wlr_scene_tree_create(tree);

        let clear_color = [0.0f32, 0.0f32, 0.0f32, 0.0f32];
        let window_background = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());
        if window_background.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(&mut (*capture_scene).tree as *mut ffi::wlr_scene_tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create window background rect");
        }

        let surfaces = match crate::scene::SaveableSurfaces::init(tree) {
            Ok(s) => s,
            Err(e) => {
                ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
                ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
                ffi::wlr_scene_node_destroy(&mut (*capture_scene).tree as *mut ffi::wlr_scene_tree as *mut ffi::wlr_scene_node);
                return Err(e);
            }
        };

        // Created after the surfaces so it is ABOVE them in the window tree:
        // the bevel is an inner rim drawn over the client's outermost pixels,
        // not something tucked behind them. Geometry, light and colour are
        // synced per frame in update_bevel; a null pointer disables the
        // effect rather than failing window creation.
        let bevel_color = [1.0f32, 1.0f32, 1.0f32, 1.0f32];
        let bevel = ffi::wlr_scene_bevel_create(tree, 0, 0, 0, 0.0, bevel_color.as_ptr());
        if !bevel.is_null() {
            ffi::wlr_scene_node_set_enabled(&mut (*bevel).node, false);
        }

        // The invisible hit catchers stay in the window tree so pointer
        // hit-testing and z-order are unchanged. The visible segments live in
        // a sibling tree parented to the global border overlay layer, so a
        // revealed edge draws over the neighbouring window it overhangs.
        let border_left = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());
        let border_right = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());
        let border_top = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());
        let border_bottom = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());

        let border_tree = ffi::wlr_scene_tree_create((*server).scene.layers.border_overlay);
        if border_tree.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(&mut (*capture_scene).tree as *mut ffi::wlr_scene_tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create window border tree");
        }
        let mut border_segments = [std::ptr::null_mut(); 12];
        for seg in border_segments.iter_mut() {
            *seg = ffi::wlr_scene_rect_create(border_tree, 0, 0, clear_color.as_ptr());
        }
        // The resize-handle ring. One node draws all eight zones, because the
        // thickness swells continuously along each side and rects cannot
        // (see scenefx frame.frag). The 12 segment rects above are what it
        // replaced; they stay allocated but disabled — the status-bar code
        // still reaches for the array, and freeing them would be a wider
        // change than this.
        let border_frame = ffi::wlr_scene_frame_create(border_tree, 0, 0, 0, clear_color.as_ptr());

        let decorations_above_tree = ffi::wlr_scene_tree_create(tree);

        let mut window = Box::new(Window {
            ref_key: crate::slotmap::Key { generation: 0, index: 0 },
            server,
            object: std::ptr::null_mut(),
            node: std::mem::zeroed(),
            state: WindowState::Init,
            impl_type,
            tree,
            fullscreen_background,
            window_background,
            shadow,
            bevel,
            droplet,
            decorations_below: std::mem::zeroed(),
            decorations_below_tree,
            surfaces,
            border: BorderRects {
                left: border_left,
                right: border_right,
                top: border_top,
                bottom: border_bottom,
                segments: border_segments,
                frame: border_frame,
                tree: border_tree,
            },
            hovered_border_element: None,
            border_hover_drawn: None,
            border_reveal: [0.0; 8],
            decorations_above: std::mem::zeroed(),
            decorations_above_tree,
            popup_tree,
            capture_scene,
            capture_source: std::ptr::null_mut(),
            tiling_mode: crate::tiling::TilingMode::Floating,
            mode_locked: false,
            is_new: true,
            restored: false,
            hint_placed: false,
            pending_view_center: false,
            session_restored: false,
            restored_focused: false,
            closed: false,
            close_requested: false,
            has_parent: false,
            minimized: false,
            fs_anim: None,
            pre_fullscreen: None,
            circular: false,
            blur: false,
            scale: 1.0,
            last_applied_scale: 1.0,
            virtual_x: unsafe { (*server).wm.desk_pan_x + 100.0 },
            virtual_y: unsafe { (*server).wm.desk_pan_y + 100.0 },
            resize_start_vx: 0.0,
            resize_start_vy: 0.0,
            resize_start_w: 0,
            resize_start_h: 0,
            resize_edges: None,
            popover_region: None,
            self_resized: false,
            status_collapsed_len: 0,
            stream_dirty: true,
            commit: std::mem::zeroed(),
            was_fullscreen: false,
            saved_width: 0,
            saved_height: 0,
            saved_virtual_x: 0.0,
            saved_virtual_y: 0.0,
            was_tiled: false,
            grid_declared: false,
            grid_patch_pending: None,
            grid_patch_acked: None,
            grid_patch_current: None,
            grid_patch_serial: 0,
            grid_patch_stale: false,
            saved_floating_width: 0,
            saved_floating_height: 0,
            saved_floating_virtual_x: 0.0,
            saved_floating_virtual_y: 0.0,
            wm_scheduled: WmScheduledState {
                dimensions_hint: DimensionsHint { min_width: 0, min_height: 0, max_width: 0, max_height: 0 },
                decoration_hint: ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_ONLY_SUPPORTS_CSD,
                show_window_menu_requested: None,
                fullscreen_requested: FullscreenRequest::NoRequest,
                maximize_requested: MaximizeRequest::NoRequest,
                minimize_requested: false,
                dirty_app_id: false,
                dirty_title: false,
                pointer_move_requested: std::ptr::null_mut(),
                pointer_resize_requested: None,
            },
            wm_sent: WmSentState {
                dimensions_hint: DimensionsHint { min_width: 0, min_height: 0, max_width: 0, max_height: 0 },
                decoration_hint: ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_ONLY_SUPPORTS_CSD,
                parent: None,
            },
            wm_requested: WmRequestedState {
                dimensions: None,
                bounds: Dimensions { width: 0, height: 0 },
                ssd: false,
                tiled: 0,
                capabilities: 1 | 2 | 4 | 8,
                resizing: false,
                maximized: false,
                fullscreen: std::ptr::null_mut(),
                inform_fullscreen: false,
                close: false,
            },
            configure_scheduled: Configure::new(),
            configure_sent: Configure::new(),
            rendering_scheduled: WindowRenderingScheduled {
                width: 0,
                height: 0,
                resend_dimensions: false,
            },
            rendering_sent: WindowRenderingSent {
                width: 0,
                height: 0,
                presentation_hint: ffi::zcce_output_v1_presentation_mode_ZCCE_OUTPUT_V1_PRESENTATION_MODE_VSYNC,
            },
            rendering_requested: WindowRenderingRequested {
                x: 0,
                y: 0,
                hidden: false,
                border: Border::none(),
                clip: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
                content_clip: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
                opacity: 1.0f32,
                circular: false,
                blur: false,
            },
            box_geom: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
            margin_x: 0,
            margin_y: 0,
            last_decor_w: 0,
            last_decor_h: 0,
            foreign_toplevel_handle: std::ptr::null_mut(),
            wlr_toplevel_handle: std::ptr::null_mut(),
            csd_buffer_size_bug: false,
            status_edge: StatusEdge::Unspecified,
        });

        ffi::wl_list_init(&mut window.decorations_below);
        ffi::wl_list_init(&mut window.decorations_above);

        let raw = Box::into_raw(window);
        let key = (*(*raw).server).wm.windows.put(raw);
        (*raw).ref_key = key;
        (*raw).node.init(crate::wm_node::WmNodeTag::Window);

        ffi::wlr_scene_node_set_enabled(tree as *mut ffi::wlr_scene_node, false);
        ffi::wlr_scene_node_set_enabled(popup_tree as *mut ffi::wlr_scene_node, false);
        ffi::wlr_scene_node_set_enabled(fullscreen_background as *mut ffi::wlr_scene_node, false);

        crate::scene_node_data::SceneNodeData::attach(
            tree as *mut ffi::wlr_scene_node,
            crate::scene_node_data::SceneNodeDataVal::Window(raw),
        );
        crate::scene_node_data::SceneNodeData::attach(
            popup_tree as *mut ffi::wlr_scene_node,
            crate::scene_node_data::SceneNodeDataVal::Window(raw),
        );
        // The border segments sit outside the window tree; without data of
        // their own a hit on a revealed segment would resolve to no window at
        // all, so tag them with the window they belong to.
        crate::scene_node_data::SceneNodeData::attach(
            border_tree as *mut ffi::wlr_scene_node,
            crate::scene_node_data::SceneNodeDataVal::Window(raw),
        );
        ffi::wlr_scene_node_set_enabled(border_tree as *mut ffi::wlr_scene_node, false);

        Ok(raw)
    }

    pub unsafe fn set_impl(&mut self, impl_type: WindowImpl) {
        self.impl_type = impl_type;
    }

    pub unsafe fn impl_destroying(&mut self) {
        self.impl_type = WindowImpl::Destroying;
    }

    pub unsafe fn get_title(&self) -> *const libc::c_char {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    std::ptr::null()
                } else {
                    ffi::river_wlr_xdg_toplevel_get_title((*toplevel).wlr_toplevel)
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    std::ptr::null()
                } else {
                    (*(*xwindow).xsurface).title
                }
            }
            WindowImpl::Destroying => std::ptr::null(),
        }
    }

    pub unsafe fn get_app_id(&self) -> *const libc::c_char {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    std::ptr::null()
                } else {
                    ffi::river_wlr_xdg_toplevel_get_app_id((*toplevel).wlr_toplevel)
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    std::ptr::null()
                } else {
                    (*(*xwindow).xsurface).class
                }
            }
            WindowImpl::Destroying => std::ptr::null(),
        }
    }

    pub unsafe fn get_app_id_string(&self) -> Option<String> {
        let ptr = self.get_app_id();
        if ptr.is_null() {
            None
        } else {
            Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned())
        }
    }

    pub unsafe fn get_title_string(&self) -> Option<String> {
        let ptr = self.get_title();
        if ptr.is_null() {
            None
        } else {
            Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned())
        }
    }

    /// Dest-size factor for this window's surface buffers on top of the
    /// overview zoom: 1/output-scale for an X11 window under
    /// `xwayland_hidpi`, whose buffer is physical pixels (see
    /// `xwayland_window::x11_scale`); 1 for everything else.
    pub unsafe fn x11_buffer_scale(&self) -> f64 {
        if matches!(self.impl_type, WindowImpl::Xwayland(_)) {
            1.0 / crate::xwayland_window::x11_scale(self.server) as f64
        } else {
            1.0
        }
    }

    pub unsafe fn get_parent(&self) -> *mut Window {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    std::ptr::null_mut()
                } else {
                    let wlr_parent = ffi::river_wlr_xdg_toplevel_get_parent((*toplevel).wlr_toplevel);
                    if wlr_parent.is_null() {
                        std::ptr::null_mut()
                    } else {
                        let base = ffi::river_wlr_xdg_toplevel_get_base(wlr_parent);
                        let parent_xdg = ffi::river_wlr_xdg_surface_get_data(base) as *mut crate::xdg_toplevel::XdgToplevel;
                        if parent_xdg.is_null() {
                            std::ptr::null_mut()
                        } else {
                            (*parent_xdg).window
                        }
                    }
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    std::ptr::null_mut()
                } else {
                    let parent_xsurface = (*(*xwindow).xsurface).parent;
                    if parent_xsurface.is_null() {
                        std::ptr::null_mut()
                    } else {
                        let parent_data = (*parent_xsurface).data;
                        if parent_data.is_null() {
                            std::ptr::null_mut()
                        } else {
                            let parent_xwindow = parent_data as *mut crate::xwayland_window::XwaylandWindow;
                            (*parent_xwindow).window
                        }
                    }
                }
            }
            WindowImpl::Destroying => std::ptr::null_mut(),
        }
    }

    pub unsafe fn unreliable_pid(&self) -> i32 {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    0
                } else {
                    let base = ffi::river_wlr_xdg_toplevel_get_base((*toplevel).wlr_toplevel);
                    let surface = ffi::river_wlr_xdg_surface_get_surface(base);
                    if surface.is_null() {
                        0
                    } else {
                        let res = ffi::river_wlr_surface_get_resource(surface);
                        if res.is_null() {
                            0
                        } else {
                            let client = ffi::wl_resource_get_client(res);
                            if client.is_null() {
                                0
                            } else {
                                let mut pid = 0;
                                let mut uid = 0;
                                let mut gid = 0;
                                ffi::wl_client_get_credentials(client, &mut pid, &mut uid, &mut gid);
                                pid
                            }
                        }
                    }
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    0
                } else {
                    (*(*xwindow).xsurface).pid
                }
            }
            WindowImpl::Destroying => 0,
        }
    }

    /// Overlay-mode UI (cce-cloud menus and the like): takes keyboard input
    /// while open, but is invisible to the window manager's notion of "the
    /// focused window" — persistence, camera follow, arrange focus styling
    /// and refocus rules all look through it to the real window underneath.
    pub unsafe fn is_overlay_ui(&self) -> bool {
        self.tiling_mode == crate::tiling::TilingMode::Overlay
            || self.get_app_id_string().as_deref() == Some("cce-cloud")
    }

    pub unsafe fn try_restore(&mut self) {
        if self.restored {
            return;
        }
        // No geometry is ever saved for a Utility window, so none may be
        // restored over it — a pre-Utility state.json entry for the same
        // app_id would otherwise dictate a stale size to a self-sizing
        // client. (Belt over suspenders: the arrange pass restates the
        // "you choose" 0x0 for Utility anyway, so even a slipped-through
        // restore heals on the client's next commit.)
        if self.tiling_mode == crate::tiling::TilingMode::Utility {
            return;
        }
        // A transient — an xdg toplevel with a parent, or an X11 window with
        // WM_TRANSIENT_FOR — is a dialog of the window it hangs off, and is
        // never what a saved entry describes. It shares its app_id with the
        // main window, so the app_id-only third pass of the state matchers
        // (kept for a relaunched main window whose title has changed) would
        // hand it the MAIN window's geometry: Houdini's Preferences opened at
        // the full 1856x1141 of the session it belongs to, and hkey's
        // "Redeem Result" at the administrator's size. The save pass skips
        // transients for the same reason, so there is nothing of their own to
        // restore either; they size themselves.
        if !self.get_parent().is_null() {
            return;
        }
        // For an X11 window that check is only meaningful once its properties
        // are all in: they arrive one PropertyNotify at a time, and WM_CLASS
        // (the app_id) lands before WM_TRANSIENT_FOR, so on the app_id notify
        // a dialog still looks parentless and the app_id-only match below
        // restored it anyway — first match wins, and the restore overwrites
        // the client's own requested size, so it cannot be undone when the
        // parent turns up. (Waiting for the wl_surface was not enough: GTK's
        // dialog was still title-less and parentless at association.) Wait
        // for `map`, which calls back in here; by then every property the
        // client set before mapping has been read.
        if matches!(self.impl_type, WindowImpl::Xwayland(_)) && self.state != WindowState::Mapped {
            return;
        }
        let app_id_str = self.get_app_id_string().unwrap_or_default();
        if app_id_str.is_empty()
            || app_id_str.starts_with("cce-status")
            || app_id_str == "cce-wallpaper"
            || app_id_str == "cce-grid"
        {
            return;
        }
        let title_str = self.get_title_string().unwrap_or_default();
        let mut saved_opt = (*self.server).wm.match_and_remove_restore_state(&app_id_str, &title_str);
        let from_session = saved_opt.is_some();
        if saved_opt.is_none() {
            saved_opt = (*self.server).wm.match_last_window_state(&app_id_str, &title_str);
        }
        if let Some(saved) = saved_opt {
            log::info!("Restoring saved state for window: app_id={}, title={}. Position: ({}, {}), Size: {}x{}", app_id_str, title_str, saved.virtual_x, saved.virtual_y, saved.width, saved.height);
            self.tiling_mode = saved.tiling_mode;
            self.minimized = saved.minimized;
            self.virtual_x = saved.virtual_x;
            self.virtual_y = saved.virtual_y;
            self.scale = saved.scale;
            self.box_geom.width = saved.width as i32;
            self.box_geom.height = saved.height as i32;
            
            self.wm_requested.dimensions = Some(crate::window::Dimensions {
                width: saved.width,
                height: saved.height,
            });
            self.wm_requested.bounds = crate::window::Dimensions {
                width: saved.width,
                height: saved.height,
            };
            
            self.rendering_scheduled.width = saved.width;
            self.rendering_scheduled.height = saved.height;
            self.rendering_sent.width = saved.width;
            self.rendering_sent.height = saved.height;

            match self.impl_type {
                WindowImpl::Toplevel(toplevel) => {
                    if !toplevel.is_null() {
                        (*toplevel).geometry.width = saved.width as i32;
                        (*toplevel).geometry.height = saved.height as i32;
                    }
                }
                WindowImpl::Xwayland(xwindow) => {
                    if !xwindow.is_null() && !(*xwindow).xsurface.is_null() {
                        let s = crate::xwayland_window::x11_scale(self.server);
                        (*(*xwindow).xsurface).width = crate::xwayland_window::to_x11(saved.width as i32, s) as u16;
                        (*(*xwindow).xsurface).height = crate::xwayland_window::to_x11(saved.height as i32, s) as u16;
                    }
                }
                _ => {}
            }

            // A restored non-Floating mode is EXPLICIT state, and has to be
            // latched to survive. `get_mode_for_window` returns the window's own
            // mode only when `mode_locked`; unlocked, it resolves from the config
            // rules and falls through to Floating — and the arrange pass writes
            // that resolution straight back into `tiling_mode`
            // (`window_manager.rs`, the `wp.tiling_mode` apply). So a window
            // restored Tiled but unlocked was demoted by the very next arrange,
            // which is why a relaunched app came back floating however exactly
            // its geometry had been restored: position, size and cell were all
            // right, and the mode was gone before the first frame.
            //
            // Both sibling promotions already pair the mode with the lock — the
            // seat's op_end detection, and the geometric one just below, which is
            // why a window saved Floating-but-aligned survived while one saved
            // Tiled did not. Only Floating is left unlatched here, so a window
            // with no explicit mode still resolves from the rules as before.
            if saved.tiling_mode != crate::tiling::TilingMode::Floating {
                self.mode_locked = true;
            }

            // Geometric promotion at restore time: a window whose saved
            // geometry sits cell-aligned IS tiled, even if an older session
            // saved it as Floating (pre-rework state, or a session that
            // never touched it after it landed on the grid). Same test and
            // lock as the op_end detection. No demotion here — a saved
            // Tiled window off the current grid is re-snapped by the Tiled
            // arrange arm instead.
            if self.tiling_mode == crate::tiling::TilingMode::Floating {
                let sp = (*self.server).wm.layout.snap_params();
                if crate::policy::snap::is_cell_aligned(
                    self.virtual_x,
                    self.virtual_y,
                    saved.width as f64,
                    saved.height as f64,
                    &sp,
                    1.0,
                ) {
                    self.tiling_mode = crate::tiling::TilingMode::Tiled;
                    self.mode_locked = true;
                }
            }

            self.restored = true;
            self.session_restored = from_session;
            // The saved `focused` flag only means something for the startup
            // restore queue; on a `last_window_states` borrow it is stale
            // (whether the app happened to be focused when last closed) and
            // must not feed the settle-phase focus gates.
            self.restored_focused = from_session && saved.focused;
        }
    }

    /// Apply a one-shot `place-next` hint: land the window's top-left just
    /// below-right of the hinted layout position (the control that spawned
    /// it), clamped to the output so it stays fully on-screen. Runs after
    /// `try_restore` so the remembered SIZE is kept — only the position is
    /// overridden — and marks `hint_placed` so the spawn viewport pan is
    /// skipped (the window is already under the user's pointer).
    /// Layout box of the first enabled output — `(phys_x, phys_y, width,
    /// height)`, the viewport every placement decision is measured against.
    /// Falls back to a 1920x1080 box at the origin before any output is up.
    unsafe fn first_enabled_output_box(&self) -> (f64, f64, f64, f64) {
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let b = (*output).sent.box_layout();
                return (b.x as f64, b.y as f64, b.width as f64, b.height as f64);
            }
            curr_out = (*curr_out).next;
        }
        (0.0, 0.0, 1920.0, 1080.0)
    }

    /// Virtual position to layout (screen) position, ROUNDED — the same
    /// conversion the arrange pass makes (`PlacementCtx::virtual_to_screen`).
    /// Every writer of a window's screen origin has to agree on the
    /// rounding: the seat op and the resize-commit anchoring truncated while
    /// the arrange pass rounds, so whenever the fractional part was .5 or
    /// more the window stepped a pixel back and forth between a commit and
    /// the next arrange — a twitch on every resize step at overview zoom,
    /// and a one-pixel hop on grab and release.
    pub unsafe fn virtual_to_screen(&self, vx: f64, vy: f64) -> (i32, i32) {
        let wm = &(*self.server).wm;
        let (out_x, out_y, _, _) = self.first_enabled_output_box();
        (
            out_x as i32 + ((vx - wm.desk_pan_x) * wm.desk_zoom).round() as i32,
            out_y as i32 + ((vy - wm.desk_pan_y) * wm.desk_zoom).round() as i32,
        )
    }

    /// Best-known window size in VIRTUAL units at map time. `box_geom` is the
    /// render pass's size and is only filled in once a frame has been drawn
    /// (or by `try_restore` from the saved geometry), so a first-ever launch
    /// falls back to the client's committed toplevel geometry.
    unsafe fn mapped_size_hint(&self) -> (f64, f64) {
        if self.box_geom.width > 0 && self.box_geom.height > 0 {
            return (self.box_geom.width as f64, self.box_geom.height as f64);
        }
        if let WindowImpl::Toplevel(toplevel) = self.impl_type {
            if !toplevel.is_null() {
                let g = (*toplevel).geometry;
                if g.width > 0 && g.height > 0 {
                    return (g.width as f64, g.height as f64);
                }
            }
        }
        (400.0, 400.0)
    }

    unsafe fn try_hint_placement(&mut self) {
        let app_id = self.get_app_id_string().unwrap_or_default();
        if app_id.is_empty() {
            return;
        }
        // Claimed before the mode is judged, so a hint aimed at this window
        // does not linger and land on the next one to open.
        let Some((hx, hy, cell_anchored)) = (*self.server).wm.take_pending_placement(&app_id)
        else {
            return;
        };
        if cell_anchored {
            // TILED IS THE POINT here, unlike the position-only hint below: a
            // window that reopens filling four squares is exactly the case
            // this exists for. Only the modes that do not own a position at
            // all are excluded.
            if matches!(
                self.tiling_mode,
                crate::tiling::TilingMode::Fullscreen
                    | crate::tiling::TilingMode::Popup
                    | crate::tiling::TilingMode::Overlay
                    | crate::tiling::TilingMode::Status
            ) {
                return;
            }
            self.place_on_invocation_cell(&app_id, hx, hy);
            return;
        }
        // Utility included: the hint moves only the POSITION, which a utility
        // window does not own — only its size is the client's.
        if !matches!(
            self.tiling_mode,
            crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Utility
        ) {
            return;
        }

        let (phys_x, phys_y, vp_w, vp_h) = self.first_enabled_output_box();

        let wm = &(*self.server).wm;
        let zoom = wm.desk_zoom.max(0.01);
        let (vw, vh) = self.mapped_size_hint();
        let (w, h) = (vw * zoom, vh * zoom);

        const OFFSET: f64 = 12.0; // context-menu-style drop below-right of the control
        const MARGIN: f64 = 8.0;
        let sx = (hx + OFFSET)
            .min(phys_x + vp_w - w - MARGIN)
            .max(phys_x + MARGIN);
        let sy = (hy + OFFSET)
            .min(phys_y + vp_h - h - MARGIN)
            .max(phys_y + MARGIN);

        // screen = phys + (virtual - desk_pan) * zoom  →  invert for virtual.
        self.virtual_x = wm.desk_pan_x + (sx - phys_x) / zoom;
        self.virtual_y = wm.desk_pan_y + (sy - phys_y) / zoom;
        self.hint_placed = true;
        log::info!(
            "place-next hint applied: app_id={} screen=({:.0},{:.0}) virtual=({:.1},{:.1})",
            app_id, sx, sy, self.virtual_x, self.virtual_y
        );
    }

    /// Step a freshly-spawned TILED window off any tiled window it would open
    /// on top of, keeping its size and staying as close to its intended spot
    /// as possible (`policy::spawn::nearest_free`).
    ///
    /// The remembered-position path has no idea whether that position is still
    /// free — it was when the window closed, and something else may have taken
    /// it since. Two tiled windows stacked on the same squares is never what
    /// was meant: tiled windows are the ones laid out to sit side by side.
    ///
    /// Deliberately narrow:
    /// - Only TILED windows are moved, and only tiled windows count as
    ///   obstacles. Floating windows overlap by nature; that is the difference
    ///   between the two modes, not a fault to correct.
    /// - Session restore is exempt. A restored layout is a layout the user
    ///   arranged and saved, and mapping order is arbitrary, so nudging there
    ///   would rearrange a deliberate desktop at every login.
    unsafe fn avoid_tiled_overlap(&mut self) {
        if self.session_restored || self.tiling_mode != crate::tiling::TilingMode::Tiled {
            return;
        }
        let wm = &(*self.server).wm;
        let sp = wm.layout.snap_params();
        if sp.cell_w <= 0.5 || sp.cell_h <= 0.5 {
            return;
        }
        let (vw, vh) = self.mapped_size_hint();
        if vw <= 0.0 || vh <= 0.0 {
            return;
        }
        let (c0, r0, c1, r1) = crate::policy::cells::window_span(
            self.virtual_x, self.virtual_y, vw, vh, sp.cell_w, sp.cell_h, sp.gap_width,
        );
        let want = crate::policy::spawn::CellBlock::new(c0, r0, c1, r1);

        let mut occupied = Vec::new();
        for &w in wm.windows.iter() {
            if w.is_null() || w == (self as *mut Window) || (*w).closed || (*w).minimized {
                continue;
            }
            if !matches!((*w).state, WindowState::Mapped) {
                continue;
            }
            if (*w).tiling_mode != crate::tiling::TilingMode::Tiled {
                continue;
            }
            let (ow, oh) = ((*w).box_geom.width as f64, (*w).box_geom.height as f64);
            if ow <= 0.0 || oh <= 0.0 {
                continue;
            }
            let (oc0, or0, oc1, or1) = crate::policy::cells::window_span(
                (*w).virtual_x, (*w).virtual_y, ow, oh, sp.cell_w, sp.cell_h, sp.gap_width,
            );
            occupied.push(crate::policy::spawn::CellBlock::new(oc0, or0, oc1, or1));
        }
        if occupied.is_empty() {
            return;
        }

        // Bounded: a window that cannot find room nearby stays put rather than
        // being flung to an empty region of a desktop that has no edges.
        const SEARCH_SQUARES: i32 = 12;
        let free = crate::policy::spawn::nearest_free(want, &occupied, SEARCH_SQUARES);
        if free == want {
            return;
        }
        let (bx, by, _, _) = crate::policy::cells::block_rect(
            free.col0, free.row0, free.col1, free.row1,
            sp.cell_w, sp.cell_h, sp.gap_width, sp.cell_inset,
        );
        log::info!(
            "spawn overlap: {} would open on a tiled window at {} -> moved to {}",
            self.get_app_id_string().unwrap_or_default(),
            crate::policy::cells::span_label(want.col0, want.row0, want.col1, want.row1),
            crate::policy::cells::span_label(free.col0, free.row0, free.col1, free.row1),
        );
        self.virtual_x = bx;
        self.virtual_y = by;
    }

    /// Place this window on the grid square the user invoked it from, keeping
    /// its remembered SIZE and growing away from the windows already there
    /// (`policy::spawn::place_at_cell`).
    ///
    /// The size comes from the remembered geometry `try_restore` just applied,
    /// measured in whole squares: a window last seen filling four squares
    /// opens filling four squares, at the corner of the invocation square that
    /// leaves it clear of its neighbours.
    unsafe fn place_on_invocation_cell(&mut self, app_id: &str, hx: f64, hy: f64) {
        let wm = &(*self.server).wm;
        let sp = wm.layout.snap_params();
        if sp.cell_w <= 0.5 || sp.cell_h <= 0.5 {
            return;
        }
        let (phys_x, phys_y, vp_w, vp_h) = self.first_enabled_output_box();
        let zoom = wm.desk_zoom.max(0.01);
        // The hint is a layout point; the grid is in virtual coordinates.
        let inv_vx = wm.desk_pan_x + (hx - phys_x) / zoom;
        let inv_vy = wm.desk_pan_y + (hy - phys_y) / zoom;
        let col = crate::policy::cells::cell_index(inv_vx, sp.cell_w, sp.gap_width);
        let row = crate::policy::cells::cell_index(inv_vy, sp.cell_h, sp.gap_width);

        // Size in squares, from the geometry `try_restore` left in place.
        let (vw, vh) = self.mapped_size_hint();
        let (c0, r0, c1, r1) = crate::policy::cells::window_span(
            0.0, 0.0, vw, vh, sp.cell_w, sp.cell_h, sp.gap_width,
        );
        let (cols, rows) = (c1 - c0 + 1, r1 - r0 + 1);

        // Everything else already on the desktop, in squares. Chrome and the
        // canvas itself are not obstacles.
        let mut occupied = Vec::new();
        for &w in wm.windows.iter() {
            if w.is_null() || w == (self as *mut Window) || (*w).closed || (*w).minimized {
                continue;
            }
            if !matches!((*w).state, WindowState::Mapped) {
                continue;
            }
            if (*w).is_status_bar() || (*w).is_wallpaper() || (*w).is_grid() {
                continue;
            }
            let (ow, oh) = ((*w).box_geom.width as f64, (*w).box_geom.height as f64);
            if ow <= 0.0 || oh <= 0.0 {
                continue;
            }
            let (oc0, or0, oc1, or1) = crate::policy::cells::window_span(
                (*w).virtual_x, (*w).virtual_y, ow, oh, sp.cell_w, sp.cell_h, sp.gap_width,
            );
            occupied.push(crate::policy::spawn::CellBlock::new(oc0, or0, oc1, or1));
        }

        // Visible squares, so a tie between two clear corners goes to the one
        // on screen.
        let view = {
            let (vx0, vy0) = (wm.desk_pan_x, wm.desk_pan_y);
            let (vx1, vy1) = (vx0 + vp_w / zoom, vy0 + vp_h / zoom);
            let c0 = crate::policy::cells::cell_index(vx0, sp.cell_w, sp.gap_width);
            let r0 = crate::policy::cells::cell_index(vy0, sp.cell_h, sp.gap_width);
            let c1 = crate::policy::cells::cell_index(vx1, sp.cell_w, sp.gap_width);
            let r1 = crate::policy::cells::cell_index(vy1, sp.cell_h, sp.gap_width);
            crate::policy::spawn::CellBlock::new(c0, r0, c1, r1)
        };

        let block = crate::policy::spawn::place_at_cell(col, row, cols, rows, &occupied, Some(view));
        let (bx, by, bw, bh) = crate::policy::cells::block_rect(
            block.col0, block.row0, block.col1, block.row1,
            sp.cell_w, sp.cell_h, sp.gap_width, sp.cell_inset,
        );
        self.virtual_x = bx;
        self.virtual_y = by;
        // A window that was filling whole squares keeps doing so — it is the
        // same window, in the same shape, somewhere else. One that was not
        // keeps its own size and simply starts at the square's corner.
        if self.tiling_mode == crate::tiling::TilingMode::Tiled {
            self.box_geom.width = bw.round() as i32;
            self.box_geom.height = bh.round() as i32;
            self.wm_requested.dimensions = Some(crate::window::Dimensions {
                width: bw.round() as u32,
                height: bh.round() as u32,
            });
        }
        self.hint_placed = true;
        log::info!(
            "place-next-cell: {} -> {} ({}x{} squares) at virtual ({:.0}, {:.0})",
            app_id,
            crate::policy::cells::span_label(block.col0, block.row0, block.col1, block.row1),
            cols, rows, bx, by
        );
    }

    /// Open a session modal in the middle of what the user is looking at,
    /// ignoring wherever it last sat.
    ///
    /// On a panning desktop a remembered position is actively wrong for these
    /// windows: the camera has almost always moved since the last time, so
    /// the window maps somewhere off-view and the prompt reads as never
    /// having appeared — which for the polkit agent means the privileged
    /// action silently times out.
    ///
    /// Runs after `try_restore`, so the remembered SIZE is still available
    /// and only the position is overridden — the same split
    /// `try_hint_placement` uses — and marks `hint_placed` so the spawn
    /// viewport pan is skipped: the window is already centered in view, and
    /// panning the camera to it would move the desktop out from under the
    /// user for a dialog that is about to close again.
    /// Windows that open centered on the current view rather than wherever
    /// they last were: DE session modals whose whole job is to interrupt, and
    /// which the user must be able to answer immediately.
    ///
    /// Hardcoded by app_id like the compositor's other DE-internal window
    /// classes (`cce-status*`/`cce-wallpaper`/`cce-grid` in `try_restore`,
    /// `cce-notifier`/`cce-cloud` in `get_mode_for_window`). The config's
    /// per-app window rules assign a tiling MODE, not a placement, so there
    /// is nothing there to hang this off yet.
    fn is_view_centered_modal(app_id: &str) -> bool {
        // The polkit prompt, and the file chooser cce-files runs in --select/
        // --save mode: both are spawned BY an action in the current view and
        // must be answered immediately — a remembered position is actively
        // wrong for them (the chooser used to map wherever the file manager
        // was last used, squares away from the app that opened it).
        app_id == "cce-authenticator" || app_id == "cce-filesystem-chooser"
    }

    unsafe fn try_center_on_view(&mut self) {
        let app_id = self.get_app_id_string().unwrap_or_default();
        if !Self::is_view_centered_modal(&app_id) {
            return;
        }

        // Whatever history says, a modal has to be visible and free-floating:
        // a restored Tiled mode would re-snap it onto a grid cell (undoing
        // the centering) and a restored `minimized` would hide the prompt
        // outright. `mode_locked` is the "explicit beats heuristic" latch, so
        // the arrange pass cannot geometrically re-promote it either.
        //
        // Utility is exempt from the mode forcing ONLY — like
        // `try_hint_placement`, this owns the window's POSITION, never its
        // size. A Utility window already satisfies everything the forcing is
        // for: it always floats, never tiles, and both the grid snap and the
        // overview displacement skip it. Overwriting the field would silently
        // strip the mode — `set_utility` arrives before map, and every Utility
        // gate reads `tiling_mode` RAW — leaving the modal resizable, its
        // geometry saved, and a stale size restored over it next time.
        if self.tiling_mode != crate::tiling::TilingMode::Utility {
            self.tiling_mode = crate::tiling::TilingMode::Floating;
        }
        self.mode_locked = true;
        self.minimized = false;

        // A self-sizing modal has not committed its geometry yet, so
        // `mapped_size_hint` here is still the 400x400 floor — centering
        // against that misses by half the difference from the real size (a
        // 640x360 prompt landed 120px right and 20px high). Center anyway so
        // the first frame is not wildly off, and latch a redo for the commit
        // that brings the truth.
        //
        // Any mode with unknown geometry latches the redo — not Utility only.
        // The file chooser disproved the old Utility-only reasoning: a
        // FLOATING self-sizer on its first ever run has no restored geometry
        // and no arrange-given size either, so it was centered against the
        // 400x400 floor and stuck there, ~250px off for a 900x500 dialog.
        // A Floating modal with restored geometry still skips the latch
        // (box_geom is already filled by the time we run).
        self.pending_view_center = self.box_geom.width <= 0 || self.box_geom.height <= 0;
        self.apply_view_centering();
    }

    /// The centering itself, split out so the self-sizing commit path can redo
    /// it once the client's real size lands.
    unsafe fn apply_view_centering(&mut self) {
        let (_, _, vp_w, vp_h) = self.first_enabled_output_box();
        let wm = &(*self.server).wm;
        let zoom = wm.desk_zoom.max(0.01);
        let (w, h) = self.mapped_size_hint();

        // Policy owns the camera math; the output's origin cancels out of the
        // centering, so only the extent is needed per axis.
        self.virtual_x = crate::policy::camera::centered_window_origin(wm.desk_pan_x, vp_w, zoom, w);
        self.virtual_y = crate::policy::camera::centered_window_origin(wm.desk_pan_y, vp_h, zoom, h);
        self.hint_placed = true;
        log::info!(
            "view-centered modal: app_id={} size=({:.0}x{:.0}) zoom={:.2} virtual=({:.1},{:.1})",
            self.get_app_id_string().unwrap_or_default(),
            w, h, zoom, self.virtual_x, self.virtual_y
        );
    }

    /// Redo a latched view-centering now that a self-sizing modal's real
    /// geometry has arrived. One-shot: a later commit (or a user dragging the
    /// window) must not snap it back to the middle.
    pub unsafe fn take_pending_view_center(&mut self) {
        if !self.pending_view_center || self.box_geom.width <= 0 || self.box_geom.height <= 0 {
            return;
        }
        self.pending_view_center = false;
        self.apply_view_centering();
    }

    pub unsafe fn map(&mut self) -> Result<(), &'static str> {
        log::debug!("window '{:?}' mapped", self.get_title());
        if self.get_app_id_string().map_or(false, |id| id.starts_with("cce-status")) {
            log::debug!("[LinkDbg] map app={:?} was_state={:?} linked={}",
                self.get_app_id_string(), self.state, self.is_linked());
        }
        assert!(!matches!(self.impl_type, WindowImpl::Destroying));
        assert_eq!(self.state, WindowState::Initialized);
        self.state = WindowState::Mapped;

        self.try_restore();
        self.try_hint_placement();
        // Last: a session modal's placement is not negotiable, so it wins
        // over both the remembered geometry and any stale place-next hint.
        self.try_center_on_view();
        // After every placement decision, including the invocation-square one:
        // whichever chose this spot, a tiled window must not open stacked on
        // another. The anchor rule already avoids that when any corner is
        // clear, so this only acts when none was.
        self.avoid_tiled_overlap();

        let surface = self.root_surface();
        if !surface.is_null() {
            let commit_listener = &mut self.commit as *mut ffi::wl_listener as *mut WlListener;
            (*commit_listener).notify = Some(handle_window_commit);
            wl_signal_add(ffi::river_wlr_surface_get_commit_signal(surface), &mut self.commit);
        }

        let app_id_ptr = self.get_app_id();
        let (is_status_bar, is_wallpaper) = if !app_id_ptr.is_null() {
            let app_id = std::ffi::CStr::from_ptr(app_id_ptr).to_string_lossy();
            (app_id.starts_with("cce-status"), app_id.as_ref() == "cce-wallpaper")
        } else {
            (false, false)
        };

        if is_status_bar || is_wallpaper {
            self.tiling_mode = crate::tiling::TilingMode::Status;
            if is_status_bar && self.status_edge == StatusEdge::Unspecified {
                let app_id = std::ffi::CStr::from_ptr(app_id_ptr).to_string_lossy();
                let name = if let Some(stripped) = app_id.strip_prefix("cce-status-interface-left-").or_else(|| app_id.strip_prefix("cce-status-left-")) {
                    stripped
                } else if let Some(stripped) = app_id.strip_prefix("cce-status-interface-right-").or_else(|| app_id.strip_prefix("cce-status-right-")) {
                    stripped
                } else {
                    &app_id
                };
                let mut loaded_edge = None;
                if name == "light_source" {
                    let mut light_pos = 2.356194490192345_f32; // Default 135 deg in rad
                    if let Ok(content) = std::fs::read_to_string(cce_ui::config::get_config_path()) {
                        let val = cce_ui::config::parse_kdl_to_json(&content);
                        if let Some(wm_obj) = val.get("window_manager") {
                            if let Some(pos_val) = wm_obj.get("light_source_position") {
                                if let Some(f) = pos_val.as_f64() {
                                    light_pos = f as f32;
                                } else if let Some(i) = pos_val.as_i64() {
                                    let deg = i as f32;
                                    if deg > 2.0 * std::f32::consts::PI {
                                        light_pos = deg.to_radians();
                                    } else {
                                        light_pos = deg;
                                    }
                                }
                            }
                        }
                    }
                    
                    let two_pi = 2.0 * std::f32::consts::PI;
                    let mut angle = light_pos % two_pi;
                    if angle < 0.0 {
                        angle += two_pi;
                    }
                    
                    let pi = std::f32::consts::PI;
                    let edge = if angle < pi / 8.0 || angle >= 15.0 * pi / 8.0 {
                        StatusEdge::Right
                    } else if angle < 3.0 * pi / 8.0 {
                        StatusEdge::TopRight
                    } else if angle < 5.0 * pi / 8.0 {
                        StatusEdge::TopCenter
                    } else if angle < 7.0 * pi / 8.0 {
                        StatusEdge::TopLeft
                    } else if angle < 9.0 * pi / 8.0 {
                        StatusEdge::Left
                    } else if angle < 11.0 * pi / 8.0 {
                        StatusEdge::BottomLeft
                    } else if angle < 13.0 * pi / 8.0 {
                        StatusEdge::BottomCenter
                    } else {
                        StatusEdge::BottomRight
                    };
                    loaded_edge = Some(edge);
                } else if let Ok(content) = std::fs::read_to_string(cce_ui::config::get_config_path()) {
                    let val = cce_ui::config::parse_kdl_to_json(&content);
                    if let Some(layout_obj) = val.get("layout") {
                        if let Some(status_bar_obj) = layout_obj.get("status_bar") {
                            if let Some(edge_val) = status_bar_obj.get(name) {
                                if let Some(edge_str) = edge_val.as_str() {
                                    loaded_edge = match edge_str.to_lowercase().as_str() {
                                        "left" => Some(StatusEdge::Left),
                                        "right" => Some(StatusEdge::Right),
                                        "top-left" => Some(StatusEdge::TopLeft),
                                        "top-center" => Some(StatusEdge::TopCenter),
                                        "top-right" => Some(StatusEdge::TopRight),
                                        "bottom-left" => Some(StatusEdge::BottomLeft),
                                        "bottom-center" => Some(StatusEdge::BottomCenter),
                                        "bottom-right" => Some(StatusEdge::BottomRight),
                                        _ => None,
                                    };
                                }
                            }
                        }
                    }
                }
                self.status_edge = if let Some(edge) = loaded_edge {
                    edge
                } else {
                    if app_id.contains("viewport") {
                        StatusEdge::TopLeft
                    } else if app_id.contains("window") {
                        StatusEdge::TopCenter
                    } else {
                        StatusEdge::TopRight
                    }
                };
            }
        } else {
            let mut should_focus = true;
            if self.session_restored && self.restored_focused {
                (*self.server).wm.restored_focused_window_mapped = true;
                if (*self.server).wm.startup_input_seen {
                    // The user already typed/clicked somewhere (e.g. into
                    // the keepassxc unlock dialog) while this window was
                    // still loading — mapping now must not yank focus out
                    // from under them.
                    log::info!("[FocusRestore] Restored focused window {:?} mapped after user input; leaving focus alone", self.get_title());
                    should_focus = false;
                } else {
                    log::info!("[FocusRestore] Restored focused window mapped: {:?}", self.get_title());
                }
            } else if (*self.server).wm.has_restored_focused_window
                && !(*self.server).wm.restored_focused_window_mapped
                && !(*self.server).wm.startup_input_seen
            {
                // Strict settle phase: until the session's focused window
                // maps (or the user intervenes), NOTHING else auto-focuses —
                // neither restored siblings mapping first nor autostarts.
                // This also keeps the focus-follow pan parked at the saved
                // camera instead of wandering to whichever window loads
                // fastest.
                log::info!("[FocusRestore] Holding focus for the session's focused window; {:?} maps unfocused", self.get_title());
                should_focus = false;
            } else if (*self.server).wm.has_restored_focused_window
                && (*self.server).wm.restored_focused_window_mapped
            {
                if self.session_restored {
                    // A restored sibling mapping after the session's focused
                    // window: never steal back. Only true session restores —
                    // a mid-session spawn that borrowed geometry from
                    // last_window_states is a fresh launch and must focus
                    // (and spawn-pan) normally, else it maps invisible at
                    // its remembered off-viewport spot for the whole session.
                    log::info!("[FocusRestore] Blocking focus to non-focused restored window {:?} because restored focused window is already mapped", self.get_title());
                    should_focus = false;
                } else if !(*self.server).wm.startup_input_seen {
                    // A window mapping unbidden while the session is still
                    // settling (no key/button pressed yet) — an autostart
                    // like keepassxc popping up after the restored windows.
                    // It must not steal focus (or drag the focus-follow pan
                    // over to itself) from the session's focused window.
                    log::info!("[FocusRestore] Blocking focus steal by unrestored window {:?} mapping before first input", self.get_title());
                    should_focus = false;
                }
            }

            // A client whose connection broke rebuilds its surface from
            // scratch (cce-ui window_runner::run) and maps again seconds
            // later. The user never asked for that window, so it must not
            // take focus from whatever they moved on to.
            //
            // Keyed on the previous window vanishing WITHOUT a requested
            // close — not on matching saved state, which a mid-session spawn
            // does too and which must still focus and spawn-pan normally.
            if should_focus {
                if let Some(app_id) = self.get_app_id_string() {
                    if (*self.server).wm.take_recent_vanish(&app_id) {
                        log::info!("[FocusRestore] Blocking focus steal by reconnecting client {:?} ({})", self.get_title(), app_id);
                        should_focus = false;
                    }
                }
            }

            // A WORLD window spawning during overview pulls the session
            // out of it, landing at zoom 1 on the new window — the user
            // asked for it (launcher pick, spawn keybind). Chrome
            // (Popup/Overlay), status, wallpaper and the grid spawn without
            // disturbing the overview. Before the focus loop, so the
            // focus-follow pan sees the settled zoom-1 camera and no-ops.
            if should_focus
                && (*self.server).wm.mode == crate::window_manager::WindowManagerMode::Overview
                && !self.is_grid()
                && !self.is_status_bar()
                && !self.is_wallpaper()
            {
                let resolved = (*self.server).wm.get_mode_for_window(self as *mut Window);
                if !matches!(
                    resolved,
                    crate::tiling::TilingMode::Popup | crate::tiling::TilingMode::Overlay
                ) {
                    (*self.server).wm.exit_overview_to_window(self as *mut Window);
                }
            }

            // The grid layer never takes focus — it is desktop furniture,
            // not a window (it is also input-transparent, so focus here
            // would be unreachable-by-click and unswitchable-away for
            // keyboard input).
            if should_focus && !self.is_grid() {
                let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
                let mut curr = (*seats).next;
                while curr != seats {
                    let next = (*curr).next;
                    let seat = crate::container_of!(curr, crate::seat::Seat, link);
                    (*seat).focus(crate::seat::Focus::Window(self as *mut Window));
                    curr = next;
                }
            }
        }

        (*self.server).wm.dirty_windowing();
        Ok(())
    }

    pub unsafe fn set_closing(&mut self) {
        if self.state != WindowState::Closing {
            if self.get_app_id_string().map_or(false, |id| id.starts_with("cce-status")) {
                log::debug!("[LinkDbg] set_closing app={:?} was_state={:?} was_linked={}",
                    self.get_app_id_string(), self.state, self.is_linked());
            }
            self.state = WindowState::Closing;
            if self.is_linked() {
                wl_list_remove_and_reinit(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
            }
        }
    }

    pub unsafe fn unmap(&mut self) {
        log::debug!("window '{:?}' unmapped", self.get_title());
        if self.get_app_id_string().map_or(false, |id| id.starts_with("cce-status")) {
            log::debug!("[LinkDbg] unmap app={:?} state={:?} linked={}",
                self.get_app_id_string(), self.state, self.is_linked());
        }
        if self.state != WindowState::Mapped {
            return;
        }
        // Nobody asked this window to go: either its program exited on its
        // own or — the case this feeds — its Wayland connection broke and
        // cce-ui is about to rebuild the surface on a fresh one. Chrome is
        // the exception: a Popup (the cce-cloud launcher) or an Overlay dock
        // closes itself as part of being used — Escape, a pick, a click-away,
        // a keyboard leave — and the next super+d inside the grace is a
        // deliberate relaunch that must focus, not a crashed client
        // reconnecting. Counting it left the reopened launcher unfocused.
        if !self.close_requested
            && !matches!(
                self.tiling_mode,
                crate::tiling::TilingMode::Popup | crate::tiling::TilingMode::Overlay
            )
        {
            if let Some(app_id) = self.get_app_id_string() {
                (*self.server).wm.note_vanished(app_id);
            }
        }
        wl_listener_remove_safe(&mut self.commit);
        self.surfaces.save();
        assert!(!matches!(self.impl_type, WindowImpl::Destroying));
        self.set_closing();
        (*self.server).wm.dirty_windowing();

        if !self.foreign_toplevel_handle.is_null() {
            ffi::wlr_ext_foreign_toplevel_handle_v1_destroy(self.foreign_toplevel_handle);
            self.foreign_toplevel_handle = std::ptr::null_mut();
        }
        if !self.wlr_toplevel_handle.is_null() {
            ffi::wlr_foreign_toplevel_handle_v1_destroy(self.wlr_toplevel_handle);
            self.wlr_toplevel_handle = std::ptr::null_mut();
        }


    }

    pub unsafe fn close(&mut self) {
        self.close_requested = true;
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if !toplevel.is_null() {
                    ffi::wlr_xdg_toplevel_send_close((*toplevel).wlr_toplevel);
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if !xwindow.is_null() {
                    ffi::wlr_xwayland_surface_close((*xwindow).xsurface);
                }
            }
            WindowImpl::Destroying => {}
        }
    }

    pub unsafe fn destroy(window: *mut Window) {
        assert!(matches!((*window).impl_type, WindowImpl::Destroying));
        match (*window).state {
            WindowState::Init => {}
            WindowState::Closing => {
                (*(*window).server).wm.dirty_windowing();
                return;
            }
            _ => unreachable!(),
        }
        assert!((*window).object.is_null());

        let seats = &mut (*(*window).server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            if let crate::seat::Focus::Window(w) = (*seat).focused {
                if w == window {
                    (*seat).focus(crate::seat::Focus::None);
                    (*(*window).server).wm.focus_next_visible_window(seat);
                }
            }
            if let Some(ref op) = (*seat).op {
                if op.window_ptr == window {
                    (*seat).op = None;
                }
            }
            curr = next;
        }



        // Destroy decorations
        for decorations in [&mut (*window).decorations_above as *mut ffi::wl_list, &mut (*window).decorations_below as *mut ffi::wl_list] {
            let list_head = decorations as *mut WlList;
            let mut curr = (*list_head).next;
            while curr != list_head {
                let next = (*curr).next;
                let dec = crate::container_of!(curr, Decoration, link);
                (*dec).destroy();
                curr = next;
            }
        }

        wl_listener_remove_safe(&mut (*window).commit);
        ffi::wlr_scene_node_destroy((*window).tree as *mut ffi::wlr_scene_node);
        ffi::wlr_scene_node_destroy((*window).popup_tree as *mut ffi::wlr_scene_node);
        // The border segments hang off the global overlay layer, not off
        // `tree`, so destroying the window tree does not take them with it.
        // Left behind they would both leak and keep a SceneNodeData pointing
        // at this freed window for the next hit test to find.
        ffi::wlr_scene_node_destroy((*window).border.tree as *mut ffi::wlr_scene_node);
        ffi::wlr_scene_node_destroy(&mut (*(*window).capture_scene).tree as *mut ffi::wlr_scene_tree as *mut ffi::wlr_scene_node);

        (*window).node.deinit();

        (*(*window).server).wm.remove_from_history(window);
        (*(*window).server).wm.windows.remove((*window).ref_key);
        (*(*window).server).wm.check_clean_exit_progress();

        let _ = Box::from_raw(window);
    }

    pub unsafe fn set_dimensions_hint(&mut self, hint: DimensionsHint) {
        self.wm_scheduled.dimensions_hint = hint;
        if self.wm_sent.dimensions_hint != hint {
            // Overlay included: a self-sizing overlay (cce-cloud) changes its hint
            // on every resize, and skipping it meant no arrange pass was scheduled.
            // Utility for the same reason: it is self-sizing by definition.
            if matches!(self.tiling_mode, crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Popup | crate::tiling::TilingMode::Status | crate::tiling::TilingMode::Overlay | crate::tiling::TilingMode::Utility) {
                (*self.server).wm.dirty_windowing();
            }
            self.wm_sent.dimensions_hint = hint;
        }
    }

    pub unsafe fn set_dimensions(&mut self, width: u32, height: u32) {
        self.rendering_scheduled.width = width;
        self.rendering_scheduled.height = height;

        if self.rendering_scheduled.resend_dimensions ||
           self.rendering_scheduled.width != self.rendering_sent.width ||
           self.rendering_scheduled.height != self.rendering_sent.height {
            (*self.server).wm.dirty_rendering();
        }
    }

    /// Is a pointer resize op on this window still in progress on any seat?
    pub unsafe fn resize_op_active(&self) -> bool {
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let Some(ref op) = (*seat).op {
                if op.window_ptr == self as *const Window as *mut Window {
                    if let crate::seat::PointerOpType::Resize { .. } = op.op_type {
                        return true;
                    }
                }
            }
            curr_seat = (*curr_seat).next;
        }
        false
    }

    /// Interactive-resize anchoring for the commit path, shared by every
    /// surface kind. While a LEFT/TOP edge is being dragged, the client's
    /// committed size decides where the window's origin goes: the opposite
    /// edge stays where the drag found it, so the dragged edge is the one
    /// that appears to move. Without this the origin holds still and the
    /// window grows away from the grabbed edge — which is what Xwayland
    /// windows did until they were routed through here: only the
    /// xdg-toplevel commit handler had the math.
    ///
    /// `committed_w`/`committed_h` are the size the client just committed,
    /// in `box_geom` units (content size; for wine X11 windows the caller has
    /// already taken the 32px frame off, as the render pass does). Updates
    /// the virtual position and the requested/box screen origin and returns
    /// that origin, or `None` when no resize is armed. The anchoring outlives
    /// the seat op by one commit — the last configure is usually still in
    /// flight at release — so the first commit after the op disarms it.
    pub unsafe fn anchor_resize_commit(&mut self, committed_w: i32, committed_h: i32) -> Option<(i32, i32)> {
        let edges = self.resize_edges?;
        let resize_active = self.resize_op_active();

        if edges.left {
            self.virtual_x = self.resize_start_vx + (self.resize_start_w as f64 - committed_w as f64);
        }
        if edges.top {
            self.virtual_y = self.resize_start_vy + (self.resize_start_h as f64 - committed_h as f64);
        }

        let (final_x, final_y) = self.virtual_to_screen(self.virtual_x, self.virtual_y);
        self.rendering_requested.x = final_x;
        self.rendering_requested.y = final_y;
        self.box_geom.x = final_x;
        self.box_geom.y = final_y;

        if !resize_active {
            self.resize_edges = None;
        }
        Some((final_x, final_y))
    }

    pub unsafe fn set_decoration_hint(&mut self, hint: ffi::zcce_window_v1_decoration_hint) {
        self.wm_scheduled.decoration_hint = hint;
        if hint != self.wm_sent.decoration_hint {
            (*self.server).wm.dirty_windowing();
            self.wm_sent.decoration_hint = hint;
        }
    }

    pub unsafe fn root_surface(&self) -> *mut ffi::wlr_surface {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    std::ptr::null_mut()
                } else {
                    let base = ffi::river_wlr_xdg_toplevel_get_base((*toplevel).wlr_toplevel);
                    ffi::river_wlr_xdg_surface_get_surface(base)
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() || (*xwindow).xsurface.is_null() {
                    std::ptr::null_mut()
                } else {
                    (*(*xwindow).xsurface).surface
                }
            }
            _ => std::ptr::null_mut(),
        }
    }

    pub unsafe fn get_decorations_size(&self) -> (i32, i32) {
        if self.wm_requested.ssd {
            return (0, 0);
        }
        self.measure_decorations()
    }

    /// Raw client-side decoration size (surface minus geometry), regardless
    /// of the current SSD setting. Callers that honor SSD gate on it
    /// themselves.
    pub unsafe fn measure_decorations(&self) -> (i32, i32) {
        let surface = self.root_surface();
        if surface.is_null() {
            return (0, 0);
        }
        let surf_w = ffi::river_wlr_surface_get_width(surface);
        let surf_h = ffi::river_wlr_surface_get_height(surface);
        
        let (geom_w, geom_h) = match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    (surf_w, surf_h)
                } else {
                    ((*toplevel).geometry.width, (*toplevel).geometry.height)
                }
            }
            _ => (surf_w, surf_h),
        };
        
        let dec_w = (surf_w - geom_w).max(0);
        let dec_h = (surf_h - geom_h).max(0);
        (dec_w, dec_h)
    }

    pub unsafe fn send_frame_done(&self) {
        assert_eq!(self.state, WindowState::Mapped);
        if !matches!(self.impl_type, WindowImpl::Destroying) {
            let mut now = std::mem::zeroed();
            clock_gettime(libc::CLOCK_MONOTONIC, &mut now);
            let now_ffi = ffi::timespec {
                tv_sec: now.tv_sec as _,
                tv_nsec: now.tv_nsec as _,
            };
            ffi::wlr_surface_send_frame_done(self.root_surface(), &now_ffi);
        }
    }

    pub unsafe fn manage_start(&mut self) {
        match self.state {
            WindowState::Init => {}
            WindowState::Closing => {
                if self.get_app_id_string().map_or(false, |id| id.starts_with("cce-status")) {
                    log::debug!("[LinkDbg] manage_start closing->init app={:?} was_linked={}",
                        self.get_app_id_string(), self.is_linked());
                }
                self.state = WindowState::Init;
                self.wm_sent = WmSentState {
                    dimensions_hint: DimensionsHint { min_width: 0, min_height: 0, max_width: 0, max_height: 0 },
                    decoration_hint: ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_ONLY_SUPPORTS_CSD,
                    parent: None,
                };
                self.wm_requested = WmRequestedState {
                    dimensions: None,
                    bounds: Dimensions { width: 0, height: 0 },
                    ssd: false,
                    tiled: 0,
                    capabilities: 1 | 2 | 4 | 8,
                    resizing: false,
                    maximized: false,
                    fullscreen: std::ptr::null_mut(),
                    inform_fullscreen: false,
                    close: false,
                };
                self.rendering_sent = WindowRenderingSent {
                    width: 0,
                    height: 0,
                    presentation_hint: ffi::zcce_output_v1_presentation_mode_ZCCE_OUTPUT_V1_PRESENTATION_MODE_VSYNC,
                };
                self.rendering_requested = WindowRenderingRequested {
                    x: 0,
                    y: 0,
                    hidden: false,
                    border: Border::none(),
                    clip: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
                    content_clip: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
                    opacity: 1.0f32,
                    circular: false,
                    blur: false,
                };

                if self.is_linked() {
                    wl_list_remove_and_reinit(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
                }

                self.make_inert();
            }
            WindowState::Ready | WindowState::Initialized | WindowState::Mapped => {
                let wm_v1 = (*self.server).wm.object;
                if wm_v1.is_null() {
                    let is_linked = self.is_linked();
                    if !is_linked {
                        if self.get_app_id_string().map_or(false, |id| id.starts_with("cce-status")) {
                            log::debug!("[LinkDbg] manage_start LINK app={:?} state={:?}",
                                self.get_app_id_string(), self.state);
                        }
                        if !self.node.link.prev.is_null() && !self.node.link.next.is_null() {
                            wl_list_remove_and_reinit(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
                        }
                        let rendering_list = &mut (*self.server).wm.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
                        wl_list_insert((*rendering_list).prev, &mut self.node.link as *mut ffi::wl_list as *mut WlList);

                        if self.foreign_toplevel_handle.is_null() {
                            let list = (*self.server).foreign_toplevel_list;
                            let title = self.get_title();
                            let app_id = self.get_app_id();
                            let state = ffi::wlr_ext_foreign_toplevel_handle_v1_state {
                                title,
                                app_id,
                            };
                            let handle = ffi::wlr_ext_foreign_toplevel_handle_v1_create(list, &state);
                            if !handle.is_null() {
                                self.foreign_toplevel_handle = handle;
                                (*handle).data = self as *mut Window as *mut _;
                            }
                        }

                        if self.wlr_toplevel_handle.is_null() {
                            let manager = (*self.server).wlr_foreign_toplevel_manager;
                            let handle = ffi::wlr_foreign_toplevel_handle_v1_create(manager);
                            if !handle.is_null() {
                                self.wlr_toplevel_handle = handle;
                                let title = self.get_title();
                                if !title.is_null() {
                                    ffi::wlr_foreign_toplevel_handle_v1_set_title(handle, title);
                                }
                                let app_id = self.get_app_id();
                                if !app_id.is_null() {
                                    ffi::wlr_foreign_toplevel_handle_v1_set_app_id(handle, app_id);
                                }
                            }
                        }
                        self.rendering_scheduled.resend_dimensions = true;
                    }
                    return;
                }
                let new_resource = self.object.is_null();
                let window_v1 = if new_resource {
                    let client = ffi::wl_resource_get_client(wm_v1);
                    let res = ffi::wl_resource_create(client, &ffi::zcce_window_v1_interface, ffi::wl_resource_get_version(wm_v1), 0);
                    if res.is_null() {
                        log::error!("out of memory");
                        return;
                    }
                    self.object = res;
                    self.rendering_scheduled.resend_dimensions = true;
                    ffi::wl_resource_set_implementation(
                        res,
                        &WINDOW_INTERFACE as *const _ as *const _,
                        self as *mut Window as *mut _,
                        Some(handle_destroy_resource),
                    );
                    
                    // Send window to manager
                    ffi::wl_resource_post_event(wm_v1, ffi::ZCCE_WINDOW_MANAGER_V1_WINDOW, res); // zcce_window_manager_v1.window
                    res
                } else {
                    self.object
                };

                let is_linked = self.is_linked();
                if !is_linked {
                    if !self.node.link.prev.is_null() && !self.node.link.next.is_null() {
                        wl_list_remove_and_reinit(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
                    }
                    let rendering_list = &mut (*self.server).wm.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
                    wl_list_insert((*rendering_list).prev, &mut self.node.link as *mut ffi::wl_list as *mut WlList);

                    if self.foreign_toplevel_handle.is_null() {
                        let list = (*self.server).foreign_toplevel_list;
                        let title = self.get_title();
                        let app_id = self.get_app_id();
                        let state = ffi::wlr_ext_foreign_toplevel_handle_v1_state {
                            title,
                            app_id,
                        };
                        let handle = ffi::wlr_ext_foreign_toplevel_handle_v1_create(list, &state);
                        if !handle.is_null() {
                            self.foreign_toplevel_handle = handle;
                            (*handle).data = self as *mut Window as *mut _;
                        }
                    }

                    if self.wlr_toplevel_handle.is_null() {
                        let manager = (*self.server).wlr_foreign_toplevel_manager;
                        let handle = ffi::wlr_foreign_toplevel_handle_v1_create(manager);
                        if !handle.is_null() {
                            self.wlr_toplevel_handle = handle;
                            let title = self.get_title();
                            if !title.is_null() {
                                ffi::wlr_foreign_toplevel_handle_v1_set_title(handle, title);
                            }
                            let app_id = self.get_app_id();
                            if !app_id.is_null() {
                                ffi::wlr_foreign_toplevel_handle_v1_set_app_id(handle, app_id);
                            }
                        }
                    }
                };

                if new_resource {
                    let version = ffi::wl_resource_get_version(window_v1);
                    if version >= 2 {
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_UNRELIABLE_PID, self.unreliable_pid()); // sendUnreliablePid
                    }
                    if version >= 4 {
                        if !self.foreign_toplevel_handle.is_null() {
                            let identifier = (*self.foreign_toplevel_handle).identifier;
                            ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_IDENTIFIER, identifier);
                        }
                    }
                }

                if new_resource || self.wm_scheduled.dimensions_hint != self.wm_sent.dimensions_hint {
                    ffi::wl_resource_post_event(
                        window_v1,
                        ffi::ZCCE_WINDOW_V1_DIMENSIONS_HINT, // sendDimensionsHint
                        self.wm_scheduled.dimensions_hint.min_width as i32,
                        self.wm_scheduled.dimensions_hint.min_height as i32,
                        self.wm_scheduled.dimensions_hint.max_width as i32,
                        self.wm_scheduled.dimensions_hint.max_height as i32,
                    );
                    self.wm_sent.dimensions_hint = self.wm_scheduled.dimensions_hint;
                }

                if new_resource || self.wm_scheduled.decoration_hint != self.wm_sent.decoration_hint {
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_DECORATION_HINT, self.wm_scheduled.decoration_hint); // sendDecorationHint
                    self.wm_sent.decoration_hint = self.wm_scheduled.decoration_hint;
                }

                if let Some(ref offset) = self.wm_scheduled.show_window_menu_requested {
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_SHOW_WINDOW_MENU_REQUESTED, offset.x, offset.y); // sendShowWindowMenuRequested
                    self.wm_scheduled.show_window_menu_requested = None;
                }

                match self.wm_scheduled.fullscreen_requested {
                    FullscreenRequest::NoRequest => {}
                    FullscreenRequest::Fullscreen(output) => {
                        let mut out_resource = if output.is_null() { std::ptr::null_mut() } else { (*output).object };
                        if !window_v1.is_null() && !out_resource.is_null() {
                            let client_win = ffi::wl_resource_get_client(window_v1);
                            let client_out = ffi::wl_resource_get_client(out_resource);
                            if client_win != client_out {
                                log::error!(
                                    "Fullscreen output client mismatch: win_client={:?}, out_client={:?}. Fallback to null_mut",
                                    client_win,
                                    client_out
                                );
                                out_resource = std::ptr::null_mut();
                            }
                        }
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_FULLSCREEN_REQUESTED, out_resource); // sendFullscreenRequested
                    }
                    FullscreenRequest::Exit => {
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_EXIT_FULLSCREEN_REQUESTED); // sendExitFullscreenRequested
                    }
                }
                self.wm_scheduled.fullscreen_requested = FullscreenRequest::NoRequest;

                match self.wm_scheduled.maximize_requested {
                    MaximizeRequest::NoRequest => {}
                    MaximizeRequest::Maximize => {
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_MAXIMIZE_REQUESTED); // sendMaximizeRequested
                    }
                    MaximizeRequest::Unmaximize => {
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_UNMAXIMIZE_REQUESTED); // sendUnmaximizeRequested
                    }
                }
                self.wm_scheduled.maximize_requested = MaximizeRequest::NoRequest;

                if self.wm_scheduled.minimize_requested {
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_MINIMIZE_REQUESTED); // sendMinimizeRequested
                }
                self.wm_scheduled.minimize_requested = false;

                let parent = self.get_parent();
                if !parent.is_null() {
                    let parent_ref = Some((*parent).ref_key);
                    if self.wm_sent.parent.is_none() || self.wm_sent.parent != parent_ref {
                        let parent_obj = (*parent).object;
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_PARENT, parent_obj); // sendParent
                        self.wm_sent.parent = parent_ref;
                    }
                } else if self.wm_sent.parent.is_some() {
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_PARENT, std::ptr::null_mut::<ffi::wl_resource>()); // sendParent
                    self.wm_sent.parent = None;
                }

                if new_resource || self.wm_scheduled.dirty_app_id {
                    let app_id = self.get_app_id();
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_APP_ID, app_id); // sendAppId
                    self.wm_scheduled.dirty_app_id = false;
                }

                if new_resource || self.wm_scheduled.dirty_title {
                    let title = self.get_title();
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_TITLE, title); // sendTitle
                    self.wm_scheduled.dirty_title = false;
                }

                if let Some(seat) = self.wm_scheduled.pointer_move_requested.as_mut() {
                    if !seat.object.is_null() {
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_POINTER_MOVE_REQUESTED, seat.object); // sendPointerMoveRequested
                    }
                }
                self.wm_scheduled.pointer_move_requested = std::ptr::null_mut();

                if let Some(ref data) = self.wm_scheduled.pointer_resize_requested {
                    if let Some(seat) = unsafe { data.seat.as_ref() } {
                        if !seat.object.is_null() {
                            ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_POINTER_RESIZE_REQUESTED, seat.object, data.edges); // sendPointerResizeRequested
                        }
                    }
                }
                self.wm_scheduled.pointer_resize_requested = None;
            }
        }
    }

    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_V1_CLOSED); // sendClosed // sendClosed
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_WINDOW_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
            (*self.server).wm.dirty_windowing();
            self.node.make_inert();

            for decorations in [&mut self.decorations_above as *mut ffi::wl_list, &mut self.decorations_below as *mut ffi::wl_list] {
                let list_head = decorations as *mut WlList;
                let mut curr = (*list_head).next;
                while curr != list_head {
                    let next = (*curr).next;
                    let dec = crate::container_of!(curr, Decoration, link);
                    (*dec).make_inert();
                    curr = next;
                }
            }

            let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*seats).next;
            while curr != seats {
                let next = (*curr).next;
                let seat = crate::container_of!(curr, crate::seat::Seat, link);
                if let crate::seat::Focus::Window(w) = (*seat).focused {
                    if w == self as *mut Window {
                        (*seat).focus(crate::seat::Focus::None);
                    }
                }
                curr = next;
            }
        }
    }

    pub unsafe fn manage_finish(&mut self) -> bool {
        if matches!(self.impl_type, WindowImpl::Destroying) {
            assert_eq!(self.state, WindowState::Closing);
            return false;
        }

        match self.state {
            WindowState::Init => unreachable!(),
            WindowState::Ready => {
                if self.wm_requested.dimensions.is_none() && self.wm_requested.fullscreen.is_null() {
                    return false;
                }
                if self.get_app_id_string().map_or(false, |id| id.starts_with("cce-status")) {
                    log::debug!("[LinkDbg] manage_finish ready->initialized app={:?} linked={}",
                        self.get_app_id_string(), self.is_linked());
                }
                self.state = WindowState::Initialized;
            }
            WindowState::Initialized | WindowState::Mapped => {}
            WindowState::Closing => return false,
        }

        if self.wm_requested.close {
            self.close();
            self.wm_requested.close = false;
        }

        let mut activated = false;
        let seats = &mut (*self.server).wm.sent.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link_sent);
            if let crate::seat::Focus::Window(w) = (*seat).focused {
                if w == self as *mut Window {
                    activated = true;
                    break;
                }
            }
            curr = next;
        }

        if !self.wlr_toplevel_handle.is_null() {
            ffi::wlr_foreign_toplevel_handle_v1_set_activated(self.wlr_toplevel_handle, activated);
        }

        let output = if !self.wm_requested.fullscreen.is_null() {
            self.wm_requested.fullscreen
        } else if self.is_fullscreen() {
            let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*outputs_list).next;
            let mut found_output = std::ptr::null_mut();
            while curr != outputs_list {
                let out = crate::container_of!(curr, crate::output::Output, link);
                if (*out).sent.state == crate::output::OutputStateValue::Enabled {
                    found_output = out;
                    break;
                }
                curr = (*curr).next;
            }
            found_output
        } else {
            std::ptr::null_mut()
        };

        let new_fullscreen = !output.is_null();
        if new_fullscreen && !self.was_fullscreen {
            if self.box_geom.width > 0 && self.box_geom.height > 0 {
                self.start_fs_anim();
                self.saved_width = self.box_geom.width;
                self.saved_height = self.box_geom.height;
                self.saved_virtual_x = self.virtual_x;
                self.saved_virtual_y = self.virtual_y;
                self.was_fullscreen = true;
                log::info!("[Fullscreen] Saved window {:?} geometry: {}x{} at ({}, {})", self.get_title_string().as_deref().unwrap_or(""), self.saved_width, self.saved_height, self.saved_virtual_x, self.saved_virtual_y);
            }
        } else if !new_fullscreen && self.was_fullscreen {
            if self.saved_width > 0 && self.saved_height > 0 {
                // Captures the on-screen fullscreen rect before the restore
                // below rewrites box_geom.
                self.start_fs_anim();
                self.box_geom.width = self.saved_width;
                self.box_geom.height = self.saved_height;
                self.virtual_x = self.saved_virtual_x;
                self.virtual_y = self.saved_virtual_y;
                self.was_fullscreen = false;

                self.wm_requested.dimensions = Some(crate::window::Dimensions {
                    width: self.saved_width as u32,
                    height: self.saved_height as u32,
                });
                self.wm_requested.bounds = crate::window::Dimensions {
                    width: self.saved_width as u32,
                    height: self.saved_height as u32,
                };

                (*self.server).wm.dirty_windowing();
                log::info!("[Fullscreen] Restored window {:?} geometry: {}x{} at ({}, {})", self.get_title_string().as_deref().unwrap_or(""), self.saved_width, self.saved_height, self.saved_virtual_x, self.saved_virtual_y);
            }
        }

        let (width, height) = if !output.is_null() {
            let (w, h) = (*output).sent.dimensions();
            if self.configure_sent.width != Some(w as u32) || self.configure_sent.height != Some(h as u32) {
                self.configure_scheduled.width = Some(w as u32);
                self.configure_scheduled.height = Some(h as u32);
                self.rendering_scheduled.resend_dimensions = true;
                (Some(w as u32), Some(h as u32))
            } else {
                (None, None)
            }
        } else if let Some(dimensions) = self.wm_requested.dimensions {
            self.rendering_scheduled.resend_dimensions = true;
            (Some(dimensions.width), Some(dimensions.height))
        } else {
            (None, None)
        };
        self.wm_requested.dimensions = None;

        // A tiled window thinks it is maximized: the xdg maximized state
        // follows the mode.
        let is_maximized_layout = self.tiling_mode == crate::tiling::TilingMode::Tiled;
        self.configure_scheduled = Configure {
            width,
            height,
            bounds: self.wm_requested.bounds,
            activated,
            ssd: self.wm_requested.ssd,
            tiled: self.wm_requested.tiled,
            capabilities: self.wm_requested.capabilities,
            maximized: self.wm_requested.maximized || is_maximized_layout,
            inform_fullscreen: self.wm_requested.inform_fullscreen || self.is_fullscreen(),
            resizing: self.wm_requested.resizing,
        };

        let track_configure = match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    false
                } else {
                    (*toplevel).configure()
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    false
                } else {
                    (*xwindow).configure()
                }
            }
            WindowImpl::Destroying => unreachable!(),
        };

        if track_configure && matches!(self.state, WindowState::Mapped) {
            self.surfaces.save();
            self.send_frame_done();
        }

        track_configure
    }

    pub unsafe fn render_start(&mut self) {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if !toplevel.is_null() {
                    match (*toplevel).configure_state {
                        ConfigureState::Inflight(serial) => {
                            (*toplevel).configure_state = ConfigureState::TimedOut(serial);
                        }
                        ConfigureState::Acked => {
                            (*toplevel).configure_state = ConfigureState::TimedOutAcked;
                        }
                        ConfigureState::Committed => {
                            (*toplevel).configure_state = ConfigureState::Idle;
                        }
                        _ => {}
                    }
                    self.rendering_scheduled.width = (*toplevel).geometry.width as u32;
                    self.rendering_scheduled.height = (*toplevel).geometry.height as u32;
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if !xwindow.is_null() {
                    let s = crate::xwayland_window::x11_scale(self.server);
                    let mut w = crate::xwayland_window::from_x11((*(*xwindow).xsurface).width as i32, s) as u32;
                    let mut h = crate::xwayland_window::from_x11((*(*xwindow).xsurface).height as i32, s) as u32;
                    let has_parent = !(*(*xwindow).xsurface).parent.is_null();
                    if self.is_wine() && !has_parent && !self.is_fullscreen() {
                        w = w.saturating_sub((crate::xwayland_window::WINE_MARGIN * 2) as u32);
                        h = h.saturating_sub((crate::xwayland_window::WINE_MARGIN * 2) as u32);
                    }
                    self.rendering_scheduled.width = w;
                    self.rendering_scheduled.height = h;
                }
            }
            WindowImpl::Destroying => {}
        }

        let presentation_hint = self.presentation_hint();
        let sent = &mut self.rendering_sent;
        let scheduled = &mut self.rendering_scheduled;

        if matches!(self.state, WindowState::Mapped) &&
           (scheduled.resend_dimensions ||
            scheduled.width != sent.width || scheduled.height != sent.height) {
            if !self.object.is_null() {
                ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_V1_DIMENSIONS, scheduled.width as i32, scheduled.height as i32); // sendDimensions
                scheduled.resend_dimensions = false;
            }
        }
        sent.width = scheduled.width;
        sent.height = scheduled.height;
        if sent.presentation_hint != presentation_hint {
            if !self.object.is_null() {
                let version = ffi::wl_resource_get_version(self.object);
                if version >= 4 {
                    ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_V1_PRESENTATION_HINT, presentation_hint); // sendPresentationHint
                }
            }
            sent.presentation_hint = presentation_hint;
        }
    }

    pub unsafe fn presentation_hint(&self) -> ffi::zcce_output_v1_presentation_mode {
        let root = self.root_surface();
        if root.is_null() {
            return ffi::zcce_output_v1_presentation_mode_ZCCE_OUTPUT_V1_PRESENTATION_MODE_VSYNC;
        }
        
        // tearing control check stub:
        // switch (server.tearing_control_manager.hintFromSurface(root)) {
        //     .async => .async,
        //     .vsync => .vsync,
        // }
        // For now, return VSYNC by default.
        ffi::zcce_output_v1_presentation_mode_ZCCE_OUTPUT_V1_PRESENTATION_MODE_VSYNC
    }

    pub unsafe fn notify_title(&mut self) {
        self.wm_scheduled.dirty_title = true;
        self.try_restore();
        (*self.server).wm.dirty_windowing();

        if !self.foreign_toplevel_handle.is_null() {
            let title = self.get_title();
            let app_id = self.get_app_id();
            let state = ffi::wlr_ext_foreign_toplevel_handle_v1_state {
                title,
                app_id,
            };
            ffi::wlr_ext_foreign_toplevel_handle_v1_update_state(self.foreign_toplevel_handle, &state);
        }

        if !self.wlr_toplevel_handle.is_null() {
            let title = self.get_title();
            if !title.is_null() {
                ffi::wlr_foreign_toplevel_handle_v1_set_title(self.wlr_toplevel_handle, title);
            }
        }
    }

    pub unsafe fn notify_app_id(&mut self) {
        self.wm_scheduled.dirty_app_id = true;
        let app_id_str = self.get_app_id_string();
        if app_id_str.as_deref().map_or(false, |id| id.starts_with("cce-status") || id == "cce-wallpaper") {
            self.tiling_mode = crate::tiling::TilingMode::Status;
        }
        self.try_restore();
        (*self.server).wm.dirty_windowing();

        if !self.foreign_toplevel_handle.is_null() {
            let title = self.get_title();
            let app_id = self.get_app_id();
            let state = ffi::wlr_ext_foreign_toplevel_handle_v1_state {
                title,
                app_id,
            };
            ffi::wlr_ext_foreign_toplevel_handle_v1_update_state(self.foreign_toplevel_handle, &state);
        }

        if !self.wlr_toplevel_handle.is_null() {
            let app_id = self.get_app_id();
            if !app_id.is_null() {
                ffi::wlr_foreign_toplevel_handle_v1_set_app_id(self.wlr_toplevel_handle, app_id);
            }
        }
    }

    pub unsafe fn render_finish(&mut self) {
        let requested = &self.rendering_requested;
        let enabled = !requested.hidden && (matches!(self.state, WindowState::Mapped) || matches!(self.state, WindowState::Closing));

        let title_ptr = match self.impl_type {
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() { std::ptr::null() } else { (*(*xwindow).xsurface).title }
            }
            _ => std::ptr::null(),
        };
        let title = if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") };
        if title.contains("Ubisoft") {
            log::info!("render_finish for '{}' (addr={:p}): enabled={} hidden={} state={:?}", title, self as *const Window, enabled, requested.hidden, self.state);
        }

        ffi::wlr_scene_node_set_enabled(self.tree as *mut ffi::wlr_scene_node, enabled);
        ffi::wlr_scene_node_set_enabled(self.popup_tree as *mut ffi::wlr_scene_node, enabled);
        if !enabled {
            // The segment tree is not a child of `tree`, so disabling the
            // window does not hide a revealed border with it.
            self.border_reveal = [0.0; 8];
            ffi::wlr_scene_node_set_enabled(self.border.tree as *mut ffi::wlr_scene_node, false);
        }

        if enabled {
            let app_id = self.get_app_id_string().unwrap_or_default();
            let is_status = self.tiling_mode == crate::tiling::TilingMode::Status ||
                            app_id.starts_with("cce-status");
            let is_decorated = (*self.server).wm.is_decorated_app(&app_id);
            let blur_enabled = requested.blur && (self.wm_requested.ssd || is_decorated || is_status) && !self.droplet_backdrop_on();
            let mut ignore_transparent = (*self.server).wm.layout.window_backdrop_blur_ignore_transparent;
            if is_status {
                ignore_transparent = (*self.server).wm.layout.status_backdrop_blur_ignore_transparent;
            }
            // Hoisted above the blur setup: the blur node needs this radius, and whether
            // the window wants rounded corners at all decides the optimized-blur question
            // below.
            let radius = if self.is_fullscreen() {
                0
            } else if requested.circular {
                let w = self.rendering_sent.width as i32;
                let h = self.rendering_sent.height as i32;
                w.min(h) / 2
            } else if is_status {
                // Status segments draw their own module-box corners. The
                // root plate clip is invisible on a bar-thin segment (the
                // half-extent cap keeps it inside the transparent band) but
                // carves visible sweeps into an EXPANDED segment's in-surface
                // menu box once the cap stops binding.
                0
            } else if self.wm_requested.ssd || is_decorated {
                (*self.server).wm.layout.root_plate_corner_radius
            } else {
                0
            };
            // Rounded corners do NOT require live blur: the corner shape is applied by the
            // standard blur node's sampler (wlr_scene_blur_set_corner_radius) in both modes;
            // the optimized node only re-bakes the shared offscreen cache
            // (fx_render_pass_add_optimized_blur -> read_to_buffer) and never paints on
            // screen. The old `radius > 0` opt-out silently disabled the optimization for
            // every (rounded) window, forcing full-backdrop dual-kawase blur per frame per
            // translucent window — the DE-wide hover-lag / constant-GPU-load root cause.
            let use_optimized = if is_status {
                false
            } else {
                (*self.server).wm.layout.scenefx_optimized_blur
            };
            let toplevel_w = match self.impl_type {
                WindowImpl::Toplevel(toplevel) => {
                    if toplevel.is_null() { 0 } else { (*toplevel).geometry.width }
                }
                _ => 0,
            };
            let toplevel_h = match self.impl_type {
                WindowImpl::Toplevel(toplevel) => {
                    if toplevel.is_null() { 0 } else { (*toplevel).geometry.height }
                }
                _ => 0,
            };
            // Status segments are self-sizing: their committed geometry is
            // fresher than the render-start snapshot (`rendering_sent`),
            // which lags an expand/contract commit by a render pass — same
            // rule as the commit-path blur sizing in xdg_toplevel.rs.
            let (actual_w, actual_h) = if is_status && toplevel_w > 0 && toplevel_h > 0 {
                (toplevel_w as u32, toplevel_h as u32)
            } else {
                (
                    if self.rendering_sent.width > 0 { self.rendering_sent.width } else { toplevel_w as u32 },
                    if self.rendering_sent.height > 0 { self.rendering_sent.height } else { toplevel_h as u32 },
                )
            };
            // Widen squircle corners to the span the clients draw (see
            // widen_corner_radius); circles already sit at the half-extent cap.
            let radius = if requested.circular { radius } else { widen_corner_radius(radius, actual_w as i32, actual_h as i32) };
            // Mid fullscreen-toggle the window draws at the animated rect:
            // buffers stretch per-axis toward it (aspect changes in flight,
            // so the axes diverge) and the effect extents follow.
            let (scale_x, scale_y) = match self.fs_anim {
                Some(anim) if actual_w > 0 && actual_h > 0 => {
                    (anim.w / actual_w as f64, anim.h / actual_h as f64)
                }
                _ => (self.scale, self.scale),
            };
            let width = (actual_w as f64 * scale_x) as i32;
            let height = (actual_h as f64 * scale_y) as i32;
            ffi::river_scene_node_enable_blur(
                self.tree as *mut ffi::wlr_scene_node,
                blur_enabled,
                use_optimized,
                ignore_transparent,
                0,
                0,
                width,
                height,
                // width/height above are scaled to device pixels, so the radius must be too
                // (cf. the window_background rect, which scales it the same way).
                (radius as f64 * self.scale) as i32,
            );
            // The decorated-window predicate feeds two things: the shadow, and
            // the bevel's focus glint. Only the shadow honours the tiled switch.
            let want_decor = !is_status && (self.wm_requested.ssd || is_decorated) && !self.is_fullscreen();
            let want_shadow = want_decor && self.wants_tiled_shadow();
                // The bevel keys on its OWN app list, not on is_decorated:
                // every cce-ui app draws its own bevel, so a compositor one
                // would sit on top of it.
                let want_bevel = !is_status
                    && !self.is_fullscreen()
                    && (*self.server).wm.is_beveled_app(&app_id);
            self.update_shadow(width, height, radius, want_shadow);
                self.update_bevel(width, height, radius, want_bevel, want_decor);
                self.update_droplet(width, height);
            ffi::river_scene_node_set_opacity(self.tree as *mut ffi::wlr_scene_node, requested.opacity);

            // Device px, like the blur radius above: the surface content is
            // scaled to its dest size, so an unscaled clip radius would keep
            // cutting zoom-1-sized corners into a zoomed-down window (the
            // clients' own drawn corners shrink with the buffer).
            ffi::river_scene_node_set_corner_radius(
                self.surfaces.tree as *mut ffi::wlr_scene_node,
                (radius as f64 * self.scale) as i32,
            );
            ffi::river_scene_rect_set_corner_radius(
                self.window_background,
                (radius as f64 * self.scale) as i32,
            );

            struct ScaleData {
                scale_x: f64,
                scale_y: f64,
                ancestor: *mut ffi::wlr_scene_node,
            }

            unsafe extern "C" fn set_overview_scale_iterator(
                buffer: *mut ffi::wlr_scene_buffer,
                sx: i32,
                sy: i32,
                user_data: *mut std::ffi::c_void,
            ) {
                let data = &*(user_data as *const ScaleData);
                let node = buffer as *mut ffi::wlr_scene_node;

                let surface = ffi::river_scene_node_get_surface(node);
                if !surface.is_null() {
                    let (w, h, ox, oy) = surface_buffer_extent(buffer, surface);
                    if data.scale_x == 1.0 && data.scale_y == 1.0 {
                        ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                        ffi::river_scene_node_set_position_if_changed(node, ox, oy);
                    } else {
                        let dest_w = (w as f64 * data.scale_x) as i32;
                        let dest_h = (h as f64 * data.scale_y) as i32;
                        ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                        // The parent offset scales like the content; the
                        // clip origin rides on top of it, scaled the same.
                        let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                        let dest_x = (px as f64 * (data.scale_x - 1.0) + ox as f64 * data.scale_x) as i32;
                        let dest_y = (py as f64 * (data.scale_y - 1.0) + oy as f64 * data.scale_y) as i32;
                        ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                    }
                    // Keep the opaque region in step with the dest scale —
                    // unscaled it covers the shrunken node's translucent CSD
                    // margins and occlusion culling stops repainting behind
                    // the client shadow (stale pixels show through it). The
                    // region must never overclaim, so a briefly non-uniform
                    // stretch takes the smaller axis.
                    ffi::river_scene_buffer_set_scaled_opaque_region(buffer, surface, data.scale_x.min(data.scale_y));
                }
                // Non-surface buffers are frozen SAVED copies (see
                // save_surface_tree_iter): their natural buffer size is
                // meaningless for geometry — HiDPI clients commit scale-N
                // buffers and Chromium pads buffers beyond the surface,
                // cropping via viewport src — so rescaling from it ballooned
                // ghosts around the window at any zoom change. A frozen copy
                // keeps its save-time dest/position; a zoom mid-transaction
                // leaves it briefly at the old zoom, which restore corrects.
            }

            let scale_data_surfaces = ScaleData { scale_x, scale_y, ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.tree as *mut ffi::wlr_scene_node,
                Some(set_overview_scale_iterator),
                &scale_data_surfaces as *const ScaleData as *mut std::ffi::c_void,
            );

            if self.surfaces.saved {
                let scale_data_saved = ScaleData { scale_x, scale_y, ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node };
                ffi::wlr_scene_node_for_each_buffer(
                    self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                    Some(set_overview_scale_iterator),
                    &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
                );
            }
            
            let scale_data_popup = ScaleData { scale_x, scale_y, ancestor: self.popup_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.popup_tree as *mut ffi::wlr_scene_node,
                Some(set_overview_scale_iterator),
                &scale_data_popup as *const ScaleData as *mut std::ffi::c_void,
            );
            self.last_applied_scale = self.scale;
        }

        // During an interactive resize, size the box from the client's
        // CURRENT committed geometry instead of the render-start snapshot
        // (rendering_sent): commits land between render_start and
        // render_finish, and the anchored position (rendering_requested.x,
        // updated by the commit handler) always tracks the newest commit.
        // Pairing it with the older snapshot size clips the surface short
        // and makes the anchored edge bounce every cycle.
        // self_resized: same reasoning, for a client that resized itself without a
        // configure — its newest buffer is already on screen, so rendering_sent is
        // behind and would drag the border back to the previous size.
        let mut resize_synced = false;
        if self.resize_edges.is_some() || self.self_resized {
            if let WindowImpl::Toplevel(toplevel) = self.impl_type {
                if !toplevel.is_null() {
                    self.box_geom.width = (*toplevel).geometry.width;
                    self.box_geom.height = (*toplevel).geometry.height;
                    resize_synced = true;
                }
            }
        }
        if !resize_synced {
            if self.rendering_sent.width > 0 {
                self.box_geom.width = self.rendering_sent.width as i32;
            }
            if self.rendering_sent.height > 0 {
                self.box_geom.height = self.rendering_sent.height as i32;
            }
        }
        self.self_resized = false;

        let mut clip = requested.clip;
        let mut content_clip = requested.content_clip;

        let output = if !self.wm_requested.fullscreen.is_null() {
            self.wm_requested.fullscreen
        } else if self.is_fullscreen() {
            let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*outputs_list).next;
            let mut found_output = std::ptr::null_mut();
            while curr != outputs_list {
                let out = crate::container_of!(curr, crate::output::Output, link);
                if (*out).sent.state == crate::output::OutputStateValue::Enabled {
                    found_output = out;
                    break;
                }
                curr = (*curr).next;
            }
            found_output
        } else {
            std::ptr::null_mut()
        };

        if !output.is_null() {
            self.box_geom.x = (*output).sent.x;
            self.box_geom.y = (*output).sent.y;

            let app_id_ptr = self.get_app_id();
            let (is_status_bar, is_wallpaper) = if !app_id_ptr.is_null() {
                let app_id = std::ffi::CStr::from_ptr(app_id_ptr).to_string_lossy();
                (app_id.starts_with("cce-status"), app_id.as_ref() == "cce-wallpaper")
            } else {
                (false, false)
            };

            ffi::wlr_scene_node_set_enabled(self.fullscreen_background as *mut ffi::wlr_scene_node, !is_status_bar && !is_wallpaper);
            let (width, height) = (*output).sent.dimensions();
            ffi::wlr_scene_rect_set_size(self.fullscreen_background, width as i32, height as i32);
            clip = ffi::wlr_box { x: 0, y: 0, width: width as i32, height: height as i32 };
            content_clip = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };

            ffi::wlr_scene_node_set_enabled(self.border.left as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.right as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.top as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.bottom as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.window_background as *mut ffi::wlr_scene_node, false);
            // Fullscreen skips draw_borders entirely, and the segment tree
            // lives outside this window's tree, so it has to be taken down
            // explicitly or a revealed edge would hang over the fullscreen
            // surface.
            self.border_reveal = [0.0; 8];
            ffi::wlr_scene_node_set_enabled(self.border.tree as *mut ffi::wlr_scene_node, false);
        } else {
            self.box_geom.x = requested.x;
            self.box_geom.y = requested.y;
            ffi::wlr_scene_node_set_enabled(self.fullscreen_background as *mut ffi::wlr_scene_node, false);
            if self.fs_anim.is_none() {
                self.draw_borders();
            }
        }

        ffi::river_scene_node_set_position_if_changed(self.tree as *mut ffi::wlr_scene_node, self.box_geom.x, self.box_geom.y);
        ffi::river_scene_node_set_position_if_changed(self.popup_tree as *mut ffi::wlr_scene_node, self.box_geom.x, self.box_geom.y);

        // Mid fullscreen-toggle: draw at the animated rect regardless of which
        // branch above ran. The tree overrides its settled position, the black
        // backdrop rides the rect (it is what grows/shrinks visually on both
        // directions), the clip follows, and the borders stay down until the
        // animation settles — the final settling frame re-runs the branch
        // above with fs_anim cleared and puts everything back.
        if let Some(anim) = self.fs_anim {
            let ax = anim.x.round() as i32;
            let ay = anim.y.round() as i32;
            let aw = (anim.w.round() as i32).max(1);
            let ah = (anim.h.round() as i32).max(1);
            ffi::river_scene_node_set_position_if_changed(self.tree as *mut ffi::wlr_scene_node, ax, ay);
            ffi::river_scene_node_set_position_if_changed(self.popup_tree as *mut ffi::wlr_scene_node, ax, ay);
            ffi::wlr_scene_node_set_enabled(self.fullscreen_background as *mut ffi::wlr_scene_node, true);
            ffi::wlr_scene_rect_set_size(self.fullscreen_background, aw, ah);
            clip = ffi::wlr_box { x: 0, y: 0, width: aw, height: ah };
            content_clip = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
            ffi::wlr_scene_node_set_enabled(self.border.left as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.right as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.top as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.bottom as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.window_background as *mut ffi::wlr_scene_node, false);
            self.border_reveal = [0.0; 8];
            ffi::wlr_scene_node_set_enabled(self.border.tree as *mut ffi::wlr_scene_node, false);
        }

        // No geometry compensation here: wlr_scene_xdg_surface_create already
        // anchors its subtree at the top-left of the xdg window geometry (it
        // re-offsets by -geometry on every commit), so subtracting geometry.x/y
        // again shifted CSD windows with shadow margins (Electron/Chromium
        // floating) up-left by their shadow size, off the desktop grid.
        ffi::river_scene_node_set_position_if_changed(self.surfaces.tree as *mut ffi::wlr_scene_node, 0, 0);

        self.apply_surface_clip(&clip, &content_clip);

        for decorations in [&mut self.decorations_above as *mut ffi::wl_list, &mut self.decorations_below as *mut ffi::wl_list] {
            let list_head = decorations as *mut WlList;
            let mut curr = (*list_head).next;
            while curr != list_head {
                let next = (*curr).next;
                let dec = crate::container_of!(curr, Decoration, link);
                (*dec).render_finish(&clip);
                curr = next;
            }
        }

        match self.impl_type {
            WindowImpl::Xwayland(xwindow) => {
                if !xwindow.is_null() {
                    if !(*xwindow).surface_tree.is_null() {
                        let has_parent = !(*(*xwindow).xsurface).parent.is_null();
                        if self.is_wine() && !has_parent && !self.is_fullscreen() {
                            ffi::wlr_scene_node_set_position((*xwindow).surface_tree as *mut ffi::wlr_scene_node, -16, -16);
                        } else {
                            ffi::wlr_scene_node_set_position((*xwindow).surface_tree as *mut ffi::wlr_scene_node, 0, 0);
                        }
                    }
                    (*xwindow).configure();
                }
            }
            _ => {}
        }
    }

    pub unsafe fn scale_only_render_finish(&mut self) {
        // Mid fullscreen-toggle the animation tick owns the buffer dest
        // sizes; a uniform-scale pass here would stomp the stretch.
        if self.fs_anim.is_some() {
            return;
        }
        // The zoom the overview asks for, times 1/output-scale for an X11
        // surface whose buffer is physical pixels (`x11_buffer_scale`).
        let eff_scale = self.scale * self.x11_buffer_scale();
        if eff_scale == 1.0 {
            self.last_applied_scale = 1.0;
            return;
        }

        // No last_applied_scale short-circuit here: wlroots' scene-surface
        // commit listener resets a committed buffer's dest size and opaque
        // region to the surface's natural extent, so any client repainting
        // while scaled (browser animations, caret blink) pops back to full
        // size even though the cached scale says nothing changed. This runs
        // per rendered frame (output.rs render_and_commit), after commits and
        // before build_state, and every setter below is change-checked — an
        // already-correct tree produces no damage.
        self.last_applied_scale = self.scale;

        struct ScaleData {
            scale: f64,
            ancestor: *mut ffi::wlr_scene_node,
        }

        unsafe extern "C" fn set_overview_scale_iterator(
            buffer: *mut ffi::wlr_scene_buffer,
            sx: i32,
            sy: i32,
            user_data: *mut std::ffi::c_void,
        ) {
            let data = &*(user_data as *const ScaleData);
            let node = buffer as *mut ffi::wlr_scene_node;

            let surface = ffi::river_scene_node_get_surface(node);
            if !surface.is_null() {
                let (w, h, ox, oy) = surface_buffer_extent(buffer, surface);
                if data.scale == 1.0 {
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                    ffi::river_scene_node_set_position_if_changed(node, ox, oy);
                } else {
                    let dest_w = (w as f64 * data.scale) as i32;
                    let dest_h = (h as f64 * data.scale) as i32;
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                    let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                    let dest_x = (px as f64 * (data.scale - 1.0) + ox as f64 * data.scale) as i32;
                    let dest_y = (py as f64 * (data.scale - 1.0) + oy as f64 * data.scale) as i32;
                    ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                }
                // Keep the opaque region in step with the dest scale —
                // unscaled it covers the shrunken node's translucent CSD
                // margins and occlusion culling stops repainting behind
                // the client shadow (stale pixels show through it).
                ffi::river_scene_buffer_set_scaled_opaque_region(buffer, surface, data.scale);
            }
            // Non-surface buffers are frozen SAVED copies (see
            // save_surface_tree_iter): their natural buffer size is
            // meaningless for geometry — HiDPI clients commit scale-N
            // buffers and Chromium pads buffers beyond the surface,
            // cropping via viewport src — so rescaling from it ballooned
            // ghosts around the window at any zoom change. A frozen copy
            // keeps its save-time dest/position; a zoom mid-transaction
            // leaves it briefly at the old zoom, which restore corrects.
        }

        let scale_data_surfaces = ScaleData { scale: eff_scale, ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.surfaces.tree as *mut ffi::wlr_scene_node,
            Some(set_overview_scale_iterator),
            &scale_data_surfaces as *const ScaleData as *mut std::ffi::c_void,
        );

        if self.surfaces.saved {
            let scale_data_saved = ScaleData { scale: eff_scale, ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                Some(set_overview_scale_iterator),
                &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
            );
        }

        let scale_data_popup = ScaleData { scale: eff_scale, ancestor: self.popup_tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.popup_tree as *mut ffi::wlr_scene_node,
            Some(set_overview_scale_iterator),
            &scale_data_popup as *const ScaleData as *mut std::ffi::c_void,
        );

        for decorations in [&mut self.decorations_above as *mut ffi::wl_list, &mut self.decorations_below as *mut ffi::wl_list] {
            let list_head = decorations as *mut WlList;
            let mut curr = (*list_head).next;
            while curr != list_head {
                let next = (*curr).next;
                let dec = crate::container_of!(curr, Decoration, link);
                (*dec).scale_only_render_finish();
                curr = next;
            }
        }
    }

    pub unsafe fn render_viewport_update(&mut self) {
        let requested = &self.rendering_requested;
        let enabled = !requested.hidden && (matches!(self.state, WindowState::Mapped) || matches!(self.state, WindowState::Closing));

        ffi::wlr_scene_node_set_enabled(self.tree as *mut ffi::wlr_scene_node, enabled);
        ffi::wlr_scene_node_set_enabled(self.popup_tree as *mut ffi::wlr_scene_node, enabled);
        if !enabled {
            self.border_reveal = [0.0; 8];
            ffi::wlr_scene_node_set_enabled(self.border.tree as *mut ffi::wlr_scene_node, false);
        }

        if enabled {
            self.box_geom.x = requested.x;
            self.box_geom.y = requested.y;
            ffi::river_scene_node_set_position_if_changed(self.tree as *mut ffi::wlr_scene_node, self.box_geom.x, self.box_geom.y);
            ffi::river_scene_node_set_position_if_changed(self.popup_tree as *mut ffi::wlr_scene_node, self.box_geom.x, self.box_geom.y);

            // Blur stays on through a pan for every window. Non-cce windows
            // used to have their blur nodes DESTROYED on the first motion
            // frame and rebuilt 120ms after the gesture — a visible pop at
            // the end of every pan — on the theory that per-frame blur was
            // too expensive to keep during motion. Since the scene freezes
            // its optimized-blur caches for the duration of the motion
            // (river_scene_set_blur_frozen), a blurred window costs one
            // cached-texture sample per frame while moving, so the same
            // treatment cce apps always had now applies to all.
            // The geometry below is computed for EVERY window regardless: the drop
            // shadow has to track the zoom even where live blur does not (see the
            // update_shadow call at the end of the block).
            let app_id = self.get_app_id_string().unwrap_or_default();
            {
                let is_status = self.tiling_mode == crate::tiling::TilingMode::Status ||
                                app_id.starts_with("cce-status");
                let is_decorated = (*self.server).wm.is_decorated_app(&app_id);
                let blur_enabled = requested.blur && (self.wm_requested.ssd || is_decorated || is_status) && !self.droplet_backdrop_on();
                let mut ignore_transparent = (*self.server).wm.layout.window_backdrop_blur_ignore_transparent;
                if is_status {
                    ignore_transparent = (*self.server).wm.layout.status_backdrop_blur_ignore_transparent;
                }
                // Same radius/optimized reasoning as set_rendering_state. Before, this path
                // set no radius at all, so a blur node recreated during a pan came back
                // square and stayed that way.
                let radius = if self.is_fullscreen() {
                    0
                } else if requested.circular {
                    let w = self.rendering_sent.width as i32;
                    let h = self.rendering_sent.height as i32;
                    w.min(h) / 2
                } else if is_status {
                    // Same status exemption as set_rendering_state — the two
                    // paths drive the same nodes and must agree.
                    0
                } else if self.wm_requested.ssd || is_decorated {
                    (*self.server).wm.layout.root_plate_corner_radius
                } else {
                    0
                };
                // Rounded corners do NOT require live blur: the corner shape is applied by the
                // standard blur node's sampler (wlr_scene_blur_set_corner_radius) in both modes;
                // the optimized node only re-bakes the shared offscreen cache
                // (fx_render_pass_add_optimized_blur -> read_to_buffer) and never paints on
                // screen. The old `radius > 0` opt-out silently disabled the optimization for
                // every (rounded) window, forcing full-backdrop dual-kawase blur per frame per
                // translucent window — the DE-wide hover-lag / constant-GPU-load root cause.
                let use_optimized = if is_status {
                    false
                } else {
                    (*self.server).wm.layout.scenefx_optimized_blur
                };
                let toplevel_w = match self.impl_type {
                    WindowImpl::Toplevel(toplevel) => {
                        if toplevel.is_null() { 0 } else { (*toplevel).geometry.width }
                    }
                    _ => 0,
                };
                let toplevel_h = match self.impl_type {
                    WindowImpl::Toplevel(toplevel) => {
                        if toplevel.is_null() { 0 } else { (*toplevel).geometry.height }
                    }
                    _ => 0,
                };
                // Same self-sizing rule as set_rendering_state above.
                let (actual_w, actual_h) = if is_status && toplevel_w > 0 && toplevel_h > 0 {
                    (toplevel_w as u32, toplevel_h as u32)
                } else {
                    (
                        if self.rendering_sent.width > 0 { self.rendering_sent.width } else { toplevel_w as u32 },
                        if self.rendering_sent.height > 0 { self.rendering_sent.height } else { toplevel_h as u32 },
                    )
                };
                // Same span widening as set_rendering_state — the two paths
                // drive the same blur node and must agree.
                let radius = if requested.circular { radius } else { widen_corner_radius(radius, actual_w as i32, actual_h as i32) };
                let width = (actual_w as f64 * self.scale) as i32;
                let height = (actual_h as f64 * self.scale) as i32;
                ffi::river_scene_node_enable_blur(
                    self.tree as *mut ffi::wlr_scene_node,
                    blur_enabled,
                    use_optimized,
                    ignore_transparent,
                    0,
                    0,
                    width,
                    height,
                    (radius as f64 * self.scale) as i32,
                );
                // Every window, blurred or not: the shadow's size, blur sigma,
                // offset and — critically — the clipped region that punches the
                // window out of it are all scale-dependent, and nothing else on
                // the motion path touches them. Left stale they keep the scale
                // from before the gesture, so the punch-out overruns the shrunken
                // window and swallows the shadow whole.
                // The decorated-window predicate feeds two things: the shadow, and
                // the bevel's focus glint. Only the shadow honours the tiled switch.
                let want_decor = !is_status && (self.wm_requested.ssd || is_decorated) && !self.is_fullscreen();
                let want_shadow = want_decor && self.wants_tiled_shadow();
                // The bevel keys on its OWN app list, not on is_decorated:
                // every cce-ui app draws its own bevel, so a compositor one
                // would sit on top of it.
                let want_bevel = !is_status
                    && !self.is_fullscreen()
                    && (*self.server).wm.is_beveled_app(&app_id);
                self.update_shadow(width, height, radius, want_shadow);
                self.update_bevel(width, height, radius, want_bevel, want_decor);
                self.update_droplet(width, height);
            }

            self.scale_only_render_finish();
            self.draw_borders();
        }
    }

    /// The root plate / content-clip corner radius in logical px, before span
    /// widening. Single source for every writer of that radius: the two render
    /// paths clip the surface with it, and `draw_borders` shapes the root plate
    /// rect with it. Those disagreed — draw_borders applied the BORDER ring's
    /// radius to the root plate node and, running last, silently overrode the
    /// value set_rendering_state had just written, making
    /// `root_plate_corner_radius` dead config.
    pub unsafe fn root_plate_radius_base(&self) -> i32 {
        if self.is_fullscreen() {
            return 0;
        }
        if self.rendering_requested.circular {
            let w = self.rendering_sent.width as i32;
            let h = self.rendering_sent.height as i32;
            return w.min(h) / 2;
        }
        let app_id = self.get_app_id_string().unwrap_or_default();
        let is_status = self.tiling_mode == crate::tiling::TilingMode::Status
            || app_id.starts_with("cce-status");
        if is_status {
            return 0;
        }
        let is_decorated = (*self.server).wm.is_decorated_app(&app_id);
        if self.wm_requested.ssd || is_decorated {
            (*self.server).wm.layout.root_plate_corner_radius
        } else {
            0
        }
    }

    /// Sync the drop shadow with the current geometry. `width`/`height` are the
    /// content size in device px, `radius` the corner radius in logical px (as
    /// computed for the blur/rounding paths). scenefx's box-shadow shader draws
    /// the shadow of a box inset by sigma on all sides of the node box, so the
    /// node is padded by sigma and offset so the casting box lands exactly on
    /// the window, displaced by the configured offset — which should point away
    /// from the light (down-right for the DE's default top-left light). The
    /// window's own box is punched out via the clipped region so the shadow
    /// darkens only the desktop around the window, never the (translucent)
    /// window itself.
    pub unsafe fn update_shadow(&self, width: i32, height: i32, radius: i32, want: bool) {
        if self.shadow.is_null() {
            return;
        }
        let node = &mut (*self.shadow).node as *mut ffi::wlr_scene_node;
        let layout = &(*self.server).wm.layout;
        let enabled = want && layout.shadow_enabled && width > 0 && height > 0;
        ffi::wlr_scene_node_set_enabled(node, enabled);
        if !enabled {
            return;
        }
        let sigma = (layout.shadow_sigma as f64 * self.scale) as f32;
        let pad = sigma.ceil() as i32;
        let ox = (layout.shadow_offset_x as f64 * self.scale) as i32;
        let oy = (layout.shadow_offset_y as f64 * self.scale) as i32;
        let radius_dev = (radius as f64 * self.scale) as i32;
        ffi::wlr_scene_shadow_set_color(self.shadow, layout.shadow_color.as_ptr());
        ffi::wlr_scene_shadow_set_blur_sigma(self.shadow, sigma);
        ffi::wlr_scene_shadow_set_corner_radius(self.shadow, radius_dev);
        ffi::wlr_scene_shadow_set_size(self.shadow, width + 2 * pad, height + 2 * pad);
        ffi::river_scene_node_set_position_if_changed(node, -pad + ox, -pad + oy);
        let r = radius_dev.clamp(0, u16::MAX as i32) as u16;
        ffi::wlr_scene_shadow_set_clipped_region(self.shadow, ffi::clipped_region {
            area: ffi::wlr_box { x: pad - ox, y: pad - oy, width, height },
            corners: ffi::fx_corner_radii {
                top_left: r, top_right: r, bottom_right: r, bottom_left: r,
            },
        });
    }

    /// Sync the edge bevel with the current geometry. `width`/`height` are the
    /// content size in device px and `radius` the corner radius in logical px,
    /// exactly as `update_shadow` takes them. The rim is drawn INSIDE that box
    /// (see the shader), so it overlays the client's outermost pixels and needs
    /// no room of its own.
    ///
    /// The light direction is the DE's convention — the same top-left source
    /// the drop shadow is offset away from — so a window reads as a slab lit
    /// from the same place as everything else on the desktop.
    /// Is this window any seat's keyboard focus? The window's `activated`
    /// field is a configure-time snapshot, not live state, so live answers
    /// come from the seats.
    pub unsafe fn is_seat_focused(&self) -> bool {
        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            if let crate::seat::Focus::Window(w) = (*seat).focused {
                if w == self as *const Window as *mut Window {
                    return true;
                }
            }
            curr = (*curr).next;
        }
        false
    }

    pub unsafe fn update_bevel(&self, width: i32, height: i32, radius: i32, want: bool, want_focus: bool) {
        if self.bevel.is_null() {
            return;
        }
        let node = &mut (*self.bevel).node as *mut ffi::wlr_scene_node;
        let layout = &(*self.server).wm.layout;
        // Focused-window treatment: the rim highlight wraps all four sides
        // in the accent (the DE focus glint). Focus is read off the seats —
        // the window's `activated` field is a configure-time snapshot, not
        // live state — and this runs on both render paths, so a focus switch
        // restyles on the next frame. A focused window that is NOT in the
        // bevel app list still enables the node: the shader's focus branch
        // draws ONLY the glint, so it lays cleanly over a cce-ui app's own
        // client-side bevel instead of doubling its shading.
        let focused = self.is_seat_focused();
        let enabled = (want || (focused && want_focus))
            && layout.bevel_enabled
            && layout.bevel_thickness > 0.0
            && width > 0
            && height > 0;
        ffi::wlr_scene_node_set_enabled(node, enabled);
        if !enabled {
            return;
        }

        // Device px, like the blur radius and shadow sigma: the content is
        // scaled to its dest size, so an unscaled rim would keep its zoom-1
        // width while the window shrinks.
        let thickness = (layout.bevel_thickness as f64 * self.scale) as f32;
        let radius_dev = (radius as f64 * self.scale) as i32;

        // Light from the top-left, matching shadow_offset_x/y pointing away
        // from it. Normalized here so the shader can take it as-is.
        let (lx, ly) = (layout.bevel_light_x, layout.bevel_light_y);
        let len = (lx * lx + ly * ly).sqrt();
        let (lx, ly) = if len > 1e-6 { (lx / len, ly / len) } else { (-0.7071, -0.7071) };

        ffi::wlr_scene_bevel_set_size(self.bevel, width, height);
        ffi::wlr_scene_bevel_set_corner_radius(self.bevel, radius_dev);
        ffi::wlr_scene_bevel_set_thickness(self.bevel, thickness.max(1.0));
        ffi::wlr_scene_bevel_set_light(
            self.bevel,
            lx,
            ly,
            layout.bevel_light_intensity,
            layout.bevel_shade_intensity,
        );
        ffi::wlr_scene_bevel_set_shoulder(self.bevel, layout.bevel_shoulder);
        ffi::wlr_scene_bevel_set_color(self.bevel, layout.bevel_color.as_ptr());
        ffi::wlr_scene_bevel_set_focus(
            self.bevel,
            if focused { 1.0 } else { 0.0 },
            layout.bevel_focus_sharpness,
            layout.bevel_focus_color.as_ptr(),
        );
        ffi::river_scene_node_set_position_if_changed(node, 0, 0);
    }
    /// Sync the droplet backdrop-refraction node for a droplet-styled status
    /// segment. Called from BOTH render paths, like update_bevel — one-path
    /// effects freeze at the pre-gesture zoom (the shadow's old trap).
    pub unsafe fn update_droplet(&self, width: i32, height: i32) {
        if self.droplet.is_null() {
            return;
        }
        let node = &mut (*self.droplet).node as *mut ffi::wlr_scene_node;
        let layout = &(*self.server).wm.layout;
        let is_status = self.tiling_mode == crate::tiling::TilingMode::Status;
        // Only bar-strip segments: an expanded (menu) segment is taller than
        // the bar and draws its own grown drop client-side — refracting the
        // collapsed silhouette beneath it would be wrong.
        let enabled = is_status
            && layout.status_droplet.is_some()
            && width > 0
            && height > 0
            && height <= layout.bar_height as i32;
        if !enabled {
            ffi::wlr_scene_node_set_enabled(node, false);
            return;
        }
        let spec = cce_ui::scene::paint::DropletSpec::parse(
            layout.status_droplet.as_deref().unwrap_or(""),
        );
        if spec.refr <= 0.0 && spec.ghost <= 0.0 {
            ffi::wlr_scene_node_set_enabled(node, false);
            return;
        }
        ffi::wlr_scene_node_set_enabled(node, true);

        // Match the client's drop box: inset 1px from the surface bottom.
        // Camera-zoom scaling like the bevel; output scale is applied by the
        // render pass itself.
        let w = width as f32;
        let h = (height as f32 - 1.0).max(1.0);
        let (sr, ar, bow) = spec.resolve_silhouette(w, h);
        let k = (spec.blend.max(0.0) * h).max(1.0);
        let band = (spec.band.max(0.05) * h).max(1.0);
        let zs = self.scale as f32;
        ffi::wlr_scene_droplet_set_size(self.droplet, width, height);
        ffi::wlr_scene_droplet_set_silhouette(
            self.droplet,
            ar * zs,
            sr * zs,
            bow * zs,
            k * zs,
            spec.curve.clamp(2.0, 6.0),
        );
        ffi::wlr_scene_droplet_set_lens(self.droplet, band * zs, spec.refr * zs, spec.ghost.clamp(0.0, 1.0));
        ffi::river_scene_node_set_position_if_changed(node, 0, 0);
    }
    /// True when this status segment's droplet backdrop node is live. The
    /// per-window blur must yield to it: the blur pass would composite the
    /// UNREFRACTED cached backdrop over the lens output.
    pub unsafe fn droplet_backdrop_on(&self) -> bool {
        if self.droplet.is_null() || self.tiling_mode != crate::tiling::TilingMode::Status {
            return false;
        }
        match (*self.server).wm.layout.status_droplet.as_deref() {
            Some(raw) => {
                let spec = cce_ui::scene::paint::DropletSpec::parse(raw);
                spec.refr > 0.0 || spec.ghost > 0.0
            }
            None => false,
        }
    }



    /// Advance the hover fade one tick. Every zone eases toward 1.0 if it is
    /// the one under the pointer and 0.0 otherwise. Returns true while any
    /// zone is still in motion, so the caller knows to schedule another tick.
    pub unsafe fn step_border_fade(&mut self) -> bool {
        let mut moving = false;
        let mut changed = false;
        // In overview the FOCUSED window shows its whole ring for as long as
        // the mode is on; other windows show nothing. Hover-to-focus in the
        // motion path means the ring follows the pointer from window to
        // window, each swap easing through this same fade. Hover still reads
        // through on the focused ring, as `color_for` paints the hovered
        // zone in hover_color over the full reveal.
        let all_on = (*self.server).wm.mode == crate::window_manager::WindowManagerMode::Overview
            && window_takes_handles(self as *mut Window)
            && self.is_seat_focused();
        for elem in BorderElement::ALL {
            let i = elem.index();
            let target = if all_on || self.hovered_border_element == Some(elem) { 1.0 } else { 0.0 };
            let delta = target - self.border_reveal[i];
            if delta.abs() <= BORDER_FADE_EPSILON {
                if self.border_reveal[i] != target {
                    self.border_reveal[i] = target;
                    changed = true;
                }
                continue;
            }
            self.border_reveal[i] += delta * BORDER_FADE_STEP;
            moving = true;
            changed = true;
        }
        // A hover swap on a fully revealed ring moves nothing above, but the
        // shader still has to be told which zone to paint.
        if self.hovered_border_element != self.border_hover_drawn {
            changed = true;
        }
        if changed {
            self.draw_borders();
        }
        moving
    }

    /// The output a fullscreen window fills: the one the WM pinned it to, or
    /// the first enabled output (the same fallback manage/render use).
    pub unsafe fn fullscreen_output(&self) -> *mut crate::output::Output {
        if !self.wm_requested.fullscreen.is_null() {
            return self.wm_requested.fullscreen;
        }
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs_list).next;
        while curr != outputs_list {
            let out = crate::container_of!(curr, crate::output::Output, link);
            if (*out).sent.state == crate::output::OutputStateValue::Enabled {
                return out;
            }
            curr = (*curr).next;
        }
        std::ptr::null_mut()
    }

    /// Arms the fullscreen-toggle animation at the window's current on-screen
    /// rect. Called from manage_finish on the enter/exit transition, before
    /// the settled geometry is rewritten; a re-toggle mid-flight continues
    /// from wherever the previous animation had reached. Sized with
    /// last_applied_scale (the scale actually drawn) because self.scale has
    /// already been rewritten to the destination state's scale by the arrange
    /// pass in this same cycle.
    unsafe fn start_fs_anim(&mut self) {
        if !matches!(self.impl_type, WindowImpl::Toplevel(_))
            || !matches!(self.state, WindowState::Mapped)
            || self.box_geom.width <= 0
            || self.box_geom.height <= 0
        {
            return;
        }
        let (x, y, w, h) = if let Some(a) = self.fs_anim {
            (a.x, a.y, a.w, a.h)
        } else {
            let s = if self.last_applied_scale > 0.0 { self.last_applied_scale } else { 1.0 };
            (
                self.box_geom.x as f64,
                self.box_geom.y as f64,
                self.box_geom.width as f64 * s,
                self.box_geom.height as f64 * s,
            )
        };
        self.fs_anim = Some(FsAnim { x, y, w, h, moved: false, ticks: 0 });
        (*self.server).wm.arm_border_fade();
    }

    /// One tick of the fullscreen-toggle animation. Returns true while the
    /// caller should re-render (including the final settling frame). The
    /// target rect is recomputed live every tick — the output box while
    /// fullscreen, else the arranged position at the last configured size —
    /// so it tracks the client's asynchronous resize instead of freezing a
    /// stale goal on the first frame.
    pub unsafe fn step_fs_anim(&mut self) -> bool {
        let Some(mut anim) = self.fs_anim else {
            return false;
        };

        let (tx, ty, tw, th) = if self.is_fullscreen() {
            let output = self.fullscreen_output();
            if output.is_null() {
                self.fs_anim = None;
                return true;
            }
            let (w, h) = (*output).sent.dimensions();
            ((*output).sent.x as f64, (*output).sent.y as f64, w as f64, h as f64)
        } else {
            let w = self.configure_sent.width.map(|w| w as i32).unwrap_or(self.box_geom.width);
            let h = self.configure_sent.height.map(|h| h as i32).unwrap_or(self.box_geom.height);
            (
                self.rendering_requested.x as f64,
                self.rendering_requested.y as f64,
                w as f64 * self.scale,
                h as f64 * self.scale,
            )
        };

        anim.ticks += 1;
        let dx = tx - anim.x;
        let dy = ty - anim.y;
        let dw = tw - anim.w;
        let dh = th - anim.h;
        let settled = dx.abs() < FS_ANIM_EPSILON
            && dy.abs() < FS_ANIM_EPSILON
            && dw.abs() < FS_ANIM_EPSILON
            && dh.abs() < FS_ANIM_EPSILON;
        if !settled {
            anim.moved = true;
        }
        if (settled && anim.moved) || anim.ticks > FS_ANIM_MAX_TICKS {
            self.fs_anim = None;
            return true;
        }
        anim.x += dx * FS_ANIM_STEP;
        anim.y += dy * FS_ANIM_STEP;
        anim.w += dw * FS_ANIM_STEP;
        anim.h += dh * FS_ANIM_STEP;
        self.fs_anim = Some(anim);
        true
    }

    /// Outward extent (unscaled px) the interactive border may reach on each
    /// side — `[left, right, top, bottom]` — after the foam rule against the
    /// other windows: where two windows' bands would overlap across a gap,
    /// each band stops at the gap's midline (the ramp key-ring behavior,
    /// rectangular — the wall is equidistant from the two content edges).
    /// Stacked windows (content rects overlapping) do not clip each other,
    /// mirroring the rings' degenerate-distance guard. Per-side, not
    /// per-span: one near neighbor claims the whole facing side.
    pub unsafe fn border_side_extents(&self, band_unscaled: f64) -> [f64; 4] {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let band = band_unscaled * scale;
        let ax0 = self.box_geom.x as f64;
        let ay0 = self.box_geom.y as f64;
        let ax1 = ax0 + self.box_geom.width as f64 * scale;
        let ay1 = ay0 + self.box_geom.height as f64 * scale;
        let mut ext = [band; 4]; // left, right, top, bottom (layout px)

        let self_ptr = self as *const Window as *mut Window;
        for &other in (*self.server).wm.windows.iter() {
            if other.is_null() || other == self_ptr {
                continue;
            }
            let o = &*other;
            if o.closed
                || o.minimized
                || o.rendering_requested.hidden
                || o.rendering_requested.circular
                || matches!(
                    o.tiling_mode,
                    crate::tiling::TilingMode::Popup
                        | crate::tiling::TilingMode::Fullscreen
                        | crate::tiling::TilingMode::Status
                )
                || o.is_status_bar()
                || o.is_wallpaper()
            {
                continue;
            }
            let os = if o.scale > 0.0 { o.scale } else { 1.0 };
            let bx0 = o.box_geom.x as f64;
            let by0 = o.box_geom.y as f64;
            let bx1 = bx0 + o.box_geom.width as f64 * os;
            let by1 = by0 + o.box_geom.height as f64 * os;
            // Stacked: keep the full band.
            if bx0 < ax1 && bx1 > ax0 && by0 < ay1 && by1 > ay0 {
                continue;
            }
            let ob = border_band_width(o.rendering_requested.border.width) * os;
            // Spans (including bands) must overlap for a wall to exist.
            let v_overlap = by0 - ob < ay1 + band && by1 + ob > ay0 - band;
            let h_overlap = bx0 - ob < ax1 + band && bx1 + ob > ax0 - band;
            if v_overlap {
                if bx0 >= ax1 {
                    let gap = bx0 - ax1;
                    if gap < band + ob {
                        ext[1] = ext[1].min((gap / 2.0).max(0.0));
                    }
                } else if bx1 <= ax0 {
                    let gap = ax0 - bx1;
                    if gap < band + ob {
                        ext[0] = ext[0].min((gap / 2.0).max(0.0));
                    }
                }
            }
            if h_overlap {
                if by0 >= ay1 {
                    let gap = by0 - ay1;
                    if gap < band + ob {
                        ext[3] = ext[3].min((gap / 2.0).max(0.0));
                    }
                } else if by1 <= ay0 {
                    let gap = ay0 - by1;
                    if gap < band + ob {
                        ext[2] = ext[2].min((gap / 2.0).max(0.0));
                    }
                }
            }
        }
        [ext[0] / scale, ext[1] / scale, ext[2] / scale, ext[3] / scale]
    }

    }

/// Does this window get resize handles at all?
///
/// The single answer for both halves — `cursor::get_border_zone`'s hit test
/// and `draw_borders`' visuals — so a window can never show a handle it
/// would not honour, or honour one it does not show. Excluded: the internal
/// roles that are not user-geometry (Popup, Fullscreen, Status), Utility
/// (self-sizing by definition — the client owns its size), circular windows
/// (no rectangular ring to hug), and hidden ones.
pub unsafe fn window_takes_handles(window: *mut Window) -> bool {
    !matches!(
        (*window).tiling_mode,
        crate::tiling::TilingMode::Popup
            | crate::tiling::TilingMode::Fullscreen
            | crate::tiling::TilingMode::Status
            | crate::tiling::TilingMode::Utility
    ) && !(*window).rendering_requested.circular
        && !(*window).rendering_requested.hidden
}

impl Window {
    pub unsafe fn draw_borders(&mut self) {
        // Taken before `requested` borrows self: `window_takes_handles` is
        // the shared predicate with cursor::get_border_zone and must not be
        // duplicated here just to satisfy borrowck.
        let self_ptr = self as *mut Window;
        let requested = &self.rendering_requested;

        let border = &requested.border;
        let border_color = border.color;
        ffi::river_scene_node_set_position_if_changed(self.window_background as *mut ffi::wlr_scene_node, 0, 0);
        let bg_width = (self.box_geom.width as f64 * self.scale) as i32;
        let bg_height = (self.box_geom.height as f64 * self.scale) as i32;
        ffi::river_scene_rect_set_size_if_changed(self.window_background, bg_width, bg_height);
        ffi::wlr_scene_rect_set_color(self.window_background, border_color.as_ptr());
        // The background plate sits directly under the client's plate, so it
        // takes the ROOT_PLATE radius and the same span widening as the
        // blur/clip radius — not the border ring's radius, which is a
        // separate key describing a different edge.
        let bg_radius = widen_corner_radius(
            self.root_plate_radius_base(),
            self.box_geom.width,
            self.box_geom.height,
        );
        ffi::river_scene_rect_set_corner_radius(self.window_background, (bg_radius as f64 * self.scale) as i32);
        ffi::wlr_scene_node_set_enabled(self.window_background as *mut ffi::wlr_scene_node, !requested.hidden && self.wm_requested.ssd);

        // The border draws as 8 zone segments (4 edge bars + 4 two-rect L
        // corners) with BORDER_SEGMENT_GAP between them; the hovered zone
        // draws in hover_color. Underneath, the 4 full-band rects stay
        // enabled but transparent as scene hit-test catchers, so the pointer
        // never falls through the gaps (and width 0 keeps the legacy
        // invisible 8px virtual resize zones).
        //
        // Segments live in `border.tree`, parented to the global border
        // overlay layer rather than to this window's tree, so it has to be
        // positioned and enabled in step with the window by hand.
        let is_virtual_border = border.width == 0;
        // Deliberately NOT gated on `wm_requested.ssd`: that flag defaults to
        // false and is only set by a client calling use_ssd, and the segments
        // have never depended on it — only `window_background` does.
        let borders_visible = !requested.hidden
            && !requested.circular
            && !is_virtual_border
            && self.border_reveal.iter().any(|&a| a > 0.0);
        ffi::wlr_scene_node_set_enabled(self.border.tree as *mut ffi::wlr_scene_node, borders_visible);
        if borders_visible {
            ffi::river_scene_node_set_position_if_changed(
                self.border.tree as *mut ffi::wlr_scene_node,
                self.box_geom.x,
                self.box_geom.y,
            );
        }
        if requested.circular {
            ffi::wlr_scene_node_set_enabled(self.border.left as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.right as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.top as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.bottom as *mut ffi::wlr_scene_node, false);
            for &seg in self.border.segments.iter() {
                ffi::wlr_scene_node_set_enabled(seg as *mut ffi::wlr_scene_node, false);
            }
            return;
        }
        let content = ffi::wlr_box {
            x: 0,
            y: 0,
            width: self.box_geom.width,
            height: self.box_geom.height,
        };

        let mut intersect = std::mem::zeroed();
        let clip_empty = requested.content_clip.width == 0 && requested.content_clip.height == 0;
        if clip_empty || ffi::wlr_box_intersection(&mut intersect, &content, &requested.content_clip) {
            let border = &requested.border;
            // The interactive band (doubled configured width, floored). The
            // per-side foam clipping this used to carry is gone with the
            // outside band: it split a gap SHARED with a neighbouring window,
            // and the inside ring shares nothing.
            let band_f = border_band_width(border.width);
            let band = band_f as i32;
            let transparent = [0.0f32; 4];

            // The rounded-frame path used to leave radius/clip state on the
            // top band rect; keep it reset.
            ffi::river_scene_rect_set_corner_radius(self.border.top, 0);
            ffi::wlr_scene_rect_set_clipped_region(self.border.top, ffi::clipped_region_get_default());

            let apply = |rect: *mut ffi::wlr_scene_rect, bx: ffi::wlr_box, color: &[f32; 4], enabled: bool| {
                let mut bx = bx;
                if enabled && (requested.clip.width != 0 || requested.clip.height != 0) {
                    let mut clip_intersect = std::mem::zeroed();
                    ffi::wlr_box_intersection(&mut clip_intersect, &bx, &requested.clip);
                    bx = clip_intersect;
                }
                let enabled = enabled && bx.width > 0 && bx.height > 0;
                ffi::wlr_scene_node_set_enabled(rect as *mut ffi::wlr_scene_node, enabled);
                if !enabled {
                    return;
                }
                ffi::river_scene_node_set_position_if_changed(
                    rect as *mut ffi::wlr_scene_node,
                    (bx.x as f64 * self.scale) as i32,
                    (bx.y as f64 * self.scale) as i32,
                );
                ffi::river_scene_rect_set_size_if_changed(
                    rect,
                    (bx.width as f64 * self.scale) as i32,
                    (bx.height as f64 * self.scale) as i32,
                );
                ffi::wlr_scene_rect_set_color(rect, color.as_ptr());
            };

            // Handles live INSIDE the content rect, and only in overview
            // mode — see `cursor::get_border_zone`, which hit-tests the same
            // ring from the same band width and corner length. In normal
            // mode there is nothing to grab, so the catchers and the visible
            // segments are both disabled outright. The window's own border
            // (`window_background`, above) is untouched in either mode: this
            // moved the HANDLES inward, not the border.
            let in_overview = (*self.server).wm.mode
                == crate::window_manager::WindowManagerMode::Overview;
            let bw = band;
            let layout_handle_w = (*self.server).wm.layout.border_handle_width;
            let sc = if self.scale > 0.0 { self.scale } else { 1.0 };
            let (cw, ch) = (content.width, content.height);
            // A window thinner than two bands has no interior left for a
            // ring; drawing one would be a solid block over the whole window.
            let handles_on = in_overview
                && window_takes_handles(self_ptr)
                // Focused-only, like the reveal in step_border_fade: without
                // this the invisible catcher rects would keep intercepting
                // scene hits on windows whose ring is not even drawn.
                && self.is_seat_focused()
                && !is_virtual_border
                && bw > 0
                && (cw as f64 * sc) >= 12.0
                && (ch as f64 * sc) >= 12.0;
            if !handles_on {
                // Nothing is drawn, so nothing is stale: without this the
                // fade step would see a mismatch and repaint every tick.
                self.border_hover_drawn = self.hovered_border_element;
                for r in [self.border.left, self.border.right, self.border.top, self.border.bottom] {
                    ffi::wlr_scene_node_set_enabled(r as *mut ffi::wlr_scene_node, false);
                }
                for &seg in self.border.segments.iter() {
                    ffi::wlr_scene_node_set_enabled(seg as *mut ffi::wlr_scene_node, false);
                }
                ffi::wlr_scene_node_set_enabled(
                    &mut (*self.border.frame).node as *mut ffi::wlr_scene_node,
                    false,
                );
                return;
            }

            // The band is a SCREEN width, not a world one. Handles exist only
            // in overview, which is zoomed OUT, so a band that scaled with the
            // window would be at its thinnest exactly where it is the only way
            // to resize — 16px becomes 7 at a typical overview zoom, and the
            // thin corners 2.5. `apply` scales the boxes it is given, so the
            // catchers are sized in unscaled units that come back to
            // `band_screen` on screen. cursor::get_border_zone measures the
            // same width in layout px; the two must agree.
            // Screen thickness, but never more than a fifth of the smaller
            // on-screen side: a zoomed-out window would otherwise be mostly
            // ring. Shrinking beats the old hard cutoff, which dropped the
            // handles altogether below a threshold — a window you cannot
            // resize at all is worse than one with a slimmer grip.
            let short_side = (cw.min(ch) as f64 * sc).max(1.0);
            let band_screen = (layout_handle_w as f64)
                .max(crate::window::HOVER_BAND_MIN)
                .min(short_side * 0.2);
            let bw_u = (band_screen / sc).round().max(1.0) as i32;

            // Hit catchers: the inside ring, sides spanning the full height
            // so the corners belong to them. No foam clipping — that exists
            // to split a gap SHARED with a neighbouring window, and an inside
            // ring shares nothing.
            let b = ffi::wlr_box { x: 0, y: 0, width: bw_u, height: ch };
            apply(self.border.left, b, &transparent, true);
            let b = ffi::wlr_box { x: cw - bw_u, y: 0, width: bw_u, height: ch };
            apply(self.border.right, b, &transparent, true);
            let b = ffi::wlr_box { x: bw_u, y: 0, width: cw - 2 * bw_u, height: bw_u };
            apply(self.border.top, b, &transparent, true);
            let b = ffi::wlr_box { x: bw_u, y: ch - bw_u, width: cw - 2 * bw_u, height: bw_u };
            apply(self.border.bottom, b, &transparent, true);

            let layout = &(*self.server).wm.layout;
            // The ring hugs the window's own silhouette, so its outer arc IS
            // the window's content radius (the widened root plate radius the
            // corner clip uses) rather than that plus a band.
            let r_in = bg_radius;
            // corner_len and gap are retired by the wave profile (the
            // valleys place the seams now, a quarter along each side) and
            // ignored by the shader; still passed so the node API holds.
            let cl = border_corner_len(bw as f64, layout.border_corner_length, r_in as f64) as i32;
            let g = layout.border_segment_gap;

            // Handles rest invisible and fade in with the mode; one alpha for
            // the whole ring now that every zone reveals together in overview
            // (`step_border_fade`'s all_on branch). The Top slot carries it —
            // they are all equal while the ring is up, and taking one keeps
            // the fade a single number.
            let a = self.border_reveal[BorderElement::Top.index()].clamp(0.0, 1.0);
            let premul = |c: &[f32; 4]| [c[0] * a, c[1] * a, c[2] * a, c[3] * a];

            let px = |v: i32| (v as f64 * sc) as i32;
            ffi::wlr_scene_frame_set_size(self.border.frame, px(cw), px(ch));
            ffi::wlr_scene_frame_set_corner_radius(self.border.frame, px(r_in));
            // band is the hill height, band_min (taper × band) the valley
            // floor; bulge is the fillet radius that domes each corner hill.
            ffi::wlr_scene_frame_set_shape(
                self.border.frame,
                band_screen as f32,
                (band_screen as f32 * layout.border_taper.clamp(0.0, 1.0)).max(2.0),
                (px(cl) as f64).max(band_screen) as f32,
                px(g) as f32,
                layout.border_swell_curve,
                (layout.border_corner_bulge as f64).min(short_side * 0.3) as f32,
            );
            // The popover hint arrives in surface-local LOGICAL px; the node
            // space is zoom-scaled device px like everything else here, so it
            // takes the same px() mapping. Zeroed when clear.
            let ex = match self.popover_region {
                Some(r) => [
                    px(r.x) as f32,
                    px(r.y) as f32,
                    px(r.width) as f32,
                    px(r.height) as f32,
                ],
                None => [0.0; 4],
            };
            ffi::wlr_scene_frame_set_exclusion(self.border.frame, ex.as_ptr());
            ffi::wlr_scene_frame_set_color(self.border.frame, premul(&border_color).as_ptr());
            let hovered = self
                .hovered_border_element
                .map(|e| e.index() as f32)
                .unwrap_or(-1.0);
            self.border_hover_drawn = self.hovered_border_element;
            ffi::wlr_scene_frame_set_hover(
                self.border.frame,
                hovered,
                premul(&border.hover_color).as_ptr(),
            );
            ffi::river_scene_node_set_position_if_changed(
                &mut (*self.border.frame).node as *mut ffi::wlr_scene_node,
                0,
                0,
            );
            ffi::wlr_scene_node_set_enabled(
                &mut (*self.border.frame).node as *mut ffi::wlr_scene_node,
                a > 0.0,
            );
            // The rects the ring replaced.
            for &seg in self.border.segments.iter() {
                ffi::wlr_scene_node_set_enabled(seg as *mut ffi::wlr_scene_node, false);
            }
        }
    }

    #[allow(unused_assignments)]
    pub unsafe fn apply_surface_clip(&mut self, a: *const ffi::wlr_box, b: *const ffi::wlr_box) {
        let mut surface_clip = std::mem::zeroed::<ffi::wlr_box>();
        let a_empty = (*a).width == 0 && (*a).height == 0;
        let b_empty = (*b).width == 0 && (*b).height == 0;

        let layout_box = ffi::wlr_box {
            x: 0,
            y: 0,
            width: self.box_geom.width,
            height: self.box_geom.height,
        };

        if !a_empty && !b_empty {
            let mut temp_clip = std::mem::zeroed::<ffi::wlr_box>();
            if !ffi::wlr_box_intersection(&mut temp_clip, a, b) {
                self.surfaces.set_enabled(false);
                return;
            }
            if !ffi::wlr_box_intersection(&mut surface_clip, &temp_clip, &layout_box) {
                self.surfaces.set_enabled(false);
                return;
            }
        } else if !a_empty {
            if !ffi::wlr_box_intersection(&mut surface_clip, a, &layout_box) {
                self.surfaces.set_enabled(false);
                return;
            }
        } else if !b_empty {
            if !ffi::wlr_box_intersection(&mut surface_clip, b, &layout_box) {
                self.surfaces.set_enabled(false);
                return;
            }
        } else {
            surface_clip = layout_box;
        }

        self.surfaces.set_enabled(true);
        let margin = 0;
        surface_clip.x -= margin;
        surface_clip.y -= margin;
        surface_clip.width += 2 * margin;
        surface_clip.height += 2 * margin;

        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if !toplevel.is_null() {
                    let x = if self.wm_requested.ssd { 0 } else { (*toplevel).geometry.x };
                    let y = if self.wm_requested.ssd { 0 } else { (*toplevel).geometry.y };
                    surface_clip.x += x;
                    surface_clip.y += y;
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if !xwindow.is_null() {
                    let title_ptr = (*(*xwindow).xsurface).title;
                    let title = if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") };
                    if title.contains("Ubisoft") {
                        log::info!(
                            "XWayland window clip check: title='{}' box_geom=({}, {}, {}, {}) xsurface=({}, {}, {}, {})",
                            title,
                            self.box_geom.x,
                            self.box_geom.y,
                            self.box_geom.width,
                            self.box_geom.height,
                            (*(*xwindow).xsurface).x,
                            (*(*xwindow).xsurface).y,
                            (*(*xwindow).xsurface).width,
                            (*(*xwindow).xsurface).height,
                        );
                    }
                }
            }
            _ => {}
        }

        // Crop a CSD toplevel to its xdg geometry. Chromium-family clients
        // paint a translucent shadow band outside the geometry whenever they
        // are not maximized; the compositor draws its own shadow, and it
        // rounds corners per buffer at the buffer's edge, so uncropped the
        // rounding fell in that band and the visible window read
        // square-cornered (an Electron window un-tiled by a fullscreen round
        // trip). A geometry clip was set once and nulled in 34b3ae64: the
        // scaling passes rewrote every buffer's dest size from the full
        // surface each commit and stretched the crop back out — they go
        // through surface_buffer_extent now. Skipped while a cce-ui client
        // has a popover overhanging its geometry (set_popover_region): that
        // rim is live menu content, not a shadow. And skipped mid
        // fullscreen-toggle, where the animation owns the buffers' stretch.
        let mut crop = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
        if let WindowImpl::Toplevel(toplevel) = self.impl_type {
            if !toplevel.is_null()
                && !self.wm_requested.ssd
                && self.popover_region.is_none()
                && self.fs_anim.is_none()
            {
                crop = (*toplevel).geometry;
            }
        }
        let clip: *const ffi::wlr_box = if crop.width > 0 && crop.height > 0 {
            &crop
        } else {
            std::ptr::null()
        };
        let children_head = ffi::river_scene_tree_get_children(self.surfaces.tree) as *mut WlList;
        if (*children_head).next != children_head {
            ffi::wlr_scene_subsurface_tree_set_clip(self.surfaces.tree as *mut ffi::wlr_scene_node, clip);
        }
    }
}

unsafe fn clock_gettime(clk_id: libc::clockid_t, tp: &mut libc::timespec) -> libc::c_int {
    libc::clock_gettime(clk_id, tp)
}

unsafe extern "C" fn window_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn window_close(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.close = true;
}

unsafe extern "C" fn window_get_node(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    if !(*window).node.object.is_null() {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_NODE_EXISTS,
            b"window already has a node object\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).node.create_object(client, ffi::wl_resource_get_version(resource) as u32, id);
}

unsafe extern "C" fn window_propose_dimensions(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    width: i32,
    height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    if width < 0 || height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_DIMENSIONS,
            b"dimensions must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    if (*window).get_parent().is_null() {
        (*window).wm_requested.dimensions = Some(Dimensions {
            width: width as u32,
            height: height as u32,
        });
    }
}

unsafe extern "C" fn window_hide(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.hidden = true;
}

unsafe extern "C" fn window_show(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.hidden = false;
}

unsafe extern "C" fn window_use_csd(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.ssd = false;
    (*server).wm.dirty_windowing();
}

unsafe extern "C" fn window_use_ssd(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.ssd = true;
    (*server).wm.dirty_windowing();
}

unsafe extern "C" fn window_set_borders(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    edges: u32,
    width: i32,
    r: u32,
    g: u32,
    b: u32,
    a: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    if width < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_BORDER,
            b"border width must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    let alpha = (a as f64 / u32::MAX as f64) as f32;
    // Protocol channels are straight alpha; scene colors are premultiplied.
    let color = [
        (r as f64 / u32::MAX as f64) as f32 * alpha,
        (g as f64 / u32::MAX as f64) as f32 * alpha,
        (b as f64 / u32::MAX as f64) as f32 * alpha,
        alpha,
    ];
    (*window).rendering_requested.border = Border {
        edges: Edges::from_u32(edges),
        width: width as u32,
        color,
        // Protocol-set borders don't participate in hover highlighting.
        hover_color: color,
        corner_radius: 0,
    };
}

unsafe extern "C" fn window_set_tiled(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    edges: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.tiled = edges;
}

unsafe extern "C" fn window_get_decoration_above(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    wl_surface: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let wlr_surface = ffi::wlr_surface_from_resource(wl_surface);
    let decoration = match Decoration::create(
        client,
        ffi::wl_resource_get_version(resource) as u32,
        id,
        wlr_surface,
        (*window).decorations_above_tree,
        window,
    ) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to create decoration: {}", e);
            ffi::wl_client_post_no_memory(client);
            return;
        }
    };
    let list_head = &mut (*window).decorations_above as *mut ffi::wl_list as *mut WlList;
    wl_list_insert((*list_head).prev, &mut (*decoration).link as *mut ffi::wl_list as *mut WlList);
}

unsafe extern "C" fn window_get_decoration_below(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    wl_surface: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let wlr_surface = ffi::wlr_surface_from_resource(wl_surface);
    let decoration = match Decoration::create(
        client,
        ffi::wl_resource_get_version(resource) as u32,
        id,
        wlr_surface,
        (*window).decorations_below_tree,
        window,
    ) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to create decoration: {}", e);
            ffi::wl_client_post_no_memory(client);
            return;
        }
    };
    let list_head = &mut (*window).decorations_below as *mut ffi::wl_list as *mut WlList;
    wl_list_insert((*list_head).prev, &mut (*decoration).link as *mut ffi::wl_list as *mut WlList);
}

unsafe extern "C" fn window_inform_resize_start(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.resizing = true;
}

unsafe extern "C" fn window_inform_resize_end(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.resizing = false;
}

unsafe extern "C" fn window_set_capabilities(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    caps: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.capabilities = caps;
}

unsafe extern "C" fn window_inform_maximized(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.maximized = true;
}

unsafe extern "C" fn window_inform_unmaximized(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.maximized = false;
}

unsafe extern "C" fn window_inform_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.inform_fullscreen = true;
}

unsafe extern "C" fn window_inform_not_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.inform_fullscreen = false;
}

unsafe extern "C" fn window_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    output: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    let out = if output.is_null() {
        std::ptr::null_mut()
    } else {
        let wlr_output = ffi::wlr_output_from_resource(output);
        if wlr_output.is_null() {
            std::ptr::null_mut()
        } else {
            ffi::river_wlr_output_get_data(wlr_output) as *mut crate::output::Output
        }
    };
    (*window).wm_requested.fullscreen = out;
}

unsafe extern "C" fn window_exit_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.fullscreen = std::ptr::null_mut();
}

unsafe extern "C" fn window_set_clip_box(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    if width < 0 || height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_CLIP_BOX,
            b"width/height must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).rendering_requested.clip = ffi::wlr_box {
        x,
        y,
        width,
        height,
    };
}

unsafe extern "C" fn window_set_content_clip_box(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    if width < 0 || height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_CLIP_BOX,
            b"width/height must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).rendering_requested.content_clip = ffi::wlr_box {
        x,
        y,
        width,
        height,
    };
}

unsafe extern "C" fn window_set_dimension_bounds(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    max_width: i32,
    max_height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    if max_width < 0 || max_height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_DIMENSIONS,
            b"dimensions must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).wm_requested.bounds = Dimensions {
        width: max_width as u32,
        height: max_height as u32,
    };
}

unsafe extern "C" fn window_set_opacity(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    opacity: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    let opacity_f32 = opacity as f32 / u32::MAX as f32;
    (*window).rendering_requested.opacity = opacity_f32;
}

unsafe extern "C" fn window_set_circular(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    circular: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.circular = circular != 0;
}

unsafe extern "C" fn window_set_blur(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    blur: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.blur = blur != 0;
}

// zcce_window_v1 implementation
static WINDOW_INTERFACE: ffi::zcce_window_v1_interface = ffi::zcce_window_v1_interface {
    destroy: Some(window_destroy),
    close: Some(window_close),
    get_node: Some(window_get_node),
    propose_dimensions: Some(window_propose_dimensions),
    hide: Some(window_hide),
    show: Some(window_show),
    use_csd: Some(window_use_csd),
    use_ssd: Some(window_use_ssd),
    set_borders: Some(window_set_borders),
    set_tiled: Some(window_set_tiled),
    get_decoration_above: Some(window_get_decoration_above),
    get_decoration_below: Some(window_get_decoration_below),
    inform_resize_start: Some(window_inform_resize_start),
    inform_resize_end: Some(window_inform_resize_end),
    set_capabilities: Some(window_set_capabilities),
    inform_maximized: Some(window_inform_maximized),
    inform_unmaximized: Some(window_inform_unmaximized),
    inform_fullscreen: Some(window_inform_fullscreen),
    inform_not_fullscreen: Some(window_inform_not_fullscreen),
    fullscreen: Some(window_fullscreen),
    exit_fullscreen: Some(window_exit_fullscreen),
    set_clip_box: Some(window_set_clip_box),
    set_content_clip_box: Some(window_set_content_clip_box),
    set_dimension_bounds: Some(window_set_dimension_bounds),
    set_opacity: Some(window_set_opacity),
    set_circular: Some(window_set_circular),
    set_blur: Some(window_set_blur),
};

static INERT_WINDOW_INTERFACE: ffi::zcce_window_v1_interface = ffi::zcce_window_v1_interface {
    destroy: Some(window_destroy),
    close: None,
    get_node: None,
    propose_dimensions: None,
    hide: None,
    show: None,
    use_csd: None,
    use_ssd: None,
    set_borders: None,
    set_tiled: None,
    get_decoration_above: None,
    get_decoration_below: None,
    inform_resize_start: None,
    inform_resize_end: None,
    set_capabilities: None,
    inform_maximized: None,
    inform_unmaximized: None,
    inform_fullscreen: None,
    inform_not_fullscreen: None,
    fullscreen: None,
    exit_fullscreen: None,
    set_clip_box: None,
    set_content_clip_box: None,
    set_dimension_bounds: None,
    set_opacity: None,
    set_circular: None,
    set_blur: None,
};

unsafe extern "C" fn handle_destroy_resource(resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if !window.is_null() {
        if (*window).object != resource {
            return;
        }
        (*window).object = std::ptr::null_mut();
        (*window).node.make_inert();
        
        for decorations in [&mut (*window).decorations_above as *mut ffi::wl_list, &mut (*window).decorations_below as *mut ffi::wl_list] {
            let list_head = decorations as *mut WlList;
            let mut curr = (*list_head).next;
            while curr != list_head {
                let next = (*curr).next;
                let dec = crate::container_of!(curr, Decoration, link);
                (*dec).make_inert();
                curr = next;
            }
        }
    }
}

// zcce_decoration_v1 implementation
pub struct DecorationRenderingRequested {
    pub offset_x: i32,
    pub offset_y: i32,
    pub sync_next_commit: bool,
    pub blur: bool,
}

pub struct Decoration {
    pub object: *mut ffi::wl_resource, // zcce_decoration_v1
    pub surface: *mut ffi::wlr_surface,
    pub tree: *mut ffi::wlr_scene_tree,
    pub surfaces: crate::scene::SaveableSurfaces,
    pub link: ffi::wl_list,
    pub window: *mut Window,
    pub rendering_requested: DecorationRenderingRequested,
}

impl Decoration {
    pub unsafe fn create(
        client: *mut ffi::wl_client,
        version: u32,
        id: u32,
        surface: *mut ffi::wlr_surface,
        parent: *mut ffi::wlr_scene_tree,
        window: *mut Window,
    ) -> Result<*mut Self, &'static str> {
        let decoration_v1 = ffi::wl_resource_create(client, &ffi::zcce_decoration_v1_interface, version as i32, id);
        if decoration_v1.is_null() {
            ffi::wl_client_post_no_memory(client);
            return Err("wl_resource_create failed");
        }

        if !ffi::wlr_surface_set_role(
            surface,
            &raw const DECORATION_ROLE,
            decoration_v1,
            ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_ROLE,
        ) {
            return Err("wlr_surface_set_role failed");
        }
        ffi::river_wlr_surface_set_role_object(surface, decoration_v1);

        let tree = ffi::wlr_scene_tree_create(parent);
        if tree.is_null() {
            return Err("wlr_scene_tree_create failed");
        }

        let surfaces = crate::scene::SaveableSurfaces::init(tree)?;
        let subsurface_tree = ffi::wlr_scene_subsurface_tree_create(surfaces.tree, surface);
        if subsurface_tree.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            return Err("wlr_scene_subsurface_tree_create failed");
        }

        let dec = Box::new(Decoration {
            object: decoration_v1,
            surface,
            tree,
            surfaces,
            link: std::mem::zeroed(),
            window,
            rendering_requested: DecorationRenderingRequested {
                offset_x: 0,
                offset_y: 0,
                sync_next_commit: false,
                blur: false,
            },
        });
        let raw = Box::into_raw(dec);

        ffi::wl_resource_set_implementation(
            decoration_v1,
            &DECORATION_INTERFACE as *const _ as *const _,
            raw as *mut _,
            Some(handle_dec_destroy_resource),
        );

        Ok(raw)
    }

    pub unsafe fn destroy(&mut self) {
        assert!(self.object.is_null());
        ffi::wlr_scene_node_destroy(self.tree as *mut ffi::wlr_scene_node);
        wl_list_remove(&mut self.link as *mut ffi::wl_list as *mut WlList);
        let _ = Box::from_raw(self);
    }

    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_DECORATION_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
        }
        if !self.surface.is_null() {
            ffi::river_wlr_surface_set_role_object(self.surface, std::ptr::null_mut());
        }
        self.surfaces.save();
    }

    pub unsafe fn render_finish(&mut self, _window_clip: *const ffi::wlr_box) {
        if self.rendering_requested.sync_next_commit {
            self.rendering_requested.sync_next_commit = false;

            if !self.surfaces.saved {
                if !self.object.is_null() {
                    ffi::wl_resource_post_error(
                        self.object,
                        ffi::zcce_decoration_v1_error_ZCCE_DECORATION_V1_ERROR_NO_COMMIT,
                        b"no wl_surface.commit after sync_next_commit and before update_rendering_finish\0".as_ptr() as *const _,
                    );
                }
            }
        }

        self.surfaces.drop_saved();

        let server = (*self.window).server;
        let app_id = (*self.window).get_app_id_string().unwrap_or_default();
        let mut ignore_transparent = (*server).wm.layout.window_backdrop_blur_ignore_transparent;
        let is_status = (*self.window).tiling_mode == crate::tiling::TilingMode::Status ||
                        app_id.starts_with("cce-status");
        if is_status {
            ignore_transparent = (*server).wm.layout.status_backdrop_blur_ignore_transparent;
        }
        let is_decorated = (*server).wm.is_decorated_app(&app_id);
        let blur_enabled = self.rendering_requested.blur && ((*self.window).wm_requested.ssd || is_decorated || is_status) && !(*self.window).droplet_backdrop_on();
        // Radius 0 preserves existing behaviour on the layer-surface path (see layer_shell.rs)
        // — it never had a blur radius applied, and this fix is scoped to toplevels.
        ffi::river_scene_node_enable_blur(self.surfaces.tree as *mut ffi::wlr_scene_node, blur_enabled, (*server).wm.layout.scenefx_optimized_blur, ignore_transparent, 0, 0, 0, 0, 0);

        let scale = (*self.window).scale;
        let scaled_x = (self.rendering_requested.offset_x as f64 * scale) as i32;
        let scaled_y = (self.rendering_requested.offset_y as f64 * scale) as i32;
        ffi::river_scene_node_set_position_if_changed(self.tree as *mut ffi::wlr_scene_node, scaled_x, scaled_y);

        struct ScaleData {
            scale: f64,
            ancestor: *mut ffi::wlr_scene_node,
        }

        unsafe extern "C" fn set_overview_scale_iterator(
            buffer: *mut ffi::wlr_scene_buffer,
            sx: i32,
            sy: i32,
            user_data: *mut std::ffi::c_void,
        ) {
            let data = &*(user_data as *const ScaleData);
            let node = buffer as *mut ffi::wlr_scene_node;

            let surface = ffi::river_scene_node_get_surface(node);
            if !surface.is_null() {
                let w = ffi::river_wlr_surface_get_width(surface);
                let h = ffi::river_wlr_surface_get_height(surface);
                if data.scale == 1.0 {
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                    ffi::river_scene_node_set_position_if_changed(node, 0, 0);
                } else {
                    let dest_w = (w as f64 * data.scale) as i32;
                    let dest_h = (h as f64 * data.scale) as i32;
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                    let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                    let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                    let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                    ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                }
                // Keep the opaque region in step with the dest scale —
                // unscaled it covers the shrunken node's translucent CSD
                // margins and occlusion culling stops repainting behind
                // the client shadow (stale pixels show through it).
                ffi::river_scene_buffer_set_scaled_opaque_region(buffer, surface, data.scale);
            }
            // Non-surface buffers are frozen SAVED copies (see
            // save_surface_tree_iter): their natural buffer size is
            // meaningless for geometry — HiDPI clients commit scale-N
            // buffers and Chromium pads buffers beyond the surface,
            // cropping via viewport src — so rescaling from it ballooned
            // ghosts around the window at any zoom change. A frozen copy
            // keeps its save-time dest/position; a zoom mid-transaction
            // leaves it briefly at the old zoom, which restore corrects.
        }

        let scale_data = ScaleData { scale: scale * (*self.window).x11_buffer_scale(), ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.surfaces.tree as *mut ffi::wlr_scene_node,
            Some(set_overview_scale_iterator),
            &scale_data as *const ScaleData as *mut std::ffi::c_void,
        );

        if self.surfaces.saved {
            let scale_data_saved = ScaleData { scale: scale * (*self.window).x11_buffer_scale(), ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                Some(set_overview_scale_iterator),
                &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
            );
        }

        let children_head = ffi::river_scene_tree_get_children(self.surfaces.tree) as *mut WlList;
        if (*children_head).next != children_head {
            ffi::wlr_scene_subsurface_tree_set_clip(self.surfaces.tree as *mut ffi::wlr_scene_node, std::ptr::null());
        }
    }

    pub unsafe fn scale_only_render_finish(&mut self) {
        let scale = (*self.window).scale;
        if scale * (*self.window).x11_buffer_scale() == 1.0 {
            return;
        }

        struct ScaleData {
            scale: f64,
            ancestor: *mut ffi::wlr_scene_node,
        }

        unsafe extern "C" fn set_overview_scale_iterator(
            buffer: *mut ffi::wlr_scene_buffer,
            sx: i32,
            sy: i32,
            user_data: *mut std::ffi::c_void,
        ) {
            let data = &*(user_data as *const ScaleData);
            let node = buffer as *mut ffi::wlr_scene_node;

            let surface = ffi::river_scene_node_get_surface(node);
            if !surface.is_null() {
                let w = ffi::river_wlr_surface_get_width(surface);
                let h = ffi::river_wlr_surface_get_height(surface);
                if data.scale == 1.0 {
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                    ffi::river_scene_node_set_position_if_changed(node, 0, 0);
                } else {
                    let dest_w = (w as f64 * data.scale) as i32;
                    let dest_h = (h as f64 * data.scale) as i32;
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                    let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                    let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                    let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                    ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                }
                // Keep the opaque region in step with the dest scale —
                // unscaled it covers the shrunken node's translucent CSD
                // margins and occlusion culling stops repainting behind
                // the client shadow (stale pixels show through it).
                ffi::river_scene_buffer_set_scaled_opaque_region(buffer, surface, data.scale);
            }
            // Non-surface buffers are frozen SAVED copies (see
            // save_surface_tree_iter): their natural buffer size is
            // meaningless for geometry — HiDPI clients commit scale-N
            // buffers and Chromium pads buffers beyond the surface,
            // cropping via viewport src — so rescaling from it ballooned
            // ghosts around the window at any zoom change. A frozen copy
            // keeps its save-time dest/position; a zoom mid-transaction
            // leaves it briefly at the old zoom, which restore corrects.
        }

        let scale_data = ScaleData { scale: scale * (*self.window).x11_buffer_scale(), ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.surfaces.tree as *mut ffi::wlr_scene_node,
            Some(set_overview_scale_iterator),
            &scale_data as *const ScaleData as *mut std::ffi::c_void,
        );

        if self.surfaces.saved {
            let scale_data_saved = ScaleData { scale: scale * (*self.window).x11_buffer_scale(), ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                Some(set_overview_scale_iterator),
                &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
            );
        }
    }
}

pub unsafe fn decoration_from_wlr_surface(surface: *mut ffi::wlr_surface) -> *mut Decoration {
    if surface.is_null() {
        return std::ptr::null_mut();
    }
    let role_ptr = ffi::river_wlr_surface_get_role(surface);
    if role_ptr != &raw const DECORATION_ROLE {
        return std::ptr::null_mut();
    }
    let resource = ffi::river_wlr_surface_get_role_resource(surface);
    if resource.is_null() {
        return std::ptr::null_mut();
    }
    ffi::wl_resource_get_user_data(resource) as *mut Decoration
}

unsafe extern "C" fn dec_client_commit(surface: *mut ffi::wlr_surface) {
    let dec = decoration_from_wlr_surface(surface);
    if dec.is_null() {
        return;
    }
    if (*dec).rendering_requested.sync_next_commit {
        (*dec).surfaces.save();
    }
}

unsafe extern "C" fn dec_commit(surface: *mut ffi::wlr_surface) {
    if ffi::wlr_surface_has_buffer(surface) {
        ffi::wlr_surface_map(surface);
    }
}

unsafe extern "C" fn dec_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn dec_set_offset(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if dec.is_null() {
        return;
    }
    let server = (*(*dec).window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*dec).rendering_requested.offset_x = x;
    (*dec).rendering_requested.offset_y = y;
}

unsafe extern "C" fn dec_sync_next_commit(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if dec.is_null() {
        return;
    }
    let server = (*(*dec).window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*dec).rendering_requested.sync_next_commit = true;
}

unsafe extern "C" fn dec_set_blur(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    blur: u32,
) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if dec.is_null() {
        return;
    }
    let server = (*(*dec).window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*dec).rendering_requested.blur = blur != 0;
}

static DECORATION_INTERFACE: ffi::zcce_decoration_v1_interface = ffi::zcce_decoration_v1_interface {
    destroy: Some(dec_destroy),
    set_offset: Some(dec_set_offset),
    sync_next_commit: Some(dec_sync_next_commit),
    set_blur: Some(dec_set_blur),
};

static INERT_DECORATION_INTERFACE: ffi::zcce_decoration_v1_interface = ffi::zcce_decoration_v1_interface {
    destroy: Some(dec_destroy),
    set_offset: None,
    sync_next_commit: None,
    set_blur: None,
};

unsafe extern "C" fn handle_dec_destroy_resource(resource: *mut ffi::wl_resource) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if !dec.is_null() {
        ffi::river_wlr_surface_set_role_object((*dec).surface, std::ptr::null_mut());
        (*dec).object = std::ptr::null_mut();
        (*dec).destroy();
    }
}

unsafe extern "C" fn dec_role_destroy(surface: *mut ffi::wlr_surface) {
    let dec = decoration_from_wlr_surface(surface);
    if dec.is_null() {
        return;
    }
    ffi::river_wlr_surface_set_role_object(surface, std::ptr::null_mut());
    if !(*dec).object.is_null() {
        ffi::wl_resource_set_user_data((*dec).object, std::ptr::null_mut());
        ffi::wl_resource_destroy((*dec).object);
        (*dec).object = std::ptr::null_mut();
    }
    (*dec).destroy();
}

#[no_mangle]
pub static mut DECORATION_ROLE: ffi::wlr_surface_role = ffi::wlr_surface_role {
    name: b"zcce_decoration_v1\0".as_ptr() as *const _,
    no_object: false,
    client_commit: Some(dec_client_commit),
    commit: Some(dec_commit),
    map: None,
    unmap: None,
    destroy: Some(dec_role_destroy),
};

/// A surface buffer's visible extent for the scaling passes: `(width,
/// height, x, y)` — the subsurface clip when one is set (the xdg geometry,
/// see `apply_surface_clip`), placed where wlroots puts the cropped content
/// in its parent, else the whole surface at the origin. wlroots re-derives
/// dest size and position from the clip on every commit; a pass that
/// overrides them from the full surface size stretches the crop back out.
unsafe fn surface_buffer_extent(
    buffer: *mut ffi::wlr_scene_buffer,
    surface: *mut ffi::wlr_surface,
) -> (i32, i32, i32, i32) {
    let mut clip = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
    if ffi::river_scene_buffer_get_surface_clip(buffer, &mut clip) {
        (clip.width, clip.height, clip.x, clip.y)
    } else {
        (
            ffi::river_wlr_surface_get_width(surface),
            ffi::river_wlr_surface_get_height(surface),
            0,
            0,
        )
    }
}

unsafe fn get_parent_position_relative_to(
    node: *mut ffi::wlr_scene_node,
    ancestor: *mut ffi::wlr_scene_node,
) -> (i32, i32) {
    let mut x = 0;
    let mut y = 0;
    if !node.is_null() {
        let mut curr = ffi::river_scene_node_get_parent(node) as *mut ffi::wlr_scene_node;
        while !curr.is_null() && curr != ancestor {
            x += ffi::river_scene_node_get_x(curr);
            y += ffi::river_scene_node_get_y(curr);
            curr = ffi::river_scene_node_get_parent(curr) as *mut ffi::wlr_scene_node;
        }
    }
    (x, y)
}

unsafe fn wl_listener_remove_safe(listener: *mut ffi::wl_listener) {
    let prev = (*listener).link.prev;
    let next = (*listener).link.next;
    if !prev.is_null() && !next.is_null() && prev != listener as *mut ffi::wl_list && next != listener as *mut ffi::wl_list {
        ffi::wl_list_remove(&mut (*listener).link);
        (*listener).link.prev = std::ptr::null_mut();
        (*listener).link.next = std::ptr::null_mut();
    }
}

unsafe extern "C" fn handle_window_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let window = crate::container_of!(listener, Window, commit);
    (*window).stream_dirty = true;
    let was_status = (*window).is_status_bar();
    // An X11 client committing under a left/top-edge drag: anchor on the
    // size it just committed, ahead of the render_finish below that places
    // the tree at `rendering_requested`. (xdg toplevels do the same in their
    // own commit handler, where the toplevel geometry is the authority.)
    if let WindowImpl::Xwayland(xwindow) = (*window).impl_type {
        if !xwindow.is_null() && !(*xwindow).xsurface.is_null() && (*window).resize_edges.is_some() {
            let surface = (*(*xwindow).xsurface).surface;
            if !surface.is_null() {
                let mut w = ffi::river_wlr_surface_get_width(surface);
                let mut h = ffi::river_wlr_surface_get_height(surface);
                let has_parent = !(*(*xwindow).xsurface).parent.is_null();
                if (*window).is_wine() && !has_parent && !(*window).is_fullscreen() {
                    w = (w - 32).max(0);
                    h = (h - 32).max(0);
                }
                (*window).anchor_resize_commit(w, h);
            }
        }
    }
    (*window).render_finish();
    if was_status {
        (*(*window).server).wm.dirty_windowing();
    }
}
