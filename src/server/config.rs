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
    pub overlay_behavior: String,
    pub overlay_width: i32,
    pub overlay_position: String,
    pub overlay_border_gap: i32,
    pub status_normal_color: String,
    pub desktop_background: String,
    pub transparency_opacity: f32,
    pub window_opacity: bool,
    pub desktop_grid_color: [f32; 4],
    pub desktop_grid_scale: f64,
    pub desktop_line_width: i32,
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
            overlay_behavior: "inline".to_string(),
            overlay_width: 360,
            overlay_position: "left".to_string(),
            overlay_border_gap: 0,
            status_normal_color: "#ccccd8".to_string(),
            desktop_background: "#000000".to_string(),
            transparency_opacity: 0.9,
            window_opacity: true,
            desktop_grid_color: [1.0, 1.0, 1.0, 0.05],
            desktop_grid_scale: 100.0,
            desktop_line_width: 1,
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
}

fn default_scale() -> f64 {
    1.0
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

#[derive(Debug, Deserialize, Clone, Default)]
pub struct InputConfig {
    pub tap_to_click: Option<bool>,
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<String>,
    pub natural_scroll: Option<bool>,
    pub dwt: Option<bool>,
    pub dwtp: Option<bool>,
    pub trackpoint_accel_speed: Option<f64>,
    pub trackpoint_accel_profile: Option<String>,
    pub gestures: Option<GesturesConfig>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TransparencyConfig {
    pub opacity: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SurfacesConfig {
    #[serde(default = "default_desktop_background")]
    pub desktop_background: String,
    #[serde(default = "default_desktop_grid_color")]
    pub desktop_grid_color: String,
    #[serde(default = "default_desktop_grid_scale")]
    pub desktop_grid_scale: i64,
    #[serde(default = "default_desktop_line_width")]
    pub desktop_line_width: i64,
}

impl Default for SurfacesConfig {
    fn default() -> Self {
        Self {
            desktop_background: default_desktop_background(),
            desktop_grid_color: default_desktop_grid_color(),
            desktop_grid_scale: default_desktop_grid_scale(),
            desktop_line_width: default_desktop_line_width(),
        }
    }
}

fn default_desktop_background() -> String {
    "#000000".to_string()
}

fn default_desktop_grid_color() -> String {
    "#ffffff0d".to_string()
}

fn default_desktop_grid_scale() -> i64 {
    100
}

fn default_desktop_line_width() -> i64 {
    1
}


#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub layout: LayoutConfig,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub keybind: Vec<KeybindConfig>,
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
    pub surfaces: SurfacesConfig,
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
    #[serde(default = "default_border_width")]
    pub border_width: i64,
    #[serde(default = "default_fullscreen_border_width")]
    pub fullscreen_border_width: i64,
    #[serde(default = "default_cascade_border_width")]
    pub cascade_border_width: i64,
    #[serde(default = "default_grid_border_width")]
    pub grid_border_width: i64,
    #[serde(default = "default_floating_border_width")]
    pub floating_border_width: i64,
    #[serde(default = "default_border_color")]
    pub border_color: String,
    #[serde(default = "default_background_color")]
    pub background_color: String,
    #[serde(default = "default_border_font_size")]
    pub border_font_size: i64,
    #[serde(default = "default_transition_duration")]
    pub transition_duration: i64,
    #[serde(default = "default_grid_gap")]
    pub grid_gap: i64,
    #[serde(default = "default_border_blur")]
    pub border_blur: bool,
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
    #[serde(default = "default_window_opacity")]
    pub window_opacity: bool,
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
            border_width: default_border_width(),
            fullscreen_border_width: default_fullscreen_border_width(),
            cascade_border_width: default_cascade_border_width(),
            grid_border_width: default_grid_border_width(),
            floating_border_width: default_floating_border_width(),
            border_color: default_border_color(),
            background_color: default_background_color(),
            border_font_size: default_border_font_size(),
            transition_duration: default_transition_duration(),
            grid_gap: default_grid_gap(),
            border_blur: default_border_blur(),
            window_blur: default_window_blur(),
            overlay_behavior: default_overlay_behavior(),
            overlay_width: default_overlay_width(),
            overlay_position: default_overlay_position(),
            overlay_border_gap: default_overlay_border_gap(),
            status_normal_color: default_status_normal_color(),
            window_opacity: default_window_opacity(),
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
fn default_border_width() -> i64 { 0 }
fn default_fullscreen_border_width() -> i64 { 0 }
fn default_cascade_border_width() -> i64 { 0 }
fn default_grid_border_width() -> i64 { 0 }
fn default_floating_border_width() -> i64 { 0 }
fn default_border_color() -> String { "#3e3e3e".to_string() }
fn default_background_color() -> String { "#0a0a0a".to_string() }
fn default_border_font_size() -> i64 { 11 }
fn default_transition_duration() -> i64 { 300 }
fn default_grid_gap() -> i64 { 18 }
fn default_border_blur() -> bool { false }
fn default_window_blur() -> bool { false }
fn default_overlay_behavior() -> String { "inline".to_string() }
fn default_overlay_width() -> i64 { 360 }
fn default_overlay_position() -> String { "left".to_string() }
fn default_overlay_border_gap() -> i64 { 0 }
fn default_status_normal_color() -> String { "#ccccd8".to_string() }
fn default_window_opacity() -> bool { true }

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
            return [
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
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
    [1.0, 1.0, 1.0, 0.05] // fallback default (5% white)
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
    if s == "close" {
        Action::Close
    } else if s == "exit" {
        Action::Exit
    } else if s == "reload" {
        Action::Reload
    } else if s == "focus-next" {
        Action::FocusNext
    } else if s == "focus-prev" {
        Action::FocusPrev
    } else if s == "move" {
        Action::Move
    } else if s == "resize" {
        Action::Resize
    } else if s == "layout-next" {
        Action::LayoutNext
    } else if s == "mode-next" {
        Action::ModeNext
    } else if s == "mode-next-shared" {
        Action::ModeNextShared
    } else if s == "fullscreen" {
        Action::Fullscreen
    } else if s == "minimize" {
        Action::Minimize
    } else if s.starts_with("spawn")
        && (s.len() == 5 || s.as_bytes()[5] == b' ' || s.as_bytes()[5] == b'-')
    {
        Action::Spawn
    } else if s.starts_with("view") {
        let rest = &s[4..];
        let tag_str = rest
            .strip_prefix('-')
            .or_else(|| rest.strip_prefix(' '))
            .unwrap_or(rest);
        if let Ok(tag) = tag_str.parse::<i32>() {
            if tag >= 1 && tag <= 4 {
                return match tag {
                    1 => Action::View1,
                    2 => Action::View2,
                    3 => Action::View3,
                    4 => Action::View4,
                    _ => Action::None,
                };
            }
        }
        Action::None
    } else if s == "toggle" {
        Action::Toggle
    } else if s.starts_with("set-viewport") || s.starts_with("set-tag") {
        let is_viewport = s.starts_with("set-viewport");
        let rest = if is_viewport { &s[12..] } else { &s[7..] };
        let tag_str = rest
            .strip_prefix('-')
            .or_else(|| rest.strip_prefix(' '))
            .unwrap_or(rest);
        if let Ok(tag) = tag_str.parse::<i32>() {
            if tag >= 1 && tag <= 4 {
                return match tag {
                    1 => Action::SetViewport1,
                    2 => Action::SetViewport2,
                    3 => Action::SetViewport3,
                    4 => Action::SetViewport4,
                    _ => Action::None,
                };
            }
        }
        Action::None
    } else if s == "expose" {
        Action::Expose
    } else if s == "side-panel-left" || s == "pinned-left" || s == "overlay-left" {
        Action::OverlayLeft
    } else if s == "side-panel-right" || s == "pinned-right" || s == "overlay-right" {
        Action::OverlayRight
    } else if s == "zoom-in" {
        Action::ZoomIn
    } else if s == "zoom-out" {
        Action::ZoomOut
    } else if s == "zoom-reset" {
        Action::ZoomReset
    } else if s == "pan-left" {
        Action::PanLeft
    } else if s == "pan-right" {
        Action::PanRight
    } else if s == "pan-up" {
        Action::PanUp
    } else if s == "pan-down" {
        Action::PanDown
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
    let path = if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
        format!("{}/cce/config.json", xdg_config_home)
    } else if let Ok(home) = std::env::var("HOME") {
        format!("{}/.config/cce/config.json", home)
    } else {
        return None;
    };
    if std::path::Path::new(&path).exists() {
        Some(path)
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

pub fn parse_config(path: &str, state: &mut crate::window_manager::WindowManager) -> Result<(), String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return Err(format!("cannot open {}: {}", path, e)),
    };

    let config: Config = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(e) => return Err(format!("JSON parse error: {}", e)),
    };

    state.output_scale = config.output.as_ref().map(|o| o.scale as f32).unwrap_or(1.0f32);
    state.display = config.display.clone();

    state.layout.gap = config.layout.gap as i32;
    state.layout.gap_top = config.layout.gap_top as i32;
    state.layout.gap_left = config.layout.gap_left as i32;
    state.layout.gap_right = config.layout.gap_right as i32;
    state.layout.gap_bottom = config.layout.gap_bottom as i32;
    state.layout.cascade_offset = config.layout.cascade_offset as i32;
    state.layout.bar_height = config.layout.bar_height as i32;
    state.layout.border_width = config.layout.border_width as i32;
    state.layout.fullscreen_border_width = config.layout.fullscreen_border_width as i32;
    state.layout.cascade_border_width = config.layout.cascade_border_width as i32;
    state.layout.grid_border_width = config.layout.grid_border_width as i32;
    state.layout.floating_border_width = config.layout.floating_border_width as i32;
    
    let border_color_val = parse_hex_color(&config.layout.border_color);
    state.layout.border_r = ((border_color_val >> 16) & 0xFF) * 0x01010101;
    state.layout.border_g = ((border_color_val >> 8) & 0xFF) * 0x01010101;
    state.layout.border_b = (border_color_val & 0xFF) * 0x01010101;
    state.layout.border_a = 0xFFFFFFFF;

    let desktop_background_str = config.surfaces.desktop_background.clone();
    state.layout.desktop_background = desktop_background_str.clone();

    let background_color_val = parse_hex_color(&desktop_background_str);
    state.layout.background_r = ((background_color_val >> 16) & 0xFF) * 0x01010101;
    state.layout.background_g = ((background_color_val >> 8) & 0xFF) * 0x01010101;
    state.layout.background_b = (background_color_val & 0xFF) * 0x01010101;
    state.layout.background_a = 0xFFFFFFFF;

    state.layout.desktop_grid_color = parse_hex_color_rgba(&config.surfaces.desktop_grid_color);
    state.layout.desktop_grid_scale = config.surfaces.desktop_grid_scale as f64;
    state.layout.desktop_line_width = config.surfaces.desktop_line_width as i32;

    state.layout.border_font_size = config.layout.border_font_size as i32;
    state.layout.transition_duration = config.layout.transition_duration as i32;
    state.layout.grid_gap = config.layout.grid_gap as i32;
    state.layout.border_blur = config.layout.border_blur;
    state.layout.window_blur = config.layout.window_blur;
    state.layout.overlay_behavior = config.layout.overlay_behavior;
    state.layout.overlay_width = config.layout.overlay_width as i32;
    state.layout.overlay_position = config.layout.overlay_position;
    state.layout.overlay_border_gap = config.layout.overlay_border_gap as i32;
    state.layout.status_normal_color = config.layout.status_normal_color.clone();
    state.layout.transparency_opacity = config.transparency.as_ref().and_then(|t| t.opacity).unwrap_or(0.9) as f32;
    state.layout.window_opacity = config.layout.window_opacity;

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
    for kb in &config.keybind {
        let mods = parse_modifiers(&kb.mods);
        let keysym = parse_keysym(&kb.key);
        let action = parse_action(&kb.action);
        let command = if action == Action::Spawn || action == Action::Toggle {
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
            println!("TEST_WM_STARTUP: {:?}", server.wm.startup);
            println!("TEST_WM_PATH: {:?}", std::env::var("PATH"));
            assert!(!server.wm.startup.is_empty(), "Startup list should not be empty!");
            assert_eq!(server.wm.input_config.tap_to_click, Some(true));
            assert_eq!(server.wm.input_config.accel_speed, Some(0.2));
            assert_eq!(server.wm.input_config.accel_profile, Some("adaptive".to_string()));
            assert_eq!(server.wm.input_config.natural_scroll, Some(true));
            assert_eq!(server.wm.input_config.dwt, Some(true));
            assert_eq!(server.wm.input_config.dwtp, Some(true));
            assert_eq!(server.wm.input_config.trackpoint_accel_speed, Some(0.6));
            assert_eq!(server.wm.input_config.trackpoint_accel_profile, Some("flat".to_string()));
            let parsed_color = parse_hex_color(&server.wm.layout.desktop_background);
            let r = (parsed_color >> 16) & 0xFF;
            let g = (parsed_color >> 8) & 0xFF;
            let b = parsed_color & 0xFF;
            assert_eq!(server.wm.layout.background_r, r * 0x01010101);
            assert_eq!(server.wm.layout.background_g, g * 0x01010101);
            assert_eq!(server.wm.layout.background_b, b * 0x01010101);
        }
    }

    #[test]
    fn test_display_scale_parsing() {
        let content = r#"{
            "display": {
                "scale_eDP-1": 2.0,
                "scale_DP-1": 1.5
            }
        }"#;
        let config: Config = serde_json::from_str(content).unwrap();
        assert_eq!(config.display.get("scale_eDP-1"), Some(&2.0));
        assert_eq!(config.display.get("scale_DP-1"), Some(&1.5));
    }
}
