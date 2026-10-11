// KDL config parsing for monolithic cce server
 
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use crate::tiling::TilingMode;

#[path = "config/kdl_read.rs"]
mod kdl_read;
use kdl_read::*;
#[path = "config/parse.rs"]
mod parse;
use parse::*;
#[path = "config/apply.rs"]
mod apply;
pub use apply::*;

#[derive(Debug, Clone)]
pub struct Layout {
    pub gap: i32,           // inter-window spacing
    pub gap_top: i32,       // screen edge inset, top (above bar)
    pub gap_left: i32,      // screen edge inset, left
    pub gap_right: i32,     // screen edge inset, right
    pub gap_bottom: i32,    // screen edge inset, bottom
    pub cascade_offset: i32,
    pub bar_height: i32,
    pub border_width: i32,
    pub fullscreen_border_width: i32,
    pub cascade_border_width: i32,
    pub grid_border_width: i32,
    pub floating_border_width: i32,
    /// Premultiplied-alpha RGBA, 0.0–1.0 per channel (scenefx convention).
    pub border_color: [f32; 4],
    /// Border color for the focused window; defaults to `border_color`.
    pub border_color_focused: [f32; 4],
    /// Border color while the pointer hovers the border (the grab surface);
    /// defaults to a lightened `border_color_focused`.
    pub border_color_hover: [f32; 4],
    /// Visual gap between the 8 border zone segments.
    pub border_segment_gap: i32,
    /// How thin the resize-handle ring gets at the corners, as a fraction of
    /// its thickness at the middle of a side. 1.0 is an even ring; smaller
    /// values swell the middle of each side, like a picture-frame moulding.
    pub border_taper: f32,
    /// Thickness of the resize-handle ring at the middle of a side, in SCREEN
    /// px. Deliberately its own knob rather than derived from `width`: the
    /// ring has to be thick enough to see and hit while the desktop is zoomed
    /// out, and the window's visible border is a much finer line than that.
    pub border_handle_width: f32,
    /// Opacity a Floating window is dimmed to while it overlaps the window
    /// whose resize handles are up (adjust mode), 0..1. 1.0 disables.
    pub border_overlap_opacity: f32,
    /// How long a window or overlay dissolves in when it maps, in ms. 0
    /// disables the open fade and windows appear at full strength.
    pub fade_in_ms: u32,
    /// How long a client that asked to close dissolves out over, in ms. This
    /// is also the deadline the client waits on before it exits, so it is
    /// answered back over the control socket rather than assumed — see the
    /// `fade-out` command. 0 disables the close fade.
    pub fade_out_ms: u32,
    /// Shape of the handle ring's swell along a side. Below 1 the ring gains
    /// its thickness early — a corner that visibly swells, then a long slow
    /// approach to the middle. Above 1 stays thin near the corner and gains
    /// late. 1.0 is the plain eased ramp.
    pub border_swell_curve: f32,
    /// Radius of the round pad on each corner of the handle ring, in screen
    /// px. 0 disables the pads and leaves the plain moulding.
    pub border_corner_bulge: f32,
    /// Corner zone length measured from the outer corner along each band;
    /// 0 = auto (max(2 * width, 16)).
    pub border_corner_length: i32,
    pub background_r: u32,
    pub background_g: u32,
    pub background_b: u32,
    pub background_a: u32,
    pub border_font_size: i32,
    pub transition_duration: i32,
    pub grid_gap: i32,
    pub border_blur: bool,
    pub window_blur: bool,
    /// THE window corner radius, logical px before span widening —
    /// `style.surface.plate.root.corner_radius`, the same key cce-ui's
    /// root plates and `window_corner_radius` read. Every rounded thing a
    /// window has derives from it (`Window::root_plate_radius_base`): the
    /// content clip, the root plate, the border ring, frame and corner
    /// discs, and the desktop grid's cells (`background_spec`), so a tiled
    /// window's arc sits on its cell's. There is no second radius: the
    /// border's own `corner_radius` key was dead config and was removed
    /// on 2026-09-28.
    pub root_plate_corner_radius: i32,
    pub overlay_behavior: String,
    pub overlay_width: i32,
    pub overlay_position: String,
    pub overlay_border_gap: i32,
    pub status_normal_color: String,
    pub status_background_blur: f32,
    pub desktop_gap_color: String,
    pub transparency_opacity: f32,
    pub window_opacity: bool,
    pub desktop_cell_color: [f32; 4],
    /// Desktop grid cell width/height in virtual units (each axis's period
    /// is cell + gap). Config: `grid_cell_width` / `grid_cell_height`, with
    /// the legacy `grid_cell_size` setting both.
    pub desktop_cell_width: f64,
    pub desktop_cell_height: f64,
    pub desktop_gap_width: i32,
    /// Grid-line lip width in logical px; None = follow the DE-wide relief
    /// material (bevel_thickness clamped to the rail), 0 = no lip.
    pub desktop_line_relief: Option<f64>,
    pub desktop_cell_fade_inset: i64,
    pub desktop_grid_fade_mode: String,
    /// Chess-style coordinates on the desktop squares while overview is
    /// open. KDL: `surface { desktop cell_labels=(bool)false }`.
    pub desktop_cell_labels: bool,
    /// Drop shadow under cce/ssd windows (scenefx box-shadow node).
    pub shadow_enabled: bool,
    /// Gaussian spread in logical px.
    pub shadow_sigma: f32,
    /// Premultiplied-alpha RGBA, like `border_color`.
    pub shadow_color: [f32; 4],
    /// Displacement of the cast shadow in logical px — should point away from
    /// `light_source_position` (down-right for the default top-left light).
    pub shadow_offset_x: i32,
    pub shadow_offset_y: i32,
    /// Whether tiled (and maximized — an alias of Tiled) windows cast the
    /// shadow at all. Off leaves shadows to floating windows only, so a
    /// tiled layout reads as a flat sheet with no smearing across cell gaps.
    pub shadow_tiled: bool,
    /// Edge bevel: a lit chamfer drawn around the INSIDE of a decorated
    /// window's edge (scenefx bevel node), lit from `bevel_light_*` — the
    /// same top-left source the drop shadow is offset away from.
    pub bevel_enabled: bool,
    /// Rim width in logical px.
    pub bevel_thickness: f32,
    /// Direction toward the light; y is down, so the default is up-left.
    pub bevel_light_x: f32,
    pub bevel_light_y: f32,
    /// Strength of the lit and shaded sides, 0..1.
    pub bevel_light_intensity: f32,
    pub bevel_shade_intensity: f32,
    /// 0 = hard flat chamfer, 1 = fully rounded shoulder.
    pub bevel_shoulder: f32,
    /// Highlight tint, premultiplied RGBA; alpha scales the whole effect.
    pub bevel_color: [f32; 4],
    /// Focused-window rim accent: the bevel highlight wraps all four sides
    /// in this color for the focused window (the DE focus glint).
    pub bevel_focus_color: [f32; 3],
    /// How sharply the focused rim's glint falls off across the bevel:
    /// the exponent on the rim slope. 1 is a linear ramp over the whole
    /// thickness (reads as a wash); higher concentrates the light at the
    /// silhouette, at some cost in peak brightness, since the outermost
    /// rendered sample is already below 1.0. Past ~12 it only dims.
    pub bevel_focus_sharpness: f32,
    /// Magnetic grid snap for interactive move/resize.
    pub desktop_snap: bool,
    /// Speed ramp + duration (ms) for the overview enter/exit transition.
    /// `None` (no/invalid `desktop { overview_ramp= }`) falls back to the
    /// exponential-approach camera animation.
    pub overview_anim: Option<(crate::policy::ramp::SpeedRamp, f64)>,
    /// Snap radius in virtual units.
    pub desktop_snap_threshold: f64,
    /// Edge auto-pan: dragging/resizing against a screen edge scrolls the
    /// desktop underneath the pinned cursor.
    pub desktop_edge_pan: bool,
    /// Width of the trigger band inside each output edge, in layout px.
    pub desktop_edge_pan_band: f64,
    /// Full-tilt pan speed at the screen edge, in screen px/s (the tick
    /// divides by zoom; speed ramps linearly across the band).
    pub desktop_edge_pan_speed: f64,
    pub scenefx_optimized_blur: bool,
    pub status_backdrop_blur_ignore_transparent: bool,
    pub window_backdrop_blur_ignore_transparent: bool,
    pub status_module_hide_mode_preview: i64,
    /// Gap between adjacent status segments; the bar's own config file
    /// (`~/.config/cce/cce-status-interface/config.kdl`, `module { spacing }`)
    /// overrides the shared `style { status module_spacing }`.
    pub status_module_spacing: i64,
    /// The bar's `module { droplet }` spec string when present — the water-
    /// drop module style. The compositor drives a scenefx droplet node
    /// (backdrop refraction) per status segment from the SAME spec the bar
    /// draws its drops from, so the two silhouettes cannot drift.
    pub status_droplet: Option<String>,
    /// Backdrop compression for status segments, from the bar's
    /// `module { backdrop_compress }` (the minimum WCAG contrast ratio the
    /// module text must hold against any backdrop pixel) and its
    /// `module { text_color }`: `(ceil, knee, invert)` in linear luminance,
    /// see `backdrop_compress_params`. `None` = off.
    pub status_backdrop_compress: Option<(f32, f32, bool)>,
    pub cloud_position_default: Option<[i32; 2]>,

