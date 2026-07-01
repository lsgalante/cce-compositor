// TOML config parsing for monolithic cce server
 
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
    pub border_r: u32,
    pub border_g: u32,
    pub border_b: u32,
    pub border_a: u32,
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
    pub desktop_enable_solid_color: bool,
    pub desktop_solid_color: [f32; 4],
    pub scenefx_optimized_blur: bool,
    pub status_backdrop_blur_ignore_transparent: bool,
    pub window_backdrop_blur_ignore_transparent: bool,
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
            border_r: 0x3E3E3E3Eu32,
            border_g: 0x3E3E3E3Eu32,
            border_b: 0x3E3E3E3Eu32,
            border_a: 0xFFFFFFFFu32,
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
            desktop_enable_solid_color: false,
            desktop_solid_color: [0.0, 0.0, 0.0, 1.0],
            scenefx_optimized_blur: true,
            status_backdrop_blur_ignore_transparent: true,
            window_backdrop_blur_ignore_transparent: true,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    Spawn,
    Toggle,
    Close,
    FocusNext,
    FocusPrev,
    Move,
    Resize,
    Exit,
    Reload,
    Fullscreen,
    LayoutNext,
    ModeNext,
    ModeNextShared,
    View1,
    View2,
    View3,
    View4,
    SetViewport1,
    SetViewport2,
    SetViewport3,
    SetViewport4,
    Expose,
    Minimize,
    OverlayLeft,
    OverlayRight,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybind {
    pub mods: u32,
    pub keysym: u32,
    pub action: Action,
    pub command: Option<String>,
}

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
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default = "default_scenefx_optimized_blur")]
    pub scenefx_optimized_blur: bool,
}

fn default_scale() -> f64 {
    1.0
}

fn default_scenefx_optimized_blur() -> bool {
    true
}

#[derive(Debug, Deserialize, Clone)]
pub struct InputDeviceConfigRule {
    pub name: String,
    pub scroll_factor: Option<f64>,
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
}

#[derive(Debug, Deserialize, Clone, Default, PartialEq)]
pub struct TrackpointConfig {
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct InputConfig {
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<String>,
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
    #[serde(default = "default_desktop_mode")]
    pub desktop_mode: String,
    #[serde(default = "default_desktop_solid_color")]
    pub desktop_solid_color: String,
    #[serde(default = "default_backplate_color")]
    pub backplate_color: String,
    #[serde(default = "default_backplate_blur")]
    pub backplate_blur: f64,
    #[serde(default = "default_backplate_corner_radius")]
    pub backplate_corner_radius: i64,
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self {
            desktop_gap_color: default_desktop_gap_color(),
            desktop_cell_color: default_desktop_cell_color(),
            desktop_grid_scale: default_desktop_grid_scale(),
            desktop_gap_width: default_desktop_gap_width(),
            desktop_cell_corner_radius: default_desktop_cell_corner_radius(),
            desktop_cell_fade_inset: default_desktop_cell_fade_inset(),
            desktop_mode: default_desktop_mode(),
            desktop_solid_color: default_desktop_solid_color(),
            backplate_color: default_backplate_color(),
            backplate_blur: default_backplate_blur(),
            backplate_corner_radius: default_backplate_corner_radius(),
        }
     }
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

fn default_desktop_mode() -> String {
    "solid".to_string()
}

fn default_desktop_solid_color() -> String {
    "#000000".to_string()
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
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/lsgalante".to_string());
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
        let scale = get_child_arg_f64(node, "scale", 1.0);
        let scenefx_optimized_blur = get_child_arg_bool(node, "scenefx_optimized_blur", true);
        output = Some(OutputConfig { scale, scenefx_optimized_blur });

        if let Some(children) = node.children() {
            for child in children.nodes() {
                let name = child.name().value();
                if name.starts_with("scale_") {
                    if let Some(entry) = child.entries().first() {
                        if let Some(num) = entry.value().as_f64() {
                            display.insert(name.to_string(), num);
                        }
                    }
                }
            }
        }
    }

