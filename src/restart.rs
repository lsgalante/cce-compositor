// Restart and reload logic for clearwm

use crate::config::parse_config;
use crate::types::WindowManager;

/// Restart the window manager process.
///
/// This uses execl() to replace the current process with a fresh instance.
/// The CLEARWM_RESTARTING environment variable signals that this is a restart
/// (not a cold start), so the new process skips cold_start_only apps.
///
/// Ported from C wm_restart() with throttle logic.
pub fn wm_restart() {
    use std::time::Instant;

    static mut LAST_RESTART: Option<Instant> = None;

    // Throttle restarts (minimum 2 seconds between restarts)
    // Safety: single-threaded access; acceptable for this use case
    let now = Instant::now();
    let should_throttle = unsafe {
        if let Some(last) = LAST_RESTART {
            now.duration_since(last).as_secs_f64() < 2.0
        } else {
            false
        }
    };

    if should_throttle {
        std::thread::sleep(std::time::Duration::from_secs(2));
    }

    unsafe {
        LAST_RESTART = Some(now);
    }

    // Signal that this is a restart, not a cold start
    std::env::set_var("CLEARWM_RESTARTING", "1");

    // Remove the IPC socket
    let _ = std::fs::remove_file("/tmp/clearwm.sock");

    // Get the current executable path
    if let Ok(exe_path) = std::env::current_exe() {
        let path_str = exe_path.to_string_lossy().to_string();
        // Use execl via libc to replace the current process
        let ret = unsafe {
            libc::execl(
                path_str.as_ptr() as *const i8,
                path_str.as_ptr() as *const i8,
                std::ptr::null::<i8>(),
            )
        };
        if ret < 0 {
            eprintln!("wm_restart: execl failed");
        }
    }

    // If execl failed, exit
    std::process::exit(1);
}

/// Reload the configuration file.
///
/// This clears mode rules, pending bindings, and seat bindings, then
/// re-parses the config file.
///
/// Ported from C wm_reload().
pub fn wm_reload(state: &mut WindowManager) {
    // Unlock mode on all windows so the new rules apply to them
    for window in &mut state.windows {
        window.mode_locked = false;
    }

    // Clear per-tag layout defaults
    state.has_tag_layout = [false; crate::types::NUM_TAGS];

    // Clear mode rules
    state.mode_rules.clear();

    // Clear pending bindings
    state.pending_bindings.clear();
    state.pending_pointer_bindings.clear();

    // Try TOML config first
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        let config_path = format!("{}/.config/clearwm/config.toml", home);
        if std::path::Path::new(&config_path).exists() {
            parse_config(&config_path, false, state);
        }
    }
}

/// Spawn a command in the background (re-export from config for convenience)
pub use crate::config::spawn_command_bg;

/// Check if a process with the given name is already running (re-export from config)
pub use crate::config::process_running;
