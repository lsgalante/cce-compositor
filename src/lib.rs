// cce — Unified compositor server and window manager client

// ==========================================
// Compositor Server Modules
// ==========================================
#[path = "server/ffi.rs"]
pub mod ffi;
#[path = "server/server.rs"]
pub mod server;
#[path = "server/process.rs"]
pub mod process;
#[path = "server/util.rs"]
pub mod util;
pub use cce_window_manager::slotmap;
#[path = "server/window_manager.rs"]
pub mod window_manager;
#[path = "server/xkb_bindings.rs"]
pub mod xkb_bindings;
#[path = "server/layer_shell.rs"]
pub mod layer_shell;
#[path = "server/scene.rs"]
pub mod scene;
// The window-management policy layer lives in the sibling crate
// `cce-window-manager` (pure Rust, no FFI). The aliases keep the historical
// `crate::policy::…` / `crate::tiling` / `crate::slotmap` paths working.
pub use cce_window_manager as policy;
pub use cce_window_manager::tiling;
#[path = "server/config.rs"]
pub mod config;
#[path = "server/ipc_server.rs"]
pub mod ipc_server;
#[path = "server/status_server.rs"]
pub mod status_server;
#[path = "server/scene_node_data.rs"]
pub mod scene_node_data;
#[path = "server/output.rs"]
pub mod output;
#[path = "server/output_manager.rs"]
pub mod output_manager;
#[path = "server/input_manager.rs"]
pub mod input_manager;
#[path = "server/libinput_config.rs"]
pub mod libinput_config;
#[path = "server/libinput_device.rs"]
pub mod libinput_device;
#[path = "server/libinput_accel_config.rs"]
pub mod libinput_accel_config;
#[path = "server/xkb_keyboard.rs"]
pub mod xkb_keyboard;
#[path = "server/xkb_config.rs"]
pub mod xkb_config;
#[path = "server/idle_inhibit_manager.rs"]
pub mod idle_inhibit_manager;
#[path = "server/lock_manager.rs"]
pub mod lock_manager;
#[path = "server/input_device.rs"]
pub mod input_device;
#[path = "server/pointer_constraint.rs"]
pub mod pointer_constraint;
#[path = "server/keyboard.rs"]
pub mod keyboard;
#[path = "server/cursor.rs"]
pub mod cursor;
#[path = "server/seat.rs"]
pub mod seat;
#[path = "server/tablet.rs"]
pub mod tablet;
#[path = "server/tablet_tool.rs"]
pub mod tablet_tool;
#[path = "server/window.rs"]
pub mod window;
#[path = "server/xdg_toplevel.rs"]
pub mod xdg_toplevel;
#[path = "server/xdg_popup.rs"]
pub mod xdg_popup;
#[path = "server/shell_surface.rs"]
pub mod shell_surface;
#[path = "server/wm_node.rs"]
pub mod wm_node;
#[path = "server/xwayland_window.rs"]
pub mod xwayland_window;
#[path = "server/xwayland_override_redirect.rs"]
pub mod xwayland_override_redirect;
#[path = "server/text_input.rs"]
pub mod text_input;
#[path = "server/input_relay.rs"]
pub mod input_relay;
#[path = "server/input_popup.rs"]
pub mod input_popup;
#[path = "server/drag_icon.rs"]
pub mod drag_icon;
#[path = "server/pointer_binding.rs"]
pub mod pointer_binding;
#[path = "server/keyboard_group.rs"]
pub mod keyboard_group;
#[path = "server/inspector.rs"]
pub mod inspector;
#[path = "server/cce_window_management.rs"]
pub mod cce_window_management;

#[path = "server/run_server.rs"]
pub mod run_server;
pub use run_server::run_server;


// ==========================================
// IPC Client Modules
// ==========================================
#[path = "cce_ctl.rs"]
pub mod cce_ctl;
pub use cce_ctl::run_cce_ctl;

#[path = "migrate_input.rs"]
pub mod migrate_input;
