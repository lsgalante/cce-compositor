// Restart and reload logic for ccec

use crate::config::parse_config;
use crate::types::WindowManager;

/// Restart the window manager process.
///
/// This forks a child process that waits briefly for the parent to die
/// (so River cleans up the old Wayland connection), then execs a fresh
/// ccec instance. The parent (current process) exits immediately.
///
/// The CCEC_RESTARTING environment variable signals that this is a restart
/// (not a cold start), so the new process skips cold_start_only apps.
///
/// ## Why setsid() is required
///
/// ccec is typically a session leader (PID == SID, started by River's
/// `-c` launch script). When a session leader exits, the kernel sends SIGHUP
/// to all processes in that session — including the forked child. Without
/// `setsid()`, the child dies from SIGHUP before it can exec, and ccec
/// never comes back.
pub fn wm_restart() {
    use std::time::Instant;

    use crate::paths;

    // Write to death log before fork — this is the last chance to capture
    // why we're restarting, since fork+process::exit(0) silently kills the parent.
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::get_death_log_path())
    {
        use std::io::Write;
        let _ = writeln!(f, "wm_restart() called — about to fork+exit");
        let _ = f.sync_all(); // ensure it hits disk before process::exit(0)
    }

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
    std::env::set_var("CCEC_RESTARTING", "1");

    // Persist state to ~/.cache/ccec_state for the new instance to restore.
    // Note: we can't pass &WindowManager here since wm_restart() has no access
    // to it. The state file is kept fresh by RenderStart's needs_status_update
    // path, so it should be reasonably up-to-date already.

    // Save the current log before River's launch script truncates it on restart.
    // This preserves the crash/reason for the restart.
    let _ = std::fs::copy(paths::get_log_path(), paths::get_prev_log_path());

    // Remove the IPC socket
    let _ = std::fs::remove_file(paths::get_socket_path());

    // Get the current executable path.
    // CString is required because execl() needs a null-terminated C string;
    // Rust's String::as_ptr() is NOT guaranteed to be null-terminated.
    //
    // We prefer the symlink path (~/.local/bin/ccec) over current_exe()
    // because current_exe() resolves through /proc/self/exe to the real path,
    // which may be on a sync filesystem (Dropbox) that temporarily moves files.
    // The symlink is on the root filesystem and always available.
    let home = std::env::var("HOME").unwrap_or_default();
    let exe_path = if !home.is_empty() {
        let symlink = format!("{}/.local/bin/ccec", home);
        if std::path::Path::new(&symlink).exists() {
            std::path::PathBuf::from(symlink)
        } else {
            std::env::current_exe().unwrap_or_else(|_| std::process::exit(1))
        }
    } else {
        std::env::current_exe().unwrap_or_else(|_| std::process::exit(1))
    };
    eprintln!("wm_restart: exe_path={}", exe_path.display());
    let path_cstr = std::ffi::CString::new(exe_path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| std::process::exit(1));

    // Fork: child waits for parent to die, then execs fresh ccec.
    // Parent exits so River tears down the old Wayland connection.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        // fork failed, just exit
        std::process::exit(1);
    } else if pid == 0 {
        // ── Child process ──

        // Create a new session IMMEDIATELY. Without this, the child stays
        // in the parent's session. When the parent (session leader) calls
        // process::exit(0), the kernel sends SIGHUP to every process in that
        // session — killing the child before it can exec.
        unsafe {
            libc::setsid();
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }

        // Wait for parent to exit so River cleans up the old Wayland
        // connection before we try to connect fresh.
        let parent_pid = unsafe { libc::getppid() };
        std::thread::sleep(std::time::Duration::from_millis(500));

        // If parent is somehow still alive, wait a bit more
        if unsafe { libc::kill(parent_pid, 0) == 0 } {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        // Close inherited Wayland FDs so the new ccec instance doesn't
        // confuse River with stale connections.
        // (close everything except stdin/stdout/stderr)
        let max_fd = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) } as i32;
        for fd in 3..max_fd {
            unsafe {
                libc::close(fd);
            }
        }

        // Exec the same binary — replaces this process with a fresh ccec.
        // Retry up to 3 times with a short delay — the binary may be temporarily
        // unavailable if cargo build is replacing it mid-write (atomic rename).
        for attempt in 0..3 {
            let ret = unsafe {
                libc::execl(
                    path_cstr.as_ptr(),
                    path_cstr.as_ptr(),
                    std::ptr::null::<i8>(),
                )
            };
            let errno = unsafe { *libc::__errno_location() };
            if attempt < 2 {
                eprintln!(
                    "wm_restart: execl attempt {} failed (errno={} {}), retrying in 500ms...",
                    attempt + 1,
                    errno,
                    std::io::Error::from_raw_os_error(errno)
                );
                std::thread::sleep(std::time::Duration::from_millis(500));
                let _ = ret; // suppress unused
            } else {
                // Final attempt failed — log and exit
                eprintln!(
                    "wm_restart: execl failed after 3 attempts! errno={} ({})",
                    errno,
                    std::io::Error::from_raw_os_error(errno)
                );
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(paths::get_death_log_path())
                {
                    use std::io::Write;
                    let _ = writeln!(
                        f,
                        "child: execl failed after 3 attempts! errno={} ({}) path={}",
                        errno,
                        std::io::Error::from_raw_os_error(errno),
                        exe_path.display()
                    );
                }
                let _ = ret; // suppress unused
                std::process::exit(1);
            }
        }
    } else {
        // ── Parent process ──
        // Exit immediately so River sees the Wayland connection drop
        // and tears down the old WM binding.
        std::process::exit(0);
    }
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
    state.tag_layouts = [crate::types::TilingMode::Cascade; crate::types::NUM_TAGS];
    state.has_tag_layout = [true; crate::types::NUM_TAGS];

    // Mark status for update after reload
    state.needs_status_update = true;
    state.tap_config_applied = false;
    state.startup_spawned = false;

    // Clear mode rules
    state.mode_rules.clear();

    // Clear pending bindings
    state.pending_bindings.clear();
    state.pending_pointer_bindings.clear();

    // Try TOML config first
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        let config_path = format!("{}/.config/ccec/config.toml", home);
        if std::path::Path::new(&config_path).exists() {
            match parse_config(&config_path, false, state) {
                Ok(_) => {
                    for cmd in &state.reload_commands {
                        eprintln!("[reload] executing reload command: {}", cmd);
                        spawn_command_bg(cmd);
                    }
                    if state.notifications_enable {
                        crate::config::show_notification("ccec", "Configuration reloaded successfully");
                    }
                }
                Err(e) => {
                    if state.notifications_enable {
                        crate::config::show_notification("ccec", &format!("Config reload failed:\n{}", e));
                    }
                }
            }
        }
    }
}

/// Spawn a command in the background (re-export from config for convenience)
pub use crate::config::spawn_command_bg;

/// Check if a process with the given name is already running (re-export from config)
pub use crate::config::process_running;
