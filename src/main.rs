// clearwm — Wayland window manager for river

use clearwm::config::parse_config;
use clearwm::restart;
use clearwm::status_server;
use clearwm::wayland::wayland_init;
use std::env;
use std::fs;

const SOCKET_PATH: &str = "/tmp/clearwm.sock";

/// Write a crash/exit trace to /tmp/clearwm-death.log so we can diagnose
/// why clearwm dies even when the normal log gets overwritten on restart.
fn log_death(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/clearwm-death.log")
    {
        let _ = writeln!(f, "{}", msg);
    }
    eprintln!("{}", msg);
}

fn main() {
    // Install a panic hook that writes to a separate log file before aborting.
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("[PANIC] {}", info);
        eprintln!("{}", msg);
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/clearwm-death.log")
        {
            let _ = writeln!(f, "{}", msg);
        }
    }));

    eprintln!("clearwm starting...");

    // Start the status socket server thread (for waybar integration)
    let status_sender = status_server::spawn_status_server();

    // Ignore SIGPIPE — when the Wayland compositor closes the connection
    // (protocol error, unresponsive timeout, etc.), writes to the socket
    // generate SIGPIPE. Without ignoring it, the process is killed before
    // conn.flush() or blocking_dispatch() can return an error, masking the
    // real cause of the disconnect.
    unsafe {
        nix::libc::signal(nix::libc::SIGPIPE, nix::libc::SIG_IGN);
    }

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
    // SIGUSR2 handler: dump backtrace to /tmp/clearwm-bt.txt for debugging busy loops
    unsafe {
        nix::sys::signal::sigaction(
            nix::sys::signal::SIGUSR2,
            &nix::sys::signal::SigAction::new(
                nix::sys::signal::SigHandler::Handler(dump_backtrace),
                nix::sys::signal::SaFlags::empty(),
                nix::sys::signal::SigSet::empty(),
            ),
        )
        .expect("failed to set SIGUSR2 handler");
    }

    let (_conn, mut event_queue, mut state) = match wayland_init() {
        Ok(c) => c,
        Err(e) => {
            log_death(&format!("fatal: {}", e));
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

    // Don't remove WAYLAND_DEBUG — we need protocol debug logging to diagnose crashes
    // env::remove_var("WAYLAND_DEBUG");

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
    let mut loop_count: u64 = 0;
    loop {
        loop_count += 1;
        if loop_count % 10000 == 0 {
            eprintln!("[main] loop iteration {}", loop_count);
        }
        match event_queue.blocking_dispatch(&mut state) {
            Ok(n) => {
                eprintln!(
                    "[main] blocking_dispatch returned Ok({}), about to flush",
                    n
                );
                // Explicitly flush after every dispatch cycle.
                // blocking_dispatch only flushes when dispatched==0 (before blocking read),
                // so if events were dispatched, pending requests like manage_finish
                // and render_finish stay in the buffer until the next cycle.
                // Flushing here ensures River receives our responses promptly.
                if let Err(e) = event_queue.flush() {
                    log_death(&format!("flush error after dispatch: {:?}", e));
                    let _ = std::fs::copy("/tmp/clearwm.log", "/tmp/clearwm-prev.log");
                    if !state.wm.exit_requested {
                        restart::wm_restart();
                    }
                    break;
                }
                eprintln!("[main] flush ok, looping");
            }
            Err(e) => {
                log_death(&format!(
                    "wayland dispatch error: {:?}\n  exit_requested={} wm.exit_requested={}",
                    e, state.exit_requested, state.wm.exit_requested
                ));
                // Save log before restart overwrites it
                let _ = std::fs::copy("/tmp/clearwm.log", "/tmp/clearwm-prev.log");
                if !state.wm.exit_requested {
                    restart::wm_restart();
                }
                break;
            }
        }
        if state.exit_requested || state.wm.exit_requested {
            log_death(&format!(
                "main loop exit: exit_requested={} wm.exit_requested={}",
                state.exit_requested, state.wm.exit_requested
            ));
            break;
        }
    }
    let _ = fs::remove_file(SOCKET_PATH);
    if !state.wm.exit_requested {
        // Save log before restart overwrites it
        let _ = std::fs::copy("/tmp/clearwm.log", "/tmp/clearwm-prev.log");
        restart::wm_restart();
    }
    log_death("main loop exited cleanly");
}

extern "C" fn sigchld_handler(_sig: nix::libc::c_int) {
    use nix::sys::wait::WaitStatus;
    while let Ok(status) = nix::sys::wait::waitpid(
        nix::unistd::Pid::from_raw(-1),
        Some(nix::sys::wait::WaitPidFlag::WNOHANG),
    ) {
        match status {
            WaitStatus::StillAlive => break, // no more children to reap
            _ => continue,                   // reaped a child, check for more
        }
    }
}

extern "C" fn dump_backtrace(_sig: nix::libc::c_int) {
    use std::io::Write;
    let bt = std::backtrace::Backtrace::capture();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("/tmp/clearwm-bt.txt")
    {
        let _ = writeln!(f, "SIGUSR2 backtrace:\n{:?}", bt);
        let _ = f.sync_all();
    }
}