    // ---- Window-manager configuration, moved here from `WindowManager` on
    // 2026-10-10 so code outside the window manager reads it through the
    // shared snapshot (`crate::shared::layout()`) instead of a `&mut` to it.
    pub pointer_binds: Vec<crate::config::PointerBind>,
    pub gesture_binds: Vec<crate::config::GestureBind>,
    /// Xwayland sees a physical-pixel screen and X11 surfaces draw at
    /// 1/scale (see `WindowManagerConfig::xwayland_hidpi`).
    pub xwayland_hidpi: bool,
    /// X11 windows kept in the logical world while `xwayland_hidpi` is on
    /// (see `xwayland_window::x11_scale_for`).
    pub xwayland_hidpi_except: Vec<String>,
    /// Trackpad-to-view-drag emulation (see `cursor::ViewDrag`).
    pub touchpad_view_apps: Vec<String>,
    pub touchpad_view_swipe_tumble: bool,
    pub touchpad_view_sensitivity: f64,
    pub touchpad_view_invert: bool,
    /// `window_manager { osk_on_touch }` (`osk.rs`).
    pub osk_on_touch: bool,
    /// `window_manager { swipe_peek }`: the desktop's lean toward a
    /// directional swipe bind at its threshold, screen px (default 60;
    /// 0 disables). See `cursor::swipe_peek_for`.
    pub swipe_peek_px: f64,
    /// `window_manager { swipe_repeat_peek }`: the lean toward each further
    /// step once a swipe has switched focus, screen px at
    /// `swipe_repeat_threshold` (default half of `swipe_peek_px`).
    pub swipe_repeat_peek_px: f64,
    /// `window_manager { swipe_focus_cone }`: degrees off a focus swipe's
    /// direction within which a window center can take focus (default 45).
    /// See `focus_toward`.
    pub swipe_focus_cone_deg: f64,
    /// `window_manager { swipe_threshold }`: accumulated swipe travel
    /// (libinput units) at which a swipe bind fires (default 70).
    pub swipe_threshold: f64,
    /// `window_manager { swipe_repeat_threshold }`: the travel each FURTHER
    /// fire of the same swipe needs after its first (default four times
    /// `swipe_threshold`) — the resistance that keeps a swipe from
    /// running on through a second window.
    pub swipe_repeat_threshold: f64,
    /// See `WindowManagerConfig::touchpad_hscroll_shift_apps`.
    pub touchpad_hscroll_shift_apps: Vec<String>,
    pub input_config: crate::config::InputConfig,
    /// `window_manager.center_on_spawn`: whether a newly spawned window pulls the viewport
    /// over to it when it takes focus. Off, the desk stays put and the window opens wherever
    /// the layout placed it. Focus-follow panning between EXISTING windows is unaffected.
    pub center_on_spawn: bool,
    /// `window_manager.rounded_apps`: extra app_ids that get the decorated-window
    /// treatment (rounded corner clip, blur-behind, shadow) alongside cce-* apps
    /// and SSD requesters.
    pub rounded_apps: Vec<String>,
    pub bevel_apps: Vec<String>,
}

impl Layout {
    /// The desktop background as the policy crate's declarative spec. The
    /// desktop is always the grid; per-frame geometry comes from
    /// `policy::background::grid_frame`.
    pub fn background_spec(&self) -> crate::policy::api::BackgroundSpec {
        use crate::policy::api::{BackgroundSpec, GridFadeMode, GridSpec, Rgba};
        BackgroundSpec::Grid(GridSpec {
            gap_color: Rgba(parse_hex_color_rgba(&self.desktop_gap_color)),
            cell_color: Rgba(self.desktop_cell_color),
            cell_w: self.desktop_cell_width,
            cell_h: self.desktop_cell_height,
            gap_width: self.desktop_gap_width as f64,
            // Cells inherit the window root plate radius: a tiled window's
            // content covers exactly the visible cell box, so its arc sits
            // precisely on the cell's arc underneath.
            cell_corner_radius: self.root_plate_corner_radius,
            cell_fade_inset: self.desktop_cell_fade_inset as i32,
            fade_mode: GridFadeMode::from_name(&self.desktop_grid_fade_mode),
        })
    }

    /// Snap parameters for interactive ops. A zero threshold (snap
    /// disabled) makes every snap function a no-op.
    pub fn snap_params(&self) -> crate::policy::snap::SnapParams {
        crate::policy::snap::SnapParams {
            cell_w: self.desktop_cell_width,
            cell_h: self.desktop_cell_height,
            gap_width: self.desktop_gap_width as f64,
            cell_inset: self.desktop_cell_fade_inset as f64,
            threshold: if self.desktop_snap { self.desktop_snap_threshold } else { 0.0 },
        }
    }
}

impl Layout {
    /// Whether an app_id gets the decorated-window treatment (rounded corner
    /// clip, blur-behind, drop shadow) without requesting SSD: every cce app,
    /// plus the `window_manager.rounded_apps` config allowlist. The one
    /// predicate behind every radius/blur/shadow decision — the mirrored
    /// render sites must all agree or the effects visibly disagree per pass.
    pub fn is_decorated_app(&self, app_id: &str) -> bool {
        app_id.starts_with("cce-") || self.rounded_apps.iter().any(|a| crate::window_manager::app_id_matches(a, app_id))
    }

