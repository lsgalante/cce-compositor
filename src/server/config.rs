// KDL config parsing for monolithic cce server
 
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use crate::tiling::TilingMode;

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
    pub border_corner_radius: i32,
    /// Visual gap between the 8 border zone segments.
    pub border_segment_gap: i32,
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
    pub backplate_corner_radius: i32,
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
    pub desktop_grid_scale: f64,
    pub desktop_gap_width: i32,
    pub desktop_cell_corner_radius: i32,
    pub desktop_cell_fade_inset: i64,
    pub desktop_grid_fade_mode: String,
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
    /// Magnetic grid snap for interactive move/resize.
    pub desktop_snap: bool,
    /// Snap radius in virtual units.
    pub desktop_snap_threshold: f64,
    pub scenefx_optimized_blur: bool,
    pub status_backdrop_blur_ignore_transparent: bool,
    pub window_backdrop_blur_ignore_transparent: bool,
    pub status_module_hide_mode_preview: i64,
    pub cloud_position_default: Option<[i32; 2]>,
}

impl Layout {
    /// Snap parameters for interactive ops. A zero threshold (snap
    /// disabled) makes every snap function a no-op.
    pub fn snap_params(&self) -> crate::policy::snap::SnapParams {
        crate::policy::snap::SnapParams {
            cell_size: self.desktop_grid_scale,
            gap_width: self.desktop_gap_width as f64,
            cell_inset: self.desktop_cell_fade_inset as f64,
            threshold: if self.desktop_snap { self.desktop_snap_threshold } else { 0.0 },
        }
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
            border_corner_radius: 0,
            border_segment_gap: 4,
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
            backplate_corner_radius: 12,
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
            desktop_grid_scale: 100.0,
            desktop_gap_width: 1,
            desktop_cell_corner_radius: 0,
            desktop_cell_fade_inset: 0,
            desktop_grid_fade_mode: "linear".to_string(),
            shadow_enabled: true,
            shadow_sigma: 22.0,
            shadow_color: [0.0, 0.0, 0.0, 0.55],
            shadow_offset_x: 7,
            shadow_offset_y: 7,
            desktop_snap: true,
            desktop_snap_threshold: 24.0,
            scenefx_optimized_blur: true,
            status_backdrop_blur_ignore_transparent: true,
            window_backdrop_blur_ignore_transparent: true,
            status_module_hide_mode_preview: 4,
            cloud_position_default: None,
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
    /// Whether a newly spawned window pulls the viewport over to it. `None` = the default,
    /// which is to centre (what the compositor has always done).
    pub center_on_spawn: Option<bool>,
    /// Corner-shape exponent for scenefx's rounded-corner cuts (window
    /// surfaces, blur, shadows' clip): 2 = circular arc, > 2 = superellipse
    /// squircle. The same `window_manager.corner_shape` key the cce-ui
    /// clients read, so the compositor's cut lands on the corners they draw.
    pub corner_shape: Option<f64>,
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
}

#[derive(Debug, Deserialize, Clone, Default, PartialEq)]
pub struct MouseConfig {
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<String>,
    pub scroll_factor: Option<f64>,
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
    pub mouse: Option<MouseConfig>,
    pub touchpad: Option<TouchpadConfig>,
    pub trackpoint: Option<TrackpointConfig>,
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
    #[serde(default = "default_desktop_gap_width")]
    pub desktop_gap_width: i64,
    #[serde(default = "default_desktop_cell_corner_radius")]
    pub desktop_cell_corner_radius: i64,
    #[serde(default = "default_desktop_cell_fade_inset")]
    pub desktop_cell_fade_inset: i64,
    #[serde(default = "default_desktop_grid_fade_mode")]
    pub desktop_grid_fade_mode: String,
    #[serde(default = "default_desktop_snap")]
    pub desktop_snap: bool,
    #[serde(default = "default_desktop_snap_threshold")]
    pub desktop_snap_threshold: i64,
    #[serde(default = "default_backplate_color")]
    pub backplate_color: String,
    #[serde(default = "default_backplate_blur")]
    pub backplate_blur: f64,
    #[serde(default = "default_backplate_corner_radius")]
    pub backplate_corner_radius: i64,
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
    #[serde(default = "default_border_corner_radius")]
    pub border_corner_radius: i64,
    #[serde(default = "default_border_segment_gap")]
    pub border_segment_gap: i64,
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
}

fn default_shadow_enabled() -> bool { true }
fn default_shadow_sigma() -> f64 { 22.0 }
fn default_shadow_color() -> String { "#0000008c".to_string() }
fn default_shadow_offset_x() -> i64 { 7 }
fn default_shadow_offset_y() -> i64 { 7 }

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self {
            desktop_gap_color: default_desktop_gap_color(),
            desktop_cell_color: default_desktop_cell_color(),
            desktop_grid_scale: default_desktop_grid_scale(),
            desktop_gap_width: default_desktop_gap_width(),
            desktop_cell_corner_radius: default_desktop_cell_corner_radius(),
            desktop_cell_fade_inset: default_desktop_cell_fade_inset(),
            desktop_grid_fade_mode: default_desktop_grid_fade_mode(),
            desktop_snap: default_desktop_snap(),
            desktop_snap_threshold: default_desktop_snap_threshold(),
            backplate_color: default_backplate_color(),
            backplate_blur: default_backplate_blur(),
            backplate_corner_radius: default_backplate_corner_radius(),
            border_width: default_border_width(),
            border_color: default_border_color(),
            border_color_focused: None,
            border_color_hover: None,
            border_corner_radius: default_border_corner_radius(),
            border_segment_gap: default_border_segment_gap(),
            border_corner_length: 0,
            cloud_position_default: default_cloud_position_default(),
            shadow_enabled: default_shadow_enabled(),
            shadow_sigma: default_shadow_sigma(),
            shadow_color: default_shadow_color(),
            shadow_offset_x: default_shadow_offset_x(),
            shadow_offset_y: default_shadow_offset_y(),
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

fn default_desktop_cell_corner_radius() -> i64 {
    0
}

fn default_desktop_cell_fade_inset() -> i64 {
    0
}

fn default_desktop_grid_fade_mode() -> String {
    "linear".to_string()
}

fn default_desktop_snap() -> bool {
    true
}

fn default_desktop_snap_threshold() -> i64 {
    24
}

fn default_backplate_color() -> String {
    "#151520e6".to_string()
}

fn default_backplate_blur() -> f64 {
    0.8
}

fn default_backplate_corner_radius() -> i64 {
    12
}

fn default_border_width() -> i64 {
    0
}

fn default_border_color() -> String {
    "#3e3e3e".to_string()
}

fn default_border_corner_radius() -> i64 {
    0
}

fn default_border_segment_gap() -> i64 {
    4
}

/// How far the default hover color moves toward white.
const HOVER_LIGHTEN: f32 = 0.35;

/// Mix a premultiplied-alpha color toward white (which is `[a, a, a, a]` in
/// premultiplied space), keeping the alpha.
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

pub fn parse_tiling_mode(s: &str) -> TilingMode {
    match s.to_lowercase().as_str() {
        "floating" => TilingMode::Floating,
        "cascade" => TilingMode::Cascade,
        "grid" => TilingMode::Grid,
        "fullscreen" => TilingMode::Fullscreen,
        "popup" => TilingMode::Popup,
        "sidepanel" | "side_panel" | "side-panel" | "pinned" | "overlay" => TilingMode::Overlay,
        "status" => TilingMode::Status,
        "maximized" => TilingMode::Maximized,
        _ => TilingMode::Cascade,
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

pub fn default_state_path() -> Option<String> {
    if let Ok(xdg_state_home) = std::env::var("XDG_STATE_HOME") {
        Some(format!("{}/cce/state.json", xdg_state_home))
    } else if let Ok(home) = std::env::var("HOME") {
        Some(format!("{}/.local/state/cce/state.json", home))
    } else {
        None
    }
}

pub fn extract_program_name(cmd: &str) -> String {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let first_token = trimmed.split_whitespace().next().unwrap_or("");
    if let Some(pos) = first_token.rfind('/') {
        first_token[pos+1..].to_string()
    } else {
        first_token.to_string()
    }
}

fn expand_env_vars(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() {
            if chars[i + 1] == '{' {
                // ${VAR} form
                if let Some(end) = chars[i + 2..].iter().position(|c| *c == '}') {
                    let var_name: String = chars[i + 2..i + 2 + end].iter().collect();
                    let val = std::env::var(&var_name).unwrap_or_default();
                    result.push_str(&val);
                    i = i + 2 + end + 1; // skip ${VAR}
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            } else if chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_' {
                // $VAR form — name is [A-Za-z_][A-Za-z0-9_]*
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_')
                {
                    end += 1;
                }
                let var_name: String = chars[start..end].iter().collect();
                let val = std::env::var(&var_name).unwrap_or_default();
                result.push_str(&val);
                i = end;
            } else {
                // $ followed by non-identifier char (e.g. $$, $:, $@) — keep as-is
                result.push(chars[i]);
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn get_child_arg_i64(node: &kdl::KdlNode, child_name: &str, default: i64) -> i64 {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_i64().unwrap_or(default);
                }
            }
        }
    }
    default
}

fn get_child_arg_f64(node: &kdl::KdlNode, child_name: &str, default: f64) -> f64 {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    let mut val = entry.value().as_f64().unwrap_or(default);
                    if let Some(ty) = entry.ty() {
                        let ty_str = ty.value();
                        if ty_str.starts_with("f64:") {
                            let range_str = ty_str.trim_start_matches("f64:");
                            if let Some(dash_idx) = range_str.find('-') {
                                let min_str = &range_str[..dash_idx].trim();
                                let max_str = &range_str[dash_idx + 1..].trim();
                                if let (Ok(min_f), Ok(max_f)) = (min_str.parse::<f64>(), max_str.parse::<f64>()) {
                                    val = val.clamp(min_f, max_f);
                                }
                            }
                        }
                    }
                    return val;
                }
            }
        }
    }
    default
}

