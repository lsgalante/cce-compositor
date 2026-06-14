// TOML config parsing for cce-client

use serde::Deserialize;
use std::collections::HashMap;
use std::fs;

use crate::types::{
    parse_action, parse_button, parse_hex_color, parse_modifiers, parse_tiling_mode, ModeRule,
    PendingPointerBinding, PendingXkbBinding, WindowManager, NUM_TAGS,
};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub layout: LayoutConfig,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub input: InputConfig,
    #[serde(default)]
    pub repeat: RepeatConfig,
    #[serde(default)]
    pub startup: Vec<StartupEntryConfig>,
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
    pub notifications: NotificationsConfig,
    #[serde(default)]
    pub reload: Vec<ReloadEntryConfig>,
    #[serde(default)]
    pub inertial: Option<InertialConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ReloadEntryConfig {
    pub exec: String,
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
    #[serde(default = "default_border_color", alias = "high_color")]
    pub border_color: String,
    #[serde(default = "default_background_color", alias = "low_color")]
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
    #[serde(default = "default_side_panel_behavior")]
    pub side_panel_behavior: String,
    #[serde(default = "default_side_panel_width")]
    pub side_panel_width: i64,
    #[serde(default = "default_side_panel_position")]
    pub side_panel_position: String,
    #[serde(default = "default_side_panel_border_gap")]
    pub side_panel_border_gap: i64,
    #[serde(default = "default_side_panel_border_opacity")]
    pub side_panel_border_opacity: i64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        LayoutConfig {
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
            side_panel_behavior: default_side_panel_behavior(),
            side_panel_width: default_side_panel_width(),
            side_panel_position: default_side_panel_position(),
            side_panel_border_gap: default_side_panel_border_gap(),
            side_panel_border_opacity: default_side_panel_border_opacity(),
        }
    }
}

fn default_grid_gap() -> i64 {
    18
}

fn default_border_blur() -> bool {
    false
}

fn default_window_blur() -> bool {
    false
}

fn default_side_panel_behavior() -> String {
    "inline".to_string()
}

fn default_side_panel_width() -> i64 {
    360
}

fn default_side_panel_position() -> String {
    "left".to_string()
}

fn default_side_panel_border_gap() -> i64 {
    0
}

fn default_side_panel_border_opacity() -> i64 {
    100
}

fn default_gap() -> i64 {
    48
}
fn default_gap_top() -> i64 {
    48
}
fn default_gap_left() -> i64 {
    48
}
fn default_gap_right() -> i64 {
    48
}
fn default_gap_bottom() -> i64 {
    48
}
fn default_cascade_offset() -> i64 {
    20
}
fn default_bar_height() -> i64 {
    24
}
fn default_border_width() -> i64 {
    6
}
fn default_fullscreen_border_width() -> i64 {
    0
}
fn default_cascade_border_width() -> i64 {
    6
}
fn default_grid_border_width() -> i64 {
    6
}
fn default_floating_border_width() -> i64 {
    6
}
fn default_border_color() -> String {
    "#3e3e3e".to_string()
}
fn default_background_color() -> String {
    "#0a1a0e".to_string()
}
fn default_border_font_size() -> i64 {
    11
}
fn default_transition_duration() -> i64 {
    300
}

#[derive(Debug, Deserialize, Default)]
pub struct OutputConfig {
    #[serde(default)]
    pub scale: f64,
}

#[derive(Debug, Deserialize, Default)]
pub struct RepeatConfig {
    #[serde(default)]
    pub rate: i64,
    #[serde(default)]
    pub delay: i64,
}

