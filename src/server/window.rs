// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlList, wl_list_insert, wl_list_remove_and_reinit};
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

impl From<Edges> for crate::policy::drag::Edges {
    fn from(e: Edges) -> Self {
        Self { top: e.top, bottom: e.bottom, left: e.left, right: e.right }
    }
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
}

impl Border {
    pub fn none() -> Self {
        Self { edges: Edges::new(), width: 0, color: [0.0; 4], hover_color: [0.0; 4] }
    }
}

/// Whether a window a title rule matches skips the saved-state restore: it
/// does when it opens over a sibling, and when the only entry on offer is
/// one the app_id-only pass would lend it from another window.
pub fn rule_skips_restore(has_sibling: bool, own_entry: bool) -> bool {
    has_sibling || !own_entry
}

/// Origin that centres a `size` window over `sibling` (x, y, w, h), then
/// slides it into `view` (x, y, w, h) on each axis it fits on — a sibling
/// lying half off screen must not take its settings window with it. All in
/// virtual units.
pub fn centered_over(sibling: (f64, f64, f64, f64), size: (f64, f64), view: (f64, f64, f64, f64)) -> (f64, f64) {
    let axis = |s0: f64, s_len: f64, len: f64, v0: f64, v_len: f64| {
        let c = s0 + (s_len - len) / 2.0;
        if len <= v_len { c.clamp(v0, v0 + v_len - len) } else { c }
    };
    (
        axis(sibling.0, sibling.2, size.0, view.0, view.2).round(),
        axis(sibling.1, sibling.3, size.1, view.1, view.3).round(),
    )
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

/// Number of handle discs: the eight resize zones and the three buttons.
pub const HANDLE_COUNT: usize = 11;

/// One of the interactive handle discs: the eight resize zones, then the
/// three window buttons beside the top-right disc. Each is its own disc
/// and highlights independently on hover.
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
    /// The window buttons (`window_takes_buttons`): a click, not a grab.
    Minimize,
    Maximize,
    ToggleTile,
}

impl BorderElement {
    /// Every zone, in `index()` order.
    pub const ALL: [BorderElement; HANDLE_COUNT] = [
        BorderElement::Top,
        BorderElement::Bottom,
        BorderElement::Left,
        BorderElement::Right,
        BorderElement::TopLeft,
        BorderElement::TopRight,
        BorderElement::BottomLeft,
        BorderElement::BottomRight,
        BorderElement::Minimize,
        BorderElement::Maximize,
        BorderElement::ToggleTile,
    ];

    /// A window button rather than a resize handle.
    pub fn is_button(self) -> bool {
        matches!(self, BorderElement::Minimize | BorderElement::Maximize | BorderElement::ToggleTile)
    }

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
            BorderElement::Minimize => 8,
            BorderElement::Maximize => 9,
            BorderElement::ToggleTile => 10,
        }
    }
}

/// Per-frame step of the hover fade, as a fraction of the remaining distance
/// to the target (the same exponential-approach shape the viewport pan uses).
pub const BORDER_FADE_STEP: f32 = 0.15;
/// Below this the fade is treated as finished and snapped to its target.
pub const BORDER_FADE_EPSILON: f32 = 0.004;

/// The hover/dim step in force: [`BORDER_FADE_STEP`], or the whole distance
/// when animations are off (`cce_core::motion`), which lands in one tick.
fn border_fade_step() -> f32 {
    if cce_core::motion::enabled() { BORDER_FADE_STEP } else { 1.0 }
}

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

/// Centre-to-centre spacing of the top row's discs when the window buttons
/// are shown, in disc diameters. The frame shader's `STEP`.
pub const HANDLE_BUTTON_STEP: f64 = 1.25;

/// Where the handle discs sit — one disc per zone, in
/// `BorderElement::index()` order — for a window whose content is `w`×`h`
/// ON SCREEN, with silhouette corner radius `r_in` and handle diameter `d`
/// (all screen px). Returns the centres, the disc radius and how many of
/// the discs are live: 8 (the resize handles), or all [`HANDLE_COUNT`]
/// when `buttons` asks for the window buttons and the top row has room for
/// all six of its discs.
///
/// Every disc sits the same distance in from the edges it touches, so the
/// three along an edge are inline: a corner disc sits on the corner's
/// diagonal, tangent to the rounded corner arc when that arc is wider than
/// the disc and tucked into the two straight edges otherwise, and the side
/// discs take that same inset. The buttons run leftward from the
/// top-right disc — minimize, maximize, float/tile toggle, then the corner
/// — and the Top disc leaves the midpoint only when it would crowd them.
///
/// The frame shader (scenefx `frame.frag`) lays out the same discs from the
/// same inputs; `draw_borders` (the catchers) and `cursor::get_border_zone`
/// (the hit test) both call this, so what is drawn is what grabs. Keep the
/// shader and this in step.
pub fn handle_disc_layout(
    w: f64,
    h: f64,
    r_in: f64,
    d: f64,
    buttons: bool,
) -> ([(f64, f64); HANDLE_COUNT], f64, usize) {
    let r = 0.5 * d;
    let t = if r_in > r { r_in - (r_in - r) / std::f64::consts::SQRT_2 } else { r };
    let s = HANDLE_BUTTON_STEP * d;
    let with_buttons = buttons && w >= 2.0 * t + 5.0 * s;
    let top_x = if with_buttons { (0.5 * w).min(w - t - 4.0 * s) } else { 0.5 * w };
    let centres = [
        (top_x, t),             // Top
        (0.5 * w, h - t),       // Bottom
        (t, 0.5 * h),           // Left
        (w - t, 0.5 * h),       // Right
        (t, t),                 // TopLeft
        (w - t, t),             // TopRight
        (t, h - t),             // BottomLeft
        (w - t, h - t),         // BottomRight
        (w - t - 3.0 * s, t),   // Minimize
        (w - t - 2.0 * s, t),   // Maximize
        (w - t - s, t),         // ToggleTile
    ];
    (centres, r, if with_buttons { HANDLE_COUNT } else { 8 })
}

