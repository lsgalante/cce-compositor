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
    pub repeat: RepeatConfig,
    #[serde(default)]
    pub startup: StartupConfig,
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
    #[serde(default = "default_offset")]
    pub offset: i64,
    #[serde(default = "default_bar_height")]
    pub bar_height: i64,
    #[serde(default = "default_border_width")]
    pub border_width: i64,
    #[serde(default = "default_border_color")]
    pub border_color: String,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        LayoutConfig {
            gap: default_gap(),
            offset: default_offset(),
            bar_height: default_bar_height(),
            border_width: default_border_width(),
            border_color: default_border_color(),
        }
    }
}

fn default_gap() -> i64 {
    48
}
fn default_offset() -> i64 {
    20
}
fn default_bar_height() -> i64 {
    24
}
fn default_border_width() -> i64 {
    6
}
fn default_border_color() -> String {
    "#3e3e3e".to_string()
}

#[derive(Debug, Deserialize, Default)]
pub struct OutputConfig {
    #[serde(default)]
    pub scale: i64,
}

#[derive(Debug, Deserialize, Default)]
pub struct RepeatConfig {
    #[serde(default)]
    pub rate: i64,
    #[serde(default)]
    pub delay: i64,
}

#[derive(Debug, Deserialize, Default)]
pub struct StartupConfig {
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default)]
    pub cold_start_only: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
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
/// `cold_start` controls whether cold_start_only apps are spawned.
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
    state.layout.gap = config.layout.gap as i32;
    state.layout.offset = config.layout.offset as i32;
    state.layout.bar_height = config.layout.bar_height as i32;
    state.layout.border_width = config.layout.border_width as i32;
    if let Some((r, g, b, a)) = parse_hex_color(&config.layout.border_color) {
        state.layout.border_r = r;
        state.layout.border_g = g;
        state.layout.border_b = b;
        state.layout.border_a = a;
    }

    // [[keybind]] array
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

    // [startup] section — spawn apps
    for app in &config.startup.apps {
        // Extract program name for skip-if-running check
        let name = extract_program_name(app);
        if process_running(&name) {
            continue;
        }
        // If waybar, kill existing before launching
        // NOTE: pkill + sleep is too slow for nested mode where River's
        // 3-second unresponsive timer is ticking. Just launch waybar
        // directly — if an existing waybar is running, the new one will
        // replace it (or the old one can be killed manually).
        // if name == "waybar" {
        //     let _ = std::process::Command::new("pkill")
        //         .arg("waybar")
        //         .output();
        //     std::thread::sleep(std::time::Duration::from_millis(100));
        // }
        spawn_command_bg(app);
    }

    // Cold-start-only apps
    if cold_start {
        for app in &config.startup.cold_start_only {
            spawn_command_bg(app);
        }
    }

    // [startup.env] — set environment variables
    for (key, value) in &config.startup.env {
        std::env::set_var(key, value);
        state.env_vars.insert(key.clone(), value.clone());
    }

    // [output] scale — handled at startup
    if config.output.scale > 0 && cold_start {
        // Scale is applied via wlr-randr; we just store the value
        // The actual wlr-randr call would happen in the Wayland integration layer
    }

    // Signal config-done
    state.config_done = true;
}

/// Extract the program name (first word, basename) from a command string
fn extract_program_name(cmd: &str) -> String {
    let cmd = cmd.trim_start();
    let first_word: String = cmd
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect();
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

/// Spawn a command in the background (double-fork style)
pub fn spawn_command_bg(cmd: &str) {
    use std::os::unix::process::CommandExt;
    let cmd = cmd.to_string();
    // Double-fork: first fork setsid, second fork execs
    // Safety: pre_exec is unsafe because it runs between fork and exec.
    // We only call setsid() which is async-signal-safe.
    let _ = unsafe {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .pre_exec(|| {
                libc::setsid();
                Ok(())
            })
            .spawn()
    };
}

/// Check if a process with the given name is already running
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
        assert_eq!(lc.offset, 20);
        assert_eq!(lc.bar_height, 24);
        assert_eq!(lc.border_width, 6);
        assert_eq!(lc.border_color, "#3e3e3e");
    }
}