fn get_child_arg_bool(node: &kdl::KdlNode, child_name: &str, default: bool) -> bool {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_bool().unwrap_or(default);
                }
            }
        }
    }
    default
}

fn get_child_arg_string(node: &kdl::KdlNode, child_name: &str, default: &str) -> String {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_string().map(|s| s.to_string()).unwrap_or_else(|| default.to_string());
                }
            }
        }
    }
    default.to_string()
}

fn get_child_arg_bool_opt(node: &kdl::KdlNode, child_name: &str) -> Option<bool> {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_bool();
                }
            }
        }
    }
    None
}

fn get_child_arg_f64_opt(node: &kdl::KdlNode, child_name: &str) -> Option<f64> {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    let mut val = entry.value().as_f64()?;
                    if let Some(ty) = entry.ty() {
                        let ty_str = ty.value();
                        if ty_str.starts_with("f64:") {
                            let range_str = ty_str.trim_start_matches("f64:");
                            if let Some(dash_idx) = range_str.find('-') {
                                let min_str = &range_str[..dash_idx].trim();
                                let max_str = &range_str[dash_idx + 1..].trim();
                                if let (Ok(min_f), Ok(max_f)) = (min_str.parse::<f64>(), max_str.parse::<f64>()) {
                                    val = val.clamp(min_f, max_f);
                                }
                            }
                        }
                    }
                    return Some(val);
                }
            }
        }
    }
    None
}

fn get_child_arg_vec2i_opt(node: &kdl::KdlNode, child_name: &str) -> Option<[i32; 2]> {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                let entries = child.entries();
                if entries.len() >= 2 {
                    let has_tag = entries.first().and_then(|e| e.ty()).map_or(false, |t| t.value() == "vec2i");
                    if has_tag {
                        let x = entries[0].value().as_i64()? as i32;
                        let y = entries[1].value().as_i64()? as i32;
                        return Some([x, y]);
                    }
                }
            }
        }
    }
    None
}


fn get_child_arg_string_opt(node: &kdl::KdlNode, child_name: &str) -> Option<String> {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_string().map(|s| s.to_string());
                }
            }
        }
    }
    None
}

fn get_prop_string(node: &kdl::KdlNode, key: &str, default: &str) -> String {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_string().map(|s| s.to_string()).unwrap_or_else(|| default.to_string());
            }
        }
    }
    default.to_string()
}

fn get_prop_string_opt(node: &kdl::KdlNode, key: &str) -> Option<String> {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_string().map(|s| s.to_string());
            }
        }
    }
    None
}

fn get_prop_i64(node: &kdl::KdlNode, key: &str, default: i64) -> i64 {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_i64().unwrap_or(default);
            }
        }
    }
    default
}

fn get_prop_bool(node: &kdl::KdlNode, key: &str, default: bool) -> bool {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_bool().unwrap_or(default);
            }
        }
    }
    default
}

fn get_prop_bool_opt(node: &kdl::KdlNode, key: &str) -> Option<bool> {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_bool();
            }
        }
    }
    None
}

fn get_prop_i64_opt(node: &kdl::KdlNode, key: &str) -> Option<i64> {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_i64();
            }
        }
    }
    None
}

fn get_prop_f64_opt(node: &kdl::KdlNode, key: &str) -> Option<f64> {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                let mut val = entry.value().as_f64()?;
                if let Some(ty) = entry.ty() {
                    let ty_str = ty.value();
                    if ty_str.starts_with("f64:") {
                        let range_str = ty_str.trim_start_matches("f64:");
                        if let Some(dash_idx) = range_str.find('-') {
                            let min_str = &range_str[..dash_idx].trim();
                            let max_str = &range_str[dash_idx + 1..].trim();
                            if let (Ok(min_f), Ok(max_f)) = (min_str.parse::<f64>(), max_str.parse::<f64>()) {
                                val = val.clamp(min_f, max_f);
                            }
                        }
                    }
                }
                return Some(val);
            }
        }
    }
    None
}