pub struct BorderRects {
    /// The old full-band hit catchers. Retired by the disc handles — the
    /// pointer between two discs must reach the app, not a catcher — and
    /// kept disabled.
    pub left: *mut ffi::wlr_scene_rect,
    pub right: *mut ffi::wlr_scene_rect,
    pub top: *mut ffi::wlr_scene_rect,
    pub bottom: *mut ffi::wlr_scene_rect,
    /// Invisible square catchers, one per handle disc, indexed by
    /// `BorderElement::index()`: they make a scene hit on a disc resolve to
    /// this window even where the client's input region does not cover it.
    pub segments: [*mut ffi::wlr_scene_rect; HANDLE_COUNT],
    /// The handles: every disc, buttons included, in one shader-drawn node.
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

#[path = "window/placement.rs"]
mod placement;
#[path = "window/effects.rs"]
mod effects;
#[path = "window/borders.rs"]
mod borders;
pub use borders::*;
pub(crate) unsafe fn clock_gettime(clk_id: libc::clockid_t, tp: &mut libc::timespec) -> libc::c_int {
    libc::clock_gettime(clk_id, tp)
}

pub struct Window {
    pub ref_key: crate::slotmap::Key,
    pub server: *mut Server,
    pub node: WmNode,
    pub state: WindowState,
    pub impl_type: WindowImpl,
    /// Where a two-finger scroll over this window becomes an emulated
    /// view drag (see `cursor::ViewDrag`), when the app has said so through
    /// `touchpad-view-regions`: rectangles in surface-local pixels, `[x, y,
    /// w, h]`. `None` means the whole window, which is what an app that
    /// never sends any gets. Outside the rectangles the scroll reaches the
    /// client untouched — Houdini's parameter editor scrolls, its 3D
    /// viewports tumble.
    pub view_regions: Option<Vec<[f64; 4]>>,

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
    pub border_reveal: [f32; HANDLE_COUNT],
    /// How far this window is dimmed for lying OVER the adjust target, 0.0
    /// (full opacity) to 1.0 (`border.overlap_opacity`): a Floating window
    /// overlapping the window whose handles are up would hide them, so it
    /// eases down while the mode is on and back up when it ends. Stepped by
    /// `step_adjust_dim` on the border-fade timer; applied through
    /// `effective_opacity`.
    pub adjust_dim: f32,
    /// The map/close fade, 0.0 (invisible) to 1.0 (fully drawn). A window
    /// starts at 0 when it maps and eases to 1; a client that asks to close
    /// (`fade-out` on the control socket) eases it back to 0 and then exits.
    /// Applied through `effective_opacity`, so it MULTIPLIES the arrange
    /// pass's own opacity and the adjust-mode dim rather than fighting them.
    /// Stepped by `step_map_fade` on the border-fade timer.
    pub map_fade: f32,
    /// Where `map_fade` is easing to: 1.0 while the window lives, 0.0 once a
    /// close fade has been asked for.
    pub map_fade_target: f32,
    /// Linear per-tick step for `map_fade`, derived from the configured
    /// duration at the moment the fade starts. Linear, not the borders'
    /// exponential approach: an exponential close fade never actually
    /// reaches zero, and the client is waiting on a deadline to exit.
    pub map_fade_step: f32,
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
    /// Matched a `mode_rule` with `over_sibling` while a sibling was up: a
    /// settings-style window of a running app. Never restored from saved
    /// state, never saved, and centred over that sibling at map.
    pub satellite: bool,
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
    /// The last scale pass left the surface buffers at a dest size other
    /// than their natural one. Landing back on 1.0 has to undo that once —
    /// see `scale_only_render_finish`.
    pub buffers_scaled: bool,
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
    /// Surface size at the last commit of a status segment, so
    /// `handle_window_commit` re-arranges only when the segment actually
    /// changed size rather than on every content refresh.
    pub status_commit_size: (i32, i32),
    pub commit: crate::listener::Listener,
    pub was_fullscreen: bool,
    /// A fullscreen window drawn on the desk rather than pinned to its
    /// output: stepped aside, or sliding back in under the camera. Set by
    /// `WindowManager::place_fullscreen_windows`, read by the render pass.
    pub fs_on_desk: bool,
    /// The desk spot this window covered when its previous incarnation was
    /// last saved fullscreen (`SavedWindowState::fullscreen_at`), set by
    /// `try_restore` and spent by the first fullscreen enter, which lands
    /// there and brings the camera along instead of anchoring to the view.
    pub restore_fullscreen_at: Option<(f64, f64)>,
    /// The desk spot this window covered the last time it LEFT fullscreen,
    /// so `save_state` can still name one for a window closed windowed.
    pub last_fullscreen_at: Option<(f64, f64)>,
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
    /// The patch last issued was sized for a camera FLIGHT's destination —
    /// small enough for the client to render before the ramp lands, not the
    /// roomy cap-filling rect a resting camera wants for pan headroom. Once
    /// the camera is at rest with that patch latched, `update_grid_patches`
    /// re-issues the roomy one and clears this.
    pub grid_patch_flight: bool,
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

