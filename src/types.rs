// Core data structures for cce-client

use std::collections::HashMap;

pub const NUM_TAGS: usize = 4;

/// Tiling mode for a window
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilingMode {
    Floating,
    Cascade,
    Grid,
    Fullscreen,
    Popup,
    SidePanel,
}

impl TilingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            TilingMode::Floating => "Floating",
            TilingMode::Cascade => "Cascade",
            TilingMode::Grid => "Grid",
            TilingMode::Fullscreen => "Fullscreen",
            TilingMode::Popup => "Popup",
            TilingMode::SidePanel => "Side Panel",
        }
    }
}

/// Actions that can be triggered by keybindings or IPC
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Fullscreen,
    LayoutNext,
    ModeNext,
    ModeNextShared,
    Reload,
    Restart,
    View1,
    View2,
    View3,
    View4,
    Toggle1,
    Toggle2,
    Toggle3,
    Toggle4,
    SetTag1,
    SetTag2,
    SetTag3,
    SetTag4,
    Expose,
    Minimize,
}

/// Layout parameters
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
    pub side_panel_behavior: String,
    pub side_panel_width: i32,
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
            border_width: 6,
            fullscreen_border_width: 0,
            cascade_border_width: 6,
            grid_border_width: 6,
            floating_border_width: 6,
            border_r: 0x3E3E3E3Eu32,
            border_g: 0x3E3E3E3Eu32,
            border_b: 0x3E3E3E3Eu32,
            border_a: 0xFFFFFFFFu32,
            background_r: 0x0A0A0A0Au32,
            background_g: 0x1A1A1A1Au32,
            background_b: 0x0E0E0E0Eu32,
            background_a: 0xFFFFFFFFu32,
            border_font_size: 11,
            transition_duration: 300,
            grid_gap: 18,
            border_blur: false,
            window_blur: false,
            side_panel_behavior: "inline".to_string(),
            side_panel_width: 360,
        }
    }
}

/// A rule that matches windows by app_id/title and assigns a tiling mode
#[derive(Debug, Clone)]
pub struct ModeRule {
    pub mode: TilingMode,
    pub app_id_pattern: String,
    pub title_pattern: Option<String>,
    pub single_instance: bool,
    pub tag: i32,
    pub circular: bool,
}

/// A pending keyboard binding waiting to be applied to seats
#[derive(Debug, Clone)]
pub struct PendingXkbBinding {
    pub mods: u32,
    pub keysym: u32,
    pub action: Action,
    pub command: Option<String>,
}

/// A pending pointer binding waiting to be applied to seats
#[derive(Debug, Clone)]
pub struct PendingPointerBinding {
    pub mods: u32,
    pub button: u32,
    pub action: Action,
}

/// User data attached to XKB/pointer binding proxies, carrying the action
/// to execute when the binding is triggered.
#[derive(Debug, Clone)]
pub struct BindingUserData {
    pub seat_id: u64,
    pub action: Action,
    pub command: Option<String>,
}

/// An input device (keyboard or other)
#[derive(Debug, Clone)]
pub struct InputDevice {
    pub is_keyboard: bool,
}

/// A managed output (monitor)
#[derive(Debug, Clone)]
pub struct Output {
    pub id: u64,
    pub removed: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub usable_x: i32,
    pub usable_y: i32,
    pub usable_width: i32,
    pub usable_height: i32,
    pub wl_output_name: Option<u32>,
}

impl Default for Output {
    fn default() -> Self {
        Output {
            id: 0,
            removed: false,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            usable_x: 0,
            usable_y: 0,
            usable_width: 0,
            usable_height: 0,
            wl_output_name: None,
        }
    }
}