fn get_nested_prop_string(node: &kdl::KdlNode, child_name: &str, prop_name: &str, default: &str) -> String {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                for entry in child.entries() {
                    if let Some(id) = entry.name() {
                        if id.value() == prop_name {
                            return entry.value().as_string().map(|s| s.to_string()).unwrap_or_else(|| default.to_string());
                        }
                    }
                }
            }
        }
    }
    default.to_string()
}

fn get_nested_prop_i64(node: &kdl::KdlNode, child_name: &str, prop_name: &str, default: i64) -> i64 {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                for entry in child.entries() {
                    if let Some(id) = entry.name() {
                        if id.value() == prop_name {
                            return entry.value().as_i64().unwrap_or(default);
                        }
                    }
                }
            }
        }
    }
    default
}

fn get_nested_prop_bool(node: &kdl::KdlNode, child_name: &str, prop_name: &str, default: bool) -> bool {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                for entry in child.entries() {
                    if let Some(id) = entry.name() {
                        if id.value() == prop_name {
                            return entry.value().as_bool().unwrap_or(default);
                        }
                    }
                }
            }
        }
    }
    default
}

fn get_nested_prop_f64(node: &kdl::KdlNode, child_name: &str, prop_name: &str, default: f64) -> f64 {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                for entry in child.entries() {
                    if let Some(id) = entry.name() {
                        if id.value() == prop_name {
                            let mut val = entry.value().as_f64().unwrap_or(default);
                            if let Some(ty) = entry.ty() {
                                let ty_str = ty.value();
                                if ty_str.starts_with("f64:") {
                                    let range_str = ty_str.trim_start_matches("f64:");
                                    if let Some(dash_idx) = range_str.find('-') {
                                        let min_str = &range_str[..dash_idx].trim();
                                        let max_str = &range_str[dash_idx + 1..].trim();
                                        if let (Ok(min_f), Ok(max_f)) = (min_str.parse::<f64>(), max_str.parse::<f64>()) {
                                            val = val.clamp(min_f, max_f);
                                        }
                                    }
                                }
                            }
                            return val;
                        }
                    }
                }
            }
        }
    }
    default
}

