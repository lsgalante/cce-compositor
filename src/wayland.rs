// Wayland display connection, registry, and event dispatch for clearwm

use wayland_client::{
    event_created_child, protocol::wl_registry, Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};

use crate::protocol::river_input_management::client::{
    river_input_device_v1::{self, RiverInputDeviceV1},
    river_input_manager_v1::{self, RiverInputManagerV1},
};
use crate::protocol::river_layer_shell::client::{
    river_layer_shell_output_v1::{self, RiverLayerShellOutputV1},
    river_layer_shell_v1::{self, RiverLayerShellV1},
};
use crate::protocol::river_window_management::client::{
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

use crate::types::{BindingUserData, Output, Seat, TilingMode, Window, WindowManager};

// Interface name constants (from river protocol XML)
const IFACE_WINDOW_MANAGER: &str = "river_window_manager_v1";
const IFACE_XKB_BINDINGS: &str = "river_xkb_bindings_v1";
const IFACE_LAYER_SHELL: &str = "river_layer_shell_v1";
const IFACE_INPUT_MANAGER: &str = "river_input_manager_v1";

/// Wayland proxy objects stored alongside each Window, so we can
/// call protocol methods (set_position, propose_dimensions, etc.) on it.
pub struct WindowProxy {
    pub river_window: RiverWindowV1,
}

/// Wayland proxy objects stored alongside each Seat.
pub struct SeatProxy {
    pub river_seat: RiverSeatV1,
    pub xkb_bindings_seat: Option<RiverXkbBindingsSeatV1>,
}

/// Wayland proxy objects stored alongside each Output.
pub struct OutputProxy {
    pub river_output: RiverOutputV1,
    pub layer_shell_output: Option<RiverLayerShellOutputV1>,
}

/// The full app state combining logic state + protocol proxy storage.
pub struct AppState {
    pub wm: WindowManager,

    // Protocol objects (None until bound via registry)
    pub window_manager: Option<RiverWindowManagerV1>,
    pub xkb_bindings: Option<RiverXkbBindingsV1>,
    pub layer_shell: Option<RiverLayerShellV1>,
    pub input_manager: Option<RiverInputManagerV1>,

    // Whether we got all required globals
    pub has_window_manager: bool,
    pub has_xkb_bindings: bool,

    // Proxy objects indexed by window/seat/output ID
    pub window_proxies: Vec<(u64, WindowProxy)>,
    pub seat_proxies: Vec<(u64, SeatProxy)>,
    pub output_proxies: Vec<(u64, OutputProxy)>,

    // River node proxies for window positioning (created via get_node request)
    pub window_nodes: Vec<(u64, RiverNodeV1)>,

    // Next ID counter for new windows/outputs/seats
    pub next_id: u64,

    // Exit flag
    pub exit_requested: bool,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            wm: WindowManager::new(),
            window_manager: None,
            xkb_bindings: None,
            layer_shell: None,
            input_manager: None,
            has_window_manager: false,
            has_xkb_bindings: false,
            window_proxies: Vec::new(),
            seat_proxies: Vec::new(),
            output_proxies: Vec::new(),
            window_nodes: Vec::new(),
            next_id: 1,
            exit_requested: false,
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
        _conn: &Connection,
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
                    crate::restart::wm_restart();
                }
            }

            river_window_manager_v1::Event::ManageStart => {
                eprintln!("manage sequence start");
                state.wm.in_manage_sequence = true;
                state.wm.focused_tags = 0;
                state.wm.needs_render = true;

                // Remove closed windows
                state.wm.windows.retain(|w| !w.closed);
                // Remove removed outputs
                state.wm.outputs.retain(|o| !o.removed);
                // Remove removed seats
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

                wm_proxy.manage_finish();
                state.wm.in_manage_sequence = false;
                crate::status::update_status_files(&state.wm);
            }

            river_window_manager_v1::Event::RenderStart => {
                eprintln!("EVENT: RenderStart (needs_render={})", state.wm.needs_render);
                if state.wm.needs_render {
                    crate::wm::render_windows(state, qhandle);
                    state.wm.needs_render = false;
                }

                wm_proxy.render_finish();
                eprintln!("EVENT: RenderFinish sent");
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
                    .push((id, WindowProxy { river_window }));
                eprintln!("wm_handle_window: new window id={}", id);
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

                state.wm.seats.push(seat);
                state.seat_proxies.push((
                    id,
                    SeatProxy {
                        river_seat,
                        xkb_bindings_seat: None,
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
                    eprintln!("window id={} closed", wid);
                }
            }

            river_window_v1::Event::Dimensions { width, height } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    window.width = width;
                    window.height = height;
                }
            }

            river_window_v1::Event::AppId { app_id } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    if window.app_id != app_id {
                        window.app_id = app_id;
                        state.wm.needs_render = true;
                    }
                }
            }

            river_window_v1::Event::Title { title } => {
                if let Some(window) = state.wm.get_window_mut(wid) {
                    if window.title != title {
                        window.title = title;
                        state.wm.needs_render = true;
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
        _qhandle: &QueueHandle<Self>,
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

            river_seat_v1::Event::WindowInteraction { window: river_window } => {
                if let Some(wid) = state.window_id_for_proxy(&river_window) {
                    if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == sid) {
                        seat.focused_window_id = Some(wid);
                        state.wm.needs_render = true;
                    }
                }
            }

            river_seat_v1::Event::PointerEnter { window: river_window } => {
                if let Some(wid) = state.window_id_for_proxy(&river_window) {
                    if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == sid) {
                        seat.hovered_window_id = Some(wid);
                    }
                }
            }

            river_seat_v1::Event::PointerLeave => {
                if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == sid) {
                    seat.hovered_window_id = None;
                }
            }

            river_seat_v1::Event::OpDelta { .. } => {}
            river_seat_v1::Event::OpRelease => {}

            _ => {}
        }
    }
}

