// clearwm — Wayland window manager for river

use clearwm::config::parse_config;
use clearwm::ipc;
use clearwm::ipc_server;
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

    // Create a pipe to wake up the main loop when IPC commands arrive
    let mut pipe_fds = [0; 2];
    unsafe {
        libc::pipe(pipe_fds.as_mut_ptr());
    }
    let pipe_read = pipe_fds[0];
    let pipe_write = pipe_fds[1];

    // Start the IPC server thread (for clearctl and clear-system-interface)
    let ipc_server = ipc_server::spawn_ipc_server(pipe_write);
    let ipc_rx = ipc_server.rx;
    let ipc_tx = ipc_server.tx;

    // Setup channel for configuration updates:
    let (config_tx, config_rx) = tokio::sync::mpsc::unbounded_channel::<(clearwm::config::InertialConfig, bool)>();
    state.wm.input_controller = Some(config_tx);

    // Spawn the input subsystem background thread:
    let pipe_write_clone = pipe_write;
    let ipc_tx_clone = ipc_tx.clone();
    std::thread::Builder::new()
        .name("clearwm-input-subsystem".into())
        .spawn(move || {
            if let Err(e) = clearwm::input::run_input_daemon(config_rx, ipc_tx_clone, pipe_write_clone) {
                eprintln!("[input-subsystem] Fatal error: {:?}", e);
            }
        })
        .expect("failed to spawn input subsystem thread");

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
            if let Err(e) = parse_config(&config_path, cold_start, &mut state.wm) {
                eprintln!("[init] failed to load config: {}", e);
            }
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

    // Main loop — using poll to block on both Wayland socket and IPC wake-up pipe.
    // All work (including spawning) happens inside Dispatch callbacks.
    let mut loop_count: u64 = 0;
    loop {
        loop_count += 1;
        if loop_count % 10000 == 0 {
            eprintln!("[main] loop iteration {}", loop_count);
        }

        // 1. Dispatch any already pending events in the queue
        let _ = event_queue.dispatch_pending(&mut state);

        // 2. Flush outgoing requests to the compositor
        if let Err(e) = event_queue.flush() {
            log_death(&format!("flush error: {:?}", e));
            let _ = std::fs::copy("/tmp/clearwm.log", "/tmp/clearwm-prev.log");
            if !state.wm.exit_requested {
                restart::wm_restart();
            }
            break;
        }

        // 3. Prepare to read Wayland events
        let read_guard = match event_queue.prepare_read() {
            Some(g) => g,
            None => {
                // If None, events are already in the queue, dispatch them immediately
                let _ = event_queue.dispatch_pending(&mut state);
                continue;
            }
        };

        // 4. Poll both the Wayland display FD and the IPC wake-up pipe
        use std::os::fd::{AsFd, AsRawFd};
        let wl_fd = event_queue.as_fd().as_raw_fd();
        let mut poll_fds = [
            nix::poll::PollFd::new(unsafe { std::os::fd::BorrowedFd::borrow_raw(wl_fd) }, nix::poll::PollFlags::POLLIN),
            nix::poll::PollFd::new(unsafe { std::os::fd::BorrowedFd::borrow_raw(pipe_read) }, nix::poll::PollFlags::POLLIN),
        ];

        match nix::poll::poll(&mut poll_fds, nix::poll::PollTimeout::NONE) {
            Ok(_) => {
                // If Wayland FD is readable, read the events
                if poll_fds[0].revents().unwrap_or(nix::poll::PollFlags::empty()).contains(nix::poll::PollFlags::POLLIN) {
                    let _ = read_guard.read();
                } else {
                    // Otherwise drop the read guard to release the lock
                    std::mem::drop(read_guard);
                }

                // If IPC pipe is readable, drain it
                if poll_fds[1].revents().unwrap_or(nix::poll::PollFlags::empty()).contains(nix::poll::PollFlags::POLLIN) {
                    let mut buf = [0u8; 128];
                    unsafe {
                        libc::read(pipe_read, buf.as_mut_ptr() as *mut libc::c_void, buf.len());
                    }
                }
            }
            Err(e) => {
                std::mem::drop(read_guard);
                if e != nix::errno::Errno::EINTR {
                    log_death(&format!("poll error: {:?}", e));
                    break;
                }
            }
        }

        // 5. Dispatch read events
        let _ = event_queue.dispatch_pending(&mut state);

        // 6. Process pending IPC commands from the socket channel
        let mut ipc_commands = false;
        loop {
            match ipc_rx.try_recv() {
                Ok(cmd) => {
                    eprintln!("[main] IPC command: {}", cmd);
                    ipc::handle_ipc_command(&cmd, &mut state.wm);
                    ipc_commands = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("[main] IPC server disconnected");
                    break;
                }
            }
        }
        // Force a render sequence if we processed IPC commands
        if ipc_commands {
            if let Some(ref wm) = state.window_manager {
                wm.manage_dirty();
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
