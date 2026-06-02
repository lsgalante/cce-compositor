// Wayland display connection, registry, and event dispatch for ccec

use wayland_client::{
    event_created_child, protocol::wl_registry, Connection, Dispatch, EventQueue, Proxy,
    QueueHandle,
};

use crate::protocol::river_input_management::client::{
    river_input_device_v1::{self, RiverInputDeviceV1},
    river_input_manager_v1::{self, RiverInputManagerV1},
};
use crate::protocol::river_libinput_config::client::{
    river_libinput_config_v1::{self, RiverLibinputConfigV1},
    river_libinput_device_v1::{self, RiverLibinputDeviceV1},
    river_libinput_result_v1::{self, RiverLibinputResultV1},
};
use crate::protocol::river_layer_shell::client::{
    river_layer_shell_output_v1::{self, RiverLayerShellOutputV1},
    river_layer_shell_v1::{self, RiverLayerShellV1},
};
use crate::protocol::river_window_management::client::{
    river_decoration_v1::{self, RiverDecorationV1},
    river_node_v1::{self, RiverNodeV1},
    river_output_v1::{self, RiverOutputV1},
    river_pointer_binding_v1::{self, RiverPointerBindingV1},
    river_seat_v1::{self, Modifiers, RiverSeatV1},
    river_window_manager_v1::{self, RiverWindowManagerV1},
    river_window_v1::{self, RiverWindowV1},
};
use crate::protocol::river_xkb_bindings::client::{
    river_xkb_binding_v1::{self, RiverXkbBindingV1},
    river_xkb_bindings_seat_v1::{self, RiverXkbBindingsSeatV1},
    river_xkb_bindings_v1::{self, RiverXkbBindingsV1},
};
use crate::protocol::wlr_output_management::client::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

use crate::types::{Action, BindingUserData, Output, Seat, TilingMode, Window, WindowManager};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_pointer, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1::{self, WpCursorShapeDeviceV1, Shape},
    wp_cursor_shape_manager_v1::{self, WpCursorShapeManagerV1},
};

// Interface name constants (from river protocol XML)
const IFACE_WINDOW_MANAGER: &str = "river_window_manager_v1";
const IFACE_CURSOR_SHAPE_MANAGER: &str = "wp_cursor_shape_manager_v1";
const IFACE_XKB_BINDINGS: &str = "river_xkb_bindings_v1";
const IFACE_LAYER_SHELL: &str = "river_layer_shell_v1";
const IFACE_INPUT_MANAGER: &str = "river_input_manager_v1";
const IFACE_LIBINPUT_CONFIG: &str = "river_libinput_config_v1";
const IFACE_WLR_OUTPUT_MANAGER: &str = "zwlr_output_manager_v1";

const CORNER_THRESHOLD: f64 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerOpType {
    Move,
    Resize,
    ResizeLeft,
    ResizeRight,
    ResizeBottom,
    ResizeTop,
    ResizeBottomLeft,
    ResizeBottomRight,
    ResizeTopLeft,
    ResizeTopRight,
}

#[derive(Debug, Clone)]
pub struct PointerOp {
    pub window_id: u64,
    pub op_type: PointerOpType,
    pub start_x: i32,
    pub start_y: i32,
    pub start_width: i32,
    pub start_height: i32,
}

#[derive(Debug, Clone)]
pub struct PendingBorderDrag {
    pub window_id: u64,
    pub seat_id: u64,
    pub start_surface_x: f64,
    pub start_surface_y: f64,
    pub op_type: PointerOpType,
}

fn update_screen_bounds(outputs: &[crate::types::Output]) {
    let mut max_x = 1920;
    let mut max_y = 1080;
    for out in outputs {
        if !out.removed {
            let right = out.x + out.width;
            let bottom = out.y + out.height;
            if right > max_x {
                max_x = right;
            }
            if bottom > max_y {
                max_y = bottom;
            }
        }
    }
    crate::input::SCREEN_WIDTH.store(max_x, std::sync::atomic::Ordering::SeqCst);
    crate::input::SCREEN_HEIGHT.store(max_y, std::sync::atomic::Ordering::SeqCst);
}

/// Wayland proxy objects stored alongside each Window, so we can
/// call protocol methods (set_position, propose_dimensions, etc.) on it.
pub struct WindowProxy {
    pub river_window: RiverWindowV1,
    pub decoration: Option<crate::decorations::WindowDecoration>,
    pub dec_left: Option<crate::decorations::WindowDecoration>,
    pub dec_right: Option<crate::decorations::WindowDecoration>,
    pub dec_bottom: Option<crate::decorations::WindowDecoration>,
}

/// Wayland proxy objects stored alongside each Seat.
pub struct SeatProxy {
    pub river_seat: RiverSeatV1,
    pub xkb_bindings_seat: Option<RiverXkbBindingsSeatV1>,
    pub wl_seat: Option<wl_seat::WlSeat>,
    pub wl_pointer: Option<wl_pointer::WlPointer>,
    pub cursor_shape_device: Option<WpCursorShapeDeviceV1>,
    pub last_pointer_enter_serial: u32,
}

/// Wayland proxy objects stored alongside each Output.
pub struct OutputProxy {
    pub river_output: RiverOutputV1,
    pub layer_shell_output: Option<RiverLayerShellOutputV1>,
}

/// Tracked wl_output proxy and its physical properties.
pub struct WlOutputInfo {
    pub name: u32,
    pub wl_output: wl_output::WlOutput,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
}

/// The full app state combining logic state + protocol proxy storage.
pub struct AppState {
    pub wm: WindowManager,
    pub wl_outputs: Vec<WlOutputInfo>,

    // Protocol objects (None until bound via registry)
    pub registry: Option<wl_registry::WlRegistry>,
    pub window_manager: Option<RiverWindowManagerV1>,
    pub xkb_bindings: Option<RiverXkbBindingsV1>,
    pub layer_shell: Option<RiverLayerShellV1>,
    pub input_manager: Option<RiverInputManagerV1>,
    pub cursor_shape_manager: Option<WpCursorShapeManagerV1>,
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub shm: Option<wl_shm::WlShm>,

    // Whether we got all required globals
    pub has_window_manager: bool,
    pub has_xkb_bindings: bool,

    // Proxy objects indexed by window/seat/output ID
    pub window_proxies: Vec<(u64, WindowProxy)>,
    pub seat_proxies: Vec<(u64, SeatProxy)>,
    pub output_proxies: Vec<(u64, OutputProxy)>,

    // River node proxies for window positioning (created via get_node request)
    pub window_nodes: Vec<(u64, RiverNodeV1)>,

    // Active binding proxies — must be held alive for bindings to remain registered.
    // On reload, these are cleared (which destroys the old protocol objects) before
    // new bindings are created.
    pub xkb_binding_proxies: Vec<RiverXkbBindingV1>,
    pub pointer_binding_proxies: Vec<RiverPointerBindingV1>,

    // Next ID counter for new windows/outputs/seats
    pub next_id: u64,

    // Exit flag
    pub exit_requested: bool,

    // Debug: render cycle counter
    pub render_count: u32,

    // --- wlr-output-management protocol state ---
    pub output_manager: Option<ZwlrOutputManagerV1>,
    /// Latest serial from the output manager's done event.
    /// Required to create a valid configuration.
    pub output_serial: u32,
    /// Discovered output heads: (head_proxy, name, enabled, current_scale)
    /// The head proxy must be kept alive to reference it in configurations.
    pub output_heads: Vec<OutputHeadInfo>,
    /// Pending configuration (alive until succeeded/failed/cancelled)
    pub output_config: Option<ZwlrOutputConfigurationV1>,

    // --- Status socket sender for waybar ---
    pub status_sender: Option<crate::status_server::StatusSender>,

    // --- Libinput config protocol state ---
    pub libinput_config: Option<RiverLibinputConfigV1>,
    /// Tracked libinput devices with their tap state
    pub libinput_devices: Vec<LibinputDeviceInfo>,
    /// The surface the pointer is currently hovering over
    pub pointer_hovered_surface: Option<wl_surface::WlSurface>,
    pub active_pointer_op: Option<PointerOp>,
    pub pointer_op_release_pending: bool,
    pub pending_border_drag: Option<PendingBorderDrag>,
    pub pending_pointer_op_type: Option<PointerOpType>,
    pub last_pointer_surface_x: f64,
    pub last_pointer_surface_y: f64,
    pub input_device_names: std::collections::HashMap<u32, String>,
    pub border_font: Option<fontdue::Font>,
    pub border_font_path: Option<String>,
}

/// Info tracked for each libinput device discovered via river_libinput_config_v1
pub struct LibinputDeviceInfo {
    pub device: RiverLibinputDeviceV1,
    /// Device name (from the river_input_device_v1 name event)
    pub name: String,
    pub input_device: Option<RiverInputDeviceV1>,
    pub name_received: bool,
    /// Number of fingers supported for tap (0 = unsupported)
    pub tap_finger_count: i32,
    /// Whether we've received enough events to apply tap config
    pub tap_info_received: bool,
    pub accel_profiles_support: Option<u32>,
    pub natural_scroll_supported: Option<bool>,
    pub dwt_supported: Option<bool>,
    pub dwtp_supported: Option<bool>,
    pub config_applied: bool,
}

/// Tracked info for a wlr-output-management head.
pub struct OutputHeadInfo {
    pub proxy: ZwlrOutputHeadV1,
    pub name: String,
    pub enabled: bool,
    pub scale: f64,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            wm: WindowManager::new(),
            wl_outputs: Vec::new(),
            registry: None,
            window_manager: None,
            xkb_bindings: None,
            layer_shell: None,
            input_manager: None,
            cursor_shape_manager: None,
            compositor: None,
            shm: None,
            has_window_manager: false,
            has_xkb_bindings: false,
            window_proxies: Vec::new(),
            seat_proxies: Vec::new(),
            output_proxies: Vec::new(),
            window_nodes: Vec::new(),
            xkb_binding_proxies: Vec::new(),
            pointer_binding_proxies: Vec::new(),
            next_id: 1,
            exit_requested: false,
            render_count: 0,
            output_manager: None,
            output_serial: 0,
            output_heads: Vec::new(),
            output_config: None,
            status_sender: None,
            libinput_config: None,
            libinput_devices: Vec::new(),
            pointer_hovered_surface: None,
            active_pointer_op: None,
            pointer_op_release_pending: false,
            pending_border_drag: None,
            pending_pointer_op_type: None,
            last_pointer_surface_x: 0.0,
            last_pointer_surface_y: 0.0,
            input_device_names: std::collections::HashMap::new(),
            border_font: None,
            border_font_path: None,
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn get_window_proxy(&self, id: u64) -> Option<&WindowProxy> {
        self.window_proxies
            .iter()
            .find(|(wid, _)| *wid == id)
            .map(|(_, p)| p)
    }

    pub fn get_seat_proxy(&self, id: u64) -> Option<&SeatProxy> {
        self.seat_proxies
            .iter()
            .find(|(sid, _)| *sid == id)
            .map(|(_, p)| p)
    }

    pub fn get_output_proxy(&self, id: u64) -> Option<&OutputProxy> {
        self.output_proxies
            .iter()
            .find(|(oid, _)| *oid == id)
            .map(|(_, p)| p)
    }

    /// Find the internal window ID for a given RiverWindowV1 proxy.
    fn window_id_for_proxy(&self, proxy: &RiverWindowV1) -> Option<u64> {
        let pid = proxy.id().protocol_id();
        self.window_proxies
            .iter()
            .find(|(_, wp)| wp.river_window.id().protocol_id() == pid)
            .map(|(id, _)| *id)
    }

    /// Find the internal seat ID for a given RiverSeatV1 proxy.
    fn seat_id_for_proxy(&self, proxy: &RiverSeatV1) -> Option<u64> {
        let pid = proxy.id().protocol_id();
        self.seat_proxies
            .iter()
            .find(|(_, sp)| sp.river_seat.id().protocol_id() == pid)
            .map(|(id, _)| *id)
    }

    fn update_cursor_shape_for_surface(state: &mut Self, pointer: &wl_pointer::WlPointer) {
        let seat_info = state.seat_proxies.iter().find_map(|(_, sp)| {
            if sp.wl_pointer.as_ref() == Some(pointer) {
                Some((sp.cursor_shape_device.clone(), sp.last_pointer_enter_serial))
            } else {
                None
            }
        });

        let (device, serial) = match seat_info {
            Some((Some(dev), ser)) => (dev, ser),
            _ => return,
        };

        let mut shape = Shape::Default;

        if let Some(ref current_surface) = state.pointer_hovered_surface {
            let matched_window = state.window_proxies.iter().find_map(|(id, proxy)| {
                if let Some(dec) = &proxy.decoration {
                    if &dec.surface == current_surface {
                        return Some((*id, "top"));
                    }
                }
                if let Some(dec) = &proxy.dec_left {
                    if &dec.surface == current_surface {
                        return Some((*id, "left"));
                    }
                }
                if let Some(dec) = &proxy.dec_right {
                    if &dec.surface == current_surface {
                        return Some((*id, "right"));
                    }
                }
                if let Some(dec) = &proxy.dec_bottom {
                    if &dec.surface == current_surface {
                        return Some((*id, "bottom"));
                    }
                }
                None
            });

            if let Some((wid, surface_type)) = matched_window {
                if let Some(w) = state.wm.windows.iter().find(|win| win.id == wid) {
                    let border_w = if state.wm.expose_active && w.tiling_mode != crate::types::TilingMode::Popup {
                        state.wm.layout.grid_border_width
                    } else {
                        match w.tiling_mode {
                            crate::types::TilingMode::Cascade => state.wm.layout.cascade_border_width,
                            crate::types::TilingMode::Fullscreen => state.wm.layout.fullscreen_border_width,
                            crate::types::TilingMode::Grid => state.wm.layout.grid_border_width,
                            crate::types::TilingMode::Floating => state.wm.layout.floating_border_width,
                            crate::types::TilingMode::Popup => 0,
                        }
                    };
                    let grab_w = border_w.max(10);

                    let logical_height = border_w.max(16);
                    shape = match surface_type {
                        "top" => {
                            let total_width = w.width + 2 * border_w;
                            let mid_y = (logical_height as f64) / 2.0;
                            if state.last_pointer_surface_y < mid_y {
                                if state.last_pointer_surface_x < CORNER_THRESHOLD {
                                    Shape::NwseResize
                                } else if state.last_pointer_surface_x > (total_width as f64 - CORNER_THRESHOLD) {
                                    Shape::NeswResize
                                } else {
                                    Shape::NsResize
                                }
                            } else {
                                Shape::Default
                            }
                        }
                        "left" => {
                            let mid_x = (grab_w as f64) / 2.0;
                            if state.last_pointer_surface_x < mid_x {
                                if state.last_pointer_surface_y < CORNER_THRESHOLD {
                                    Shape::NwseResize
                                } else if state.last_pointer_surface_y > (w.height as f64 - CORNER_THRESHOLD) {
                                    Shape::NeswResize
                                } else {
                                    Shape::EwResize
                                }
                            } else {
                                Shape::Default
                            }
                        }
                        "right" => {
                            let mid_x = (grab_w as f64) / 2.0;
                            if state.last_pointer_surface_x >= mid_x {
                                if state.last_pointer_surface_y < CORNER_THRESHOLD {
                                    Shape::NeswResize
                                } else if state.last_pointer_surface_y > (w.height as f64 - CORNER_THRESHOLD) {
                                    Shape::NwseResize
                                } else {
                                    Shape::EwResize
                                }
                            } else {
                                Shape::Default
                            }
                        }
                        "bottom" => {
                            let total_width = w.width + 2 * grab_w;
                            let mid_y = (grab_w as f64) / 2.0;
                            if state.last_pointer_surface_y >= mid_y {
                                if state.last_pointer_surface_x < CORNER_THRESHOLD {
                                    Shape::NeswResize
                                } else if state.last_pointer_surface_x > (total_width as f64 - CORNER_THRESHOLD) {
                                    Shape::NwseResize
                                } else {
                                    Shape::NsResize
                                }
                            } else {
                                Shape::Default
                            }
                        }
                        _ => Shape::Default,
                    };
                }
            }
        }

        device.set_shape(serial, shape);
    }
}