    // 6. input
    let mut input = None;
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "input") {
        let accel_speed = get_child_arg_f64_opt(node, "accel_speed");
        let accel_profile = get_child_arg_string_opt(node, "accel_profile");

        let mut touchpad = None;
        if let Some(children) = node.children() {
            if let Some(tp_node) = children.nodes().iter().find(|n| n.name().value() == "touchpad") {
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
                });
            }
        }

        input = Some(InputConfig {
            accel_speed,
            accel_profile,
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
                                    "mode" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.desktop_mode = val.to_string();
                                        }
                                    }
                                    "solid_color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.desktop_solid_color = val.to_string();
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
                }
            }
        }
    }
    if !found_nested {
        if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "surface") {
            surface.desktop_gap_color = get_child_arg_string(node, "desktop_gap_color", &default_desktop_gap_color());
            surface.desktop_cell_color = get_child_arg_string(node, "desktop_cell_color", &default_desktop_cell_color());
            surface.desktop_grid_scale = get_child_arg_i64(node, "desktop_grid_scale", default_desktop_grid_scale());
            surface.desktop_gap_width = get_child_arg_i64(node, "desktop_gap_width", default_desktop_gap_width());
            surface.desktop_cell_corner_radius = get_child_arg_i64(node, "desktop_cell_corner_radius", default_desktop_cell_corner_radius());
            surface.desktop_cell_fade_inset = get_child_arg_i64(node, "desktop_cell_fade_inset", default_desktop_cell_fade_inset());
            surface.desktop_mode = get_child_arg_string(node, "desktop_mode", &default_desktop_mode());
            surface.desktop_solid_color = get_child_arg_string(node, "desktop_solid_color", &default_desktop_solid_color());
            surface.backplate_color = get_child_arg_string(node, "backplate_color", &default_backplate_color());
            surface.backplate_blur = get_child_arg_f64(node, "backplate_blur", default_backplate_blur());
            surface.backplate_corner_radius = get_child_arg_i64(node, "backplate_corner_radius", default_backplate_corner_radius());
        }
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
    })
}