    /// Should the compositor draw an edge bevel on this app? Unlike
    /// `is_decorated_app` there is NO implicit cce-* arm: every cce-ui app
    /// draws its own bevel, and a second one from the compositor just doubles
    /// the rim. Only apps named in `bevel_apps` (defaulting to `rounded_apps`)
    /// get one.
    pub fn is_beveled_app(&self, app_id: &str) -> bool {
        self.bevel_apps.iter().any(|a| crate::window_manager::app_id_matches(a, app_id))
    }
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            gap: 48,
            gap_top: 48,
            gap_left: 48,
            gap_right: 48,
            gap_bottom: 48,
            cascade_offset: 20,
            bar_height: 24,
            border_width: 0,
            fullscreen_border_width: 0,
            cascade_border_width: 0,
            grid_border_width: 0,
            floating_border_width: 0,
            border_color: [62.0 / 255.0, 62.0 / 255.0, 62.0 / 255.0, 1.0],
            border_color_focused: [62.0 / 255.0, 62.0 / 255.0, 62.0 / 255.0, 1.0],
            border_color_hover: lighten_premultiplied([62.0 / 255.0, 62.0 / 255.0, 62.0 / 255.0, 1.0], HOVER_LIGHTEN),
            border_segment_gap: 4,
            border_taper: 0.35,
            border_handle_width: 32.0,
            border_overlap_opacity: 0.4,
            fade_in_ms: 140,
            fade_out_ms: 120,
            border_swell_curve: 0.45,
            border_corner_bulge: 48.0,
            border_corner_length: 0,
            background_r: 0x1C1C1C1Cu32,
            background_g: 0x20202020u32,
            background_b: 0x20202020u32,
            background_a: 0xFFFFFFFFu32,
            border_font_size: 11,
            transition_duration: 300,
            grid_gap: 18,
            border_blur: false,
            window_blur: false,
            root_plate_corner_radius: 12,
            overlay_behavior: "inline".to_string(),
            overlay_width: 360,
            overlay_position: "left".to_string(),
            overlay_border_gap: 0,
            status_normal_color: "#ccccd8".to_string(),
            status_background_blur: 0.8,
            desktop_gap_color: "#000000".to_string(),
            transparency_opacity: 0.9,
            window_opacity: true,
            desktop_cell_color: [0.05, 0.05, 0.05, 0.05],
            desktop_cell_width: 100.0,
            desktop_cell_height: 100.0,
            desktop_gap_width: 1,
            desktop_line_relief: None,
            desktop_cell_fade_inset: 0,
            desktop_grid_fade_mode: "linear".to_string(),
            desktop_cell_labels: true,
            bevel_enabled: true,
            bevel_thickness: 10.0,
            bevel_light_x: -0.7071,
            bevel_light_y: -0.7071,
            bevel_light_intensity: 0.6,
            bevel_shade_intensity: 0.5,
            bevel_shoulder: 0.55,
            bevel_color: [1.0, 1.0, 1.0, 1.0],
            bevel_focus_color: [0.35, 0.78, 0.78],
            bevel_focus_sharpness: 3.0,
            shadow_enabled: true,
            shadow_sigma: 22.0,
            shadow_color: [0.0, 0.0, 0.0, 0.55],
            shadow_offset_x: 7,
            shadow_offset_y: 7,
            shadow_tiled: true,
            desktop_snap: true,
            overview_anim: None,
            desktop_snap_threshold: 24.0,
            desktop_edge_pan: true,
            desktop_edge_pan_band: 32.0,
            desktop_edge_pan_speed: 1000.0,
            scenefx_optimized_blur: true,
            status_backdrop_blur_ignore_transparent: true,
            window_backdrop_blur_ignore_transparent: true,
            status_module_hide_mode_preview: 4,
            status_module_spacing: 12,
            status_droplet: None,
            status_backdrop_compress: None,
            cloud_position_default: None,
            pointer_binds: Vec::new(),
            gesture_binds: Vec::new(),
            xwayland_hidpi: true,
            xwayland_hidpi_except: Vec::new(),
            touchpad_view_apps: Vec::new(),
            touchpad_view_swipe_tumble: false,
            touchpad_view_sensitivity: 1.0,
            touchpad_view_invert: false,
            osk_on_touch: true,
            swipe_peek_px: 60.0,
            swipe_repeat_peek_px: 30.0,
            swipe_focus_cone_deg: 45.0,
            swipe_threshold: 70.0,
            swipe_repeat_threshold: 280.0,
            touchpad_hscroll_shift_apps: Vec::new(),
            input_config: InputConfig::default(),
            center_on_spawn: true,
            rounded_apps: Vec::new(),
            bevel_apps: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModeRule {
    pub mode: TilingMode,
    pub app_id_pattern: String,
    pub title_pattern: Option<String>,
    pub single_instance: bool,
    pub tag: i32,
    pub circular: bool,
    pub ssd: Option<bool>,
    /// The window is a satellite of its app's main window (a settings
    /// window opened as a parentless toplevel): it is never restored from
    /// saved state, sizes itself, and maps centred over a mapped sibling of
    /// the same app_id. See `Window::try_center_on_sibling`.
    pub over_sibling: bool,
    /// The window opens centred on the current view, whatever position is
    /// remembered for it: a prompt the user has to answer now (1Password's
    /// authorization popup). Placement only — the remembered size still
    /// applies, and a self-sizing window is re-centred once its real size
    /// lands. See `Window::try_center_on_view`.
    pub center: bool,
}

pub use cce_window_manager::api::Action;

#[derive(Debug, Deserialize, Clone)]
pub struct KeybindConfig {
    pub mods: String,
    pub key: String,
    pub action: String,
    pub command: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PointerBindConfig {
    pub mods: String,
    pub button: String,
    pub action: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GestureBindConfig {
    #[serde(default)]
    pub mods: Option<String>,
    #[serde(rename = "type")]
    pub gesture_type: String,
    pub fingers: u32,
    pub direction: String,
    pub action: String,
    pub command: Option<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct StartupConfig {
    pub exec: String,
    #[serde(default)]
    pub once: bool,
    #[serde(default)]
    pub restart: bool,
}

// The compositor's resolved keybind is the policy crate's `Binding`
// (mods, keysym, action, command), kept under its historical name here.
pub use cce_window_manager::bindings::Binding as Keybind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerBind {
    pub mods: u32,
    pub button: u32,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GestureBind {
    pub mods: u32,
    pub gesture_type: String,
    pub fingers: u32,
    pub direction: String,
    pub action: Action,
    pub command: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OutputConfig {
    #[serde(default = "default_scenefx_optimized_blur")]
    pub scenefx_optimized_blur: bool,
}

fn default_scenefx_optimized_blur() -> bool {
    true
}

#[derive(Debug, Deserialize, Clone)]
pub struct InputDeviceConfigRule {
    pub name: String,
    pub scroll_factor: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WindowManagerConfig {
    pub close_window: Option<String>,
    pub toggle_fullscreen: Option<String>,
    pub toggle_overview: Option<String>,
    pub window_switcher: Option<String>,
    pub window_switcher_prev: Option<String>,
    /// Whether a newly spawned window pulls the viewport over to it. `None` = the default,
    /// which is to centre (what the compositor has always done).
    pub center_on_spawn: Option<bool>,
    /// Camera reaction when the focused window goes away (app exit, close,
    /// minimize): "focus_previous" (pan to the fallback focus — the historic
    /// behavior), "overview", or "none". Menu-typed in config.kdl so the
    /// settings tree renders a dropdown.
    pub on_app_exit: Option<String>,
    /// Corner-shape exponent for scenefx's rounded-corner cuts (window
    /// surfaces, blur, shadows' clip): 2 = circular arc, > 2 = superellipse
    /// squircle. The same `window_manager.corner_shape` key the cce-ui
    /// clients read, so the compositor's cut lands on the corners they draw.
    pub corner_shape: Option<f64>,
    /// Extra app_ids (beyond cce-* apps and SSD requesters) that get the full
    /// decorated-window treatment: rounded corner clip, blur-behind, shadow.
    /// KDL: `rounded_apps "*claude*" "org.example.App"`.
    ///
    /// Entries are matched case-insensitively by `app_id_matches`, and one
    /// containing `*` is a glob. Prefer a glob for anything third-party: an
    /// app_id is not a stable identifier, and an exact entry that stops
    /// matching after a rename takes the corners, blur, shadow and bevel with
    /// it in silence. `ccectl windows` reports the outcome as `decorated=`.
    pub rounded_apps: Option<Vec<String>>,
    /// Which apps the compositor draws an edge BEVEL on. Separate from
    /// `rounded_apps` because drawing one is only right for apps that do not
    /// bevel themselves — every cce-ui app already draws its own, so beveling
    /// them compositor-side doubles the rim. Unset falls back to
    /// `rounded_apps` (never the implicit cce-* set).
    /// KDL: `bevel_apps "*claude*"`. Globbed like `rounded_apps`; reported by
    /// `ccectl windows` as `beveled=`.
    pub bevel_apps: Option<Vec<String>>,
    /// Present Xwayland with a PHYSICAL-pixel screen (the xdg-output global is
    /// hidden from it, so it sizes its root from the wl_output mode) and draw
    /// X11 surfaces at 1/scale, so HiDPI-aware X11 apps render sharp instead
    /// of being upscaled from logical size. Default on. KDL:
    /// `xwayland_hidpi (bool)false` to get the old blurry-but-1:1 behaviour.
    pub xwayland_hidpi: Option<bool>,
    /// Per-app exception to `xwayland_hidpi`: windows matching one of these
    /// patterns are configured and drawn in the logical world (factor 1)
    /// while every other X11 window stays physical. Meant for games and other
    /// X11 clients that size themselves to the whole screen and would render
    /// scale² times the pixels for nothing. A pattern is tried against the
    /// window's WM_CLASS class, its WM_CLASS instance and its title, since
    /// every Proton window shares the class `steam_proton`; `*` wildcards as
    /// in `rounded_apps`. KDL: `xwayland_hidpi_except "Trackmania"`.
    ///
    /// A named window is treated as the full-screen X11 game it is
    /// (`xwayland_window::window_is_hidpi_exempt`): it is drawn at 1 in the
    /// logical world, it is NOT restored to a saved size at map (it sizes
    /// itself to the screen — a restored 1214x689 is what Trackmania then
    /// pinned in its hints), its own position requests are granted (its
    /// "windowedfull" asks for the desktop origin, and refusing that was a
    /// ~170/s configure loop), and only a compositor fullscreen or tiling
    /// overrides its size.
    pub xwayland_hidpi_except: Option<Vec<String>>,
    /// Apps whose windows turn trackpad input into a view drag (Space +
    /// button) — see `cursor::ViewDrag`. A scroll over one of their popups
    /// becomes a wheel under a held Ctrl instead — see `cursor::PopupWheel`.
    /// KDL: `touchpad_view_apps "Houdini FX"`.
    pub touchpad_view_apps: Option<Vec<String>>,
    /// What an unmodified two-finger swipe does there: "pan" (default) or
    /// "tumble"; Shift does the other. KDL: `touchpad_view_swipe "tumble"`.
    pub touchpad_view_swipe: Option<String>,
    /// Finger-to-pointer distance factor for the emulated drag (default 1).
    pub touchpad_view_sensitivity: Option<f64>,
    /// How far the desktop leans toward a directional swipe bind by the
    /// time the swipe reaches its threshold, screen px (default 60; 0
    /// turns the lean off). KDL: `swipe_peek (f64)60.0`. Lives here, not
    /// under `input`, because input.kdl's `input {}` block replaces
    /// config.kdl's wholesale. See "Swipe binds peek" in CLAUDE.md.
    pub swipe_peek: Option<f64>,
    /// The lean toward each FURTHER step of a swipe that has already
    /// switched focus, screen px at `swipe_repeat_threshold` (default half
    /// of `swipe_peek`; 0 turns it off). Slower than the first step's
    /// lean, so a swipe that has just switched reads as settled. KDL:
    /// `swipe_repeat_peek (f64)30.0`.
    pub swipe_repeat_peek: Option<f64>,
    /// How far off the swipe's own direction (degrees) a window center may
    /// lie and still take focus from a three-finger focus swipe (default
    /// 45; clamped to 1-89). A swipe toward nothing within it changes
    /// nothing. KDL: `swipe_focus_cone (f64)45.0`.
    pub swipe_focus_cone: Option<f64>,
    /// Accumulated travel (libinput units, roughly mm) at which a swipe
    /// bind fires (default 70). KDL: `swipe_threshold (f64)70.0`.
    pub swipe_threshold: Option<f64>,
    /// Travel each further fire of the same swipe needs after its first
    /// (default four times `swipe_threshold`), so a swipe that has just
    /// stepped focus meets more resistance before stepping again. KDL:
    /// `swipe_repeat_threshold (f64)280.0`.
    pub swipe_repeat_threshold: Option<f64>,
    /// Reverse the drag direction, on top of the natural-scroll correction
    /// the drag already makes. KDL: `touchpad_view_invert (bool)true`.
    pub touchpad_view_invert: Option<bool>,
    /// Apps whose native widgets scroll sideways only under Shift and read a
    /// horizontal wheel as a vertical one (Houdini's spreadsheet, list and
    /// parameter panes): a horizontal two-finger scroll over their windows is
    /// delivered as a vertical scroll with Shift held for the gesture. KDL:
    /// `touchpad_hscroll_shift_apps "Houdini FX"`.
    pub touchpad_hscroll_shift_apps: Option<Vec<String>>,
    /// Show the on-screen keyboard when a touch activates a text field, and
    /// hide it when the field lets go (`osk.rs`). Default on. KDL:
    /// `osk_on_touch (bool)false`.
    pub osk_on_touch: Option<bool>,
}

#[derive(Debug, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct GesturesConfig {
    pub swipe: Option<bool>,
    pub pinch: Option<bool>,
}

#[derive(Debug, Deserialize, Clone, Default, PartialEq)]
pub struct TouchpadConfig {
    pub tap_to_click: Option<bool>,
    pub natural_scroll: Option<bool>,
    pub dwt: Option<bool>,
    pub dwtp: Option<bool>,
    pub gestures: Option<GesturesConfig>,
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<String>,
    pub scroll_factor: Option<f64>,
}

#[derive(Debug, Deserialize, Clone, Default, PartialEq)]
pub struct TrackpointConfig {
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<String>,
    pub scroll_factor: Option<f64>,
    /// libinput scroll method: `"none"`, `"button"` (scroll while the
    /// middle button is held) or `"two_finger"` / `"edge"` for devices that
    /// support them. libinput defaults a pointing stick to `"button"`, which
    /// withholds every middle press until the release to see whether it was
    /// a scroll: clients then get a press and release in the same instant,
    /// so a middle *click* never registers and a middle *drag* scrolls.
    pub scroll_method: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Default, PartialEq)]
pub struct MouseConfig {
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<String>,
    pub scroll_factor: Option<f64>,
    /// See `TrackpointConfig::scroll_method`.
    pub scroll_method: Option<String>,
}

/// Pointer device configuration. Per-class blocks (`mouse` / `touchpad` /
/// `trackpad` alias / `trackpoint`) override the top-level values for
/// devices of that class; a device is a trackpoint if its name says so, a
/// touchpad if it supports tap, and a mouse otherwise.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct InputConfig {
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<String>,
    pub scroll_factor: Option<f64>,
    /// Desktop-pan smooth scrolling (the compositor's own consumption of the
    /// wheel: background/overview/super pans and ctrl+super zoom). Same keys
    /// and defaults as cce-ui's `widget::scroll_motion`, so a wheel notch
    /// glides the same way over a list and over the desktop.
    /// `scroll_ease`: wheel-glide rate, 1/s (default 12).
    pub scroll_ease: Option<f64>,
    /// `kinetic_scroll`: a trackpad flick keeps panning after the lift (default true).
    pub kinetic_scroll: Option<bool>,
    /// `scroll_friction`: coast decay, 1/s (default 6).
    pub scroll_friction: Option<f64>,
    /// `repeat_rate`: key repeats per second once repeating starts
    /// (default `keyboard::DEFAULT_REPEAT_RATE`; 0 disables repeat).
    pub repeat_rate: Option<i64>,
    /// `repeat_delay`: ms a key is held before it starts repeating
    /// (default `keyboard::DEFAULT_REPEAT_DELAY`).
    pub repeat_delay: Option<i64>,
    pub mouse: Option<MouseConfig>,
    pub touchpad: Option<TouchpadConfig>,
    pub trackpoint: Option<TrackpointConfig>,
}

impl InputConfig {
    /// `(rate, delay)` for hardware keyboards, defaults filled in. Negative
    /// values are clamped to 0, which `wl_keyboard.repeat_info` reads as
    /// "no repeat" for the rate; the protocol rejects negatives outright.
    pub fn repeat_info(&self) -> (i32, i32) {
        let rate = self.repeat_rate.map_or(crate::keyboard::DEFAULT_REPEAT_RATE, |v| v.clamp(0, i32::MAX as i64) as i32);
        let delay = self.repeat_delay.map_or(crate::keyboard::DEFAULT_REPEAT_DELAY, |v| v.clamp(0, i32::MAX as i64) as i32);
        (rate, delay)
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TransparencyConfig {
    pub opacity: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SurfaceConfig {
    #[serde(default = "default_desktop_gap_color")]
    pub desktop_gap_color: String,
    #[serde(default = "default_desktop_cell_color")]
    pub desktop_cell_color: String,
    #[serde(default = "default_desktop_grid_scale")]
    pub desktop_grid_scale: i64,
    /// Per-axis cell sizes; None falls back to `desktop_grid_scale`
    /// (the legacy square `grid_cell_size`).
    #[serde(default)]
    pub grid_cell_width: Option<i64>,
    #[serde(default)]
    pub grid_cell_height: Option<i64>,
    #[serde(default = "default_desktop_gap_width")]
    pub desktop_gap_width: i64,
    /// Negative = unset (follow the DE-wide relief material).
    #[serde(default = "default_desktop_line_relief")]
    pub desktop_line_relief: i64,
    #[serde(default = "default_desktop_cell_fade_inset")]
    pub desktop_cell_fade_inset: i64,
    #[serde(default = "default_desktop_grid_fade_mode")]
    pub desktop_grid_fade_mode: String,
    #[serde(default = "default_desktop_cell_labels")]
    pub desktop_cell_labels: bool,
    #[serde(default = "default_desktop_snap")]
    pub desktop_snap: bool,
    #[serde(default)]
    pub desktop_overview_ramp: String,
    #[serde(default = "default_desktop_overview_ms")]
    pub desktop_overview_ms: i64,
    #[serde(default = "default_desktop_snap_threshold")]
    pub desktop_snap_threshold: i64,
    #[serde(default = "default_desktop_edge_pan")]
    pub desktop_edge_pan: bool,
    #[serde(default = "default_desktop_edge_pan_band")]
    pub desktop_edge_pan_band: i64,
    #[serde(default = "default_desktop_edge_pan_speed")]
    pub desktop_edge_pan_speed: i64,
    #[serde(default = "default_root_plate_color")]
    pub root_plate_color: String,
    #[serde(default = "default_root_plate_blur")]
    pub root_plate_blur: f64,
    #[serde(default = "default_root_plate_corner_radius")]
    pub root_plate_corner_radius: i64,
    #[serde(default = "default_border_width")]
    pub border_width: i64,
    #[serde(default = "default_border_color")]
    pub border_color: String,
    /// `None` falls back to `border_color`.
    #[serde(default)]
    pub border_color_focused: Option<String>,
    /// `None` falls back to a lightened `border_color_focused`.
    #[serde(default)]
    pub border_color_hover: Option<String>,
    #[serde(default = "default_border_segment_gap")]
    pub border_segment_gap: i64,
    #[serde(default = "default_border_taper")]
    pub border_taper: f64,
    #[serde(default = "default_border_handle_width")]
    pub border_handle_width: f64,
    #[serde(default = "default_border_overlap_opacity")]
    pub border_overlap_opacity: f64,
    /// `surface { fade in_ms=.. out_ms=.. }` — the DE-wide open/close
    /// dissolve. Milliseconds; 0 on either disables that direction.
    #[serde(default = "default_fade_in_ms")]
    pub fade_in_ms: i64,
    #[serde(default = "default_fade_out_ms")]
    pub fade_out_ms: i64,
    #[serde(default = "default_border_swell_curve")]
    pub border_swell_curve: f64,
    #[serde(default = "default_border_corner_bulge")]
    pub border_corner_bulge: f64,
    /// 0 = auto (max(2 * width, 16)).
    #[serde(default)]
    pub border_corner_length: i64,
    #[serde(default = "default_cloud_position_default")]
    pub cloud_position_default: Option<[i32; 2]>,
    #[serde(default = "default_shadow_enabled")]
    pub shadow_enabled: bool,
    #[serde(default = "default_shadow_sigma")]
    pub shadow_sigma: f64,
    #[serde(default = "default_shadow_color")]
    pub shadow_color: String,
    #[serde(default = "default_shadow_offset_x")]
    pub shadow_offset_x: i64,
    #[serde(default = "default_shadow_offset_y")]
    pub shadow_offset_y: i64,
    #[serde(default = "default_shadow_tiled")]
    pub shadow_tiled: bool,
    #[serde(default = "default_bevel_enabled")]
    pub bevel_enabled: bool,
    #[serde(default = "default_bevel_thickness")]
    pub bevel_thickness: f64,
    #[serde(default = "default_bevel_light")]
    pub bevel_light: String,
    #[serde(default = "default_bevel_light_intensity")]
    pub bevel_light_intensity: f64,
    #[serde(default = "default_bevel_shade_intensity")]
    pub bevel_shade_intensity: f64,
    #[serde(default = "default_bevel_shoulder")]
    pub bevel_shoulder: f64,
    #[serde(default = "default_bevel_color")]
    pub bevel_color: String,
    #[serde(default = "default_bevel_focus_color")]
    pub bevel_focus_color: String,
    #[serde(default = "default_bevel_focus_sharpness")]
    pub bevel_focus_sharpness: f64,
}

fn default_shadow_enabled() -> bool { true }
fn default_shadow_sigma() -> f64 { 22.0 }
fn default_shadow_color() -> String { "#0000008c".to_string() }
fn default_shadow_offset_x() -> i64 { 7 }
fn default_shadow_offset_y() -> i64 { 7 }
fn default_shadow_tiled() -> bool { true }
fn default_bevel_enabled() -> bool { true }
fn default_bevel_thickness() -> f64 { 10.0 }
/// Compass point the light comes FROM, matching the shadow's top-left source.
fn default_bevel_light() -> String { "top-left".to_string() }
fn default_bevel_light_intensity() -> f64 { 0.6 }
fn default_bevel_shade_intensity() -> f64 { 0.5 }
fn default_bevel_shoulder() -> f64 { 0.55 }
fn default_bevel_color() -> String { "#ffffffff".to_string() }
fn default_bevel_focus_color() -> String { "#59c7c7".to_string() }
fn default_bevel_focus_sharpness() -> f64 { 3.0 }

/// Map a compass point to a unit vector pointing TOWARD the light, in screen
/// space (y down). Anything unrecognized keeps the DE's top-left default.
pub fn parse_light_direction(s: &str) -> (f32, f32) {
    let d = 0.7071_f32;
    match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "top" | "up" | "north" => (0.0, -1.0),
        "bottom" | "down" | "south" => (0.0, 1.0),
        "left" | "west" => (-1.0, 0.0),
        "right" | "east" => (1.0, 0.0),
        "top-right" | "up-right" | "north-east" => (d, -d),
        "bottom-left" | "down-left" | "south-west" => (-d, d),
        "bottom-right" | "down-right" | "south-east" => (d, d),
        _ => (-d, -d),
    }
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self {
            desktop_gap_color: default_desktop_gap_color(),
            desktop_cell_color: default_desktop_cell_color(),
            desktop_grid_scale: default_desktop_grid_scale(),
            grid_cell_width: None,
            grid_cell_height: None,
            desktop_gap_width: default_desktop_gap_width(),
            desktop_line_relief: default_desktop_line_relief(),
            desktop_cell_fade_inset: default_desktop_cell_fade_inset(),
            desktop_grid_fade_mode: default_desktop_grid_fade_mode(),
            desktop_cell_labels: default_desktop_cell_labels(),
            desktop_snap: default_desktop_snap(),
            desktop_overview_ramp: String::new(),
            desktop_overview_ms: default_desktop_overview_ms(),
            desktop_snap_threshold: default_desktop_snap_threshold(),
            desktop_edge_pan: default_desktop_edge_pan(),
            desktop_edge_pan_band: default_desktop_edge_pan_band(),
            desktop_edge_pan_speed: default_desktop_edge_pan_speed(),
            root_plate_color: default_root_plate_color(),
            root_plate_blur: default_root_plate_blur(),
            root_plate_corner_radius: default_root_plate_corner_radius(),
            border_width: default_border_width(),
            border_color: default_border_color(),
            border_color_focused: None,
            border_color_hover: None,
            border_segment_gap: default_border_segment_gap(),
            border_taper: default_border_taper(),
            border_handle_width: default_border_handle_width(),
            border_overlap_opacity: default_border_overlap_opacity(),
            fade_in_ms: default_fade_in_ms(),
            fade_out_ms: default_fade_out_ms(),
            border_swell_curve: default_border_swell_curve(),
            border_corner_bulge: default_border_corner_bulge(),
            border_corner_length: 0,
            cloud_position_default: default_cloud_position_default(),
            shadow_enabled: default_shadow_enabled(),
            shadow_sigma: default_shadow_sigma(),
            shadow_color: default_shadow_color(),
            shadow_offset_x: default_shadow_offset_x(),
            shadow_offset_y: default_shadow_offset_y(),
            shadow_tiled: default_shadow_tiled(),
            bevel_enabled: default_bevel_enabled(),
            bevel_thickness: default_bevel_thickness(),
            bevel_light: default_bevel_light(),
            bevel_light_intensity: default_bevel_light_intensity(),
            bevel_shade_intensity: default_bevel_shade_intensity(),
            bevel_shoulder: default_bevel_shoulder(),
            bevel_color: default_bevel_color(),
            bevel_focus_color: default_bevel_focus_color(),
            bevel_focus_sharpness: default_bevel_focus_sharpness(),
        }
     }
}

fn default_cloud_position_default() -> Option<[i32; 2]> {
    None
}

fn default_desktop_gap_color() -> String {

    "#000000".to_string()
}

fn default_desktop_cell_color() -> String {
    "#ffffff0d".to_string()
}

fn default_desktop_grid_scale() -> i64 {
    100
}

fn default_desktop_gap_width() -> i64 {
    1
}

fn default_desktop_line_relief() -> i64 {
    -1
}

fn default_desktop_cell_fade_inset() -> i64 {
    0
}

fn default_desktop_cell_labels() -> bool {
    true
}

fn default_desktop_grid_fade_mode() -> String {
    "linear".to_string()
}

fn default_desktop_overview_ms() -> i64 {
    350
}

fn default_desktop_snap() -> bool {
    true
}

fn default_desktop_snap_threshold() -> i64 {
    24
}

fn default_desktop_edge_pan() -> bool {
    true
}

fn default_desktop_edge_pan_band() -> i64 {
    32
}

fn default_desktop_edge_pan_speed() -> i64 {
    1000
}

fn default_root_plate_color() -> String {
    "#151520e6".to_string()
}

fn default_root_plate_blur() -> f64 {
    0.8
}

fn default_root_plate_corner_radius() -> i64 {
    12
}

fn default_border_width() -> i64 {
    0
}

fn default_border_color() -> String {
    "#3e3e3e".to_string()
}

fn default_border_taper() -> f64 { 0.35 }
fn default_border_handle_width() -> f64 { 32.0 }
fn default_border_overlap_opacity() -> f64 { 0.4 }
fn default_fade_in_ms() -> i64 { 140 }
fn default_fade_out_ms() -> i64 { 120 }
fn default_border_swell_curve() -> f64 { 0.45 }
fn default_border_corner_bulge() -> f64 { 48.0 }

fn default_border_segment_gap() -> i64 {
    4
}

/// How far the default hover color moves toward white.
const HOVER_LIGHTEN: f32 = 0.35;

/// Mix a premultiplied-alpha color toward white (which is `[a, a, a, a]` in
/// premultiplied space), keeping the alpha.
/// Curvature-matched corner-span factor for window-scale squircle corners,
/// mirroring cce-ui's `layout::corner_span_factor()` exactly: a raw
/// superellipse of exponent n at a circle's nominal radius turns tighter at
/// the diagonal, so the corner span is scaled by (n − 1)·2^(1/n)/√2 to make
/// the diagonal curvature equal the configured radius. The clients widen
/// their plate corners by this factor, so every compositor-side corner cut
/// (blur, shadow, surface clip, background rect) must widen the same way or
/// the cuts land outside the corners the clients draw. Exactly 1 at n = 2.
/// Written on config load/reload (same place the exponent is pushed into
/// scenefx), read on the render paths — f64 bits in an atomic, like
/// scenefx's own global_corner_shape.
static CORNER_SPAN_FACTOR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0x3FF0000000000000); // 1.0f64

pub fn corner_span_factor() -> f64 {
    f64::from_bits(CORNER_SPAN_FACTOR.load(std::sync::atomic::Ordering::Relaxed))
}

pub fn lighten_premultiplied(c: [f32; 4], t: f32) -> [f32; 4] {
    [
        c[0] + (c[3] - c[0]) * t,
        c[1] + (c[3] - c[1]) * t,
        c[2] + (c[3] - c[2]) * t,
        c[3],
    ]
}


#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub layout: LayoutConfig,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default, rename = "key_bindings")]
    pub key_bindings: Vec<KeybindConfig>,
    #[serde(default)]
    pub pointer_bind: Vec<PointerBindConfig>,
    #[serde(default)]
    pub mode_rule: Vec<ModeRuleConfig>,
    #[serde(default)]
    pub tag_layout: Vec<TagLayoutConfig>,
    #[serde(default)]
    pub startup: Vec<StartupConfig>,
    #[serde(default)]
    pub output: Option<OutputConfig>,
    #[serde(default)]
    pub display: HashMap<String, f64>,
    #[serde(default)]
    pub device: Vec<InputDeviceConfigRule>,
    #[serde(default)]
    pub input: Option<InputConfig>,
    #[serde(default)]
    pub gesture_bind: Vec<GestureBindConfig>,
    #[serde(default)]
    pub transparency: Option<TransparencyConfig>,
    #[serde(default)]
    pub surface: SurfaceConfig,
    #[serde(default)]
    pub window_manager: Option<WindowManagerConfig>,
    /// The `idle { }` block: display-off and sleep timeouts (seconds).
    #[serde(skip)]
    pub idle: crate::idle::IdleConfig,
}

#[derive(Debug, Deserialize)]
pub struct LayoutConfig {
    #[serde(default = "default_gap")]
    pub gap: i64,
    #[serde(default = "default_gap_top")]
    pub gap_top: i64,
    #[serde(default = "default_gap_left")]
    pub gap_left: i64,
    #[serde(default = "default_gap_right")]
    pub gap_right: i64,
    #[serde(default = "default_gap_bottom")]
    pub gap_bottom: i64,
    #[serde(default = "default_cascade_offset")]
    pub cascade_offset: i64,
    #[serde(default = "default_bar_height")]
    pub bar_height: i64,
    #[serde(default = "default_transition_duration")]
    pub transition_duration: i64,
    #[serde(default = "default_grid_gap")]
    pub grid_gap: i64,
    #[serde(default = "default_window_blur")]
    pub window_blur: bool,
    #[serde(default = "default_overlay_behavior", alias = "pinned_behavior", alias = "side_panel_behavior")]
    pub overlay_behavior: String,
    #[serde(default = "default_overlay_width", alias = "pinned_width", alias = "side_panel_width")]
    pub overlay_width: i64,
    #[serde(default = "default_overlay_position", alias = "pinned_position", alias = "side_panel_position")]
    pub overlay_position: String,
    #[serde(default = "default_overlay_border_gap", alias = "pinned_border_gap", alias = "side_panel_border_gap")]
    pub overlay_border_gap: i64,
    #[serde(default = "default_status_normal_color")]
    pub status_normal_color: String,
    #[serde(default = "default_status_background_blur")]
    pub status_background_blur: f64,
    #[serde(default = "default_window_opacity")]
    pub window_opacity: bool,
    #[serde(default = "default_status_backdrop_blur_ignore_transparent")]
    pub status_backdrop_blur_ignore_transparent: bool,
    #[serde(default = "default_window_backdrop_blur_ignore_transparent")]
    pub window_backdrop_blur_ignore_transparent: bool,
    #[serde(default = "default_status_module_hide_mode_preview")]
    pub status_module_hide_mode_preview: i64,
    #[serde(default = "default_status_module_spacing")]
    pub status_module_spacing: i64,
    /// The bar's `module { droplet }` spec string (presence enables the
    /// droplet style; the compositor's per-segment backdrop-refraction node
    /// reads the same spec the bar draws from).
    #[serde(default)]
    pub status_droplet: Option<String>,
    /// Backdrop compression for status segments, from the bar's
    /// `module { backdrop_compress }` (the minimum WCAG contrast ratio the
    /// module text must hold against any backdrop pixel) and its
    /// `module { text_color }`: `(ceil, knee, invert)` in linear luminance,
    /// see `backdrop_compress_params`. `None` = off.
    #[serde(default)]
    pub status_backdrop_compress: Option<(f32, f32, bool)>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            gap: default_gap(),
            gap_top: default_gap_top(),
            gap_left: default_gap_left(),
            gap_right: default_gap_right(),
            gap_bottom: default_gap_bottom(),
            cascade_offset: default_cascade_offset(),
            bar_height: default_bar_height(),
            transition_duration: default_transition_duration(),
            grid_gap: default_grid_gap(),
            window_blur: default_window_blur(),
            overlay_behavior: default_overlay_behavior(),
            overlay_width: default_overlay_width(),
            overlay_position: default_overlay_position(),
            overlay_border_gap: default_overlay_border_gap(),
            status_normal_color: default_status_normal_color(),
            status_background_blur: default_status_background_blur(),
            window_opacity: default_window_opacity(),
            status_backdrop_blur_ignore_transparent: default_status_backdrop_blur_ignore_transparent(),
            window_backdrop_blur_ignore_transparent: default_window_backdrop_blur_ignore_transparent(),
            status_module_hide_mode_preview: default_status_module_hide_mode_preview(),
            status_module_spacing: default_status_module_spacing(),
            status_droplet: None,
            status_backdrop_compress: None,
        }
    }
}

fn default_gap() -> i64 { 48 }
fn default_gap_top() -> i64 { 48 }
fn default_gap_left() -> i64 { 48 }
fn default_gap_right() -> i64 { 48 }
fn default_gap_bottom() -> i64 { 48 }
fn default_cascade_offset() -> i64 { 20 }
fn default_bar_height() -> i64 { 24 }
fn default_transition_duration() -> i64 { 300 }
fn default_grid_gap() -> i64 { 18 }
fn default_window_blur() -> bool { false }
fn default_overlay_behavior() -> String { "inline".to_string() }
fn default_overlay_width() -> i64 { 360 }
fn default_overlay_position() -> String { "left".to_string() }
fn default_overlay_border_gap() -> i64 { 0 }
fn default_status_normal_color() -> String { "#ccccd8".to_string() }
fn default_status_background_blur() -> f64 { 0.8 }
fn default_window_opacity() -> bool { true }
fn default_status_backdrop_blur_ignore_transparent() -> bool { true }

fn default_status_module_hide_mode_preview() -> i64 { 4 }
fn default_status_module_spacing() -> i64 {
    crate::policy::arrange::DEFAULT_STATUS_MODULE_SPACING as i64
}
fn default_window_backdrop_blur_ignore_transparent() -> bool { true }

#[derive(Debug, Deserialize)]
pub struct ModeRuleConfig {
    pub mode: String,
    pub app_id: String,
    pub title: Option<String>,
    pub single: Option<bool>,
    pub tag: Option<i64>,
    pub circular: Option<bool>,
    pub ssd: Option<bool>,
    pub over_sibling: Option<bool>,
    pub center: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TagLayoutConfig {
    pub tag: i64,
    pub mode: String,
}

pub fn parse_hex_color(hex_str: &str) -> u32 {
    let hex = hex_str.trim_matches(|c| c == '"' || c == '\'' || c == ' ');
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return 0xFFFFFFFF; // fallback to white
    }
    if let Ok(val) = u32::from_str_radix(hex, 16) {
        val
    } else {
        0xFFFFFFFF
    }
}

/// Backdrop compression parameters for text of color `text_hex` that must
/// hold a WCAG contrast `ratio` against every backdrop pixel: `(ceil, knee,
/// invert)` in linear luminance, as scenefx's `wlr_scene_blur_set_compress`
/// takes them. Light text caps the backdrop at the brightest luminance that
/// still gives the ratio; dark text (`invert`) floors it at the darkest, and
/// the ceiling is then measured in the inverted image (1 - floor). The knee,
/// where compression starts, sits at half the ceiling, so a backdrop already
/// dark enough for the text is left exactly as it is. `None` for a ratio of
/// 1 or less, or an unparseable color.
///
/// The bubble's own translucent fill and droplet lighting composite on top
/// of the compressed backdrop and lift it, so the ratio reached is well
/// under the one asked for (droplet style on pure white: 4.5 asked, 2.7
/// reached; 10 asked, 4.7 reached).
pub fn backdrop_compress_params(text_hex: &str, ratio: f64) -> Option<(f32, f32, bool)> {
    if !(ratio > 1.0) {
        return None;
    }
    let hex = text_hex.trim_matches(|c| c == '"' || c == '\'' || c == ' ').trim_start_matches('#');
    if hex.len() < 6 {
        return None;
    }
    let lin = |i: usize| -> Option<f64> {
        let c = u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()? as f64 / 255.0;
        Some(if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) })
    };
    let lt = 0.2126 * lin(0)? + 0.7152 * lin(2)? + 0.0722 * lin(4)?;
    // Which way the text reads: against black or against white.
    let light = (lt + 0.05) / 0.05 >= 1.05 / (lt + 0.05);
    let ceil = if light {
        (lt + 0.05) / ratio - 0.05
    } else {
        1.0 - (ratio * (lt + 0.05) - 0.05)
    };
    // A ratio the text color cannot reach at all still compresses as hard
    // as it sensibly can rather than crushing the backdrop to black.
    let ceil = ceil.clamp(0.005, 1.0) as f32;
    Some((ceil, ceil * 0.5, !light))
}