// --- Dispatch implementations ---

/// User data for the registry
pub struct RegistryData;

impl Dispatch<wl_registry::WlRegistry, RegistryData> for AppState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &RegistryData,
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                if interface == IFACE_WINDOW_MANAGER {
                    if version >= 4 {
                        eprintln!("registry: binding {} v{}", IFACE_WINDOW_MANAGER, version);
                        // Use the Proxy trait's bind method via the interface
                        let wm: RiverWindowManagerV1 =
                            registry.bind::<RiverWindowManagerV1, _, _>(name, 4, qhandle, ());
                        state.window_manager = Some(wm);
                        state.has_window_manager = true;
                    } else {
                        eprintln!(
                            "warning: {} version {} < 4, skipping",
                            IFACE_WINDOW_MANAGER, version
                        );
                    }
                } else if interface == IFACE_XKB_BINDINGS {
                    let bind_ver = std::cmp::min(version, 2);
                    eprintln!("registry: binding {} v{}", IFACE_XKB_BINDINGS, bind_ver);
                    let xb: RiverXkbBindingsV1 =
                        registry.bind::<RiverXkbBindingsV1, _, _>(name, bind_ver, qhandle, ());
                    state.xkb_bindings = Some(xb);
                    state.has_xkb_bindings = true;
                } else if interface == IFACE_LAYER_SHELL {
                    eprintln!("registry: binding {}", IFACE_LAYER_SHELL);
                    let ls: RiverLayerShellV1 =
                        registry.bind::<RiverLayerShellV1, _, _>(name, 1, qhandle, ());
                    state.layer_shell = Some(ls);
                } else if interface == IFACE_INPUT_MANAGER {
                    eprintln!("registry: binding {}", IFACE_INPUT_MANAGER);
                    let im: RiverInputManagerV1 =
                        registry.bind::<RiverInputManagerV1, _, _>(name, 1, qhandle, ());
                    state.input_manager = Some(im);
                } else if interface == IFACE_LIBINPUT_CONFIG {
                    eprintln!("registry: binding {}", IFACE_LIBINPUT_CONFIG);
                    let lc: RiverLibinputConfigV1 =
                        registry.bind::<RiverLibinputConfigV1, _, _>(name, 1, qhandle, ());
                    state.libinput_config = Some(lc);
                } else if interface == IFACE_WLR_OUTPUT_MANAGER {
                    eprintln!("registry: binding {}", IFACE_WLR_OUTPUT_MANAGER);
                    let om: ZwlrOutputManagerV1 =
                        registry.bind::<ZwlrOutputManagerV1, _, _>(name, 4, qhandle, ());
                    state.output_manager = Some(om);
                } else if interface == IFACE_CURSOR_SHAPE_MANAGER {
                    eprintln!("registry: binding {}", IFACE_CURSOR_SHAPE_MANAGER);
                    let csm: WpCursorShapeManagerV1 =
                        registry.bind::<WpCursorShapeManagerV1, _, _>(name, 1, qhandle, ());
                    state.cursor_shape_manager = Some(csm);
                } else if interface == "wl_compositor" {
                    eprintln!("registry: binding wl_compositor name={}", name);
                    let comp: wl_compositor::WlCompositor =
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 4, qhandle, ());
                    state.compositor = Some(comp);
                } else if interface == "wl_shm" {
                    eprintln!("registry: binding wl_shm name={}", name);
                    let shm: wl_shm::WlShm =
                        registry.bind::<wl_shm::WlShm, _, _>(name, 1, qhandle, ());
                    state.shm = Some(shm);
                } else if interface == "wl_output" {
                    eprintln!("registry: binding wl_output name={}", name);
                    let wl_out: wl_output::WlOutput =
                        registry.bind::<wl_output::WlOutput, _, _>(name, 4, qhandle, ());
                    state.wl_outputs.push(WlOutputInfo {
                        name,
                        wl_output: wl_out,
                        width: 0,
                        height: 0,
                        x: 0,
                        y: 0,
                    });
                }
            }
            wl_registry::Event::GlobalRemove { name: _ } => {}
            _ => {}
        }
    }
}

// --- RiverWindowManagerV1 events ---

impl Dispatch<RiverWindowManagerV1, ()> for AppState {
    event_created_child!(AppState, RiverWindowManagerV1, [
        river_window_manager_v1::EVT_WINDOW_OPCODE => (RiverWindowV1, ()),
        river_window_manager_v1::EVT_OUTPUT_OPCODE => (RiverOutputV1, ()),
        river_window_manager_v1::EVT_SEAT_OPCODE => (RiverSeatV1, ()),
    ]);