/// A managed window
#[derive(Debug, Clone)]
pub struct Window {
    pub id: u64,
    pub is_new: bool,
    pub closed: bool,
    pub tags: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub identifier: Option<String>,
    pub parent_id: Option<u64>,
    pub has_parent: bool,
    pub pid: u32,
    pub hint_min_width: i32,
    pub hint_min_height: i32,
    pub hint_max_width: i32,
    pub hint_max_height: i32,
    pub decoration_hint: u32,
    pub presentation_hint: u32,
    pub fullscreen_requested: bool,
    pub maximize_requested: bool,
    pub minimize_requested: bool,
    pub minimized: bool,
    pub tiling_mode: TilingMode,
    pub mode_locked: bool,
    /// Whether we've queued an xprop check for XWayland parent detection.
    /// River doesn't forward WM_TRANSIENT_FOR for XWayland windows, so we
    /// check via xprop as a fallback.
    pub needs_xprop_check: bool,
    /// How many ManageStart cycles we've waited for the xprop result file.
    pub xprop_check_attempts: u8,
    pub anim_x: Option<f64>,
    pub anim_y: Option<f64>,
    pub anim_w: Option<f64>,
    pub anim_h: Option<f64>,
    pub anim_opacity: Option<f64>,
    pub circular: bool,
    pub size_hint_applied: bool,
}

impl Default for Window {
    fn default() -> Self {
        Window {
            id: 0,
            is_new: true,
            closed: false,
            tags: 1,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            app_id: None,
            title: None,
            identifier: None,
            parent_id: None,
            has_parent: false,
            pid: 0,
            hint_min_width: 0,
            hint_min_height: 0,
            hint_max_width: 0,
            hint_max_height: 0,
            decoration_hint: 3,   // no_preference
            presentation_hint: 0, // vsync
            fullscreen_requested: false,
            maximize_requested: false,
            minimize_requested: false,
            minimized: false,
            tiling_mode: TilingMode::Floating,
            mode_locked: false,
            needs_xprop_check: false,
            xprop_check_attempts: 0,
            anim_x: None,
            anim_y: None,
            anim_w: None,
            anim_h: None,
            anim_opacity: None,
            circular: false,
            size_hint_applied: false,
        }
    }
}

/// A seat (input device group)
#[derive(Debug, Clone)]
pub struct Seat {
    pub id: u64,
    pub is_new: bool,
    pub removed: bool,
    pub focused_window_id: Option<u64>,
    pub hovered_window_id: Option<u64>,
    pub interacted_window_id: Option<u64>,
    pub pending_action: Action,
    pub pending_command: Option<String>,
}

impl Default for Seat {
    fn default() -> Self {
        Seat {
            id: 0,
            is_new: true,
            removed: false,
            focused_window_id: None,
            hovered_window_id: None,
            interacted_window_id: None,
            pending_action: Action::None,
            pending_command: None,
        }
    }
}

/// The main window manager state
#[derive(Debug, Clone)]
pub struct WindowManager {
    pub outputs: Vec<Output>,
    pub windows: Vec<Window>,
    pub seats: Vec<Seat>,
    pub pending_bindings: Vec<PendingXkbBinding>,
    pub pending_pointer_bindings: Vec<PendingPointerBinding>,
    pub mode_rules: Vec<ModeRule>,
    pub layout: Layout,
    pub active_tags: u32,
    pub focused_tags: u32,
    pub config_done: bool,
    pub in_manage_sequence: bool,
    pub needs_render: bool,
    pub needs_focus: bool,
    pub needs_status_update: bool,
    pub exit_requested: bool,
    pub global_layout: TilingMode,
    pub tag_layouts: [TilingMode; NUM_TAGS],
    pub has_tag_layout: [bool; NUM_TAGS],
    pub input_devices: Vec<InputDevice>,
    pub env_vars: HashMap<String, String>,
    pub pending_startup_apps: Vec<String>,
    pub startup_spawned: bool,
    pub output_scale: f64,
    /// When true, apply configured output_scale via wlr-output-management
    /// on the next output_manager done event. Set by config load and
    /// by VT-switch-back (where wlroots resets scale to 1).
    pub pending_scale_apply: bool,
    /// When true, apply persisted state from ~/.cache/cce_client_state on the
    /// next ManageStart cycle (after windows have been re-advertised).
    /// Set to true on startup/restart, consumed after application.
    pub needs_state_restore: bool,
    /// Counter for how many ManageStart cycles we've waited for window metadata
    /// before applying persisted state. Reset to 0 after state is applied.
    pub state_restore_attempts: u8,
    /// Whether tap-to-click is enabled on touchpad devices
    pub tap_to_click: bool,
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<String>,
    pub natural_scroll: Option<bool>,
    pub dwt: Option<bool>,
    pub dwtp: Option<bool>,
    pub trackpoint_accel_speed: Option<f64>,
    pub trackpoint_accel_profile: Option<String>,
    /// Whether tap-to-click config has been applied to libinput devices yet
    pub tap_config_applied: bool,
    pub cursor_theme: Option<String>,
    pub cursor_size: Option<u32>,
    pub cursor_theme_applied: bool,
    /// Whether system notifications are enabled
    pub notifications_enable: bool,
    /// Reload commands to execute on configuration reload
    pub reload_commands: Vec<String>,
    pub input_controller: Option<tokio::sync::mpsc::UnboundedSender<crate::input::InputDaemonMsg>>,
    pub trackpad_disabled: bool,
    pub expose_active: bool,
    pub expose_visual_active: bool,
    pub animating: bool,
}