fn parse_kdl_config(content: &str) -> Result<Config, String> {
    let doc: kdl::KdlDocument = content.parse().map_err(|e| format!("KDL parse error: {}", e))?;
    
    // 1. layout & style
    let mut layout = LayoutConfig::default();
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "layout") {
        layout.gap = get_child_arg_i64(node, "gap", default_gap());
        layout.gap_top = get_child_arg_i64(node, "gap_top", default_gap_top());
        layout.gap_left = get_child_arg_i64(node, "gap_left", default_gap_left());
        layout.gap_right = get_child_arg_i64(node, "gap_right", default_gap_right());
        layout.gap_bottom = get_child_arg_i64(node, "gap_bottom", default_gap_bottom());
        layout.cascade_offset = get_child_arg_i64(node, "cascade_offset", default_cascade_offset());
        layout.bar_height = get_child_arg_i64(node, "bar_height", default_bar_height());
        layout.grid_gap = get_child_arg_i64(node, "grid_gap", default_grid_gap());
    }
    
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "style") {
        layout.transition_duration = get_nested_prop_i64(node, "window", "transition_duration", default_transition_duration());
        layout.window_backdrop_blur_ignore_transparent = get_nested_prop_bool(node, "window", "backdrop_blur_ignore_transparent", default_window_backdrop_blur_ignore_transparent());
        
        layout.overlay_behavior = get_nested_prop_string(node, "overlay", "behavior", &default_overlay_behavior());
        layout.overlay_width = get_nested_prop_i64(node, "overlay", "width", default_overlay_width());
        layout.overlay_position = get_nested_prop_string(node, "overlay", "position", &default_overlay_position());
        layout.overlay_border_gap = get_nested_prop_i64(node, "overlay", "border_gap", default_overlay_border_gap());
        
        layout.status_normal_color = get_nested_prop_string(node, "status", "normal_color", &default_status_normal_color());
        layout.status_background_blur = get_nested_prop_f64(node, "status", "background_blur", default_status_background_blur());
        layout.status_backdrop_blur_ignore_transparent = get_nested_prop_bool(node, "status", "backdrop_blur_ignore_transparent", default_status_backdrop_blur_ignore_transparent());
        layout.status_module_hide_mode_preview = get_nested_prop_i64(node, "status", "module_hide_mode_preview", default_status_module_hide_mode_preview());
    }

    // 2. env
    let mut env = HashMap::new();
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "env") {
        if let Some(children) = node.children() {
            for child in children.nodes() {
                if let Some(entry) = child.entries().first() {
                    if let Some(val) = entry.value().as_string() {
                        env.insert(child.name().value().to_string(), val.to_string());
                    }
                }
            }
        }
    }

    // 3. lists
    let mut key_bindings = Vec::new();
    let mut pointer_bind = Vec::new();
    let mut gesture_bind = Vec::new();
    let mut mode_rule = Vec::new();
    let mut tag_layout = Vec::new();
    let mut startup = Vec::new();
    let mut device = Vec::new();
    
    for node in doc.nodes() {
        match node.name().value() {
            "key_bindings" => {
                if let Some(children) = node.children() {
                    for child in children.nodes() {
                        if child.name().value() == "bind" {
                            let mods = get_prop_string(child, "mods", "");
                            let key = get_prop_string(child, "key", "");
                            let action = get_prop_string(child, "action", "");
                            let command = get_prop_string_opt(child, "command");
                            key_bindings.push(KeybindConfig { mods, key, action, command });
                        }
                    }
                } else {
                    let mods = get_prop_string(node, "mods", "");
                    let key = get_prop_string(node, "key", "");
                    let action = get_prop_string(node, "action", "");
                    let command = get_prop_string_opt(node, "command");
                    key_bindings.push(KeybindConfig { mods, key, action, command });
                }
            }
            "pointer_bind" => {
                let mods = get_prop_string(node, "mods", "");
                let button = get_prop_string(node, "button", "");
                let action = get_prop_string(node, "action", "");
                pointer_bind.push(PointerBindConfig { mods, button, action });
            }
            "gesture_bind" => {
                let mods = get_prop_string_opt(node, "mods");
                let gesture_type = get_prop_string(node, "type", "");
                let fingers = get_prop_i64(node, "fingers", 0) as u32;
                let direction = get_prop_string(node, "direction", "");
                let action = get_prop_string(node, "action", "");
                let command = get_prop_string_opt(node, "command");
                gesture_bind.push(GestureBindConfig { mods, gesture_type, fingers, direction, action, command });
            }
            "mode_rule" => {
                let mode = get_prop_string(node, "mode", "");
                let app_id = get_prop_string(node, "app_id", "");
                let title = get_prop_string_opt(node, "title");
                let single = get_prop_bool_opt(node, "single");
                let tag = get_prop_i64_opt(node, "tag");
                let circular = get_prop_bool_opt(node, "circular");
                let ssd = get_prop_bool_opt(node, "ssd");
                mode_rule.push(ModeRuleConfig { mode, app_id, title, single, tag, circular, ssd });
            }
            "tag_layout" => {
                let tag = get_prop_i64(node, "tag", 0);
                let mode = get_prop_string(node, "mode", "");
                tag_layout.push(TagLayoutConfig { tag, mode });
            }
            "startup" => {
                let exec = get_prop_string(node, "exec", "");
                let once = get_prop_bool(node, "once", false);
                let restart = get_prop_bool(node, "restart", false);
                startup.push(StartupConfig { exec, once, restart });
            }
            "device" => {
                let name = get_prop_string(node, "name", "");
                let scroll_factor = get_prop_f64_opt(node, "scroll_factor");
                device.push(InputDeviceConfigRule { name, scroll_factor });
            }
            _ => {}
        }
    }

    if key_bindings.is_empty() {
        if let Some(input_node) = doc.nodes().iter().find(|n| n.name().value() == "input") {
            if let Some(children) = input_node.children() {
                for child_node in children.nodes() {
                    if child_node.name().value() == "key_bindings" {
                        if let Some(bind_children) = child_node.children() {
                            for child in bind_children.nodes() {
                                if child.name().value() == "bind" {
                                    let mods = get_prop_string(child, "mods", "");
                                    let key = get_prop_string(child, "key", "");
                                    let action = get_prop_string(child, "action", "");
                                    let command = get_prop_string_opt(child, "command");
                                    key_bindings.push(KeybindConfig { mods, key, action, command });
                                }
                            }
                        } else {
                            let mods = get_prop_string(child_node, "mods", "");
                            let key = get_prop_string(child_node, "key", "");
                            let action = get_prop_string(child_node, "action", "");
                            let command = get_prop_string_opt(child_node, "command");
                            key_bindings.push(KeybindConfig { mods, key, action, command });
                        }
                    }
                }
            }
        }
    }

    // 4. output
    let mut output = None;
    let mut display = HashMap::new();
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "output") {
        let scenefx_optimized_blur = get_child_arg_bool(node, "scenefx_optimized_blur", true);
        output = Some(OutputConfig { scenefx_optimized_blur });

        if let Some(children) = node.children() {
            for child in children.nodes() {
                let name = child.name().value();
                let mut parsed_scale = None;
                if let Some(entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("scale")) {
                    if let Some(num) = entry.value().as_f64() {
                        parsed_scale = Some(num);
                    }
                }
                let mut parsed_interval = None;
                if let Some(entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("brightness_interval")) {
                    if let Some(num) = entry.value().as_i64() {
                        parsed_interval = Some(num);
                    }
                }
                let mut parsed_up = None;
                if let Some(entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("brightness_up")) {
                    if let Some(key_val) = entry.value().as_string() {
                        parsed_up = Some(key_val.to_string());
                    }
                }
                let mut parsed_down = None;
                if let Some(entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("brightness_down")) {
                    if let Some(key_val) = entry.value().as_string() {
                        parsed_down = Some(key_val.to_string());
                    }
                }

                if let Some(display_children) = child.children() {
                    if parsed_scale.is_none() {
                        if let Some(scale_node) = display_children.nodes().iter().find(|n| n.name().value() == "scale") {
                            if let Some(entry) = scale_node.entries().first() {
                                if let Some(num) = entry.value().as_f64() {
                                    parsed_scale = Some(num);
                                }
                            }
                        }
                    }
                    if parsed_interval.is_none() {
                        if let Some(interval_node) = display_children.nodes().iter().find(|n| n.name().value() == "brightness_interval") {
                            if let Some(entry) = interval_node.entries().first() {
                                if let Some(num) = entry.value().as_i64() {
                                    parsed_interval = Some(num);
                                }
                            }
                        }
                    }
                    if parsed_up.is_none() {
                        if let Some(up_node) = display_children.nodes().iter().find(|n| n.name().value() == "brightness_up") {
                            if let Some(entry) = up_node.entries().first() {
                                if let Some(key_val) = entry.value().as_string() {
                                    parsed_up = Some(key_val.to_string());
                                }
                            }
                        }
                    }
                    if parsed_down.is_none() {
                        if let Some(down_node) = display_children.nodes().iter().find(|n| n.name().value() == "brightness_down") {
                            if let Some(entry) = down_node.entries().first() {
                                if let Some(key_val) = entry.value().as_string() {
                                    parsed_down = Some(key_val.to_string());
                                }
                            }
                        }
                    }
                }

                if let Some(num) = parsed_scale {
                    display.insert(format!("scale_{}", name), num);
                }
                let interval = parsed_interval.unwrap_or(10);
                if parsed_interval.is_some() {
                    display.insert(format!("brightness_interval_{}", name), interval as f64);
                }
                if let Some(key_val) = parsed_up {
                    key_bindings.push(KeybindConfig {
                        mods: "".to_string(),
                        key: key_val,
                        action: "spawn".to_string(),
                        command: Some(format!("brightnessctl set {}%+", interval)),
                    });
                }
                if let Some(key_val) = parsed_down {
                    key_bindings.push(KeybindConfig {
                        mods: "".to_string(),
                        key: key_val,
                        action: "spawn".to_string(),
                        command: Some(format!("brightnessctl set {}%-", interval)),
                    });
                }
            }
        }
    }

    // 6. input
    let mut input = None;
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "input") {
        let accel_speed = get_child_arg_f64_opt(node, "accel_speed");
        let accel_profile = get_child_arg_string_opt(node, "accel_profile");
        let scroll_factor = get_child_arg_f64_opt(node, "scroll_factor");

        let mut touchpad = None;
        if let Some(children) = node.children() {
            // `trackpad` is the input.kdl spelling, `touchpad` the legacy one.
            if let Some(tp_node) = children
                .nodes()
                .iter()
                .find(|n| n.name().value() == "touchpad" || n.name().value() == "trackpad")
            {
                let tap_to_click = get_child_arg_bool_opt(tp_node, "tap_to_click");
                let natural_scroll = get_child_arg_bool_opt(tp_node, "natural_scroll");
                let dwt = get_child_arg_bool_opt(tp_node, "dwt");
                let dwtp = get_child_arg_bool_opt(tp_node, "dwtp");

                let mut gestures = None;
                if let Some(tp_children) = tp_node.children() {
                    if let Some(gestures_node) = tp_children.nodes().iter().find(|n| n.name().value() == "gestures") {
                        let swipe = get_child_arg_bool_opt(gestures_node, "swipe");
                        let pinch = get_child_arg_bool_opt(gestures_node, "pinch");
                        gestures = Some(GesturesConfig { swipe, pinch });
                    }
                }

                touchpad = Some(TouchpadConfig {
                    tap_to_click,
                    natural_scroll,
                    dwt,
                    dwtp,
                    gestures,
                    accel_speed: get_child_arg_f64_opt(tp_node, "accel_speed"),
                    accel_profile: get_child_arg_string_opt(tp_node, "accel_profile"),
                    scroll_factor: get_child_arg_f64_opt(tp_node, "scroll_factor"),
                });
            }
        }

        let mut trackpoint = None;
        if let Some(children) = node.children() {
            if let Some(tp_node) = children.nodes().iter().find(|n| n.name().value() == "trackpoint") {
                let accel_speed = get_child_arg_f64_opt(tp_node, "accel_speed");
                let accel_profile = get_child_arg_string_opt(tp_node, "accel_profile");
                trackpoint = Some(TrackpointConfig {
                    accel_speed,
                    accel_profile,
                    scroll_factor: get_child_arg_f64_opt(tp_node, "scroll_factor"),
                });
            }
        }

        let mut mouse = None;
        if let Some(children) = node.children() {
            if let Some(m_node) = children.nodes().iter().find(|n| n.name().value() == "mouse") {
                mouse = Some(MouseConfig {
                    accel_speed: get_child_arg_f64_opt(m_node, "accel_speed"),
                    accel_profile: get_child_arg_string_opt(m_node, "accel_profile"),
                    scroll_factor: get_child_arg_f64_opt(m_node, "scroll_factor"),
                });
            }
        }

        input = Some(InputConfig {
            accel_speed,
            accel_profile,
            scroll_factor,
            mouse,
            touchpad,
            trackpoint,
        });
    }

    // 7. transparency
    let mut transparency = None;
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "transparency") {
        let opacity = get_child_arg_f64(node, "opacity", 0.9);
        transparency = Some(TransparencyConfig { opacity: Some(opacity) });
    }

    // 8. surface
    let mut surface = SurfaceConfig::default();
    let mut found_nested = false;
    if let Some(style_node) = doc.nodes().iter().find(|n| n.name().value() == "style") {
        if let Some(style_children) = style_node.children() {
            if let Some(surface_node) = style_children.nodes().iter().find(|n| n.name().value() == "surface") {
                if let Some(surface_children) = surface_node.children() {
                    if let Some(desktop_node) = surface_children.nodes().iter().find(|n| n.name().value() == "desktop") {
                        found_nested = true;
                        for entry in desktop_node.entries() {
                            if let Some(id) = entry.name() {
                                match id.value() {
                                    "gap_color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.desktop_gap_color = val.to_string();
                                        }
                                    }
                                    "cell_color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.desktop_cell_color = val.to_string();
                                        }
                                    }
                                    "gap_width" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_gap_width = val;
                                        }
                                    }
                                    "cell_corner_radius" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_cell_corner_radius = val;
                                        }
                                    }
                                    "cell_fade_inset" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_cell_fade_inset = val;
                                        }
                                    }
                                    "grid_fade_mode" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.desktop_grid_fade_mode = val.to_string();
                                        }
                                    }
                                    "grid_cell_size" | "desktop_grid_scale" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_grid_scale = val;
                                        }
                                    }
                                    "snap" => {
                                        if let Some(val) = entry.value().as_bool() {
                                            surface.desktop_snap = val;
                                        }
                                    }
                                    "snap_threshold" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_snap_threshold = val;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if let Some(backplate_node) = surface_children.nodes().iter().find(|n| n.name().value() == "backplate") {
                        found_nested = true;
                        for entry in backplate_node.entries() {
                            if let Some(id) = entry.name() {
                                match id.value() {
                                    "color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.backplate_color = val.to_string();
                                        }
                                    }
                                    "blur" => {
                                        if let Some(mut val) = entry.value().as_f64() {
                                            if let Some(ty) = entry.ty() {
                                                let ty_str = ty.value();
                                                if ty_str.starts_with("f64:") {
                                                    let range_str = ty_str.trim_start_matches("f64:");
                                                    if let Some(dash_idx) = range_str.find('-') {
                                                        let min_str = &range_str[..dash_idx].trim();
                                                        let max_str = &range_str[dash_idx + 1..].trim();
                                                        if let (Ok(min_f), Ok(max_f)) = (min_str.parse::<f64>(), max_str.parse::<f64>()) {
                                                            val = val.clamp(min_f, max_f);
                                                        }
                                                    }
                                                }
                                            }
                                            surface.backplate_blur = val;
                                        }
                                    }
                                    "corner_radius" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.backplate_corner_radius = val;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if let Some(border_node) = surface_children.nodes().iter().find(|n| n.name().value() == "border") {
                        found_nested = true;
                        for entry in border_node.entries() {
                            if let Some(id) = entry.name() {
                                match id.value() {
                                    "width" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.border_width = val;
                                        }
                                    }
                                    "color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.border_color = val.to_string();
                                        }
                                    }
                                    "color_focused" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.border_color_focused = Some(val.to_string());
                                        }
                                    }
                                    "color_hover" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.border_color_hover = Some(val.to_string());
                                        }
                                    }
                                    "corner_radius" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.border_corner_radius = val;
                                        }
                                    }
                                    "segment_gap" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.border_segment_gap = val;
                                        }
                                    }
                                    "corner_length" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.border_corner_length = val;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if let Some(shadow_node) = surface_children.nodes().iter().find(|n| n.name().value() == "shadow") {
                        found_nested = true;
                        for entry in shadow_node.entries() {
                            if let Some(id) = entry.name() {
                                match id.value() {
                                    "enabled" => {
                                        if let Some(val) = entry.value().as_bool() {
                                            surface.shadow_enabled = val;
                                        }
                                    }
                                    // Named `blur` to match the status/backplate blur keys.
                                    "blur" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.shadow_sigma = val;
                                        } else if let Some(val) = entry.value().as_i64() {
                                            surface.shadow_sigma = val as f64;
                                        }
                                    }
                                    "color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.shadow_color = val.to_string();
                                        }
                                    }
                                    "offset_x" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.shadow_offset_x = val;
                                        }
                                    }
                                    "offset_y" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.shadow_offset_y = val;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if let Some(cloud_node) = surface_children.nodes().iter().find(|n| n.name().value() == "cloud") {
                        found_nested = true;
                        if let Some(pos) = get_child_arg_vec2i_opt(cloud_node, "position_default") {
                            surface.cloud_position_default = Some(pos);
                        }
                    }
                }
            }
        }
    }
    if !found_nested {
        if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "surface") {
            surface.desktop_gap_color = get_child_arg_string(node, "desktop_gap_color", &default_desktop_gap_color());
            surface.desktop_cell_color = get_child_arg_string(node, "desktop_cell_color", &default_desktop_cell_color());
            surface.desktop_grid_scale = get_child_arg_i64(node, "grid_cell_size", get_child_arg_i64(node, "desktop_grid_scale", default_desktop_grid_scale()));
            surface.desktop_gap_width = get_child_arg_i64(node, "desktop_gap_width", default_desktop_gap_width());
            surface.desktop_cell_corner_radius = get_child_arg_i64(node, "desktop_cell_corner_radius", default_desktop_cell_corner_radius());
            surface.desktop_cell_fade_inset = get_child_arg_i64(node, "desktop_cell_fade_inset", default_desktop_cell_fade_inset());
            surface.desktop_grid_fade_mode = get_child_arg_string(node, "grid_fade_mode", &default_desktop_grid_fade_mode());
            surface.desktop_snap = get_child_arg_bool(node, "desktop_snap", default_desktop_snap());
            surface.desktop_snap_threshold = get_child_arg_i64(node, "desktop_snap_threshold", default_desktop_snap_threshold());
            surface.backplate_color = get_child_arg_string(node, "backplate_color", &default_backplate_color());
            surface.backplate_blur = get_child_arg_f64(node, "backplate_blur", default_backplate_blur());
            surface.backplate_corner_radius = get_child_arg_i64(node, "backplate_corner_radius", default_backplate_corner_radius());
            surface.border_width = get_child_arg_i64(node, "border_width", default_border_width());
            surface.border_color = get_child_arg_string(node, "border_color", &default_border_color());
            surface.border_color_focused = get_child_arg_string_opt(node, "border_color_focused");
            surface.border_color_hover = get_child_arg_string_opt(node, "border_color_hover");
            surface.border_corner_radius = get_child_arg_i64(node, "border_corner_radius", default_border_corner_radius());
            surface.border_segment_gap = get_child_arg_i64(node, "border_segment_gap", default_border_segment_gap());
            surface.border_corner_length = get_child_arg_i64(node, "border_corner_length", 0);
            surface.cloud_position_default = get_child_arg_vec2i_opt(node, "cloud_position_default");
        }
    }

    // window manager
    let mut window_manager = None;
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "window manager" || n.name().value() == "window_manager") {
        let close_window = get_child_arg_string_opt(node, "close_window");
        let toggle_fullscreen = get_child_arg_string_opt(node, "toggle_fullscreen");
        let toggle_overview = get_child_arg_string_opt(node, "toggle_overview");
        let window_switcher = get_child_arg_string_opt(node, "window_switcher");
        let center_on_spawn = get_child_arg_bool_opt(node, "center_on_spawn");
        let corner_shape = get_child_arg_f64_opt(node, "corner_shape");
        window_manager = Some(WindowManagerConfig { close_window, toggle_fullscreen, toggle_overview, window_switcher, center_on_spawn, corner_shape });
    }

    Ok(Config {
        layout,
        env,
        key_bindings,
        pointer_bind,
        mode_rule,
        tag_layout,
        startup,
        output,
        display,
        device,
        input,
        gesture_bind,
        transparency,
        surface,
        window_manager,
    })
}