    fn event(
        state: &mut Self,
        wm_proxy: &RiverWindowManagerV1,
        event: river_window_manager_v1::Event,
        _data: &(),
        conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        match event {
            river_window_manager_v1::Event::Unavailable => {
                eprintln!("error: another window manager is already running");
                state.exit_requested = true;
                state.wm.exit_requested = true;
            }

            river_window_manager_v1::Event::Finished => {
                if state.wm.exit_requested {
                    eprintln!("river sent finished, exiting");
                    state.exit_requested = true;
                } else {
                    eprintln!("river sent finished unexpectedly, restarting");
                    // Write to death log before fork+exit loses all traces
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(crate::paths::get_death_log_path())
                    {
                        use std::io::Write;
                        let _ = writeln!(
                            f,
                            "river sent Finished event — protocol error likely, in_manage={}, render_count={}",
                            state.wm.in_manage_sequence,
                            state.render_count
                        );
                        let _ = f.sync_all();
                    }
                    crate::restart::wm_restart();
                }
            }

            river_window_manager_v1::Event::ManageStart => {
                let ms_start = std::time::Instant::now();
                state.wm.in_manage_sequence = true;
                state.wm.focused_tags = 0;
                state.wm.needs_render = true;

                // Remove closed windows and their proxy objects.
                // Collect IDs first, then clean up proxies, then remove from windows vector.
                // This matches tinyrwm's remove_windows() pattern.
                let closed_ids: Vec<(u64, Option<String>)> = state
                    .wm
                    .windows
                    .iter()
                    .filter(|w| w.closed)
                    .map(|w| (w.id, w.app_id.clone()))
                    .collect();
                // Destroy proxy objects for closed windows.
                // Also clean up seat state (focused_window_id, hovered_window_id,
                // interacted_window_id) that references closed windows, matching
                // tinyrwm's remove_windows() pattern which cleans up seat ops
                // referencing closed window proxies.
                for (closed_id, closed_app_id) in &closed_ids {
                    state.window_proxies.retain_mut(|(id, wp)| {
                        if id == closed_id {
                            if let Some(dec) = wp.decoration.take() {
                                dec.decoration.destroy();
                                dec.surface.destroy();
                            }
                            if let Some(dec) = wp.dec_left.take() {
                                dec.decoration.destroy();
                                dec.surface.destroy();
                            }
                            if let Some(dec) = wp.dec_right.take() {
                                dec.decoration.destroy();
                                dec.surface.destroy();
                            }
                            if let Some(dec) = wp.dec_bottom.take() {
                                dec.decoration.destroy();
                                dec.surface.destroy();
                            }
                            wp.river_window.destroy();
                            false
                        } else {
                            true
                        }
                    });
                    state.window_nodes.retain(|(id, node)| {
                        if id == closed_id {
                            node.destroy();
                            false
                        } else {
                            true
                        }
                    });
                    // Clear seat references to this closed window.
                    // If the focused window closed, reassign focus to another
                    // visible window (matching tinyrwm's focus_top pattern).
                    let active_tags = state.wm.active_tags;
                    for seat in &mut state.wm.seats {
                        if seat.focused_window_id == Some(*closed_id) {
                            // Pick the last visible window (cascade front)
                            let visible_ids: Vec<u64> = state
                                .wm
                                .windows
                                .iter()
                                .filter(|w| {
                                    !w.closed && (w.tags & active_tags) != 0 && w.id != *closed_id && w.app_id.as_deref() != Some("clear-status-interface")
                                })
                                .map(|w| w.id)
                                .collect();
                            seat.focused_window_id = visible_ids.last().copied();
                            if seat.focused_window_id.is_some() {
                                state.wm.needs_focus = true;
                                state.wm.needs_status_update = true;
                            }
                            eprintln!(
                                "[manage] focused window {} (app_id={:?}) closed, reassigned to {:?}",
                                closed_id, closed_app_id, seat.focused_window_id
                            );
                        }
                        if seat.hovered_window_id == Some(*closed_id) {
                            seat.hovered_window_id = None;
                        }
                        if seat.interacted_window_id == Some(*closed_id) {
                            seat.interacted_window_id = None;
                        }
                    }
                }
                state.wm.windows.retain(|w| !w.closed);

                // Remove removed outputs — destroy protocol proxies first
                // (matching tinyrwm's remove_outputs pattern).
                let removed_output_ids: Vec<u64> = state
                    .wm
                    .outputs
                    .iter()
                    .filter(|o| o.removed)
                    .map(|o| o.id)
                    .collect();
                for removed_id in &removed_output_ids {
                    state.output_proxies.retain(|(id, op)| {
                        if id == removed_id {
                            op.river_output.destroy();
                            false
                        } else {
                            true
                        }
                    });
                }
                state.wm.outputs.retain(|o| !o.removed);
                update_screen_bounds(&state.wm.outputs);

                // Remove removed seats — destroy protocol proxies first
                // (matching tinyrwm's remove_seats pattern which destroys
                // seat proxy and all binding proxies).
                let removed_seat_ids: Vec<u64> = state
                    .wm
                    .seats
                    .iter()
                    .filter(|s| s.removed)
                    .map(|s| s.id)
                    .collect();
                for removed_id in &removed_seat_ids {
                    state.seat_proxies.retain(|(id, sp)| {
                        if id == removed_id {
                            sp.river_seat.destroy();
                            false
                        } else {
                            true
                        }
                    });
                }
                state.wm.seats.retain(|s| !s.removed);

                // Set default layer shell on first output
                if let Some(first_output) = state.wm.outputs.first() {
                    if let Some(op) = state.get_output_proxy(first_output.id) {
                        if let Some(ref lso) = op.layer_shell_output {
                            lso.set_default();
                        }
                    }
                }

                // Enforce single_instance mode rules
                enforce_single_instance(&mut state.wm);

                // Apply pending bindings to seats
                apply_pending_bindings(state, qhandle);

                // Manage seats (set focused tags)
                for seat in &state.wm.seats {
                    if let Some(focused_id) = seat.focused_window_id {
                        if let Some(fw) = state.wm.get_window(focused_id) {
                            state.wm.focused_tags |= fw.tags;
                        }
                    }
                }

                // Check xprop results for XWayland parent detection.
                // River doesn't send the Parent event for XWayland windows,
                // so we detect WM_TRANSIENT_FOR / _NET_WM_WINDOW_TYPE_DIALOG|UTILITY
                // via an async xprop check spawned in the UnreliablePid handler.
                // This must run before assign_window_modes so has_parent is set
                // before mode assignment decides Floating vs tiled.
                {
                    let mut xprop_parent_changed = false;
                    let max_attempts: u8 = 10;
                    for window in &mut state.wm.windows {
                        if !window.needs_xprop_check || window.has_parent || window.closed {
                            continue;
                        }
                        window.xprop_check_attempts += 1;
                        let path = crate::paths::get_xprop_path(window.id);
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let _ = std::fs::remove_file(&path);
                            // Each line: title|window_type_line
                            // Match by title against our window's title.
                            let my_title = window.title.as_deref().unwrap_or("");
                            for line in content.lines() {
                                let parts: Vec<&str> = line.splitn(2, '|').collect();
                                if parts.len() < 2 {
                                    continue;
                                }
                                let x11_title = parts[0];
                                let wtype = parts[1];
                                if x11_title != my_title {
                                    continue;
                                }
                                let is_dialog =
                                    wtype.contains("DIALOG") || wtype.contains("UTILITY");
                                // Only use the window type hint, not WM_TRANSIENT_FOR alone.
                                // Transient-for is unreliable: some apps (e.g. Houdini)
                                // set WM_TRANSIENT_FOR on their main window (transient to the
                                // splash screen). The window type hint is the reliable signal
                                // that a window is actually a dialog/utility child.
                                if is_dialog {
                                    window.has_parent = true;
                                    xprop_parent_changed = true;
                                    eprintln!(
                                        "[xprop] id={} (app_id={:?}) detected XWayland dialog/utility window",
                                        window.id, window.app_id
                                    );
                                }
                            }
                            window.needs_xprop_check = false;
                        } else if window.xprop_check_attempts >= max_attempts {
                            // Give up — xprop file never appeared (native Wayland window?)
                            window.needs_xprop_check = false;
                            eprintln!(
                                "[xprop] id={} (app_id={:?}) giving up after {} attempts",
                                window.id, window.app_id, max_attempts
                            );
                        }
                    }
                    // If any window gained has_parent, trigger a re-manage so
                    // assign_window_modes picks it up on the next cycle.
                    if xprop_parent_changed {
                        if let Some(ref wm) = state.window_manager {
                            wm.manage_dirty();
                        }
                    }
                }

                // Apply persisted state from ~/.cache/ccec_state on first ManageStart.
                // This restores window tag assignments, active_tags, tag_layouts, and
                // locked tiling modes from the previous session. Must run before
                // assign_window_modes so restored tags/modes take effect.
                //
                // We retry across multiple ManageStart cycles because window metadata
                // (identifier, app_id, title) arrives as separate events AFTER the
                // Window creation event. On the first ManageStart, these fields may
                // still be None, so matching would fail. We keep needs_state_restore
                // true until we've had at least one window with metadata (or after
                // 5 cycles, giving up gracefully).
                if state.wm.needs_state_restore {
                    if let Some(pstate) = crate::state::read_state() {
                        // Check if any window has metadata yet
                        let has_metadata = state.wm.windows.iter().any(|w| {
                            w.identifier.is_some() || w.app_id.is_some()
                        });
                        if has_metadata {
                            crate::state::apply_state(&mut state.wm, &pstate);
                            state.wm.needs_state_restore = false;
                            eprintln!("[state] restored state on ManageStart (windows have metadata)");
                        } else {
                            // Increment a counter; give up after 5 cycles
                            state.wm.state_restore_attempts += 1;
                            if state.wm.state_restore_attempts >= 5 {
                                // No windows with metadata yet — probably a fresh start
                                // with no existing windows. Apply global state only
                                // (active_tags, tag_layouts) and stop retrying.
                                state.wm.active_tags = pstate.active_tags;
                                for (idx, mode, has) in &pstate.tag_layouts {
                                    if *idx < crate::types::NUM_TAGS {
                                        state.wm.tag_layouts[*idx] = *mode;
                                        state.wm.has_tag_layout[*idx] = *has;
                                    }
                                }
                                state.wm.needs_state_restore = false;
                                eprintln!("[state] no windows with metadata after 5 cycles, applied global state only");
                            }
                        }
                    } else {
                        // No state file — fresh start
                        state.wm.needs_state_restore = false;
                    }
                }

                // Assign tiling modes to windows based on mode_rules / tag_layouts / global_layout.
                // Must happen before manage_windows so tiling computation uses the correct modes.
                crate::wm::assign_window_modes(&mut state.wm);

                // Window management: set position + propose dimensions.
                // These modify window management state and can ONLY be called
                // during a manage sequence (per River protocol spec).
                crate::wm::manage_windows(state, qhandle);

                // Auto-focus: new windows on the current tags get keyboard focus.
                // This runs after manage_windows so the new window is tiled and
                // has a valid position/proxy, but before focus application so
                // needs_focus will trigger the actual focus_window() call.
                {
                    let active_tags = state.wm.active_tags;
                    // Find the last new window on active tags (cascade-front).
                    // Multiple new windows can appear in one manage cycle (e.g.
                    // spawning several apps at once); focusing the last one is
                    // consistent with FocusNext and WindowInteraction which also
                    // pick visible_ids.last().
                    let new_focused_id = state
                        .wm
                        .windows
                        .iter()
                        .filter(|w| w.is_new && !w.closed && (w.tags & active_tags) != 0 && w.app_id.as_deref() != Some("clear-status-interface") && w.app_id.as_deref() != Some("clear-notification-daemon"))
                        .map(|w| w.id)
                        .last();
                    if let Some(new_id) = new_focused_id {
                        for seat in &mut state.wm.seats {
                            if seat.removed {
                                continue;
                            }
                            seat.focused_window_id = Some(new_id);
                        }
                        // Move to cascade-front position (last in windows vec)
                        state.wm.move_window_to_end(new_id);
                        state.wm.needs_focus = true;
                        state.wm.needs_status_update = true;
                        let app_id = state.wm.get_window(new_id).and_then(|w| w.app_id.clone());
                        eprintln!(
                            "[focus] auto-focusing new window id={} (app_id={:?})",
                            new_id, app_id
                        );
                    }
                    // Clear is_new on all windows (only matters once)
                    for window in &mut state.wm.windows {
                        window.is_new = false;
                    }
                }

                // Focus management: focus the focused window on each seat.
                // focus_window() modifies window management state.
                // Only call when needs_focus is set to avoid stealing focus from
                // layer-shell clients (like fuzzel) that trigger manage sequences.
                if state.wm.needs_focus {
                    for seat in &state.wm.seats {
                        if seat.removed {
                            continue;
                        }
                        if let Some(focused_id) = seat.focused_window_id {
                            if let Some((_id, sp)) =
                                state.seat_proxies.iter().find(|(id, _)| *id == seat.id)
                            {
                                if let Some(wp) = state.get_window_proxy(focused_id) {
                                    let app_id = state
                                        .wm
                                        .get_window(focused_id)
                                        .and_then(|w| w.app_id.clone());
                                    eprintln!(
                                        "[focus] calling focus_window for id={} (app_id={:?})",
                                        focused_id, app_id
                                    );
                                    sp.river_seat.focus_window(&wp.river_window);
                                }
                            }
                        } else {
                            // No focused window — clear focus
                            if let Some((_id, sp)) =
                                state.seat_proxies.iter().find(|(id, _)| *id == seat.id)
                            {
                                eprintln!("[focus] calling clear_focus");
                                sp.river_seat.clear_focus();
                            }
                        }
                    }
                    state.wm.needs_focus = false;
                }

                if state.pointer_op_release_pending {
                    state.pointer_op_release_pending = false;
                    if let Some(ref op) = state.active_pointer_op {
                        if op.op_type != PointerOpType::Move {
                            if let Some(wp) = state.get_window_proxy(op.window_id) {
                                wp.river_window.inform_resize_end();
                            }
                        }
                    }
                    state.active_pointer_op = None;
                    for (_sid, sp) in &state.seat_proxies {
                        sp.river_seat.op_end();
                    }
                    for seat in &mut state.wm.seats {
                        seat.interacted_window_id = None;
                    }
                    state.wm.needs_render = true;
                }

                // Execute pending actions from key/pointer bindings (tinyrwm pattern).
                // Like tinyrwm, we defer action execution to ManageStart so all
                // state mutations happen during the manage sequence.
                // Collect pending actions first to avoid borrow checker issues
                // (execute_action borrows state mutably).
                let pending: Vec<(u64, Action, Option<String>)> = state
                    .wm
                    .seats
                    .iter_mut()
                    .filter_map(|seat| {
                        if seat.pending_action != Action::None {
                            let action = seat.pending_action;
                            let command = seat.pending_command.take();
                            seat.pending_action = Action::None;
                            Some((seat.id, action, command))
                        } else {
                            None
                        }
                    })
                    .collect();
                let has_pending = !pending.is_empty();
                for (seat_id, action, command) in pending {
                    execute_action(state, seat_id, &action, command.as_deref());
                }

                if has_pending {
                    if let Some(ref wm) = state.window_manager {
                        wm.manage_dirty();
                    }
                }

                wm_proxy.manage_finish();
                // Flush immediately so River can start the configure/render cycle
                // without waiting for our blocking_dispatch to complete.
                if let Err(e) = conn.flush() {
                    eprintln!("[manage] FATAL: flush after manage_finish failed: {:?}", e);
                }
                state.wm.in_manage_sequence = false;
                eprintln!("[manage] ManageStart done in {:?}", ms_start.elapsed());
                // NOTE: Do NOT call update_status_files() here — it calls
                // pkill with .output() which blocks the event loop.
            }

            river_window_manager_v1::Event::RenderStart => {
                state.render_count += 1;
                eprintln!(
                    "[render] RenderStart #{} needs_render={}",
                    state.render_count, state.wm.needs_render
                );

                // Show/hide windows based on tag visibility.
                // This is rendering state and must happen during a render sequence.
                let active_tags = state.wm.active_tags;
                for window in &state.wm.windows {
                    if window.closed {
                        continue;
                    }
                    let visible = (window.tags & active_tags) != 0;
                    if let Some(wp) = state.get_window_proxy(window.id) {
                        if visible {
                            wp.river_window.show();
                        } else {
                            wp.river_window.hide();
                        }
                    }
                }

                if state.wm.needs_render {
                    // Only do rendering state here: borders, place_top/place_bottom.
                    // set_position and propose_dimensions are window management state
                    // and must happen during ManageStart.
                    crate::wm::render_borders(state);
                    crate::wm::render_opacity(state);
                    crate::wm::render_circular(state);

                    // Update and render window title decorations on borders
                    crate::decorations::update_decorations(state, qhandle);

                    // Raise and stack windows according to z-axis logic:
                    // 1. clear-status-interface at the absolute bottom (score = 0)
                    // 2. Unfocused tiled/fullscreen windows (score = 1)
                    // 3. Focused tiled/fullscreen window (score = 2)
                    // 4. Unfocused floating windows (score = 3)
                    // 5. Focused floating window (score = 4)
                    let active_tags = state.wm.active_tags;
                    let focused_id = state.wm.seats.iter().find(|s| !s.removed).and_then(|s| s.focused_window_id);

                    let get_window_score = |win: &crate::types::Window| -> i32 {
                        if win.app_id.as_deref() == Some("clear-status-interface") {
                            0
                        } else if win.tiling_mode == crate::types::TilingMode::Popup {
                            5
                        } else if win.tiling_mode != crate::types::TilingMode::Floating {
                            if Some(win.id) == focused_id {
                                2
                            } else {
                                1
                            }
                        } else {
                            if Some(win.id) == focused_id {
                                4
                            } else {
                                3
                            }
                        }
                    };

                    let mut nodes_to_place: Vec<(i32, usize, u64, Option<String>, &RiverNodeV1)> = Vec::new();
                    for &(wid, ref node) in &state.window_nodes {
                        if let Some((idx, win)) = state.wm.windows.iter().enumerate().find(|(_, w)| w.id == wid) {
                            if win.closed {
                                continue;
                            }
                            let visible = (win.tags & active_tags) != 0;
                            if visible {
                                let score = get_window_score(win);
                                nodes_to_place.push((score, idx, win.id, win.app_id.clone(), node));
                            }
                        }
                    }

                    // Sort ascending by score, then by original window list index
                    nodes_to_place.sort_by_key(|&(score, idx, _, _, _)| (score, idx));

                    for &(score, _, id, ref app_id, node) in &nodes_to_place {
                        eprintln!(
                            "[render] placing node id={} (app_id={:?}) at top with score {}",
                            id, app_id, score
                        );
                        node.place_top();
                    }

                    state.wm.needs_render = false;
                }

                wm_proxy.render_finish();
                // Flush immediately so River receives render_finish without waiting
                // for blocking_dispatch to complete. River has a 3-second unresponsive
                // timeout, and if we don't flush promptly, River will kill us.
                if let Err(e) = conn.flush() {
                    eprintln!(
                        "[render] FATAL: flush after render_finish #{} failed: {:?}",
                        state.render_count, e
                    );
                }
                eprintln!("[render] render_finish #{} flushed", state.render_count);
                // Spawn startup apps inside the callback, like tinyrwm does.
                // Spawning between blocking_dispatch calls corrupts the Wayland
                // connection state because the fork inherits the socket fd.
                if !state.wm.startup_spawned {
                    let apps: Vec<String> = state.wm.pending_startup_apps.drain(..).collect();
                    for cmd in &apps {
                        eprintln!("[init] spawning startup app: {}", cmd);
                        crate::config::spawn_command_bg(cmd);
                    }
                    state.wm.startup_spawned = true;
                }

                // Write status files after all manage+render state is settled.
                // Uses the deferred flag pattern: handlers set needs_status_update,
                // we check it here inside the Dispatch callback.
                // write_status_files() does only file I/O — no fork, no pkill.
                if state.wm.needs_status_update {
                    crate::status::write_status_files(&state.wm);

                    // Persist state to ~/.cache/ccec_state for restart recovery.
                    // Safe: just file I/O, no fork, no blocking.
                    crate::state::write_state(&state.wm);

                    // Push the same data through the status socket so waybar
                    // gets updates in real-time without needing signal-based pkill.
                    if let Some(ref sender) = state.status_sender {
                        let update = crate::status_server::build_status_update(&state.wm);
                        sender.send(update);
                    }

                    state.wm.needs_status_update = false;
                }

                // Re-apply input config if it was changed via IPC
                if !state.wm.tap_config_applied && !state.libinput_devices.is_empty() {
                    crate::wayland::apply_input_config(state, qhandle);
                }

                if !state.wm.cursor_theme_applied {
                    let theme = state.wm.cursor_theme.clone().unwrap_or_else(|| "default".to_string());
                    let size = state.wm.cursor_size.unwrap_or(24);
                    for (_, sp) in &state.seat_proxies {
                        sp.river_seat.set_xcursor_theme(theme.clone(), size);
                    }
                    state.wm.cursor_theme_applied = true;
                }
            }

            // Window event: field is `id` (the new RiverWindowV1 proxy)
            river_window_manager_v1::Event::Window { id: river_window } => {
                let already_tracked = state.window_proxies.iter().any(|(_, wp)| {
                    wp.river_window.id().protocol_id() == river_window.id().protocol_id()
                });
                if already_tracked {
                    eprintln!("wm_handle_window: duplicate skipped");
                    return;
                }

                let id = state.alloc_id();
                let mut window = Window::default();
                window.id = id;
                window.is_new = true;
                window.tags = state.wm.active_tags;

                state.wm.windows.push(window);
                state
                    .window_proxies
                    .push((id, WindowProxy {
                        river_window,
                        decoration: None,
                        dec_left: None,
                        dec_right: None,
                        dec_bottom: None,
                    }));
                eprintln!("[window] new window id={} (app_id pending)", id);
            }

            // Output event: field is `id` (the new RiverOutputV1 proxy)
            river_window_manager_v1::Event::Output { id: river_output } => {
                let already_tracked = state.output_proxies.iter().any(|(_, op)| {
                    op.river_output.id().protocol_id() == river_output.id().protocol_id()
                });
                if already_tracked {
                    eprintln!("wm_handle_output: duplicate skipped");
                    return;
                }

                let id = state.alloc_id();
                let mut output = Output::default();
                output.id = id;

                let lso = if let Some(ref ls) = state.layer_shell {
                    Some(ls.get_output(&river_output, qhandle, ()))
                } else {
                    None
                };

                state.wm.outputs.push(output);
                update_screen_bounds(&state.wm.outputs);
                state.output_proxies.push((
                    id,
                    OutputProxy {
                        river_output,
                        layer_shell_output: lso,
                    },
                ));
                eprintln!("wm_handle_output: new output id={}", id);
            }

            // Seat event: field is `id` (the new RiverSeatV1 proxy)
            river_window_manager_v1::Event::Seat { id: river_seat } => {
                let already_tracked = state.seat_proxies.iter().any(|(_, sp)| {
                    sp.river_seat.id().protocol_id() == river_seat.id().protocol_id()
                });
                if already_tracked {
                    return;
                }

                let id = state.alloc_id();
                let mut seat = Seat::default();
                seat.id = id;
                seat.is_new = true;

                let cursor_theme = state.wm.cursor_theme.clone().unwrap_or_else(|| "default".to_string());
                let cursor_size = state.wm.cursor_size.unwrap_or(24);
                river_seat.set_xcursor_theme(cursor_theme, cursor_size);

                state.wm.seats.push(seat);
                state.seat_proxies.push((
                    id,
                    SeatProxy {
                        river_seat,
                        xkb_bindings_seat: None,
                        wl_seat: None,
                        wl_pointer: None,
                        cursor_shape_device: None,
                        last_pointer_enter_serial: 0,
                    },
                ));
                eprintln!("wm_handle_seat: new seat id={}", id);
            }

            river_window_manager_v1::Event::SessionLocked => {}
            river_window_manager_v1::Event::SessionUnlocked => {}

            _ => {}
        }
    }
}

// --- RiverWindowV1 events ---