pub fn parse_config(path: &str, state: &mut crate::window_manager::WindowManager) -> Result<(), String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return Err(format!("cannot open {}: {}", path, e)),
    };

    let config: Config = parse_kdl_config(&content)?;

    state.output_scale = config.output.as_ref().map(|o| o.scale as f32).unwrap_or(1.0f32);
    state.display = config.display.clone();

    state.layout.gap = config.layout.gap as i32;
    state.layout.gap_top = config.layout.gap_top as i32;
    state.layout.gap_left = config.layout.gap_left as i32;
    state.layout.gap_right = config.layout.gap_right as i32;
    state.layout.gap_bottom = config.layout.gap_bottom as i32;
    state.layout.cascade_offset = config.layout.cascade_offset as i32;
    state.layout.bar_height = config.layout.bar_height as i32;
    state.layout.border_width = 0;
    state.layout.fullscreen_border_width = 0;
    state.layout.cascade_border_width = 0;
    state.layout.grid_border_width = 0;
    state.layout.floating_border_width = 0;
    
    state.layout.border_r = 0x3E3E3E3E;
    state.layout.border_g = 0x3E3E3E3E;
    state.layout.border_b = 0x3E3E3E3E;
    state.layout.border_a = 0xFFFFFFFF;

    state.layout.desktop_gap_color = config.surface.desktop_gap_color.clone();

    let enable_solid = config.surface.desktop_mode == "solid";
    let desktop_background_str = if enable_solid {
        config.surface.desktop_solid_color.clone()
    } else {
        config.surface.desktop_gap_color.clone()
    };

    let background_color_val = parse_hex_color(&desktop_background_str);
    state.layout.background_r = ((background_color_val >> 16) & 0xFF) * 0x01010101;
    state.layout.background_g = ((background_color_val >> 8) & 0xFF) * 0x01010101;
    state.layout.background_b = (background_color_val & 0xFF) * 0x01010101;
    state.layout.background_a = 0xFFFFFFFF;

    state.layout.desktop_cell_color = parse_hex_color_rgba(&config.surface.desktop_cell_color);
    state.layout.desktop_grid_scale = config.surface.desktop_grid_scale as f64;
    state.layout.desktop_gap_width = config.surface.desktop_gap_width as i32;
    state.layout.desktop_cell_corner_radius = config.surface.desktop_cell_corner_radius as i32;
    state.layout.desktop_cell_fade_inset = config.surface.desktop_cell_fade_inset;
    state.layout.desktop_enable_solid_color = enable_solid;
    state.layout.desktop_solid_color = parse_hex_color_rgba(&config.surface.desktop_solid_color);

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

    for (key, val) in &config.env {
        let expanded = expand_env_vars(val);
        std::env::set_var(key, &expanded);
    }

    state.input_rules = config.device.clone();
    state.input_config = config.input.clone().unwrap_or_default();
    unsafe {
        state.apply_input_rules();
        state.apply_input_config();
    }

    state.keybinds.clear();
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
        
        let binding_key = (mods, keysym);
        if !seen.insert(binding_key) {
            eprintln!("[WARNING] Keybinding conflict: multiple actions mapped to mods={:?}, key={:?}", mods_str, key_str);
        }

        let action = parse_action(&kb.action);
        let command = if action == Action::Spawn || action == Action::Toggle {
            if let Some(ref cmd_str) = kb.command {
                let cmd_exe = cmd_str.split_whitespace().next().unwrap_or("");
                if !cmd_exe.is_empty() {
                    let mut found = false;
                    if let Ok(path_var) = std::env::var("PATH") {
                        for path_dir in std::env::split_paths(&path_var) {
                            if path_dir.join(cmd_exe).is_file() {
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        eprintln!("[WARNING] Configured keybinding command not found in PATH: {}", cmd_exe);
                    }
                }
            }
            kb.command.clone()
        } else {
            None
        };
        state.keybinds.push(Keybind {
            mods,
            keysym,
            action,
            command,
        });
    }

    let super_mod = parse_modifiers("super");
    let left_sym = parse_keysym("Left");
    let right_sym = parse_keysym("Right");
    if !state.keybinds.iter().any(|b| b.mods == super_mod && b.keysym == left_sym) {
        state.keybinds.push(Keybind {
            mods: super_mod,
            keysym: left_sym,
            action: Action::OverlayLeft,
            command: None,
        });
    }
    if !state.keybinds.iter().any(|b| b.mods == super_mod && b.keysym == right_sym) {
        state.keybinds.push(Keybind {
            mods: super_mod,
            keysym: right_sym,
            action: Action::OverlayRight,
            command: None,
        });
    }

    let super_ctrl_mod = parse_modifiers("super+ctrl");
    let up_sym = parse_keysym("Up");
    let down_sym = parse_keysym("Down");
    let equal_sym = parse_keysym("equal");
    let minus_sym = parse_keysym("minus");

    if !state.keybinds.iter().any(|b| b.mods == super_ctrl_mod && b.keysym == up_sym) {
        state.keybinds.push(Keybind {
            mods: super_ctrl_mod,
            keysym: up_sym,
            action: Action::PanUp,
            command: None,
        });
    }
    if !state.keybinds.iter().any(|b| b.mods == super_ctrl_mod && b.keysym == down_sym) {
        state.keybinds.push(Keybind {
            mods: super_ctrl_mod,
            keysym: down_sym,
            action: Action::PanDown,
            command: None,
        });
    }
    if !state.keybinds.iter().any(|b| b.mods == super_ctrl_mod && b.keysym == left_sym) {
        state.keybinds.push(Keybind {
            mods: super_ctrl_mod,
            keysym: left_sym,
            action: Action::PanLeft,
            command: None,
        });
    }
    if !state.keybinds.iter().any(|b| b.mods == super_ctrl_mod && b.keysym == right_sym) {
        state.keybinds.push(Keybind {
            mods: super_ctrl_mod,
            keysym: right_sym,
            action: Action::PanRight,
            command: None,
        });
    }
    let super_ctrl_shift_mod = parse_modifiers("super+ctrl+shift");
    if !state.keybinds.iter().any(|b| b.mods == super_ctrl_shift_mod && b.keysym == equal_sym) {
        state.keybinds.push(Keybind {
            mods: super_ctrl_shift_mod,
            keysym: equal_sym,
            action: Action::ZoomIn,
            command: None,
        });
    }
    if !state.keybinds.iter().any(|b| b.mods == super_ctrl_mod && b.keysym == minus_sym) {
        state.keybinds.push(Keybind {
            mods: super_ctrl_mod,
            keysym: minus_sym,
            action: Action::ZoomOut,
            command: None,
        });
    }
    if !state.keybinds.iter().any(|b| b.mods == super_ctrl_mod && b.keysym == equal_sym) {
        state.keybinds.push(Keybind {
            mods: super_ctrl_mod,
            keysym: equal_sym,
            action: Action::ZoomReset,
            command: None,
        });
    }

    let super_shift_mod = parse_modifiers("super+shift");
    let r_sym = parse_keysym("r");
    if !state.keybinds.iter().any(|b| b.mods == super_shift_mod && b.keysym == r_sym) {
        state.keybinds.push(Keybind {
            mods: super_shift_mod,
            keysym: r_sym,
            action: Action::Reload,
            command: None,
        });
    }

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
            assert_eq!(server.wm.layout.desktop_gap_color, "#a5cfc2");
            assert_eq!(server.wm.layout.desktop_gap_width, 24);
            assert_eq!(server.wm.layout.desktop_cell_corner_radius, 8);
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
                scale_eDP-1 (f64)2.0
                scale_DP-1 (f64)1.5
            }
        "#;
        let config = parse_kdl_config(content).unwrap();
        assert_eq!(config.display.get("scale_eDP-1"), Some(&2.0));
        assert_eq!(config.display.get("scale_DP-1"), Some(&1.5));
    }
}
