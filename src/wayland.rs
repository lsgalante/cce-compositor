// Wayland display connection, registry, and event dispatch for clearwm

use wayland_client::{
    event_created_child, protocol::wl_registry, Connection, Dispatch, EventQueue, Proxy,
    QueueHandle,
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
use crate::protocol::wlr_output_management::client::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

use crate::types::{Action, BindingUserData, Output, Seat, TilingMode, Window, WindowManager};

// Interface name constants (from river protocol XML)
const IFACE_WINDOW_MANAGER: &str = "river_window_manager_v1";
const IFACE_XKB_BINDINGS: &str = "river_xkb_bindings_v1";
const IFACE_LAYER_SHELL: &str = "river_layer_shell_v1";
const IFACE_INPUT_MANAGER: &str = "river_input_manager_v1";
const IFACE_WLR_OUTPUT_MANAGER: &str = "zwlr_output_manager_v1";

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
                } else if interface == IFACE_WLR_OUTPUT_MANAGER {
                    eprintln!("registry: binding {}", IFACE_WLR_OUTPUT_MANAGER);
                    let om: ZwlrOutputManagerV1 =
                        registry.bind::<ZwlrOutputManagerV1, _, _>(name, 4, qhandle, ());
                    state.output_manager = Some(om);
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
                    crate::restart::wm_restart();
                }
            }

            river_window_manager_v1::Event::ManageStart => {
                let ms_start = std::time::Instant::now();
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

                // Assign tiling modes to windows based on mode_rules / tag_layouts / global_layout.
                // Must happen before manage_windows so tiling computation uses the correct modes.
                crate::wm::assign_window_modes(&mut state.wm);

                // Window management: set position + propose dimensions.
                // These modify window management state and can ONLY be called
                // during a manage sequence (per River protocol spec).
                crate::wm::manage_windows(state, qhandle);

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
                                    eprintln!("[focus] calling focus_window for id={}", focused_id);
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

                // Execute pending actions from key/pointer bindings (tinyrwm pattern).
                // Like tinyrwm, we defer action execution to ManageStart so all
                // state mutations happen during the manage sequence.
                // Collect pending actions first to avoid borrow checker issues
                // (execute_action borrows state mutably).
                let pending: Vec<(Action, Option<String>)> = state
                    .wm
                    .seats
                    .iter_mut()
                    .filter_map(|seat| {
                        if seat.pending_action != Action::None {
                            let action = seat.pending_action;
                            let command = seat.pending_command.take();
                            seat.pending_action = Action::None;
                            Some((action, command))
                        } else {
                            None
                        }
                    })
                    .collect();
                for (action, command) in pending {
                    execute_action(state, &action, command.as_deref());
                }

                wm_proxy.manage_finish();
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

                    // Raise the focused window to the top of the visual stack.
                    // place_top() modifies rendering state and must be called
                    // during a render sequence.
                    if let Some(seat) = state.wm.seats.iter().find(|s| !s.removed) {
                        if let Some(focused_id) = seat.focused_window_id {
                            if let Some(node) = state
                                .window_nodes
                                .iter()
                                .find(|(id, _)| *id == focused_id)
                                .map(|(_, n)| n)
                            {
                                eprintln!(
                                    "[render] place_top for focused window id={}",
                                    focused_id
                                );
                                node.place_top();
                            }
                        }
                    }

                    state.wm.needs_render = false;
                }

                wm_proxy.render_finish();
                eprintln!("[render] render_finish #{} queued", state.render_count);

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

                    // Push the same data through the status socket so waybar
                    // gets updates in real-time without needing signal-based pkill.
                    if let Some(ref sender) = state.status_sender {
                        let update = crate::status_server::build_status_update(&state.wm);
                        sender.send(update);
                    }

                    state.wm.needs_status_update = false;
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

            river_seat_v1::Event::WindowInteraction {
                window: river_window,
            } => {
                if let Some(wid) = state.window_id_for_proxy(&river_window) {
                    if let Some(seat) = state.wm.seats.iter_mut().find(|s| s.id == sid) {
                        eprintln!(
                            "[focus] WindowInteraction: seat={} focused_window_id={} -> {}",
                            sid,
                            seat.focused_window_id.unwrap_or(0),
                            wid
                        );
                        seat.focused_window_id = Some(wid);
                        // Move clicked window to front of cascade stack
                        state.wm.move_window_to_end(wid);
                        state.wm.needs_render = true;
                        state.wm.needs_focus = true;
                        state.wm.needs_status_update = true;
                    }
                }
            }

            river_seat_v1::Event::PointerEnter {
                window: river_window,
            } => {
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
                    dev.is_keyboard = matches!(
                        dev_type,
                        wayland_client::WEnum::Value(river_input_device_v1::Type::Keyboard)
                    );
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
fn execute_action(state: &mut AppState, action: &crate::types::Action, command: Option<&str>) {
    use crate::types::Action;
    match action {
        Action::None => {}
        Action::Spawn => {
            if let Some(cmd) = command {
                eprintln!("spawn: {}", cmd);
                // Close inherited FDs > 2 in the child so that spawned
                // Wayland clients (fuzzel, etc.) never accidentally read
                // from clearwm's Wayland socket fd. This prevents protocol
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
                            let max_fd =
                                libc::sysconf(libc::_SC_OPEN_MAX) as libc::c_int;
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
            // Mark the focused window for closing. The actual close() call
            // happens during the ManageStart sequence, since close()
            // modifies window management state and can only be called during
            // a manage sequence.
            if let Some(seat) = state.wm.seats.first() {
                if let Some(focused_id) = seat.focused_window_id {
                    if let Some(window) = state.wm.get_window_mut(focused_id) {
                        window.closed = true;
                    }
                    // Shift focus to the next visible window (excluding the one we just closed)
                    let visible_ids: Vec<u64> = state
                        .wm
                        .windows
                        .iter()
                        .filter(|w| {
                            (w.tags & state.wm.active_tags) != 0 && !w.closed && w.id != focused_id
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
        Action::FocusNext => {
            // Focus the next visible window (wrapping) and move it to the
            // front of the cascade stack (end of windows vector).
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
                }
            }
            // Trigger a manage sequence so focus_window() is called
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
                            decoration_hint: 3,
                            presentation_hint: 0,
                            tiling_mode: TilingMode::Fullscreen,
                            mode_locked,
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
                TilingMode::Vsplit,
                TilingMode::Hsplit,
            ];
            let current = state.wm.global_layout;
            let next = cycle
                .iter()
                .position(|m| *m == current)
                .map(|i| cycle[(i + 1) % cycle.len()])
                .unwrap_or(TilingMode::Cascade);
            state.wm.global_layout = next;
            eprintln!("layout-next: global layout is now {}", next.as_str());

            // Unlock windows that got their mode from the layout (not from mode_rules
            // or manual set-mode) so assign_window_modes will reassign them.
            // Windows with mode_locked=true were explicitly set by the user and stay.
            // Windows matched by mode_rules will get reassigned to the same rule mode.
            // Only windows that fell through to global_layout will change.

            state.wm.needs_render = true;
            state.wm.needs_status_update = true;
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

            // Reassign focus to a visible window on the new tag
            let visible_ids: Vec<u64> = state
                .wm
                .windows
                .iter()
                .filter(|w| (w.tags & state.wm.active_tags) != 0 && !w.closed)
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
                    .filter(|w| (w.tags & state.wm.active_tags) != 0 && !w.closed)
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
                let window_left_active_tag = state
                    .wm
                    .get_window_mut(focused_id)
                    .map_or(false, |window| {
                        window.tags = 1 << (tag - 1);
                        (window.tags & active_tags) == 0
                    });

                // If the window is no longer on an active tag, shift focus
                if window_left_active_tag {
                    let visible_ids: Vec<u64> = state
                        .wm
                        .windows
                        .iter()
                        .filter(|w| (w.tags & state.wm.active_tags) != 0 && !w.closed)
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
        qhandle: &QueueHandle<Self>,
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

    let _registry = conn.display().get_registry(&qh, RegistryData);

    // Do initial roundtrip to receive global events and bind protocols
    let mut state = AppState::new();
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

    eprintln!("clearwm: Wayland connection established");

    Ok((conn, event_queue, state))
}