// --- RiverOutputV1 events ---

impl Dispatch<RiverOutputV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &RiverOutputV1,
        event: river_output_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            river_output_v1::Event::WlOutput { .. } => {}
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
                    if output.usable_width != width || output.usable_height != height
                        || output.usable_x != x || output.usable_y != y
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
        _proxy: &RiverInputDeviceV1,
        event: river_input_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            river_input_device_v1::Event::Type { _type: dev_type } => {
                if let Some(dev) = state.wm.input_devices.last_mut() {
                    // dev_type is WEnum<Type>; check for Keyboard variant
                    dev.is_keyboard = matches!(dev_type, wayland_client::WEnum::Value(river_input_device_v1::Type::Keyboard));
                }
            }
            river_input_device_v1::Event::Name { name } => {
                eprintln!("input_device name: {}", name);
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
                execute_action(state, &data.action, data.command.as_deref());
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
                execute_action(state, &data.action, data.command.as_deref());
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
fn execute_action(state: &mut AppState, action: &crate::types::Action, command: Option<&str>) {
    use crate::types::Action;
    match action {
        Action::None => {}
        Action::Spawn => {
            if let Some(cmd) = command {
                eprintln!("spawn: {}", cmd);
                unsafe {
                    match nix::unistd::fork() {
                        Ok(nix::unistd::ForkResult::Child) => {
                            nix::unistd::setsid().ok();
                            let cmd_c = std::ffi::CString::new(cmd).unwrap();
                            nix::unistd::execvp(
                                &std::ffi::CString::new("/bin/sh").unwrap(),
                                &[
                                    std::ffi::CString::new("sh").unwrap(),
                                    std::ffi::CString::new("-c").unwrap(),
                                    cmd_c,
                                ],
                            )
                            .ok();
                            libc::_exit(127);
                        }
                        Ok(nix::unistd::ForkResult::Parent { .. }) => {}
                        Err(_) => {}
                    }
                }
            }
        }
        Action::Close => {
            // Close the focused window
            if let Some(seat) = state.wm.seats.first() {
                if let Some(focused_id) = seat.focused_window_id {
                    if let Some(wp) = state.get_window_proxy(focused_id) {
                        wp.river_window.close();
                    }
                }
            }
        }
        Action::FocusNext => {
            // Focus the next visible window (wrapping)
            if let Some(seat) = state.wm.seats.iter_mut().find(|s| !s.removed) {
                let focused_id = seat.focused_window_id;
                let visible_ids: Vec<u64> = state
                    .wm
                    .windows
                    .iter()
                    .filter(|w| (w.tags & state.wm.active_tags) != 0 && !w.closed)
                    .map(|w| w.id)
                    .collect();
                if let Some(fid) = focused_id {
                    if let Some(idx) = visible_ids.iter().position(|id| *id == fid) {
                        let next_idx = (idx + 1) % visible_ids.len();
                        seat.focused_window_id = Some(visible_ids[next_idx]);
                        state.wm.needs_render = true;
                    }
                }
            }
        }
        Action::Move => {
            // TODO: pointer move
        }
        Action::Resize => {
            // TODO: pointer resize
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
                    if let Some(window) = state.wm.get_window_mut(focused_id) {
                        if window.tiling_mode == TilingMode::Fullscreen {
                            window.tiling_mode = TilingMode::Cascade; // TODO: get_mode_for_window
                        } else {
                            window.tiling_mode = TilingMode::Fullscreen;
                        }
                        window.mode_locked = true;
                        state.wm.needs_render = true;
                    }
                }
            }
            if let Some(ref wm) = state.window_manager {
                wm.manage_dirty();
            }
        }
        Action::LayoutNext => {
            let cycle = [TilingMode::Cascade, TilingMode::Grid, TilingMode::Vsplit, TilingMode::Hsplit];
            let current = state.wm.global_layout;
            let next = cycle
                .iter()
                .position(|m| *m == current)
                .map(|i| cycle[(i + 1) % cycle.len()])
                .unwrap_or(TilingMode::Cascade);
            state.wm.global_layout = next;
            eprintln!("layout-next: global layout is now {}", next.as_str());
            state.wm.needs_render = true;
            if let Some(ref wm) = state.window_manager {
                wm.manage_dirty();
            }
        }
        Action::Reload => {
            // TODO: implement reload (re-run config)
            eprintln!("reload: not yet implemented");
        }
        Action::Restart => {
            crate::restart::wm_restart();
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
            state.wm.needs_render = true;
            if let Some(ref wm) = state.window_manager {
                wm.manage_dirty();
            }
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
            state.wm.needs_render = true;
            if let Some(ref wm) = state.window_manager {
                wm.manage_dirty();
            }
        }
        Action::SetTag1 | Action::SetTag2 | Action::SetTag3 | Action::SetTag4 => {
            let tag = match action {
                Action::SetTag1 => 1,
                Action::SetTag2 => 2,
                Action::SetTag3 => 3,
                Action::SetTag4 => 4,
                _ => return,
            };
            if let Some(seat) = state.wm.seats.first() {
                if let Some(focused_id) = seat.focused_window_id {
                    if let Some(window) = state.wm.get_window_mut(focused_id) {
                        window.tags = 1 << (tag - 1);
                        state.wm.needs_render = true;
                    }
                }
            }
            if let Some(ref wm) = state.window_manager {
                wm.manage_dirty();
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
            let match_app = rule.app_id_pattern == "*"
                || window
                    .app_id
                    .as_deref()
                    .map_or(false, |aid| aid.contains(&rule.app_id_pattern));
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
    for pb in &bindings {
        for (_sid, sp) in &state.seat_proxies {
            if let Some(ref xb) = state.xkb_bindings {
                if let Some(ref _xbs) = sp.xkb_bindings_seat {
                    let modifiers = u32_to_modifiers(pb.mods);
                    let binding_data = BindingUserData {
                        action: pb.action.clone(),
                        command: pb.command.clone(),
                    };
                    let binding =
                        xb.get_xkb_binding(&sp.river_seat, pb.keysym, modifiers, qhandle, binding_data);
                    binding.enable();
                }
            }
        }
    }

    // Apply pointer bindings to each seat
    let ptr_bindings: Vec<_> = state.wm.pending_pointer_bindings.drain(..).collect();
    for ppb in &ptr_bindings {
        for (_sid, sp) in &state.seat_proxies {
            let modifiers = u32_to_modifiers(ppb.mods);
            let binding_data = BindingUserData {
                action: ppb.action.clone(),
                command: None,
            };
            let _pb = sp
                .river_seat
                .get_pointer_binding(ppb.button, modifiers, qhandle, binding_data);
        }
    }
}

/// Connect to the Wayland display and set up the event queue.
/// Returns the Connection, EventQueue, and AppState after the initial registry roundtrip.
pub fn wayland_init() -> Result<(Connection, EventQueue<AppState>, AppState), String> {
    let conn = Connection::connect_to_env()
        .map_err(|e| format!("failed to connect to Wayland: {:?}", e))?;

    let mut event_queue = conn.new_event_queue::<AppState>();
    let qh = event_queue.handle();

    let _registry = conn.display().get_registry(&qh, RegistryData);

    // Do initial roundtrip to receive global events and bind protocols
    let mut state = AppState::new();
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| format!("initial roundtrip failed: {:?}", e))?;

    // Check we got the required protocols
    if !state.has_window_manager {
        return Err("river_window_manager_v1 not available".to_string());
    }
    if !state.has_xkb_bindings {
        return Err("river_xkb_bindings_v1 not available".to_string());
    }

    // Second roundtrip for input device events
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| format!("second roundtrip failed: {:?}", e))?;

    eprintln!("clearwm: Wayland connection established");

    Ok((conn, event_queue, state))
}