impl Default for WindowManager {
    fn default() -> Self {
        WindowManager {
            outputs: Vec::new(),
            windows: Vec::new(),
            seats: Vec::new(),
            pending_bindings: Vec::new(),
            pending_pointer_bindings: Vec::new(),
            mode_rules: Vec::new(),
            layout: Layout::default(),
            active_tags: 1,
            focused_tags: 0,
            config_done: false,
            in_manage_sequence: false,
            needs_render: true,        // render on first frame
            needs_focus: true,         // focus on first frame
            needs_status_update: true, // update status files on first cycle
            exit_requested: false,
            global_layout: TilingMode::Cascade,
            tag_layouts: [TilingMode::Cascade; NUM_TAGS],
            has_tag_layout: [true; NUM_TAGS],
            input_devices: Vec::new(),
            env_vars: HashMap::new(),
            pending_startup_apps: Vec::new(),
            startup_spawned: false,
            output_scale: 0.0,
            pending_scale_apply: false,
            needs_state_restore: true,
            state_restore_attempts: 0,
            tap_to_click: false,
            accel_speed: None,
            accel_profile: None,
            natural_scroll: None,
            dwt: None,
            dwtp: None,
            trackpoint_accel_speed: None,
            trackpoint_accel_profile: None,
            tap_config_applied: false,
            cursor_theme: None,
            cursor_size: None,
            cursor_theme_applied: false,
            notifications_enable: true,
            reload_commands: Vec::new(),
            input_controller: None,
            trackpad_disabled: false,
            expose_active: false,
            expose_visual_active: false,
            animating: false,
        }
    }
}

impl WindowManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Find a window by its ID
    pub fn get_window(&self, id: u64) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Find a window by its ID (mutable)
    pub fn get_window_mut(&mut self, id: u64) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// Find the first seat that has a focused window
    pub fn first_seat_with_focus(&self) -> Option<&Seat> {
        self.seats.iter().find(|s| s.focused_window_id.is_some())
    }

    /// Get the focused window for the first seat that has one
    pub fn focused_window(&self) -> Option<&Window> {
        let seat = self.first_seat_with_focus()?;
        let wid = seat.focused_window_id?;
        self.get_window(wid)
    }

    /// Get the focused window (mutable)
    pub fn focused_window_mut(&mut self) -> Option<&mut Window> {
        let (wid, _) = {
            let seat = self.first_seat_with_focus()?;
            (seat.focused_window_id?, seat.id)
        };
        self.get_window_mut(wid)
    }

    /// Move a window to the end of the windows vector.
    /// This makes it the last cascade window (front of visual stack,
    /// rightmost/bottommost position, brightest border).
    /// Returns true if the window was moved, false if not found or already last.
    pub fn move_window_to_end(&mut self, id: u64) -> bool {
        let idx = match self.windows.iter().position(|w| w.id == id) {
            Some(i) => i,
            None => return false,
        };
        // Already last?
        if idx == self.windows.len() - 1 {
            return false;
        }
        let win = self.windows.remove(idx);
        self.windows.push(win);
        true
    }
}

