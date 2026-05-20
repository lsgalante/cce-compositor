// TOML config parsing for clearwm

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
    #[serde(default = "default_vsplit_border_width")]
    pub vsplit_border_width: i64,
    #[serde(default = "default_hsplit_border_width")]
    pub hsplit_border_width: i64,
    #[serde(default = "default_floating_border_width")]
    pub floating_border_width: i64,
    #[serde(default = "default_border_color")]
    pub border_color: String,
    #[serde(default = "default_background_color")]
    pub background_color: String,
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
            vsplit_border_width: default_vsplit_border_width(),
            hsplit_border_width: default_hsplit_border_width(),
            floating_border_width: default_floating_border_width(),
            border_color: default_border_color(),
            background_color: default_background_color(),
        }
    }
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
fn default_vsplit_border_width() -> i64 {
    6
}
fn default_hsplit_border_width() -> i64 {
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
}

#[derive(Debug, Deserialize)]
pub struct StartupEntryConfig {
    pub exec: String,
    #[serde(default)]
    pub once: bool,
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
}

#[derive(Debug, Deserialize)]
pub struct TagLayoutConfig {
    pub tag: i64,
    pub mode: String,
}

/// Parse the TOML config file and apply it to the WindowManager state.
/// `cold_start` controls whether `once = true` startup entries are spawned.
pub fn parse_config(path: &str, cold_start: bool, state: &mut WindowManager) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("parse_config: cannot open {}: {}", path, e);
            return;
        }
    };

    let config: Config = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("parse_config: TOML parse error: {}", e);
            return;
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
    state.layout.vsplit_border_width = config.layout.vsplit_border_width as i32;
    state.layout.hsplit_border_width = config.layout.hsplit_border_width as i32;
    state.layout.floating_border_width = config.layout.floating_border_width as i32;
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
        let command = if action == crate::types::Action::Spawn {
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
        if entry.once && !cold_start {
            eprintln!(
                "[config] startup[{}]: exec=\"{}\" once=true → skipped (not cold start)",
                i, entry.exec
            );
            continue;
        }
        let running = process_running(&name);
        if running {
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

    // Signal config-done
    state.config_done = true;
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
fn extract_program_name(cmd: &str) -> String {
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
/// read from clearwm's Wayland socket fd. Also redirects stdout/stderr
/// to /dev/null so child output doesn't pollute clearwm's log, and
/// calls setsid() to detach from clearwm's process group.
pub fn spawn_command_bg(cmd: &str) {
    use std::os::unix::process::CommandExt;
    let cmd = cmd.to_string();
    let _ = unsafe {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .env_remove("WAYLAND_DEBUG")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .pre_exec(|| {
                // Close all inherited FDs > 2 to prevent the child from
                // accidentally reading clearwm's Wayland socket or status
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
    match std::process::Command::new("pgrep")
        .arg("-x")
        .arg(name)
        .output()
    {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
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
    }

    #[test]
    fn test_expand_env_vars_simple() {
        std::env::set_var("CLEARWM_TEST_VAR", "hello");
        assert_eq!(expand_env_vars("$CLEARWM_TEST_VAR"), "hello");
        std::env::remove_var("CLEARWM_TEST_VAR");
    }

    #[test]
    fn test_expand_env_vars_braces() {
        std::env::set_var("CLEARWM_TEST_VAR", "world");
        assert_eq!(expand_env_vars("${CLEARWM_TEST_VAR}!"), "world!");
        std::env::remove_var("CLEARWM_TEST_VAR");
    }

    #[test]
    fn test_expand_env_vars_mid_string() {
        std::env::set_var("CLEARWM_TEST_HOME", "/home/user");
        assert_eq!(
            expand_env_vars("$CLEARWM_TEST_HOME/.local/bin:$CLEARWM_TEST_HOME/bin"),
            "/home/user/.local/bin:/home/user/bin"
        );
        std::env::remove_var("CLEARWM_TEST_HOME");
    }

    #[test]
    fn test_expand_env_vars_unset() {
        assert_eq!(expand_env_vars("$CLEARWM_NONEXISTENT_VAR"), "");
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
"#;
        let config: Config = toml::from_str(toml_str).expect("TOML parse failed");
        assert_eq!(config.startup.len(), 2);
        assert_eq!(config.startup[0].exec, "waybar");
        assert!(!config.startup[0].once);
        assert_eq!(config.startup[1].exec, "fuzzel");
        assert!(config.startup[1].once);
        assert_eq!(config.env.get("XDG_CURRENT_DESKTOP").unwrap(), "river");
    }
}