impl Dispatch<RiverWindowV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowV1,
        event: river_window_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        let wid = match state.window_id_for_proxy(proxy) {
            Some(id) => id,
            None => return,
        };

        match event {
            river_window_v1::Event::Closed => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.closed = true;
                    eprintln!("window id={} (app_id={:?}) closed", wid, window.app_id);
                }
            }

            river_window_v1::Event::Dimensions { width, height } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.width = width;
                    window.height = height;
                }
            }

            river_window_v1::Event::AppId { app_id } => {
                let mut re_eval = false;
                if let Some(window) = state.wm.get_window_mut(wid) {
                    if window.app_id != app_id {
                        eprintln!(
                            "[window] id={} app_id: {:?} -> {:?}",
                            wid, window.app_id, app_id
                        );
                        window.app_id = app_id;
                        re_eval = !window.mode_locked;

                        // Check if this is a blank steam_proton helper window
                        let is_proton = window.app_id.as_deref() == Some("steam_proton");
                        let is_blank = window.title.is_none() || window.title.as_deref().map_or(true, |t| t.is_empty());
                        if is_proton && is_blank {
                            if window.tags != 0 {
                                window.tags = 0;
                                re_eval = true;
                            }
                        }
                    }
                }
                if re_eval {
                    state.wm.needs_render = true;
                    if let Some(ref wm) = state.window_manager {
                        wm.manage_dirty();
                    }
                }
            }

            river_window_v1::Event::Title { title } => {
                let mut re_eval = false;
                let mut needs_render = false;
                let active_tags = state.wm.active_tags;
                if let Some(window) = state.wm.get_window_mut(wid) {
                    if window.title != title {
                        window.title = title;
                        needs_render = true;

                        // Check if we need to update/restore tags based on title
                        let is_proton = window.app_id.as_deref() == Some("steam_proton");
                        let is_blank = window.title.is_none() || window.title.as_deref().map_or(true, |t| t.is_empty());
                        if is_proton {
                            if is_blank {
                                if window.tags != 0 {
                                    window.tags = 0;
                                    re_eval = true;
                                }
                            } else {
                                if window.tags == 0 {
                                    window.tags = active_tags;
                                    re_eval = true;
                                }
                            }
                        }
                    }
                }
                if needs_render {
                    state.wm.needs_render = true;
                }
                if re_eval {
                    if let Some(ref wm) = state.window_manager {
                        wm.manage_dirty();
                    }
                }
            }

            river_window_v1::Event::DecorationHint { hint } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    // Store the raw u32 value from the WEnum
                    window.decoration_hint = hint.into();
                }
            }

            river_window_v1::Event::PresentationHint { hint } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.presentation_hint = hint.into();
                }
            }

            river_window_v1::Event::Identifier { identifier } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.identifier = Some(identifier);
                }
            }

            river_window_v1::Event::UnreliablePid { unreliable_pid } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.pid = unreliable_pid as u32;
                    // Spawn an async xprop check for XWayland parent detection.
                    // River doesn't forward WM_TRANSIENT_FOR for XWayland windows,
                    // so we check via xdotool + xprop as a fallback.
                    // The script writes results to /tmp/ccec-xprop-{wid} which
                    // is read on the next ManageStart cycle.
                    if !window.has_parent && window.pid > 0 {
                        let pid = window.pid;
                        let id = window.id;
                        let path = crate::paths::get_xprop_path(id);
                        let cmd = format!(
                            "for xid in $(xdotool search --pid {pid} 2>/dev/null); do \
                             t=$(xdotool getwindowname $xid 2>/dev/null); \
                             wt=$(xprop -id $xid _NET_WM_WINDOW_TYPE 2>/dev/null); \
                             printf '%s|%s\\n' \"$t\" \"$wt\"; \
                             done > {path}",
                            pid = pid,
                            path = path
                        );
                        crate::config::spawn_command_bg(&cmd);
                        window.needs_xprop_check = true;
                        eprintln!(
                            "[window] id={} (app_id={:?}) spawned xprop check for pid={}",
                            wid, window.app_id, pid
                        );
                    }
                }
            }

            river_window_v1::Event::DimensionsHint {
                min_width,
                min_height,
                max_width,
                max_height,
            } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.hint_min_width = min_width;
                    window.hint_min_height = min_height;
                    window.hint_max_width = max_width;
                    window.hint_max_height = max_height;
                    eprintln!(
                        "[window] id={} (app_id={:?}) dimensions_hint: min={}x{} max={}x{}",
                        wid, window.app_id, min_width, min_height, max_width, max_height
                    );
                }
            }

            river_window_v1::Event::Parent { parent } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    let had_parent = window.has_parent;
                    match &parent {
                        Some(parent_proxy) => {
                            window.has_parent = true;
                            // Look up our internal ID for the parent proxy
                            let parent_id = state
                                .window_proxies
                                .iter()
                                .find(|(_, wp)| {
                                    wp.river_window.id().protocol_id()
                                        == parent_proxy.id().protocol_id()
                                })
                                .map(|(id, _)| *id);
                            window.parent_id = parent_id;
                            eprintln!(
                                "[window] id={} (app_id={:?}) has parent (internal_id={:?})",
                                wid, window.app_id, parent_id
                            );
                        }
                        None => {
                            window.has_parent = false;
                            window.parent_id = None;
                        }
                    }
                    // Parent status changed: re-assign mode (child windows float)
                    if window.has_parent != had_parent && !window.mode_locked {
                        if let Some(ref wm) = state.window_manager {
                            wm.manage_dirty();
                        }
                    }
                }
            }

            river_window_v1::Event::FullscreenRequested { .. } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.fullscreen_requested = true;
                    eprintln!(
                        "[window] id={} (app_id={:?}) requested fullscreen",
                        wid, window.app_id
                    );
                }
            }

            river_window_v1::Event::ExitFullscreenRequested { .. } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.fullscreen_requested = false;
                    eprintln!(
                        "[window] id={} (app_id={:?}) requested exit fullscreen",
                        wid, window.app_id
                    );
                }
            }

            river_window_v1::Event::MaximizeRequested { .. } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.maximize_requested = true;
                    eprintln!(
                        "[window] id={} (app_id={:?}) requested maximize",
                        wid, window.app_id
                    );
                }
            }

            river_window_v1::Event::UnmaximizeRequested { .. } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.maximize_requested = false;
                    eprintln!(
                        "[window] id={} (app_id={:?}) requested unmaximize",
                        wid, window.app_id
                    );
                }
            }

            river_window_v1::Event::MinimizeRequested { .. } => {
                let mut minimized_id = None;
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.minimize_requested = true;
                    window.minimized = true;
                    minimized_id = Some(wid);
                    eprintln!(
                        "[window] id={} (app_id={:?}) requested minimize",
                        wid, window.app_id
                    );
                }

                if let Some(wid) = minimized_id {
                    // Shift focus to the next visible window if the minimized window was focused
                    if let Some(seat) = state.wm.seats.iter_mut().find(|s| !s.removed) {
                        if seat.focused_window_id == Some(wid) {
                            let active_tags = state.wm.active_tags;
                            let visible_ids: Vec<u64> = state.wm
                                .windows
                                .iter()
                                .filter(|w| (w.tags & active_tags) != 0 && !w.closed && !w.minimized && w.id != wid && w.app_id.as_deref() != Some("clear-status-interface"))
                                .map(|w| w.id)
                                .collect();
                            seat.focused_window_id = visible_ids.last().copied();
                        }
                    }
                }

                state.wm.needs_render = true;
                state.wm.needs_focus = true;
                state.wm.needs_status_update = true;
                if let Some(ref wm) = state.window_manager {
                    wm.manage_dirty();
                }
            }

            river_window_v1::Event::ShowWindowMenuRequested { x, y } => {
                eprintln!(
                    "[window] id={} show_window_menu_requested at ({}, {}) — ignored",
                    wid, x, y
                );
            }

            river_window_v1::Event::PointerMoveRequested { .. } => {
                // Will handle pointer ops later
            }

            river_window_v1::Event::PointerResizeRequested { .. } => {
                // Will handle pointer ops later
            }

            _ => {}
        }
    }
}

// --- RiverSeatV1 events ---

impl Dispatch<RiverSeatV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverSeatV1,
        event: river_seat_v1::Event,
        _data: &(),
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        let sid = match state.seat_id_for_proxy(proxy) {
            Some(id) => id,
            None => return,
        };

        match event {
            river_seat_v1::Event::Removed => {
                if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == sid) {
                    seat.removed = true;
                }
            }

            river_seat_v1::Event::WindowInteraction {
                window: river_window,
            } => {
                if let Some(wid) = state.window_id_for_proxy(&river_window) {
                    let target_app_id = state.wm.get_window(wid).and_then(|w| w.app_id.clone());
                    if target_app_id.as_deref() == Some("clear-status-interface") {
                        // Do not focus status bar!
                        return;
                    }
                    if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == sid) {
                        eprintln!(
                            "[focus] WindowInteraction: seat={} focused_window_id={} -> {} (app_id={:?})",
                            sid,
                            seat.focused_window_id.unwrap_or(0),
                            wid,
                            target_app_id
                        );
                        let already_focused = seat.focused_window_id == Some(wid);
                        seat.focused_window_id = Some(wid);
                        // Move clicked window to front of cascade stack
                        let moved = state.wm.move_window_to_end(wid);

                        // Disable expose if it was active
                        let mut expose_changed = false;
                        if state.wm.expose_active {
                            crate::wm::set_expose_active(&mut state.wm, false);
                            expose_changed = true;
                        }

                        state.wm.needs_render = true;
                        state.wm.needs_focus = true;
                        state.wm.needs_status_update = true;

                        if !already_focused || moved || expose_changed {
                            if let Some(ref wm) = state.window_manager {
                                wm.manage_dirty();
                            }
                        }
                    }
                }
            }

            river_seat_v1::Event::PointerEnter {
                window: river_window,
            } => {
                if let Some(wid) = state.window_id_for_proxy(&river_window) {
                    if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == sid) {
                        seat.hovered_window_id = Some(wid);

                        if state.wm.expose_active {
                            let already_focused = seat.focused_window_id == Some(wid);
                            seat.focused_window_id = Some(wid);

                            state.wm.needs_focus = true;
                            state.wm.needs_render = true;
                            state.wm.needs_status_update = true;

                            if !already_focused {
                                if let Some(ref wm) = state.window_manager {
                                    wm.manage_dirty();
                                }
                            }
                        }
                    }
                }
            }

            river_seat_v1::Event::PointerLeave => {
                if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == sid) {
                    seat.hovered_window_id = None;
                }
            }

            river_seat_v1::Event::WlSeat { name } => {
                if let Some(ref registry) = state.registry {
                    if let Some((_, seat_proxy)) = state.seat_proxies.iter_mut().find(|(_, sp)| sp.river_seat == *proxy) {
                        if seat_proxy.wl_seat.is_none() {
                            let wl_seat = registry.bind::<wl_seat::WlSeat, _, _>(name, 2, qhandle, ());
                            seat_proxy.wl_seat = Some(wl_seat);
                        }
                    }
                }
            }

            river_seat_v1::Event::OpDelta { dx, dy } => {
                if let Some(ref op) = state.active_pointer_op {
                    let wid = op.window_id;
                    if let Some(win) = state.wm.get_window_mut(wid) {
                        match op.op_type {
                            PointerOpType::Move => {
                                win.x = op.start_x + dx;
                                win.y = op.start_y + dy;
                            }
                            PointerOpType::Resize | PointerOpType::ResizeRight => {
                                win.width = std::cmp::max(50, op.start_width + dx);
                            }
                            PointerOpType::ResizeLeft => {
                                let target_width = std::cmp::max(50, op.start_width - dx);
                                let actual_dx = op.start_width - target_width;
                                win.x = op.start_x + actual_dx;
                                win.width = target_width;
                            }
                            PointerOpType::ResizeBottom => {
                                win.height = std::cmp::max(50, op.start_height + dy);
                            }
                            PointerOpType::ResizeTop => {
                                let target_height = std::cmp::max(50, op.start_height - dy);
                                let actual_dy = op.start_height - target_height;
                                win.y = op.start_y + actual_dy;
                                win.height = target_height;
                            }
                            PointerOpType::ResizeBottomRight => {
                                win.width = std::cmp::max(50, op.start_width + dx);
                                win.height = std::cmp::max(50, op.start_height + dy);
                            }
                            PointerOpType::ResizeBottomLeft => {
                                let target_width = std::cmp::max(50, op.start_width - dx);
                                let actual_dx = op.start_width - target_width;
                                win.x = op.start_x + actual_dx;
                                win.width = target_width;
                                win.height = std::cmp::max(50, op.start_height + dy);
                            }
                            PointerOpType::ResizeTopLeft => {
                                let target_width = std::cmp::max(50, op.start_width - dx);
                                let actual_dx = op.start_width - target_width;
                                win.x = op.start_x + actual_dx;
                                win.width = target_width;

                                let target_height = std::cmp::max(50, op.start_height - dy);
                                let actual_dy = op.start_height - target_height;
                                win.y = op.start_y + actual_dy;
                                win.height = target_height;
                            }
                            PointerOpType::ResizeTopRight => {
                                win.width = std::cmp::max(50, op.start_width + dx);

                                let target_height = std::cmp::max(50, op.start_height - dy);
                                let actual_dy = op.start_height - target_height;
                                win.y = op.start_y + actual_dy;
                                win.height = target_height;
                            }
                        }
                    }
                    if let Some(ref wm) = state.window_manager {
                        wm.manage_dirty();
                    }
                }
            }
            river_seat_v1::Event::OpRelease => {
                state.pointer_op_release_pending = true;
                if let Some(ref wm) = state.window_manager {
                    wm.manage_dirty();
                }
            }

            _ => {}
        }
    }
}

// --- RiverOutputV1 events ---