    /// A fullscreen window has stepped aside for another: a window focused
    /// more recently than it is still up (the switcher, `focus-window`, a
    /// focus chord). It stays fullscreen — the client keeps its size and
    /// mode — but the stacking pass drops it out of `layers.fullscreen` to
    /// behind every window, or the window just focused would be drawn under
    /// it. Focusing it again brings it back on top.
    ///
    /// Read from the focus history, not the seat's live focus: overlay UI
    /// (a launcher, the switcher itself) never enters the history, so
    /// opening one over the window you switched to does not pop the
    /// fullscreen one back over it. Only a desk window displaces it — not
    /// its own popups or dialogs, a status segment, or a window that has
    /// since been minimized or unmapped.
    ///
    /// Stepped aside, it is drawn on the desk at the spot it covered
    /// (`virtual_x/y`, kept in step with the camera while it is on top), so
    /// the camera pans away from it like any other window rather than
    /// leaving it fixed behind the screen.
    pub unsafe fn fullscreen_yields(&self) -> bool {
        let me = self as *const Window as *mut Window;
        for &w in (*self.server).wm.focus_history.iter() {
            if w == me {
                return false;
            }
            if w.is_null()
                || (*w).closed
                || (*w).minimized
                || !matches!((*w).state, WindowState::Mapped)
                || !matches!(
                    (*w).tiling_mode,
                    crate::tiling::TilingMode::Floating
                        | crate::tiling::TilingMode::Tiled
                        | crate::tiling::TilingMode::Utility
                        | crate::tiling::TilingMode::Fullscreen
                )
            {
                continue;
            }
            // A dialog of this window opens over it, fullscreen or not.
            let mut p = (*w).get_parent();
            let mut depth = 0;
            while !p.is_null() && p != me && depth < 16 {
                p = (*p).get_parent();
                depth += 1;
            }
            if p == me {
                continue;
            }
            return true;
        }
        false
    }

    /// The camera pan that puts this fullscreen window's desk spot exactly
    /// on its output — where focusing it pans back to, so a window that
    /// stepped aside slides in and lands pinned without a jump. `None`
    /// without an output to fill.
    pub unsafe fn fullscreen_anchor_pan(&self) -> Option<(f64, f64)> {
        let output = self.fullscreen_output();
        if output.is_null() {
            return None;
        }
        let zoom = (*self.server).wm.desk_zoom.max(0.01);
        let (first_x, first_y, _, _) = self.first_enabled_output_box();
        Some((
            self.virtual_x - ((*output).sent.x as f64 - first_x) / zoom,
            self.virtual_y - ((*output).sent.y as f64 - first_y) / zoom,
        ))
    }

    /// Eases the camera onto a fullscreen enter's restored desk spot, so the
    /// window rides the desk there (`place_fullscreen_windows` reads the
    /// target as `returning`) and pins on landing — the same slide a
    /// stepped-aside window takes back. Only for a window that will be on
    /// top: a stepped-aside one stays at its spot until it is focused, and
    /// that focus pans (`Seat::focus_follow_pan`). Not in overview or under
    /// a camera flight, where the window is a slab on the desk anyway and
    /// the camera is not the enter's to move.
    unsafe fn pan_to_restored_fullscreen_spot(&self) {
        let wm = &mut (*self.server).wm;
        if wm.mode == crate::window_manager::WindowManagerMode::Overview
            || wm.camera_ramp_anim.is_some()
            || self.fullscreen_yields()
        {
            return;
        }
        let Some((px, py)) = self.fullscreen_anchor_pan() else { return };
        if (wm.desk_pan_x - px).abs() >= 0.5 || (wm.desk_pan_y - py).abs() >= 0.5 {
            log::info!(
                "[Fullscreen] {:?} enters at its saved desk spot ({:.0}, {:.0}); panning there",
                self.get_title_string().as_deref().unwrap_or(""),
                self.virtual_x,
                self.virtual_y
            );
            wm.target_desk_pan_x = Some(px);
            wm.target_desk_pan_y = Some(py);
            wm.start_panning_animation();
        }
    }

