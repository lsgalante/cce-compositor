// clearwm — Wayland window manager for river

use clearwm::config::parse_config;
use clearwm::restart;
use clearwm::status_server;
use clearwm::wayland::wayland_init;
use std::env;
use std::fs;

const SOCKET_PATH: &str = "/tmp/clearwm.sock";

fn main() {
    eprintln!("clearwm starting...");

    // Start the status socket server thread (for waybar integration)
    let status_sender = status_server::spawn_status_server();

    // Set up SIGCHLD handler to reap child processes
    let sa = nix::sys::signal::SigAction::new(
        nix::sys::signal::SigHandler::Handler(sigchld_handler),
        nix::sys::signal::SaFlags::SA_RESTART,
        nix::sys::signal::SigSet::empty(),
    );
    unsafe {
        nix::sys::signal::sigaction(nix::sys::signal::Signal::SIGCHLD, &sa)
            .expect("failed to set SIGCHLD handler");
    }

    // Connect to Wayland display and get initial state.
    let (_conn, mut event_queue, mut state) = match wayland_init() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fatal: {}", e);
            std::process::exit(1);
        }
    };

    // Store the status sender in the app state so RenderStart can push updates
    state.status_sender = Some(status_sender);

    // Check if this is a restart
    let cold_start = if env::var("CLEARWM_RESTARTING").as_deref() == Ok("1") {
        env::remove_var("CLEARWM_RESTARTING");
        false
    } else {
        true
    };

    env::remove_var("WAYLAND_DEBUG");

    // Load config immediately, before entering the event loop.
    // This ensures bindings are registered before any render_start arrives.
    // Following the tinyrwm pattern: do setup, then simple blocking_dispatch loop.
    eprintln!(
        "[init] about to load config, render_count={}",
        state.render_count
    );
    let config_start = std::time::Instant::now();
    if let Ok(home) = env::var("HOME") {
        let config_path = format!("{}/.config/clearwm/config.toml", home);
        if fs::metadata(&config_path).is_ok() {
            parse_config(&config_path, cold_start, &mut state.wm);
        }
    }
    eprintln!(
        "[init] config loaded in {:?}, render_count={}",
        config_start.elapsed(),
        state.render_count
    );

    // Apply output scale immediately if heads were discovered during init roundtrips.
    if state.wm.pending_scale_apply && state.wm.output_scale > 0.0 && !state.output_heads.is_empty()
    {
        let qh = event_queue.handle();
        clearwm::wayland::apply_output_scale(&mut state, &qh);
    }

    // Flush any queued requests from config loading (bindings, etc.)
    eprintln!("[init] flushed, entering main loop");

    // Main loop — tinyrwm pattern: just blocking_dispatch in a loop.
    // All work (including spawning) happens inside Dispatch callbacks.
    loop {
        match event_queue.blocking_dispatch(&mut state) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("wayland dispatch error: {:?}", e);
                if !state.wm.exit_requested {
                    restart::wm_restart();
                }
                break;
            }
        }
        if state.exit_requested || state.wm.exit_requested {
            break;
        }
    }
    let _ = fs::remove_file(SOCKET_PATH);
    if !state.wm.exit_requested {
        restart::wm_restart();
    }
    eprintln!("main loop exited");
}

extern "C" fn sigchld_handler(_sig: nix::libc::c_int) {
    while nix::sys::wait::waitpid(
        nix::unistd::Pid::from_raw(-1),
        Some(nix::sys::wait::WaitPidFlag::WNOHANG),
    )
    .is_ok()
    {}
}