pub fn parse_hex_color_rgba(hex_str: &str) -> [f32; 4] {
    let hex = hex_str.trim_matches(|c| c == '"' || c == '\'' || c == ' ');
    let hex = hex.trim_start_matches('#');
    if hex.len() == 8 {
        if let (Ok(r), Ok(g), Ok(b), Ok(a)) = (
            u8::from_str_radix(&hex[0..2], 16),
            u8::from_str_radix(&hex[2..4], 16),
            u8::from_str_radix(&hex[4..6], 16),
            u8::from_str_radix(&hex[6..8], 16),
        ) {
            let alpha = a as f32 / 255.0;
            return [
                (r as f32 / 255.0) * alpha,
                (g as f32 / 255.0) * alpha,
                (b as f32 / 255.0) * alpha,
                alpha,
            ];
        }
    } else if hex.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&hex[0..2], 16),
            u8::from_str_radix(&hex[2..4], 16),
            u8::from_str_radix(&hex[4..6], 16),
        ) {
            return [
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                1.0,
            ];
        }
    }
    [0.05, 0.05, 0.05, 0.05] // fallback default (5% white)
}

/// Camera reaction when the focused window goes away — see
/// `WindowManager::focus_next_visible_window`. The fallback focus itself
/// always transfers (keyboard input needs a live target); this only decides
/// what the CAMERA does about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnAppExit {
    /// Pan to the fallback-focused window (the historic behavior).
    FocusPrevious,
    /// Enter overview instead of chasing any one window.
    Overview,
    /// The camera stays exactly where the user left it.
    Nothing,
}