    pub unsafe fn role(&self) -> crate::policy::api::WindowRole {
        if self.grid_declared {
            return crate::policy::api::WindowRole::Grid;
        }
        // Borrowed, not `get_app_id_string()`: this runs several times per
        // pointer-motion event (`is_status_bar`/`is_grid`/`is_wallpaper` in
        // the cursor passthrough) and per window per transaction, and each
        // call used to heap-allocate a String just to prefix-match it.
        let ptr = self.get_app_id();
        let app_id = if ptr.is_null() { None } else { std::ffi::CStr::from_ptr(ptr).to_str().ok() };
        crate::policy::api::WindowRole::from_app_id(app_id)
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
        let mut border_segments = [std::ptr::null_mut(); HANDLE_COUNT];
        for seg in border_segments.iter_mut() {
            *seg = ffi::wlr_scene_rect_create(border_tree, 0, 0, clear_color.as_ptr());
        }
        // The handles: one node draws every disc, the window buttons
        // included (scenefx frame.frag). Not rounded scene rects, because a
        // scene rect takes the renderer's global corner shape — a squircle —
        // so a rect with radius half its size would not be a circle. The
        // rects above are the discs' invisible hit catchers.
        let border_frame = ffi::wlr_scene_frame_create(border_tree, 0, 0, 0, clear_color.as_ptr());

        let window = Box::new(Window {
            view_regions: None,
            ref_key: crate::slotmap::Key { generation: 0, index: 0 },
            server,
            node: std::mem::zeroed(),
            state: WindowState::Init,
            impl_type,
            tree,
            fullscreen_background,
            window_background,
            shadow,
            bevel,
            droplet,
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
            border_reveal: [0.0; HANDLE_COUNT],
            adjust_dim: 0.0,
            // 1.0, not 0.0: a window only starts its fade in `map()`, and
            // one that never fades (fading disabled, a status segment) must
            // render at full strength from its first frame.
            map_fade: 1.0,
            map_fade_target: 1.0,
            map_fade_step: 1.0,
            popup_tree,
            capture_scene,
            capture_source: std::ptr::null_mut(),
            tiling_mode: crate::tiling::TilingMode::Floating,
            mode_locked: false,
            is_new: true,
            restored: false,
            hint_placed: false,
            pending_view_center: false,
            satellite: false,
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
            buffers_scaled: false,
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
            status_commit_size: (0, 0),
            commit: std::mem::zeroed(),
            was_fullscreen: false,
            fs_on_desk: false,
            restore_fullscreen_at: None,
            last_fullscreen_at: None,
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
            grid_patch_flight: false,
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


        let raw = Box::into_raw(window);
        let key = (*(*raw).server).wm.windows.put(raw);
        (*raw).ref_key = key;
        (*raw).node.init();

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

    /// The app_id borrowed, for hot paths that only compare it (the per-
    /// commit and per-transaction passes): `get_app_id_string` allocates.
    /// `None` when absent or not UTF-8.
    pub unsafe fn app_id_str(&self) -> Option<&str> {
        let ptr = self.get_app_id();
        if ptr.is_null() { None } else { std::ffi::CStr::from_ptr(ptr).to_str().ok() }
    }

    /// The title borrowed — see `app_id_str`.
    pub unsafe fn title_str(&self) -> Option<&str> {
        let ptr = self.get_title();
        if ptr.is_null() { None } else { std::ffi::CStr::from_ptr(ptr).to_str().ok() }
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
    /// `xwayland_window::x11_scale_for`); 1 for everything else, including
    /// an X11 window named in `xwayland_hidpi_except`.
    pub unsafe fn x11_buffer_scale(&self) -> f64 {
        if let WindowImpl::Xwayland(xwindow) = self.impl_type {
            let xsurface = if xwindow.is_null() { std::ptr::null() } else { (*xwindow).xsurface as *const _ };
            1.0 / crate::xwayland_window::x11_scale_for(self.server, xsurface) as f64
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

    /// A "shy" X11 window: a top-level that declines input focus
    /// (WM_HINTS input = False) and asks to be skipped by the taskbar —
    /// what Wine emits for a WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW window.
    /// Apps use those as helpers they place themselves: Ubisoft Connect
    /// keeps an untitled one exactly behind its borderless main window
    /// (the shadow-window trick), where Windows never shows it. Managed
    /// like an app window it was restored to a saved spot, pulled on-desk
    /// and raised — a blank white window with the app icon, over
    /// everything. So it is left to the client: no saved-state restore, its
    /// own position honoured at map and on request, never focused, never
    /// raised, stacked at the bottom.
    pub unsafe fn is_shy(&self) -> bool {
        let WindowImpl::Xwayland(xwindow) = self.impl_type else {
            return false;
        };
        if xwindow.is_null() || (*xwindow).xsurface.is_null() {
            return false;
        }
        let xs = (*xwindow).xsurface;
        if !(*xs).parent.is_null() || !(*xs).skip_taskbar || (*xs).hints.is_null() {
            return false;
        }
        let hints = (*xs).hints;
        let input_flag = ffi::xcb_icccm_wm_t_XCB_ICCCM_WM_HINT_INPUT as i32;
        (*hints).flags & input_flag != 0 && (*hints).input == 0
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
        self.try_center_on_sibling();
        // After every placement decision, including the invocation-square one:
        // whichever chose this spot, a tiled window must not open stacked on
        // another. The anchor rule already avoids that when any corner is
        // clear, so this only acts when none was.
        self.avoid_tiled_overlap();

        let surface = self.root_surface();
        if !surface.is_null() {
            self.commit.connect(ffi::river_wlr_surface_get_commit_signal(surface), handle_window_commit);
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
                    if let Ok(content) = std::fs::read_to_string(cce_core::config::get_config_path()) {
                        let val = cce_core::config::parse_kdl_to_json(&content);
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
                } else if let Ok(content) = std::fs::read_to_string(cce_core::config::get_config_path()) {
                    let val = cce_core::config::parse_kdl_to_json(&content);
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
                    let program = crate::window_manager::proc_args(self.unreliable_pid()).into_iter().next();
                    if (*self.server).wm.take_recent_vanish(&app_id, program.as_deref()) {
                        log::info!("[FocusRestore] Blocking focus steal by reconnecting client {:?} ({})", self.get_title(), app_id);
                        should_focus = false;
                    }
                }
            }

            // A WORLD window spawning during overview stays in overview:
            // the camera keeps its zoom and only pans, as little as it
            // must, to show the whole new window. Until 2026-10-05 it
            // flew out to zoom 1 on the window, so launching from the
            // overview left it. Chrome (Popup/Overlay), status, wallpaper
            // and the grid spawn without touching the camera. Here rather
            // than left to the focus loop's focus-follow pan, which skips
            // a first focus unless `center_on_spawn` allows it — the exit
            // this replaced always moved the camera.
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
                    (*self.server).wm.pan_overview_to_window(self as *mut Window);
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

        // The open dissolve. Last in `map`, so the window is fully placed and
        // its scene tree built before the ramp touches it — and so a window
        // that failed to map never starts one. `start_map_fade` snaps rather
        // than ramps when fading is off or this surface opts out (status
        // segments, wallpaper), so there is no second branch here.
        // Animations off (`cce_core::motion`) is a zero-length fade, the
        // same as `surface { fade in_ms=0 }`.
        let fade_ms = if cce_core::motion::enabled() { (*self.server).wm.layout.fade_in_ms } else { 0 };
        if self.wants_map_fade() && fade_ms > 0 {
            self.map_fade = 0.0;
        }
        self.start_map_fade(1.0, fade_ms);

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
                let program = crate::window_manager::proc_args(self.unreliable_pid()).into_iter().next();
                (*self.server).wm.note_vanished(app_id, program);
            }
        }
        self.commit.disconnect();
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

        (*window).commit.disconnect();
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
        (*(*window).server).wm.selection_forget(window);
        // A seat cursor may still name this window as its adjust target.
        // The next hover evaluation would replace it, but a window allocated
        // at the same address in the meantime must not inherit the ring.
        {
            let seats = &mut (*(*window).server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*seats).next;
            while curr != seats {
                let seat = crate::container_of!(curr, crate::seat::Seat, link);
                if (*seat).cursor.adjust_hover == window {
                    (*seat).cursor.adjust_hover = std::ptr::null_mut();
                }
                (*seat).group_move.retain(|&(w, _, _)| w != window);
                curr = (*curr).next;
            }
        }
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

    /// Record the size this window is about to have, and ask for a render
    /// pass only when it differs from the one the last pass applied
    /// (`rendering_sent`). A pass is not needed just because a window is
    /// linked: the transaction that links it renders it. (A
    /// `resend_dimensions` flag used to force one on every call; only the
    /// external manager's dimensions event ever cleared it, so from a
    /// window's first link on, every unchanged X11 configure request and
    /// every resize-drag motion that left the size alone cost a render pass.)
    /// `#[track_caller]` so `CCE_DIRTY_TRACE` names the caller.
    #[track_caller]
    pub unsafe fn set_dimensions(&mut self, width: u32, height: u32) {
        self.rendering_scheduled.width = width;
        self.rendering_scheduled.height = height;

        if self.rendering_scheduled.width != self.rendering_sent.width ||
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
            }
            WindowState::Ready | WindowState::Initialized | WindowState::Mapped => {
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
                    // The tail is the top of the stack. A shy helper
                    // window (`is_shy`) links at the head instead — beneath
                    // the app's own windows, where its app keeps it.
                    let anchor = if self.is_shy() { rendering_list } else { (*rendering_list).prev };
                    wl_list_insert(anchor, &mut self.node.link as *mut ffi::wl_list as *mut WlList);

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
                }
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
                // From here on virtual_x/y is the desk spot the window covers
                // — where the camera is now — so stepping aside leaves it
                // there (`WindowManager::place_fullscreen_windows`, which
                // keeps it in step while the window is on top).
                //
                // Unless a previous incarnation was saved fullscreen
                // somewhere (`try_restore`): then the spot is that one, and
                // the camera goes to it. Trackmania reopened wherever the
                // user happened to be looking, because the enter always
                // took the view and only the pre-fullscreen spot was saved.
                let restored_spot = self.restore_fullscreen_at.take();
                let (vx, vy) = restored_spot
                    .unwrap_or_else(|| self.screen_to_virtual((*output).sent.x, (*output).sent.y));
                self.virtual_x = vx;
                self.virtual_y = vy;
                if restored_spot.is_some() {
                    self.pan_to_restored_fullscreen_spot();
                }
                log::info!("[Fullscreen] Saved window {:?} geometry: {}x{} at ({}, {})", self.get_title_string().as_deref().unwrap_or(""), self.saved_width, self.saved_height, self.saved_virtual_x, self.saved_virtual_y);
            }
        } else if !new_fullscreen && self.was_fullscreen {
            if self.saved_width > 0 && self.saved_height > 0 {
                // Captures the on-screen fullscreen rect before the restore
                // below rewrites box_geom.
                self.start_fs_anim();
                self.last_fullscreen_at = Some((self.virtual_x, self.virtual_y));
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
                (Some(w as u32), Some(h as u32))
            } else {
                (None, None)
            }
        } else if let Some(dimensions) = self.wm_requested.dimensions {
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
                    // The client's committed geometry is the authority on
                    // this window's size — but only once the client has
                    // ANSWERED a configure. Before its first ack, `geometry`
                    // holds the size the client asked for on its own:
                    // Chromium restores its remembered bounds with
                    // `set_window_geometry` before it ever acks, and those
                    // bounds are its window PLUS its CSD shadow insets, so
                    // they always overhang the cell block the restore just
                    // gave it. Adopting that wish made it `box_geom` (see
                    // `render_finish`), the next Tiled arrange covered every
                    // cell the overhang touched (`snap::tiled_span` floors the
                    // low edge and CEILS the high one), the grown size was
                    // saved, and Chrome came back a whole cell wider and
                    // taller on every login — a one-way ratchet, since each
                    // session's insets sit on top of the last session's block.
                    // A window with no restored size still seeds its block
                    // from the client's first wish, which is where a freshly
                    // launched app's size comes from.
                    if !self.restored || (*toplevel).acked_once {
                        self.rendering_scheduled.width = (*toplevel).geometry.width as u32;
                        self.rendering_scheduled.height = (*toplevel).geometry.height as u32;
                    }
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if !xwindow.is_null() {
                    let s = crate::xwayland_window::x11_scale_for(self.server, (*xwindow).xsurface);
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

        sent.width = scheduled.width;
        sent.height = scheduled.height;
        sent.presentation_hint = presentation_hint;
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
        // A title is arrangement input only through a mode rule that matches
        // on it (`title=` in a rule); the built-in policy is what runs, so
        // nothing else in the manage sequence reads it. A
        // terminal running a busy program retitles several times a second,
        // and each retitle used to cost a full manage/arrange/render pass.
        // Without a title rule the title's other consumers are the status
        // bar's `title` topic and the saved-state file, so feed those directly.
        let wm = &mut (*self.server).wm;
        if wm.mode_rules.iter().any(|r| r.title_pattern.is_some()) {
            wm.dirty_windowing();
        } else {
            wm.update_status();
            wm.schedule_save_state();
        }

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

        ffi::wlr_scene_node_set_enabled(self.tree as *mut ffi::wlr_scene_node, enabled);
        ffi::wlr_scene_node_set_enabled(self.popup_tree as *mut ffi::wlr_scene_node, enabled);
        if !enabled {
            // The segment tree is not a child of `tree`, so disabling the
            // window does not hide a revealed border with it.
            self.border_reveal = [0.0; HANDLE_COUNT];
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
            // below. ONE source — `root_plate_radius_base` — for this path,
            // `render_viewport_update`, the toplevel commit path and
            // `draw_borders`: until 2026-09-28 each carried its own copy of
            // the fullscreen / circular / status / decorated decision.
            let radius = self.root_plate_radius_base();
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
            let width = (actual_w as f64 * scale_x).round() as i32;
            let height = (actual_h as f64 * scale_y).round() as i32;
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
                self.sync_backdrop_compress();
            ffi::river_scene_node_set_opacity(self.tree as *mut ffi::wlr_scene_node, self.effective_opacity());

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
                /// The grid's buffers are pinned (see
                /// `wlr_scene_buffer_set_geometry_pinned`). Its surface is
                /// always shown scaled and covers the screen, so the scene
                /// helper resetting its dest size and opaque region on each
                /// commit, and this pass putting them back, repainted the
                /// whole output for every frame of an image being dragged.
                pin: bool,
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
                    if data.pin {
                        ffi::river_scene_buffer_set_geometry_pinned(buffer, true);
                    }
                    let (w, h, ox, oy) = surface_buffer_extent(buffer, surface);
                    if data.scale_x == 1.0 && data.scale_y == 1.0 {
                        ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                        ffi::river_scene_node_set_position_if_changed(node, ox, oy);
                    } else {
                        let dest_w = (w as f64 * data.scale_x).round() as i32;
                        let dest_h = (h as f64 * data.scale_y).round() as i32;
                        ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                        // The parent offset scales like the content; the
                        // clip origin rides on top of it, scaled the same.
                        let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                        let dest_x = (px as f64 * (data.scale_x - 1.0) + ox as f64 * data.scale_x).round() as i32;
                        let dest_y = (py as f64 * (data.scale_y - 1.0) + oy as f64 * data.scale_y).round() as i32;
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

            let scale_data_surfaces = ScaleData { scale_x, scale_y, ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node, pin: self.is_grid() };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.tree as *mut ffi::wlr_scene_node,
                Some(set_overview_scale_iterator),
                &scale_data_surfaces as *const ScaleData as *mut std::ffi::c_void,
            );

            if self.surfaces.saved {
                let scale_data_saved = ScaleData { scale_x, scale_y, ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node, pin: self.is_grid() };
                ffi::wlr_scene_node_for_each_buffer(
                    self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                    Some(set_overview_scale_iterator),
                    &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
                );
            }
            
            let scale_data_popup = ScaleData { scale_x, scale_y, ancestor: self.popup_tree as *mut ffi::wlr_scene_node, pin: self.is_grid() };
            ffi::wlr_scene_node_for_each_buffer(
                self.popup_tree as *mut ffi::wlr_scene_node,
                Some(set_overview_scale_iterator),
                &scale_data_popup as *const ScaleData as *mut std::ffi::c_void,
            );
            self.last_applied_scale = self.scale;
            self.buffers_scaled = scale_x != 1.0 || scale_y != 1.0;
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
            if self.fs_on_desk {
                // Stepped aside: at its desk spot, which the arrange pass
                // put in rendering_requested (`place_fullscreen_windows`).
                self.box_geom.x = requested.x;
                self.box_geom.y = requested.y;
            } else {
                self.box_geom.x = (*output).sent.x;
                self.box_geom.y = (*output).sent.y;
            }

            let app_id_ptr = self.get_app_id();
            let (is_status_bar, is_wallpaper) = if !app_id_ptr.is_null() {
                let app_id = std::ffi::CStr::from_ptr(app_id_ptr).to_string_lossy();
                (app_id.starts_with("cce-status"), app_id.as_ref() == "cce-wallpaper")
            } else {
                (false, false)
            };

            ffi::wlr_scene_node_set_enabled(self.fullscreen_background as *mut ffi::wlr_scene_node, !is_status_bar && !is_wallpaper);
            let (width, height) = (*output).sent.dimensions();
            self.size_fullscreen_background(width as i32, height as i32);
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
            self.border_reveal = [0.0; HANDLE_COUNT];
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
            self.border_reveal = [0.0; HANDLE_COUNT];
            ffi::wlr_scene_node_set_enabled(self.border.tree as *mut ffi::wlr_scene_node, false);
        }

        // No geometry compensation here: wlr_scene_xdg_surface_create already
        // anchors its subtree at the top-left of the xdg window geometry (it
        // re-offsets by -geometry on every commit), so subtracting geometry.x/y
        // again shifted CSD windows with shadow margins (Electron/Chromium
        // floating) up-left by their shadow size, off the desktop grid.
        ffi::river_scene_node_set_position_if_changed(self.surfaces.tree as *mut ffi::wlr_scene_node, 0, 0);

        self.apply_surface_clip(&clip, &content_clip);

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
        // At 1.0 there is nothing to apply — unless the previous pass left
        // the buffers shrunk. An overview exit's landing frame runs on the
        // viewport path (`render_viewport_update` -> here), not through
        // `render_finish`, so returning early there left every window drawn
        // at the ramp's second-to-last zoom (~98%) until the 120ms viewport
        // settle, or its own next commit, popped it to full size: the
        // windows visibly "settled" a beat after the animation ended.
        if eff_scale == 1.0 {
            self.last_applied_scale = 1.0;
            if !self.buffers_scaled {
                return;
            }
        }

        // No last_applied_scale short-circuit here: wlroots' scene-surface
        // commit listener resets a committed buffer's dest size and opaque
        // region to the surface's natural extent, so any client repainting
        // while scaled (browser animations, caret blink) pops back to full
        // size even though the cached scale says nothing changed. This runs
        // per rendered frame (output.rs render_and_commit), after commits and
        // before build_state, and every setter below is change-checked — an
        // already-correct tree produces no damage.
        if eff_scale != 1.0 {
            self.last_applied_scale = self.scale;
        }
        self.buffers_scaled = eff_scale != 1.0;

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
                    let dest_w = (w as f64 * data.scale).round() as i32;
                    let dest_h = (h as f64 * data.scale).round() as i32;
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                    let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                    let dest_x = (px as f64 * (data.scale - 1.0) + ox as f64 * data.scale).round() as i32;
                    let dest_y = (py as f64 * (data.scale - 1.0) + oy as f64 * data.scale).round() as i32;
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
    }

    pub unsafe fn render_viewport_update(&mut self) {
        let requested = &self.rendering_requested;
        let enabled = !requested.hidden && (matches!(self.state, WindowState::Mapped) || matches!(self.state, WindowState::Closing));

        ffi::wlr_scene_node_set_enabled(self.tree as *mut ffi::wlr_scene_node, enabled);
        ffi::wlr_scene_node_set_enabled(self.popup_tree as *mut ffi::wlr_scene_node, enabled);
        if !enabled {
            self.border_reveal = [0.0; HANDLE_COUNT];
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
                // Same radius/optimized reasoning as set_rendering_state — the
                // one `root_plate_radius_base`, so the two paths, which drive
                // the same nodes, cannot disagree. Before, this path set no
                // radius at all, so a blur node recreated during a pan came
                // back square and stayed that way.
                let radius = self.root_plate_radius_base();
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
                self.sync_backdrop_compress();
            }

            if self.fs_on_desk && self.fs_anim.is_none() {
                let output = self.fullscreen_output();
                if !output.is_null() {
                    let (w, h) = (*output).sent.dimensions();
                    self.size_fullscreen_background(w as i32, h as i32);
                }
            }

            self.scale_only_render_finish();
            self.draw_borders();
        }
    }
}

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

unsafe extern "C" fn handle_window_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let window = crate::container_of!(listener, Window, commit);
    (*window).stream_dirty = true;
    let was_status = (*window).is_status_bar();
    // An X11 client committing under a left/top-edge drag: anchor on the
    // size it just committed, ahead of the render_finish below that places
    // the tree at `rendering_requested`. (xdg toplevels do the same in their
    // own commit handler, where the toplevel geometry is the authority.)
    //
    // The committed surface is PHYSICAL pixels — under `xwayland_hidpi` twice
    // the logical box, as the scale pass below says — while
    // `anchor_resize_commit` works in box_geom's logical units. Convert first,
    // and take the wine frame off after, the way `render_finish` reads the
    // xsurface size back. Feeding it the raw buffer width put the origin a
    // whole window-width to the left and made it track the pointer at double
    // speed, every X11 left/top drag at scale 2.
    if let WindowImpl::Xwayland(xwindow) = (*window).impl_type {
        if !xwindow.is_null() && !(*xwindow).xsurface.is_null() && (*window).resize_edges.is_some() {
            let surface = (*(*xwindow).xsurface).surface;
            if !surface.is_null() {
                let s = crate::xwayland_window::x11_scale_for((*window).server, (*xwindow).xsurface);
                let mut w = crate::xwayland_window::from_x11(
                    ffi::river_wlr_surface_get_width(surface), s);
                let mut h = crate::xwayland_window::from_x11(
                    ffi::river_wlr_surface_get_height(surface), s);
                let has_parent = !(*(*xwindow).xsurface).parent.is_null();
                if (*window).is_wine() && !has_parent && !(*window).is_fullscreen() {
                    w = (w - crate::xwayland_window::WINE_MARGIN * 2).max(0);
                    h = (h - crate::xwayland_window::WINE_MARGIN * 2).max(0);
                }
                (*window).anchor_resize_commit(w, h);
            }
        }
    }
    (*window).render_finish();
    // The scene's own commit handler (registered before this one, so it has
    // already run) resets the committed buffer's dest size to natural. For
    // an X11 surface under xwayland_hidpi that is the physical size — twice
    // the logical box — and until the per-frame pass restores it every
    // pointer event hit-tests through the unscaled buffer and reaches the
    // client at HALF its coordinates. Houdini repaints on every hover
    // change, so hover flickered: each repaint opened the gap, the next
    // motion event landed elsewhere, the widget un-hovered, repeat.
    if (*window).x11_buffer_scale() != 1.0 {
        (*window).scale_only_render_finish();
    }
    // A status segment that changed size needs the bar re-arranged around
    // it. One that merely repainted (the clock, once a second; the cpu
    // meter) does not — and this used to dirty on every commit, which made
    // the status bar alone run a full manage/arrange/render transaction for
    // each of its ticks, all day. Compare the committed surface size against
    // the last commit's; the xdg commit handler tracks box_geom the same way.
    if was_status {
        let surface = (*window).root_surface();
        if !surface.is_null() {
            let size = (
                ffi::river_wlr_surface_get_width(surface),
                ffi::river_wlr_surface_get_height(surface),
            );
            if size != (*window).status_commit_size {
                (*window).status_commit_size = size;
                (*(*window).server).wm.dirty_windowing();
            }
        } else {
            (*(*window).server).wm.dirty_windowing();
        }
    }
}

#[cfg(test)]
mod handle_disc_tests {
    use super::*;

    #[test]
    fn discs_follow_border_element_order_and_stay_inside() {
        let (c, r, n) = handle_disc_layout(400.0, 300.0, 0.0, 32.0, false);
        assert_eq!(r, 16.0);
        assert_eq!(n, 8);
        assert_eq!(c[BorderElement::Top.index()], (200.0, 16.0));
        assert_eq!(c[BorderElement::Bottom.index()], (200.0, 284.0));
        assert_eq!(c[BorderElement::Left.index()], (16.0, 150.0));
        assert_eq!(c[BorderElement::Right.index()], (384.0, 150.0));
        assert_eq!(c[BorderElement::TopLeft.index()], (16.0, 16.0));
        assert_eq!(c[BorderElement::BottomRight.index()], (384.0, 284.0));
        for &(x, y) in &c[..n] {
            assert!(x - r >= 0.0 && x + r <= 400.0 && y - r >= 0.0 && y + r <= 300.0);
        }
    }

    #[test]
    fn corner_disc_is_tangent_to_a_wider_corner_arc() {
        let (c, r, _) = handle_disc_layout(400.0, 300.0, 40.0, 32.0, false);
        let (tx, ty) = c[BorderElement::TopLeft.index()];
        assert_eq!(tx, ty);
        // Distance from the arc centre (40, 40) plus the disc radius is the
        // arc radius: tangent from the inside.
        let d = ((tx - 40.0).powi(2) + (ty - 40.0).powi(2)).sqrt();
        assert!((d + r - 40.0).abs() < 1e-9);
    }

    #[test]
    fn discs_sharing_an_edge_are_inline() {
        use BorderElement::*;
        // A corner arc wider than the disc pulls the corners in; the side
        // discs must come in with them.
        let (c, _, n) = handle_disc_layout(800.0, 600.0, 40.0, 32.0, true);
        assert_eq!(n, HANDLE_COUNT);
        let y = |e: BorderElement| c[e.index()].1;
        let x = |e: BorderElement| c[e.index()].0;
        for e in [Top, TopRight, Minimize, Maximize, ToggleTile] {
            assert_eq!(y(e), y(TopLeft));
        }
        assert_eq!(y(Bottom), y(BottomLeft));
        assert_eq!(y(BottomRight), y(BottomLeft));
        assert_eq!(x(Left), x(TopLeft));
        assert_eq!(x(BottomLeft), x(TopLeft));
        assert_eq!(x(Right), x(TopRight));
        assert_eq!(x(BottomRight), x(TopRight));
    }

    #[test]
    fn buttons_run_left_from_the_top_right_disc_without_overlap() {
        use BorderElement::*;
        let (c, r, n) = handle_disc_layout(800.0, 600.0, 0.0, 32.0, true);
        assert_eq!(n, HANDLE_COUNT);
        let x = |e: BorderElement| c[e.index()].0;
        let row = [TopLeft, Top, Minimize, Maximize, ToggleTile, TopRight];
        for pair in row.windows(2) {
            assert!(x(pair[1]) - x(pair[0]) >= 2.0 * r, "{:?} crowds {:?}", pair[0], pair[1]);
        }
        // A wide window keeps the Top disc on its midpoint.
        assert_eq!(x(Top), 400.0);
    }

    #[test]
    fn top_disc_steps_aside_and_buttons_drop_when_the_row_is_full() {
        use BorderElement::*;
        // 32 px discs, 40 px step: the six-disc row needs 2*16 + 5*40 = 232.
        let (c, r, n) = handle_disc_layout(240.0, 300.0, 0.0, 32.0, true);
        assert_eq!(n, HANDLE_COUNT);
        let top = c[Top.index()].0;
        assert!(top < 120.0);
        assert!(c[Minimize.index()].0 - top >= 2.0 * r);
        assert!(top - c[TopLeft.index()].0 >= 2.0 * r);
        let (c, _, n) = handle_disc_layout(200.0, 300.0, 0.0, 32.0, true);
        assert_eq!(n, 8);
        assert_eq!(c[Top.index()].0, 100.0);
    }
}

#[cfg(test)]
mod satellite_tests {
    use super::*;

    const VIEW: (f64, f64, f64, f64) = (0.0, 0.0, 1920.0, 1200.0);

    #[test]
    fn centred_on_a_sibling_in_view() {
        let at = centered_over((200.0, 100.0, 1400.0, 1000.0), (800.0, 600.0), VIEW);
        assert_eq!(at, (500.0, 300.0));
    }

    #[test]
    fn slides_into_view_when_the_sibling_hangs_off_it() {
        let at = centered_over((-1000.0, 900.0, 1400.0, 1000.0), (800.0, 600.0), VIEW);
        assert_eq!(at, (0.0, 600.0));
    }

    #[test]
    fn larger_than_the_view_stays_centred_on_the_sibling() {
        let at = centered_over((0.0, 0.0, 1000.0, 1000.0), (2000.0, 600.0), VIEW);
        assert_eq!(at, (-500.0, 200.0));
    }

    #[test]
    fn a_title_rule_beats_a_borrowed_entry_only() {
        assert!(rule_skips_restore(false, false));
        assert!(!rule_skips_restore(false, true));
        assert!(rule_skips_restore(true, true));
        assert!(rule_skips_restore(true, false));
    }
}