#[derive(Debug, Deserialize, Default)]
pub struct InputConfig {
    #[serde(default)]
    pub tap_to_click: bool,
    pub accel_speed: Option<f64>,
    pub accel_profile: Option<String>,
    pub natural_scroll: Option<bool>,
    pub dwt: Option<bool>,
    pub dwtp: Option<bool>,
    pub trackpoint_accel_speed: Option<f64>,
    pub trackpoint_accel_profile: Option<String>,
    pub cursor_theme: Option<String>,
    pub cursor_size: Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct InertialConfig {
    #[serde(default = "default_true")]
    pub inertial_scroll: bool,
    #[serde(default = "default_scroll_friction")]
    pub scroll_friction: u16,
    #[serde(default = "default_false")]
    pub inertial_pointer: bool,
    #[serde(default = "default_pointer_friction")]
    pub pointer_friction: u16,
    #[serde(default = "default_false")]
    pub inertial_trackpad: bool,
    #[serde(default = "default_trackpad_friction")]
    pub trackpad_friction: u16,
    #[serde(default = "default_speed")]
    pub pointer_speed: f64,
    #[serde(default = "default_speed")]
    pub scroll_speed: f64,
    #[serde(default = "default_speed")]
    pub trackpad_speed: f64,
}

fn default_scroll_friction() -> u16 { 90 }
fn default_pointer_friction() -> u16 { 95 }
fn default_trackpad_friction() -> u16 { 95 }
fn default_speed() -> f64 { 1.0 }
fn default_false() -> bool { false }

#[derive(Debug, Deserialize)]
pub struct NotificationsConfig {
    #[serde(default = "default_true")]
    pub enable: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self { enable: true }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct StartupEntryConfig {
    pub exec: String,
    #[serde(default)]
    pub once: bool,
    pub restart: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct KeybindConfig {
    pub mods: String,
    pub key: String,
    pub action: String,
    pub command: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PointerBindConfig {
    pub mods: String,
    pub button: String,
    pub action: String,
}

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

/// Parse the TOML config file and apply it to the WindowManager state.
/// `cold_start` controls whether `once = true` startup entries are spawned.
pub fn parse_config(path: &str, cold_start: bool, state: &mut WindowManager) -> Result<(), String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            let err_msg = format!("cannot open {}: {}", path, e);
            eprintln!("parse_config: {}", err_msg);
            return Err(err_msg);
        }
    };