pub fn parse_on_app_exit(s: &str) -> OnAppExit {
    match s.to_lowercase().as_str() {
        "focus_previous" => OnAppExit::FocusPrevious,
        "overview" => OnAppExit::Overview,
        _ => OnAppExit::Nothing,
    }
}

pub fn parse_tiling_mode(s: &str) -> TilingMode {
    match s.to_lowercase().as_str() {
        "fullscreen" => TilingMode::Fullscreen,
        "popup" => TilingMode::Popup,
        "sidepanel" | "side_panel" | "side-panel" | "pinned" | "overlay" => TilingMode::Overlay,
        "status" => TilingMode::Status,
        "utility" => TilingMode::Utility,
        // "maximized" is the retired name for grid-locked windows.
        "tiled" | "maximized" => TilingMode::Tiled,
        // Everything else — including the retired "cascade"/"grid" layout
        // modes still present in old configs — is Floating.
        _ => TilingMode::Floating,
    }
}

pub fn parse_modifiers(mod_str: &str) -> u32 {
    let mut mods = 0u32;
    if mod_str.contains("super") || mod_str.contains("mod4") {
        mods |= 0x40; // RIVER_SEAT_V1_MODIFIERS_MOD4
    }
    if mod_str.contains("shift") {
        mods |= 0x01; // RIVER_SEAT_V1_MODIFIERS_SHIFT
    }
    if mod_str.contains("ctrl") {
        mods |= 0x04; // RIVER_SEAT_V1_MODIFIERS_CTRL
    }
    if mod_str.contains("alt") || mod_str.contains("mod1") {
        mods |= 0x08; // RIVER_SEAT_V1_MODIFIERS_MOD1
    }
    mods
}