pub fn parse_config(path: &str, state: &mut crate::window_manager::WindowManager) -> Result<(), String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return Err(format!("cannot open {}: {}", path, e)),
    };

    let mut config: Config = parse_kdl_config(&content)?;

    let path_buf = std::path::Path::new(path);
    let input_path = path_buf.parent().unwrap_or_else(|| std::path::Path::new(".")).join("input.kdl");
    let mut wm_domain_entries: Vec<cce_ui::input::BindingEntry> = Vec::new();
    if input_path.exists() {
        if let Ok(input_content) = fs::read_to_string(&input_path) {
            // New domain-scoped format: a `cce-window-manager { ... }` block
            // of `<action_name> "<chord>"` bindings. Other domains belong to
            // clients/widgets and are ignored here.
            match cce_ui::input::InputConfig::parse(&input_content) {
                Ok(ic) => {
                    wm_domain_entries = ic.domain(cce_ui::input::WINDOW_MANAGER_DOMAIN).to_vec();
                }
                Err(e) => eprintln!("[WARNING] {}: {}", input_path.display(), e),
            }
            // Legacy input.kdl contents: root-level key_bindings nodes and
            // the input section.
            if let Ok(input_config) = parse_kdl_config(&input_content) {
                config.key_bindings.extend(input_config.key_bindings);
                if input_config.input.is_some() {
                    config.input = input_config.input;
                }
            }
        }
    }

    state.output_scale = 1.0f32;
    state.display = config.display.clone();
    state.center_on_spawn = config
        .window_manager
        .as_ref()
        .and_then(|wm| wm.center_on_spawn)
        .unwrap_or(true);

    // Feed scenefx's rounded-corner shaders the DE-wide corner-shape exponent
    // (clamped like cce-ui's corner_shape()). Plain C state, safe pre-renderer
    // and on live reload.
    let corner_shape = config
        .window_manager
        .as_ref()
        .and_then(|wm| wm.corner_shape)
        .unwrap_or(2.0)
        .clamp(2.0, 16.0) as f32;
    unsafe {
        crate::ffi::fx_renderer_set_corner_shape(corner_shape);
    }

    state.layout.gap = config.layout.gap as i32;
    state.layout.gap_top = config.layout.gap_top as i32;
    state.layout.gap_left = config.layout.gap_left as i32;
    state.layout.gap_right = config.layout.gap_right as i32;
    state.layout.gap_bottom = config.layout.gap_bottom as i32;
    state.layout.cascade_offset = config.layout.cascade_offset as i32;
    state.layout.bar_height = config.layout.bar_height as i32;
    state.layout.border_width = config.surface.border_width as i32;
    state.layout.fullscreen_border_width = 0;
    state.layout.cascade_border_width = 0;
    state.layout.grid_border_width = 0;
    state.layout.floating_border_width = 0;

    state.layout.border_color = parse_hex_color_rgba(&config.surface.border_color);
    state.layout.border_color_focused = config
        .surface
        .border_color_focused
        .as_deref()
        .map(parse_hex_color_rgba)
        .unwrap_or(state.layout.border_color);
    state.layout.border_color_hover = config
        .surface
        .border_color_hover
        .as_deref()
        .map(parse_hex_color_rgba)
        .unwrap_or_else(|| lighten_premultiplied(state.layout.border_color_focused, HOVER_LIGHTEN));
    state.layout.border_corner_radius = config.surface.border_corner_radius as i32;
    state.layout.border_segment_gap = config.surface.border_segment_gap.max(0) as i32;
    state.layout.border_corner_length = config.surface.border_corner_length.max(0) as i32;

    state.layout.desktop_gap_color = config.surface.desktop_gap_color.clone();

    let background_color_val = parse_hex_color(&config.surface.desktop_gap_color);
    state.layout.background_r = ((background_color_val >> 16) & 0xFF) * 0x01010101;
    state.layout.background_g = ((background_color_val >> 8) & 0xFF) * 0x01010101;
    state.layout.background_b = (background_color_val & 0xFF) * 0x01010101;
    state.layout.background_a = 0xFFFFFFFF;

    state.layout.desktop_cell_color = parse_hex_color_rgba(&config.surface.desktop_cell_color);
    state.layout.desktop_grid_scale = config.surface.desktop_grid_scale as f64;
    state.layout.desktop_snap = config.surface.desktop_snap;
    state.layout.desktop_snap_threshold = config.surface.desktop_snap_threshold.max(0) as f64;
    state.layout.desktop_gap_width = config.surface.desktop_gap_width as i32;
    state.layout.desktop_cell_corner_radius = config.surface.desktop_cell_corner_radius as i32;
    state.layout.desktop_cell_fade_inset = config.surface.desktop_cell_fade_inset;
    state.layout.desktop_grid_fade_mode = config.surface.desktop_grid_fade_mode.clone();

    state.layout.border_font_size = 11;
    state.layout.transition_duration = config.layout.transition_duration as i32;
    state.layout.grid_gap = config.layout.grid_gap as i32;
    state.layout.border_blur = false;
    state.layout.window_blur = config.surface.backplate_blur > 0.001;
    state.layout.backplate_corner_radius = config.surface.backplate_corner_radius as i32;
    state.layout.overlay_behavior = config.layout.overlay_behavior;
    state.layout.overlay_width = config.layout.overlay_width as i32;
    state.layout.overlay_position = config.layout.overlay_position;
    state.layout.overlay_border_gap = config.layout.overlay_border_gap as i32;
    state.layout.status_normal_color = config.layout.status_normal_color.clone();
    state.layout.status_background_blur = config.layout.status_background_blur as f32;
    state.layout.transparency_opacity = config.transparency.as_ref().and_then(|t| t.opacity).unwrap_or(0.9) as f32;
    let backplate_rgba = parse_hex_color_rgba(&config.surface.backplate_color);
    state.layout.window_opacity = backplate_rgba[3] < 0.999;
    state.layout.scenefx_optimized_blur = config.output.as_ref().map(|o| o.scenefx_optimized_blur).unwrap_or(true);
    state.layout.status_backdrop_blur_ignore_transparent = config.layout.status_backdrop_blur_ignore_transparent;
    state.layout.window_backdrop_blur_ignore_transparent = config.layout.window_backdrop_blur_ignore_transparent;
    state.layout.status_module_hide_mode_preview = config.layout.status_module_hide_mode_preview;
    state.layout.cloud_position_default = config.surface.cloud_position_default;
    state.layout.shadow_enabled = config.surface.shadow_enabled;
    state.layout.shadow_sigma = config.surface.shadow_sigma.max(0.0) as f32;
    state.layout.shadow_color = parse_hex_color_rgba(&config.surface.shadow_color);
    state.layout.shadow_offset_x = config.surface.shadow_offset_x as i32;
    state.layout.shadow_offset_y = config.surface.shadow_offset_y as i32;

    for (key, val) in &config.env {
        let expanded = expand_env_vars(val);
        std::env::set_var(key, &expanded);
    }

    state.input_rules = config.device.clone();
    state.input_config = config.input.clone().unwrap_or_default();
    unsafe {
        // Config first (its per-class scroll factors are defaults), then the
        // name-based device rules so they stay the most specific override.
        state.apply_input_config();
        state.apply_input_rules();
    }

    state.keybinds.clear();
    let mut table = cce_window_manager::bindings::BindingTable::new();

    // Primary source: the `cce-window-manager` domain of input.kdl.
    for entry in &wm_domain_entries {
        let Some(chord) = cce_window_manager::bindings::parse_chord(&entry.chord) else {
            eprintln!("[WARNING] input.kdl: invalid chord {:?} for {}", entry.chord, entry.name);
            continue;
        };
        let Some(action) = Action::from_name(&entry.name) else {
            eprintln!("[WARNING] input.kdl: unknown window-manager action {:?}", entry.name);
            continue;
        };
        let command = if action == Action::Spawn || action == Action::Toggle {
            warn_if_command_missing(entry.command.as_deref());
            entry.command.clone()
        } else {
            None
        };
        let keysym = parse_keysym(&chord.key);
        if keysym == 0 {
            eprintln!("[WARNING] input.kdl: unknown key {:?} in chord {:?}", chord.key, entry.chord);
            continue;
        }
        if table.add(Keybind { mods: chord.mods, keysym, action, command }) {
            eprintln!("[WARNING] input.kdl: {:?} is bound more than once", entry.chord);
        }
    }

    // Legacy sources: config.kdl `key_bindings` nodes (including the
    // synthesized brightness binds) and the `window_manager` section.
    // input.kdl wins on chord conflicts via add_default.
    let mut seen = std::collections::HashSet::new();
    for kb in &config.key_bindings {
        let (mods_str, key_str) = if kb.mods.is_empty() {
            if let Some(last_plus) = kb.key.rfind('+') {
                (kb.key[..last_plus].to_string(), kb.key[last_plus+1..].to_string())
            } else {
                ("".to_string(), kb.key.clone())
            }
        } else {
            (kb.mods.clone(), kb.key.clone())
        };
        let mods = parse_modifiers(&mods_str);
        let keysym = parse_keysym(&key_str);

        if !seen.insert((mods, keysym)) {
            eprintln!("[WARNING] Keybinding conflict: multiple actions mapped to mods={:?}, key={:?}", mods_str, key_str);
        }

        let action = parse_action(&kb.action);
        let command = if action == Action::Spawn || action == Action::Toggle {
            warn_if_command_missing(kb.command.as_deref());
            kb.command.clone()
        } else {
            None
        };
        table.add_default(Keybind { mods, keysym, action, command });
    }

    if let Some(ref wm_config) = config.window_manager {
        let wm_section_binds = [
            (&wm_config.close_window, Action::Close),
            (&wm_config.toggle_fullscreen, Action::Fullscreen),
            (&wm_config.window_switcher, Action::WindowSwitcher),
        ];
        for (chord_str, action) in wm_section_binds {
            let Some(chord_str) = chord_str else { continue };
            if let Some(chord) = cce_window_manager::bindings::parse_chord(chord_str) {
                let keysym = parse_keysym(&chord.key);
                table.add_default(Keybind { mods: chord.mods, keysym, action, command: None });
            }
        }
    }

    // Stock defaults from the policy crate; never shadow configured chords.
    for d in cce_window_manager::bindings::DEFAULT_BINDINGS {
        let keysym = parse_keysym(d.key);
        table.add_default(Keybind { mods: d.mods, keysym, action: d.action, command: None });
    }

    state.keybinds = table.into_bindings();

    state.pointer_binds.clear();
    for pb in &config.pointer_bind {
        let mods = parse_modifiers(&pb.mods);
        let button = parse_button(&pb.button);
        let action = parse_action(&pb.action);
        state.pointer_binds.push(PointerBind {
            mods,
            button,
            action,
        });
    }

    state.gesture_binds.clear();
    for gb in &config.gesture_bind {
        let mods = gb.mods.as_ref().map(|m| parse_modifiers(m)).unwrap_or(0);
        let action = parse_action(&gb.action);
        let command = if action == Action::Spawn || action == Action::Toggle {
            gb.command.clone()
        } else {
            None
        };
        state.gesture_binds.push(GestureBind {
            mods,
            gesture_type: gb.gesture_type.clone(),
            fingers: gb.fingers,
            direction: gb.direction.clone(),
            action,
            command,
        });
    }

    if let Some(ref wm_config) = config.window_manager {
        if let Some(ref toggle_ov_str) = wm_config.toggle_overview {
            let normalized = toggle_ov_str.to_lowercase().replace('-', "_");
            let gesture_type = if normalized.starts_with("swipe") {
                Some("swipe")
            } else if normalized.starts_with("pinch") {
                Some("pinch")
            } else {
                None
            };
            if let Some(g_type) = gesture_type {
                let direction = normalized.trim_start_matches(g_type).trim_start_matches('_').to_string();
                for fingers in [3, 4] {
                    state.gesture_binds.push(GestureBind {
                        mods: 0,
                        gesture_type: g_type.to_string(),
                        fingers,
                        direction: direction.clone(),
                        action: Action::Expose,
                        command: None,
                    });
                }
            }
        }
    }

    state.mode_rules.clear();
    for rule in config.mode_rule {
        state.mode_rules.push(ModeRule {
            mode: parse_tiling_mode(&rule.mode),
            app_id_pattern: rule.app_id,
            title_pattern: rule.title,
            single_instance: rule.single.unwrap_or(false),
            tag: rule.tag.unwrap_or(-1) as i32,
            circular: rule.circular.unwrap_or(false),
            ssd: rule.ssd,
        });
    }

    for _tag_layout in config.tag_layout {
        // Tag layouts are ignored in the pannable coordinate system.
    }

    state.startup.clear();
    for st in config.startup {
        state.startup.push(st);
    }

    log::info!(
        "Parsed config: {} keybinds, {} pointer binds, {} gesture binds, {} startup programs",
        state.keybinds.len(),
        state.pointer_binds.len(),
        state.gesture_binds.len(),
        state.startup.len()
    );

    unsafe {
        if !state.server.is_null() {
            let outputs_head = &mut (*state.server).om.outputs as *mut crate::ffi::wl_list as *mut crate::server::WlList;
            let mut curr = (*outputs_head).next;
            while curr != outputs_head {
                let next = (*curr).next;
                let output = &mut *crate::container_of!(curr, crate::output::Output, link);
                output.update_background_color();
                curr = next;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_config() {
        if let Some(path) = default_config_path() {
            let mut server = crate::server::Server::default();
            parse_config(&path, &mut server.wm).unwrap();
            assert!(!server.wm.layout.desktop_gap_color.is_empty());
            assert!(server.wm.layout.desktop_gap_width >= 0);
            assert!(server.wm.layout.desktop_cell_corner_radius >= 0);
            println!("TEST_WM_STARTUP: {:?}", server.wm.startup);
            println!("TEST_WM_PATH: {:?}", std::env::var("PATH"));
        }
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
        assert_eq!(config.display.get("brightness_interval_eDP-1"), Some(&10.0));

        let up_bind = config.key_bindings.iter().find(|kb| kb.key == "XF86MonBrightnessUp").unwrap();
        assert_eq!(up_bind.action, "spawn");
        assert_eq!(up_bind.command, Some("brightnessctl set 10%+".to_string()));

        let down_bind = config.key_bindings.iter().find(|kb| kb.key == "XF86MonBrightnessDown").unwrap();
        assert_eq!(down_bind.action, "spawn");
        assert_eq!(down_bind.command, Some("brightnessctl set 10%-".to_string()));
    }

    #[test]
    fn test_kdl_window_manager_parsing() {
        let content = r#"
            "window_manager" {
                close_window (keybind)"super+q"
                toggle_fullscreen (keybind)"super+f"
                toggle_overview ("menu:swipe_up,swipe_down,swipe_left,swipe_right,pinch_in,pinch_out")"swipe_up"
            }
        "#;
        let config = parse_kdl_config(content).unwrap();
        assert!(config.window_manager.is_some());
        let wm = config.window_manager.unwrap();
        assert_eq!(wm.close_window, Some("super+q".to_string()));
        assert_eq!(wm.toggle_fullscreen, Some("super+f".to_string()));
        assert_eq!(wm.toggle_overview, Some("swipe_up".to_string()));
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
                mouse {
                    accel_speed (f64)0.5
                    scroll_factor (f64)2.0
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
                }
            }
        "#;
        let config = parse_kdl_config(content).unwrap();
        let input = config.input.unwrap();
        assert_eq!(input.accel_profile, Some("flat".to_string()));
        assert_eq!(input.accel_speed, Some(1.0));
        assert_eq!(input.scroll_factor, Some(1.0));
        let mouse = input.mouse.unwrap();
        assert_eq!(mouse.accel_speed, Some(0.5));
        assert_eq!(mouse.scroll_factor, Some(2.0));
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
    }

    #[test]
    fn test_kdl_surface_border_parsing() {
        let content = r##"
            style {
                surface {
                    border width=2 color="#ff8800" color_focused="#00ff88" color_hover="#88ffcc" corner_radius=10 segment_gap=6 corner_length=24
                }
            }
        "##;
        let config = parse_kdl_config(content).unwrap();
        assert_eq!(config.surface.border_width, 2);
        assert_eq!(config.surface.border_color, "#ff8800");
        assert_eq!(config.surface.border_color_focused, Some("#00ff88".to_string()));
        assert_eq!(config.surface.border_color_hover, Some("#88ffcc".to_string()));
        assert_eq!(config.surface.border_corner_radius, 10);
        assert_eq!(config.surface.border_segment_gap, 6);
        assert_eq!(config.surface.border_corner_length, 24);

        // Defaults keep borders off; the focused color falls back to `color`
        // and the hover color to a lightened focused color.
        let config = parse_kdl_config("").unwrap();
        assert_eq!(config.surface.border_width, 0);
        assert_eq!(config.surface.border_corner_radius, 0);
        assert_eq!(config.surface.border_color_focused, None);
        assert_eq!(config.surface.border_color_hover, None);
        assert_eq!(config.surface.border_segment_gap, 4);
        assert_eq!(config.surface.border_corner_length, 0);
    }
}
