// clearwm — Wayland window manager for river, written in Rust

pub mod borders;
pub mod config;
pub mod ipc;
pub mod protocol;
pub mod restart;
pub mod state;
pub mod status;
pub mod status_server;
pub mod tiling;
pub mod types;
#[allow(unreachable_patterns)] // wayland event match arms use _ => {} for forward-compat
pub mod wayland;
pub mod wm;