/// A touchscreen edge-swipe chord: `edge_left`, `edge_right`, `edge_top`
/// or `edge_bottom` — the edge the finger starts from — optionally behind
/// modifiers (`super+edge_top`). Returns the modifier mask and the edge.
/// Kept apart from `cce_window_manager::bindings::parse_gesture`, whose
/// gestures are the touchpad's and carry a finger count.
pub fn parse_edge_gesture(chord: &str) -> Option<(u32, String)> {
    let lower = chord.trim().to_lowercase().replace('-', "_");
    let (mods, gesture) = match lower.rsplit_once('+') {
        Some((mods, gesture)) => (parse_modifiers(mods), gesture.trim()),
        None => (0, lower.as_str()),
    };
    let edge = gesture.strip_prefix("edge_")?;
    matches!(edge, "left" | "right" | "top" | "bottom").then(|| (mods, edge.to_string()))
}

pub fn parse_button(s: &str) -> u32 {
    match s.trim() {
        "left" => 0x110,   // BTN_LEFT
        "right" => 0x111,  // BTN_RIGHT
        "middle" => 0x112, // BTN_MIDDLE
        "side" => 0x113,   // BTN_SIDE
        "extra" => 0x114,  // BTN_EXTRA
        _ => s.trim().parse().unwrap_or(0),
    }
}