/// Parse a hex color string like "#RRGGBB" or "#RRGGBBAA" into
/// byte-replicated 32-bit channel values for River's color format.
/// River divides each u32 by maxInt(u32) to get a float, so each byte
/// must be replicated: 0xVV -> 0xVVVVVVVV.
pub fn parse_hex_color(s: &str) -> Option<(u32, u32, u32, u32)> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 && s.len() != 8 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()? as u32;
    let g = u8::from_str_radix(&s[2..4], 16).ok()? as u32;
    let b = u8::from_str_radix(&s[4..6], 16).ok()? as u32;
    let a = if s.len() == 8 {
        u8::from_str_radix(&s[6..8], 16).ok()? as u32
    } else {
        0xFFu32
    };
    // Byte-replicate: 0xVV -> 0xVVVVVVVV
    Some((r * 0x01010101, g * 0x01010101, b * 0x01010101, a * 0x01010101))
}

/// Parse a tiling mode string
pub fn parse_tiling_mode(s: &str) -> TilingMode {
    match s {
        "cascade" => TilingMode::Cascade,
        "grid" => TilingMode::Grid,
        "fullscreen" => TilingMode::Fullscreen,
        "floating" => TilingMode::Floating,
        "popup" => TilingMode::Popup,
        "side-panel" | "side_panel" | "side panel" | "Side Panel" => TilingMode::SidePanel,
        _ => TilingMode::Floating,
    }
}

