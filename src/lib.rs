// clearwm — Wayland window manager for river, written in Rust

pub mod protocol;
pub mod types;
pub mod config;
pub mod tiling;
pub mod ipc;
pub mod borders;
pub mod status;
pub mod status_server;
pub mod restart;
pub mod wm;
#[allow(unreachable_patterns)] // wayland event match arms use _ => {} for forward-compat
pub mod wayland;