pub fn parse_action(s: &str) -> Action {
    let trimmed = s.trim();
    if trimmed.starts_with("spawn")
        && (trimmed.len() == 5 || trimmed.as_bytes()[5] == b' ' || trimmed.as_bytes()[5] == b'-')
    {
        Action::Spawn
    } else if trimmed == "toggle" {
        Action::Toggle
    } else if trimmed == "move" {
        Action::Move
    } else if trimmed == "resize" {
        Action::Resize
    } else {
        Action::None
    }
}

/// Warn when a spawn/toggle binding's executable can't be found (PATH lookup;
/// absolute paths are checked directly).
fn warn_if_command_missing(command: Option<&str>) {
    let Some(cmd_str) = command else { return };
    let cmd_exe = cmd_str.split_whitespace().next().unwrap_or("");
    if cmd_exe.is_empty() {
        return;
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for path_dir in std::env::split_paths(&path_var) {
            if path_dir.join(cmd_exe).is_file() {
                return;
            }
        }
        eprintln!("[WARNING] Configured keybinding command not found in PATH: {}", cmd_exe);
    }
}

pub fn parse_keysym(key_str: &str) -> u32 {
    let name = if key_str.starts_with("XKB_KEY_") {
        &key_str[8..]
    } else {
        key_str
    };
    xkbcommon::xkb::keysym_from_name(name, xkbcommon::xkb::KEYSYM_CASE_INSENSITIVE).into()
}

pub fn default_config_path() -> Option<String> {
    let xdg_config_home = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{}/.config", home)
        });
        
    let kdl_path = format!("{}/cce/config.kdl", xdg_config_home);
    if std::path::Path::new(&kdl_path).exists() {
        Some(kdl_path)
    } else {
        None
    }
}