    let config: Config = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            let err_msg = format!("TOML parse error: {}", e);
            eprintln!("parse_config: {}", err_msg);
            return Err(err_msg);
        }
    };

    // [layout] section
    eprintln!("[config] applying layout section...");
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
    state.layout.border_font_size = config.layout.border_font_size as i32;
    state.layout.transition_duration = config.layout.transition_duration as i32;
    state.layout.grid_gap = config.layout.grid_gap as i32;
    state.layout.border_blur = config.layout.border_blur;
    state.layout.window_blur = config.layout.window_blur;
    state.layout.side_panel_behavior = config.layout.side_panel_behavior.clone();
    state.layout.side_panel_width = config.layout.side_panel_width as i32;
    state.layout.side_panel_position = config.layout.side_panel_position.clone();
    state.layout.side_panel_border_gap = config.layout.side_panel_border_gap as i32;
    state.layout.side_panel_border_opacity = config.layout.side_panel_border_opacity as i32;
    if let Some((r, g, b, a)) = parse_hex_color(&config.layout.border_color) {
        state.layout.border_r = r;
        state.layout.border_g = g;
        state.layout.border_b = b;
        state.layout.border_a = a;
    }
    if let Some((r, g, b, a)) = parse_hex_color(&config.layout.background_color) {
        state.layout.background_r = r;
        state.layout.background_g = g;
        state.layout.background_b = b;
        state.layout.background_a = a;
    }

    // [[keybind]] array
    eprintln!("[config] processing {} keybinds...", config.keybind.len());
    for kb in &config.keybind {
        let mods = parse_modifiers(&kb.mods);
        let keysym = parse_keysym(&kb.key);
        let action = parse_action(&kb.action);
        let command = if action == crate::types::Action::Spawn || action == crate::types::Action::Toggle {
            kb.command.clone()
        } else {
            None
        };

        state.pending_bindings.push(PendingXkbBinding {
            mods,
            keysym,
            action,
            command,
        });
    }

    // Register default super+left and super+right bindings for side panel position if not overridden
    let super_mod = parse_modifiers("super");
    let left_sym = parse_keysym("Left");
    let right_sym = parse_keysym("Right");

    let has_super_left = state.pending_bindings.iter().any(|b| b.mods == super_mod && b.keysym == left_sym);
    if !has_super_left {
        state.pending_bindings.push(PendingXkbBinding {
            mods: super_mod,
            keysym: left_sym,
            action: crate::types::Action::SidePanelLeft,
            command: None,
        });
    }

    let has_super_right = state.pending_bindings.iter().any(|b| b.mods == super_mod && b.keysym == right_sym);
    if !has_super_right {
        state.pending_bindings.push(PendingXkbBinding {
            mods: super_mod,
            keysym: right_sym,
            action: crate::types::Action::SidePanelRight,
            command: None,
        });
    }

    // [[pointer_bind]] array
    for pb in &config.pointer_bind {
        let mods = parse_modifiers(&pb.mods);
        let button = parse_button(&pb.button);
        let action = parse_action(&pb.action);

        state.pending_pointer_bindings.push(PendingPointerBinding {
            mods,
            button,
            action,
        });
    }

    // [[mode_rule]] array
    for mr in &config.mode_rule {
        let mode = parse_tiling_mode(&mr.mode);
        let tag = mr.tag.unwrap_or(0) as i32;
        state.mode_rules.push(ModeRule {
            mode,
            app_id_pattern: mr.app_id.clone(),
            title_pattern: mr.title.clone(),
            single_instance: mr.single.unwrap_or(false),
            tag,
            circular: mr.circular.unwrap_or(false),
            ssd: mr.ssd,
        });
    }

    // [[tag_layout]] array
    for tl in &config.tag_layout {
        let tag = tl.tag as i32;
        if tag >= 1 && tag <= NUM_TAGS as i32 {
            let mode = parse_tiling_mode(&tl.mode);
            state.tag_layouts[tag as usize - 1] = mode;
            state.has_tag_layout[tag as usize - 1] = true;
        }
    }

    // [env] — set environment variables BEFORE startup apps are processed,
    // so that PATH and other vars are available when spawn_command_bg runs.
    // Supports $VAR and ${VAR} expansion using the current environment.
    for (key, value) in &config.env {
        let expanded = expand_env_vars(value);
        std::env::set_var(key, &expanded);
        state.env_vars.insert(key.clone(), expanded);
    }

    // [[startup]] array — queue apps for spawning inside the render callback.
    // Spawning between blocking_dispatch calls corrupts the Wayland connection
    // because the fork inherits the socket fd, so we defer to render time.
    state.pending_startup_apps.clear();
    eprintln!(
        "[config] processing {} startup entries (cold_start={})...",
        config.startup.len(),
        cold_start
    );
    for (i, entry) in config.startup.iter().enumerate() {
        let name = extract_program_name(&entry.exec);
        let should_restart = entry.restart.unwrap_or(false);

        if entry.once && !cold_start && !should_restart {
            eprintln!(
                "[config] startup[{}]: exec=\"{}\" once=true → skipped (not cold start)",
                i, entry.exec
            );
            continue;
        }

        let running = process_running(&name);
        if should_restart && running {
            eprintln!(
                "[config] startup[{}]: exec=\"{}\" restart=true → killing existing process {}",
                i, entry.exec, name
            );
            let mut pkill_cmd = std::process::Command::new("pkill");
            if name.len() > 15 {
                pkill_cmd.arg("-f").arg(&name);
            } else {
                pkill_cmd.arg("-x").arg(&name);
            }
            match pkill_cmd.status() {
                Ok(status) => {
                    eprintln!("[config] pkill status: {}", status);
                    let start = std::time::Instant::now();
                    while process_running(&name) && start.elapsed().as_secs_f64() < 1.0 {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    eprintln!(
                        "[config] process {} exited after pkill: {}",
                        name,
                        !process_running(&name)
                    );
                }
                Err(e) => {
                    eprintln!("[config] failed to execute pkill: {:?}", e);
                }
            }
            state.pending_startup_apps.push(entry.exec.clone());
        } else if running {
            eprintln!("[config] startup[{}]: exec=\"{}\" once={} → skipped (already running, pgrep -x {})", i, entry.exec, entry.once, name);
        } else {
            eprintln!(
                "[config] startup[{}]: exec=\"{}\" once={} → queued for spawn",
                i, entry.exec, entry.once
            );
            state.pending_startup_apps.push(entry.exec.clone());
        }
    }

    // [output] scale — applied via wlr-output-management protocol.
    // After storing the scale, set pending_scale_apply so that the next
    // output_manager done event triggers the configuration. This handles
    // both initial startup and VT-switch-back (where wlroots resets scale to 1).
    state.output_scale = if config.output.scale > 0.0 {
        state.pending_scale_apply = true;
        config.output.scale
    } else {
        0.0
    };

    // [input] tap_to_click — applied via river-libinput-config protocol.
    // The tap config will be applied when libinput devices are discovered
    // (in the RiverLibinputDeviceV1 TapSupport event handler).
    state.tap_to_click = config.input.tap_to_click;
    state.accel_speed = config.input.accel_speed;
    state.accel_profile = config.input.accel_profile.clone();
    state.natural_scroll = config.input.natural_scroll;
    state.dwt = config.input.dwt;
    state.dwtp = config.input.dwtp;
    state.trackpoint_accel_speed = config.input.trackpoint_accel_speed;
    state.trackpoint_accel_profile = config.input.trackpoint_accel_profile.clone();
    state.tap_config_applied = false;

    if state.cursor_theme != config.input.cursor_theme || state.cursor_size != config.input.cursor_size {
        state.cursor_theme = config.input.cursor_theme.clone();
        state.cursor_size = config.input.cursor_size;
        state.cursor_theme_applied = false;
    }

    // Send InertialConfig and tap_to_click state to input subsystem daemon
    let inertial_cfg = config.inertial.clone().unwrap_or_else(|| {
        InertialConfig {
            inertial_scroll: true,
            scroll_friction: 90,
            inertial_pointer: false,
            pointer_friction: 95,
            inertial_trackpad: false,
            trackpad_friction: 95,
            pointer_speed: 1.0,
            scroll_speed: 1.0,
            trackpad_speed: 1.0,
        }
    });
    if let Some(ref controller) = state.input_controller {
        let _ = controller.send(crate::input::InputDaemonMsg::UpdateConfig(inertial_cfg, config.input.tap_to_click));
    }

    // [notifications]
    state.notifications_enable = config.notifications.enable;

    // [[reload]] array
    state.reload_commands.clear();
    for entry in &config.reload {
        state.reload_commands.push(entry.exec.clone());
    }

    // Signal config-done
    state.config_done = true;
    Ok(())
}

