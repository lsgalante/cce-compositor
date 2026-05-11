// clearwm — Wayland window manager for river

use clearwm::config::parse_config;
use clearwm::ipc::handle_ipc_command;
use clearwm::restart;
use clearwm::status::update_status_files;
use clearwm::wayland::wayland_init;
use std::env;
use std::fs;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};

const SOCKET_PATH: &str = "/tmp/clearwm.sock";

fn main() {
    eprintln!("clearwm starting...");

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

    // Create IPC socket (non-blocking for polling)
    let ipc_listener = create_ipc_socket();
    if let Some(ref listener) = ipc_listener {
        listener.set_nonblocking(true).ok();
    }

    // Connect to Wayland display and get initial state.
    let (conn, mut event_queue, mut state) = match wayland_init() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fatal: {}", e);
            std::process::exit(1);
        }
    };

    // Dispatch any events buffered during init
    let _ = event_queue.dispatch_pending(&mut state);

    // Flush and trigger manage cycle
    let _ = conn.flush();
    if let Some(ref wm) = state.window_manager {
        wm.manage_dirty();
    }

    // Check if this is a restart
    let cold_start = if env::var("CLEARWM_RESTARTING").as_deref() == Ok("1") {
        env::remove_var("CLEARWM_RESTARTING");
        false
    } else {
        true
    };

    let mut need_config_load = true;
    env::remove_var("WAYLAND_DEBUG");

    // Main event loop.
    // Uses blocking_dispatch() which is the Rust equivalent of the C version's
    // wl_display_dispatch() — it blocks until events are available, then reads
    // and dispatches them. This is the simplest and most reliable pattern.

    eprintln!("[DEBUG] entering main loop");

    loop {
        // Flush outgoing Wayland requests
        if let Err(e) = conn.flush() {
            eprintln!("wayland flush error: {:?}", e);
            break;
        }

        eprintln!("[DEBUG] calling blocking_dispatch...");
        match event_queue.blocking_dispatch(&mut state) {
            Ok(n) => {
                eprintln!("[DEBUG] blocking_dispatch returned Ok({})", n);
            }
            Err(e) => {
                eprintln!("wayland dispatch error: {:?}", e);
                if !state.wm.exit_requested {
                    restart::wm_restart();
                }
                break;
            }
        }
        update_status_files(&state.wm);

        // Handle IPC connections (non-blocking)
        if let Some(ref listener) = ipc_listener {
            while let Ok((stream, _)) = listener.accept() {
                handle_ipc_connection(stream, &mut state.wm);
            }
            update_status_files(&state.wm);
        }

        // Deferred config loading — must happen AFTER we've handled at least
        // one render_start/render_finish cycle. But we also need to keep
        // handling render cycles DURING config loading, because River's
        // 3-second timer expects continuous responsiveness.
        if need_config_load {
            eprintln!("[DEBUG] loading config...");
            if let Ok(home) = env::var("HOME") {
                let config_path = format!("{}/.config/clearwm/config.toml", home);
                if fs::metadata(&config_path).is_ok() {
                    parse_config(&config_path, cold_start, &mut state.wm);
                }
            }

            // After config loading, we MUST flush and dispatch before
            // continuing — River may have sent render_start while we
            // were busy. Do a non-blocking flush+read+dispatch cycle.
            let _ = conn.flush();
            if let Some(guard) = event_queue.prepare_read() {
                // Non-blocking read using libc::recv with MSG_DONTWAIT
                let fd = guard.connection_fd().as_raw_fd();
                let mut buf = [0u8; 4096];
                let _ = unsafe {
                    libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), libc::MSG_DONTWAIT)
                };
                let _ = guard.read();
            }
            let pending = event_queue.dispatch_pending(&mut state).unwrap_or(0);
            if pending > 0 {
                eprintln!("[DEBUG] dispatched {} events after config", pending);
                update_status_files(&state.wm);
            }

            // Trigger manage cycle for bindings
            if state.wm.config_done {
                if let Some(ref wm) = state.window_manager {
                    wm.manage_dirty();
                }
                let _ = conn.flush();
            }

            need_config_load = false;
            eprintln!("[DEBUG] config loaded, continuing loop");
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

fn create_ipc_socket() -> Option<UnixListener> {
    let _ = fs::remove_file(SOCKET_PATH);
    match UnixListener::bind(SOCKET_PATH) {
        Ok(listener) => {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(SOCKET_PATH, fs::Permissions::from_mode(0o600));
            Some(listener)
        }
        Err(e) => {
            eprintln!("failed to create IPC socket: {}", e);
            let _ = fs::remove_file(SOCKET_PATH);
            UnixListener::bind(SOCKET_PATH).ok()
        }
    }
}

fn handle_ipc_connection(mut stream: UnixStream, state: &mut clearwm::types::WindowManager) {
    let mut buf = [0u8; 4096];
    match stream.read(&mut buf) {
        Ok(n) if n > 0 => {
            let cmd = String::from_utf8_lossy(&buf[..n]);
            handle_ipc_command(cmd.trim(), state);
            state.needs_render = true;
        }
        _ => {}
    }
}

extern "C" fn sigchld_handler(_sig: nix::libc::c_int) {
    while nix::sys::wait::waitpid(
        nix::unistd::Pid::from_raw(-1),
        Some(nix::sys::wait::WaitPidFlag::WNOHANG),
    )
    .is_ok()
    {}
}