impl Dispatch<RiverOutputV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverOutputV1,
        event: river_output_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        let oid = state
            .output_proxies
            .iter()
            .find(|(_, op)| op.river_output.id() == proxy.id())
            .map(|(id, _)| *id);

        match event {
            river_output_v1::Event::WlOutput { name } => {
                if let Some(oid) = oid {
                    if let Some(output) = state.wm.outputs.iter_mut().find(|o| o.id == oid) {
                        output.wl_output_name = Some(name);
                        // Also try to copy dimensions from WlOutputInfo if already populated
                        if let Some(info) = state.wl_outputs.iter().find(|info| info.name == name) {
                            if info.width > 0 && info.height > 0 {
                                output.width = info.width;
                                output.height = info.height;
                                output.x = info.x;
                                output.y = info.y;
                                state.wm.needs_render = true;
                                update_screen_bounds(&state.wm.outputs);
                                eprintln!(
                                    "river_output linked to wl_output name={} dimensions (copied): {}x{} at ({},{})",
                                    name, info.width, info.height, info.x, info.y
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// --- wl_output::WlOutput events ---

impl Dispatch<wl_output::WlOutput, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // Find registry name
        let name = state
            .wl_outputs
            .iter()
            .find(|info| info.wl_output.id() == proxy.id())
            .map(|info| info.name);

        let Some(name) = name else { return };

        match event {
            wl_output::Event::Geometry {
                x,
                y,
                ..
            } => {
                if let Some(info) = state.wl_outputs.iter_mut().find(|info| info.name == name) {
                    info.x = x;
                    info.y = y;
                }
                // Also update matched Output in WindowManager
                if let Some(output) = state.wm.outputs.iter_mut().find(|o| o.wl_output_name == Some(name)) {
                    if output.x != x || output.y != y {
                        output.x = x;
                        output.y = y;
                        state.wm.needs_render = true;
                        update_screen_bounds(&state.wm.outputs);
                    }
                }
            }
            wl_output::Event::Mode {
                flags,
                width,
                height,
                ..
            } => {
                let is_current = match flags {
                    wayland_client::WEnum::Value(mode) => mode.contains(wl_output::Mode::Current),
                    _ => false,
                };

                if is_current {
                    if let Some(info) = state.wl_outputs.iter_mut().find(|info| info.name == name) {
                        info.width = width;
                        info.height = height;
                    }
                    // Also update matched Output in WindowManager
                    if let Some(output) = state.wm.outputs.iter_mut().find(|o| o.wl_output_name == Some(name)) {
                        if output.width != width || output.height != height {
                            output.width = width;
                            output.height = height;
                            state.wm.needs_render = true;
                            update_screen_bounds(&state.wm.outputs);
                            eprintln!(
                                "wl_output name={} updated current mode dimensions: {}x{}",
                                name, width, height
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// --- RiverLayerShellV1 (no events) ---

impl Dispatch<RiverLayerShellV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &RiverLayerShellV1,
        _event: river_layer_shell_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

// --- RiverLayerShellOutputV1 events ---

impl Dispatch<RiverLayerShellOutputV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverLayerShellOutputV1,
        event: river_layer_shell_output_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        let oid = state
            .output_proxies
            .iter()
            .find(|(_, op)| {
                op.layer_shell_output.as_ref().map_or(false, |lso| {
                    lso.id().protocol_id() == proxy.id().protocol_id()
                })
            })
            .map(|(id, _)| *id);

        let Some(oid) = oid else { return };

        match event {
            river_layer_shell_output_v1::Event::NonExclusiveArea {
                x,
                y,
                width,
                height,
            } => {
                if let Some(output) = state.wm.outputs.iter_mut().find(|o| o.id == oid) {
                    if output.usable_width != width
                        || output.usable_height != height
                        || output.usable_x != x
                        || output.usable_y != y
                    {
                        output.usable_x = x;
                        output.usable_y = y;
                        output.usable_width = width;
                        output.usable_height = height;
                        state.wm.needs_render = true;
                        eprintln!(
                            "output id={} usable area: {}x{} at ({},{})",
                            oid, width, height, x, y
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

// --- RiverInputManagerV1 events ---

impl Dispatch<RiverInputManagerV1, ()> for AppState {
    event_created_child!(AppState, RiverInputManagerV1, [
        river_input_manager_v1::EVT_INPUT_DEVICE_OPCODE => (RiverInputDeviceV1, ()),
    ]);

    fn event(
        state: &mut Self,
        _proxy: &RiverInputManagerV1,
        event: river_input_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            river_input_manager_v1::Event::InputDevice { id: _input_device } => {
                use crate::types::InputDevice;
                let dev = InputDevice { is_keyboard: false };
                state.wm.input_devices.push(dev);
                eprintln!("input_device discovered");
            }
            river_input_manager_v1::Event::Finished => {}
            _ => {}
        }
    }
}

// --- RiverInputDeviceV1 events ---

impl Dispatch<RiverInputDeviceV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverInputDeviceV1,
        event: river_input_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        match event {
            river_input_device_v1::Event::Type { _type: dev_type } => {
                if let Some(dev) = state.wm.input_devices.last_mut() {
                    // dev_type is WEnum<Type>; check for Keyboard variant
                    dev.is_keyboard = matches!(
                        dev_type,
                        wayland_client::WEnum::Value(river_input_device_v1::Type::Keyboard)
                    );
                }
            }
            river_input_device_v1::Event::Name { name } => {
                eprintln!("input_device name: {}", name);
                state.input_device_names.insert(proxy.id().protocol_id(), name.clone());
                if let Some(dev) = state.libinput_devices.iter_mut().find(|d| {
                    if let Some(ref id) = d.input_device {
                        id.id().protocol_id() == proxy.id().protocol_id()
                    } else {
                        false
                    }
                }) {
                    dev.name = name.clone();
                    dev.name_received = true;
                    eprintln!("[libinput] associated name \"{}\" with device", name);
                }
                crate::wayland::apply_input_config(state, qhandle);
            }
            river_input_device_v1::Event::Removed => {}
            _ => {}
        }
    }
}

// --- RiverPointerBindingV1 events ---

impl Dispatch<RiverPointerBindingV1, BindingUserData> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &RiverPointerBindingV1,
        event: river_pointer_binding_v1::Event,
        data: &BindingUserData,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            river_pointer_binding_v1::Event::Pressed => {
                eprintln!(
                    "[binding] pointer pressed: action={:?} seat_id={}",
                    data.action, data.seat_id
                );
                if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == data.seat_id) {
                    seat.pending_action = data.action.clone();
                    seat.pending_command = data.command.clone();
                }
                // Trigger a manage sequence so the pending action is processed
                if let Some(ref wm) = state.window_manager {
                    wm.manage_dirty();
                }
            }
            _ => {}
        }
    }
}

// --- XKB bindings ---

impl Dispatch<RiverXkbBindingsV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &RiverXkbBindingsV1,
        _event: river_xkb_bindings_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<RiverXkbBindingV1, BindingUserData> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &RiverXkbBindingV1,
        event: river_xkb_binding_v1::Event,
        data: &BindingUserData,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            river_xkb_binding_v1::Event::Pressed => {
                eprintln!(
                    "[binding] xkb pressed: action={:?} command={:?} seat_id={}",
                    data.action, data.command, data.seat_id
                );
                if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == data.seat_id) {
                    seat.pending_action = data.action.clone();
                    seat.pending_command = data.command.clone();
                }
                // Trigger a manage sequence so the pending action is processed
                if let Some(ref wm) = state.window_manager {
                    wm.manage_dirty();
                }
            }
            river_xkb_binding_v1::Event::Released => {}
            river_xkb_binding_v1::Event::StopRepeat => {}
            _ => {}
        }
    }
}

impl Dispatch<RiverXkbBindingsSeatV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &RiverXkbBindingsSeatV1,
        _event: river_xkb_bindings_seat_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

// --- RiverNodeV1 (no events, used for set_position) ---

impl Dispatch<RiverNodeV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &RiverNodeV1,
        _event: river_node_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // river_node_v1 has no events
    }
}

// --- Helper functions ---

/// Execute an action triggered by a keybinding or pointer binding.
fn execute_action(state: &mut AppState, seat_id: u64, action: &crate::types::Action, command: Option<&str>) {
    use crate::types::Action;
    match action {
        Action::None => {}
        Action::Spawn => {
            if let Some(cmd) = command {
                eprintln!("spawn: {}", cmd);
                // Close inherited FDs > 2 in the child so that spawned
                // Wayland clients (fuzzel, etc.) never accidentally read
                // from ccec's Wayland socket fd. This prevents protocol
                // corruption and the CPU spin loop that results from it.
                use std::os::unix::process::CommandExt;
                let _ = unsafe {
                    std::process::Command::new("sh")
                        .arg("-c")
                        .arg(cmd)
                        .env_remove("WAYLAND_DEBUG")
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .pre_exec(|| {
                            let max_fd = libc::sysconf(libc::_SC_OPEN_MAX) as libc::c_int;
                            for fd in 3..max_fd {
                                libc::close(fd);
                            }
                            libc::setsid();
                            Ok(())
                        })
                        .spawn()
                };
            }
        }
        Action::Close => {
            // Ask the compositor to close the focused window by calling
            // close() on its River protocol proxy. This matches tinyrwm's
            // approach: close() sends a request to River, which asks the
            // client to close. When the client actually closes, River sends
            // Event::Closed, which sets window.closed = true. Then on the
            // next ManageStart, remove_windows() drops it from the vector.
            //
            // close() modifies window management state and can only be called
            // during a manage sequence — which it is, since execute_action
            // runs inside ManageStart.
            if let Some(seat) = state.wm.seats.first() {
                if let Some(focused_id) = seat.focused_window_id {
                    // Send the close request to the compositor
                    if let Some(wp) = state.get_window_proxy(focused_id) {
                        wp.river_window.close();
                    }
                    // Shift focus to the next visible window (excluding the one we just closed).
                    // The window isn't closed=true yet (that happens when River sends Event::Closed),
                    // so we exclude it by ID instead.
                    let visible_ids: Vec<u64> = state
                        .wm
                        .windows
                        .iter()
                        .filter(|w| {
                            (w.tags & state.wm.active_tags) != 0 && !w.closed && !w.minimized && w.id != focused_id && w.app_id.as_deref() != Some("clear-status-interface")
                        })
                        .map(|w| w.id)
                        .collect();
                    let next_id = visible_ids.last().copied();
                    if let Some(seat) = state.wm.seats.iter_mut().find(|s| !s.removed) {
                        seat.focused_window_id = next_id;
                    }
                }
            }
            state.wm.needs_render = true;
            state.wm.needs_focus = true;
            state.wm.needs_status_update = true;
        }
        Action::Minimize => {
            // Get the focused window ID on this seat
            let focused_id = state.wm.seats.iter().find(|s| s.id == seat_id).and_then(|s| s.focused_window_id);
            if let Some(focused_id) = focused_id {
                if let Some(w) = state.wm.get_window_mut(focused_id) {
                    w.minimized = true;
                }
                // Shift focus to the next visible window
                let active_tags = state.wm.active_tags;
                let visible_ids: Vec<u64> = state.wm
                    .windows
                    .iter()
                    .filter(|w| (w.tags & active_tags) != 0 && !w.closed && !w.minimized && w.id != focused_id && w.app_id.as_deref() != Some("clear-status-interface"))
                    .map(|w| w.id)
                    .collect();
                let next_id = visible_ids.last().copied();
                if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == seat_id) {
                    seat.focused_window_id = next_id;
                }
            }
            state.wm.needs_render = true;
            state.wm.needs_focus = true;
            state.wm.needs_status_update = true;
            if let Some(ref wm) = state.window_manager {
                wm.manage_dirty();
            }
        }
        Action::FocusNext => {
            // Focus the next visible window (wrapping) of the same tiling mode,
            // and move it to the front of the cascade stack (end of windows vector).
            if let Some(seat) = state.wm.seats.iter_mut().find(|s| !s.removed) {
                let focused_id = seat.focused_window_id;
                let focused_mode = focused_id.and_then(|fid| {
                    state.wm.windows.iter().find(|w| w.id == fid).map(|w| w.tiling_mode)
                });
                let visible_ids: Vec<u64> = state
                    .wm
                    .windows
                    .iter()
                    .filter(|w| {
                        (w.tags & state.wm.active_tags) != 0
                            && !w.closed
                            && !w.minimized
                            && w.app_id.as_deref() != Some("clear-status-interface")
                            && (focused_mode.is_none() || Some(w.tiling_mode) == focused_mode)
                    })
                    .map(|w| w.id)
                    .collect();
                if let Some(fid) = focused_id {
                    if let Some(idx) = visible_ids.iter().position(|id| *id == fid) {
                        let next_idx = (idx + 1) % visible_ids.len();
                        let next_id = visible_ids[next_idx];
                        seat.focused_window_id = Some(next_id);
                        // Move the newly focused window to the end of the
                        // windows vector so it gets the front cascade position
                        // (rightmost/bottommost) and brightest border.
                        state.wm.move_window_to_end(next_id);
                        state.wm.needs_render = true;
                        state.wm.needs_focus = true;
                        state.wm.needs_status_update = true;
                    }
                } else if !visible_ids.is_empty() {
                    let next_id = visible_ids[visible_ids.len() - 1];
                    seat.focused_window_id = Some(next_id);
                    state.wm.move_window_to_end(next_id);
                    state.wm.needs_render = true;
                    state.wm.needs_focus = true;
                    state.wm.needs_status_update = true;
                }
            }
        }
        Action::FocusPrev => {
            // Focus the previous visible window (wrapping) of the same tiling mode,
            // and move it to the front of the cascade stack (end of windows vector).
            if let Some(seat) = state.wm.seats.iter_mut().find(|s| !s.removed) {
                let focused_id = seat.focused_window_id;
                let focused_mode = focused_id.and_then(|fid| {
                    state.wm.windows.iter().find(|w| w.id == fid).map(|w| w.tiling_mode)
                });
                let visible_ids: Vec<u64> = state
                    .wm
                    .windows
                    .iter()
                    .filter(|w| {
                        (w.tags & state.wm.active_tags) != 0
                            && !w.closed
                            && !w.minimized
                            && w.app_id.as_deref() != Some("clear-status-interface")
                            && (focused_mode.is_none() || Some(w.tiling_mode) == focused_mode)
                    })
                    .map(|w| w.id)
                    .collect();
                if let Some(fid) = focused_id {
                    if let Some(idx) = visible_ids.iter().position(|id| *id == fid) {
                        let prev_idx = if idx == 0 { visible_ids.len() - 1 } else { idx - 1 };
                        let prev_id = visible_ids[prev_idx];
                        seat.focused_window_id = Some(prev_id);
                        // Move the newly focused window to the end of the
                        // windows vector so it gets the front cascade position
                        // (rightmost/bottommost) and brightest border.
                        state.wm.move_window_to_end(prev_id);
                        state.wm.needs_render = true;
                        state.wm.needs_focus = true;
                        state.wm.needs_status_update = true;
                    }
                } else if !visible_ids.is_empty() {
                    let prev_id = visible_ids[visible_ids.len() - 1];
                    seat.focused_window_id = Some(prev_id);
                    state.wm.move_window_to_end(prev_id);
                    state.wm.needs_render = true;
                    state.wm.needs_focus = true;
                    state.wm.needs_status_update = true;
                }
            }
        }
        Action::Move | Action::Resize => {
            let op_type = if *action == Action::Move {
                PointerOpType::Move
            } else {
                state.pending_pointer_op_type.take().unwrap_or(PointerOpType::Resize)
            };

            let target_wid = if let Some(seat) = state.wm.seats.iter().find(|s| s.id == seat_id) {
                seat.interacted_window_id.or(seat.focused_window_id)
            } else {
                None
            };

            if let Some(wid) = target_wid {
                let mut window_found = false;
                let mut needs_render = false;
                if let Some(win) = state.wm.get_window_mut(wid) {
                    if win.tiling_mode != TilingMode::Fullscreen && win.tiling_mode != TilingMode::Popup {
                        if win.tiling_mode != TilingMode::Floating {
                            win.tiling_mode = TilingMode::Floating;
                            needs_render = true;
                        }
                        win.mode_locked = true;
                        window_found = true;
                    }
                }

                if window_found {
                    if needs_render {
                        state.wm.needs_render = true;
                        state.wm.needs_focus = true;
                        state.wm.needs_status_update = true;
                    }
                    if let Some((_, sp)) = state.seat_proxies.iter().find(|(sid, _)| *sid == seat_id) {
                        sp.river_seat.op_start_pointer();
                    }
                    if op_type != PointerOpType::Move {
                        if let Some(wp) = state.get_window_proxy(wid) {
                            wp.river_window.inform_resize_start();
                        }
                    }
                    if let Some(win) = state.wm.get_window_mut(wid) {
                        win.anim_x = None;
                        win.anim_y = None;
                        win.anim_w = None;
                        win.anim_h = None;
                        win.anim_opacity = None;
                        state.active_pointer_op = Some(PointerOp {
                            window_id: wid,
                            op_type,
                            start_x: win.x,
                            start_y: win.y,
                            start_width: win.width,
                            start_height: win.height,
                        });
                    }
                }
            }
        }
        Action::Exit => {
            state.wm.exit_requested = true;
            state.exit_requested = true;
            if let Some(ref wm) = state.window_manager {
                wm.exit_session();
            }
        }
        Action::Fullscreen => {
            if let Some(seat) = state.wm.seats.first() {
                if let Some(focused_id) = seat.focused_window_id {
                    // Read current state and compute target mode before mutable borrow
                    let is_fullscreen = state
                        .wm
                        .get_window(focused_id)
                        .map(|w| w.tiling_mode == TilingMode::Fullscreen)
                        .unwrap_or(false);

                    let resolved_mode = if is_fullscreen {
                        // Exiting fullscreen: snapshot the window info we need for mode resolution
                        let (app_id, title, tags, mode_locked) = state
                            .wm
                            .get_window(focused_id)
                            .map(|w| (w.app_id.clone(), w.title.clone(), w.tags, w.mode_locked))
                            .unwrap_or((None, None, 1, false));
                        // Build a temporary Window for mode resolution
                        let temp_win = Window {
                            id: focused_id,
                            is_new: false,
                            closed: false,
                            tags,
                            x: 0,
                            y: 0,
                            width: 0,
                            height: 0,
                            app_id,
                            title,
                            identifier: None,
                            parent_id: None,
                            has_parent: false,
                            pid: 0,
                            hint_min_width: 0,
                            hint_min_height: 0,
                            hint_max_width: 0,
                            hint_max_height: 0,
                            decoration_hint: 3,
                            presentation_hint: 0,
                            fullscreen_requested: false,
                            maximize_requested: false,
                            minimize_requested: false,
                            tiling_mode: TilingMode::Fullscreen,
                            mode_locked,
                            xprop_check_attempts: 0,
                            ..Default::default()
                        };
                        crate::wm::get_mode_for_window(&state.wm, &temp_win)
                            .unwrap_or(state.wm.global_layout)
                    } else {
                        TilingMode::Fullscreen
                    };

                    if let Some(window) = state.wm.get_window_mut(focused_id) {
                        if is_fullscreen {
                            window.tiling_mode = resolved_mode;
                            window.mode_locked = false;
                        } else {
                            window.tiling_mode = TilingMode::Fullscreen;
                            window.mode_locked = true;
                        }
                        state.wm.needs_render = true;
                        state.wm.needs_status_update = true;
                    }
                }
            }
        }
        Action::LayoutNext => {
            let cycle = [
                TilingMode::Cascade,
                TilingMode::Grid,
                TilingMode::Fullscreen,
            ];
            // Cycle the layout for the currently active tag(s) only.
            // Determine the "current" mode from the first active tag's layout
            // (or global_layout if no tag_layout is set for it), then advance.
            let active_tags = state.wm.active_tags;
            let first_tag_bit = (0..crate::types::NUM_TAGS).find(|b| (active_tags & (1u32 << b)) != 0);
            let current = if let Some(bit) = first_tag_bit {
                if state.wm.has_tag_layout[bit] {
                    state.wm.tag_layouts[bit]
                } else {
                    state.wm.global_layout
                }
            } else {
                state.wm.global_layout
            };
            let next = cycle
                .iter()
                .position(|m| *m == current)
                .map(|i| cycle[(i + 1) % cycle.len()])
                .unwrap_or(TilingMode::Cascade);

            // Set the layout for every currently active tag.
            for tag_bit in 0..crate::types::NUM_TAGS {
                if (active_tags & (1u32 << tag_bit)) != 0 {
                    state.wm.tag_layouts[tag_bit] = next;
                    state.wm.has_tag_layout[tag_bit] = true;
                }
            }
            eprintln!(
                "layout-next: tag layout set to {} for active_tags=0b{:b}",
                next.as_str(),
                active_tags
            );

            if state.wm.notifications_enable {
                crate::config::show_notification("ccec", &format!("Layout set to {} for active tags", next.as_str()));
            }

            // Unlock windows that got their mode from the layout (not from mode_rules
            // or manual set-mode) so assign_window_modes will reassign them.
            // Windows with mode_locked=true were explicitly set by the user and stay.
            // Windows matched by mode_rules will get reassigned to the same rule mode.
            // Only windows that fell through to tag_layouts/global_layout will change.

            state.wm.needs_render = true;
            state.wm.needs_status_update = true;
        }
        Action::ModeNext => {
            let cycle = [
                TilingMode::Cascade,
                TilingMode::Grid,
                TilingMode::Fullscreen,
                TilingMode::Floating,
            ];
            let focused_id = state
                .wm
                .seats
                .iter()
                .find(|s| !s.removed)
                .and_then(|s| s.focused_window_id);
            if let Some(fid) = focused_id {
                let notifications_enable = state.wm.notifications_enable;
                if let Some(win) = state.wm.get_window_mut(fid) {
                    let next = cycle
                        .iter()
                        .position(|m| *m == win.tiling_mode)
                        .map(|i| cycle[(i + 1) % cycle.len()])
                        .unwrap_or(TilingMode::Cascade);
                    eprintln!(
                        "mode-next: window {} ({:?}) {} -> {}",
                        fid,
                        win.app_id,
                        win.tiling_mode.as_str(),
                        next.as_str()
                    );
                    win.tiling_mode = next;
                    win.mode_locked = true;
                    if notifications_enable {
                        let win_title = win.title.as_deref().unwrap_or("Window");
                        crate::config::show_notification("ccec", &format!("Tiling mode set to {} for: {}", next.as_str(), win_title));
                    }
                    state.wm.needs_render = true;
                    state.wm.needs_status_update = true;
                }
            }
        }
        Action::Reload => {
            crate::restart::wm_restart();
        }
        Action::Restart => {
            crate::restart::wm_restart();
        }
        Action::Expose => {
            let active = !state.wm.expose_active;
            crate::wm::set_expose_active(&mut state.wm, active);
            state.wm.needs_render = true;
            state.wm.needs_status_update = true;
        }
        Action::View1 | Action::View2 | Action::View3 | Action::View4 => {
            let tag = match action {
                Action::View1 => 1,
                Action::View2 => 2,
                Action::View3 => 3,
                Action::View4 => 4,
                _ => return,
            };
            state.wm.active_tags = 1 << (tag - 1);

            // Reassign focus to a visible window on the new tag
            let visible_ids: Vec<u64> = state
                .wm
                .windows
                .iter()
                .filter(|w| (w.tags & state.wm.active_tags) != 0 && !w.closed && w.app_id.as_deref() != Some("clear-status-interface"))
                .map(|w| w.id)
                .collect();
            if let Some(seat) = state.wm.seats.iter_mut().find(|s| !s.removed) {
                seat.focused_window_id = visible_ids.last().copied();
            }

            state.wm.needs_render = true;
            state.wm.needs_focus = true;
            state.wm.needs_status_update = true;
        }
        Action::Toggle1 | Action::Toggle2 | Action::Toggle3 | Action::Toggle4 => {
            let tag = match action {
                Action::Toggle1 => 1,
                Action::Toggle2 => 2,
                Action::Toggle3 => 3,
                Action::Toggle4 => 4,
                _ => return,
            };
            state.wm.active_tags ^= 1 << (tag - 1);

            // If the focused window is no longer visible, reassign focus
            let focused_id = state
                .wm
                .seats
                .iter()
                .find(|s| !s.removed)
                .and_then(|s| s.focused_window_id);
            let focused_still_visible = focused_id.map_or(false, |fid| {
                state
                    .wm
                    .get_window(fid)
                    .map_or(false, |w| (w.tags & state.wm.active_tags) != 0 && !w.closed)
            });
            if !focused_still_visible {
                let visible_ids: Vec<u64> = state
                    .wm
                    .windows
                    .iter()
                    .filter(|w| (w.tags & state.wm.active_tags) != 0 && !w.closed && w.app_id.as_deref() != Some("clear-status-interface"))
                    .map(|w| w.id)
                    .collect();
                if let Some(seat) = state.wm.seats.iter_mut().find(|s| !s.removed) {
                    seat.focused_window_id = visible_ids.last().copied();
                }
                state.wm.needs_focus = true;
            }

            state.wm.needs_render = true;
            state.wm.needs_status_update = true;
        }
        Action::SetTag1 | Action::SetTag2 | Action::SetTag3 | Action::SetTag4 => {
            let tag = match action {
                Action::SetTag1 => 1,
                Action::SetTag2 => 2,
                Action::SetTag3 => 3,
                Action::SetTag4 => 4,
                _ => return,
            };
            // Read focused_id before any mutable borrow
            let focused_id = state
                .wm
                .seats
                .iter()
                .find(|s| !s.removed)
                .and_then(|s| s.focused_window_id);
            if let Some(focused_id) = focused_id {
                // Set the window's tag
                let active_tags = state.wm.active_tags;
                let window_left_active_tag =
                    state.wm.get_window_mut(focused_id).map_or(false, |window| {
                        window.tags = 1 << (tag - 1);
                        (window.tags & active_tags) == 0
                    });

                // If the window is no longer on an active tag, shift focus
                if window_left_active_tag {
                    let visible_ids: Vec<u64> = state
                        .wm
                        .windows
                        .iter()
                        .filter(|w| (w.tags & state.wm.active_tags) != 0 && !w.closed && w.app_id.as_deref() != Some("clear-status-interface"))
                        .map(|w| w.id)
                        .collect();
                    if let Some(seat) = state.wm.seats.iter_mut().find(|s| !s.removed) {
                        seat.focused_window_id = visible_ids.last().copied();
                    }
                    state.wm.needs_focus = true;
                }

                state.wm.needs_render = true;
                state.wm.needs_status_update = true;
            }
        }
    }
}

fn enforce_single_instance(wm: &mut WindowManager) {
    for rule in &wm.mode_rules {
        if !rule.single_instance {
            continue;
        }
        let mut first_matched = false;
        for window in &mut wm.windows {
            if window.closed {
                continue;
            }
            let has_app_id = window.app_id.as_deref().map_or(false, |s| !s.is_empty());
            let match_app = rule.app_id_pattern == "*"
                || window
                    .app_id
                    .as_deref()
                    .map_or(false, |aid| aid.contains(&rule.app_id_pattern))
                || (!has_app_id && window.title.as_deref().map_or(false, |t| {
                    let normalize = |s: &str| -> String {
                        s.to_lowercase().replace(|c: char| c == '-' || c == '_', " ")
                    };
                    normalize(t).contains(&normalize(&rule.app_id_pattern))
                }));
            let match_title = rule.title_pattern.as_deref() == Some("*")
                || rule.title_pattern.is_none()
                || window.title.as_deref().map_or(false, |t| {
                    t.contains(rule.title_pattern.as_deref().unwrap_or(""))
                });
            if match_app && match_title {
                if first_matched {
                    window.tiling_mode = TilingMode::Floating;
                    window.mode_locked = true;
                }
                first_matched = true;
            }
        }
    }
}

/// Convert our u32 modifier bitmask to the generated Modifiers bitflags.
fn u32_to_modifiers(mods: u32) -> Modifiers {
    Modifiers::from_bits_truncate(mods)
}

fn apply_pending_bindings(state: &mut AppState, qhandle: &QueueHandle<AppState>) {
    // Clear old binding proxies (destroying them unregisters the bindings)
    state.xkb_binding_proxies.clear();
    state.pointer_binding_proxies.clear();

    // Create xkb binding seats for any seats that don't have one yet
    if state.xkb_bindings.is_some() {
        for (_sid, sp) in &mut state.seat_proxies {
            if sp.xkb_bindings_seat.is_none() {
                if let Some(ref xb) = state.xkb_bindings {
                    sp.xkb_bindings_seat = Some(xb.get_seat(&sp.river_seat, qhandle, ()));
                }
            }
        }
    }

    // Apply xkb bindings to each seat
    let bindings: Vec<_> = state.wm.pending_bindings.drain(..).collect();
    eprintln!(
        "[bindings] applying {} xkb bindings to {} seats",
        bindings.len(),
        state.seat_proxies.len()
    );
    for pb in &bindings {
        for (sid, sp) in &state.seat_proxies {
            if let Some(ref xb) = state.xkb_bindings {
                if let Some(ref _xbs) = sp.xkb_bindings_seat {
                    let modifiers = u32_to_modifiers(pb.mods);
                    let binding_data = BindingUserData {
                        seat_id: *sid,
                        action: pb.action.clone(),
                        command: pb.command.clone(),
                    };
                    let binding = xb.get_xkb_binding(
                        &sp.river_seat,
                        pb.keysym,
                        modifiers,
                        qhandle,
                        binding_data,
                    );
                    binding.enable();
                    // Store the proxy so it stays alive (binding would be unregistered on drop)
                    state.xkb_binding_proxies.push(binding);
                }
            }
        }
    }

    // Apply pointer bindings to each seat
    let ptr_bindings: Vec<_> = state.wm.pending_pointer_bindings.drain(..).collect();
    for ppb in &ptr_bindings {
        for (sid, sp) in &state.seat_proxies {
            let modifiers = u32_to_modifiers(ppb.mods);
            let binding_data = BindingUserData {
                seat_id: *sid,
                action: ppb.action.clone(),
                command: None,
            };
            let pb =
                sp.river_seat
                    .get_pointer_binding(ppb.button, modifiers, qhandle, binding_data);
            // Store the proxy so it stays alive
            state.pointer_binding_proxies.push(pb);
        }
    }
}

// --- wlr-output-management dispatch implementations ---

/// Helper: convert an f64 scale value to the wl_fixed_t format
/// used by the Wayland protocol set_scale method.
/// wayland-scanner already converts wl_fixed -> f64 for events,
/// but set_scale takes f64 directly (the scanner handles the conversion).
fn _f64_to_wl_fixed(v: f64) -> i32 {
    (v * 256.0) as i32
}

impl Dispatch<ZwlrOutputManagerV1, ()> for AppState {
    event_created_child!(AppState, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);

    fn event(
        state: &mut Self,
        _proxy: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head: _ } => {
                eprintln!("[output-mgmt] Head event (new head advertised)");
            }
            zwlr_output_manager_v1::Event::Done { serial } => {
                eprintln!(
                    "[output-mgmt] Done serial={} pending_scale_apply={} output_scale={} heads={}",
                    serial,
                    state.wm.pending_scale_apply,
                    state.wm.output_scale,
                    state.output_heads.len()
                );
                for h in &state.output_heads {
                    eprintln!(
                        "[output-mgmt]   head {} enabled={} scale={}",
                        h.name, h.enabled, h.scale
                    );
                }
                state.output_serial = serial;

                // If config requested scale application and we have heads, apply it
                if state.wm.pending_scale_apply && state.wm.output_scale > 0.0 {
                    apply_output_scale(state, qhandle);
                }
            }
            zwlr_output_manager_v1::Event::Finished => {
                eprintln!(
                    "[output-mgmt] manager finished, heads before clear: {}",
                    state.output_heads.len()
                );
                state.output_manager = None;
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputHeadV1, ()> for AppState {
    event_created_child!(AppState, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);

    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // Find or create the head entry by proxy ID
        let pid = proxy.id().protocol_id();

        match event {
            zwlr_output_head_v1::Event::Name { name } => {
                // Check if we already track this head
                if let Some(head) = state
                    .output_heads
                    .iter_mut()
                    .find(|h| h.proxy.id().protocol_id() == pid)
                {
                    head.name = name.clone();
                } else {
                    state.output_heads.push(OutputHeadInfo {
                        proxy: proxy.clone(),
                        name,
                        enabled: false,
                        scale: 1.0,
                    });
                }
                eprintln!(
                    "[output-mgmt] head name={}",
                    state
                        .output_heads
                        .iter()
                        .find(|h| h.proxy.id().protocol_id() == pid)
                        .map(|h| h.name.as_str())
                        .unwrap_or("?")
                );
            }
            zwlr_output_head_v1::Event::Description { description } => {
                let _ = description; // not needed for scale config
            }
            zwlr_output_head_v1::Event::PhysicalSize { width, height } => {
                let _ = (width, height);
            }
            zwlr_output_head_v1::Event::Enabled { enabled } => {
                let head_name = state
                    .output_heads
                    .iter()
                    .find(|h| h.proxy.id().protocol_id() == pid)
                    .map(|h| h.name.as_str())
                    .unwrap_or("?");
                eprintln!(
                    "[output-mgmt] head {} Enabled enabled={} pending_scale_apply={}",
                    head_name, enabled, state.wm.pending_scale_apply
                );
                if let Some(head) = state
                    .output_heads
                    .iter_mut()
                    .find(|h| h.proxy.id().protocol_id() == pid)
                {
                    head.enabled = enabled != 0;
                }
            }
            zwlr_output_head_v1::Event::Scale { scale } => {
                let head_name = state
                    .output_heads
                    .iter()
                    .find(|h| h.proxy.id().protocol_id() == pid)
                    .map(|h| h.name.as_str())
                    .unwrap_or("?");
                eprintln!(
                    "[output-mgmt] head {} Scale={} output_scale={} pending_scale_apply={}",
                    head_name, scale, state.wm.output_scale, state.wm.pending_scale_apply
                );
                // wayland-scanner converts wl_fixed to f64 automatically
                if let Some(head) = state
                    .output_heads
                    .iter_mut()
                    .find(|h| h.proxy.id().protocol_id() == pid)
                {
                    // Detect VT-switch-back: compositor reports a scale that
                    // differs from our configured output_scale. This covers
                    // both: (a) the old head had scale=X and compositor sends 1.0,
                    // and (b) the head was re-created after Finished (so its
                    // internal scale was reset to 1.0) and compositor sends 1.0
                    // instead of the configured scale.
                    if state.wm.output_scale > 0.0
                        && head.enabled
                        && (scale - state.wm.output_scale).abs() > 0.01
                    {
                        eprintln!(
                            "[output-mgmt] scale mismatch (got {}, configured {}), will re-apply",
                            scale, state.wm.output_scale
                        );
                        state.wm.pending_scale_apply = true;
                    }
                    head.scale = scale;
                }
            }
            zwlr_output_head_v1::Event::Finished => {
                let head_name = state
                    .output_heads
                    .iter()
                    .find(|h| h.proxy.id().protocol_id() == pid)
                    .map(|h| h.name.as_str())
                    .unwrap_or("?");
                eprintln!(
                    "[output-mgmt] head {} Finished (removing from heads)",
                    head_name
                );
                state
                    .output_heads
                    .retain(|h| h.proxy.id().protocol_id() != pid);
            }
            zwlr_output_head_v1::Event::CurrentMode { mode: _ } => {}
            zwlr_output_head_v1::Event::Position { x, y } => {
                let _ = (x, y);
            }
            zwlr_output_head_v1::Event::Transform { transform } => {
                let _ = transform;
            }
            zwlr_output_head_v1::Event::Make { make } => {
                let _ = make;
            }
            zwlr_output_head_v1::Event::Model { model } => {
                let _ = model;
            }
            zwlr_output_head_v1::Event::SerialNumber { serial_number } => {
                let _ = serial_number;
            }
            zwlr_output_head_v1::Event::AdaptiveSync { state: _ } => {}
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputModeV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                let _ = (width, height);
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => {
                let _ = refresh;
            }
            zwlr_output_mode_v1::Event::Preferred => {}
            zwlr_output_mode_v1::Event::Finished => {}
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for AppState {
    event_created_child!(AppState, ZwlrOutputConfigurationV1, [
        zwlr_output_configuration_v1::REQ_ENABLE_HEAD_OPCODE => (ZwlrOutputConfigurationHeadV1, ()),
    ]);

    fn event(
        state: &mut Self,
        _proxy: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_configuration_v1::Event::Succeeded => {
                eprintln!("[output-mgmt] configuration succeeded");
                state.output_config = None;
            }
            zwlr_output_configuration_v1::Event::Failed => {
                eprintln!("[output-mgmt] configuration failed");
                state.output_config = None;
            }
            zwlr_output_configuration_v1::Event::Cancelled => {
                eprintln!("[output-mgmt] configuration cancelled (stale serial, will retry)");
                state.output_config = None;
                // Re-queue: the done event will fire again with a fresh serial
                state.wm.pending_scale_apply = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrOutputConfigurationHeadV1,
        _event: zwlr_output_configuration_head_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // zwlr_output_configuration_head_v1 has no events — it's request-only
    }
}

/// Apply the configured output scale via the wlr-output-management protocol.
/// Called when a `done` event arrives and `pending_scale_apply` is true.
pub fn apply_output_scale(state: &mut AppState, qhandle: &QueueHandle<AppState>) {
    let Some(ref om) = state.output_manager else {
        eprintln!("[output-mgmt] no output_manager, cannot apply scale");
        return;
    };

    if state.output_heads.is_empty() {
        eprintln!("[output-mgmt] no heads discovered yet, deferring scale apply");
        return;
    }

    // Check if any enabled head actually needs a scale change
    let target_scale = state.wm.output_scale;
    let needs_change = state
        .output_heads
        .iter()
        .any(|h| h.enabled && (h.scale - target_scale).abs() > 0.01);
    if !needs_change {
        eprintln!(
            "[output-mgmt] all heads already at target scale {}",
            target_scale
        );
        state.wm.pending_scale_apply = false;
        return;
    }

    eprintln!(
        "[output-mgmt] applying scale {} to {} head(s)",
        target_scale,
        state.output_heads.len()
    );

    let serial = state.output_serial;

    // Destroy any existing pending configuration
    if let Some(ref config) = state.output_config {
        config.destroy();
    }
    state.output_config = None;

    // Create a new configuration object
    let config = om.create_configuration(serial, qhandle, ());

    // For each head: enable it and set the scale
    for head in &state.output_heads {
        if head.enabled {
            let config_head = config.enable_head(&head.proxy, qhandle, ());
            config_head.set_scale(target_scale);
        } else {
            config.disable_head(&head.proxy);
        }
    }

    config.apply();

    // Store the config proxy so it stays alive until succeeded/failed/cancelled
    state.output_config = Some(config);
    state.wm.pending_scale_apply = false;
}

/// Connect to the Wayland display and set up the event queue.
/// Returns the Connection, EventQueue, and AppState after the initial registry roundtrip.
pub fn wayland_init() -> Result<(Connection, EventQueue<AppState>, AppState), String> {
    let conn = Connection::connect_to_env()
        .map_err(|e| format!("failed to connect to Wayland: {:?}", e))?;

    let mut event_queue = conn.new_event_queue::<AppState>();
    let qh = event_queue.handle();

    let registry = conn.display().get_registry(&qh, RegistryData);

    // Do initial roundtrip to receive global events and bind protocols
    let mut state = AppState::new();
    state.registry = Some(registry);
    eprintln!("[init] first roundtrip starting...");
    let rt1 = std::time::Instant::now();
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| format!("initial roundtrip failed: {:?}", e))?;
    eprintln!(
        "[init] first roundtrip done in {:?}, render_count={}",
        rt1.elapsed(),
        state.render_count
    );

    // Check we got the required protocols
    if !state.has_window_manager {
        return Err("river_window_manager_v1 not available".to_string());
    }
    if !state.has_xkb_bindings {
        return Err("river_xkb_bindings_v1 not available".to_string());
    }

    // Second roundtrip for input device events
    eprintln!("[init] second roundtrip starting...");
    let rt2 = std::time::Instant::now();
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| format!("second roundtrip failed: {:?}", e))?;
    eprintln!(
        "[init] second roundtrip done in {:?}, render_count={}",
        rt2.elapsed(),
        state.render_count
    );

    eprintln!("ccec: Wayland connection established");

    Ok((conn, event_queue, state))
}

// --- RiverLibinputConfigV1 events ---

impl Dispatch<RiverLibinputConfigV1, ()> for AppState {
    event_created_child!(AppState, RiverLibinputConfigV1, [
        river_libinput_config_v1::EVT_LIBINPUT_DEVICE_OPCODE => (RiverLibinputDeviceV1, ()),
    ]);

    fn event(
        state: &mut Self,
        _proxy: &RiverLibinputConfigV1,
        event: river_libinput_config_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            river_libinput_config_v1::Event::LibinputDevice { id: device } => {
                eprintln!("[libinput] device discovered");
                state.libinput_devices.push(LibinputDeviceInfo {
                    device,
                    name: String::new(),
                    input_device: None,
                    name_received: false,
                    tap_finger_count: -1, // not yet received
                    tap_info_received: false,
                    accel_profiles_support: None,
                    natural_scroll_supported: None,
                    dwt_supported: None,
                    dwtp_supported: None,
                    config_applied: false,
                });
                state.wm.tap_config_applied = false;
            }
            river_libinput_config_v1::Event::Finished => {}
            _ => {}
        }
    }
}

// --- RiverLibinputDeviceV1 events ---

impl Dispatch<RiverLibinputDeviceV1, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &RiverLibinputDeviceV1,
        event: river_libinput_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        match event {
            river_libinput_device_v1::Event::InputDevice { device } => {
                if let Some(dev) = state.libinput_devices.iter_mut().find(|d| d.device.id().protocol_id() == proxy.id().protocol_id()) {
                    dev.input_device = Some(device.clone());
                    let dev_id = device.id().protocol_id();
                    if let Some(name) = state.input_device_names.get(&dev_id) {
                        dev.name = name.clone();
                        dev.name_received = true;
                        eprintln!("[libinput] associated name \"{}\" with device on InputDevice event", name);
                    }
                }
                crate::wayland::apply_input_config(state, qhandle);
            }
            river_libinput_device_v1::Event::TapSupport { finger_count } => {
                if let Some(dev) = state.libinput_devices.iter_mut().find(|d| d.device.id().protocol_id() == proxy.id().protocol_id()) {
                    dev.tap_finger_count = finger_count;
                    eprintln!("[libinput] tap support: {} fingers", finger_count);
                }
                crate::wayland::apply_input_config(state, qhandle);
            }
            river_libinput_device_v1::Event::AccelProfilesSupport { profiles } => {
                if let Some(dev) = state.libinput_devices.iter_mut().find(|d| d.device.id().protocol_id() == proxy.id().protocol_id()) {
                    dev.accel_profiles_support = Some(profiles.into());
                    eprintln!("[libinput] accel profiles support: {:?}", profiles);
                }
                crate::wayland::apply_input_config(state, qhandle);
            }
            river_libinput_device_v1::Event::NaturalScrollSupport { supported } => {
                if let Some(dev) = state.libinput_devices.iter_mut().find(|d| d.device.id().protocol_id() == proxy.id().protocol_id()) {
                    dev.natural_scroll_supported = Some(supported != 0);
                    eprintln!("[libinput] natural scroll support: {}", supported != 0);
                }
                crate::wayland::apply_input_config(state, qhandle);
            }
            river_libinput_device_v1::Event::DwtSupport { supported } => {
                if let Some(dev) = state.libinput_devices.iter_mut().find(|d| d.device.id().protocol_id() == proxy.id().protocol_id()) {
                    dev.dwt_supported = Some(supported != 0);
                    eprintln!("[libinput] dwt support: {}", supported != 0);
                }
                crate::wayland::apply_input_config(state, qhandle);
            }
            river_libinput_device_v1::Event::DwtpSupport { supported } => {
                if let Some(dev) = state.libinput_devices.iter_mut().find(|d| d.device.id().protocol_id() == proxy.id().protocol_id()) {
                    dev.dwtp_supported = Some(supported != 0);
                    eprintln!("[libinput] dwtp support: {}", supported != 0);
                }
                crate::wayland::apply_input_config(state, qhandle);
            }
            river_libinput_device_v1::Event::Removed => {
                eprintln!("[libinput] device removed");
                state.libinput_devices.retain(|d| d.device.id().protocol_id() != proxy.id().protocol_id());
                state.wm.tap_config_applied = false;
            }
            _ => {}
        }
    }
}

// --- RiverLibinputResultV1 events ---

impl Dispatch<RiverLibinputResultV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &RiverLibinputResultV1,
        event: river_libinput_result_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            river_libinput_result_v1::Event::Success => {
                eprintln!("[libinput] config applied successfully");
            }
            river_libinput_result_v1::Event::Unsupported => {
                eprintln!("[libinput] config unsupported by device");
            }
            river_libinput_result_v1::Event::Invalid => {
                eprintln!("[libinput] config invalid");
            }
        }
    }
}

/// Apply input configuration to all libinput devices that support it.
pub fn apply_input_config(state: &mut AppState, qhandle: &QueueHandle<AppState>) {
    if state.wm.tap_config_applied {
        return;
    }

    // Wait until ALL discovered devices have received their name and support events (if they have a corresponding input_device).
    let all_info_received = state.libinput_devices.iter().all(|d| {
        if d.input_device.is_none() {
            true
        } else {
            d.name_received &&
            d.tap_finger_count >= 0 &&
            d.accel_profiles_support.is_some() &&
            d.natural_scroll_supported.is_some() &&
            d.dwt_supported.is_some() &&
            d.dwtp_supported.is_some()
        }
    });
    if !all_info_received {
        return;
    }

    // Reset config_applied for all devices to force setting them
    for dev_info in &mut state.libinput_devices {
        dev_info.config_applied = false;
    }

    for dev_info in &mut state.libinput_devices {
        if dev_info.config_applied {
            continue;
        }

        if dev_info.input_device.is_none() {
            dev_info.config_applied = true;
            continue;
        }

        let is_trackpoint = dev_info.name.to_lowercase().contains("trackpoint");

        // 1. Tap to click
        if dev_info.tap_finger_count > 0 {
            let tap_state = if state.wm.tap_to_click {
                river_libinput_device_v1::TapState::Enabled
            } else {
                river_libinput_device_v1::TapState::Disabled
            };
            eprintln!(
                "[libinput] setting tap={} on device ({})",
                if state.wm.tap_to_click { "enabled" } else { "disabled" },
                &dev_info.name
            );
            dev_info.device.set_tap(tap_state, qhandle, ());
        }

        // 2. Accel speed
        let speed = if is_trackpoint {
            state.wm.trackpoint_accel_speed.or(state.wm.accel_speed)
        } else {
            state.wm.accel_speed
        };
        if let Some(s) = speed {
            let s = s.clamp(-1.0, 1.0);
            let speed_bytes = s.to_ne_bytes().to_vec();
            eprintln!("[libinput] setting accel_speed={} on device ({})", s, &dev_info.name);
            dev_info.device.set_accel_speed(speed_bytes, qhandle, ());
        }

        // 3. Accel profile
        let profile_str = if is_trackpoint {
            state.wm.trackpoint_accel_profile.as_ref().or(state.wm.accel_profile.as_ref())
        } else {
            state.wm.accel_profile.as_ref()
        };
        if let Some(profile_name) = profile_str {
            let profile = match profile_name.as_str() {
                "flat" => Some(river_libinput_device_v1::AccelProfile::Flat),
                "adaptive" => Some(river_libinput_device_v1::AccelProfile::Adaptive),
                "none" => Some(river_libinput_device_v1::AccelProfile::None),
                "custom" => Some(river_libinput_device_v1::AccelProfile::Custom),
                _ => None,
            };
            if let Some(p) = profile {
                eprintln!("[libinput] setting accel_profile={:?} on device ({})", p, &dev_info.name);
                dev_info.device.set_accel_profile(p, qhandle, ());
            }
        }

        // 4. Natural scroll
        if let Some(supported) = dev_info.natural_scroll_supported {
            if supported {
                if let Some(natural) = state.wm.natural_scroll {
                    let ns_state = if natural {
                        river_libinput_device_v1::NaturalScrollState::Enabled
                    } else {
                        river_libinput_device_v1::NaturalScrollState::Disabled
                    };
                    eprintln!("[libinput] setting natural_scroll={} on device ({})", natural, &dev_info.name);
                    dev_info.device.set_natural_scroll(ns_state, qhandle, ());
                }
            }
        }

        // 5. Disable while typing (dwt)
        if let Some(supported) = dev_info.dwt_supported {
            if supported {
                if let Some(dwt) = state.wm.dwt {
                    let dwt_state = if dwt {
                        river_libinput_device_v1::DwtState::Enabled
                    } else {
                        river_libinput_device_v1::DwtState::Disabled
                    };
                    eprintln!("[libinput] setting dwt={} on device ({})", dwt, &dev_info.name);
                    dev_info.device.set_dwt(dwt_state, qhandle, ());
                }
            }
        }

        // 6. Disable while trackpointing (dwtp)
        if let Some(supported) = dev_info.dwtp_supported {
            if supported {
                if let Some(dwtp) = state.wm.dwtp {
                    let dwtp_state = if dwtp {
                        river_libinput_device_v1::DwtpState::Enabled
                    } else {
                        river_libinput_device_v1::DwtpState::Disabled
                    };
                    eprintln!("[libinput] setting dwtp={} on device ({})", dwtp, &dev_info.name);
                    dev_info.device.set_dwtp(dwtp_state, qhandle, ());
                }
            }
        }
        // 7. Send events (enable/disable trackpad)
        let is_touchpad = dev_info.name.to_lowercase().contains("touchpad");
        let send_events_mode = if state.wm.trackpad_disabled && is_touchpad {
            river_libinput_device_v1::SendEventsModes::Disabled
        } else {
            river_libinput_device_v1::SendEventsModes::Enabled
        };
        eprintln!(
            "[libinput] setting send_events={:?} on device ({})",
            send_events_mode, &dev_info.name
        );
        dev_info.device.set_send_events(send_events_mode, qhandle, ());

        dev_info.config_applied = true;
    }

    let all_done = state.libinput_devices.iter().all(|d| d.config_applied);
    if all_done && !state.libinput_devices.is_empty() {
        state.wm.tap_config_applied = true;
        eprintln!("[libinput] all input configurations applied to all devices");
    }
}

// --- wl_seat events ---

impl Dispatch<wl_seat::WlSeat, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        match event {
            wl_seat::Event::Capabilities { capabilities } => {
                let has_pointer = match capabilities {
                    wayland_client::WEnum::Value(caps) => caps.contains(wl_seat::Capability::Pointer),
                    _ => false,
                };
                if let Some((_, seat_proxy)) = state.seat_proxies.iter_mut().find(|(_, sp)| {
                    sp.wl_seat.as_ref() == Some(proxy)
                }) {
                    if has_pointer && seat_proxy.wl_pointer.is_none() {
                        let wl_pointer = proxy.get_pointer(qhandle, ());
                        
                        if let Some(ref csm) = state.cursor_shape_manager {
                            let device = csm.get_pointer(&wl_pointer, qhandle, ());
                            device.set_shape(0, Shape::Default);
                            seat_proxy.cursor_shape_device = Some(device);
                        }
                        
                        seat_proxy.wl_pointer = Some(wl_pointer);
                    } else if !has_pointer && seat_proxy.wl_pointer.is_some() {
                        seat_proxy.cursor_shape_device = None;
                        if let Some(pointer) = seat_proxy.wl_pointer.take() {
                            pointer.release();
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// --- wl_pointer events ---

impl Dispatch<wl_pointer::WlPointer, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { serial, surface, surface_x, surface_y } => {
                eprintln!("[pointer] enter surface={:?} x={} y={}", surface, surface_x, surface_y);
                state.pointer_hovered_surface = Some(surface.clone());
                state.last_pointer_surface_x = surface_x;
                state.last_pointer_surface_y = surface_y;

                if let Some((_, seat_proxy)) = state.seat_proxies.iter_mut().find(|(_, sp)| {
                    sp.wl_pointer.as_ref() == Some(proxy)
                }) {
                    seat_proxy.last_pointer_enter_serial = serial;
                }

                Self::update_cursor_shape_for_surface(state, proxy);

                if state.wm.expose_active {
                    let current_surface = &surface;
                    let matched_window = state.window_proxies.iter().find_map(|(id, proxy)| {
                        if let Some(dec) = &proxy.decoration {
                            if &dec.surface == current_surface {
                                return Some(*id);
                            }
                        }
                        if let Some(dec) = &proxy.dec_left {
                            if &dec.surface == current_surface {
                                return Some(*id);
                            }
                        }
                        if let Some(dec) = &proxy.dec_right {
                            if &dec.surface == current_surface {
                                return Some(*id);
                            }
                        }
                        if let Some(dec) = &proxy.dec_bottom {
                            if &dec.surface == current_surface {
                                return Some(*id);
                            }
                        }
                        None
                    });

                    if let Some(wid) = matched_window {
                        let seat_id = state.seat_proxies.iter().find_map(|(sid, sp)| {
                            if sp.wl_pointer.as_ref() == Some(proxy) {
                                Some(*sid)
                            } else {
                                None
                            }
                        });

                        if let Some(sid) = seat_id {
                            if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == sid) {
                                let already_focused = seat.focused_window_id == Some(wid);
                                seat.focused_window_id = Some(wid);
                                state.wm.needs_focus = true;
                                state.wm.needs_render = true;
                                state.wm.needs_status_update = true;

                                if !already_focused {
                                    if let Some(ref wm) = state.window_manager {
                                        wm.manage_dirty();
                                    }
                                }
                            }
                        }
                    }
                }
            }
            wl_pointer::Event::Leave { .. } => {
                eprintln!("[pointer] leave");
                state.pointer_hovered_surface = None;
                Self::update_cursor_shape_for_surface(state, proxy);
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                state.last_pointer_surface_x = surface_x;
                state.last_pointer_surface_y = surface_y;

                Self::update_cursor_shape_for_surface(state, proxy);

                if let Some(pending) = state.pending_border_drag.clone() {
                    let dx = surface_x - pending.start_surface_x;
                    let dy = surface_y - pending.start_surface_y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    // Threshold of 4.0 logical pixels to distinguish click vs drag
                    if dist > 4.0 {
                        state.pending_border_drag = None;

                        // Start the drag! Set the window to Floating and lock it.
                        let mut needs_render = false;
                        if let Some(win) = state.wm.get_window_mut(pending.window_id) {
                            if win.tiling_mode != TilingMode::Fullscreen && win.tiling_mode != TilingMode::Popup {
                                if win.tiling_mode != TilingMode::Floating {
                                    win.tiling_mode = TilingMode::Floating;
                                    needs_render = true;
                                }
                                win.mode_locked = true;
                            }
                        }

                        if needs_render {
                            state.wm.needs_render = true;
                            state.wm.needs_focus = true;
                            state.wm.needs_status_update = true;
                        }

                        // Store the pending op type in AppState so execute_action can retrieve it
                        state.pending_pointer_op_type = Some(pending.op_type);

                        // Tell River to start pointer move/resize grab
                        if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == pending.seat_id) {
                            seat.interacted_window_id = Some(pending.window_id);
                            seat.pending_action = if pending.op_type == PointerOpType::Move {
                                crate::types::Action::Move
                            } else {
                                crate::types::Action::Resize
                            };
                            if let Some(ref wm) = state.window_manager {
                                wm.manage_dirty();
                            }
                        }
                    }
                }
            }
            wl_pointer::Event::Button { button, state: btn_state, .. } => {
                // Left click on border
                if button == 0x110 {
                    if btn_state == wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed) {
                        eprintln!("[pointer] left button pressed, hovered={:?}", state.pointer_hovered_surface);
                        if let Some(ref current_surface) = state.pointer_hovered_surface {
                            let matched_window = state.window_proxies.iter().find_map(|(id, proxy)| {
                                if let Some(dec) = &proxy.decoration {
                                    if &dec.surface == current_surface {
                                        return Some((*id, "top"));
                                    }
                                }
                                if let Some(dec) = &proxy.dec_left {
                                    if &dec.surface == current_surface {
                                        return Some((*id, "left"));
                                    }
                                }
                                if let Some(dec) = &proxy.dec_right {
                                    if &dec.surface == current_surface {
                                        return Some((*id, "right"));
                                    }
                                }
                                if let Some(dec) = &proxy.dec_bottom {
                                    if &dec.surface == current_surface {
                                        return Some((*id, "bottom"));
                                    }
                                }
                                None
                            });
                            eprintln!("[pointer] matched_window={:?}", matched_window);

                            if let Some((wid, surface_type)) = matched_window {
                                let window_tiling_mode = state.wm.windows.iter().find(|w| w.id == wid).map(|w| w.tiling_mode);
                                if let Some(mode) = window_tiling_mode {
                                    if mode != TilingMode::Fullscreen {
                                        // 1. Focus the window immediately
                                        let seat_id = state.seat_proxies.iter().find_map(|(sid, sp)| {
                                            if sp.wl_pointer.as_ref() == Some(proxy) {
                                                Some(*sid)
                                            } else {
                                                None
                                            }
                                        });

                                        if let Some(sid) = seat_id {
                                            if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == sid) {
                                                let already_focused = seat.focused_window_id == Some(wid);
                                                seat.focused_window_id = Some(wid);
                                                let moved = state.wm.move_window_to_end(wid);
                                                let mut expose_changed = false;
                                                if state.wm.expose_active {
                                                    crate::wm::set_expose_active(&mut state.wm, false);
                                                    expose_changed = true;
                                                }
                                                
                                                let mut unminimized = false;
                                                if let Some(w) = state.wm.get_window_mut(wid) {
                                                    if w.minimized {
                                                        w.minimized = false;
                                                        w.minimize_requested = false;
                                                        unminimized = true;
                                                    }
                                                }

                                                state.wm.needs_focus = true;
                                                state.wm.needs_render = true;
                                                state.wm.needs_status_update = true;
                                                if !already_focused || moved || unminimized || expose_changed {
                                                    if let Some(ref wm) = state.window_manager {
                                                        wm.manage_dirty();
                                                    }
                                                }
                                            }
                                                                 // Determine the specific PointerOpType based on coordinates on the matched surface
                                            let op_type = if let Some(w) = state.wm.windows.iter().find(|win| win.id == wid) {
                                                let border_w = if state.wm.expose_active && w.tiling_mode != crate::types::TilingMode::Popup {
                                                    state.wm.layout.grid_border_width
                                                } else {
                                                    match w.tiling_mode {
                                                        crate::types::TilingMode::Cascade => state.wm.layout.cascade_border_width,
                                                        crate::types::TilingMode::Fullscreen => state.wm.layout.fullscreen_border_width,
                                                        crate::types::TilingMode::Grid => state.wm.layout.grid_border_width,
                                                        crate::types::TilingMode::Floating => state.wm.layout.floating_border_width,
                                                        crate::types::TilingMode::Popup => 0,
                                                    }
                                                };
                                                let grab_w = border_w.max(10);
                                                let logical_height = border_w.max(16);
                                                match surface_type {
                                                    "top" => {
                                                        let total_width = w.width + 2 * border_w;
                                                        let mid_y = (logical_height as f64) / 2.0;
                                                        if state.last_pointer_surface_y < mid_y {
                                                            if state.last_pointer_surface_x < CORNER_THRESHOLD {
                                                                PointerOpType::ResizeTopLeft
                                                            } else if state.last_pointer_surface_x > (total_width as f64 - CORNER_THRESHOLD) {
                                                                PointerOpType::ResizeTopRight
                                                            } else {
                                                                PointerOpType::ResizeTop
                                                            }
                                                        } else {
                                                            PointerOpType::Move
                                                        }
                                                    }
                                                    "left" => {
                                                        let mid_x = (grab_w as f64) / 2.0;
                                                        if state.last_pointer_surface_x < mid_x {
                                                            if state.last_pointer_surface_y < CORNER_THRESHOLD {
                                                                PointerOpType::ResizeTopLeft
                                                            } else if state.last_pointer_surface_y > (w.height as f64 - CORNER_THRESHOLD) {
                                                                PointerOpType::ResizeBottomLeft
                                                            } else {
                                                                PointerOpType::ResizeLeft
                                                            }
                                                        } else {
                                                            PointerOpType::Move
                                                        }
                                                    }
                                                    "right" => {
                                                        let mid_x = (grab_w as f64) / 2.0;
                                                        if state.last_pointer_surface_x >= mid_x {
                                                            if state.last_pointer_surface_y < CORNER_THRESHOLD {
                                                                PointerOpType::ResizeTopRight
                                                            } else if state.last_pointer_surface_y > (w.height as f64 - CORNER_THRESHOLD) {
                                                                PointerOpType::ResizeBottomRight
                                                            } else {
                                                                PointerOpType::ResizeRight
                                                            }
                                                        } else {
                                                            PointerOpType::Move
                                                        }
                                                    }
                                                    "bottom" => {
                                                        let total_width = w.width + 2 * grab_w;
                                                        let mid_y = (grab_w as f64) / 2.0;
                                                        if state.last_pointer_surface_y >= mid_y {
                                                            if state.last_pointer_surface_x < CORNER_THRESHOLD {
                                                                PointerOpType::ResizeBottomLeft
                                                            } else if state.last_pointer_surface_x > (total_width as f64 - CORNER_THRESHOLD) {
                                                                PointerOpType::ResizeBottomRight
                                                            } else {
                                                                PointerOpType::ResizeBottom
                                                            }
                                                        } else {
                                                            PointerOpType::Move
                                                        }
                                                    }
                                                    _ => PointerOpType::Move,
                                                }
                                            } else {
                                                PointerOpType::Move
                                            };

                                            // 2. Store the pending drag info (threshold check is in Event::Motion)
                                            state.pending_border_drag = Some(PendingBorderDrag {
                                                window_id: wid,
                                                seat_id: sid,
                                                start_surface_x: state.last_pointer_surface_x,
                                                start_surface_y: state.last_pointer_surface_y,
                                                op_type,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    } else if btn_state == wayland_client::WEnum::Value(wl_pointer::ButtonState::Released) {
                        eprintln!("[pointer] left button released");
                        state.pending_border_drag = None;
                    }
                }

                // Middle click is button 0x112 (BTN_MIDDLE)
                if button == 0x112 && btn_state == wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed) {
                    if let Some(ref current_surface) = state.pointer_hovered_surface {
                        let matched_window_id = state.window_proxies.iter().find_map(|(id, proxy)| {
                            if let Some(dec) = &proxy.decoration {
                                if &dec.surface == current_surface {
                                    return Some(*id);
                                }
                            }
                            if let Some(dec) = &proxy.dec_left {
                                if &dec.surface == current_surface {
                                    return Some(*id);
                                }
                            }
                            if let Some(dec) = &proxy.dec_right {
                                if &dec.surface == current_surface {
                                    return Some(*id);
                                }
                            }
                            if let Some(dec) = &proxy.dec_bottom {
                                if &dec.surface == current_surface {
                                    return Some(*id);
                                }
                            }
                            None
                        });

                        if let Some(wid) = matched_window_id {
                            eprintln!("[pointer] Middle click on window {} border, closing window", wid);
                            if let Some(wp) = state.get_window_proxy(wid) {
                                wp.river_window.close();
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// --- wp_cursor_shape_manager_v1 events ---

impl Dispatch<WpCursorShapeManagerV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &WpCursorShapeManagerV1,
        _event: wp_cursor_shape_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

// --- wp_cursor_shape_device_v1 events ---

impl Dispatch<WpCursorShapeDeviceV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &WpCursorShapeDeviceV1,
        _event: wp_cursor_shape_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

// --- core Wayland and River decoration events dispatch ---

impl Dispatch<wl_compositor::WlCompositor, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_compositor::WlCompositor,
        _event: wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_shm::WlShm,
        _event: wl_shm::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_shm_pool::WlShmPool,
        _event: wl_shm_pool::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_surface::WlSurface,
        _event: wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_buffer::WlBuffer,
        _event: wl_buffer::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<RiverDecorationV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &RiverDecorationV1,
        _event: river_decoration_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}