/// Expand $VAR and ${VAR} references in a string using the current environment.
/// Unset variables expand to empty strings. $$ is not expanded (not a shell).
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

/// Extract the program name (first word, basename) from a command string
pub fn extract_program_name(cmd: &str) -> String {
    let cmd = cmd.trim_start();
    let first_word: String = cmd.chars().take_while(|c| !c.is_whitespace()).collect();
    if let Some(slash) = first_word.rfind('/') {
        first_word[slash + 1..].to_string()
    } else {
        first_word
    }
}

/// Parse a key name string into an xkb keysym value.
/// Uses xkbcommon to resolve key names.
pub fn parse_keysym(key_str: &str) -> u32 {
    let name = if key_str.starts_with("XKB_KEY_") {
        &key_str[8..]
    } else {
        key_str
    };
    // Use xkbcommon to parse the keysym
    xkbcommon::xkb::keysym_from_name(name, xkbcommon::xkb::KEYSYM_CASE_INSENSITIVE).into()
}

/// Spawn a command in the background.
///
/// Closes all inherited FDs > 2 in the child via pre_exec so that
/// spawned Wayland clients (fuzzel, foot, etc.) never accidentally
/// read from cce-client's Wayland socket fd. Also redirects stdout/stderr
/// to /dev/null so child output doesn't pollute cce-client's log, and
/// calls setsid() to detach from cce-client's process group.
pub fn spawn_command_bg(cmd: &str) {
    use std::os::unix::process::CommandExt;
    let cmd = cmd.to_string();
    
    let stdout_cfg = if let Ok(f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/cce-client-spawned-apps.log")
    {
        std::process::Stdio::from(f)
    } else {
        std::process::Stdio::null()
    };

    let stderr_cfg = if let Ok(f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/cce-client-spawned-apps.log")
    {
        std::process::Stdio::from(f)
    } else {
        std::process::Stdio::null()
    };

    let _ = unsafe {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .env_remove("WAYLAND_DEBUG")
            .stdout(stdout_cfg)
            .stderr(stderr_cfg)
            .pre_exec(|| {
                // Close all inherited FDs > 2 to prevent the child from
                // accidentally reading cce-client's Wayland socket or status
                // socket FDs. close() and setsid() are async-signal-safe.
                let max_fd = libc::sysconf(libc::_SC_OPEN_MAX) as libc::c_int;
                for fd in 3..max_fd {
                    libc::close(fd);
                }
                libc::setsid();
                Ok(())
            })
            .spawn()
    };
}
pub fn process_running(name: &str) -> bool {
    let mut cmd = std::process::Command::new("pgrep");
    if name.len() > 15 {
        cmd.arg("-f").arg(name);
    } else {
        cmd.arg("-x").arg(name);
    }
    match cmd.output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Show a system desktop notification via notify-send.
pub fn show_notification(title: &str, body: &str) {
    let title_escaped = title.replace('\'', "'\\''");
    let body_escaped = body.replace('\'', "'\\''");
    let cmd = format!("notify-send -a cce-client '{}' '{}'", title_escaped, body_escaped);
    spawn_command_bg(&cmd);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_program_name() {
        assert_eq!(extract_program_name("ghostty"), "ghostty");
        assert_eq!(extract_program_name("ghostty -e something"), "ghostty");
        assert_eq!(
            extract_program_name("/home/lsgalante/.local/bin/tmux-startup --recreate main"),
            "tmux-startup"
        );
        assert_eq!(extract_program_name("  waybar"), "waybar");
    }

    #[test]
    fn test_config_layout_defaults() {
        let lc = LayoutConfig::default();
        assert_eq!(lc.gap, 48);
        assert_eq!(lc.gap_top, 48);
        assert_eq!(lc.gap_left, 48);
        assert_eq!(lc.gap_right, 48);
        assert_eq!(lc.gap_bottom, 48);
        assert_eq!(lc.cascade_offset, 20);
        assert_eq!(lc.bar_height, 24);
        assert_eq!(lc.border_width, 6);
        assert_eq!(lc.fullscreen_border_width, 0);
        assert_eq!(lc.border_color, "#3e3e3e");
        assert_eq!(lc.border_font_size, 11);
        assert_eq!(lc.grid_gap, 18);
        assert_eq!(lc.side_panel_width, 360);
        assert_eq!(lc.side_panel_behavior, "inline");
        assert_eq!(lc.side_panel_position, "left");
        assert_eq!(lc.side_panel_border_gap, 0);
        assert_eq!(lc.side_panel_border_opacity, 100);
    }

    #[test]
    fn test_expand_env_vars_simple() {
        std::env::set_var("CCE_CLIENT_TEST_VAR_SIMPLE", "hello");
        assert_eq!(expand_env_vars("$CCE_CLIENT_TEST_VAR_SIMPLE"), "hello");
        std::env::remove_var("CCE_CLIENT_TEST_VAR_SIMPLE");
    }

    #[test]
    fn test_expand_env_vars_braces() {
        std::env::set_var("CCE_CLIENT_TEST_VAR_BRACES", "world");
        assert_eq!(expand_env_vars("${CCE_CLIENT_TEST_VAR_BRACES}!"), "world!");
        std::env::remove_var("CCE_CLIENT_TEST_VAR_BRACES");
    }

    #[test]
    fn test_expand_env_vars_mid_string() {
        std::env::set_var("CCE_CLIENT_TEST_HOME", "/home/user");
        assert_eq!(
            expand_env_vars("$CCE_CLIENT_TEST_HOME/.local/bin:$CCE_CLIENT_TEST_HOME/bin"),
            "/home/user/.local/bin:/home/user/bin"
        );
        std::env::remove_var("CCE_CLIENT_TEST_HOME");
    }

    #[test]
    fn test_expand_env_vars_unset() {
        assert_eq!(expand_env_vars("$CCE_CLIENT_NONEXISTENT_VAR"), "");
    }

    #[test]
    fn test_expand_env_vars_no_vars() {
        assert_eq!(expand_env_vars("plain string"), "plain string");
    }

    #[test]
    fn test_expand_env_vars_dollar_non_identifier() {
        // $$ and $: should be kept as-is (not a shell)
        assert_eq!(expand_env_vars("$$"), "$$");
        assert_eq!(expand_env_vars("$:"), "$:");
    }

    #[test]
    fn test_expand_env_vars_path_append() {
        // Simulates the real use case: PATH = "$HOME/.local/bin:$PATH"
        let orig_path = std::env::var("PATH").unwrap_or_default();
        let home = std::env::var("HOME").unwrap_or_default();
        let expanded = expand_env_vars("$HOME/.local/bin:$PATH");
        assert!(expanded.starts_with(&format!("{}/.local/bin:", home)));
        assert!(expanded.contains(&orig_path));
    }

    #[test]
    fn test_notifications_config_parse() {
        let toml_str = r#"
[notifications]
enable = false
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.notifications.enable);

        let toml_str_empty = "";
        let config_empty: Config = toml::from_str(toml_str_empty).unwrap();
        assert!(config_empty.notifications.enable);
    }

    #[test]
    fn test_parse_keysym_favorites() {
        let sym = parse_keysym("XF86Favorites");
        assert!(sym != 0, "XF86Favorites keysym must be resolved successfully");
    }
}

#[cfg(test)]
mod startup_format_tests {
    use super::*;

    #[test]
    fn test_startup_entry_format() {
        let toml_str = r#"
[env]
XDG_CURRENT_DESKTOP = "river"

[[startup]]
exec = "waybar"

[[startup]]
exec = "fuzzel"
once = true

[[startup]]
exec = "cce-system-interface"
once = true
restart = true
"#;
        let config: Config = toml::from_str(toml_str).expect("TOML parse failed");
        assert_eq!(config.startup.len(), 3);
        assert_eq!(config.startup[0].exec, "waybar");
        assert!(!config.startup[0].once);
        assert_eq!(config.startup[1].exec, "fuzzel");
        assert!(config.startup[1].once);
        assert_eq!(config.startup[2].exec, "cce-system-interface");
        assert!(config.startup[2].once);
        assert_eq!(config.startup[2].restart, Some(true));
        assert_eq!(config.env.get("XDG_CURRENT_DESKTOP").unwrap(), "river");
    }

    #[test]
    fn test_reload_entry_format() {
        let toml_str = r#"
[[reload]]
exec = "pkill clear-input-daemon"

[[reload]]
exec = "echo reloaded"
"#;
        let config: Config = toml::from_str(toml_str).expect("TOML parse failed");
        assert_eq!(config.reload.len(), 2);
        assert_eq!(config.reload[0].exec, "pkill clear-input-daemon");
        assert_eq!(config.reload[1].exec, "echo reloaded");

        // Also test integration via parse_config by writing to a temporary file
        let temp_path_buf = std::env::temp_dir().join("scratch_config_test.toml");
        let temp_path = temp_path_buf.to_str().unwrap();
        std::fs::write(temp_path, toml_str).unwrap();

        let mut wm = WindowManager::default();
        let parse_res = parse_config(temp_path, false, &mut wm);
        let _ = std::fs::remove_file(temp_path);

        parse_res.unwrap();
        assert_eq!(wm.reload_commands.len(), 2);
        assert_eq!(wm.reload_commands[0], "pkill clear-input-daemon");
        assert_eq!(wm.reload_commands[1], "echo reloaded");
    }
}
