// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use clap::Parser;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::ffi;
use crate::server;
use crate::process;

const USAGE: &str = "\
usage: river [options]

  -h, --help           Print this help message and exit.
  --version            Print the version number and exit.
  -c <command>         Run `sh -c <command>` on startup instead of the default init executable.
  --log-level <level>  Set the log level to error, warning, info, or debug.
  --no-xwayland        Disable xwayland even if built with support.
";

#[derive(Parser, Debug)]
#[command(disable_help_flag = true)]
struct Args {
    #[arg(short = 'h', long = "help")]
    help: bool,

    #[arg(long = "version")]
    version: bool,

    #[arg(short = 'c')]
    command: Option<String>,

    #[arg(long = "log-level", default_value = "info")]
    log_level: String,

    #[arg(long = "no-xwayland")]
    no_xwayland: bool,
}

#[no_mangle]
pub unsafe extern "C" fn river_wlroots_log_callback(
    importance: ffi::wlr_log_importance,
    ptr: *const c_char,
    len: usize,
) {
    let message_slice = std::slice::from_raw_parts(ptr as *const u8, len);
    let message = String::from_utf8_lossy(message_slice);
    let message_trimmed = message.trim();

    match importance {
        ffi::wlr_log_importance_WLR_ERROR => log::error!(target: "wlroots", "{}", message_trimmed),
        ffi::wlr_log_importance_WLR_INFO => log::info!(target: "wlroots", "{}", message_trimmed),
        ffi::wlr_log_importance_WLR_DEBUG => log::debug!(target: "wlroots", "{}", message_trimmed),
        _ => {}
    }
}

fn default_init_path() -> Option<String> {
    let path = if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
        format!("{}/cce/init", xdg_config_home)
    } else if let Ok(home) = std::env::var("HOME") {
        format!("{}/.config/cce/init", home)
    } else {
        return None;
    };

    if std::path::Path::new(&path).exists() {
        Some(path)
    } else {
        None
    }
}

fn detect_classic(path: &str) {
    if let Ok(content) = std::fs::read_to_string(path) {
        if content.contains("riverctl") {
            log::error!(
                "The init file {} contains the string \"riverctl\".\n\
                 This version of river does not support riverctl.\n\
                 See https://isaacfreund.com/software/river for more information.",
                path
            );
            std::process::exit(1);
        }
    }
}