/// `<state_home>/cce/state.json`; `None` when neither `$XDG_STATE_HOME` nor
/// `$HOME` is set, rather than a path relative to wherever we were started.
pub fn default_state_path() -> Option<String> {
    let path = cce_core::config::cce_state_dir().join("state.json");
    path.is_absolute().then(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_config() {
        if let Some(path) = default_config_path() {
            let mut server = crate::server::Server::default();
            parse_config(&path, &mut server.wm).unwrap();
            assert!(!crate::shared::layout().desktop_gap_color.is_empty());
            assert!(crate::shared::layout().desktop_gap_width >= 0);
            assert!(crate::shared::layout().root_plate_corner_radius >= 0);
            println!("TEST_WM_STARTUP: {:?}", server.wm.startup);
            println!("TEST_WM_PATH: {:?}", std::env::var("PATH"));
        }
    }

    /// RFC Phase 7a: `plate { root ... }` is the one root-plate spelling; a
    /// legacy `root plate` node is ignored (its read-alias was removed
    /// 2026-09-06), so the defaults stand when only it is present.
    #[test]
    fn test_plate_root_canonical_spelling() {
        let canonical = r##"
style {
    surface {
        plate {
            root color="#11223344" blur=0.5 corner_radius=21
        }
        backplate color="#ffffffff" blur=0.9 corner_radius=7
    }
}
"##;
        let config = parse_kdl_config(canonical).unwrap();
        assert_eq!(config.surface.root_plate_corner_radius, 21, "canonical read; legacy ignored");
        assert_eq!(config.surface.root_plate_color, "#11223344");

        // The child-node spelling of the same block — what cce-data-editor
        // writes and what a hand-kept config tends to grow into — reads the
        // same. Until 2026-09-28 it did not, and the compositor sat on its
        // default radius under root plates the apps drew at the config's.
        let children = r##"
style {
    surface {
        plate {
            root {
                blur (f64)0.1
                color (rgba)"#5e657acf"
                corner_radius (i64)24
            }
        }
    }
}
"##;
        let config = parse_kdl_config(children).unwrap();
        assert_eq!(config.surface.root_plate_corner_radius, 24, "child-node spelling");
        assert_eq!(config.surface.root_plate_color, "#5e657acf");
        assert!((config.surface.root_plate_blur - 0.1).abs() < 1e-9);

        // The legacy spelling alone is not read: the defaults stand.
        let legacy = r##"
style {
    surface {
        backplate color="#55667788" corner_radius=9
    }
}
"##;
        let config = parse_kdl_config(legacy).unwrap();
        assert_eq!(config.surface.root_plate_corner_radius, default_root_plate_corner_radius(), "legacy spelling is not read");
        assert_eq!(config.surface.root_plate_color, default_root_plate_color());
    }

    #[test]
    fn test_parse_rgba() {
        let color = parse_hex_color_rgba("#a5cfc2");
        assert_eq!(color, [165.0/255.0, 207.0/255.0, 194.0/255.0, 1.0]);
    }

    #[test]
    fn test_kdl_display_scale_parsing() {
        let content = r#"
            output {
                eDP-1 {
                    scale (f64)2.0
                    brightness_up (keybind)"XF86MonBrightnessUp"
                    brightness_down (keybind)"XF86MonBrightnessDown"
                    brightness_interval (i64)10
                }
                DP-1 {
                    scale (f64)1.5
                }
            }
        "#;
        let config = parse_kdl_config(content).unwrap();
        assert_eq!(config.display.get("scale_eDP-1"), Some(&2.0));
        assert_eq!(config.display.get("scale_DP-1"), Some(&1.5));
        assert_eq!(config.display.get("mm_w_eDP-1"), None);
        assert_eq!(config.display.get("brightness_interval_eDP-1"), Some(&10.0));

        let up_bind = config.key_bindings.iter().find(|kb| kb.key == "XF86MonBrightnessUp").unwrap();
        assert_eq!(up_bind.action, "spawn");
        assert_eq!(up_bind.command, Some("brightnessctl set 10%+".to_string()));

        let down_bind = config.key_bindings.iter().find(|kb| kb.key == "XF86MonBrightnessDown").unwrap();
        assert_eq!(down_bind.action, "spawn");
        assert_eq!(down_bind.command, Some("brightnessctl set 10%-".to_string()));
    }

    #[test]
    fn test_kdl_display_size_mm_parsing() {
        let content = r#"
            output {
                eDP-1 scale=(f64)2.0 size_mm="344x215"
                DP-1 {
                    scale (f64)1.5
                    size_mm "597 336"
                }
                HDMI-A-1 size_mm="bogus"
            }
        "#;
        let config = parse_kdl_config(content).unwrap();
        assert_eq!(config.display.get("mm_w_eDP-1"), Some(&344.0));
        assert_eq!(config.display.get("mm_h_eDP-1"), Some(&215.0));
        assert_eq!(config.display.get("mm_w_DP-1"), Some(&597.0));
        assert_eq!(config.display.get("mm_h_DP-1"), Some(&336.0));
        assert_eq!(config.display.get("mm_w_HDMI-A-1"), None);
        assert_eq!(parse_size_mm("344,215"), Some((344.0, 215.0)));
        assert_eq!(parse_size_mm("344x0"), None);
        assert_eq!(parse_size_mm("1x2x3"), None);
    }

    #[test]
    fn test_kdl_window_manager_parsing() {
        let content = r#"
            "window_manager" {
                close_window (keybind)"super+q"
                toggle_fullscreen (keybind)"super+f"
                toggle_overview ("menu:swipe_up,swipe_down,swipe_left,swipe_right,pinch_in,pinch_out")"swipe_up"
                swipe_peek (f64)40.0
                swipe_repeat_peek (f64)15.0
                swipe_focus_cone (f64)30.0
                swipe_threshold (f64)80.0
                swipe_repeat_threshold (f64)200.0
            }
        "#;
        let config = parse_kdl_config(content).unwrap();
        assert!(config.window_manager.is_some());
        let wm = config.window_manager.unwrap();
        assert_eq!(wm.close_window, Some("super+q".to_string()));
        assert_eq!(wm.toggle_fullscreen, Some("super+f".to_string()));
        assert_eq!(wm.toggle_overview, Some("swipe_up".to_string()));
        assert_eq!(wm.swipe_peek, Some(40.0));
        assert_eq!(wm.swipe_repeat_peek, Some(15.0));
        assert_eq!(wm.swipe_focus_cone, Some(30.0));
        assert_eq!(wm.swipe_threshold, Some(80.0));
        assert_eq!(wm.swipe_repeat_threshold, Some(200.0));
        // Absent means "unset", which the apply step reads as the centring default.
        assert_eq!(wm.center_on_spawn, None);
    }

    #[test]
    fn test_kdl_window_manager_center_on_spawn() {
        let off = parse_kdl_config(
            r#"
            window_manager {
                center_on_spawn (bool)false
            }
        "#,
        )
        .unwrap();
        assert_eq!(off.window_manager.unwrap().center_on_spawn, Some(false));

        let on = parse_kdl_config(
            r#"
            window_manager {
                center_on_spawn (bool)true
            }
        "#,
        )
        .unwrap();
        assert_eq!(on.window_manager.unwrap().center_on_spawn, Some(true));

        // No window_manager block at all: nothing to read, and the apply step defaults on.
        assert!(parse_kdl_config("layout {\n gap 4\n}").unwrap().window_manager.is_none());
    }

    #[test]
    fn test_kdl_window_manager_rounded_apps() {
        let listed = parse_kdl_config(
            r#"
            window_manager {
                rounded_apps "claude-desktop" "org.keepassxc.KeePassXC"
            }
        "#,
        )
        .unwrap();
        assert_eq!(
            listed.window_manager.unwrap().rounded_apps,
            Some(vec!["claude-desktop".to_string(), "org.keepassxc.KeePassXC".to_string()])
        );

        // Absent means "unset": the apply step reads it as an empty allowlist.
        let absent = parse_kdl_config(
            r#"
            window_manager {
                center_on_spawn (bool)true
            }
        "#,
        )
        .unwrap();
        assert_eq!(absent.window_manager.unwrap().rounded_apps, None);
    }

    #[test]
    fn test_kdl_window_manager_corner_shape() {
        let set = parse_kdl_config(
            r#"
            window_manager {
                corner_shape (f64)4.5
            }
        "#,
        )
        .unwrap();
        assert_eq!(set.window_manager.unwrap().corner_shape, Some(4.5));

        // Absent means "unset"; the apply step then feeds scenefx the circular default.
        let unset = parse_kdl_config("window_manager {\n center_on_spawn (bool)true\n}").unwrap();
        assert_eq!(unset.window_manager.unwrap().corner_shape, None);
    }

    #[test]
    fn test_kdl_input_device_classes_parsing() {
        let content = r#"
            input {
                accel_profile "flat"
                accel_speed (f64)1.0
                scroll_factor (f64)1.0
                scroll_ease (f64)9.5
                kinetic_scroll (bool)false
                scroll_friction (f64)4.0
                mouse {
                    accel_speed (f64)0.5
                    scroll_factor (f64)2.0
                    scroll_method "button"
                }
                trackpad {
                    tap_to_click (bool)true
                    natural_scroll (bool)true
                    scroll_factor (f64)1.5
                    accel_speed (f64)0.9
                }
                trackpoint {
                    accel_speed (f64)0.4
                    accel_profile "adaptive"
                    scroll_factor (f64)3.0
                    scroll_method ("menu:none,button,two_finger,edge")"none"
                }
            }
        "#;
        let config = parse_kdl_config(content).unwrap();
        let input = config.input.unwrap();
        assert_eq!(input.accel_profile, Some("flat".to_string()));
        assert_eq!(input.accel_speed, Some(1.0));
        assert_eq!(input.scroll_factor, Some(1.0));
        assert_eq!(input.scroll_ease, Some(9.5));
        assert_eq!(input.kinetic_scroll, Some(false));
        assert_eq!(input.scroll_friction, Some(4.0));
        let mouse = input.mouse.unwrap();
        assert_eq!(mouse.accel_speed, Some(0.5));
        assert_eq!(mouse.scroll_factor, Some(2.0));
        assert_eq!(mouse.scroll_method, Some("button".to_string()));
        // `trackpad` parses into the touchpad block (input.kdl spelling).
        let tp = input.touchpad.unwrap();
        assert_eq!(tp.tap_to_click, Some(true));
        assert_eq!(tp.natural_scroll, Some(true));
        assert_eq!(tp.scroll_factor, Some(1.5));
        assert_eq!(tp.accel_speed, Some(0.9));
        let tpoint = input.trackpoint.unwrap();
        assert_eq!(tpoint.accel_speed, Some(0.4));
        assert_eq!(tpoint.accel_profile, Some("adaptive".to_string()));
        assert_eq!(tpoint.scroll_factor, Some(3.0));
        // The annotated spelling the settings UI writes parses the same.
        assert_eq!(tpoint.scroll_method, Some("none".to_string()));
    }

    #[test]
    fn test_kdl_input_key_repeat() {
        let input = parse_kdl_config("input {\n repeat_rate 30\n repeat_delay 200\n}\n")
            .unwrap()
            .input
            .unwrap();
        assert_eq!(input.repeat_rate, Some(30));
        assert_eq!(input.repeat_delay, Some(200));
        assert_eq!(input.repeat_info(), (30, 200));

        // Unset keys fall back to the defaults; negatives clamp to 0.
        let input = parse_kdl_config("input {\n repeat_delay -5\n}\n").unwrap().input.unwrap();
        assert_eq!(
            input.repeat_info(),
            (crate::keyboard::DEFAULT_REPEAT_RATE, 0)
        );
        assert_eq!(
            InputConfig::default().repeat_info(),
            (crate::keyboard::DEFAULT_REPEAT_RATE, crate::keyboard::DEFAULT_REPEAT_DELAY)
        );
    }

    #[test]
    fn test_parse_edge_gesture() {
        assert_eq!(parse_edge_gesture("edge_left"), Some((0, "left".to_string())));
        assert_eq!(parse_edge_gesture("Edge-Bottom"), Some((0, "bottom".to_string())));
        assert_eq!(parse_edge_gesture("super+edge_top"), Some((0x40, "top".to_string())));
        assert_eq!(parse_edge_gesture("edge_middle"), None);
        assert_eq!(parse_edge_gesture("swipe3_left"), None);
        assert_eq!(parse_edge_gesture("super+t"), None);
    }

    #[test]
    fn test_kdl_osk_on_touch() {
        let off = parse_kdl_config("window_manager {\n osk_on_touch (bool)false\n}").unwrap();
        assert_eq!(off.window_manager.unwrap().osk_on_touch, Some(false));
        let unset = parse_kdl_config("window_manager {\n}").unwrap();
        assert_eq!(unset.window_manager.unwrap().osk_on_touch, None);
    }

    #[test]
    fn test_kdl_touchpad_hscroll_shift_apps() {
        let content = r#"
            window_manager {
                touchpad_view_apps "Houdini FX"
                touchpad_hscroll_shift_apps "Houdini FX" "hython*"
            }
        "#;
        let config = parse_kdl_config(content).unwrap();
        let wm = config.window_manager.unwrap();
        assert_eq!(wm.touchpad_view_apps, Some(vec!["Houdini FX".to_string()]));
        assert_eq!(wm.touchpad_hscroll_shift_apps, Some(vec!["Houdini FX".to_string(), "hython*".to_string()]));
        // Absent, the list is empty and the emulation is off.
        let config = parse_kdl_config("window_manager { }").unwrap();
        assert_eq!(config.window_manager.unwrap().touchpad_hscroll_shift_apps, None);
    }

    #[test]
    fn test_kdl_surface_border_parsing() {
        let content = r##"
            style {
                surface {
                    border width=2 color="#ff8800" color_focused="#00ff88" color_hover="#88ffcc" segment_gap=6 corner_length=24
                }
            }
        "##;
        let config = parse_kdl_config(content).unwrap();
        assert_eq!(config.surface.border_width, 2);
        assert_eq!(config.surface.border_color, "#ff8800");
        assert_eq!(config.surface.border_color_focused, Some("#00ff88".to_string()));
        assert_eq!(config.surface.border_color_hover, Some("#88ffcc".to_string()));
        assert_eq!(config.surface.border_segment_gap, 6);
        assert_eq!(config.surface.border_corner_length, 24);

        // Defaults keep borders off; the focused color falls back to `color`
        // and the hover color to a lightened focused color.
        let config = parse_kdl_config("").unwrap();
        assert_eq!(config.surface.border_width, 0);
        assert_eq!(config.surface.border_color_focused, None);
        assert_eq!(config.surface.border_color_hover, None);
        assert_eq!(config.surface.border_segment_gap, 4);
        assert_eq!(config.surface.border_corner_length, 0);
    }

    #[test]
    fn backdrop_compress_caps_light_text_and_floors_dark_text() {
        // White text at 4.5:1 caps the backdrop at (1.05 / 4.5) - 0.05.
        let (ceil, knee, invert) = backdrop_compress_params("#ffffff", 4.5).unwrap();
        assert!(!invert);
        assert!((ceil - 0.1833).abs() < 1e-3, "{ceil}");
        assert!((knee - ceil * 0.5).abs() < 1e-6);
        // Black text floors it at 4.5 * 0.05 - 0.05 = 0.175, a ceiling of
        // 0.825 in the inverted image.
        let (ceil, _, invert) = backdrop_compress_params("#000000", 4.5).unwrap();
        assert!(invert);
        assert!((ceil - 0.825).abs() < 1e-3, "{ceil}");
        // Off, or nothing to compress for.
        assert_eq!(backdrop_compress_params("#ffffff", 1.0), None);
        assert_eq!(backdrop_compress_params("#ffffff", 0.0), None);
        assert_eq!(backdrop_compress_params("nonsense", 4.5), None);
        // A ratio the color cannot reach clamps rather than going negative.
        let (ceil, _, _) = backdrop_compress_params("#808080", 21.0).unwrap();
        assert!(ceil > 0.0);
    }
}
