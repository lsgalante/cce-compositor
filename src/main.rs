// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

mod ffi;
mod server;
mod process;
mod util;
mod slotmap;
mod window_manager;
mod xkb_bindings;
mod layer_shell;
mod scene;
mod scene_node_data;
mod output;
mod output_manager;
mod input_manager;
mod libinput_config;
mod xkb_config;
mod idle_inhibit_manager;
mod lock_manager;
mod input_device;
mod pointer_constraint;
mod keyboard;
mod cursor;
mod seat;
pub mod tablet;
pub mod tablet_tool;
pub mod window;
pub mod xdg_toplevel;
pub mod xdg_popup;
pub mod shell_surface;
pub mod wm_node;
pub mod xwayland_window;
pub mod xwayland_override_redirect;
pub mod text_input;
pub mod input_relay;
pub mod input_popup;
pub mod drag_icon;
pub mod pointer_binding;
pub mod keyboard_group;


use clap::Parser;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;


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
        format!("{}/river/init", xdg_config_home)
    } else if let Ok(home) = std::env::var("HOME") {
        format!("{}/.config/river/init", home)
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

fn main() {
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

    let mut server = server::Server::default();
    if let Err(e) = server.init(!args.no_xwayland) {
        log::error!("failed to initialize server: {}", e);
        std::process::exit(1);
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

    let started = unsafe { ffi::wlr_backend_start(server.backend) };
    if !started {
        log::error!("failed to start wlr_backend");
        server.deinit();
        std::process::exit(1);
    }

    struct ChildGuard(Option<nix::unistd::Pid>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(pid) = self.0 {
                log::info!("sending SIGTERM to child process group {}", pid);
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(-pid.as_raw()),
                    nix::sys::signal::Signal::SIGTERM,
                );
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

    let _guard = ChildGuard(child_pgid);

    log::info!("running server");
    unsafe {
        ffi::wl_display_run(server.wl_server);
    }

    log::info!("shutting down server");
    server.deinit();
}