pub fn run_server() {
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(_) => {
            eprintln!("{}", USAGE);
            std::process::exit(1);
        }
    };

    if args.help {
        println!("{}", USAGE);
        std::process::exit(0);
    }

    if args.version {
        println!("0.5.0-dev -xwayland (Rust rewrite starter)");
        std::process::exit(0);
    }

    let log_level = match args.log_level.as_str() {
        "error" => log::LevelFilter::Error,
        "warning" => log::LevelFilter::Warn,
        "info" => log::LevelFilter::Info,
        "debug" => log::LevelFilter::Debug,
        _ => log::LevelFilter::Info,
    };

    env_logger::Builder::new()
        .filter(None, log_level)
        .init();

    log::info!("initializing river (Rust rewrite)");

    let importance = match log_level {
        log::LevelFilter::Debug => ffi::wlr_log_importance_WLR_DEBUG,
        log::LevelFilter::Info => ffi::wlr_log_importance_WLR_INFO,
        _ => ffi::wlr_log_importance_WLR_ERROR,
    };

    unsafe {
        ffi::river_init_wlroots_log(importance);
    }

    let startup_command = if let Some(cmd) = args.command {
        Some(cmd)
    } else {
        default_init_path()
    };

    if let Some(ref cmd) = startup_command {
        detect_classic(cmd);
    }

    let mut server = Box::new(server::Server::default());
    if let Err(e) = server.init(!args.no_xwayland) {
        log::error!("failed to initialize server: {}", e);
        std::process::exit(1);
    }

    if let Some(path) = crate::config::default_config_path() {
        log::info!("loading config from {}", path);
        if let Err(e) = crate::config::parse_config(&path, &mut server.wm) {
            log::error!("failed to parse config at {}: {}", path, e);
        }
    } else {
        log::warn!("no config file found, using defaults");
    }

    if let Some(state_path) = crate::config::default_state_path() {
        unsafe {
            server.wm.load_state(&state_path);
        }
    }

    process::setup();

    let socket_ptr = unsafe {
        ffi::wl_display_add_socket_auto(server.wl_server)
    };
    if socket_ptr.is_null() {
        log::error!("failed to add wayland socket");
        server.deinit();
        std::process::exit(1);
    }
    let socket_str = unsafe { CStr::from_ptr(socket_ptr).to_string_lossy().into_owned() };
    log::info!("running server on display socket: {}", socket_str);

    std::env::set_var("WAYLAND_DISPLAY", &socket_str);

    unsafe { server.wm.start_ipc(Some(socket_str.clone())) };

    let status_sender = crate::status_server::spawn_status_server(Some(socket_str.clone()));
    server.wm.status_sender = Some(status_sender);

    let stream_hub = crate::stream_server::spawn_stream_server(Some(socket_str.clone()));
    unsafe { server.wm.start_stream(stream_hub) };


    let started = unsafe { ffi::wlr_backend_start(server.backend) };
    if !started {
        log::error!("failed to start wlr_backend");
        server.deinit();
        std::process::exit(1);
    }

    // Suppress the "requested activation" notification burst that session
    // restore is about to trigger: every respawned window issues an
    // xdg-activation request as it maps. The grace window covers the whole
    // startup sequence (startup programs + restored windows).
    server::begin_startup_activation_grace();

    // Spawn configured startup programs
    let current_startup = server.wm.startup.clone();
    for prog in current_startup {
        unsafe {
            server.wm.spawn_startup_program(prog);
        }
    }

    unsafe {
        server.wm.spawn_restored_windows();
    }

    struct ServerGuard {
        init_pid: Option<nix::unistd::Pid>,
        wm: *mut crate::window_manager::WindowManager,
    }
    impl Drop for ServerGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file("/tmp/cce-status-interface-adjust-mode");
            if let Some(pid) = self.init_pid {
                log::info!("sending SIGTERM to child process group {}", pid);
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(-pid.as_raw()),
                    nix::sys::signal::Signal::SIGTERM,
                );
            }
            unsafe {
                if !self.wm.is_null() {
                    for (_, pid) in &(*self.wm).startup_pids {
                        log::info!("sending SIGTERM to startup program pid {}", pid);
                        let _ = nix::sys::signal::kill(
                            *pid,
                            nix::sys::signal::Signal::SIGTERM,
                        );
                    }
                }
            }
        }
    }

    let child_pgid = if let Some(ref cmd) = startup_command {
        log::info!("running init executable '{}'", cmd);
        unsafe {
            match nix::unistd::fork() {
                Ok(nix::unistd::ForkResult::Child) => {
                    process::cleanup_child();
                    std::env::set_var("WAYLAND_DISPLAY", &socket_str);

                    if !args.no_xwayland && !server.xwayland.is_null() {
                        let xwayland_cast = server.xwayland as *mut server::WlrXwayland;
                        if !(*xwayland_cast).display_name.is_null() {
                            let display_name = CStr::from_ptr((*xwayland_cast).display_name)
                                .to_string_lossy()
                                .into_owned();
                            std::env::set_var("DISPLAY", display_name);
                        }
                    }

                    let cmd_c = CString::new(cmd.clone()).unwrap();
                    let sh_c = CString::new("/bin/sh").unwrap();
                    let c_c = CString::new("-c").unwrap();
                    let args = [sh_c.as_c_str(), c_c.as_c_str(), cmd_c.as_c_str()];
                    
                    let env: Vec<CString> = std::env::vars()
                        .map(|(k, v)| CString::new(format!("{}={}", k, v)).unwrap())
                        .collect();
                    let env_ptrs: Vec<&CStr> = env.iter().map(|s| s.as_c_str()).collect();

                    eprintln!("[execve] target cmd: {}, env WAYLAND_DISPLAY: {:?}", cmd, std::env::var("WAYLAND_DISPLAY"));
                    let _ = nix::unistd::execve(&sh_c, &args, &env_ptrs);
                    std::process::exit(1);
                }
                Ok(nix::unistd::ForkResult::Parent { child }) => Some(child),
                Err(_) => {
                    log::error!("failed to fork child process");
                    None
                }
            }
        }
    } else {
        None
    };

    let _guard = ServerGuard {
        init_pid: child_pgid,
        wm: &mut server.wm as *mut crate::window_manager::WindowManager,
    };

    log::info!("running server");
    unsafe {
        ffi::wl_display_run(server.wl_server);
    }

    log::info!("shutting down server");
    unsafe {
        server.wm.save_state();
    }
    server.wm.shutting_down = true;
    std::mem::drop(_guard);
    server.deinit();
}