/// Parse an action string
pub fn parse_action(s: &str) -> Action {
    if s == "close" {
        Action::Close
    } else if s == "exit" {
        Action::Exit
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
    } else if s == "reload" {
        Action::Reload
    } else if s == "restart" {
        Action::Restart
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
            if tag >= 1 && tag <= NUM_TAGS as i32 {
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
    } else if s.starts_with("toggle") {
        let rest = &s[6..];
        let tag_str = rest
            .strip_prefix('-')
            .or_else(|| rest.strip_prefix(' '))
            .unwrap_or(rest);
        if let Ok(tag) = tag_str.parse::<i32>() {
            if tag >= 1 && tag <= NUM_TAGS as i32 {
                return match tag {
                    1 => Action::Toggle1,
                    2 => Action::Toggle2,
                    3 => Action::Toggle3,
                    4 => Action::Toggle4,
                    _ => Action::None,
                };
            }
        }
        if s == "toggle" {
            Action::Toggle
        } else {
            Action::None
        }
    } else if s.starts_with("set-tag") {
        let rest = &s[7..];
        let tag_str = rest
            .strip_prefix('-')
            .or_else(|| rest.strip_prefix(' '))
            .unwrap_or(rest);
        if let Ok(tag) = tag_str.parse::<i32>() {
            if tag >= 1 && tag <= NUM_TAGS as i32 {
                return match tag {
                    1 => Action::SetTag1,
                    2 => Action::SetTag2,
                    3 => Action::SetTag3,
                    4 => Action::SetTag4,
                    _ => Action::None,
                };
            }
        }
        Action::None
    } else if s == "expose" {
        Action::Expose
    } else {
        Action::None
    }
}

/// Parse modifier string like "alt+shift" into a bitmask
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

/// Parse a button string ("left", "right", "middle", or numeric)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color_rgb() {
        let (r, g, b, a) = parse_hex_color("#3e3e3e").unwrap();
        assert_eq!(r, 0x3E3E3E3E);
        assert_eq!(g, 0x3E3E3E3E);
        assert_eq!(b, 0x3E3E3E3E);
        assert_eq!(a, 0xFFFFFFFF);
    }

    #[test]
    fn test_parse_hex_color_rgba() {
        let (r, g, b, a) = parse_hex_color("#5c9060ff").unwrap();
        assert_eq!(r, 0x5C5C5C5C);
        assert_eq!(g, 0x90909090);
        assert_eq!(b, 0x60606060);
        assert_eq!(a, 0xFFFFFFFF);
    }

    #[test]
    fn test_parse_hex_color_invalid() {
        assert!(parse_hex_color("3e3e3e").is_none());
        assert!(parse_hex_color("#3e3e").is_none());
        assert!(parse_hex_color("#3e3e3e3e3e").is_none());
    }

    #[test]
    fn test_parse_tiling_mode() {
        assert_eq!(parse_tiling_mode("cascade"), TilingMode::Cascade);
        assert_eq!(parse_tiling_mode("grid"), TilingMode::Grid);
        assert_eq!(parse_tiling_mode("fullscreen"), TilingMode::Fullscreen);
        assert_eq!(parse_tiling_mode("floating"), TilingMode::Floating);
        assert_eq!(parse_tiling_mode("popup"), TilingMode::Popup);
        assert_eq!(parse_tiling_mode("side-panel"), TilingMode::SidePanel);
        assert_eq!(parse_tiling_mode("Side Panel"), TilingMode::SidePanel);
        assert_eq!(parse_tiling_mode("unknown"), TilingMode::Floating);
    }

    #[test]
    fn test_parse_action() {
        assert_eq!(parse_action("close"), Action::Close);
        assert_eq!(parse_action("exit"), Action::Exit);
        assert_eq!(parse_action("focus-next"), Action::FocusNext);
        assert_eq!(parse_action("focus-prev"), Action::FocusPrev);
        assert_eq!(parse_action("move"), Action::Move);
        assert_eq!(parse_action("resize"), Action::Resize);
        assert_eq!(parse_action("layout-next"), Action::LayoutNext);
        assert_eq!(parse_action("mode-next"), Action::ModeNext);
        assert_eq!(parse_action("mode-next-shared"), Action::ModeNextShared);
        assert_eq!(parse_action("reload"), Action::Reload);
        assert_eq!(parse_action("restart"), Action::Restart);
        assert_eq!(parse_action("spawn"), Action::Spawn);
        assert_eq!(parse_action("spawn something"), Action::Spawn);
        assert_eq!(parse_action("toggle"), Action::Toggle);
        assert_eq!(parse_action("view-1"), Action::View1);
        assert_eq!(parse_action("view-4"), Action::View4);
        assert_eq!(parse_action("toggle-2"), Action::Toggle2);
        assert_eq!(parse_action("set-tag-3"), Action::SetTag3);
        assert_eq!(parse_action("expose"), Action::Expose);
        assert_eq!(parse_action("minimize"), Action::Minimize);
        assert_eq!(parse_action("unknown"), Action::None);
    }

    #[test]
    fn test_parse_modifiers() {
        assert_eq!(parse_modifiers("alt"), 0x08);
        assert_eq!(parse_modifiers("alt+shift"), 0x08 | 0x01);
        assert_eq!(parse_modifiers("ctrl"), 0x04);
        assert_eq!(parse_modifiers("super"), 0x40);
        assert_eq!(parse_modifiers(""), 0x00);
    }

    #[test]
    fn test_parse_button() {
        assert_eq!(parse_button("left"), 0x110);
        assert_eq!(parse_button("right"), 0x111);
        assert_eq!(parse_button("middle"), 0x112);
        assert_eq!(parse_button("272"), 272);
    }

    #[test]
    fn test_wm_default() {
        let wm = WindowManager::default();
        assert_eq!(wm.active_tags, 1);
        assert_eq!(wm.global_layout, TilingMode::Cascade);
        assert!(!wm.config_done);
        assert_eq!(wm.layout.gap, 48);
        assert_eq!(wm.layout.gap_top, 48);
        assert_eq!(wm.layout.gap_left, 48);
        assert_eq!(wm.layout.gap_right, 48);
        assert_eq!(wm.layout.gap_bottom, 48);
    }
}
