// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, WlList, wl_listener_remove};
use crate::slotmap::SlotMap;
use std::hash::{Hash, Hasher};

pub use crate::window::Window;
pub use crate::shell_surface::ShellSurface;

pub use crate::xwayland_override_redirect::XwaylandOverrideRedirect;

/// Working directory of the shell running inside a foot window.
///
/// foot's own process cwd never follows `cd` — it stays at its launch dir for
/// the window's whole life. The live directory the user is actually in lives in
/// foot's child (the shell it spawned for that window). We read the first child
/// and return its `/proc/<pid>/cwd`. Returns `None` if it can't be read.
fn foot_shell_cwd(foot_pid: i32) -> Option<String> {
    let children =
        std::fs::read_to_string(format!("/proc/{0}/task/{0}/children", foot_pid)).ok()?;
    let child: i32 = children.split_whitespace().next()?.parse().ok()?;
    let cwd = std::fs::read_link(format!("/proc/{}/cwd", child)).ok()?;
    Some(cwd.to_string_lossy().into_owned())
}

/// The bare program name a window should be restored by, when the binary it
/// is running is NOT what its name resolves to on `PATH`.
///
/// An `exec` wrapper leaves no trace in `/proc`: `~/.local/bin/inkscape`
/// (`exec /usr/bin/inkscape "$@"`, the GDK_SCALE fix) shows up in the
/// window's cmdline as `/usr/bin/inkscape`, and a restore that replays that
/// path starts the program without its wrapper. Desktop entries and the
/// launcher run bare names, so the first `PATH` hit for the name IS how the
/// user's environment launches the program. When that hit is a different
/// file from the one running, record the name and let the restore's
/// `sh -c` resolve it the same way. Returns `None` for a relative argv[0],
/// a binary no longer on disk, or a name whose first `PATH` hit is the very
/// same file (through any symlink) — there the absolute path is already the
/// truth, and keeping it means a later `PATH` change cannot redirect it.
fn path_shadowed_name(argv0: &str, path_var: &str) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    if !argv0.starts_with('/') {
        return None;
    }
    let exe = std::path::Path::new(argv0);
    let name = exe.file_name()?.to_str()?;
    let real = std::fs::canonicalize(exe).ok()?;
    for dir in path_var.split(':').filter(|d| !d.is_empty()) {
        let candidate = std::path::Path::new(dir).join(name);
        let Ok(meta) = std::fs::metadata(&candidate) else { continue };
        if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
            continue;
        }
        // First executable hit decides, as `sh` would decide it.
        let candidate_real = std::fs::canonicalize(&candidate).unwrap_or(candidate);
        return if candidate_real == real { None } else { Some(name.to_string()) };
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowManagerState {
    Idle,
    Manage,
    InflightConfigures(u32),
    Render,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowManagerMode {
    Normal,
    Overview,
}

pub struct WindowManagerScheduled {
    pub dirty: bool,
    pub dirty_lazy: bool,
    pub output_config: *mut ffi::wlr_output_configuration_v1,
}

pub struct WindowManagerSent {
    pub session_locked: bool,
    pub outputs: ffi::wl_list,
    pub output_config: *mut ffi::wlr_output_configuration_v1,
    pub seats: ffi::wl_list,
}

pub struct WindowManagerRenderingScheduled {
    pub dirty: bool,
}

pub struct WindowManagerRenderingRequested {
    pub list: ffi::wl_list,
    pub order_hash: u64,
}

pub use crate::policy::state::{SavedState, SavedWindowState};

pub struct WindowManager {
    pub server: *mut Server,
    pub global: *mut ffi::wl_global,
    pub server_destroy: ffi::wl_listener,
    pub object: *mut ffi::wl_resource,
    pub state: WindowManagerState,
    pub windows: SlotMap<*mut Window>,
    pub focus_history: Vec<*mut Window>,
    pub scheduled: WindowManagerScheduled,
    pub sent: WindowManagerSent,
    pub rendering_scheduled: WindowManagerRenderingScheduled,
    pub rendering_requested: WindowManagerRenderingRequested,
    pub dirty_idle: *mut ffi::wl_event_source,
    pub timeout: *mut ffi::wl_event_source,
    pub desk_pan_x: f64,
    pub desk_pan_y: f64,
    pub desk_zoom: f64,
    pub mode: WindowManagerMode,
    /// Camera reaction when the focused window goes away (config
    /// `window_manager.on_app_exit`).
    pub on_app_exit: crate::config::OnAppExit,
    /// From the last arrange plan: false while a live grid client covers
    /// the desktop, so `draw_grid` keeps only the backdrop (and labels).
    pub grid_cells_enabled: bool,
    pub layout: crate::config::Layout,
    pub mode_rules: Vec<crate::config::ModeRule>,
    pub keybinds: Vec<crate::config::Keybind>,
    pub pointer_binds: Vec<crate::config::PointerBind>,
    pub gesture_binds: Vec<crate::config::GestureBind>,
    pub ipc_rx: Option<std::sync::mpsc::Receiver<crate::ipc_server::IpcRequest>>,
    /// The IPC thread's wake eventfd as a wl_event_loop fd source: fires once
    /// per queued request, so the drain runs only when there is something to
    /// drain (it was a 10 ms polling timer before).
    pub ipc_source: *mut ffi::wl_event_source,
    pub ipc_wake: Option<std::sync::Arc<std::os::fd::OwnedFd>>,
    /// One-shot timer behind `schedule_save_state`: the state file is written
    /// at most once per `SAVE_STATE_DELAY_MS`, not once per transaction.
    pub save_state_timer: *mut ffi::wl_event_source,
    pub save_state_pending: bool,
    /// Bumped at the end of every transaction; outputs compare it to know
    /// whether window geometry can have moved since they last measured the
    /// status backdrops.
    pub layout_epoch: u64,
    /// The stream hub's wake eventfd as an event source: a new subscriber
    /// arms `stream_timer`, which otherwise does not tick at all.
    pub stream_source: *mut ffi::wl_event_source,
    /// Minute tick for the traveling light_source segment: re-arranges so
    /// its perimeter position follows the time of day.
    pub sun_timer: *mut ffi::wl_event_source,
    /// Window-stream subscribers (cce-remote's live view); frames are
    /// produced by `handle_stream_timer` when a subscribed window is dirty.
    pub stream_hub: Option<crate::stream_server::StreamHub>,
    pub stream_timer: *mut ffi::wl_event_source,
    /// A full-output/region screenshot parked for the next composited frame
    /// (`ccectl screenshot`); consumed by `Output::render_and_commit`.
    pub pending_screenshot: Option<crate::screenshot::PendingScreenshot>,
    /// The reply channel of the IPC command currently being dispatched, so a
    /// command that cannot answer yet can carry it away and answer later
    /// (only `screenshot` does). Set by `handle_ipc_event` around the
    /// dispatch; if it is still here afterwards, the command answered
    /// synchronously and the drain sends its return value.
    pub pending_ipc_reply: Option<std::sync::mpsc::Sender<String>>,
    pub startup: Vec<crate::config::StartupConfig>,
    pub startup_pids: Vec<(crate::config::StartupConfig, nix::unistd::Pid)>,
    pub status_sender: Option<crate::status_server::StatusSender>,
    pub output_scale: f32,
    /// Xwayland sees a physical-pixel screen and X11 surfaces draw at
    /// 1/scale (see `WindowManagerConfig::xwayland_hidpi`).
    pub xwayland_hidpi: bool,
    /// X11 windows kept in the logical world while `xwayland_hidpi` is on
    /// (see `xwayland_window::x11_scale_for`).
    pub xwayland_hidpi_except: Vec<String>,
    /// Trackpad-to-view-drag emulation (see `cursor::ViewDrag`).
    pub touchpad_view_apps: Vec<String>,
    pub touchpad_view_swipe_tumble: bool,
    pub touchpad_view_sensitivity: f64,
    pub touchpad_view_invert: bool,
    /// Live override-redirect X11 surfaces (menus, tooltips, combo lists),
    /// so the per-frame pass can re-apply their 1/scale dest size — the
    /// scene's own commit listener resets it on every commit.
    pub override_redirects: Vec<*mut XwaylandOverrideRedirect>,
    pub display: std::collections::HashMap<String, f64>,
    pub input_rules: Vec<crate::config::InputDeviceConfigRule>,
    pub input_config: crate::config::InputConfig,
    pub last_status_update: std::cell::RefCell<Option<crate::status_server::StatusUpdate>>,
    /// What each status segment is composited over, by app_id — measured in
    /// the output's render pass (`Output::measure_status_backdrops`, which is
    /// where the frame's grid geometry already is) and read back out by
    /// `build_status_update`. See [`crate::backdrop`].
    pub status_backdrops: std::cell::RefCell<Vec<(String, u8, u8)>>,
    pub status_hide_mode: bool,
    pub adjust_position_mode: bool,
    /// xkb modifier mask currently held via injected `key-down` (see the ipc handler):
    /// OR'd over the device state on every synthetic modifiers notify so clients see
    /// ctrl/shift/alt/super combos from injection like they would from hardware.
    pub injected_key_mods: u32,
    pub restore_queue: Vec<SavedWindowState>,
    pub last_window_states: Vec<SavedWindowState>,
    /// The JSON last successfully written to `state.json`. `save_state` runs at
    /// the end of every transaction commit, and a 1 Hz status-bar clock tick is
    /// enough to run a transaction — so an idle desktop rewrote ~21KB to disk
    /// every second for state that never changed. Skipping the write when the
    /// serialization is byte-identical keeps the file exactly as current as
    /// before while making an idle session silent on disk. `None` until the
    /// first write, so a fresh start always writes once.
    pub last_saved_state_json: Option<String>,
    /// One-shot placement hints (`place-next <app_id> <x> <y>` over IPC):
    /// the next map of a floating toplevel with this app_id lands near the
    /// given layout position instead of its remembered spot — widget-spawned
    /// pickers open at the control that launched them. (app_id, screen x/y,
    /// registered-at; entries expire unconsumed after a few seconds.)
    /// One-shot placement hints: `(key, x, y, cell_anchored, when)`. `key` is
    /// matched loosely against a mapping window's app_id (see
    /// `take_pending_placement`), because a launcher knows the command it ran,
    /// not the app_id the client will choose.
    pub pending_placements: Vec<(String, f64, f64, bool, std::time::Instant)>,
    pub shutting_down: bool,
    pub target_desk_pan_x: Option<f64>,
    pub target_desk_pan_y: Option<f64>,
    /// Eased alongside the pan targets by the animation tick — the overview
    /// enter/exit transition (`SetCamera { animate: true }`) when no speed
    /// ramp is configured.
    pub target_desk_zoom: Option<f64>,
    /// Duration-based camera transition driven by the configured
    /// `desktop { overview_ramp= overview_ms= }` speed profile. When active
    /// it owns the camera; the exponential targets above stay clear.
    pub camera_ramp_anim: Option<CameraRampAnim>,
    /// When the camera animation last stepped — the exponential eases below
    /// are frame-rate independent, so a late timer tick takes a
    /// proportionally larger step instead of a stutter. `None` while the
    /// timer is idle, so the first step after arming measures from the arm.
    /// The camera's frame clock: the presentation instant the last step
    /// animated to (CLOCK_MONOTONIC ns), so each step's dt is measured
    /// vblank to vblank, not callback to callback.
    pub anim_last_tick: Option<u64>,
    /// Kinetic desktop pan after a trackpad flick: virtual units/s, decayed
    /// by `input.scroll_friction` each tick until it stalls. Zero = no coast.
    pub pan_coast_vx: f64,
    pub pan_coast_vy: f64,
    /// Output-local point a `target_desk_zoom` glide pivots on (ctrl+super
    /// wheel zoom): each step re-derives the pan so the virtual point under
    /// the cursor stays put throughout, not just at the end.
    pub zoom_anchor: Option<(f64, f64)>,
    /// Live finger-pan velocity (virtual units/s, `[x, y]`) while a trackpad
    /// gesture is panning the desktop; zero otherwise. Grid patches use it
    /// to prefetch toward where the gesture is heading.
    pub pan_finger_v: [f64; 2],
    /// A camera animation (wheel glide, coast, zoom target, overview ramp) is
    /// live: `step_camera_frame` advances it once per output frame, and the
    /// watchdog timer keeps frames coming while it is set.
    pub camera_anim_active: bool,
    /// Finger-pan motion (virtual units, `[x, y]`) queued since the last
    /// frame. Trackpad axis events used to relayout the desktop per event
    /// (twice per sample for a diagonal); they now accumulate here and are
    /// applied once, at the output frame, so the on-screen step lands on
    /// the vblank instead of whenever the last event happened to arrive.
    pub pan_pending: [f64; 2],
    /// An interactive move/resize has pointer motion the client has not
    /// been configured for yet. The seat op recomputes the dragged window's
    /// geometry on every pointer event (cheap, and the arrange pass reads
    /// the op state), but it used to also send the client a configure and
    /// run a manage pass per event — a fast mouse handed the client several
    /// sizes per frame, most rendered and never shown. Now it queues one
    /// frame and `step_op_frame` does both once, on the vblank.
    pub op_frame_pending: bool,
    pub animation_timer: *mut ffi::wl_event_source,
    /// Edge auto-pan velocity during an interactive move/resize, in SCREEN
    /// px/s (the tick divides by zoom). Written by `Seat::update_edge_pan`
    /// on every op motion; both zero when the cursor is outside the bands.
    pub edge_pan_vx: f64,
    pub edge_pan_vy: f64,
    pub edge_pan_timer: *mut ffi::wl_event_source,
    pub has_restored_focused_window: bool,
    pub restored_focused_window_mapped: bool,
    /// Dim frames at restored windows' saved geometry while their programs
    /// relaunch (see create_restore_placeholders).
    pub restore_placeholders: Vec<RestorePlaceholder>,
    pub restore_placeholder_timer: *mut ffi::wl_event_source,
    /// True after the first deliberate input (key or button press) of the
    /// session. Until then the session is still "settling" from restore:
    /// windows that map unbidden (autostarts like keepassxc) must not steal
    /// focus from the restored session's focused window.
    pub startup_input_seen: bool,
    /// app_ids whose window went away without the compositor ever asking it
    /// to close, and when. A client that loses its Wayland connection lands
    /// here and reappears a moment later having rebuilt its surface; see
    /// `take_recent_vanish`.
    pub vanished_windows: Vec<(String, std::time::Instant)>,
    pub last_viewport_zoom: f64,
    pub last_viewport_pan_x: f64,
    pub last_viewport_pan_y: f64,
    /// True while a viewport zoom/pan gesture is in progress (blur suppressed).
    /// Cleared by `viewport_settle_timer` a short debounce after the last motion,
    /// so blur restores exactly once when the gesture truly stops.
    pub viewport_is_active: bool,
    pub viewport_settle_timer: *mut ffi::wl_event_source,
    pub clean_exit_in_progress: bool,
    pub clean_exit_timer: *mut ffi::wl_event_source,
    /// Entries of the last written state snapshot whose windows a since-
    /// cancelled clean exit had already closed (see `cancel_clean_exit`).
    /// Merged into every later snapshot until the app is relaunched, so the
    /// apps a cancelled logout took down still come back next login.
    pub exit_orphans: Vec<SavedWindowState>,
    /// The `windows` list of the last state snapshot actually written.
    pub last_saved_windows: Vec<SavedWindowState>,
    /// Drives the hover fade on window borders (see `Window::border_reveal`).
    pub border_fade_timer: *mut ffi::wl_event_source,
    /// Whether the fade timer is currently armed, so re-arming while a fade is
    /// already running doesn't restart it and double the step rate.
    pub border_fade_running: bool,
    /// `window_manager.center_on_spawn`: whether a newly spawned window pulls the viewport
    /// over to it when it takes focus. Off, the desk stays put and the window opens wherever
    /// the layout placed it. Focus-follow panning between EXISTING windows is unaffected.
    pub center_on_spawn: bool,
    /// `window_manager.rounded_apps`: extra app_ids that get the decorated-window
    /// treatment (rounded corner clip, blur-behind, shadow) alongside cce-* apps
    /// and SSD requesters.
    pub rounded_apps: Vec<String>,
    pub bevel_apps: Vec<String>,
}

/// `CCE_DIRTY_BACKTRACE=1` — who called `dirty_windowing`. Separate from the
/// log level because the capture is expensive enough to distort what it measures.
fn dirty_backtrace_debug() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCE_DIRTY_BACKTRACE").is_some())
}

/// `CCE_DIRTY_TRACE=1` — one debug line per `dirty_windowing` /
/// `dirty_rendering` call naming the call site (`#[track_caller]`, so it
/// costs nothing when off). The cheap way to answer "what keeps the window
/// manager running transactions on an idle desktop".
fn dirty_trace() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCE_DIRTY_TRACE").is_some())
}

/// How long after a transaction the state file is written. One write per
/// second is plenty for a file whose job is surviving a crash, and the
/// per-window `/proc` reads in `save_state` are far too heavy to run on
/// every transaction (a drag is one transaction per pointer event).
const SAVE_STATE_DELAY_MS: i32 = 1000;

/// `CCE_ARRANGE_DEBUG=1` — the arrange pass and its per-window dump. A status
/// bar commit runs a full arrange every second, so at debug level this alone
/// wrote ~15-20 lines/second (and allocated a title + app_id String per window
/// per pass) on an otherwise idle desktop.
pub(crate) fn arrange_debug() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCE_ARRANGE_DEBUG").is_some())
}

/// `CCE_MANAGE_DEBUG=1` — per-stage timing of the manage/render transaction.
/// A 1 Hz status-bar clock commit runs a full transaction, and the compositor
/// burns ~17% of a core on an idle desktop; gating and removing the logging and
/// the state write did not move that number, so the work is in the transaction
/// itself. One line per phase, emitted once per transaction.
pub(crate) fn manage_debug() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCE_MANAGE_DEBUG").is_some())
}

/// How long after a window vanishes unbidden a re-map by the same app_id
/// still counts as that client reconnecting rather than a fresh launch.
/// cce-ui retries 200ms after losing its connection and backs off from there,
/// so a few seconds covers the early attempts; keeping it short is what stops
/// a deliberate close-then-relaunch from being mistaken for one.
const RECONNECT_FOCUS_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Does a `rounded_apps` / `bevel_apps` config pattern match this app_id?
///
/// Case-insensitive, and a pattern containing `*` is a glob (`*` stands for any
/// run of characters, including none). A pattern without `*` is still an exact
/// comparison, so existing configs keep working unchanged.
///
/// Both of those exist because **an app_id is not a stable identifier**, and an
/// exact allowlist fails silently when one changes. Claude Desktop shipped as
/// `claude-desktop` and renamed itself to `com.anthropic.Claude`; the config
/// entry stopped matching, and the window lost its rounded corners, blur,
/// shadow and bevel at once — with no error, no log line, and nothing in
/// `ccectl windows` to point at. `rounded_apps "*claude*"` survives that rename,
/// and the `decorated=`/`beveled=` fields in `windows` make the outcome
/// visible either way.
///
/// Deliberately NOT a general glob: no `?`, no character classes. An app_id is
/// a flat identifier and `*` covers the rename cases; the rest is surface for
/// a pattern to match something nobody intended.
/// Whether a surface-local point lies in one of an app's view regions
/// (`Window::view_regions`). An empty list matches nothing: an app that
/// has told us it currently shows no view pane gets no view drag at all,
/// which is different from an app that never said anything (`None`).
pub fn point_in_view_regions(regions: &[[f64; 4]], x: f64, y: f64) -> bool {
    regions.iter().any(|[rx, ry, rw, rh]| x >= *rx && y >= *ry && x < rx + rw && y < ry + rh)
}

/// The mapped window an IPC command names: `x11:<id>` is the X11 window
/// id an Xwayland client knows itself by (what `windows --json` reports as
/// `x11`), a bare number is the compositor's own id, anything else an
/// app_id. Only the first form is unambiguous for an app with several
/// windows, which is why a client that publishes per-window state should
/// use it.
pub unsafe fn window_by_query(windows: impl Iterator<Item = *mut Window>, query: &str) -> Option<*mut Window> {
    let x11 = query.strip_prefix("x11:").and_then(|v| v.parse::<u32>().ok());
    let id = query.parse::<u32>().ok();
    windows.into_iter().find(|&w| {
        if w.is_null() || (*w).closed || !matches!((*w).state, crate::window::WindowState::Mapped) {
            return false;
        }
        if let Some(x11) = x11 {
            return match (*w).impl_type {
                crate::window::WindowImpl::Xwayland(xw) if !xw.is_null() => (*(*xw).xsurface).window_id == x11,
                _ => false,
            };
        }
        if let Some(id) = id {
            return (*w).ref_key.index == id;
        }
        (*w).get_app_id_string().map_or(false, |a| app_id_matches(query, &a))
    })
}

pub fn app_id_matches(pattern: &str, app_id: &str) -> bool {
    if !pattern.contains('*') {
        return pattern.eq_ignore_ascii_case(app_id);
    }
    let pattern = pattern.to_ascii_lowercase();
    let app_id = app_id.to_ascii_lowercase();
    // Segments between the stars. The first and last are anchored to the ends
    // of the app_id; the ones between float, consuming left to right.
    let segments: Vec<&str> = pattern.split('*').collect();
    let last = segments.len() - 1;
    let mut rest = app_id.as_str();
    for (i, seg) in segments.iter().enumerate() {
        if seg.is_empty() {
            continue;
        }
        if i == 0 {
            match rest.strip_prefix(seg) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == last {
            // ends_with on what is LEFT, not on the whole app_id: an anchored
            // tail must not re-consume characters an earlier segment already
            // matched ("ab*ab" must not match "ab").
            return rest.ends_with(seg);
        } else {
            match rest.find(seg) {
                Some(at) => rest = &rest[at + seg.len()..],
                None => return false,
            }
        }
    }
    true
}

impl WindowManager {
    pub unsafe fn init(&mut self) -> Result<(), ()> {
        // This is a stub for the 0-arg struct instantiation.
        // We will call the real initialization with the server parameter.
        ffi::wl_list_init(&mut self.sent.outputs);
        self.scheduled.output_config = std::ptr::null_mut();
        self.sent.output_config = std::ptr::null_mut();
        self.output_scale = 1.0;
        self.xwayland_hidpi = true;
        self.xwayland_hidpi_except = Vec::new();
        self.touchpad_view_apps = Vec::new();
        self.touchpad_view_swipe_tumble = false;
        self.touchpad_view_sensitivity = 1.0;
        self.touchpad_view_invert = false;
        self.display = std::collections::HashMap::new();
        self.input_rules = Vec::new();
        self.input_config = crate::config::InputConfig::default();
        self.mode = WindowManagerMode::Normal;
        Ok(())
    }

    pub unsafe fn init_with_server(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        self.global = std::ptr::null_mut();
        self.object = std::ptr::null_mut();
        self.state = WindowManagerState::Idle;
        self.windows = SlotMap::new();
        self.focus_history = Vec::new();
        self.scheduled = WindowManagerScheduled {
            dirty: false,
            dirty_lazy: false,
            output_config: std::ptr::null_mut(),
        };
        self.sent = WindowManagerSent {
            session_locked: false,
            outputs: std::mem::zeroed(),
            output_config: std::ptr::null_mut(),
            seats: std::mem::zeroed(),
        };
        self.rendering_scheduled = WindowManagerRenderingScheduled {
            dirty: false,
        };
        self.rendering_requested = WindowManagerRenderingRequested {
            list: std::mem::zeroed(),
            order_hash: 0,
        };
        self.dirty_idle = std::ptr::null_mut();
        self.desk_pan_x = 0.0;
        self.desk_pan_y = 0.0;
        self.target_desk_pan_x = None;
        self.target_desk_pan_y = None;
        self.target_desk_zoom = None;
        self.camera_ramp_anim = None;
        self.anim_last_tick = None;
        self.pan_coast_vx = 0.0;
        self.pan_coast_vy = 0.0;
        self.zoom_anchor = None;
        self.pan_finger_v = [0.0, 0.0];
        self.camera_anim_active = false;
        self.pan_pending = [0.0, 0.0];
        self.op_frame_pending = false;
        self.animation_timer = std::ptr::null_mut();
        self.edge_pan_vx = 0.0;
        self.edge_pan_vy = 0.0;
        self.edge_pan_timer = std::ptr::null_mut();
        self.viewport_is_active = false;
        self.viewport_settle_timer = std::ptr::null_mut();
        self.desk_zoom = 1.0;
        self.pending_screenshot = None;
        self.pending_ipc_reply = None;
        self.mode = WindowManagerMode::Normal;
        self.on_app_exit = crate::config::OnAppExit::FocusPrevious;
        self.grid_cells_enabled = true;
        self.restore_queue = Vec::new();
        self.last_window_states = Vec::new();
        self.exit_orphans = Vec::new();
        self.last_saved_windows = Vec::new();
        self.override_redirects = Vec::new();
        self.pending_placements = Vec::new();
        self.rounded_apps = Vec::new();
        self.bevel_apps = Vec::new();
        self.shutting_down = false;
        self.layout = crate::config::Layout::default();
        self.output_scale = 1.0;
        self.xwayland_hidpi = true;
        self.xwayland_hidpi_except = Vec::new();
        self.touchpad_view_apps = Vec::new();
        self.touchpad_view_swipe_tumble = false;
        self.touchpad_view_sensitivity = 1.0;
        self.touchpad_view_invert = false;
        self.display = std::collections::HashMap::new();
        self.has_restored_focused_window = false;
        self.restored_focused_window_mapped = false;
        self.restore_placeholders = Vec::new();
        self.restore_placeholder_timer = std::ptr::null_mut();
        self.startup_input_seen = false;
        self.vanished_windows = Vec::new();
        self.mode_rules = Vec::new();
        self.keybinds = Vec::new();
        self.pointer_binds = Vec::new();
        self.gesture_binds = Vec::new();
        self.ipc_rx = None;
        self.ipc_source = std::ptr::null_mut();
        self.ipc_wake = None;
        self.save_state_timer = std::ptr::null_mut();
        self.save_state_pending = false;
        self.layout_epoch = 0;
        self.stream_source = std::ptr::null_mut();
        self.sun_timer = std::ptr::null_mut();
        self.stream_hub = None;
        self.stream_timer = std::ptr::null_mut();
        self.startup = Vec::new();
        self.startup_pids = Vec::new();
        self.status_sender = None;
        self.input_rules = Vec::new();
        self.input_config = crate::config::InputConfig::default();
        self.last_status_update = std::cell::RefCell::new(None);
        self.status_backdrops = std::cell::RefCell::new(Vec::new());
        self.status_hide_mode = false;
        self.adjust_position_mode = false;
        self.injected_key_mods = 0;
        let _ = std::fs::remove_file("/tmp/cce-status-interface-adjust-mode");

        ffi::wl_list_init(&mut self.sent.outputs);
        ffi::wl_list_init(&mut self.sent.seats);
        ffi::wl_list_init(&mut self.rendering_requested.list);

        let event_loop = ffi::wl_display_get_event_loop((*server).wl_server);
        self.timeout = ffi::wl_event_loop_add_timer(event_loop, Some(handle_timeout), self as *mut WindowManager as *mut _);
        if self.timeout.is_null() {
            return Err("Failed to create timer event source");
        }

        self.ipc_rx = None;

        self.sun_timer = ffi::wl_event_loop_add_timer(event_loop, Some(handle_sun_timer), self as *mut WindowManager as *mut _);
        if self.sun_timer.is_null() {
            ffi::wl_event_source_remove(self.timeout);
            return Err("Failed to create sun timer event source");
        }
        ffi::wl_event_source_timer_update(self.sun_timer, 60_000);

        self.clean_exit_timer = ffi::wl_event_loop_add_timer(event_loop, Some(handle_clean_exit_timeout), self as *mut WindowManager as *mut _);
        if self.clean_exit_timer.is_null() {
            ffi::wl_event_source_remove(self.timeout);
            return Err("Failed to create clean exit timer event source");
        }
        self.clean_exit_in_progress = false;

        self.border_fade_timer =
            ffi::wl_event_loop_add_timer(event_loop, Some(handle_border_fade_tick), self as *mut WindowManager as *mut _);
        if self.border_fade_timer.is_null() {
            ffi::wl_event_source_remove(self.timeout);
            ffi::wl_event_source_remove(self.clean_exit_timer);
            return Err("Failed to create border fade timer event source");
        }
        self.border_fade_running = false;

        self.stream_timer = ffi::wl_event_loop_add_timer(event_loop, Some(handle_stream_timer), self as *mut WindowManager as *mut _);
        if self.stream_timer.is_null() {
            ffi::wl_event_source_remove(self.timeout);
            ffi::wl_event_source_remove(self.clean_exit_timer);
            ffi::wl_event_source_remove(self.border_fade_timer);
            return Err("Failed to create stream timer event source");
        }
        // Not armed here: `start_stream` arms it when a subscriber appears,
        // and `handle_stream_timer` lets it lapse when the last one leaves.

        // Default until the config is parsed (which happens after this init).
        self.center_on_spawn = true;

        self.global = ffi::wl_global_create(
            (*server).wl_server,
            &ffi::zcce_window_manager_v1_interface,
            // 7 = set_popover_region on toplevels (the in-surface menu
            // hint); 6 = grid support (toplevel v4: set_grid/grid_patch/
            // ack); 5 = set_utility exists on toplevels. Clients
            // feature-gate on the negotiated version, so one launched into
            // an older compositor degrades gracefully instead of dying on
            // an unknown opcode.
            7,
            self as *mut WindowManager as *mut _,
            Some(bind),
        );
        if self.global.is_null() {
            ffi::wl_event_source_remove(self.timeout);
            return Err("Failed to create zcce_window_manager_v1 global");
        }

        let server_destroy_ptr = &mut self.server_destroy as *mut ffi::wl_listener as *mut WlListener;
        (*server_destroy_ptr).notify = Some(handle_server_destroy);
        ffi::wl_display_add_destroy_listener((*server).wl_server, &mut self.server_destroy);

        Ok(())
    }

    pub unsafe fn load_state(&mut self, path: &str) {
        log::info!("Loading state from {}", path);
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(state) = serde_json::from_str::<SavedState>(&content) {
                self.desk_pan_x = state.desk_pan_x;
                self.desk_pan_y = state.desk_pan_y;
                self.desk_zoom = state.desk_zoom;
                self.set_mode(if (state.desk_zoom - 1.0).abs() > 0.001 { WindowManagerMode::Overview } else { WindowManagerMode::Normal });
                self.restore_queue = state.windows;
                self.last_window_states = state.last_window_states;
                // Geometry in the file was measured under the grid it
                // records; if this session's grid differs, put every Tiled
                // entry back on ITS SQUARES now, before placeholders and
                // restores consume the pixels. A pre-field file (grid: None)
                // has nothing to remap from and loads as-is.
                if let Some(g) = state.grid {
                    let current = self.layout.snap_params();
                    if !g.matches(&current) {
                        self.remap_saved_entries(&g.to_params(&current));
                    }
                }
                self.has_restored_focused_window = self.restore_queue.iter().any(|w| w.focused);
                self.restored_focused_window_mapped = false;
                log::info!(
                    "State loaded successfully. {} windows in restore queue, has_restored_focused_window={}.",
                    self.restore_queue.len(),
                    self.has_restored_focused_window
                );
                self.create_restore_placeholders();
            } else {
                log::error!("Failed to parse state JSON from {}", path);
            }
        } else {
            log::info!("State file not found or unreadable at {}, starting with empty state.", path);
        }
    }

    /// Bounding box of the TILED desk, virtual units: the union of the
    /// session's Tiled entries still waiting in `restore_queue` and every
    /// live Tiled window (mapped, or restored and about to map). `None` when
    /// there is no tiled window at all. Feeds `recalled_origin`'s on-desk
    /// exemption in `try_restore`, so a floating window remembered beside
    /// the tiled columns is not recalled into the view like a lost one.
    /// Minimized entries are skipped — a minimized window is nowhere on the
    /// desk to be beside.
    pub unsafe fn tiled_desk_bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        let mut extend = |x: f64, y: f64, w: f64, h: f64| {
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let (min_x, min_y, max_x, max_y) =
                bounds.unwrap_or((f64::MAX, f64::MAX, f64::MIN, f64::MIN));
            bounds = Some((min_x.min(x), min_y.min(y), max_x.max(x + w), max_y.max(y + h)));
        };
        for e in &self.restore_queue {
            if e.tiling_mode == crate::tiling::TilingMode::Tiled && !e.minimized {
                extend(e.virtual_x, e.virtual_y, e.width as f64, e.height as f64);
            }
        }
        for &w in self.windows.iter() {
            if w.is_null()
                || (*w).closed
                || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init)
                || (*w).tiling_mode != crate::tiling::TilingMode::Tiled
                || (*w).minimized
                || (*w).is_status_bar()
                || (*w).is_wallpaper()
                || (*w).is_grid()
            {
                continue;
            }
            extend(
                (*w).virtual_x,
                (*w).virtual_y,
                (*w).box_geom.width as f64,
                (*w).box_geom.height as f64,
            );
        }
        bounds
    }

    /// Dim frames at every restored window's saved geometry, shown from
    /// login until the real window maps (or a timeout sweeps the leftovers):
    /// the desk isn't a void while slow programs load, and the saved camera
    /// has something to be pointed at. Purely visual — placeholders are
    /// scene rects, not windows; focus logic never sees them.
    pub unsafe fn create_restore_placeholders(&mut self) {
        let parent = (*self.server).scene.layers.wm;
        if parent.is_null() {
            return;
        }
        for entry in &self.restore_queue {
            if entry.width == 0 || entry.height == 0 {
                continue;
            }
            let mut color = if entry.focused {
                self.layout.border_color_focused
            } else {
                self.layout.border_color
            };
            // Dim: scale all channels (premultiplied convention).
            for c in color.iter_mut() {
                *c *= 0.25;
            }
            let rect = ffi::wlr_scene_rect_create(parent, entry.width as i32, entry.height as i32, color.as_ptr());
            if rect.is_null() {
                continue;
            }
            self.restore_placeholders.push(RestorePlaceholder {
                rect,
                app_id: entry.app_id.clone(),
                title: entry.title.clone(),
                vx: entry.virtual_x,
                vy: entry.virtual_y,
                w: entry.width,
                h: entry.height,
            });
        }
        if self.restore_placeholders.is_empty() {
            return;
        }
        self.update_restore_placeholders();
        // Sweep leftovers whose programs never came back.
        let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
        self.restore_placeholder_timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_restore_placeholder_timeout),
            self as *mut WindowManager as *mut _,
        );
        if !self.restore_placeholder_timer.is_null() {
            ffi::wl_event_source_timer_update(self.restore_placeholder_timer, 60_000);
        }
    }

    /// The restore placeholder under a layout-space point, as its virtual
    /// rect `(vx, vy, w, h)`. Topmost (latest-created) wins on overlap.
    pub unsafe fn placeholder_at(&self, lx: f64, ly: f64) -> Option<(f64, f64, f64, f64)> {
        if self.restore_placeholders.is_empty() {
            return None;
        }
        let (mut out_x, mut out_y) = (0.0, 0.0);
        let outputs_list = &(*self.server).om.outputs as *const ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let wlr_box = (*output).sent.box_layout();
                out_x = wlr_box.x as f64;
                out_y = wlr_box.y as f64;
                break;
            }
            curr_out = (*curr_out).next;
        }
        let zoom = self.desk_zoom;
        for p in self.restore_placeholders.iter().rev() {
            let x = out_x + (p.vx - self.desk_pan_x) * zoom;
            let y = out_y + (p.vy - self.desk_pan_y) * zoom;
            let w = p.w as f64 * zoom;
            let h = p.h as f64 * zoom;
            if lx >= x && lx < x + w && ly >= y && ly < y + h {
                return Some((p.vx, p.vy, p.w as f64, p.h as f64));
            }
        }
        None
    }

    /// Focus-follow camera rules for a bare virtual rect — the same minimal
    /// pan-into-view windows get, for things that are not windows (restore
    /// placeholders).
    pub unsafe fn pan_to_virtual_rect(&mut self, vx: f64, vy: f64, w: f64, h: f64) {
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        let mut viewport: Option<ffi::wlr_box> = None;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                viewport = Some((*output).sent.box_layout());
                break;
            }
            curr_out = (*curr_out).next;
        }
        let Some(viewport) = viewport else { return };
        let (vw, vh) = (viewport.width as f64, viewport.height as f64);
        let cam = self.camera();
        if let Some(target) = crate::policy::camera::pan_into_view(vx, vy, w, h, cam, vw, vh) {
            self.target_desk_pan_x = Some(target.pan_x);
            self.target_desk_pan_y = Some(target.pan_y);
            self.start_panning_animation();
        }
    }

    /// Keep placeholders tracking the camera, same transform as windows.
    pub unsafe fn update_restore_placeholders(&mut self) {
        if self.restore_placeholders.is_empty() {
            return;
        }
        let (mut out_x, mut out_y) = (0, 0);
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let wlr_box = (*output).sent.box_layout();
                out_x = wlr_box.x;
                out_y = wlr_box.y;
                break;
            }
            curr_out = (*curr_out).next;
        }
        let zoom = self.desk_zoom;
        let radius = (self.layout.root_plate_corner_radius as f64 * zoom) as i32;
        for p in &self.restore_placeholders {
            let x = out_x + ((p.vx - self.desk_pan_x) * zoom).round() as i32;
            let y = out_y + ((p.vy - self.desk_pan_y) * zoom).round() as i32;
            ffi::river_scene_node_set_position_if_changed(p.rect as *mut ffi::wlr_scene_node, x, y);
            ffi::river_scene_rect_set_size_if_changed(
                p.rect,
                (p.w as f64 * zoom) as i32,
                (p.h as f64 * zoom) as i32,
            );
            ffi::river_scene_rect_set_corner_radius(p.rect, radius);
        }
    }

    /// Drop the placeholder claimed by a matched restore entry.
    unsafe fn remove_placeholder_for(&mut self, entry: &SavedWindowState) {
        if let Some(pos) = self
            .restore_placeholders
            .iter()
            .position(|p| p.app_id == entry.app_id && p.title == entry.title)
        {
            let p = self.restore_placeholders.remove(pos);
            if !p.rect.is_null() {
                ffi::wlr_scene_node_destroy(p.rect as *mut ffi::wlr_scene_node);
            }
        }
        if self.restore_placeholders.is_empty() && !self.restore_placeholder_timer.is_null() {
            ffi::wl_event_source_remove(self.restore_placeholder_timer);
            self.restore_placeholder_timer = std::ptr::null_mut();
        }
    }

    pub unsafe fn clear_restore_placeholders(&mut self) {
        for p in self.restore_placeholders.drain(..) {
            if !p.rect.is_null() {
                ffi::wlr_scene_node_destroy(p.rect as *mut ffi::wlr_scene_node);
            }
        }
        if !self.restore_placeholder_timer.is_null() {
            ffi::wl_event_source_remove(self.restore_placeholder_timer);
            self.restore_placeholder_timer = std::ptr::null_mut();
        }
    }

    pub unsafe fn save_state(&mut self) {
        if self.shutting_down {
            return;
        }
        // A direct save supersedes a scheduled one.
        self.save_state_pending = false;
        if !self.save_state_timer.is_null() {
            ffi::wl_event_source_timer_update(self.save_state_timer, 0);
        }
        let Some(path_str) = crate::config::default_state_path() else {
            log::error!("Could not resolve state file path");
            return;
        };
        let focused_win = self.focused_window();
        let mut saved_wins = Vec::new();
        let mut last_states = self.last_window_states.clone();

        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                continue;
            }
            // The grid layer is owned by its systemd unit and anchored by
            // live patches — saving/restoring it would spawn a duplicate
            // and dictate a stale geometry.
            if (*w).is_status_bar() || (*w).is_wallpaper() || (*w).is_grid() {
                continue;
            }
            // A Utility window owns its geometry entirely; saving it would
            // let a later session restore a size over the client's request —
            // the exact bug the mode deletes. (The clean-exit loops below do
            // NOT skip it: it is a real window and must still be closed.)
            if (*w).tiling_mode == crate::tiling::TilingMode::Utility {
                continue;
            }
            // A transient (dialog) belongs to its parent's process: saved, it
            // would carry that process's cmdline and a session restore would
            // spawn the whole app a second time just to place a dialog that
            // no longer exists. `try_restore` skips transients to match.
            if !(*w).get_parent().is_null() {
                continue;
            }

            let app_id = (*w).get_app_id_string().unwrap_or_default();
            if app_id.is_empty() {
                continue;
            }
            // cce-cloud surfaces are transient popups owned by the daemon, so
            // their cmdline is `cce-cloud --daemon` — restoring one would spawn
            // a duplicate daemon that steals the socket from cce-cloud.service.
            if app_id == "cce-cloud" {
                continue;
            }
            let title = (*w).get_title_string().unwrap_or_default();
            
            let pid = (*w).unreliable_pid();
            let mut cmdline = if pid > 0 {
                let proc_cmdline = std::fs::read(format!("/proc/{}/cmdline", pid)).unwrap_or_default();
                if !proc_cmdline.is_empty() {
                    let mut args: Vec<String> = proc_cmdline
                        .split(|&b| b == 0)
                        .map(|arg| String::from_utf8_lossy(arg).into_owned())
                        .collect();
                    if args.last().map_or(false, |s| s.is_empty()) {
                        args.pop();
                    }
                    if !args.is_empty() && args[0].starts_with("/tmp/.mount_") {
                        if let Ok(environ_bytes) = std::fs::read(format!("/proc/{}/environ", pid)) {
                            let appimage_opt = environ_bytes
                                .split(|&b| b == 0)
                                .find(|env_var| env_var.starts_with(b"APPIMAGE="))
                                .map(|env_var| {
                                    let val_bytes = &env_var[b"APPIMAGE=".len()..];
                                    String::from_utf8_lossy(val_bytes).into_owned()
                                });
                            if let Some(appimage_path) = appimage_opt {
                                args[0] = appimage_path;
                            }
                        }
                    }
                    // A wrapper that exec'd the real binary is invisible here;
                    // restore by the bare name when PATH says the name is a
                    // different file (see path_shadowed_name).
                    if !args.is_empty() {
                        if let Some(name) = path_shadowed_name(
                            &args[0],
                            &std::env::var("PATH").unwrap_or_default(),
                        ) {
                            args[0] = name;
                        }
                    }
                    // foot only tracks its launch dir, not the shell's current
                    // dir, so restore the child shell's cwd via
                    // --working-directory. Strip any pre-existing one first so
                    // the flag doesn't accumulate across save/restore cycles.
                    if app_id == "foot" {
                        if let Some(cwd) = foot_shell_cwd(pid) {
                            let mut i = 1;
                            while i < args.len() {
                                if args[i] == "--working-directory" || args[i] == "-D" {
                                    args.drain(i..(i + 2).min(args.len()));
                                } else if args[i].starts_with("--working-directory=")
                                    || args[i].starts_with("-D")
                                {
                                    args.remove(i);
                                } else {
                                    i += 1;
                                }
                            }
                            let flag = format!(
                                "--working-directory='{}'",
                                cwd.replace('\'', r"'\''")
                            );
                            args.insert(1.min(args.len()), flag);
                        }
                    }
                    args.join(" ")
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            if cmdline.is_empty() {
                cmdline = app_id.clone();
            }

            let is_focused = w == focused_win;

            let win_state = SavedWindowState {
                app_id: app_id.clone(),
                title: title.clone(),
                tiling_mode: (*w).tiling_mode,
                minimized: (*w).minimized,
                virtual_x: (*w).virtual_x,
                virtual_y: (*w).virtual_y,
                scale: (*w).scale,
                width: (*w).box_geom.width as u32,
                height: (*w).box_geom.height as u32,
                cmdline,
                focused: is_focused,
            };

            saved_wins.push(win_state.clone());

            if let Some(pos) = last_states.iter().position(|s| s.app_id == app_id) {
                last_states[pos] = win_state;
            } else {
                last_states.push(win_state);
            }
        }
        // Scrub entries persisted before the cce-cloud exclusion above.
        last_states.retain(|s| s.app_id != "cce-cloud");
        self.last_window_states = last_states;

        // A cancelled logout already closed some windows; keep their entries
        // until an exit succeeds so they are restored next login. An orphan
        // is dropped once a live window is the same app — relaunched. The
        // identity is app_id AND cmdline: an X11 class is as coarse as
        // "python3", and two unrelated apps must not stand in for each other.
        self.exit_orphans.retain(|o| !saved_wins.iter().any(|s| same_app(s, o)));
        saved_wins.extend(self.exit_orphans.iter().cloned());
        self.last_saved_windows = saved_wins.clone();

        let state = SavedState {
            desk_pan_x: self.desk_pan_x,
            desk_pan_y: self.desk_pan_y,
            desk_zoom: self.desk_zoom,
            windows: saved_wins,
            last_window_states: self.last_window_states.clone(),
            // The grid these geometries were measured under, so a later
            // session under a different grid can keep each Tiled entry on
            // its squares (remap_saved_entries) instead of re-deriving the
            // span from stale pixels.
            grid: Some(crate::policy::state::SavedGrid::from_params(&self.layout.snap_params())),
        };
        
        if let Ok(json_str) = serde_json::to_string_pretty(&state) {
            if self.last_saved_state_json.as_deref() == Some(json_str.as_str()) {
                return;
            }
            log::debug!("Saving state to {}", path_str);
            let path = std::path::Path::new(&path_str);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(path, &json_str) {
                // Only remember it once it is actually on disk, so a failed
                // write is retried on the next transaction rather than latched.
                Ok(()) => self.last_saved_state_json = Some(json_str),
                Err(e) => log::error!("Failed to write state file: {}", e),
            }
        }
    }

    pub unsafe fn start_clean_exit(&mut self) {
        if self.clean_exit_in_progress {
            return;
        }
        log::info!("Starting clean exit process...");

        // Save state before closing windows and setting exit flags
        self.save_state();

        self.clean_exit_in_progress = true;
        self.shutting_down = true;

        // Get list of windows we need to wait for to close cleanly
        let mut normal_windows = Vec::new();
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                continue;
            }
            if (*w).is_status_bar() || (*w).is_wallpaper() {
                continue;
            }
            normal_windows.push(w);
        }

        if normal_windows.is_empty() {
            log::info!("No active windows to close. Exiting immediately.");
            ffi::wl_display_terminate((*self.server).wl_server);
            return;
        }

        log::info!("Sending close request to {} windows...", normal_windows.len());
        for &w in &normal_windows {
            log::info!("Closing window: {:?}", (*w).get_title_string());
            (*w).close();
        }

        // Windows that are still here when this fires are being held by
        // something — nearly always an app asking whether to save — and the
        // timeout CANCELS the logout rather than forcing the display down
        // over the prompt. Long enough for the user to answer the prompt in
        // place; a quick answer completes the logout with no cancel at all.
        ffi::wl_event_source_timer_update(self.clean_exit_timer, 10_000);
    }

    /// Abandon a clean exit whose windows did not all close in time.
    ///
    /// What stalls a logout is nearly always an app refusing its close
    /// request to ask about unsaved work — Houdini with a dirty scene. Until
    /// 2026-09-08 the timeout forced the display down at that point, which
    /// is exactly the loss the prompt exists to prevent. So the logout is
    /// cancelled instead, the user is told what held it up, and they log
    /// out again once the prompt is answered; `ccectl exit force` skips the
    /// wait for a client that is hung rather than asking.
    ///
    /// The windows the exit already closed would otherwise vanish from the
    /// next state snapshot (`save_state` runs every transaction), and with
    /// them from the next login: they are remembered as `exit_orphans`.
    pub unsafe fn cancel_clean_exit(&mut self) {
        if !self.clean_exit_in_progress {
            return;
        }
        self.clean_exit_in_progress = false;
        self.shutting_down = false;
        if !self.clean_exit_timer.is_null() {
            ffi::wl_event_source_timer_update(self.clean_exit_timer, 0);
        }

        let mut remaining: Vec<String> = Vec::new();
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                continue;
            }
            if (*w).is_status_bar() || (*w).is_wallpaper() {
                continue;
            }
            // A later self-initiated close must not read as a crash.
            (*w).close_requested = false;
            let app_id = (*w).get_app_id_string().unwrap_or_default();
            if (*w).get_parent().is_null() && !app_id.is_empty() {
                let title = (*w).get_title_string().unwrap_or_default();
                remaining.push(if title.is_empty() { app_id.clone() } else { title });
            }
        }
        // The snapshot written as the exit began lists every window it went
        // on to close; a fresh one lists what survived. The difference is
        // what must be remembered.
        let before = std::mem::take(&mut self.last_saved_windows);
        self.exit_orphans.clear();
        self.save_state();
        let survivors = std::mem::take(&mut self.last_saved_windows);
        self.exit_orphans = before
            .into_iter()
            .filter(|b| !survivors.iter().any(|l| same_app(l, b)))
            .collect();
        // Write the merged snapshot now rather than on the next transaction.
        self.save_state();

        // A restart-compositor that stalled must not leave its flag behind
        // for the next plain logout to act on.
        let user = std::env::var("USER").unwrap_or_else(|_| format!("uid{}", libc::getuid()));
        let _ = std::fs::remove_file(format!("/tmp/cce-restart-requested-{}", user));

        let held_by = remaining.join(", ");
        log::info!(
            "Clean exit cancelled: {} window(s) still open ({}); {} closed window(s) remembered for the next login",
            remaining.len(), held_by, self.exit_orphans.len()
        );
        let _ = std::process::Command::new("notify-send")
            .arg("cce")
            .arg(format!(
                "Logout cancelled — still open: {}\nAnswer any save prompt, then log out again.",
                held_by
            ))
            .spawn();
        self.dirty_windowing();
    }

    pub unsafe fn check_clean_exit_progress(&mut self) {
        if !self.clean_exit_in_progress {
            return;
        }

        let mut normal_windows_count = 0;
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                continue;
            }
            if (*w).is_status_bar() || (*w).is_wallpaper() {
                continue;
            }
            normal_windows_count += 1;
        }

        if normal_windows_count == 0 {
            log::info!("All windows closed cleanly. Exiting display server.");
            if !self.clean_exit_timer.is_null() {
                ffi::wl_event_source_remove(self.clean_exit_timer);
                self.clean_exit_timer = std::ptr::null_mut();
            }
            ffi::wl_display_terminate((*self.server).wl_server);
        } else {
            log::info!("Clean exit: waiting for {} remaining windows to close...", normal_windows_count);
        }
    }

    /// Whether an app_id gets the decorated-window treatment (rounded corner
    /// clip, blur-behind, drop shadow) without requesting SSD: every cce app,
    /// plus the `window_manager.rounded_apps` config allowlist. The one
    /// predicate behind every radius/blur/shadow decision — the mirrored
    /// render sites must all agree or the effects visibly disagree per pass.
    pub fn is_decorated_app(&self, app_id: &str) -> bool {
        app_id.starts_with("cce-") || self.rounded_apps.iter().any(|a| app_id_matches(a, app_id))
    }

    /// Should the compositor draw an edge bevel on this app? Unlike
    /// `is_decorated_app` there is NO implicit cce-* arm: every cce-ui app
    /// draws its own bevel, and a second one from the compositor just doubles
    /// the rim. Only apps named in `bevel_apps` (defaulting to `rounded_apps`)
    /// get one.
    pub fn is_beveled_app(&self, app_id: &str) -> bool {
        self.bevel_apps.iter().any(|a| app_id_matches(a, app_id))
    }

    pub unsafe fn match_and_remove_restore_state(&mut self, app_id: &str, title: &str) -> Option<SavedWindowState> {
        if app_id.is_empty() {
            return None;
        }
        // First pass: Exact match (app_id AND title)
        if let Some(pos) = self.restore_queue.iter().position(|w| w.app_id == app_id && w.title == title) {
            let entry = self.restore_queue.remove(pos);
            self.remove_placeholder_for(&entry);
            return Some(entry);
        }
        // Second pass: Fuzzy title match (e.g. prefix match, asterisk stripping)
        if let Some(pos) = self.restore_queue.iter().position(|w| {
            if w.app_id != app_id {
                return false;
            }
            let t1 = title.trim_end_matches('*');
            let t2 = w.title.trim_end_matches('*');
            t1 == t2 || t1.starts_with(t2) || t2.starts_with(t1)
        }) {
            let entry = self.restore_queue.remove(pos);
            self.remove_placeholder_for(&entry);
            return Some(entry);
        }
        // Third pass: app_id only match
        if let Some(pos) = self.restore_queue.iter().position(|w| w.app_id == app_id) {
            let entry = self.restore_queue.remove(pos);
            self.remove_placeholder_for(&entry);
            return Some(entry);
        }
        None
    }

    pub unsafe fn match_last_window_state(&self, app_id: &str, title: &str) -> Option<SavedWindowState> {
        if app_id.is_empty() {
            return None;
        }
        // First pass: Exact match (app_id AND title)
        if let Some(w) = self.last_window_states.iter().find(|w| w.app_id == app_id && w.title == title) {
            return Some(w.clone());
        }
        // Second pass: Fuzzy title match
        if let Some(w) = self.last_window_states.iter().find(|w| {
            if w.app_id != app_id {
                return false;
            }
            let t1 = title.trim_end_matches('*');
            let t2 = w.title.trim_end_matches('*');
            t1 == t2 || t1.starts_with(t2) || t2.starts_with(t1)
        }) {
            return Some(w.clone());
        }
        // Third pass: app_id only match
        if let Some(w) = self.last_window_states.iter().find(|w| w.app_id == app_id) {
            return Some(w.clone());
        }
        None
    }

    /// Consume the placement hint for `app_id`, if one was registered in the
    /// last few seconds (stale hints — a spawn that never mapped — are purged).
    /// Claim a pending placement for a window that is mapping.
    ///
    /// Matching is exact first, then the loose app_id rule `Action::Toggle`
    /// already uses (case-insensitive, either side containing the other): a
    /// menu or launcher knows the COMMAND it ran — `foot`, `cce-files` — while
    /// the client picks its own app_id, and the two agree often but not
    /// always. An exact pass first keeps a specific hint from being stolen by
    /// a loosely-matching one.
    pub fn take_pending_placement(&mut self, app_id: &str) -> Option<(f64, f64, bool)> {
        const HINT_TTL: std::time::Duration = std::time::Duration::from_secs(10);
        self.pending_placements.retain(|(_, _, _, _, at)| at.elapsed() < HINT_TTL);
        let lower = app_id.to_lowercase();
        let idx = self
            .pending_placements
            .iter()
            .position(|(id, _, _, _, _)| id == app_id)
            .or_else(|| {
                self.pending_placements.iter().position(|(id, _, _, _, _)| {
                    let k = id.to_lowercase();
                    !k.is_empty() && (lower.contains(&k) || k.contains(&lower))
                })
            })?;
        let (_, x, y, cell, _) = self.pending_placements.remove(idx);
        Some((x, y, cell))
    }

    pub unsafe fn spawn_restored_windows(&mut self) {
        log::info!("Spawning restored windows. Total: {}", self.restore_queue.len());
        let restored = self.restore_queue.clone();
        std::thread::spawn(move || {
            // Clients that reach for the Secret Service on startup start last,
            // behind the keyring barrier: launched into a still-locked keyring
            // they either fail outright or quietly fall back to plaintext
            // credential storage. gnome-keyring now comes up already unlocked
            // in ~1.3s, so the wait is short — but it is not zero, and losing
            // that race downgrades an app's storage without saying so.
            // Everything else starts immediately.
            let (secret_gated, immediate): (Vec<_>, Vec<_>) = restored
                .into_iter()
                .partition(|w| Self::needs_secret_service(&w.cmdline));

            let mut spawned_any = false;
            for w in &immediate {
                Self::spawn_restored_one(w, &mut spawned_any);
            }
            if !secret_gated.is_empty() {
                Self::wait_for_secret_service(secret_gated.len());
                for w in &secret_gated {
                    Self::spawn_restored_one(w, &mut spawned_any);
                }
            }
        });
    }

    /// True when this restored command will talk to `org.freedesktop.secrets`
    /// as it starts. Electron names its backend on the command line, and that
    /// is the population that blocks on a locked keyring (`basic` is Electron's
    /// plaintext fallback — it never touches the Secret Service).
    fn needs_secret_service(cmdline: &str) -> bool {
        match cmdline.split("--password-store=").nth(1) {
            Some(rest) => !matches!(rest.split_whitespace().next(), None | Some("basic")),
            None => false,
        }
    }

    /// Block until the Secret Service reports its default collection unlocked.
    ///
    /// Bounded twice over, because a login must never wedge here: if nothing
    /// owns the bus name shortly after startup this machine has no keyring to
    /// wait for, and if the collection simply never unlocks the gated apps
    /// still get launched (degraded, exactly as they were before this barrier
    /// existed) rather than being dropped.
    fn wait_for_secret_service(gated: usize) {
        const NAME_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
        const UNLOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
        const POLL: std::time::Duration = std::time::Duration::from_millis(500);

        let busctl = |args: &[&str]| -> Option<String> {
            let out = std::process::Command::new("busctl").args(args).output().ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        };

        let started = std::time::Instant::now();
        let mut have_name = false;
        while started.elapsed() < NAME_GRACE {
            if busctl(&[
                "--user", "call", "org.freedesktop.DBus", "/org/freedesktop/DBus",
                "org.freedesktop.DBus", "NameHasOwner", "s", "org.freedesktop.secrets",
            ])
            .as_deref()
                == Some("b true")
            {
                have_name = true;
                break;
            }
            std::thread::sleep(POLL);
        }
        if !have_name {
            log::info!(
                "No Secret Service on the bus after {NAME_GRACE:?}; starting {gated} keyring client(s) without waiting"
            );
            return;
        }

        let started = std::time::Instant::now();
        while started.elapsed() < UNLOCK_TIMEOUT {
            match busctl(&[
                "--user", "get-property", "org.freedesktop.secrets",
                "/org/freedesktop/secrets/aliases/default",
                "org.freedesktop.Secret.Collection", "Locked",
            ])
            .as_deref()
            {
                Some("b false") => {
                    log::info!(
                        "Keyring unlocked after {:?}; starting {gated} gated client(s)",
                        started.elapsed()
                    );
                    return;
                }
                _ => std::thread::sleep(POLL),
            }
        }
        log::warn!(
            "Keyring still locked after {UNLOCK_TIMEOUT:?}; starting {gated} gated client(s) anyway"
        );
    }

    fn spawn_restored_one(w: &SavedWindowState, spawned_any: &mut bool) {
        // A wine/Proton window records its WINDOWS-side exe path
        // (C:\... or C:/...) as the command — /bin/sh can never run
        // it, so each one burns a silent no-op fork per login. Skip
        // them outright.
        let cmd_trimmed = w.cmdline.trim();
        let bytes = cmd_trimmed.as_bytes();
        let is_windows_path = bytes.len() > 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'/' || bytes[2] == b'\\');
        if is_windows_path {
            log::info!(
                "Skipping unrestorable Windows-path command for {:?}: {}",
                w.app_id,
                cmd_trimmed
            );
            return;
        }
        if w.cmdline.is_empty() {
            return;
        }
        // Small stagger so N clients don't all hit Vulkan device
        // init at the same instant; restore matching and focus
        // restoration are map-order independent.
        if *spawned_any {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        *spawned_any = true;
        log::info!("Deferred spawning restored window command: {}", w.cmdline);
        let cmd = w.cmdline.clone();
        match unsafe { nix::unistd::fork() } {
            Ok(nix::unistd::ForkResult::Child) => {
                crate::process::cleanup_child();
                let env: Vec<std::ffi::CString> = std::env::vars()
                    .map(|(k, v)| std::ffi::CString::new(format!("{}={}", k, v)).unwrap())
                    .collect();
                let env_ptrs: Vec<&std::ffi::CStr> = env.iter().map(|s| s.as_c_str()).collect();
                let sh_c = std::ffi::CString::new("/bin/sh").unwrap();
                let c_c = std::ffi::CString::new("-c").unwrap();
                let cmd_c = std::ffi::CString::new(cmd).unwrap();
                let args = [sh_c.as_c_str(), c_c.as_c_str(), cmd_c.as_c_str()];
                let _ = nix::unistd::execve(&sh_c, &args, &env_ptrs);
                std::process::exit(1);
            }
            Ok(_) => {}
            Err(e) => {
                log::error!("failed to fork child process: {}", e);
            }
        }
    }

    pub unsafe fn start_ipc(&mut self, display_socket: Option<String>) {
        if self.ipc_rx.is_none() {
            let (rx, wake) = crate::ipc_server::spawn_ipc_server(display_socket);
            let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
            self.ipc_source = ffi::wl_event_loop_add_fd(
                event_loop,
                std::os::fd::AsRawFd::as_raw_fd(&*wake),
                ffi::WL_EVENT_READABLE as u32,
                Some(handle_ipc_event),
                self as *mut WindowManager as *mut _,
            );
            if self.ipc_source.is_null() {
                log::error!("failed to add the IPC wake fd to the event loop; ccectl will not work");
            }
            self.ipc_rx = Some(rx);
            self.ipc_wake = Some(wake);
        }
    }

    /// Own the window-stream hub and wake on its subscriber eventfd.
    pub unsafe fn start_stream(&mut self, hub: crate::stream_server::StreamHub) {
        let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
        self.stream_source = ffi::wl_event_loop_add_fd(
            event_loop,
            std::os::fd::AsRawFd::as_raw_fd(&*hub.wake),
            ffi::WL_EVENT_READABLE as u32,
            Some(handle_stream_wake),
            self as *mut WindowManager as *mut _,
        );
        if self.stream_source.is_null() {
            log::error!("failed to add the stream wake fd to the event loop; window streams will not run");
        }
        self.stream_hub = Some(hub);
    }

    pub unsafe fn deinit(&mut self) {
        if !self.global.is_null() {
            ffi::wl_global_destroy(self.global);
            self.global = std::ptr::null_mut();
        }
        if !self.ipc_source.is_null() {
            ffi::wl_event_source_remove(self.ipc_source);
            self.ipc_source = std::ptr::null_mut();
        }
        self.ipc_wake = None;
        if !self.stream_source.is_null() {
            ffi::wl_event_source_remove(self.stream_source);
            self.stream_source = std::ptr::null_mut();
        }
        if !self.stream_timer.is_null() {
            ffi::wl_event_source_remove(self.stream_timer);
            self.stream_timer = std::ptr::null_mut();
        }
        if !self.save_state_timer.is_null() {
            ffi::wl_event_source_remove(self.save_state_timer);
            self.save_state_timer = std::ptr::null_mut();
        }
        if !self.timeout.is_null() {
            ffi::wl_event_source_remove(self.timeout);
            self.timeout = std::ptr::null_mut();
        }
        if !self.clean_exit_timer.is_null() {
            ffi::wl_event_source_remove(self.clean_exit_timer);
            self.clean_exit_timer = std::ptr::null_mut();
        }
        if !self.sun_timer.is_null() {
            ffi::wl_event_source_remove(self.sun_timer);
            self.sun_timer = std::ptr::null_mut();
        }
        if !self.animation_timer.is_null() {
            ffi::wl_event_source_remove(self.animation_timer);
            self.animation_timer = std::ptr::null_mut();
        }
        if !self.edge_pan_timer.is_null() {
            ffi::wl_event_source_remove(self.edge_pan_timer);
            self.edge_pan_timer = std::ptr::null_mut();
        }
        if !self.restore_placeholder_timer.is_null() {
            ffi::wl_event_source_remove(self.restore_placeholder_timer);
            self.restore_placeholder_timer = std::ptr::null_mut();
        }
        // Placeholder rects go down with the scene; only the bookkeeping.
        self.restore_placeholders.clear();
        if !self.viewport_settle_timer.is_null() {
            ffi::wl_event_source_remove(self.viewport_settle_timer);
            self.viewport_settle_timer = std::ptr::null_mut();
        }
        wl_listener_remove(&mut self.server_destroy);
    }

    pub unsafe fn stop_panning_animation(&mut self) {
        self.target_desk_pan_x = None;
        self.target_desk_pan_y = None;
        self.target_desk_zoom = None;
        self.camera_ramp_anim = None;
        self.pan_coast_vx = 0.0;
        self.pan_coast_vy = 0.0;
        self.zoom_anchor = None;
        self.camera_anim_active = false;
        self.anim_last_tick = None;
    }

    /// Wheel-glide rate for the desktop camera, 1/s (`input { scroll_ease }`).
    pub fn scroll_ease_rate(&self) -> f64 {
        self.input_config
            .scroll_ease
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(12.0)
    }

    /// Whether a trackpad flick coasts the desktop (`input { kinetic_scroll }`).
    pub fn kinetic_scroll(&self) -> bool {
        self.input_config.kinetic_scroll.unwrap_or(true)
    }

    /// Coast decay, 1/s (`input { scroll_friction }`).
    pub fn scroll_friction(&self) -> f64 {
        self.input_config
            .scroll_friction
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(6.0)
    }

    /// The current camera as the policy crate's plain-data snapshot.
    pub fn camera(&self) -> crate::policy::camera::Camera {
        crate::policy::camera::Camera {
            pan_x: self.desk_pan_x,
            pan_y: self.desk_pan_y,
            zoom: self.desk_zoom,
        }
    }

    /// Snapshot for `Policy::action`: seat- and scene-dependent facts
    /// (cursor output, hovered window, focus) resolved up front, the
    /// arrange-pass convention.
    unsafe fn build_action_ctx(&mut self) -> crate::policy::api::ActionCtx {
        use crate::policy::api::{ActionCtx, ActionWindow, Rect, WindowId};

        // First enabled output: the legacy viewport for zooms and View jumps.
        let (mut viewport_w, mut viewport_h) = (1920.0, 1080.0);
        let (mut first_x, mut first_y) = (0.0, 0.0);
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let wlr_box = (*output).sent.box_layout();
                viewport_w = wlr_box.width as f64;
                viewport_h = wlr_box.height as f64;
                first_x = wlr_box.x as f64;
                first_y = wlr_box.y as f64;
                break;
            }
            curr_out = (*curr_out).next;
        }

        let mut cursor_viewport = Rect {
            x: first_x as i32,
            y: first_y as i32,
            width: viewport_w as i32,
            height: viewport_h as i32,
        };
        let mut has_cursor = false;
        let (mut cursor_x, mut cursor_y) = (0.0, 0.0);
        let mut hovered = None;
        let mut focused = None;
        if let Some(seat) = self.first_seat() {
            has_cursor = true;
            cursor_x = (*seat).cursor.x();
            cursor_y = (*seat).cursor.y();
            let wlr_output = (*self.server).om.output_at(cursor_x, cursor_y);
            if !wlr_output.is_null() {
                let mut output_box = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
                ffi::wlr_output_layout_get_box((*self.server).om.output_layout, wlr_output, &mut output_box);
                cursor_viewport = Rect {
                    x: output_box.x,
                    y: output_box.y,
                    width: output_box.width,
                    height: output_box.height,
                };
            }
            if let Some(result) = (*self.server).scene.at(cursor_x, cursor_y) {
                if let crate::scene_node_data::SceneNodeDataVal::Window(w) = result.data {
                    if !(*w).is_status_bar() && !(*w).is_wallpaper() {
                        hovered = Some(WindowId((*w).ref_key));
                    }
                }
            }
        }
        // The WM's effective focus (overlay UI looked through): actions
        // triggered from a menu act on the real window underneath.
        {
            let fw = self.focused_window();
            if !fw.is_null() && !(*fw).closed {
                focused = Some(WindowId((*fw).ref_key));
            }
        }

        // Render-list membership feeds focus_cyclable: the focus ring only
        // walks windows that are actually being rendered.
        let mut rendered = std::collections::HashSet::new();
        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            if let crate::wm_node::WmNodeType::Window(window) = (*node).get() {
                if !window.is_null() {
                    rendered.insert(window as usize);
                }
            }
            curr = (*curr).next;
        }

        let mut windows = Vec::new();
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed {
                continue;
            }
            let app_id = (*w).get_app_id_string();
            let is_status = app_id.as_deref().map_or(false, |id| id.starts_with("cce-status"));
            let is_wallpaper = app_id.as_deref() == Some("cce-wallpaper");
            let visible = !matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init);
            let resolved_mode = self.get_mode_for_window(w);
            let overview_eligible = !(*w).minimized
                && !is_status
                && !is_wallpaper
                && !(*w).is_grid()
                && visible
                && resolved_mode != crate::tiling::TilingMode::Popup
                && resolved_mode != crate::tiling::TilingMode::Overlay;
            let focus_cyclable = rendered.contains(&(w as usize))
                && !(*w).minimized
                && !is_status
                && !(*w).is_grid()
                && !(*w).is_overlay_ui();
            windows.push(ActionWindow {
                id: WindowId((*w).ref_key),
                app_id,
                title: (*w).get_title_string(),
                mapped: matches!((*w).state, crate::window::WindowState::Mapped),
                x: (*w).virtual_x,
                y: (*w).virtual_y,
                w: if (*w).box_geom.width > 0 { (*w).box_geom.width as f64 } else { 800.0 },
                h: if (*w).box_geom.height > 0 { (*w).box_geom.height as f64 } else { 600.0 },
                scale: (*w).scale,
                mode: (*w).tiling_mode,
                resolved_mode,
                pre_fullscreen: (*w).pre_fullscreen,
                visible,
                focus_cyclable,
                overview_eligible,
            });
        }

        ActionCtx {
            camera: self.camera(),
            overview: self.mode == WindowManagerMode::Overview,
            pan_target_x: self.target_desk_pan_x,
            pan_target_y: self.target_desk_pan_y,
            viewport_w,
            viewport_h,
            cursor_viewport,
            has_cursor,
            cursor_x,
            cursor_y,
            hovered,
            focused,
            grid_period_x: self.layout.desktop_cell_width.max(5.0)
                + self.layout.desktop_gap_width.max(0) as f64,
            grid_period_y: self.layout.desktop_cell_height.max(5.0)
                + self.layout.desktop_gap_width.max(0) as f64,
            windows,
        }
    }

    /// Record the edge auto-pan velocity (screen px/s) and arm its 16ms tick
    /// when nonzero. A zero velocity just parks: the armed tick sees it and
    /// stops itself without re-arming.
    pub unsafe fn set_edge_pan_velocity(&mut self, vx: f64, vy: f64) {
        self.edge_pan_vx = vx;
        self.edge_pan_vy = vy;
        if vx == 0.0 && vy == 0.0 {
            return;
        }
        if self.edge_pan_timer.is_null() {
            let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
            self.edge_pan_timer = ffi::wl_event_loop_add_timer(
                event_loop,
                Some(handle_edge_pan_tick),
                self as *mut WindowManager as *mut _,
            );
        }
        if !self.edge_pan_timer.is_null() {
            ffi::wl_event_source_timer_update(self.edge_pan_timer, 16);
        }
    }

    /// Start (or continue) the camera animation: the step itself runs in
    /// `step_camera_frame` on every output frame, so this schedules a frame
    /// and arms the watchdog timer that keeps frames flowing while the
    /// animation is live. Callers set the targets first.
    pub unsafe fn start_panning_animation(&mut self) {
        self.camera_anim_active = true;
        if self.anim_last_tick.is_none() {
            self.anim_last_tick = Some(crate::util::timestamp_ns());
        }
        self.schedule_frame_all_outputs();
        if self.animation_timer.is_null() {
            let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
            self.animation_timer = ffi::wl_event_loop_add_timer(
                event_loop,
                Some(handle_panning_animation_tick),
                self as *mut WindowManager as *mut _,
            );
        }
        if !self.animation_timer.is_null() {
            ffi::wl_event_source_timer_update(self.animation_timer, CAMERA_WATCHDOG_MS);
        }
    }

    /// Ask every enabled output for a frame (the camera step runs in the
    /// frame handler). A no-op for an output that already has one pending.
    pub unsafe fn schedule_frame_all_outputs(&mut self) {
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs_list).next;
        while curr != outputs_list {
            let next = (*curr).next;
            let output = crate::container_of!(curr, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                ffi::wlr_output_schedule_frame((*output).wlr_output);
            }
            curr = next;
        }
    }

    /// Queue finger-pan motion for the next output frame (see `pan_pending`).
    pub unsafe fn queue_pan(&mut self, dx: f64, dy: f64) {
        self.pan_pending[0] += dx;
        self.pan_pending[1] += dy;
        self.schedule_frame_all_outputs();
    }

    /// Queue the interactive move/resize's configure and relayout for the
    /// next output frame (see `op_frame_pending`).
    pub unsafe fn queue_op_frame(&mut self) {
        if !self.op_frame_pending {
            self.op_frame_pending = true;
            self.schedule_frame_all_outputs();
        }
    }

    /// The seat-op step for the frame about to render: configure the
    /// dragged window for the LATEST pointer position and run the manage
    /// pass, once per vblank — what `Seat::op_update` did per event. The
    /// pass runs synchronously, as the dirty-idle callback would run it, so
    /// this frame draws the result; with a pass already in flight the dirty
    /// flag queues it, as before.
    pub unsafe fn step_op_frame(&mut self) {
        if !self.op_frame_pending {
            return;
        }
        self.op_frame_pending = false;
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats_list).next;
        while curr != seats_list {
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            if let Some(ref op) = (*seat).op {
                let win = op.window_ptr;
                if !win.is_null() && !(*win).closed {
                    (*win).manage_finish();
                }
            }
            curr = (*curr).next;
        }
        if matches!(self.state, WindowManagerState::Idle) {
            self.scheduled.dirty = true;
            self.scheduled.dirty_lazy = false;
            self.manage_start();
        } else {
            self.dirty_windowing();
        }
    }

    /// The camera step for the frame about to render: apply queued finger
    /// motion, advance any live animation by the real elapsed time, and
    /// relayout if the camera moved. Called from the output frame handler
    /// before `render_and_commit`, so the position on screen is the one
    /// computed for this vblank.
    /// `frame_target_ns` is when the frame about to render is predicted to
    /// be presented (`Output::predicted_present_ns`); the animation
    /// advances to that instant.
    pub unsafe fn step_camera_frame(&mut self, frame_target_ns: u64) {
        let has_pending = self.pan_pending != [0.0, 0.0];
        if !self.camera_anim_active && !has_pending {
            return;
        }
        let dt = self.anim_last_tick.map_or(0.0, |t| frame_target_ns.saturating_sub(t) as f64 / 1e9);
        // A second output's frame in the same vblank takes no extra step.
        if self.camera_anim_active && !has_pending && dt < 0.002 {
            return;
        }
        if has_pending {
            self.desk_pan_x += self.pan_pending[0];
            self.desk_pan_y += self.pan_pending[1];
            self.pan_pending = [0.0, 0.0];
        }
        if self.camera_anim_active {
            if crate::output::frame_debug() {
                log::info!("[cce-frame] camera step dt={}us", (dt * 1e6) as u64);
            }
            self.anim_last_tick = Some(frame_target_ns);
            if self.advance_camera_animation(dt.clamp(0.0, 0.1), frame_target_ns) {
                self.camera_anim_active = false;
                self.anim_last_tick = None;
            }
        }
        if matches!(self.state, WindowManagerState::Idle) {
            self.update_viewport_local();
        } else {
            self.dirty_windowing();
        }
    }

    /// Advance the camera animation by `dt` seconds. Returns true when
    /// nothing is left to animate.
    unsafe fn advance_camera_animation(&mut self, dt: f64, frame_target_ns: u64) -> bool {
        let mut done = true;
        // Frame-rate independent exponential approach: the same fraction of
        // the remaining distance per unit time whatever the frame pacing.
        let factor = 1.0 - (-self.scroll_ease_rate() * dt).exp();

        // Ramp-driven transition: position is a pure function of elapsed
        // time, so a stalled frame never changes where the camera lands.
        let ramp = self.camera_ramp_anim.as_ref().map(|a| {
            (a.start, a.target, frame_target_ns.saturating_sub(a.started_ns) as f64 / 1e6 / a.duration_ms)
        });
        if let Some((start, target, t)) = ramp {
            if t >= 1.0 {
                self.desk_pan_x = target.pan_x;
                self.desk_pan_y = target.pan_y;
                self.desk_zoom = target.zoom;
                self.camera_ramp_anim = None;
            } else if let Some((ramp, _)) = &self.layout.overview_anim {
                let p = ramp.progress(t);
                let cam = crate::policy::camera::anchored_interp(start, target, p);
                self.desk_pan_x = cam.pan_x;
                self.desk_pan_y = cam.pan_y;
                self.desk_zoom = cam.zoom;
                done = false;
            } else {
                // Ramp was unconfigured mid-flight (reload): land instantly.
                self.desk_pan_x = target.pan_x;
                self.desk_pan_y = target.pan_y;
                self.desk_zoom = target.zoom;
                self.camera_ramp_anim = None;
            }
        }

        if let Some(target_x) = self.target_desk_pan_x {
            let dx = target_x - self.desk_pan_x;
            if dx.abs() > 0.5 {
                self.desk_pan_x += dx * factor;
                done = false;
            } else {
                self.desk_pan_x = target_x;
                self.target_desk_pan_x = None;
            }
        }
        if let Some(target_y) = self.target_desk_pan_y {
            let dy = target_y - self.desk_pan_y;
            if dy.abs() > 0.5 {
                self.desk_pan_y += dy * factor;
                done = false;
            } else {
                self.desk_pan_y = target_y;
                self.target_desk_pan_y = None;
            }
        }

        // Zoom eases geometrically (exponential approach in log space): a
        // linear step would leap multiple-x per frame at the small end of an
        // overview exit, while a constant per-frame RATIO reads as uniform
        // motion.
        if let Some(target_zoom) = self.target_desk_zoom {
            let cur = self.desk_zoom.max(1e-6);
            let log_delta = (target_zoom / cur).ln();
            let new_zoom = if log_delta.abs() > 0.001 {
                done = false;
                cur * (log_delta * factor).exp()
            } else {
                self.target_desk_zoom = None;
                target_zoom
            };
            // An anchored zoom (wheel zoom about the cursor) re-derives the
            // pan from the anchor every step, so the pivot never wanders.
            if let Some((ax, ay)) = self.zoom_anchor {
                let cam = crate::policy::camera::zoom_about_anchor(self.camera(), ax, ay, new_zoom);
                self.desk_pan_x = cam.pan_x;
                self.desk_pan_y = cam.pan_y;
                self.desk_zoom = cam.zoom;
                if self.target_desk_zoom.is_none() {
                    self.zoom_anchor = None;
                }
            } else {
                self.desk_zoom = new_zoom;
            }
        }

        // Kinetic pan: a trackpad flick's velocity carries the desktop on,
        // decaying under friction; it stalls below one screen pixel per frame.
        if self.pan_coast_vx != 0.0 || self.pan_coast_vy != 0.0 {
            self.desk_pan_x += self.pan_coast_vx * dt;
            self.desk_pan_y += self.pan_coast_vy * dt;
            let decay = (-self.scroll_friction() * dt).exp();
            self.pan_coast_vx *= decay;
            self.pan_coast_vy *= decay;
            let screen_speed = self.pan_coast_vx.hypot(self.pan_coast_vy) * self.desk_zoom;
            if screen_speed < 5.0 {
                self.pan_coast_vx = 0.0;
                self.pan_coast_vy = 0.0;
            } else {
                done = false;
            }
        }
        done
    }

    pub unsafe fn ensure_windowing(&self) -> bool {
        match self.state {
            WindowManagerState::Manage => true,
            _ => {
                if !self.object.is_null() {
                    ffi::wl_resource_post_error(
                        self.object,
                        ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
                        b"invalid modification of window management state\0".as_ptr() as *const _,
                    );
                }
                false
            }
        }
    }

    pub unsafe fn ensure_rendering(&self) -> bool {
        match self.state {
            WindowManagerState::Manage | WindowManagerState::InflightConfigures(_) | WindowManagerState::Render => true,
            WindowManagerState::Idle => {
                if !self.object.is_null() {
                    ffi::wl_resource_post_error(
                        self.object,
                        ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
                        b"invalid modification of rendering state\0".as_ptr() as *const _,
                    );
                }
                false
            }
        }
    }

    /// Start the border hover fade if it isn't already running. Idempotent —
    /// re-arming mid-fade would restart the timer and step it twice as fast.
/// Switch overview on or off, arming the handle fade on a real change.
    ///
    /// Every site that flips the mode goes through here. Resize handles only
    /// exist in overview (`window::draw_borders`), so the transition has to
    /// start the fade timer or they would pop in on the next unrelated
    /// redraw instead of easing — and the zoom paths that set the mode do it
    /// every frame, hence the equality guard.
    pub unsafe fn set_mode(&mut self, mode: WindowManagerMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.arm_border_fade();
    }

    pub unsafe fn arm_border_fade(&mut self) {
        if self.border_fade_running || self.border_fade_timer.is_null() {
            return;
        }
        self.border_fade_running = true;
        ffi::wl_event_source_timer_update(self.border_fade_timer, 16);
    }

    #[track_caller]
    pub unsafe fn dirty_windowing(&mut self) {
        // Capturing and symbolizing a backtrace costs far more than the event it
        // annotates, and this fires on routine commits — the session runs at
        // --log-level debug, so keying it on Debug meant ~160 log lines/second
        // and most of a 19MB session log. Behind its own switch now.
        if dirty_backtrace_debug() {
            let bt = std::backtrace::Backtrace::force_capture();
            log::debug!("dirty_windowing called from backtrace:\n{}", bt);
        }
        if dirty_trace() {
            log::debug!("dirty_windowing from {}", std::panic::Location::caller());
        }
        self.scheduled.dirty = true;
        self.add_dirty_idle();
    }
 
    pub unsafe fn dirty_windowing_lazy(&mut self) {
        self.scheduled.dirty_lazy = true;
        self.add_dirty_idle();
    }
 
    pub unsafe fn clean_windowing(&mut self) {
        self.scheduled.dirty = false;
        self.scheduled.dirty_lazy = false;
        self.remove_dirty_idle();
    }
 
    #[track_caller]
    pub unsafe fn dirty_rendering(&mut self) {
        if dirty_trace() {
            log::debug!("dirty_rendering from {}", std::panic::Location::caller());
        }
        self.rendering_scheduled.dirty = true;
        self.add_dirty_idle();
    }

    pub unsafe fn clean_rendering(&mut self) {
        self.rendering_scheduled.dirty = false;
        self.remove_dirty_idle();
    }

    unsafe fn add_dirty_idle(&mut self) {
        if self.scheduled.dirty || self.scheduled.dirty_lazy || self.rendering_scheduled.dirty {
            if self.dirty_idle.is_null() {
                let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
                self.dirty_idle = ffi::wl_event_loop_add_idle(
                    event_loop,
                    Some(dirty_idle_callback),
                    self as *mut WindowManager as *mut _,
                );
            }
        }
    }

    unsafe fn remove_dirty_idle(&mut self) {
        if !self.scheduled.dirty && !self.scheduled.dirty_lazy && !self.rendering_scheduled.dirty {
            if !self.dirty_idle.is_null() {
                ffi::wl_event_source_remove(self.dirty_idle);
                self.dirty_idle = std::ptr::null_mut();
            }
        }
    }

    pub unsafe fn manage_start(&mut self) {
        assert!(matches!(self.state, WindowManagerState::Idle));
        assert!(self.scheduled.dirty);
        self.clean_windowing();
        self.state = WindowManagerState::Manage;

        log::debug!("manage sequence start");

        let session_locked = (*self.server).lock_manager.state == crate::lock_manager::LockState::Locked;
        if session_locked != self.sent.session_locked {
            if !self.object.is_null() {
                if session_locked {
                    ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_MANAGER_V1_SESSION_LOCKED);
                } else {
                    ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_MANAGER_V1_SESSION_UNLOCKED);
                }
            }
            self.sent.session_locked = session_locked;
        }

        let mt0 = if manage_debug() { Some(std::time::Instant::now()) } else { None };

        (*self.server).om.auto_layout();
        let mt_auto = mt0.map(|s| s.elapsed().as_micros());

        let outputs = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs).next;
        while curr != outputs {
            let next = (*curr).next;
            let output = crate::container_of!(curr, crate::output::Output, link);
            (*output).manage_start();
            curr = next;
        }

        let mt_outputs = mt0.map(|s| s.elapsed().as_micros());

        if !self.sent.output_config.is_null() {
            log::warn!("sent.output_config was not null in manage_start, destroying old configuration");
            ffi::wlr_output_configuration_v1_send_failed(self.sent.output_config);
            ffi::wlr_output_configuration_v1_destroy(self.sent.output_config);
            self.sent.output_config = std::ptr::null_mut();
        }
        self.sent.output_config = self.scheduled.output_config;
        self.scheduled.output_config = std::ptr::null_mut();

        for &win_ptr in self.windows.iter() {
            (*win_ptr).manage_start();
        }
        let mt_windows = mt0.map(|s| s.elapsed().as_micros());

        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            (*seat).manage_start();
            curr = next;
        }

        let mt_seats = mt0.map(|s| s.elapsed().as_micros());

        self.arrange_views();
        self.debug_check_unlinked_status("manage_start end");

        if let (Some(s), Some(a), Some(o), Some(w), Some(t)) =
            (mt0, mt_auto, mt_outputs, mt_windows, mt_seats)
        {
            let total = s.elapsed().as_micros();
            log::info!(
                "[manage] start total={}us auto_layout={}us outputs={}us windows={}us(n={}) seats={}us arrange={}us",
                total, a, o - a, w - o, self.windows.count(), t - w, total - t
            );
        }

        if !self.object.is_null() {
            ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_MANAGER_V1_MANAGE_START);
            self.start_timeout_timer(3000);
        } else {
            self.manage_finish();
        }
    }

    /// Wedge tracer: a Mapped status window outside the render list is
    /// invisible to configures and render_finish — exactly the tray
    /// mis-slot wedge. Silent unless one exists.
    unsafe fn debug_check_unlinked_status(&self, phase: &str) {
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed {
                continue;
            }
            if !matches!((*w).state, crate::window::WindowState::Mapped) {
                continue;
            }
            if (*w).is_linked() {
                continue;
            }
            if (*w).get_app_id_string().map_or(false, |id| id.starts_with("cce-status")) {
                log::info!("[LinkDbg] UNLINKED-MAPPED at {}: app={:?} link.prev_self={} link.prev_null={}",
                    phase,
                    (*w).get_app_id_string(),
                    (*w).node.link.prev == &(*w).node.link as *const ffi::wl_list as *mut ffi::wl_list,
                    (*w).node.link.prev.is_null());
            }
        }
        self.debug_check_render_list(phase);
    }

    /// Structural check of rendering_requested.list: every member's neighbor
    /// pointers must agree, and every Mapped status window must be reachable
    /// from the head. Silent when consistent.
    unsafe fn debug_check_render_list(&self, phase: &str) {
        let head = &self.rendering_requested.list as *const ffi::wl_list as *mut WlList;
        let mut members: Vec<*mut WlList> = Vec::new();
        let mut curr = (*head).next;
        let mut steps = 0;
        while curr != head {
            if curr.is_null() {
                log::info!("[LinkDbg] LIST BROKEN at {}: null next after {} steps", phase, steps);
                return;
            }
            if (*(*curr).next).prev != curr {
                log::info!("[LinkDbg] LIST INCONSISTENT at {}: member {:p} next.prev mismatch", phase, curr);
            }
            members.push(curr);
            curr = (*curr).next;
            steps += 1;
            if steps > 10000 {
                log::info!("[LinkDbg] LIST CYCLE at {}: >10000 members", phase);
                return;
            }
        }
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed {
                continue;
            }
            if !matches!((*w).state, crate::window::WindowState::Mapped) {
                continue;
            }
            if !(*w).get_app_id_string().map_or(false, |id| id.starts_with("cce-status")) {
                continue;
            }
            let node = &(*w).node.link as *const ffi::wl_list as *mut WlList;
            let reachable = members.contains(&node);
            if (*w).is_linked() && !reachable {
                log::info!("[LinkDbg] ORPHAN-RING at {}: app={:?} is_linked=true but unreachable from head",
                    phase, (*w).get_app_id_string());
            }
        }
    }

    pub unsafe fn manage_finish(&mut self) {
        assert!(matches!(self.state, WindowManagerState::Manage));
        self.cancel_timeout_timer();

        log::debug!("manage sequence finish");

        let seats = &mut self.sent.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link_sent);
            (*seat).manage_finish();
            curr = next;
        }

        self.state = WindowManagerState::InflightConfigures(0);

        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            match (*node).get() {
                crate::wm_node::WmNodeType::Window(window) => {
                    if (*window).manage_finish() {
                        if !(*window).wm_requested.resizing {
                            if let WindowManagerState::InflightConfigures(ref mut count) = self.state {
                                *count += 1;
                            }
                        }
                    }
                }
                _ => {}
            }
            curr = next;
        }

        if let WindowManagerState::InflightConfigures(count) = self.state {
            log::debug!("sent {} tracked configure(s)", count);
            self.debug_check_unlinked_status("manage_finish end");
            if count > 0 {
                self.start_timeout_timer(100);
            } else {
                self.render_start();
            }
        }
    }

    unsafe fn start_timeout_timer(&mut self, ms: u32) {
        if !self.timeout.is_null() {
            ffi::wl_event_source_timer_update(self.timeout, ms as i32);
        }
    }

    unsafe fn cancel_timeout_timer(&mut self) {
        if !self.timeout.is_null() {
            ffi::wl_event_source_timer_update(self.timeout, 0);
        }
    }

    pub unsafe fn notify_configured(&mut self) {
        if let WindowManagerState::InflightConfigures(ref mut count) = self.state {
            *count -= 1;
            if *count == 0 {
                self.cancel_timeout_timer();
                self.render_start();
            }
        }
    }

    pub unsafe fn render_start(&mut self) {
        assert!(matches!(self.state, WindowManagerState::InflightConfigures(0)) ||
                (matches!(self.state, WindowManagerState::Idle) && self.rendering_scheduled.dirty));
        self.state = WindowManagerState::Render;
        self.clean_rendering();

        log::debug!("render sequence start");

        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            match (*node).get() {
                crate::wm_node::WmNodeType::Window(window) => {
                    (*window).render_start();
                }
                _ => {}
            }
            curr = next;
        }

        if !self.object.is_null() {
            ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_MANAGER_V1_RENDER_START);
            self.start_timeout_timer(3000);
        } else {
            self.render_finish();
        }
    }

    pub unsafe fn render_finish(&mut self) {
        assert!(matches!(self.state, WindowManagerState::Render));
        self.state = WindowManagerState::Idle;
        self.cancel_timeout_timer();

        let rf0 = if manage_debug() { Some(std::time::Instant::now()) } else { None };

        log::debug!("render sequence finish");

        for &window in self.windows.iter() {
            if !matches!((*window).state, crate::window::WindowState::Closing) {
                (*window).surfaces.drop_saved();
            }
            if matches!((*window).state, crate::window::WindowState::Init) {
                ffi::wlr_scene_node_reparent((*window).tree as *mut ffi::wlr_scene_node, (*self.server).scene.hidden_tree);
            }
            if let crate::window::WindowImpl::Destroying = (*window).impl_type {
                Window::destroy(window);
            }
        }

        let has_wallpaper = self.windows.iter().any(|&w| !w.is_null() && !(*w).closed && (*w).is_wallpaper());
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let next_out = (*curr_out).next;
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                if !(*output).background_rect.is_null() {
                    ffi::wlr_scene_node_set_enabled((*output).background_rect as *mut ffi::wlr_scene_node, !has_wallpaper);
                }
            }
            curr_out = next_out;
        }

        self.keep_status_bar_on_top();

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.layout.overlay_behavior.hash(&mut hasher);
        let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            match (*node).get() {
                crate::wm_node::WmNodeType::Window(window) => {
                    (*window).ref_key.hash(&mut hasher);
                    rendered_fullscreen(window).hash(&mut hasher);
                    (*window).rendering_requested.circular.hash(&mut hasher);
                    (*window).rendering_requested.hidden.hash(&mut hasher);
                    (*window).tiling_mode.hash(&mut hasher);
                    // A status segment's layer flips between top and popups
                    // on expand/contract (see the reorder pass below), so
                    // expansion state must participate in the hash — without
                    // it the restack waits for an unrelated reorder, and the
                    // open menu sits UNDER its sibling segments (their text
                    // stays unblurred over the menu) until one happens.
                    if (*window).tiling_mode == crate::tiling::TilingMode::Status {
                        let bg = (*window).box_geom;
                        let thickness = match (*window).status_edge {
                            crate::policy::arrange::StatusEdge::Left
                            | crate::policy::arrange::StatusEdge::Right => bg.width,
                            _ => bg.height,
                        };
                        (thickness > self.layout.bar_height).hash(&mut hasher);
                    }
                }
                crate::wm_node::WmNodeType::ShellSurface(shell_surface) => {
                    (shell_surface as usize).hash(&mut hasher);
                }
            }
            curr = next;
        }
        let new_order_hash = hasher.finish();
        let reorder = self.rendering_requested.order_hash != new_order_hash;
        self.rendering_requested.order_hash = new_order_hash;
        let mut found_fullscreen = false;
        curr = (*render_list).next;
        while curr != render_list {
            let next = (*curr).next;
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            match (*node).get() {
                crate::wm_node::WmNodeType::Window(window) => {
                    (*window).render_finish();
                    if reorder {
                        {
                            // Viewport-hidden windows are NOT parked under the
                            // disabled hidden_tree: they keep their normal layer
                            // parent and stacking slot, hidden purely by their
                            // disabled node (render_finish and
                            // render_viewport_update both own that flag).
                            // Un-hiding happens on camera-motion frames, which
                            // never run this reorder pass — a window parked here
                            // stayed invisible after scrolling into view until
                            // the next unrelated transaction reparented it (the
                            // off-screen reveal delay in overview/zoom).
                            let layer = if (*window).get_app_id_string().as_deref() == Some("cce-wallpaper") {
                                // Between the native backdrop and the fallback
                                // cells, like a layer-shell Background surface.
                                (*self.server).scene.layers.background_clients
                            } else if (*window).is_grid() {
                                // The grid client is a desktop fixture: above
                                // the native backdrop and fallback cells
                                // (layers.background) but under every window.
                                // Left to the generic wm arm it stacks by
                                // render-list order, burying whichever windows
                                // happened to map before it.
                                (*self.server).scene.layers.bottom
                            } else if rendered_fullscreen(window) {
                                (*self.server).scene.layers.fullscreen
                            } else if (*window).tiling_mode == crate::tiling::TilingMode::Popup {
                                (*self.server).scene.layers.popups
                            } else if (*window).tiling_mode == crate::tiling::TilingMode::Status {
                                // An EXPANDED segment (in-surface menu open;
                                // thicker than the bar) stacks like a popup:
                                // this loop re-raises every window in
                                // render-list order each pass, so leaving it
                                // in the shared Status layer let whichever
                                // sibling rendered last cover the menu's
                                // strip band (the strip-band click routing
                                // bug — an arrange-time raise was clobbered
                                // here every frame).
                                let bg = (*window).box_geom;
                                let thickness = match (*window).status_edge {
                                    crate::policy::arrange::StatusEdge::Left
                                    | crate::policy::arrange::StatusEdge::Right => bg.width,
                                    _ => bg.height,
                                };
                                if thickness > self.layout.bar_height {
                                    (*self.server).scene.layers.popups
                                } else {
                                    (*self.server).scene.layers.top
                                }
                            } else if (*window).rendering_requested.circular {
                                (*self.server).scene.layers.top
                            } else if (*window).tiling_mode == crate::tiling::TilingMode::Overlay && self.layout.overlay_behavior == "above" {
                                (*self.server).scene.layers.top
                            } else {
                                (*self.server).scene.layers.wm
                            };

                            ffi::wlr_scene_node_reparent((*window).tree as *mut _, layer);
                            if (*window).get_app_id_string().as_deref() == Some("cce-wallpaper") {
                                ffi::wlr_scene_node_lower_to_bottom((*window).tree as *mut _);
                            } else {
                                ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                            }
                            if !(*window).rendering_requested.hidden && rendered_fullscreen(window) {
                                found_fullscreen = true;
                            }

                            ffi::wlr_scene_node_reparent((*window).popup_tree as *mut _, layer);
                            ffi::wlr_scene_node_place_above((*window).popup_tree as *mut _, (*window).tree as *mut _);
                        }
                    }
                }
                crate::wm_node::WmNodeType::ShellSurface(shell_surface) => {
                    (*shell_surface).render_finish();
                    if reorder {
                        let layer = if found_fullscreen {
                            (*self.server).scene.layers.fullscreen
                        } else {
                            (*self.server).scene.layers.wm
                        };

                        ffi::wlr_scene_node_reparent((*shell_surface).tree as *mut _, layer);
                        ffi::wlr_scene_node_raise_to_top((*shell_surface).tree as *mut _);

                        ffi::wlr_scene_node_reparent((*shell_surface).popup_tree as *mut _, layer);
                        ffi::wlr_scene_node_place_above((*shell_surface).popup_tree as *mut _, (*shell_surface).tree as *mut _);
                    }
                }
            }
            curr = next;
        }

        // Floating windows are a plane IN FRONT of the tiled ones: a tiled
        // window never covers a floating one, however recently it was raised.
        // The loop above stacks layers.wm in render-list order alone, so
        // clicking a tiled window buried every floating window it overlaps —
        // and the two modes are meant to be independent, not interleaved.
        //
        // Re-applied on every reorder pass, walking the render list again so
        // each plane keeps its OWN relative stacking: the floating windows
        // come out in the order they were raised, above the tiled ones in the
        // order they were raised. A rule in the stacking authority, like the
        // light_source raise below — `raise_window` cannot own it, because
        // every other path that reorders the list would then have to know it.
        //
        // Only the windows the loop actually parked in layers.wm take part,
        // tested through the parent it just set: a fullscreen, popup, status
        // or circular window lives in a layer of its own, where raising it
        // would reshuffle that layer's members for no reason. `Utility` is
        // floating furniture too (a client-declared tool window — it floats
        // and moves like any other), so it rides in the same plane; `Overlay`
        // keeps its own `overlay_behavior` rule and stays out of this.
        if reorder {
            let wm_layer = (*self.server).scene.layers.wm;
            let mut focused_popups: *mut Window = std::ptr::null_mut();
            curr = (*render_list).next;
            while curr != render_list {
                let next = (*curr).next;
                let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
                if let crate::wm_node::WmNodeType::Window(window) = (*node).get() {
                    let floats = matches!(
                        (*window).tiling_mode,
                        crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Utility
                    );
                    let in_wm_layer = !(*window).tree.is_null()
                        && !wm_layer.is_null()
                        && ffi::river_scene_node_get_parent((*window).tree as *mut _) == wm_layer;
                    if floats && in_wm_layer {
                        ffi::wlr_scene_node_raise_to_top((*window).tree as *mut _);
                        ffi::wlr_scene_node_place_above(
                            (*window).popup_tree as *mut _,
                            (*window).tree as *mut _,
                        );
                    }
                    // Last, so an open menu clears the plane it was just
                    // stacked behind (`raise_focused_popups`).
                    focused_popups = if (*window).is_seat_focused() { window } else { focused_popups };
                }
                curr = next;
            }
            if !focused_popups.is_null() {
                self.raise_focused_popups(focused_popups);
            }
        }

        // The traveling light_source segment crosses over its sibling
        // segments; raise it after the loop so it stacks in front of them
        // within its layer regardless of render-list order. Re-applied on
        // every reorder pass — a rule in the stacking authority, not a
        // one-shot raise.
        if reorder {
            for &w in self.windows.iter() {
                if !w.is_null()
                    && !(*w).closed
                    && matches!((*w).state, crate::window::WindowState::Mapped)
                    && (*w).get_app_id_string().map_or(false, |id| id.ends_with("light_source"))
                {
                    ffi::wlr_scene_node_raise_to_top((*w).tree as *mut ffi::wlr_scene_node);
                    ffi::wlr_scene_node_place_above((*w).popup_tree as *mut _, (*w).tree as *mut _);
                }
            }
        }

        (*self.server).om.commit_output_state(self.server);

        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            (*seat).cursor.update_hovered();
            curr = next;
        }

        (*self.server).idle_inhibit_manager.check_active();

        log::debug!("finished committing transaction");
        self.debug_check_unlinked_status("render_finish end");

        if self.scheduled.dirty || self.scheduled.dirty_lazy || self.rendering_scheduled.dirty {
            self.add_dirty_idle();
        }
        self.layout_epoch = self.layout_epoch.wrapping_add(1);
        self.schedule_save_state();
        if let Some(r) = rf0 {
            log::info!("[manage] render_finish total={}us", r.elapsed().as_micros());
        }
    }

    /// Write the state file soon, once, no matter how many transactions land
    /// in the meantime. The first call after a save arms the timer; later
    /// calls before it fires are absorbed, so a burst of transactions costs
    /// one save at most `SAVE_STATE_DELAY_MS` behind the last change.
    pub unsafe fn schedule_save_state(&mut self) {
        if self.shutting_down || self.save_state_pending {
            return;
        }
        if self.save_state_timer.is_null() {
            let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
            self.save_state_timer = ffi::wl_event_loop_add_timer(
                event_loop,
                Some(handle_save_state_timer),
                self as *mut WindowManager as *mut _,
            );
            if self.save_state_timer.is_null() {
                log::error!("failed to create the save-state timer; saving synchronously");
                self.save_state();
                return;
            }
        }
        self.save_state_pending = true;
        ffi::wl_event_source_timer_update(self.save_state_timer, SAVE_STATE_DELAY_MS);
    }
}

// Deprecated sent_outputs that is part of structural layout compatibility
pub struct WindowManagerScheduledCompat {
    pub output_config: *mut ffi::wlr_output_configuration_v1,
}
pub struct WindowManagerSentCompat {
    pub outputs: ffi::wl_list,
    pub output_config: *mut ffi::wlr_output_configuration_v1,
}
impl WindowManager {
    // Add legacy fields so structural offsets are preserved if layout-based code is compiled
    pub fn sent_outputs_compat(&self) {}

    pub unsafe fn get_rule_for_window(&self, win: *mut Window) -> Option<&crate::config::ModeRule> {
        let app_id = (*win).get_app_id_string();
        let title = (*win).get_title_string();

        for rule in &self.mode_rules {
            let match_app = rule.app_id_pattern == "*"
                || app_id.as_ref().map_or(false, |aid| aid.contains(&rule.app_id_pattern));
            let match_title = rule.title_pattern.as_ref().map_or(true, |tp| {
                title.as_ref().map_or(false, |t| t.contains(tp))
            });

            if match_app && match_title {
                return Some(rule);
            }
        }
        None
    }

    pub unsafe fn get_mode_for_window(&self, win: *mut Window) -> crate::tiling::TilingMode {
        if (*win).is_status_bar() {
            return crate::tiling::TilingMode::Status;
        }
        let app_id = (*win).get_app_id_string();
        if app_id.as_deref() == Some("cce-notifier") || app_id.as_deref() == Some("cce-notification-daemon") || app_id.as_deref() == Some("clear-notification-daemon") {
            return crate::tiling::TilingMode::Popup;
        }
        // An explicit set_popup via the cce window-management protocol beats the
        // app_id heuristic below: a cce-cloud toplevel that flagged itself a popup
        // sizes itself (dmenu-style) instead of taking the overlay dock's
        // full-height fresh slot.
        if (*win).tiling_mode == crate::tiling::TilingMode::Popup {
            return crate::tiling::TilingMode::Popup;
        }
        if app_id.as_deref().map_or(false, |id| id.starts_with("cce-cloud")) {
            return crate::tiling::TilingMode::Overlay;
        }



        if (*win).mode_locked {
            return (*win).tiling_mode;
        }

        if (*win).has_parent {
            return crate::tiling::TilingMode::Floating;
        }

        if let Some(rule) = self.get_rule_for_window(win) {
            return rule.mode;
        }

        crate::tiling::TilingMode::Floating
    }

    pub unsafe fn get_active_resize_dimensions(&self, win_ptr: *mut Window) -> Option<(u32, u32)> {
        let seats_list = &(*self.server).input_manager.seats as *const ffi::wl_list as *const WlList as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let Some(ref op) = (*seat).op {
                if op.window_ptr == win_ptr {
                    if let crate::seat::PointerOpType::Resize { edges } = op.op_type {
                        let scale = self.desk_zoom;
                        let dx = op.x - op.start_x;
                        let dy = op.y - op.start_y;
                        let virtual_dx = dx as f64 / scale + (self.desk_pan_x - op.start_pan_x);
                        let virtual_dy = dy as f64 / scale + (self.desk_pan_y - op.start_pan_y);
                        // Same math (and snapping) as the seat op's Resize
                        // arm — this recomputation feeds the arrange
                        // snapshot and must not diverge from it.
                        let sp = self.layout.snap_params().for_zoom(self.desk_zoom);
                        let new_w = crate::policy::snap::resize_axis(
                            op.start_win_virtual_x, op.start_win_w as f64, virtual_dx,
                            edges.left, edges.right, 50.0, &sp.x(),
                        ) as u32;
                        let new_h = crate::policy::snap::resize_axis(
                            op.start_win_virtual_y, op.start_win_h as f64, virtual_dy,
                            edges.top, edges.bottom, 50.0, &sp.y(),
                        ) as u32;
                        // Same clamp as the seat op (see its Resize arm).
                        return Some((*win_ptr).wm_scheduled.dimensions_hint.clamp(new_w, new_h));
                    }
                }
            }
            curr_seat = (*curr_seat).next;
        }
        None
    }

    pub unsafe fn is_window_being_moved(&self, win_ptr: *mut Window) -> bool {
        let seats_list = &(*self.server).input_manager.seats as *const ffi::wl_list as *const WlList as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let Some(ref op) = (*seat).op {
                if op.window_ptr == win_ptr {
                    if let crate::seat::PointerOpType::Move = op.op_type {
                        return true;
                    }
                }
            }
            curr_seat = (*curr_seat).next;
        }
        false
    }



    /// Snap the camera out of overview and onto `win`: zoom 1, centered,
    /// mode Normal. The overview click-release path in cursor.rs does the
    /// same dance inline (plus its focus/seat-event bookkeeping); this is
    /// the map-time variant for windows SPAWNED during overview.
    pub unsafe fn exit_overview_to_window(&mut self, win: *mut Window) {
        if win.is_null() {
            return;
        }
        let (mut viewport_w, mut viewport_h) = (1920.0_f64, 1080.0_f64);
        let outputs_list = &(*self.server).om.outputs as *const ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let wlr_box = (*output).sent.box_layout();
                viewport_w = wlr_box.width as f64;
                viewport_h = wlr_box.height as f64;
                break;
            }
            curr_out = (*curr_out).next;
        }
        let win_w = if (*win).box_geom.width > 0 { (*win).box_geom.width as f64 } else { 800.0 };
        let win_h = if (*win).box_geom.height > 0 { (*win).box_geom.height as f64 } else { 600.0 };
        let center_x = (*win).virtual_x + win_w / 2.0;
        let center_y = (*win).virtual_y + win_h / 2.0;
        // Same animated flight as the Overview toggle's exit: SetCamera
        // owns the ramp/target bookkeeping and flips the mode by fiat.
        self.stop_panning_animation();
        crate::policy::api::Compositor::apply(
            self,
            &crate::policy::api::Command::SetCamera {
                camera: crate::policy::camera::Camera {
                    pan_x: center_x - viewport_w / 2.0,
                    pan_y: center_y - viewport_h / 2.0,
                    zoom: 1.0,
                },
                overview: Some(false),
                animate: true,
            },
        );
        crate::policy::api::Compositor::apply(self, &crate::policy::api::Command::RefreshCamera);
    }

    /// Issue grid_patch events to grid clients whose current patch no
    /// longer comfortably covers the viewport (or whose buffer resolution
    /// has drifted more than 2x from the zoom). One patch in flight per
    /// window; a failed send (no toplevel resource yet, old client) simply
    /// retries on a later pass.
    pub unsafe fn update_grid_patches(&mut self) {
        let mut out_box: Option<(ffi::wlr_box, f64)> = None;
        let outputs_list = &(*self.server).om.outputs as *const ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                out_box = Some(((*output).sent.box_layout(), (*output).sent.scale.max(1.0) as f64));
                break;
            }
            curr_out = (*curr_out).next;
        }
        let Some((out, out_scale)) = out_box else { return };
        let zoom = crate::policy::background::sanitized_zoom(self.desk_zoom);
        let vw = out.width as f64 / zoom;
        let vh = out.height as f64 / zoom;
        let (vx, vy) = (self.desk_pan_x, self.desk_pan_y);

        // The camera flight's destination, if one is running: the ramp
        // animation's target (overview enter/exit), else the exponential
        // pan/zoom targets. Patches anticipate the destination — coverage
        // spans the union of the current and target viewports, and the
        // resolution quantizes for the destination — so a flight needs ONE
        // patch that is already correct when it lands, instead of chasing
        // the interpolated camera (which exposed cell-less backdrop at the
        // leading edge of an overview enter, and left overview exits
        // resting 2x-magnified with the swap landing at the animation's
        // end as a visible readjust).
        let target_cam: Option<crate::policy::camera::Camera> = self
            .camera_ramp_anim
            .as_ref()
            .map(|a| a.target)
            .or_else(|| {
                if self.target_desk_pan_x.is_some()
                    || self.target_desk_pan_y.is_some()
                    || self.target_desk_zoom.is_some()
                {
                    Some(crate::policy::camera::Camera {
                        pan_x: self.target_desk_pan_x.unwrap_or(self.desk_pan_x),
                        pan_y: self.target_desk_pan_y.unwrap_or(self.desk_pan_y),
                        zoom: crate::policy::background::sanitized_zoom(
                            self.target_desk_zoom.unwrap_or(self.desk_zoom),
                        ),
                    })
                } else {
                    None
                }
            })
            .or_else(|| {
                // No explicit destination: predict one from the kinetic
                // coast (an exponential decay travels v/friction more) or a
                // live finger gesture (~0.3s of its current velocity), so the
                // patch is issued toward where the pan is heading before the
                // viewport reaches the current patch's edge.
                let (vx_, vy_) = if self.pan_coast_vx != 0.0 || self.pan_coast_vy != 0.0 {
                    let f = self.scroll_friction();
                    (self.pan_coast_vx / f, self.pan_coast_vy / f)
                } else if self.pan_finger_v != [0.0, 0.0] {
                    (self.pan_finger_v[0] * 0.3, self.pan_finger_v[1] * 0.3)
                } else {
                    return None;
                };
                Some(crate::policy::camera::Camera {
                    pan_x: self.desk_pan_x + vx_,
                    pan_y: self.desk_pan_y + vy_,
                    zoom: crate::policy::background::sanitized_zoom(self.desk_zoom),
                })
            });
        let in_flight = self.viewport_is_active || target_cam.is_some();

        // Buffer px per virtual unit: the DESTINATION zoom quantized to a
        // power of two (small zoom wobbles don't re-render the world),
        // times the output scale so a buffer px is a NATIVE px at that
        // zoom — then raised in pow2 steps until the patch is also
        // displayable at the CURRENT zoom without exceeding 2x
        // magnification, so the swap moment never pops visibly soft.
        let q_zoom = crate::policy::background::sanitized_zoom(
            target_cam.map(|c| c.zoom).unwrap_or(self.desk_zoom),
        );
        let mut q = (2f64).powf(q_zoom.log2().round()).clamp(0.125, 2.0) * out_scale;
        let q_max = 2.0 * out_scale;
        while zoom * out_scale / q > 2.0 && q < q_max {
            q = (q * 2.0).min(q_max);
        }
        let period_x = self.layout.desktop_cell_width
            + (self.layout.desktop_gap_width as f64).max(0.0);
        let period_y = self.layout.desktop_cell_height
            + (self.layout.desktop_gap_width as f64).max(0.0);

        // Target viewport (virtual units), for the union coverage below.
        let target_rect = target_cam.map(|c| {
            let tw = out.width as f64 / c.zoom;
            let th = out.height as f64 / c.zoom;
            (c.pan_x, c.pan_y, tw, th, c.zoom)
        });

        // Resolution drift tolerance: permissive while the camera is moving
        // (a giant re-render per animation frame would be worse than a bit
        // of scaling), but at REST the patch must sit within a band around
        // exact — the safety net that re-patches any path that settles
        // mis-resolved. The band includes 1.414 (a zoom exactly on a pow2
        // half-step boundary re-quantizes to the same q, so a settled
        // repatch can never loop) and reaches down to 0.45: an OVERSAMPLED
        // patch renders sharp and is only a memory cost, so a flight's
        // union patch may rest through a whole overview visit.
        // No lower bound in flight: oversampling renders sharp (memory-only
        // cost), and an exit-union patch is necessarily oversampled for
        // most of the flight (issued at destination resolution while the
        // zoom is still far out) — any in-flight floor re-sends the very
        // patch it just issued, every animation frame, until the zoom
        // crosses it. The rest band keeps a floor purely as the memory
        // trigger that swaps an oversized flight patch for a right-sized
        // one after landing somewhere it no longer suits.
        let (disp_lo, disp_hi) = if in_flight { (0.0, 2.01) } else { (0.40, 1.42) };
        let covers = |p: &crate::policy::api::GridPatch| -> bool {
            // Mid-flight the comfort demand on the CURRENT viewport drops
            // to near-bare: an exit union barely fits the buffer cap (zero
            // slack margin), and a 0.15-viewport demand poking past it
            // re-patched every animation frame — a 13-patch storm per
            // overview exit. The destination side of the union carries its
            // own comfort for the landing; full comfort applies at rest.
            let (mx, my) = if in_flight {
                (vw * 0.02, vh * 0.02)
            } else {
                (vw * 0.15, vh * 0.15)
            };
            // Resolution drift is native px per buffer px, so the output
            // scale belongs on the zoom side — measuring against zoom
            // alone would reject every native-res patch on a scaled
            // output (display factor 0.5 at scale 2) and repatch forever.
            let disp = zoom * out_scale / p.scale;
            let now_ok = p.x <= vx - mx
                && p.y <= vy - my
                && p.x + p.w >= vx + vw + mx
                && p.y + p.h >= vy + vh + my
                && disp > disp_lo
                && disp < disp_hi;
            // Mid-flight the patch must also suit the DESTINATION (small
            // 0.05 comfort — the union patch is sized with 0.10, so this
            // demand always fits what was sent). A patch that fails only
            // here keeps displaying while its replacement renders.
            let target_ok = target_rect.map_or(true, |(tx, ty, tw, th, tz)| {
                let tmx = tw * 0.05;
                let tmy = th * 0.05;
                let tdisp = tz * out_scale / p.scale;
                // Wide lower bound: the issuance q may be raised well above
                // the target's nominal for current-zoom displayability
                // (deep overview enters), and a bound that rejects the
                // patch we just issued is a re-send storm. Long-term
                // oversampling is corrected once by the rest band.
                p.x <= tx - tmx
                    && p.y <= ty - tmy
                    && p.x + p.w >= tx + tw + tmx
                    && p.y + p.h >= ty + th + tmy
                    && tdisp > 0.15
                    && tdisp < 1.42
            });
            now_ok && target_ok
        };
        let explain = |tag: &str, p: &crate::policy::api::GridPatch| {
            if std::env::var("CCE_GRID_DEBUG").is_err() {
                return;
            }
            let mx = vw * 0.02;
            let disp = zoom * out_scale / p.scale;
            log::info!(
                "[GridDbg] {tag} fails: z={zoom:.3} patch=({:.0},{:.0} {:.0}x{:.0} @{:.2}) now_area={} disp={disp:.2} tgt={:?}",
                p.x, p.y, p.w, p.h, p.scale,
                p.x <= vx - mx && p.x + p.w >= vx + vw + mx && p.y <= vy - vh * 0.02 && p.y + p.h >= vy + vh * 1.02,
                target_rect.map(|(tx, ty, tw, th, tz)| {
                    (p.x <= tx - tw * 0.05 && p.x + p.w >= tx + tw * 1.05
                        && p.y <= ty - th * 0.05 && p.y + p.h >= ty + th * 1.05,
                     tz * out_scale / p.scale)
                }),
            );
        };
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || !(*w).is_grid() {
                continue;
            }
            if !matches!((*w).state, crate::window::WindowState::Mapped) {
                continue;
            }
            // A stale patch (style changed since it was rendered) is
            // re-issued regardless of coverage; a covering patch already in
            // flight is left to latch first, and the flag then re-sends
            // once it has become current.
            let stale = (*w).grid_patch_stale;
            if !stale && (*w).grid_patch_current.as_ref().map_or(false, &covers) {
                continue;
            }
            if let Some(cur) = &(*w).grid_patch_current {
                explain(if stale { "stale" } else { "current" }, cur);
            }
            if let Some((_, pending)) = &(*w).grid_patch_pending {
                if covers(pending) {
                    continue;
                }
                explain("pending", pending);
            }
            if let Some((serial, acked)) = &(*w).grid_patch_acked {
                // Rendered but not yet committed: give it a frame.
                let _ = (serial, acked);
                continue;
            }
            // Coverage: the current viewport, unioned with the flight's
            // destination viewport (+0.10 comfort) when one is known. Then
            // the remaining buffer budget spreads as margin per side and
            // the rect period-aligns outward so the client draws whole
            // cells. Margin and cap are a MEMORY knob: the client's
            // framebuffer is (patch * q)^2 * 4B per swapchain image (q
            // carries the output scale) — the original 3x3-viewport margin
            // cost ~340MB per image, for scroll headroom that the 0.15
            // comfort margin rarely used. The cap is scale-aware (4096 *
            // out_scale keeps the same VIRTUAL coverage at every scale) so
            // margins can never shrink below what covers() demands.
            let mut ux0 = vx;
            let mut uy0 = vy;
            let mut ux1 = vx + vw;
            let mut uy1 = vy + vh;
            if let Some((tx, ty, tw, th, _)) = target_rect {
                ux0 = ux0.min(tx - 0.10 * tw);
                uy0 = uy0.min(ty - 0.10 * th);
                ux1 = ux1.max(tx + 1.10 * tw);
                uy1 = uy1.max(ty + 1.10 * th);
            }
            let uw = ux1 - ux0;
            let uh = uy1 - uy0;
            let max_buf: f64 = 4096.0 * out_scale;
            if uw * q > max_buf || uh * q > max_buf {
                // The union doesn't fit yet (an overview exit while still
                // zoomed far out needs a native-res patch bigger than the
                // cap): keep displaying the old patch and retry as the
                // viewport shrinks toward the destination.
                continue;
            }
            // The patch is a FIXED size for a given resolution: the largest
            // whole-period rect within the buffer cap, centered on the union
            // and period-aligned. Consecutive patches during a pan then have
            // identical buffer extents, so the client's swapchain survives
            // the swap — the old outward-aligned rect varied by a couple of
            // periods between patches, and every size change rebuilt a
            // 200MB swapchain and dropped a frame mid-gesture. (It also
            // overran the cap by up to two periods.)
            let fw = ((max_buf / q) / period_x).floor().max(1.0) * period_x;
            let fh = ((max_buf / q) / period_y).floor().max(1.0) * period_y;
            let place = |u0: f64, u1: f64, f: f64, period: f64| -> Option<f64> {
                let center = (u0 + u1) * 0.5;
                let mut p0 = ((center - f * 0.5) / period).floor() * period;
                if p0 + f < u1 {
                    p0 += period;
                }
                (p0 <= u0 && p0 + f >= u1).then_some(p0)
            };
            let (x0, y0, pw, ph) = match (place(ux0, ux1, fw, period_x), place(uy0, uy1, fh, period_y)) {
                (Some(x0), Some(y0)) => (x0, y0, fw, fh),
                _ => {
                    // The union nearly fills the cap (an overview flight's
                    // union): the legacy outward alignment, exact-fit.
                    let m = (((max_buf / q) - uw) / (2.0 * uw)).clamp(0.0, 0.5)
                        .min((((max_buf / q) - uh) / (2.0 * uh)).clamp(0.0, 0.5));
                    let x0 = ((ux0 - m * uw) / period_x).floor() * period_x;
                    let y0 = ((uy0 - m * uh) / period_y).floor() * period_y;
                    let x1 = ((ux1 + m * uw) / period_x).ceil() * period_x;
                    let y1 = ((uy1 + m * uh) / period_y).ceil() * period_y;
                    (x0, y0, x1 - x0, y1 - y0)
                }
            };
            let patch = crate::policy::api::GridPatch { x: x0, y: y0, w: pw, h: ph, scale: q };
            (*w).grid_patch_serial = (*w).grid_patch_serial.wrapping_add(1);
            let serial = (*w).grid_patch_serial;
            if (*self.server)
                .cce_window_management
                .send_grid_patch((*w).ref_key, serial, patch)
            {
                log::info!("[Grid] sent patch #{serial}: {:.0},{:.0} {:.0}x{:.0} @{:.3}{}",
                    patch.x, patch.y, patch.w, patch.h, patch.scale,
                    if stale { " (style reload)" } else { "" });
                (*w).grid_patch_pending = Some((serial, patch));
                (*w).grid_patch_stale = false;
            } else {
                log::info!("[Grid] patch #{serial} not sent (no toplevel resource yet)");
            }
        }
    }

    /// Mark every grid client's rendered patch stale so `update_grid_patches`
    /// re-issues it on the next arrange even though its coverage is still
    /// fine. The grid client is a pure function of (patch, style config) and
    /// repaints only when handed a patch, so after a config reload — or a
    /// `layout` change to a desktop key — an unmoved viewport kept showing
    /// the OLD cell size and colors until the camera happened to travel far
    /// enough to need a fresh patch.
    pub unsafe fn invalidate_grid_patches(&mut self) {
        for &w in self.windows.iter() {
            if !w.is_null() && !(*w).closed && (*w).is_grid() {
                (*w).grid_patch_stale = true;
            }
        }
    }

    pub unsafe fn arrange_views(&mut self) {
        self.update_grid_patches();
        self.update_restore_placeholders();
        if arrange_debug() {
            log::debug!("Monolithic arrange_views triggered. Windows: {}", self.windows.count());
            for (idx, &win_ptr) in self.windows.iter().enumerate() {
                if win_ptr.is_null() { continue; }
                let title = (*win_ptr).get_title_string().unwrap_or_else(|| "None".to_string());
                let aid = (*win_ptr).get_app_id_string().unwrap_or_else(|| "None".to_string());
                log::debug!("  window #{}: title={:?}, app_id={:?}, state={:?}, closed={}", idx, title, aid, (*win_ptr).state, (*win_ptr).closed);
            }
        }

        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;

        let mut active_outputs: Vec<*mut crate::output::Output> = Vec::new();
        while curr_out != outputs_list {
            let next_out = (*curr_out).next;
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                active_outputs.push(output);
            }
            curr_out = next_out;
        }

        if active_outputs.is_empty() {
            return;
        }

        let mut focused_window: *mut Window = std::ptr::null_mut();
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let crate::seat::Focus::Window(w) = (*seat).focused {
                focused_window = w;
                break;
            }
            curr_seat = next_seat;
        }

        // Snapshot outputs and windows into plain data, compute the whole
        // frame's plan in policy code, then apply it to the scene graph.
        let mut output_snaps: Vec<crate::policy::arrange::OutputSnapshot> = Vec::new();
        for &output in &active_outputs {
            let wlr_box = (*output).sent.box_layout();
            let non_ex = (*output).layer_shell.scheduled.non_exclusive_area;
            output_snaps.push(crate::policy::arrange::OutputSnapshot {
                layout_box: crate::policy::api::Rect {
                    x: wlr_box.x,
                    y: wlr_box.y,
                    width: wlr_box.width,
                    height: wlr_box.height,
                },
                non_exclusive: crate::policy::api::Rect {
                    x: non_ex.x,
                    y: non_ex.y,
                    width: non_ex.width,
                    height: non_ex.height,
                },
            });
        }

        let mut win_ptrs: Vec<*mut Window> = Vec::new();
        let mut window_snaps: Vec<crate::policy::arrange::WindowSnapshot> = Vec::new();
        for &win_ptr in self.windows.iter() {
            if win_ptr.is_null() || (*win_ptr).closed {
                continue;
            }
            let rule_ssd = if !(*win_ptr).mode_locked {
                self.get_rule_for_window(win_ptr).and_then(|rule| rule.ssd)
            } else {
                None
            };
            // Status segments: refresh the frozen collapsed slot length
            // while at bar thickness; while EXPANDED (in-surface menu, the
            // surface is thicker than the bar) keep the frozen value and
            // raise the segment above its siblings and the windows the open
            // menu now overlaps.
            if (*win_ptr).get_app_id_string().map_or(false, |id| id.starts_with("cce-status")) {
                let bg = (*win_ptr).box_geom;
                let (len, thickness) = match (*win_ptr).status_edge {
                    crate::policy::arrange::StatusEdge::Left
                    | crate::policy::arrange::StatusEdge::Right => (bg.height, bg.width),
                    _ => (bg.width, bg.height),
                };
                if thickness > 0 && thickness <= self.layout.bar_height {
                    (*win_ptr).status_collapsed_len = len;
                }
                // Stacking of the expanded segment lives in the
                // render_finish reorder pass (→ layers.popups), which
                // re-stacks every window each frame — a raise here was
                // clobbered by it.
            }
            window_snaps.push(crate::policy::arrange::WindowSnapshot {
                app_id: (*win_ptr).get_app_id_string(),
                title: (*win_ptr).get_title_string(),
                role: (*win_ptr).role(),
                minimized: (*win_ptr).minimized,
                closing_or_init: matches!((*win_ptr).state, crate::window::WindowState::Closing | crate::window::WindowState::Init),
                mode: self.get_mode_for_window(win_ptr),
                status_collapsed_len: (*win_ptr).status_collapsed_len,
                rule_ssd,
                being_moved: self.is_window_being_moved(win_ptr),
                status_edge: (*win_ptr).status_edge,
                is_focused: win_ptr == focused_window,
                box_geom: crate::policy::api::Rect {
                    x: (*win_ptr).box_geom.x,
                    y: (*win_ptr).box_geom.y,
                    width: (*win_ptr).box_geom.width,
                    height: (*win_ptr).box_geom.height,
                },
                min_size: (
                    (*win_ptr).wm_scheduled.dimensions_hint.min_width as i32,
                    (*win_ptr).wm_scheduled.dimensions_hint.min_height as i32,
                ),
                virtual_pos: ((*win_ptr).virtual_x, (*win_ptr).virtual_y),
                active_resize: self.get_active_resize_dimensions(win_ptr),
                ssd: (*win_ptr).wm_requested.ssd,
                decorations_size: (*win_ptr).measure_decorations(),
                was_tiled: (*win_ptr).was_tiled,
                saved_floating_size: ((*win_ptr).saved_floating_width, (*win_ptr).saved_floating_height),
                saved_floating_virtual: ((*win_ptr).saved_floating_virtual_x, (*win_ptr).saved_floating_virtual_y),
                grid_patch: (*win_ptr).grid_patch_current,
            });
            win_ptrs.push(win_ptr);
        }

        let params = crate::policy::arrange::ArrangeParams {
            bar_height: self.layout.bar_height,
            status_hide_mode: self.status_hide_mode,
            hide_mode_preview: self.layout.status_module_hide_mode_preview as i32,
            status_module_spacing: self.layout.status_module_spacing as i32,
            day_fraction: Some(local_day_fraction()),
            status_blur: self.layout.status_background_blur > 0.001,
            window_blur: self.layout.window_blur,
            opacity_enabled: self.layout.window_opacity,
            decoration: crate::policy::api::DecorationSpec {
                border_width: self.layout.border_width,
                border_color: crate::policy::api::Rgba(self.layout.border_color),
                corner_radius: self.layout.border_corner_radius,
            },
            border_color_focused: crate::policy::api::Rgba(self.layout.border_color_focused),
            overlay: crate::policy::arrange::OverlayParams {
                overlay_width: self.layout.overlay_width,
                border_gap: self.layout.overlay_border_gap,
                border_width: self.layout.border_width,
                position_right: self.layout.overlay_position == "right",
                cloud_position_default: self.layout.cloud_position_default,
            },
            normal: crate::policy::arrange::NormalParams {
                gap_right: self.layout.gap_right,
                gap_top: self.layout.gap_top,
                cloud_position_default: self.layout.cloud_position_default,
                desktop_cell_w: self.layout.desktop_cell_width,
                desktop_cell_h: self.layout.desktop_cell_height,
                desktop_gap_width: self.layout.desktop_gap_width as f64,
                desktop_cell_inset: self.layout.desktop_cell_fade_inset as f64,
            },
            pan_x: self.desk_pan_x,
            pan_y: self.desk_pan_y,
            zoom: self.desk_zoom,
        };

        let plan = crate::policy::arrange::arrange(&window_snaps, &output_snaps, &params);

        for &output in &active_outputs {
            if !(*output).background_rect.is_null() {
                ffi::wlr_scene_node_set_enabled((*output).background_rect as *mut ffi::wlr_scene_node, plan.background_rect_enabled);
            }
        }
        // During a camera flight the native cell lattice draws even while a
        // client patch is latched: the fallback renders BELOW the grid
        // client's surface, so it only shows through wherever the viewport
        // outruns the patch — filling the leading edge of an overview enter
        // with real cells for the frame or two the client needs to render
        // the flight's replacement patch (backdrop-only exposure was the
        // "cells at the bottom appear late" gap).
        // NOT a pure pan: enabling the cell pool is a three-frame rect
        // enable/redraw below every window, which reads to the scene as
        // content changing and re-bakes every blur at the start and end of
        // every pan. A pan that outruns its (prefetched, fixed-size) patch
        // briefly shows bare backdrop at the leading edge instead.
        let cells_wanted = plan.grid_cells_enabled
            || self.camera_ramp_anim.is_some()
            || self.target_desk_zoom.is_some();
        if self.grid_cells_enabled != cells_wanted {
            self.grid_cells_enabled = cells_wanted;
            // The cell pools redraw only on structure changes; force one so
            // the swap (client grid <-> compositor cells) is immediate.
            for &output in &active_outputs {
                (*output).grid_force_redraw_frames = 3;
            }
            // The swap changes backdrop content under the optimized-blur
            // capture set without any blur-node resize — re-bake or
            // translucent windows keep blurring the pre-swap grid.
            ffi::river_scene_mark_optimized_blur_dirty((*self.server).scene.wlr_scene);
        }

        for (&win_ptr, wp) in win_ptrs.iter().zip(plan.windows.iter()) {
            if let Some(enabled) = wp.scene_enabled {
                ffi::wlr_scene_node_set_enabled((*win_ptr).tree as *mut ffi::wlr_scene_node, enabled);
            }
            if let Some(hidden) = wp.hidden {
                (*win_ptr).rendering_requested.hidden = hidden;
            }
            if let Some(mode) = wp.tiling_mode {
                (*win_ptr).tiling_mode = mode;
            }
            if let Some(tiled) = wp.tiled {
                (*win_ptr).wm_requested.tiled = tiled;
            }
            if let Some(ssd) = wp.ssd {
                (*win_ptr).wm_requested.ssd = ssd;
            }
            if let Some(scale) = wp.scale {
                (*win_ptr).scale = scale;
            }
            if let Some((x, y)) = wp.pos {
                (*win_ptr).rendering_requested.x = x;
                (*win_ptr).rendering_requested.y = y;
            }
            if let Some(bg) = wp.box_geom {
                (*win_ptr).box_geom.x = bg.x;
                (*win_ptr).box_geom.y = bg.y;
                (*win_ptr).box_geom.width = bg.width;
                (*win_ptr).box_geom.height = bg.height;
            }
            if let Some((vx, vy)) = wp.virtual_pos {
                (*win_ptr).virtual_x = vx;
                (*win_ptr).virtual_y = vy;
            }
            if let Some((width, height)) = wp.size {
                (*win_ptr).wm_requested.dimensions = Some(crate::window::Dimensions { width, height });
                (*win_ptr).wm_requested.bounds = crate::window::Dimensions { width, height };
            }
            if let Some(dec) = wp.decoration {
                (*win_ptr).rendering_requested.border = crate::window::Border {
                    edges: crate::window::Edges { top: true, bottom: true, left: true, right: true },
                    width: dec.border_width.max(0) as u32,
                    color: dec.border_color.0,
                    hover_color: self.layout.border_color_hover,
                    corner_radius: dec.corner_radius.max(0),
                };
            }
            if let Some(blur) = wp.blur {
                (*win_ptr).rendering_requested.blur = blur;
            }
            if let Some(opacity) = wp.opacity {
                (*win_ptr).rendering_requested.opacity = opacity;
            }
            if let Some(((width, height), (vx, vy))) = wp.saved_floating {
                (*win_ptr).saved_floating_width = width;
                (*win_ptr).saved_floating_height = height;
                (*win_ptr).saved_floating_virtual_x = vx;
                (*win_ptr).saved_floating_virtual_y = vy;
            }
            if let Some(was_tiled) = wp.was_tiled {
                (*win_ptr).was_tiled = was_tiled;
            }
        }

        // Force configure for all status bar windows so they receive the new
        // geometry immediately, and apply the planned position DIRECTLY.
        // Positions normally land in render_finish, which only reaches
        // windows linked into rendering_requested.list — a segment that
        // dropped out of that list (the reconnect-churn wedge) kept its
        // stale slot through every later arrange while the plan held the
        // correct one. The arrange pass is the authority on segment slots,
        // so make every pass re-slot every segment except one the user is
        // dragging (the seat op owns its position until release).
        for &win_ptr in self.windows.iter() {
            if !win_ptr.is_null() && !(*win_ptr).closed && (*win_ptr).is_status_bar() {
                if (*win_ptr).wm_requested.dimensions.is_some() {
                    (*win_ptr).manage_finish();
                }
                if matches!((*win_ptr).state, crate::window::WindowState::Mapped)
                    && !self.is_window_being_moved(win_ptr)
                {
                    let x = (*win_ptr).rendering_requested.x;
                    let y = (*win_ptr).rendering_requested.y;
                    (*win_ptr).box_geom.x = x;
                    (*win_ptr).box_geom.y = y;
                    ffi::river_scene_node_set_position_if_changed((*win_ptr).tree as *mut ffi::wlr_scene_node, x, y);
                    ffi::river_scene_node_set_position_if_changed((*win_ptr).popup_tree as *mut ffi::wlr_scene_node, x, y);
                }
            }
        }

        // If the focused window is no longer visible, refocus
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            let mut focused_visible = false;
            match (*seat).focused {
                crate::seat::Focus::Window(w) => {
                    if !w.is_null() && !(*w).closed && !(*w).minimized && matches!((*w).state, crate::window::WindowState::Mapped) {
                        focused_visible = true;
                    }
                }
                crate::seat::Focus::None => {
                    focused_visible = true;
                }
                _ => {
                    focused_visible = true;
                }
            }
            if !focused_visible {
                self.focus_next_visible_window(seat);
            }
            curr_seat = next_seat;
        }

        self.update_status();
        self.rendering_scheduled.dirty = true;
    }

    pub unsafe fn update_viewport_local(&mut self) {
        let zoom_changed = self.desk_zoom != self.last_viewport_zoom;
        let pan_changed = self.desk_pan_x != self.last_viewport_pan_x || self.desk_pan_y != self.last_viewport_pan_y;
        let moved = zoom_changed || pan_changed;

        self.last_viewport_zoom = self.desk_zoom;
        self.last_viewport_pan_x = self.desk_pan_x;
        self.last_viewport_pan_y = self.desk_pan_y;

        let arrange_t0 = crate::output::frame_debug().then(std::time::Instant::now);
        self.arrange_views();
        if let Some(t0) = arrange_t0 {
            log::info!(
                "[cce-frame] viewport relayout {}us ({} windows, moved={moved})",
                t0.elapsed().as_micros(),
                self.windows.count()
            );
        }
        // Clear rendering dirty flag so we don't trigger the idle callback's IPC handshake
        self.rendering_scheduled.dirty = false;
        self.remove_dirty_idle();

        // Blur is toggled at most twice per gesture: off the moment real motion
        // starts, on once the debounce timer confirms motion has stopped. During
        // motion (and the brief gaps between discrete motion updates) the viewport
        // stays "active" so blur is not re-enabled mid-gesture — that on/off churn
        // was the flicker of the blurred desktop grid behind transparent windows.
        if moved {
            self.viewport_is_active = true;
            // Every motion frame moves the screen-sized backdrop under every
            // blurred window; without this, scenefx re-bakes every optimized
            // blur every frame of the pan. Through a pure pan the bakes are
            // frozen and each window samples the cache where its own bake
            // lives (the coordinates it last baked at), reading exactly its
            // own bake — correct, not stale, because the backdrop moved with
            // it. A zoom changes the scale under the window, which no shift
            // can compensate: the caches thaw (re-bake per frame) for its
            // duration and re-freeze on the next pure-pan frame.
            let scene = (*self.server).scene.wlr_scene;
            ffi::river_scene_set_blur_frozen(scene, !zoom_changed);
            for &window in self.windows.iter() {
                if !window.is_null() {
                    (*window).render_viewport_update();
                }
            }
            // (Re)arm the settle debounce: while motion keeps arriving this pushes
            // the settle out, so it only fires once the gesture truly ends.
            self.arm_viewport_settle_timer();
        } else if self.viewport_is_active {
            // A gap between motion updates within an ongoing gesture: keep blur
            // suppressed and let the settle timer decide when the gesture ended.
            for &window in self.windows.iter() {
                if !window.is_null() {
                    (*window).render_viewport_update();
                }
            }
        } else {
            // Stationary viewport: hold the finished, blurred state.
            for &window in self.windows.iter() {
                if !window.is_null() {
                    (*window).render_finish();
                }
            }
        }

        // Commit outputs or schedule frame updates. A camera-motion frame
        // re-lays-out the whole screen but per-node damage under-reports at
        // the seams (stale slivers of the previous zoom level survive — an
        // idle window's old pixels are nobody's damage), so motion forces a
        // full repaint.
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs_list).next;
        while curr != outputs_list {
            let next = (*curr).next;
            let output = crate::container_of!(curr, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                if moved && !(*output).scene_output.is_null() {
                    ffi::river_scene_output_damage_whole((*output).scene_output);
                } else {
                    ffi::wlr_output_schedule_frame((*output).wlr_output);
                }
            }
            curr = next;
        }

        // Re-evaluate cursor focus/hover since windows have moved relative to pointers
        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats).next;
        while curr_seat != seats {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            (*seat).cursor.update_hovered();
            curr_seat = next_seat;
        }
    }

    /// (Re)arm the debounce that restores backdrop blur once viewport motion
    /// stops. Called on every motion frame, so continuous panning keeps pushing
    /// the settle out; it only fires `VIEWPORT_SETTLE_MS` after the last motion.
    unsafe fn arm_viewport_settle_timer(&mut self) {
        if self.viewport_settle_timer.is_null() {
            let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
            self.viewport_settle_timer = ffi::wl_event_loop_add_timer(
                event_loop,
                Some(handle_viewport_settle_tick),
                self as *mut WindowManager as *mut _,
            );
        }
        if !self.viewport_settle_timer.is_null() {
            ffi::wl_event_source_timer_update(self.viewport_settle_timer, VIEWPORT_SETTLE_MS);
        }
    }

    /// Restore the settled (blurred) render state for every window and repaint.
    /// Runs once the settle debounce confirms the gesture has ended. Window
    /// positions are already final from the last motion frame's arrange pass, so
    /// this only flips each window back to its finished (blur-on) render.
    unsafe fn finish_viewport_settle(&mut self) {
        if !self.viewport_is_active {
            return;
        }
        self.viewport_is_active = false;
        // Thaw the blur caches (marks them all dirty once) so the settled
        // frame re-bakes against the final backdrop.
        ffi::river_scene_set_blur_frozen((*self.server).scene.wlr_scene, false);
        for &window in self.windows.iter() {
            if !window.is_null() {
                (*window).render_finish();
            }
        }
        // Settling re-enables blur and re-finishes every window; sweep any
        // remaining motion-frame slivers with one full repaint.
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs_list).next;
        while curr != outputs_list {
            let next = (*curr).next;
            let output = crate::container_of!(curr, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                if !(*output).scene_output.is_null() {
                    ffi::river_scene_output_damage_whole((*output).scene_output);
                } else {
                    ffi::wlr_output_schedule_frame((*output).wlr_output);
                }
            }
            curr = next;
        }
    }

    /// The window manager's notion of the focused window. Overlay UI
    /// (cce-cloud menus) holds SEAT focus while open so it gets input, but
    /// is transparent here: this falls through to the most recently focused
    /// real window, so opening a menu never changes what "the focused
    /// window" is — for saved state, arrange styling, viewport actions, or
    /// the stream's `focused` query. (Logging out via the desktop menu used
    /// to save the MENU as the session's focused window, poisoning the next
    /// restore.)
    pub unsafe fn focused_window(&self) -> *mut crate::window::Window {
        let seats_list = &(*self.server).input_manager.seats as *const ffi::wl_list as *const WlList as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let crate::seat::Focus::Window(w) = (*seat).focused {
                if !w.is_null() && !(*w).is_overlay_ui() {
                    return w;
                }
                break;
            }
            curr_seat = (*curr_seat).next;
        }
        // Seat focus is on overlay UI (or nothing): the effective focused
        // window is the most recent real one still on screen.
        for &w in self.focus_history.iter() {
            if !w.is_null()
                && !(*w).closed
                && matches!((*w).state, crate::window::WindowState::Mapped)
                && !(*w).is_overlay_ui()
                && !(*w).is_status_bar()
                && !(*w).is_wallpaper()
            {
                return w;
            }
        }
        std::ptr::null_mut()
    }

    pub unsafe fn window_is_valid(&self, win: *mut Window) -> bool {
        if win.is_null() {
            return false;
        }
        self.windows.iter().any(|&w| w == win)
    }

    pub unsafe fn focused_layer_surface(&self) -> *mut ffi::wlr_surface {
        let seats_list = &(*self.server).input_manager.seats as *const ffi::wl_list as *const WlList as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let crate::seat::Focus::LayerSurface(s) = (*seat).focused {
                return s;
            }
            curr_seat = (*curr_seat).next;
        }
        std::ptr::null_mut()
    }


    /// Whether any mapped status segment is currently expanded past the bar
    /// strip — i.e. an in-surface menu is open. `except` (null = none) exempts
    /// one segment, for the click-away path where a press ON an expanded
    /// segment must not dismiss that segment's own menu. Expanded is DEFINED
    /// as thicker than the configured bar height, which is also how the
    /// arrange pass recognizes an expanded segment. Shared by the click-away
    /// dismiss (cursor.rs) and the Escape dismiss (keyboard_group.rs) so the
    /// two triggers can never disagree about what counts as open.
    pub unsafe fn any_expanded_status_segment(&self, except: *mut crate::window::Window) -> bool {
        let bar_h = self.layout.bar_height;
        self.windows.iter().any(|&w| {
            !w.is_null()
                && !(*w).closed
                && w != except
                && (*w).is_status_bar()
                && matches!((*w).state, crate::window::WindowState::Mapped)
                && {
                    let bg = (*w).box_geom;
                    let thickness = match (*w).status_edge {
                        crate::policy::arrange::StatusEdge::Left
                        | crate::policy::arrange::StatusEdge::Right => bg.width,
                        _ => bg.height,
                    };
                    thickness > bar_h
                }
        })
    }

    pub unsafe fn update_status(&self) {
        if let Some(ref sender) = self.status_sender {
            let update = crate::status_server::build_status_update(self);
            let mut last = self.last_status_update.borrow_mut();
            if last.as_ref() != Some(&update) {
                sender.send(update.clone());
                *last = Some(update);
            }
        }
    }

    pub unsafe fn first_seat(&self) -> Option<*mut crate::seat::Seat> {
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let curr_seat = (*seats_list).next;
        if curr_seat != seats_list {
            Some(crate::container_of!(curr_seat, crate::seat::Seat, link))
        } else {
            None
        }
    }

    pub unsafe fn record_focus(&mut self, window: *mut Window) {
        if window.is_null() {
            return;
        }
        // Overlay UI never enters the history: it takes input while open but
        // must not displace the real window as "most recently focused".
        if (*window).is_overlay_ui() {
            return;
        }
        self.focus_history.retain(|&w| w != window);
        self.focus_history.insert(0, window);
    }

    pub unsafe fn remove_from_history(&mut self, window: *mut Window) {
        self.focus_history.retain(|&w| w != window);
    }

    /// Record that this app_id's window disappeared unbidden.
    pub fn note_vanished(&mut self, app_id: String) {
        let now = std::time::Instant::now();
        self.vanished_windows
            .retain(|(_, at)| now.duration_since(*at) < RECONNECT_FOCUS_GRACE);
        self.vanished_windows.push((app_id, now));
    }

    /// Whether this app_id vanished unbidden within the grace, consuming the
    /// record so one disappearance excuses exactly one re-map — a client that
    /// crashes twice does not get a standing exemption.
    pub fn take_recent_vanish(&mut self, app_id: &str) -> bool {
        let now = std::time::Instant::now();
        self.vanished_windows
            .retain(|(_, at)| now.duration_since(*at) < RECONNECT_FOCUS_GRACE);
        match self.vanished_windows.iter().position(|(id, _)| id == app_id) {
            Some(i) => {
                self.vanished_windows.remove(i);
                true
            }
            None => false,
        }
    }

    /// Refocus after the focused window goes away, by the policy crate's
    /// next-visible rule (most recent eligible history entry, else the last
    /// eligible window in window order, else clear focus). This side owns
    /// eligibility (mapped, not minimized, not status/background).
    pub unsafe fn focus_next_visible_window(&mut self, seat: *mut crate::seat::Seat) {
        let eligible = |w: *mut Window| -> bool {
            if (*w).closed || (*w).minimized || !matches!((*w).state, crate::window::WindowState::Mapped) {
                return false;
            }
            if (*w).is_overlay_ui() {
                return false;
            }
            let app_id = (*w).get_app_id_string();
            let is_status_bar = app_id.as_deref().map_or(false, |id| id.starts_with("cce-status"));
            let is_wallpaper = app_id.as_deref() == Some("cce-wallpaper");
            !is_status_bar && !is_wallpaper && !(*w).is_grid()
        };
        let candidate = |w: *mut Window| crate::policy::focus::FocusCandidate {
            id: crate::policy::api::WindowId((*w).ref_key),
            eligible: eligible(w),
        };
        let history: Vec<_> = self
            .focus_history
            .iter()
            .filter(|&&w| !w.is_null())
            .map(|&w| candidate(w))
            .collect();
        let windows: Vec<_> = self
            .windows
            .iter()
            .filter(|&&w| !w.is_null())
            .map(|&w| candidate(w))
            .collect();
        let next = crate::policy::focus::next_visible_focus(&history, &windows)
            .and_then(|id| self.windows.get(id.0).copied())
            .unwrap_or(std::ptr::null_mut());
        // The window the user was looking at went away. The fallback focus
        // always transfers (keyboard input needs a live target); what the
        // CAMERA does about it is the on_app_exit choice: pan to the
        // fallback (the historic behavior), jump to overview, or stay
        // exactly where the user left it.
        //
        // Chrome departing is not an app exit: dismissing the cce-cloud
        // launcher (Popup) or an Overlay dock must never fire the camera
        // reaction — those close as part of using them.
        let departing_chrome = match (*seat).focused {
            crate::seat::Focus::Window(w) if !w.is_null() => matches!(
                (*w).tiling_mode,
                crate::tiling::TilingMode::Popup | crate::tiling::TilingMode::Overlay
            ),
            _ => false,
        };
        let behavior = if departing_chrome {
            crate::config::OnAppExit::Nothing
        } else {
            self.on_app_exit
        };
        (*seat).suppress_focus_pan = behavior != crate::config::OnAppExit::FocusPrevious;
        if !next.is_null() {
            (*seat).focus(crate::seat::Focus::Window(next));
        } else {
            (*seat).focus(crate::seat::Focus::None);
        }
        (*seat).suppress_focus_pan = false;
        if behavior == crate::config::OnAppExit::Overview
            && self.mode == WindowManagerMode::Normal
        {
            self.execute_action(&crate::config::Action::Overview, None);
        }
    }

    /// True for windows that should appear in the window switcher: mapped,
    /// non-closed, and not one of the desktop-shell surfaces (status bar,
    /// wallpaper, or the switcher's own cce-cloud overlay).
    unsafe fn is_switchable_window(&self, w: *mut Window) -> bool {
        if w.is_null() || (*w).closed {
            return false;
        }
        if !matches!((*w).state, crate::window::WindowState::Mapped) {
            return false;
        }
        match (*w).get_app_id_string().as_deref() {
            Some(id) if id.starts_with("cce-status") => false,
            Some("cce-wallpaper") | Some("cce-cloud") | Some("cce-grid") => false,
            _ => true,
        }
    }

    /// Open the alt-tab window switcher: spawn a `cce-cloud --switcher` overlay,
    /// feed it the currently switchable windows in most-recently-used order, and
    /// hand off to a background thread that focuses whatever the user commits to.
    ///
    /// cce-cloud is a `Layer::Overlay` surface with exclusive keyboard focus,
    /// and it commits on Super release / cancels on Escape by itself. Tab while
    /// Super is held, however, never reaches it — that chord matches this very
    /// keybinding — so a repeat press lands back here and is forwarded as a
    /// `__cce_switcher_next__` / `__cce_switcher_prev__` line down the held-open
    /// stdin pipe, moving the highlight. The committed entry is printed to
    /// stdout; we only build the list and map the selection back to a window id.
    ///
    /// `backwards` is the super+shift+tab direction: it cycles the highlight
    /// the other way, and opening with it lands on the least-recently-used
    /// window instead of the previously focused one.
    pub unsafe fn launch_window_switcher(&mut self, backwards: bool) {
        let cycle_line: &[u8] = if backwards {
            b"__cce_switcher_prev__\n"
        } else {
            b"__cce_switcher_next__\n"
        };

        // A repeat super+(shift+)tab while the switcher is already up moves its
        // highlight instead of spawning a second switcher.
        {
            let mut active = ACTIVE_SWITCHER.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(handle) = active.as_mut() {
                use std::io::Write;
                if handle
                    .stdin
                    .write_all(cycle_line)
                    .and_then(|_| handle.stdin.flush())
                    .is_ok()
                {
                    return;
                }
                // Dead pipe: the child exited and we raced its cleanup thread.
                // Drop the stale handle and open a fresh switcher below.
                *active = None;
            }
        }
        // Most-recently-used order (focus_history is MRU-front). The focused
        // window lands first, so cce-cloud auto-selects index 1 — the previously
        // focused window — which is the classic alt-tab default.
        let mut ordered: Vec<*mut Window> = Vec::new();
        for &w in self.focus_history.iter() {
            if self.is_switchable_window(w) && !ordered.contains(&w) {
                ordered.push(w);
            }
        }
        for &w in self.windows.iter() {
            if self.is_switchable_window(w) && !ordered.contains(&w) {
                ordered.push(w);
            }
        }
        if ordered.is_empty() {
            return;
        }

        // cce-cloud echoes the committed entry back verbatim, so keep the mapping
        // from display string to window id to resolve the selection.
        let mut items: Vec<(String, String)> = Vec::new();
        let mut input = String::new();
        for &w in &ordered {
            let id = (*w).ref_key.index.to_string();
            let app_id = (*w).get_app_id_string().unwrap_or_default();
            let title = (*w).get_title_string().unwrap_or_default();
            let display = if title.is_empty() {
                app_id.clone()
            } else {
                format!("{} ({})", title, app_id)
            };
            input.push_str(&display);
            input.push('\n');
            items.push((id, display));
        }
        if backwards {
            // cce-cloud auto-highlights index 1 once the items land; two prev
            // steps from there wrap to the last entry, so a backwards open
            // starts on the least-recently-used window (classic alt+shift+tab).
            input.push_str("__cce_switcher_prev__\n__cce_switcher_prev__\n");
        }

        // Position near the top-centre of the enabled output. Passing explicit
        // -x/-y keeps cce-cloud a layer-shell overlay (omitting both would make
        // it an XDG toplevel, which would not grab keyboard the same way).
        let (mut vp_w, mut origin_x, mut origin_y) = (1920.0_f64, 0i32, 0i32);
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let wlr_box = (*output).sent.box_layout();
                vp_w = wlr_box.width as f64;
                origin_x = wlr_box.x;
                origin_y = wlr_box.y;
                break;
            }
            curr_out = (*curr_out).next;
        }
        // cce-cloud's default logical width is 600; centre it horizontally.
        let x_pos = origin_x + (((vp_w - 600.0) / 2.0).max(0.0)) as i32;
        let y_pos = origin_y + 80;

        let display_env = std::env::var("WAYLAND_DISPLAY").ok();

        let mut child = match std::process::Command::new(cce_cloud_cmd())
            .args([
                "--switcher",
                "-p",
                "Windows:",
                "-x",
                &x_pos.to_string(),
                "-y",
                &y_pos.to_string(),
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                log::error!("window switcher: failed to spawn cce-cloud: {}", e);
                return;
            }
        };

        // Feed the item list but keep stdin open: repeat super+(shift+)tab
        // presses write cycle lines down the same pipe. cce-cloud streams
        // items in as they arrive and does not wait for EOF.
        let generation = SWITCHER_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(input.as_bytes());
            let _ = stdin.flush();
            *ACTIVE_SWITCHER.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(SwitcherHandle { generation, stdin });
        }

        std::thread::spawn(move || {
            run_window_switcher(child, items, display_env);
            // The switcher is gone; release its stdin unless a newer switcher
            // already replaced it.
            let mut active = ACTIVE_SWITCHER.lock().unwrap_or_else(|e| e.into_inner());
            if active.as_ref().map_or(false, |h| h.generation == generation) {
                *active = None;
            }
        });
    }

    pub unsafe fn keep_status_bar_on_top(&mut self) {
        let mut status_bar_windows = Vec::new();
        for &win_ptr in self.windows.iter() {
            if win_ptr.is_null() || (*win_ptr).closed {
                continue;
            }
            if (*win_ptr).is_status_bar() && matches!((*win_ptr).state, crate::window::WindowState::Mapped) {
                status_bar_windows.push(win_ptr);
            }
        }
        for win_ptr in status_bar_windows {
            let node_link = &mut (*win_ptr).node.link as *mut ffi::wl_list as *mut WlList;
            let list_head = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
            if !node_link.is_null() && !list_head.is_null() {
                // Unconditional remove+reinsert, not a `next != head` tail
                // check: a node with a stale next that happened to equal the
                // head skipped the move here AND read as linked to
                // manage_start, so nothing ever re-attached it — the segment
                // froze at its last applied position (the tray mis-slot
                // wedge). The final order is identical (each Mapped status
                // window moves to the tail in windows order) and the
                // primitives no-op cleanly on every unlinked pointer state.
                if !(*win_ptr).is_linked() {
                    log::info!("[LinkDbg] keep_on_top healing unlinked app={:?}",
                        (*win_ptr).get_app_id_string());
                }
                crate::server::wl_list_remove_and_reinit(node_link);
                let last = (*list_head).prev;
                // head.prev can only name this node via pre-existing
                // corruption (a dangling backpointer); fall back to the head
                // so the node still rejoins the list.
                let after = if last.is_null() || last == node_link { list_head } else { last };
                crate::server::wl_list_insert(after, node_link);
            }
        }
    }

    /// Keep a window's open menus clear of the floating plane.
    ///
    /// Floating windows stack in front of tiled ones (the reorder pass), and
    /// a window's xdg popups ride in its own `popup_tree` just above it — so
    /// a menu opened in a TILED app was covered by any floating window over
    /// it. A menu is transient and belongs to whatever the user is working
    /// in, which is the focused window by definition, so that one window's
    /// popup tree rides above every window in layers.wm, either plane.
    ///
    /// Only the focused window, and only while it is in layers.wm: a
    /// fullscreen/popup/status window is in a layer of its own, where the
    /// raise would reorder that layer's members instead. An empty or
    /// disabled popup tree raises harmlessly (nothing to draw), so this does
    /// not need to know whether a menu is actually open —
    /// `wlr_scene_node_raise_to_top` returns early when the node is already
    /// on top, so a repeat costs nothing and damages nothing.
    ///
    /// Called from two places, because neither alone is enough: the reorder
    /// pass (a restack would otherwise drop the popup back to its window),
    /// and popup creation (opening a menu changes nothing the order hash can
    /// see, so it schedules no transaction at all).
    pub unsafe fn raise_focused_popups(&mut self, window: *mut Window) {
        if window.is_null() || (*window).popup_tree.is_null() {
            return;
        }
        if !(*window).is_seat_focused() {
            return;
        }
        let wm_layer = (*self.server).scene.layers.wm;
        if wm_layer.is_null()
            || ffi::river_scene_node_get_parent((*window).popup_tree as *mut _) != wm_layer
        {
            return;
        }
        ffi::wlr_scene_node_raise_to_top((*window).popup_tree as *mut _);
    }

    pub unsafe fn raise_window(&mut self, window: *mut Window) {
        if window.is_null() {
            return;
        }
        let node_link = &mut (*window).node.link as *mut ffi::wl_list as *mut WlList;
        let list_head = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
        if !node_link.is_null() && !list_head.is_null() {
            // Same shape as keep_status_bar_on_top: no `next != head` tail
            // check (stale pointers made it lie), just a safe move-to-tail.
            // A window that isn't Mapped stays out of the render list — its
            // linking is manage_start's job, and force-inserting a
            // Closing/Init window here would resurrect it for one frame.
            if (*window).is_linked() || matches!((*window).state, crate::window::WindowState::Mapped) {
                crate::server::wl_list_remove_and_reinit(node_link);
                let last = (*list_head).prev;
                let after = if last.is_null() || last == node_link { list_head } else { last };
                crate::server::wl_list_insert(after, node_link);
            }
        }
        self.keep_status_bar_on_top();
    }

    /// The overview action a POINTER-LESS toggle stands for — a key press
    /// or a socket command: the keyed (focused-window) exit when in
    /// overview, else the enter. `Action::Overview` itself is the
    /// cursor-driven toggle, whose exit lands on the hovered window or
    /// else on the virtual point under the pointer; that is right for the
    /// sources that ARE the pointer (the background click, the click
    /// release, the gesture binding, a pointer-button binding) and wrong
    /// for the ones that are not, where the pointer is wherever it was
    /// last left. See the "overview" arm of the control handler.
    pub fn overview_action_pointerless(&self) -> crate::config::Action {
        if self.mode == WindowManagerMode::Overview {
            crate::config::Action::OverviewExit
        } else {
            crate::config::Action::OverviewEnter
        }
    }

    /// The overview action a GESTURE toggle stands for. A swipe or pinch
    /// is pointer-located, so exiting onto the hovered window is the
    /// point — but with nothing under the pointer the cursor-driven exit
    /// lands on the empty desktop there, and the pointer was not aimed at
    /// anything: the focused window is what the user was working in, so
    /// the keyed exit takes over. Enter is the toggle's own.
    pub unsafe fn overview_action_for_gesture(&mut self) -> crate::config::Action {
        if self.mode != WindowManagerMode::Overview {
            return crate::config::Action::OverviewEnter;
        }
        if self.build_action_ctx().hovered.is_some() {
            crate::config::Action::Overview
        } else {
            crate::config::Action::OverviewExit
        }
    }

    /// A client asked for fullscreen, or to leave it, on one of its own windows:
    /// xdg_toplevel.set_fullscreen, the cce protocol's set_fullscreen, Xwayland's
    /// _NET_WM_STATE, or a state a toplevel set before its first commit (read at
    /// map, since wlroots stores that one instead of signalling it). Applied here,
    /// in-process, through the policy's fullscreen toggle, so the mode, the lock
    /// and the exit's restore are exactly the keyed action's. The scheduled
    /// zcce_window_v1 event still goes out to a window-manager client as before;
    /// until this, that event was the request's only consumer, and nothing in the
    /// session listens for it, so client fullscreen was silently dropped.
    pub unsafe fn apply_client_fullscreen(&mut self, win: *mut Window, enter: bool) {
        if win.is_null() || (*win).closed || (*win).state != crate::window::WindowState::Mapped {
            return;
        }
        let is_fullscreen = (*win).tiling_mode == crate::tiling::TilingMode::Fullscreen;
        if enter == is_fullscreen {
            return;
        }
        use crate::policy::api::{Compositor, Policy, WindowId};
        let mut ctx = self.build_action_ctx();
        ctx.focused = Some(WindowId((*win).ref_key));
        for cmd in crate::policy::actions::DefaultPolicy.action(&ctx, crate::config::Action::Fullscreen, None) {
            self.apply(&cmd);
        }
    }

    pub unsafe fn execute_action(&mut self, action: &crate::config::Action, command: Option<&str>) {
        use crate::config::Action;
        self.stop_panning_animation();

        // Snapshot → policy → commands: the camera actions (zoom, pan, view
        // jumps, overview) are decided in the policy crate. An empty command
        // list means the policy doesn't claim the action, and the legacy
        // arms below handle it.
        {
            use crate::policy::api::{Compositor, Policy};
            let ctx = self.build_action_ctx();
            let cmds = crate::policy::actions::DefaultPolicy.action(&ctx, *action, command);
            if !cmds.is_empty() {
                for cmd in &cmds {
                    self.apply(cmd);
                }
                return;
            }
        }

        match action {
            Action::None => {}
            Action::Spawn => {
                if let Some(cmd) = command {
                    log::info!("executing spawn: {}, WAYLAND_DISPLAY: {:?}", cmd, std::env::var("WAYLAND_DISPLAY"));
                    match nix::unistd::fork() {
                        Ok(nix::unistd::ForkResult::Child) => {
                            crate::process::cleanup_child();
                            let env: Vec<std::ffi::CString> = std::env::vars()
                                .map(|(k, v)| std::ffi::CString::new(format!("{}={}", k, v)).unwrap())
                                .collect();
                            let env_ptrs: Vec<&std::ffi::CStr> = env.iter().map(|s| s.as_c_str()).collect();
                            let sh_c = std::ffi::CString::new("/bin/sh").unwrap();
                            let c_c = std::ffi::CString::new("-c").unwrap();
                            let cmd_c = std::ffi::CString::new(cmd).unwrap();
                            let args = [sh_c.as_c_str(), c_c.as_c_str(), cmd_c.as_c_str()];
                            let _ = nix::unistd::execve(&sh_c, &args, &env_ptrs);
                            std::process::exit(1);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            log::error!("failed to fork child process: {}", e);
                        }
                    }
                }
            }
            Action::WindowSwitcher => {
                self.launch_window_switcher(false);
            }
            Action::WindowSwitcherPrev => {
                self.launch_window_switcher(true);
            }
            Action::Screenshot => {
                // Same capture as `ccectl screenshot`: the enabled output's
                // next frame, saved under ~/Pictures/screenshots.
                //
                // Hide any borrowed reply channel first: this action can be
                // reached from an IPC command of its own, and the screenshot
                // dispatch takes the channel to answer later — which would
                // hand this capture's verdict to whoever asked for the
                // *action*, and leave them waiting a frame for it.
                let borrowed = self.pending_ipc_reply.take();
                let _ = self.process_ipc_command("screenshot");
                self.pending_ipc_reply = borrowed;
            }
            Action::Reload => {
                log::info!("monolithic execute_action: Reload requested");
                match self.reload_config() {
                    Ok(()) => {
                        let _ = std::process::Command::new("notify-send")
                            .arg("cce")
                            .arg("Configuration reloaded successfully")
                            .spawn();
                    }
                    Err(e) => {
                        let _ = std::process::Command::new("notify-send")
                            .arg("cce")
                            .arg(format!("Failed to reload config:\n{}", e))
                            .spawn();
                    }
                }
            }
            Action::Exit => {
                log::info!("monolithic execute_action: Exit requested");
                self.start_clean_exit();
            }
            _ => {}
        }
    }

    /// Resolve a window by a query string via the policy crate's matching
    /// rules (numeric window id first, then app_id with
    /// exact-beats-substring). Returns null if nothing mapped matches.
    /// Shared by focus-window / center-window / window-stream.
    pub unsafe fn find_window_by_query(&self, query: &str) -> *mut Window {
        let mut candidates = Vec::new();
        let mut ptrs: Vec<*mut Window> = Vec::new();
        for &w in self.windows.iter() {
            if !w.is_null() && !(*w).closed
                && matches!((*w).state, crate::window::WindowState::Mapped)
            {
                candidates.push(crate::policy::query::QueryCandidate {
                    index: (*w).ref_key.index,
                    app_id: (*w).get_app_id_string(),
                });
                ptrs.push(w);
            }
        }
        match crate::policy::query::find_window(&candidates, query) {
            Some(pos) => ptrs[pos],
            None => std::ptr::null_mut(),
        }
    }

    pub unsafe fn process_ipc_command(&mut self, cmd: &str) -> String {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return "error: empty command\n".to_string();
        }

        // No blanket stop_panning_animation here: it killed any camera
        // flight on EVERY IPC command — a `windows --json` poll or a
        // status-bar ccectl call landing mid-transition froze the camera
        // partway, silently. Commands that change the camera cancel
        // in-flight animation themselves (policy StopPanAnimation, the
        // instant SetCamera arm, seat op starts).
        let action = parts[0];
        match action {
            // Scene introspection: dump EVERY buffer in the whole scene —
            // layer, layout position, dest/natural size, owning client pid.
            // Nothing on screen can hide from this.
            "debug-scene" => {
                unsafe extern "C" fn dump_iter(
                    buffer: *mut ffi::wlr_scene_buffer,
                    sx: i32,
                    sy: i32,
                    user_data: *mut std::ffi::c_void,
                ) {
                    let out = &mut *(user_data as *mut String);
                    let node = buffer as *mut ffi::wlr_scene_node;
                    let surface = ffi::river_scene_node_get_surface(node);
                    let mut pid = 0;
                    if !surface.is_null() {
                        let res = ffi::river_wlr_surface_get_resource(surface);
                        if !res.is_null() {
                            let client = ffi::wl_resource_get_client(res);
                            if !client.is_null() {
                                let (mut uid, mut gid) = (0, 0);
                                ffi::wl_client_get_credentials(client, &mut pid, &mut uid, &mut gid);
                            }
                        }
                    }
                    out.push_str(&format!(
                        "  buf sx={} sy={} dest={}x{} natural={}x{} surface={} pid={} enabled={}\n",
                        sx,
                        sy,
                        ffi::river_scene_buffer_get_dest_width(buffer),
                        ffi::river_scene_buffer_get_dest_height(buffer),
                        ffi::river_scene_buffer_get_width(buffer),
                        ffi::river_scene_buffer_get_height(buffer),
                        !surface.is_null(),
                        pid,
                        ffi::river_scene_node_get_enabled(node),
                    ));
                }
                let scene = &(*self.server).scene;
                let mut out = String::new();
                let layers: [(&str, *mut ffi::wlr_scene_tree); 12] = [
                    ("background", scene.layers.background),
                    ("bottom", scene.layers.bottom),
                    ("wm", scene.layers.wm),
                    ("top", scene.layers.top),
                    ("fullscreen", scene.layers.fullscreen),
                    ("overlay", scene.layers.overlay),
                    ("popups", scene.layers.popups),
                    ("override_redirect", scene.layers.override_redirect),
                    ("border_overlay", scene.layers.border_overlay),
                    ("drag_icons", scene.drag_icons),
                    ("hidden", scene.hidden_tree),
                    ("locked", scene.locked_tree),
                ];
                for (name, tree) in layers {
                    if tree.is_null() {
                        continue;
                    }
                    out.push_str(&format!("[{}]\n", name));
                    ffi::wlr_scene_node_for_each_buffer(
                        tree as *mut ffi::wlr_scene_node,
                        Some(dump_iter),
                        &mut out as *mut String as *mut std::ffi::c_void,
                    );
                }
                return out;
            }
            // Scene introspection: dump every scene buffer of a window's
            // trees — position, dest size, natural buffer size,
            // surface-backed or not. Found the zoom-ghost bug; kept as a
            // debugging tool.
            "debug-buffers" => {
                let win = if parts.len() >= 2 {
                    self.find_window_by_query(&parts[1..].join(" "))
                } else {
                    self.focused_window()
                };
                if win.is_null() {
                    return "error: no matching window\n".to_string();
                }
                let mut out = format!(
                    "window box=({},{},{}x{}) scale={} saved={} tree_en={} surf_en={} saved_en={}\n",
                    (*win).box_geom.x, (*win).box_geom.y, (*win).box_geom.width, (*win).box_geom.height,
                    (*win).scale,
                    (*win).surfaces.saved,
                    ffi::river_scene_node_get_enabled((*win).tree as *mut ffi::wlr_scene_node),
                    ffi::river_scene_node_get_enabled((*win).surfaces.tree as *mut ffi::wlr_scene_node),
                    ffi::river_scene_node_get_enabled((*win).surfaces.saved_tree as *mut ffi::wlr_scene_node),
                );
                unsafe extern "C" fn dump_iter(
                    buffer: *mut ffi::wlr_scene_buffer,
                    sx: i32,
                    sy: i32,
                    user_data: *mut std::ffi::c_void,
                ) {
                    let out = &mut *(user_data as *mut String);
                    let node = buffer as *mut ffi::wlr_scene_node;
                    let surface = ffi::river_scene_node_get_surface(node);
                    out.push_str(&format!(
                        "  buf sx={} sy={} dest={}x{} natural={}x{} surface={} enabled={}\n",
                        sx,
                        sy,
                        ffi::river_scene_buffer_get_dest_width(buffer),
                        ffi::river_scene_buffer_get_dest_height(buffer),
                        ffi::river_scene_buffer_get_width(buffer),
                        ffi::river_scene_buffer_get_height(buffer),
                        !surface.is_null(),
                        ffi::river_scene_node_get_enabled(node),
                    ));
                }
                for (name, node) in [
                    ("surfaces", (*win).surfaces.tree as *mut ffi::wlr_scene_node),
                    ("saved", (*win).surfaces.saved_tree as *mut ffi::wlr_scene_node),
                    ("popup", (*win).popup_tree as *mut ffi::wlr_scene_node),
                    ("whole-tree", (*win).tree as *mut ffi::wlr_scene_node),
                ] {
                    out.push_str(&format!("[{}]\n", name));
                    ffi::wlr_scene_node_for_each_buffer(
                        node,
                        Some(dump_iter),
                        &mut out as *mut String as *mut std::ffi::c_void,
                    );
                }
                return out;
            }
            // WM introspection: one line per window with the fields the
            // arrange pass keys on (state, render-list linkage, status edge,
            // seat-op move, requested vs applied position, configure state).
            // Found the tray segment mis-slot wedge; kept as a debugging tool.
            "debug-windows" => {
                let mut out = format!(
                    "wm state={:?} dirty={} dirty_lazy={} rendering_dirty={} dirty_idle_armed={} wm_object={}\n",
                    self.state,
                    self.scheduled.dirty,
                    self.scheduled.dirty_lazy,
                    self.rendering_scheduled.dirty,
                    !self.dirty_idle.is_null(),
                    !self.object.is_null(),
                );
                for &w in self.windows.iter() {
                    if w.is_null() {
                        continue;
                    }
                    let cfg = match (*w).impl_type {
                        crate::window::WindowImpl::Toplevel(t) if !t.is_null() => {
                            format!("{:?}", (*t).configure_state)
                        }
                        _ => "-".to_string(),
                    };
                    out.push_str(&format!(
                        "window id={} app_id={:?} state={:?} closed={} linked={} mode={:?} edge={:?} moved={} req_pos=({},{}) box=({},{},{}x{}) collapsed_len={} cfg={}\n",
                        (*w).ref_key.index,
                        (*w).get_app_id_string().unwrap_or_default(),
                        (*w).state,
                        (*w).closed,
                        (*w).is_linked(),
                        (*w).tiling_mode,
                        (*w).status_edge,
                        self.is_window_being_moved(w),
                        (*w).rendering_requested.x,
                        (*w).rendering_requested.y,
                        (*w).box_geom.x,
                        (*w).box_geom.y,
                        (*w).box_geom.width,
                        (*w).box_geom.height,
                        (*w).status_collapsed_len,
                        cfg,
                    ));
                }
                return out;
            }
            "status-hide-mode" => {
                let enable = if parts.len() >= 2 {
                    match parts[1] {
                        "true" | "on" | "enable" | "1" => true,
                        "false" | "off" | "disable" | "0" => false,
                        _ => !self.status_hide_mode,
                    }
                } else {
                    !self.status_hide_mode
                };
                self.status_hide_mode = enable;
                self.dirty_windowing();
                return format!("ok {}\n", enable);
            }
            "adjust-position-mode" => {
                if parts.get(1).copied() == Some("query") {
                    return format!("ok {}\n", self.adjust_position_mode);
                }
                let enable = if parts.len() >= 2 {
                    match parts[1] {
                        "true" | "on" | "enable" | "1" => true,
                        "false" | "off" | "disable" | "0" => false,
                        _ => !self.adjust_position_mode,
                    }
                } else {
                    !self.adjust_position_mode
                };
                self.adjust_position_mode = enable;
                if enable {
                    let _ = std::fs::File::create("/tmp/cce-status-interface-adjust-mode");
                } else {
                    let _ = std::fs::remove_file("/tmp/cce-status-interface-adjust-mode");
                }
                self.dirty_windowing();
                return format!("ok {}\n", enable);
            }
            "pan-by" => {
                if parts.len() < 3 { return "error: missing dx or dy\n".to_string(); }
                if let (Ok(dx), Ok(dy)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    self.desk_pan_x += dx;
                    self.desk_pan_y += dy;
                    if matches!(self.state, WindowManagerState::Idle) {
                        self.update_viewport_local();
                    } else {
                        self.dirty_windowing();
                    }
                    return "ok\n".to_string();
                }
                "error: invalid dx or dy\n".to_string()
            }
            "pan-to" => {
                if parts.len() < 3 { return "error: missing x or y\n".to_string(); }
                if let (Ok(x), Ok(y)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    self.desk_pan_x = x;
                    self.desk_pan_y = y;
                    if matches!(self.state, WindowManagerState::Idle) {
                        self.update_viewport_local();
                    } else {
                        self.dirty_windowing();
                    }
                    return "ok\n".to_string();
                }
                "error: invalid x or y\n".to_string()
            }
            "zoom-in" => {
                self.execute_action(&crate::config::Action::ZoomIn, None);
                "ok\n".to_string()
            }
            "zoom-out" => {
                self.execute_action(&crate::config::Action::ZoomOut, None);
                "ok\n".to_string()
            }
            "zoom-reset" => {
                self.execute_action(&crate::config::Action::ZoomReset, None);
                "ok\n".to_string()
            }
            "pan-left" => {
                self.execute_action(&crate::config::Action::PanLeft, None);
                "ok\n".to_string()
            }
            "pan-right" => {
                self.execute_action(&crate::config::Action::PanRight, None);
                "ok\n".to_string()
            }
            "pan-up" => {
                self.execute_action(&crate::config::Action::PanUp, None);
                "ok\n".to_string()
            }
            "pan-down" => {
                self.execute_action(&crate::config::Action::PanDown, None);
                "ok\n".to_string()
            }
            "overlay-left" => {
                self.execute_action(&crate::config::Action::OverlayLeft, None);
                "ok\n".to_string()
            }
            "overlay-right" => {
                self.execute_action(&crate::config::Action::OverlayRight, None);
                "ok\n".to_string()
            }
            "set-zoom" => {
                if parts.len() < 2 { return "error: missing zoom factor\n".to_string(); }
                if let Ok(factor) = parts[1].parse::<f64>() {
                    let new_zoom = factor.clamp(0.1, 10.0);
                    let (mut viewport_w, mut viewport_h) = (1920.0, 1080.0);
                    let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                    let mut curr_out = (*outputs_list).next;
                    while curr_out != outputs_list {
                        let output = crate::container_of!(curr_out, crate::output::Output, link);
                        if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                            let wlr_box = (*output).sent.box_layout();
                            viewport_w = wlr_box.width as f64;
                            viewport_h = wlr_box.height as f64;
                            break;
                        }
                        curr_out = (*curr_out).next;
                    }
                    let cx = self.desk_pan_x + (viewport_w / 2.0) / self.desk_zoom;
                    let cy = self.desk_pan_y + (viewport_h / 2.0) / self.desk_zoom;
                    self.desk_pan_x = cx - (viewport_w / 2.0) / new_zoom;
                    self.desk_pan_y = cy - (viewport_h / 2.0) / new_zoom;
                    self.desk_zoom = new_zoom;
                    self.set_mode(if (new_zoom - 1.0).abs() > 0.001 { WindowManagerMode::Overview } else { WindowManagerMode::Normal });
                    self.dirty_windowing();
                    return "ok\n".to_string();
                }
                "error: invalid zoom factor\n".to_string()
            }
            "set-coords" => {
                if parts.len() < 3 { return "error: missing x or y\n".to_string(); }
                if let (Ok(x), Ok(y)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    if let Some(seat) = self.first_seat() {
                        if let crate::seat::Focus::Window(fw) = (*seat).focused {
                            (*fw).virtual_x = x;
                            (*fw).virtual_y = y;
                            self.dirty_windowing();
                            return "ok\n".to_string();
                        }
                    }
                    return "error: no focused window\n".to_string();
                }
                "error: invalid x or y\n".to_string()
            }
            "set-coords-of" => {
                if parts.len() < 4 { return "error: missing app_id, x, or y\n".to_string(); }
                let app_id_query = parts[1];
                if let (Ok(x), Ok(y)) = (parts[2].parse::<f64>(), parts[3].parse::<f64>()) {
                    let mut found = false;
                    for &w in self.windows.iter() {
                        if !w.is_null() && !(*w).closed && !(*w).minimized && matches!((*w).state, crate::window::WindowState::Mapped) {
                            if let Some(aid) = (*w).get_app_id_string() {
                                if aid.to_lowercase() == app_id_query.to_lowercase() {
                                    (*w).virtual_x = x;
                                    (*w).virtual_y = y;
                                    found = true;
                                }
                            }
                        }
                    }
                    if found {
                        self.dirty_windowing();
                        return "ok\n".to_string();
                    } else {
                        return "error: window not found\n".to_string();
                    }
                }
                "error: invalid x or y\n".to_string()
            }
            "close" => {
                self.execute_action(&crate::config::Action::Close, None);
                "ok\n".to_string()
            }
            // "expose" is the retired name for the overview toggle. The
            // halves rather than `Action::Overview`: the toggle's exit is
            // cursor-driven (it lands on the hovered window, else on the
            // virtual point under the pointer), which suits a gesture but
            // not a socket command, whose pointer is wherever it was left —
            // a scripted exit in a headless shadow, pointer idle at its
            // startup position on the background, came back panned a
            // screen away from every window. The keyed exit lands on the
            // focused window and falls back to the cursor only when
            // nothing is focused; the enter half is identical to the
            // toggle's.
            "overview" | "expose" => {
                self.execute_action(&self.overview_action_pointerless(), None);
                "ok\n".to_string()
            }
            "wm-mode" => {
                if parts.len() < 2 {
                    return format!("{:?}\n", self.mode).to_lowercase();
                }
                let target = parts[1].to_lowercase();
                // Same pointer-less halves as the "overview" command above.
                if target == "normal" {
                    if self.mode == WindowManagerMode::Overview {
                        self.execute_action(&crate::config::Action::OverviewExit, None);
                    }
                    return "ok\n".to_string();
                } else if target == "overview" {
                    if self.mode == WindowManagerMode::Normal {
                        self.execute_action(&crate::config::Action::OverviewEnter, None);
                    }
                    return "ok\n".to_string();
                }
                "error: invalid mode, specify 'normal' or 'overview'\n".to_string()
            }
            "minimize" => {
                self.execute_action(&crate::config::Action::Minimize, None);
                "ok\n".to_string()
            }
            "focus-next" => {
                self.execute_action(&crate::config::Action::FocusNext, None);
                "ok\n".to_string()
            }
            "focus-prev" => {
                self.execute_action(&crate::config::Action::FocusPrev, None);
                "ok\n".to_string()
            }
            // Relative grid steps for the focused window: a tiled one swaps
            // with whatever tiled window holds the destination, a floating one
            // just moves by the period. Absolute placement is
            // "move-window <square>", above.
            "move-window-left" | "move-window-right" | "move-window-up" | "move-window-down" => {
                let a = match action {
                    "move-window-left" => crate::config::Action::MoveWindowLeft,
                    "move-window-right" => crate::config::Action::MoveWindowRight,
                    "move-window-up" => crate::config::Action::MoveWindowUp,
                    _ => crate::config::Action::MoveWindowDown,
                };
                self.execute_action(&a, None);
                "ok\n".to_string()
            }
            "focus-up" => {
                self.execute_action(&crate::config::Action::FocusUp, None);
                "ok\n".to_string()
            }
            "focus-down" => {
                self.execute_action(&crate::config::Action::FocusDown, None);
                "ok\n".to_string()
            }
            "focus-left" => {
                self.execute_action(&crate::config::Action::FocusLeft, None);
                "ok\n".to_string()
            }
            "focus-right" => {
                self.execute_action(&crate::config::Action::FocusRight, None);
                "ok\n".to_string()
            }
            "window-switcher" => {
                self.execute_action(&crate::config::Action::WindowSwitcher, None);
                "ok\n".to_string()
            }
            "fullscreen" => {
                self.execute_action(&crate::config::Action::Fullscreen, None);
                "ok\n".to_string()
            }
            "mode-next" => {
                self.execute_action(&crate::config::Action::ModeNext, None);
                "ok\n".to_string()
            }
            "mode-next-shared" => {
                self.execute_action(&crate::config::Action::ModeNextShared, None);
                "ok\n".to_string()
            }
            "set-mode" => {
                // set-mode <floating|tiled|fullscreen> [app_id|id] — set one
                // window's mode outright (the focused window when unnamed),
                // through the same SetWindowMode + Relayout pair the
                // mode_next action produces, so the geometry transitions
                // (tiled grid snap, fullscreen sizing, floating restore)
                // come from the arrange pass exactly as they do for the key.
                // The internal roles (popup/overlay/status/utility) are
                // deliberately not settable here; `mode` rules cover those.
                if parts.len() < 2 {
                    return "error: usage: set-mode <floating|tiled|fullscreen> [app_id|id]\n".to_string();
                }
                let mode = match parts[1].to_lowercase().as_str() {
                    "floating" => crate::tiling::TilingMode::Floating,
                    "tiled" => crate::tiling::TilingMode::Tiled,
                    "fullscreen" => crate::tiling::TilingMode::Fullscreen,
                    other => return format!("error: unknown mode: {} (floating|tiled|fullscreen)\n", other),
                };
                let target = if parts.len() >= 3 {
                    self.find_window_by_query(&parts[2..].join(" "))
                } else {
                    self.focused_window()
                };
                if target.is_null() {
                    return "error: no target window\n".to_string();
                }
                {
                    use crate::policy::api::{Command, Compositor, WindowId};
                    let id = WindowId((*target).ref_key);
                    self.apply(&Command::SetWindowMode { id, mode, locked: true });
                    self.apply(&Command::Relayout);
                }
                "ok\n".to_string()
            }
            "focus-window" => {
                if parts.len() < 2 { return "error: missing app_id/id\n".to_string(); }
                if let Some(seat) = self.first_seat() {
                    let best_target = self.find_window_by_query(&parts[1..].join(" "));
                    if !best_target.is_null() {
                        if (*best_target).minimized {
                            (*best_target).minimized = false;
                        }
                        (*seat).focus(crate::seat::Focus::Window(best_target));
                        self.raise_window(best_target);
                        self.dirty_windowing();
                        "ok\n".to_string()
                    } else {
                        "error: window not found\n".to_string()
                    }
                } else {
                    "error: no seat found\n".to_string()
                }
            }
            "close-window" => {
                // close-window <app_id|id> [title substring...] — ask a specific
                // window to close. The optional title filter disambiguates when an
                // app has several windows (e.g. KeePassXC's orphaned "Unlock
                // Database" prompt next to its main window).
                if parts.len() < 2 { return "error: missing app_id/id\n".to_string(); }
                let query = parts[1].to_lowercase();
                let title_filter = parts[2..].join(" ").to_lowercase();
                let mut target: *mut Window = std::ptr::null_mut();
                let mut best_score = 0;
                for &w in self.windows.iter() {
                    if w.is_null() || (*w).closed
                        || !matches!((*w).state, crate::window::WindowState::Mapped)
                    {
                        continue;
                    }
                    if !title_filter.is_empty() {
                        let title = (*w).get_title_string().unwrap_or_default().to_lowercase();
                        if !title.contains(&title_filter) {
                            continue;
                        }
                    }
                    let id_match = query.parse::<u32>().map_or(false, |id| (*w).ref_key.index == id);
                    let aid = (*w).get_app_id_string().unwrap_or_default().to_lowercase();
                    let score = if id_match || aid == query {
                        100
                    } else if !query.is_empty() && aid.contains(&query) {
                        50
                    } else {
                        0
                    };
                    if score > best_score {
                        best_score = score;
                        target = w;
                    }
                }
                if target.is_null() {
                    return "error: window not found\n".to_string();
                }
                let title = (*target).get_title_string().unwrap_or_default();
                log::info!("[ipc] close-window: closing {:?}", title);
                (*target).close();
                format!("ok {}\n", title)
            }
            "move-window" => {
                // move-window <square> [app_id|id] — put a window on a named
                // desktop square (chess style, e.g. "C-9"). With no app_id the
                // focused window moves. A Tiled window keeps the SHAPE of its
                // block and is re-anchored with its top-left on that square;
                // a Floating window keeps its size.
                if parts.len() < 2 {
                    return "error: usage: move-window <square> [app_id|id]\n".to_string();
                }
                let sp = self.layout.snap_params();
                let Some((col, row)) = crate::policy::cells::parse_square(parts[1]) else {
                    return format!(
                        "error: '{}' is not a square (expected e.g. A1, C-9, -B2)\n",
                        parts[1]
                    );
                };
                let target: *mut Window = if parts.len() >= 3 {
                    self.find_window_by_query(&parts[2..].join(" "))
                } else if let Some(seat) = self.first_seat() {
                    match (*seat).focused {
                        crate::seat::Focus::Window(w) => w,
                        _ => std::ptr::null_mut(),
                    }
                } else {
                    std::ptr::null_mut()
                };
                if target.is_null() {
                    return "error: window not found\n".to_string();
                }
                if (*target).minimized {
                    (*target).minimized = false;
                }

                let (x, y, _, _) = crate::policy::cells::square_rect(
                    col,
                    row,
                    sp.cell_w,
                    sp.cell_h,
                    sp.gap_width,
                    sp.cell_inset,
                );
                (*target).virtual_x = x;
                (*target).virtual_y = y;
                // Saved floating geometry follows the window, so a later
                // Tiled -> Floating transition restores it at the new square
                // rather than yanking it back to where it used to live.
                (*target).saved_floating_virtual_x = x;
                (*target).saved_floating_virtual_y = y;
                self.dirty_windowing();

                let cell = crate::policy::cells::window_span_label(
                    x,
                    y,
                    (*target).box_geom.width as f64,
                    (*target).box_geom.height as f64,
                    sp.cell_w,
                    sp.cell_h,
                    sp.gap_width,
                );
                format!("ok cell={} vx={:.1} vy={:.1}\n", cell, x, y)
            }
            "center-window" | "bring-window" => {
                // Pan the desktop so the target window (given app_id/id, or the
                // focused window if omitted) is centered in the output, then focus
                // and raise it. Replies with the window's resulting on-screen box so
                // the caller can screenshot it directly (grim -g "X,Y WxH").
                let target: *mut Window = if parts.len() >= 2 {
                    self.find_window_by_query(&parts[1..].join(" "))
                } else if let Some(seat) = self.first_seat() {
                    match (*seat).focused {
                        crate::seat::Focus::Window(w) => w,
                        _ => std::ptr::null_mut(),
                    }
                } else {
                    std::ptr::null_mut()
                };

                if target.is_null() {
                    return "error: window not found\n".to_string();
                }
                if (*target).minimized {
                    (*target).minimized = false;
                }

                // Enabled output's origin + size (fall back to 1920x1080 @ 0,0).
                let (mut vp_w, mut vp_h) = (1920.0_f64, 1080.0_f64);
                let (mut phys_x, mut phys_y) = (0i32, 0i32);
                let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                let mut curr_out = (*outputs_list).next;
                while curr_out != outputs_list {
                    let output = crate::container_of!(curr_out, crate::output::Output, link);
                    if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                        let wlr_box = (*output).sent.box_layout();
                        vp_w = wlr_box.width as f64;
                        vp_h = wlr_box.height as f64;
                        phys_x = wlr_box.x;
                        phys_y = wlr_box.y;
                        break;
                    }
                    curr_out = (*curr_out).next;
                }

                let w = if (*target).box_geom.width > 0 { (*target).box_geom.width as f64 } else { 800.0 };
                let h = if (*target).box_geom.height > 0 { (*target).box_geom.height as f64 } else { 600.0 };

                // Center the window's virtual center in the viewport. Sets desk_pan
                // directly (no animation) so the reply geometry is immediately valid.
                let center_x = (*target).virtual_x + w / 2.0;
                let center_y = (*target).virtual_y + h / 2.0;
                self.desk_pan_x = center_x - (vp_w / 2.0) / self.desk_zoom;
                self.desk_pan_y = center_y - (vp_h / 2.0) / self.desk_zoom;

                if let Some(seat) = self.first_seat() {
                    (*seat).focus(crate::seat::Focus::Window(target));
                }
                self.raise_window(target);
                self.dirty_windowing();

                // screen = output_origin + (virtual - desk_pan) * zoom
                let screen_x = phys_x as f64 + ((*target).virtual_x - self.desk_pan_x) * self.desk_zoom;
                let screen_y = phys_y as f64 + ((*target).virtual_y - self.desk_pan_y) * self.desk_zoom;
                format!(
                    "ok x={} y={} w={} h={}\n",
                    screen_x.round() as i32,
                    screen_y.round() as i32,
                    (w * self.desk_zoom).round() as i32,
                    (h * self.desk_zoom).round() as i32,
                )
            }
            "screenshot" => {
                // screenshot                        → the enabled output's next frame
                // screenshot region <x> <y> <w> <h> → on-screen region (logical px)
                // screenshot window [app_id|id]     → window content, even off-viewport
                let path = crate::screenshot::default_path();
                match parts.get(1).copied() {
                    Some("window") => {
                        let target: *mut Window = if parts.len() >= 3 {
                            self.find_window_by_query(&parts[2..].join(" "))
                        } else if let Some(seat) = self.first_seat() {
                            match (*seat).focused {
                                crate::seat::Focus::Window(w) => w,
                                _ => std::ptr::null_mut(),
                            }
                        } else {
                            std::ptr::null_mut()
                        };
                        if target.is_null() {
                            return "error: window not found\n".to_string();
                        }
                        match crate::screenshot::capture_window(target, path) {
                            Ok(p) => format!("ok {}\n", p),
                            Err(e) => format!("error: {}\n", e),
                        }
                    }
                    None | Some("region") => {
                        // Region args are logical on-screen coordinates relative to
                        // the output; the capture crops the physical buffer.
                        let region_logical = if parts.get(1) == Some(&"region") {
                            let vals: Vec<f64> = parts[2..].iter().filter_map(|p| p.parse().ok()).collect();
                            if vals.len() != 4 {
                                return "error: usage: screenshot region <x> <y> <w> <h>\n".to_string();
                            }
                            Some((vals[0], vals[1], vals[2], vals[3]))
                        } else {
                            None
                        };

                        // First enabled output (same walk as center-window).
                        let mut target_out: *mut crate::output::Output = std::ptr::null_mut();
                        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
                        let mut curr_out = (*outputs_list).next;
                        while curr_out != outputs_list {
                            let output = crate::container_of!(curr_out, crate::output::Output, link);
                            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                                target_out = output;
                                break;
                            }
                            curr_out = (*curr_out).next;
                        }
                        if target_out.is_null() {
                            return "error: no enabled output\n".to_string();
                        }

                        let region = region_logical.map(|(x, y, w, h)| {
                            // logical → buffer px via the output's effective scale.
                            let buf_w = ffi::river_wlr_output_get_width((*target_out).wlr_output) as f64;
                            let layout_w = (*target_out).sent.box_layout().width.max(1) as f64;
                            let scale = buf_w / layout_w;
                            ffi::wlr_box {
                                x: (x * scale).round() as i32,
                                y: (y * scale).round() as i32,
                                width: (w * scale).round() as i32,
                                height: (h * scale).round() as i32,
                            }
                        });

                        // The capture happens a frame from now, in
                        // `Output::render_and_commit`, so the reply channel
                        // rides along with it: answering `ok <path>` here
                        // claimed success for captures that then failed (an
                        // unsupported readback format, a failed commit) and
                        // named a file that never appeared. Taking the
                        // channel is what tells `handle_ipc_event` not to
                        // answer, so the returned string goes nowhere.
                        self.pending_screenshot = Some(crate::screenshot::PendingScreenshot::new(
                            target_out,
                            region,
                            path,
                            self.pending_ipc_reply.take(),
                        ));
                        ffi::wlr_output_schedule_frame((*target_out).wlr_output);
                        String::new()
                    }
                    Some(other) => format!("error: unknown screenshot target: {}\n", other),
                }
            }
            "exit" => {
                if matches!(parts.get(1), Some(&"force") | Some(&"--force")) {
                    // No waiting on close requests: whatever has not closed
                    // by now is hung rather than asking. Save and go.
                    log::info!("Forced exit requested; terminating without waiting for windows to close.");
                    self.save_state();
                    self.shutting_down = true;
                    ffi::wl_display_terminate((*self.server).wl_server);
                    return "ok\n".to_string();
                }
                self.execute_action(&crate::config::Action::Exit, None);
                "ok\n".to_string()
            }
            "restart-compositor" => {
                // Leave the restart flag for cce-display-manager's daemon (it
                // checks after the session worker exits, verifies the file is
                // owned by the session user, and relaunches this same session
                // greeter-free), then exit cleanly — which saves window state,
                // so the restored compositor brings the session back.
                let user = std::env::var("USER")
                    .unwrap_or_else(|_| format!("uid{}", unsafe { libc::getuid() }));
                let flag = format!("/tmp/cce-restart-requested-{}", user);
                if let Err(e) = std::fs::write(&flag, b"restart\n") {
                    return format!("error: cannot write {}: {}\n", flag, e);
                }
                self.execute_action(&crate::config::Action::Exit, None);
                "ok restarting\n".to_string()
            }
            "reload" => {
                unsafe {
                    match self.reload_config() {
                        Ok(()) => "ok\n".to_string(),
                        Err(e) => format!("error: failed to reload config: {}\n", e),
                    }
                }
            }
            "retile" => {
                self.dirty_windowing();
                "ok\n".to_string()
            }
            "outputs" => {
                // One line per output: the figures a client's `units::Metric`
                // is built from (mode, scale, logical size, physical mm) and
                // where the mm came from — `configured` (a `size_mm`
                // override), `measured` (EDID), or `none` (clients fall back
                // to the assumed 96 ppi). `px_per_mm` is LOGICAL px, the
                // number cce-ui resolves a `(mm)` length with.
                let as_json = parts.get(1).copied() == Some("--json");
                let mut out = String::new();
                let om = &(*self.server).om;
                let head = &om.outputs as *const ffi::wl_list as *mut ffi::wl_list;
                let mut link = om.outputs.next;
                while link != head {
                    let output = &*crate::container_of!(link, crate::output::Output, link);
                    link = (*link).next;
                    let wlr_output = output.wlr_output;
                    if wlr_output.is_null() {
                        continue;
                    }
                    let name = std::ffi::CStr::from_ptr(ffi::river_wlr_output_get_name(wlr_output)).to_string_lossy().to_string();
                    let (mut mm_w, mut mm_h) = (0i32, 0i32);
                    ffi::river_wlr_output_get_phys_size(wlr_output, &mut mm_w, &mut mm_h);
                    let source = if self.display.contains_key(&format!("mm_w_{}", name)) {
                        "configured"
                    } else if mm_w > 0 && mm_h > 0 {
                        "measured"
                    } else {
                        "none"
                    };
                    let st = output.sent;
                    let enabled = matches!(st.state, crate::output::OutputStateValue::Enabled);
                    let (pw, ph, refresh) = match st.mode {
                        crate::output::OutputMode::Standard(m) if !m.is_null() => ((*m).width, (*m).height, (*m).refresh),
                        crate::output::OutputMode::Custom { width, height, refresh } => (width, height, refresh),
                        _ => (0, 0, 0),
                    };
                    let (lw, lh) = st.dimensions();
                    let px_per_mm = if mm_w > 0 && mm_h > 0 && lw > 0 && lh > 0 {
                        0.5 * (lw as f64 / mm_w as f64 + lh as f64 / mm_h as f64)
                    } else {
                        0.0
                    };
                    if as_json {
                        out.push_str(&serde_json::json!({
                            "name": name,
                            "enabled": enabled,
                            "x": st.x, "y": st.y,
                            "mode_w": pw, "mode_h": ph, "refresh_mhz": refresh,
                            "scale": st.scale,
                            "logical_w": lw, "logical_h": lh,
                            "mm_w": mm_w, "mm_h": mm_h,
                            "px_per_mm": px_per_mm,
                            "ppi": px_per_mm * 25.4,
                            "source": source,
                        }).to_string());
                        out.push('\n');
                    } else {
                        out.push_str(&format!(
                            "output name={} enabled={} x={} y={} mode={}x{}@{:.3} scale={} logical={}x{} mm={}x{} px_per_mm={:.3} ppi={:.1} source={}\n",
                            name, enabled, st.x, st.y, pw, ph, refresh as f64 / 1000.0, st.scale, lw, lh, mm_w, mm_h, px_per_mm, px_per_mm * 25.4, source
                        ));
                    }
                }
                if out.is_empty() { "no outputs\n".to_string() } else { out }
            }
            "windows" => {
                // `windows --json` emits one JSON object per line; titles and
                // app_ids are then properly escaped, unlike the text format.
                let as_json = parts.get(1).copied() == Some("--json");
                let mut focused_window: *mut Window = std::ptr::null_mut();
                let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
                let mut curr_seat = (*seats_list).next;
                while curr_seat != seats_list {
                    let next_seat = (*curr_seat).next;
                    let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
                    if let crate::seat::Focus::Window(w) = (*seat).focused {
                        focused_window = w;
                        break;
                    }
                    curr_seat = next_seat;
                }

                let mut out = String::new();
                let sp = self.layout.snap_params();
                for &w in self.windows.iter() {
                    if !w.is_null() && !(*w).closed && !matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                        let app_id = (*w).get_app_id_string().unwrap_or_default();
                        let title = (*w).get_title_string().unwrap_or_default();
                        // Which desktop square(s) the window sits on, chess
                        // style: "C-9" for one, "C-9:D-9" for a block.
                        let cell = crate::policy::cells::window_span_label(
                            (*w).virtual_x,
                            (*w).virtual_y,
                            (*w).box_geom.width as f64,
                            (*w).box_geom.height as f64,
                            sp.cell_w,
                            sp.cell_h,
                            sp.gap_width,
                        );
                        if as_json {
                            out.push_str(&serde_json::json!({
                                "id": (*w).ref_key.index,
                                "app_id": app_id,
                                "title": title,
                                "mode": (*w).tiling_mode.as_str(),
                                "x": (*w).box_geom.x,
                                "y": (*w).box_geom.y,
                                "w": (*w).box_geom.width,
                                "h": (*w).box_geom.height,
                                "vx": (*w).virtual_x,
                                "vy": (*w).virtual_y,
                                "x11": match (*w).impl_type {
                                    crate::window::WindowImpl::Xwayland(xw) if !xw.is_null() => Some((*(*xw).xsurface).window_id),
                                    _ => None,
                                },
                                "cell": cell,
                                "minimized": (*w).minimized,
                                "has_parent": (*w).has_parent,
                                "focused": w == focused_window,
                                "ssd": (*w).wm_requested.ssd,
                                // Why a window has (or lacks) rounded corners,
                                // blur and shadow. Without it the only way to
                                // tell is a full-output screenshot: a
                                // per-window capture reads the client's
                                // dmabuf, which is pre-composite and never
                                // shows the compositor's clip.
                                "decorated": self.is_decorated_app(&app_id),
                                "beveled": self.is_beveled_app(&app_id),
                            }).to_string());
                            out.push('\n');
                        } else {
                            out.push_str(&format!(
                                "window id={} app_id={} title=\"{}\" mode={} x={} y={} w={} h={} vx={:.1} vy={:.1} cell={} minimized={} has_parent={} focused={} ssd={} decorated={} beveled={}\n",
                                (*w).ref_key.index,
                                app_id,
                                title,
                                (*w).tiling_mode.as_str(),
                                (*w).box_geom.x,
                                (*w).box_geom.y,
                                (*w).box_geom.width,
                                (*w).box_geom.height,
                                (*w).virtual_x,
                                (*w).virtual_y,
                                cell,
                                (*w).minimized,
                                (*w).has_parent,
                                w == focused_window,
                                (*w).wm_requested.ssd,
                                self.is_decorated_app(&app_id),
                                self.is_beveled_app(&app_id),
                            ));
                        }
                    }
                }
                out
            }
            "spawn" => {
                if parts.len() < 2 { return "error: missing command\n".to_string(); }
                let cmd = parts[1..].join(" ");
                self.execute_action(&crate::config::Action::Spawn, Some(&cmd));
                "ok\n".to_string()
            }
            "layout" => {
                if parts.len() < 3 { return "error: missing layout key or value\n".to_string(); }
                let key = parts[1];
                let val = parts[2];
                let old_sp = self.layout.snap_params();
                match key {
                    "desktop_gap_color" => {
                        self.layout.desktop_gap_color = val.to_string();
                        let parsed_color = crate::config::parse_hex_color(val);
                        self.layout.background_r = ((parsed_color >> 16) & 0xFF) * 0x01010101;
                        self.layout.background_g = ((parsed_color >> 8) & 0xFF) * 0x01010101;
                        self.layout.background_b = (parsed_color & 0xFF) * 0x01010101;
                        unsafe {
                            let outputs_head = &mut (*self.server).om.outputs as *mut crate::ffi::wl_list as *mut crate::server::WlList;
                            let mut curr = (*outputs_head).next;
                            while curr != outputs_head {
                                let next = (*curr).next;
                                let output = &mut *crate::container_of!(curr, crate::output::Output, link);
                                output.update_background_color();
                                curr = next;
                            }
                        }
                    }
                    "desktop_cell_color" => {
                        self.layout.desktop_cell_color = crate::config::parse_hex_color_rgba(val);
                    }
                    "desktop_grid_scale" | "grid_cell_size" => {
                        if let Ok(v) = val.parse::<f64>() {
                            self.layout.desktop_cell_width = v;
                            self.layout.desktop_cell_height = v;
                        }
                    }
                    "grid_cell_width" => {
                        if let Ok(v) = val.parse::<f64>() {
                            self.layout.desktop_cell_width = v;
                        }
                    }
                    "grid_cell_height" => {
                        if let Ok(v) = val.parse::<f64>() {
                            self.layout.desktop_cell_height = v;
                        }
                    }
                    "desktop_gap_width" => {
                        if let Ok(v) = val.parse::<i32>() {
                            self.layout.desktop_gap_width = v;
                        }
                    }
                    "desktop_cell_fade_inset" => {
                        if let Ok(v) = val.parse::<i64>() {
                            self.layout.desktop_cell_fade_inset = v;
                        }
                    }
                    "desktop_grid_fade_mode" => {
                        self.layout.desktop_grid_fade_mode = val.to_string();
                    }
                    "gap" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap = v; } }
                    "gap_top" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_top = v; } }
                    "gap_left" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_left = v; } }
                    "gap_right" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_right = v; } }
                    "gap_bottom" => { if let Ok(v) = val.parse::<i32>() { self.layout.gap_bottom = v; } }
                    "offset" | "cascade_offset" => { if let Ok(v) = val.parse::<i32>() { self.layout.cascade_offset = v; } }
                    "grid_gap" => { if let Ok(v) = val.parse::<i32>() { self.layout.grid_gap = v; } }
                    "transition_duration" => { if let Ok(v) = val.parse::<i32>() { self.layout.transition_duration = v; } }
                    "bar_height" => { if let Ok(v) = val.parse::<i32>() { self.layout.bar_height = v; } }

                    "side_panel_width" | "pinned_width" | "overlay_width" => { if let Ok(v) = val.parse::<i32>() { self.layout.overlay_width = v; } }
                    "side_panel_behavior" | "pinned_behavior" | "overlay_behavior" => { self.layout.overlay_behavior = val.to_string(); }
                    "side_panel_position" | "pinned_position" | "overlay_position" => { self.layout.overlay_position = val.to_string(); }
                    "side_panel_border_gap" | "pinned_border_gap" | "overlay_border_gap" => { if let Ok(v) = val.parse::<i32>() { self.layout.overlay_border_gap = v; } }
                    _ => return format!("error: unknown layout key: {}\n", key),
                }
                self.retile_for_grid_change(old_sp);
                self.remap_saved_entries(&old_sp);
                if key.starts_with("desktop_") || key.starts_with("grid_cell") {
                    self.invalidate_grid_patches();
                }
                self.dirty_windowing();
                "ok\n".to_string()
            }
            "mode" => {
                if parts.len() < 3 { return "error: missing mode or app_id\n".to_string(); }
                let mode = crate::config::parse_tiling_mode(parts[1]);
                let app_id = parts[2].to_string();
                let title = if parts.len() >= 4 { Some(parts[3..].join(" ")) } else { None };
                self.mode_rules.push(crate::config::ModeRule {
                    mode,
                    app_id_pattern: app_id,
                    title_pattern: title,
                    single_instance: false,
                    tag: -1,
                    circular: false,
                    ssd: None,
                });
                self.dirty_windowing();
                "ok\n".to_string()
            }
            "input" => {
                if parts.len() < 4 { return "error: usage: input <device_name|*> scroll-factor <value>\n".to_string(); }
                let device_name = parts[1];
                let key = parts[2];
                let val = parts[3];
                if key == "scroll-factor" {
                    if let Ok(factor) = val.parse::<f64>() {
                        if factor < 0.0 {
                            return "error: scroll factor cannot be negative\n".to_string();
                        }
                        let mut found = false;
                        let devices_head = &mut (*self.server).input_manager.devices as *mut ffi::wl_list as *mut WlList;
                        let mut curr = (*devices_head).next;
                        while curr != devices_head {
                            let next = (*curr).next;
                            let device = crate::container_of!(curr, crate::input_device::InputDevice, link);
                            let name_ptr = ffi::river_wlr_input_device_get_name((*device).wlr_device);
                            if !name_ptr.is_null() {
                                let name = std::ffi::CStr::from_ptr(name_ptr).to_string_lossy();
                                if device_name == "*" || name.contains(device_name) {
                                    (*device).config.scroll_factor = factor;
                                    found = true;
                                }
                            }
                            curr = next;
                        }
                        if found {
                            "ok\n".to_string()
                        } else {
                            "error: no matching device found\n".to_string()
                        }
                    } else {
                        "error: invalid scroll-factor value\n".to_string()
                    }
                } else {
                    format!("error: unknown input command: {}\n", key)
                }
            }
            // ── Synthetic pointer input (`ccectl pointer-*`): full wlrctl replacement
            // plus held buttons. Injection runs through the real cursor handlers
            // (`Cursor::inject_*`), so grabs/ops/focus behave exactly as with hardware.
            "pointer-move-to" => {
                if parts.len() < 3 { return "error: usage: pointer-move-to <x> <y>\n".to_string(); }
                if let (Ok(x), Ok(y)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    self.for_each_cursor(|cursor| cursor.inject_motion_to(x, y));
                    "ok\n".to_string()
                } else {
                    "error: invalid x or y\n".to_string()
                }
            }
            "pointer-move-by" => {
                if parts.len() < 3 { return "error: usage: pointer-move-by <dx> <dy>\n".to_string(); }
                if let (Ok(dx), Ok(dy)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                    self.for_each_cursor(|cursor| cursor.inject_motion_by(dx, dy));
                    "ok\n".to_string()
                } else {
                    "error: invalid dx or dy\n".to_string()
                }
            }
            "pointer-press" | "pointer-release" | "pointer-click" => {
                let button = match Self::parse_pointer_button(parts.get(1).copied()) {
                    Some(b) => b,
                    None => return "error: unknown button (left|right|middle|back|forward or an evdev code)\n".to_string(),
                };
                match action {
                    "pointer-press" => self.for_each_cursor(|cursor| cursor.inject_button(button, true)),
                    "pointer-release" => self.for_each_cursor(|cursor| cursor.inject_button(button, false)),
                    _ => self.for_each_cursor(|cursor| {
                        cursor.inject_button(button, true);
                        cursor.inject_button(button, false);
                    }),
                }
                "ok\n".to_string()
            }
            "pointer-scroll" => {
                if parts.len() < 2 { return "error: usage: pointer-scroll <dy> [dx] [finger|finger-stop]\n".to_string(); }
                if parts[1] == "finger-stop" {
                    self.for_each_cursor(|cursor| cursor.inject_finger_stop());
                    return "ok\n".to_string();
                }
                // `natural` marks the swipe as coming from a natural-scrolling
                // touchpad: the deltas are given as libinput would deliver
                // them (already sign-flipped), and the view drag undoes that.
                let finger = parts.iter().any(|p| *p == "finger");
                let natural = parts.iter().any(|p| *p == "natural");
                let dy = parts[1].parse::<f64>();
                let dx = parts.get(2).filter(|p| **p != "finger" && **p != "natural").map(|v| v.parse::<f64>()).unwrap_or(Ok(0.0));
                if let (Ok(dy), Ok(dx)) = (dy, dx) {
                    self.for_each_cursor(|cursor| {
                        cursor.inject_natural = natural;
                        cursor.inject_scroll(dy, dx, finger);
                        cursor.inject_natural = false;
                    });
                    "ok\n".to_string()
                } else {
                    "error: invalid dy or dx\n".to_string()
                }
            }
            "pointer-swipe" => {
                // pointer-swipe <fingers> <dx> <dy> [steps]
                if parts.len() < 4 { return "error: usage: pointer-swipe <fingers> <dx> <dy> [steps]\n".to_string(); }
                let fingers = parts[1].parse::<u32>();
                let dx = parts[2].parse::<f64>();
                let dy = parts[3].parse::<f64>();
                let steps = parts.get(4).map(|v| v.parse::<u32>()).unwrap_or(Ok(10));
                if let (Ok(fingers), Ok(dx), Ok(dy), Ok(steps)) = (fingers, dx, dy, steps) {
                    self.for_each_cursor(|cursor| cursor.inject_swipe(fingers, dx, dy, steps));
                    "ok\n".to_string()
                } else {
                    "error: invalid swipe arguments\n".to_string()
                }
            }
            "pointer-pinch" => {
                // pointer-pinch <scale> [rotation-degrees] [steps]
                // pointer-pinch begin | update <scale> [rotation] | end   (paced by the caller)
                if parts.len() < 2 { return "error: usage: pointer-pinch <scale> [rotation] [steps] | begin | update <scale> [rotation] | end\n".to_string(); }
                if matches!(parts[1], "begin" | "update" | "end") {
                    let scale = parts.get(2).and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
                    let rotation = parts.get(3).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                    let stage = parts[1].to_string();
                    self.for_each_cursor(|cursor| cursor.inject_pinch_stage(&stage, scale, rotation));
                    return "ok\n".to_string();
                }
                let scale = parts[1].parse::<f64>();
                let rotation = parts.get(2).map(|v| v.parse::<f64>()).unwrap_or(Ok(0.0));
                let steps = parts.get(3).map(|v| v.parse::<u32>()).unwrap_or(Ok(10));
                if let (Ok(scale), Ok(rotation), Ok(steps)) = (scale, rotation, steps) {
                    self.for_each_cursor(|cursor| cursor.inject_pinch(scale, rotation, steps));
                    "ok\n".to_string()
                } else {
                    "error: invalid pinch arguments\n".to_string()
                }
            }
            "touchpad-view-regions" => {
                // touchpad-view-regions <x11:ID|id|app_id> clear | <x,y,w,h> ...
                // An app named in `touchpad_view_apps` narrows the emulated
                // view drag (see `cursor::ViewDrag`) to rectangles of one of
                // its windows, in surface-local pixels — for an X11 window
                // under `xwayland_hidpi`, the physical pixels the client
                // itself measures in. A two-finger scroll outside them reaches
                // the client as the plain scroll it was, so Houdini's
                // parameter editor scrolls while its viewports still tumble;
                // it publishes its 3D viewports and network editors from
                // hou-control. `clear` restores the whole-window drag.
                if parts.len() < 3 {
                    return "error: usage: touchpad-view-regions <x11:ID|id|app_id> clear|<x,y,w,h> ...\n".to_string();
                }
                let Some(w) = window_by_query(self.windows.iter().copied(), parts[1]) else {
                    return format!("error: no mapped window matches {}\n", parts[1]);
                };
                if parts[2] == "clear" {
                    (*w).view_regions = None;
                    log::info!("[touchpad-view-regions] {} cleared", parts[1]);
                    return "ok\n".to_string();
                }
                let mut regions = Vec::new();
                for spec in &parts[2..] {
                    let v: Vec<f64> = spec.split(',').filter_map(|n| n.parse().ok()).collect();
                    if v.len() != 4 {
                        return format!("error: bad rect {} (want x,y,w,h)\n", spec);
                    }
                    regions.push([v[0], v[1], v[2], v[3]]);
                }
                log::info!("[touchpad-view-regions] {} -> {:?}", parts[1], regions);
                (*w).view_regions = Some(regions);
                "ok\n".to_string()
            }
            "place-next" => {
                // place-next <app_id> <x> <y>: one-shot hint — the next map of
                // a floating toplevel with this app_id lands near this layout
                // position (top-left, clamped on-screen) instead of its
                // remembered spot. Widgets send it with the pointer location
                // just before spawning a picker so it opens at the control.
                if parts.len() < 4 {
                    return "error: usage: place-next <app_id> <x> <y>\n".to_string();
                }
                let (x, y) = match (parts[2].parse::<f64>(), parts[3].parse::<f64>()) {
                    (Ok(x), Ok(y)) => (x, y),
                    _ => return "error: x/y must be numbers\n".to_string(),
                };
                let app_id = parts[1].to_string();
                self.pending_placements.retain(|(id, _, _, _, _)| id != &app_id);
                self.pending_placements.push((app_id, x, y, false, std::time::Instant::now()));
                "ok\n".to_string()
            }
            "place-next-cell" => {
                // place-next-cell <app_id|command> <x> <y>: the next map of a
                // matching window covers the GRID SQUARE containing this
                // layout point, keeping its remembered size and growing away
                // from whatever already occupies the neighbouring squares.
                // What the desktop menu and the launcher send: you asked for
                // the window somewhere, so it opens there rather than wherever
                // it happened to be last time.
                if parts.len() < 4 {
                    return "error: usage: place-next-cell <app_id> <x> <y>\n".to_string();
                }
                let (x, y) = match (parts[2].parse::<f64>(), parts[3].parse::<f64>()) {
                    (Ok(x), Ok(y)) => (x, y),
                    _ => return "error: x/y must be numbers\n".to_string(),
                };
                let app_id = parts[1].to_string();
                self.pending_placements.retain(|(id, _, _, _, _)| id != &app_id);
                self.pending_placements.push((app_id, x, y, true, std::time::Instant::now()));
                "ok\n".to_string()
            }
            "pointer-location" => {
                let mut reply = "error: no seat\n".to_string();
                let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
                let curr_seat = (*seats_list).next;
                if curr_seat != seats_list {
                    let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
                    let cursor = &(*seat).cursor;
                    reply = format!("x={} y={}\n", cursor.x(), cursor.y());
                }
                reply
            }
            // One-direction key events (held modifiers/keys); `keycode` is the evdev
            // code. Like `keypress`, this notifies the focused client directly and does
            // not run compositor keybindings. Modifier keycodes additionally update an
            // injected xkb mask and push a modifiers event, so the focused client's xkb
            // state tracks ctrl/shift/alt/super combos exactly as it would from
            // hardware (`wlr_seat_keyboard_notify_key` alone never changes modifier
            // state — that lives on the keyboard device, which injection bypasses).
            "key-down" | "key-up" => {
                if parts.len() < 2 { return "error: usage: key-down|key-up <keycode>\n".to_string(); }
                if let Ok(keycode) = parts[1].parse::<u32>() {
                    let pressed = action == "key-down";
                    let state = if pressed {
                        ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED
                    } else {
                        ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED
                    };
                    // evdev → real xkb modifier name (left/right pairs).
                    let mod_name: Option<&[u8]> = match keycode {
                        42 | 54 => Some(b"Shift\0"),
                        29 | 97 => Some(b"Control\0"),
                        56 | 100 => Some(b"Mod1\0"),
                        125 | 126 => Some(b"Mod4\0"),
                        _ => None,
                    };
                    let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
                    let mut curr_seat = (*seats_list).next;
                    while curr_seat != seats_list {
                        let next_seat = (*curr_seat).next;
                        let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
                        (*seat).ensure_synthetic_keyboard();
                        ffi::wlr_seat_keyboard_notify_key((*seat).wlr_seat, crate::util::msec_timestamp(), keycode, state);
                        if let Some(name) = mod_name {
                            let kb = ffi::river_wlr_seat_get_keyboard((*seat).wlr_seat);
                            if !kb.is_null() && !(*kb).keymap.is_null() {
                                let idx = ffi::xkb_keymap_mod_get_index((*kb).keymap, name.as_ptr() as *const _);
                                if idx != ffi::XKB_MOD_INVALID {
                                    let mask = 1u32 << idx;
                                    if pressed {
                                        self.injected_key_mods |= mask;
                                    } else {
                                        self.injected_key_mods &= !mask;
                                    }
                                    // Injected mask OR'd over the device's live state, so a
                                    // real keyboard keeps working mid-injection.
                                    let mut mods = (*kb).modifiers;
                                    mods.depressed |= self.injected_key_mods;
                                    ffi::wlr_seat_keyboard_notify_modifiers((*seat).wlr_seat, &mut mods);
                                }
                            }
                        }
                        curr_seat = next_seat;
                    }
                    "ok\n".to_string()
                } else {
                    "error: invalid keycode\n".to_string()
                }
            }
            "keypress" | "key-press" => {
                if parts.len() < 2 { return "error: usage: keypress <keycode>\n".to_string(); }
                if let Ok(keycode) = parts[1].parse::<u32>() {
                    let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
                    let mut curr_seat = (*seats_list).next;
                    while curr_seat != seats_list {
                        let next_seat = (*curr_seat).next;
                        let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
                        // A backend with no keyboard device leaves clients
                        // without a keymap, and a keymap-less client drops
                        // every key we notify. Attach one first.
                        (*seat).ensure_synthetic_keyboard();
                        let time = crate::util::msec_timestamp();
                        ffi::wlr_seat_keyboard_notify_key((*seat).wlr_seat, time, keycode, ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_PRESSED);
                        ffi::wlr_seat_keyboard_notify_key((*seat).wlr_seat, time + 1, keycode, ffi::wl_keyboard_key_state_WL_KEYBOARD_KEY_STATE_RELEASED);
                        curr_seat = next_seat;
                    }
                    "ok\n".to_string()
                } else {
                    "error: invalid keycode\n".to_string()
                }
            }
            _ => format!("error: unknown command: {}\n", action),
        }
    }

    /// Run `f` on every seat's cursor (the synthetic-input commands act on all seats,
    /// like the pre-existing pointer-move-to loop did).
    unsafe fn for_each_cursor(&mut self, mut f: impl FnMut(&mut crate::cursor::Cursor)) {
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            f(&mut (*seat).cursor);
            curr_seat = next_seat;
        }
    }

    /// Button-name/evdev-code parsing for the pointer commands; a missing argument
    /// means the left button, like wlrctl.
    fn parse_pointer_button(arg: Option<&str>) -> Option<u32> {
        match arg {
            None | Some("left") => Some(0x110),
            Some("right") => Some(0x111),
            Some("middle") => Some(0x112),
            Some("back") | Some("side") => Some(0x113),
            Some("forward") | Some("extra") => Some(0x114),
            Some(other) => other.parse::<u32>().ok(),
        }
    }

    pub unsafe fn apply_input_rules(&mut self) {
        if self.server.is_null() {
            return;
        }
        let devices_head = &mut (*self.server).input_manager.devices as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*devices_head).next;
        while curr != devices_head {
            let next = (*curr).next;
            let device = crate::container_of!(curr, crate::input_device::InputDevice, link);
            let name_ptr = ffi::river_wlr_input_device_get_name((*device).wlr_device);
            if !name_ptr.is_null() {
                let name = std::ffi::CStr::from_ptr(name_ptr).to_string_lossy();
                for rule in &self.input_rules {
                    if rule.name == "*" || name.contains(&rule.name) {
                        if let Some(factor) = rule.scroll_factor {
                            (*device).config.scroll_factor = factor;
                        }
                    }
                }
            }
            curr = next;
        }
    }

    pub unsafe fn apply_input_config(&mut self) {
        if self.server.is_null() {
            return;
        }
        let devices_head = &mut (*self.server).input_manager.devices as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*devices_head).next;
        while curr != devices_head {
            let next = (*curr).next;
            let device = crate::container_of!(curr, crate::input_device::InputDevice, link);
            if let Some(ref mut libinput) = (*device).libinput {
                libinput.apply_config(&self.input_config);
            }
            curr = next;
        }
    }

    pub unsafe fn spawn_startup_program(&mut self, prog: crate::config::StartupConfig) {
        log::info!("spawning startup program: {}", prog.exec);
        let cmd = prog.exec.clone();
        match nix::unistd::fork() {
            Ok(nix::unistd::ForkResult::Child) => {
                crate::process::cleanup_child();

                if !self.server.is_null() && !(*self.server).xwayland.is_null() {
                    let xwayland_cast = (*self.server).xwayland as *mut crate::server::WlrXwayland;
                    if !(*xwayland_cast).display_name.is_null() {
                        let display_name = std::ffi::CStr::from_ptr((*xwayland_cast).display_name)
                            .to_string_lossy()
                            .into_owned();
                        std::env::set_var("DISPLAY", display_name);
                    }
                }

                let env: Vec<std::ffi::CString> = std::env::vars()
                    .map(|(k, v)| std::ffi::CString::new(format!("{}={}", k, v)).unwrap())
                    .collect();
                let env_ptrs: Vec<&std::ffi::CStr> = env.iter().map(|s| s.as_c_str()).collect();
                let sh_c = std::ffi::CString::new("/bin/sh").unwrap();
                let c_c = std::ffi::CString::new("-c").unwrap();
                let cmd_c = std::ffi::CString::new(cmd).unwrap();
                let args = [sh_c.as_c_str(), c_c.as_c_str(), cmd_c.as_c_str()];
                let _ = nix::unistd::execve(&sh_c, &args, &env_ptrs);
                std::process::exit(1);
            }
            Ok(nix::unistd::ForkResult::Parent { child }) => {
                self.startup_pids.push((prog, child));
            }
            Err(e) => {
                log::error!("failed to fork child for startup program: {}", e);
            }
        }
    }

    /// Re-tile the SAVED Tiled entries (restore queue + last-window
    /// states) from `old` grid params onto the current grid — the
    /// stateful sibling of `retile_for_grid_change`, which can only reach
    /// mapped windows. Without this, a window closed under one grid and
    /// reopened under another restores misaligned pixels, touches extra
    /// cells, and the tiled snap grows it by a cell.
    pub unsafe fn remap_saved_entries(&mut self, old: &crate::policy::snap::SnapParams) {
        let new = self.layout.snap_params();
        for entry in self
            .restore_queue
            .iter_mut()
            .chain(self.last_window_states.iter_mut())
        {
            if entry.tiling_mode != crate::tiling::TilingMode::Tiled {
                continue;
            }
            if entry.width == 0 || entry.height == 0 {
                continue;
            }
            let (nx, ny, nw, nh) = crate::policy::cells::remap_block(
                entry.virtual_x,
                entry.virtual_y,
                entry.width as f64,
                entry.height as f64,
                old,
                &new,
            );
            entry.virtual_x = nx;
            entry.virtual_y = ny;
            entry.width = nw.round() as u32;
            entry.height = nh.round() as u32;
        }
    }

    /// Re-tile every Tiled window after a grid-geometry change (cell
    /// sizes, gap, or fade inset): each window keeps its BLOCK of squares
    /// (`cells::remap_block`), so it resizes with the grid instead of
    /// keeping its old pixel box and later spanning whatever new cells that
    /// box happens to touch. A window under an active seat op is left
    /// alone — the op owns its geometry until release. No-op when the
    /// geometry is unchanged.
    pub unsafe fn retile_for_grid_change(&mut self, old: crate::policy::snap::SnapParams) {
        let new = self.layout.snap_params();
        if old.cell_w == new.cell_w
            && old.cell_h == new.cell_h
            && old.gap_width == new.gap_width
            && old.cell_inset == new.cell_inset
        {
            return;
        }
        let op_win = self
            .first_seat()
            .and_then(|s| (*s).op.as_ref().map(|op| op.window_ptr))
            .unwrap_or(std::ptr::null_mut());
        let wins: Vec<*mut crate::window::Window> = self.windows.iter().copied().collect();
        for w in wins {
            if w.is_null() || (*w).closed || w == op_win {
                continue;
            }
            if !matches!((*w).state, crate::window::WindowState::Mapped) {
                continue;
            }
            if self.get_mode_for_window(w) != crate::tiling::TilingMode::Tiled {
                continue;
            }
            let (nx, ny, nw, nh) = crate::policy::cells::remap_block(
                (*w).virtual_x,
                (*w).virtual_y,
                (*w).box_geom.width as f64,
                (*w).box_geom.height as f64,
                &old,
                &new,
            );
            (*w).virtual_x = nx;
            (*w).virtual_y = ny;
            (*w).box_geom.width = nw.round() as i32;
            (*w).box_geom.height = nh.round() as i32;
            // The saved floating spot follows like move-window: a later
            // Tiled -> Floating exit restores at the remapped square rather
            // than yanking the window back across the resized grid.
            (*w).saved_floating_virtual_x = nx;
            (*w).saved_floating_virtual_y = ny;
        }
    }

    pub unsafe fn reload_config(&mut self) -> Result<(), String> {
        if let Some(path) = crate::config::default_config_path() {
            let old_sp = self.layout.snap_params();
            let old_pids = std::mem::take(&mut self.startup_pids);
            match crate::config::parse_config(&path, self) {
                Ok(()) => {
                    // Update scales of existing outputs from the newly loaded config
                    let om_outputs = &mut (*self.server).om.outputs as *mut ffi::wl_list;
                    let mut link = (*om_outputs).next;
                    while link != om_outputs {
                        let output = &mut *crate::container_of!(link, crate::output::Output, link);
                        let wlr_output = output.wlr_output;
                        if !wlr_output.is_null() {
                            let name_raw = ffi::river_wlr_output_get_name(wlr_output);
                            let name = std::ffi::CStr::from_ptr(name_raw).to_string_lossy();
                            let scale_key = format!("scale_{}", name);
                            let output_scale = self.display.get(&scale_key)
                                .map(|&s| s as f32)
                                .unwrap_or(self.output_scale);
                            if output.scheduled.scale != output_scale {
                                output.scheduled.scale = output_scale;
                            }
                        }
                        link = (*link).next;
                    }

                    self.retile_for_grid_change(old_sp);
                    // The saved entries hold geometry from before the
                    // reload too — closed windows must reopen on their
                    // squares, not their stale pixels.
                    self.remap_saved_entries(&old_sp);
                    // The grid client reads the same desktop keys and only
                    // repaints when handed a patch: hand it one.
                    self.invalidate_grid_patches();
                    self.dirty_windowing();

                    // Process old PIDs
                    for (old_prog, old_pid) in old_pids {
                        // If it is still in new startup and once == true, keep it running
                        let still_exists_and_once = self.startup.iter().any(|p| p.exec == old_prog.exec && p.once && !p.restart);
                        if still_exists_and_once {
                            self.startup_pids.push((old_prog, old_pid));
                        } else {
                            log::info!("Terminating old startup program pid {} ({})", old_pid, old_prog.exec);
                            let _ = nix::sys::signal::kill(old_pid, nix::sys::signal::Signal::SIGTERM);
                        }
                    }

                    // Spawn new/restarted programs
                    let current_startup = self.startup.clone();
                    for prog in current_startup {
                        if prog.once {
                            // Only spawn if not already running
                            let running = self.startup_pids.iter().any(|(p, _)| p.exec == prog.exec);
                            if !running {
                                self.spawn_startup_program(prog);
                            }
                        } else {
                            // once == false: spawn a new instance
                            self.spawn_startup_program(prog);
                        }
                    }

                    Ok(())
                }
                Err(e) => {
                    self.startup_pids = old_pids;
                    log::error!("failed to reload config: {}", e);
                    Err(e)
                }
            }
        } else {
            log::error!("no config file found to reload");
            Err("No config file found".to_string())
        }
    }
}

/// Local time as a fraction of the day (0 = midnight, 0.5 = noon) — the
/// input driving the light_source segment's perimeter position.
pub fn local_day_fraction() -> f64 {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return 0.5;
        }
        (tm.tm_hour as f64 * 3600.0 + tm.tm_min as f64 * 60.0 + tm.tm_sec as f64) / 86400.0
    }
}

/// Minute tick: the light_source segment's perimeter position depends on the
/// time of day, so a periodic re-arrange keeps it drifting (~a few px/min).
unsafe extern "C" fn handle_sun_timer(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    (*wm).dirty_windowing();
    ffi::wl_event_source_timer_update((*wm).sun_timer, 60_000);
    0
}

unsafe extern "C" fn handle_save_state_timer(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    (*wm).save_state_pending = false;
    (*wm).save_state();
    0
}

/// The IPC wake fd fired: clear it and dispatch every queued request.
unsafe extern "C" fn handle_ipc_event(fd: std::os::raw::c_int, _mask: u32, data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    crate::ipc_server::drain_wake_fd(fd);

    if let Some(ref rx) = (*wm).ipc_rx {
        while let Ok(req) = rx.try_recv() {
            // Lend the reply channel to the dispatch: a command whose real
            // outcome is only known later (`screenshot`, which lands a frame
            // from now) takes it and answers itself. If it is still here, the
            // command answered synchronously and its return value is the
            // reply.
            (*wm).pending_ipc_reply = Some(req.reply_tx);
            let reply = (*wm).process_ipc_command(&req.command);
            if let Some(tx) = (*wm).pending_ipc_reply.take() {
                let _ = tx.send(reply);
            }
        }
    }

    0
}

/// Window-stream tick: resolve each subscription (`focused` re-resolves per
/// tick, so streams follow focus), capture windows that are dirty (commit
/// listener set `stream_dirty`) or due a keepalive, and try_send frames to
/// the writer threads — never blocking the compositor (a full channel means
/// the client is slow and simply skips the frame). Fast cadence only while
/// subscribers exist.
/// A subscriber joined the stream hub: start (or keep) the frame tick.
unsafe extern "C" fn handle_stream_wake(fd: std::os::raw::c_int, _mask: u32, data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    crate::ipc_server::drain_wake_fd(fd);
    if !(*wm).stream_timer.is_null() {
        ffi::wl_event_source_timer_update((*wm).stream_timer, 1);
    }
    0
}

/// Runs at ~30 Hz while there are subscribers and not at all otherwise:
/// the tick simply does not re-arm once the subscriber list is empty, and
/// `handle_stream_wake` restarts it when the accept thread adds one. (It
/// used to re-arm at 200-500 ms forever, a 2-5 Hz idle wakeup for a feature
/// that is rarely in use.)
unsafe extern "C" fn handle_stream_timer(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = &mut *(data as *mut WindowManager);
    let idle_rearm = |wm: &WindowManager, ms: i32| {
        if !wm.stream_timer.is_null() {
            ffi::wl_event_source_timer_update(wm.stream_timer, ms);
        }
    };
    let Some(hub) = wm.stream_hub.clone() else {
        return 0;
    };
    let Ok(mut subs) = hub.subs.lock() else {
        // A poisoned lock never heals; a retry is still cheaper than a
        // permanently dead stream, and bounded to twice a second.
        idle_rearm(wm, 500);
        return 0;
    };
    if subs.is_empty() {
        return 0;
    }

    // Each unique window is captured at most once per tick, shared by Arc.
    let mut captured: Vec<(*mut Window, std::sync::Arc<crate::stream_server::Frame>)> = Vec::new();
    let mut dead: Vec<usize> = Vec::new();
    for i in 0..subs.len() {
        let win = if subs[i].query == "focused" {
            wm.focused_window()
        } else {
            wm.find_window_by_query(&subs[i].query)
        };
        if win.is_null() {
            continue;
        }
        let keepalive = subs[i].last_sent.elapsed().as_secs() >= 15;
        if !(*win).stream_dirty && !subs[i].needs_frame && !keepalive {
            continue;
        }
        let frame = match captured.iter().find(|(w, _)| *w == win) {
            Some((_, f)) => f.clone(),
            None => match crate::screenshot::capture_window_rgba(win) {
                Ok((rgba, w, h)) => {
                    let f = std::sync::Arc::new(crate::stream_server::Frame { width: w, height: h, rgba });
                    captured.push((win, f.clone()));
                    (*win).stream_dirty = false;
                    f
                }
                Err(_) => continue,
            },
        };
        match subs[i].tx.try_send(frame) {
            Ok(()) => {
                subs[i].needs_frame = false;
                subs[i].last_sent = std::time::Instant::now();
            }
            Err(std::sync::mpsc::TrySendError::Full(_)) => {} // slow client: drop frame
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => dead.push(i),
        }
    }
    for i in dead.into_iter().rev() {
        subs.remove(i);
    }
    idle_rearm(wm, 33);
    0
}

unsafe fn rendered_fullscreen(window: *mut Window) -> bool {
    (*window).is_fullscreen() && !(*window).rendering_requested.hidden
}

/// Prefer the installed `~/.local/bin/cce-cloud`, falling back to PATH lookup.
fn cce_cloud_cmd() -> String {
    if let Ok(home) = std::env::var("HOME") {
        let path = format!("{}/.local/bin/cce-cloud", home);
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }
    "cce-cloud".to_string()
}

/// Stdin of the currently open `cce-cloud --switcher` child. Held open (in
/// `ACTIVE_SWITCHER`) so repeat super+tab presses can advance the highlight via
/// cce-cloud's magic `__cce_switcher_next__` stdin line; the generation lets the
/// per-switcher cleanup thread avoid clearing a newer switcher's handle.
struct SwitcherHandle {
    generation: u64,
    stdin: std::process::ChildStdin,
}

static ACTIVE_SWITCHER: std::sync::Mutex<Option<SwitcherHandle>> = std::sync::Mutex::new(None);
static SWITCHER_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Tail of the window switcher, run on a detached thread: waits for the
/// committed selection on the already-spawned child's stdout, and asks the
/// compositor to focus it via the control socket (so the actual focus change
/// happens on the main thread through the IPC dispatcher).
fn run_window_switcher(
    mut child: std::process::Child,
    items: Vec<(String, String)>,
    display_env: Option<String>,
) {
    use std::io::{Read, Write};

    let mut selected = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut selected);
    }
    let _ = child.wait();

    let selected = selected.trim();
    if selected.is_empty() {
        return; // cancelled (Escape) or empty selection
    }

    let id = match items.iter().find(|(_, display)| display == selected) {
        Some((id, _)) => id.clone(),
        None => return,
    };

    let sock = match display_env {
        Some(d) => format!("/tmp/cce-{}.sock", d),
        None => "/tmp/cce.sock".to_string(),
    };
    if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&sock) {
        let _ = stream.write_all(format!("focus-window {}\n", id).as_bytes());
        let _ = stream.flush();
        let mut resp = String::new();
        let _ = stream.read_to_string(&mut resp);
    }
}

unsafe extern "C" fn dirty_idle_callback(data: *mut std::ffi::c_void) {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    (*wm).dirty_idle = std::ptr::null_mut();
    
    if matches!((*wm).state, WindowManagerState::Idle) {
        if (*wm).scheduled.dirty || (*wm).scheduled.dirty_lazy {
            (*wm).scheduled.dirty = true;
            (*wm).scheduled.dirty_lazy = false;
            (*wm).manage_start();
        } else if (*wm).rendering_scheduled.dirty {
            (*wm).render_start();
        }
    }
}

/// Two saved entries describe the same app: same app_id and the same
/// command line (an X11 class alone is as coarse as "python3").
fn same_app(a: &SavedWindowState, b: &SavedWindowState) -> bool {
    a.app_id == b.app_id && a.cmdline == b.cmdline
}

unsafe extern "C" fn handle_clean_exit_timeout(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    log::info!("Clean exit timeout reached with windows still open; cancelling the logout.");
    (*wm).cancel_clean_exit();
    0
}

unsafe extern "C" fn handle_timeout(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }

    match (*wm).state {
        WindowManagerState::InflightConfigures(count) => {
            log::error!("timeout occurred, some imperfect frames may be shown");
            assert!(count > 0);
            (*wm).state = WindowManagerState::InflightConfigures(0);
            (*wm).render_start();
        }
        WindowManagerState::Manage | WindowManagerState::Render => {
            if !(*wm).object.is_null() {
                log::error!("window manager unresponsive for more than 3 seconds, disconnecting");
                ffi::wl_resource_post_error(
                    (*wm).object,
                    ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_UNRESPONSIVE,
                    b"unresponsive for more than 3 seconds\0".as_ptr() as *const _,
                );
                let client = ffi::wl_resource_get_client((*wm).object);
                ffi::wl_client_destroy(client);
            }
        }
        WindowManagerState::Idle => {}
    }
    0
}

unsafe extern "C" fn handle_server_destroy(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let wm = crate::container_of!(listener, WindowManager, server_destroy);
    (*wm).deinit();
}

// WM request handlers
unsafe extern "C" fn wm_stop(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if !wm.is_null() {
        (*wm).object = std::ptr::null_mut();
        ffi::wl_resource_post_event(resource, ffi::ZCCE_WINDOW_MANAGER_V1_FINISHED);
        ffi::wl_resource_set_implementation(
            resource,
            &INERT_WM_INTERFACE as *const _ as *const _,
            std::ptr::null_mut(),
            None,
        );
    }
}

unsafe extern "C" fn wm_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn wm_manage_finish(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    if !matches!((*wm).state, WindowManagerState::Manage) {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
            b"manage_finish request does not match manage_start\0".as_ptr() as *const _,
        );
        return;
    }
    (*wm).manage_finish();
}

unsafe extern "C" fn wm_manage_dirty(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    (*wm).scheduled.dirty_lazy = true;
    (*wm).add_dirty_idle();
}

unsafe extern "C" fn wm_render_finish(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    if !matches!((*wm).state, WindowManagerState::Render) {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_SEQUENCE_ORDER,
            b"render_finish request does not match render_start\0".as_ptr() as *const _,
        );
        return;
    }
    (*wm).render_finish();
}

unsafe extern "C" fn wm_get_shell_surface(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    surface_resource: *mut ffi::wl_resource,
) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    let surface = ffi::wlr_surface_from_resource(surface_resource);
    let version = ffi::wl_resource_get_version(resource) as u32;
    if let Err(e) = crate::shell_surface::ShellSurface::create(client, version, id, surface, (*wm).server) {
        log::error!("Failed to create shell surface: {}", e);
        ffi::wl_client_post_no_memory(client);
    }
}

unsafe extern "C" fn wm_exit_session(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    log::info!("window manager requested to exit session");
    ffi::wl_display_terminate((*(*wm).server).wl_server);
}

static WM_INTERFACE: ffi::zcce_window_manager_v1_interface = ffi::zcce_window_manager_v1_interface {
    stop: Some(wm_stop),
    destroy: Some(wm_destroy),
    manage_finish: Some(wm_manage_finish),
    manage_dirty: Some(wm_manage_dirty),
    render_finish: Some(wm_render_finish),
    get_shell_surface: Some(wm_get_shell_surface),
    exit_session: Some(wm_exit_session),
    get_cce_toplevel: Some(crate::cce_window_management::cce_wm_get_cce_toplevel),
};

static INERT_WM_INTERFACE: ffi::zcce_window_manager_v1_interface = ffi::zcce_window_manager_v1_interface {
    stop: None,
    destroy: Some(wm_destroy),
    manage_finish: None,
    manage_dirty: None,
    render_finish: None,
    get_shell_surface: None,
    exit_session: None,
    get_cce_toplevel: None,
};

unsafe extern "C" fn bind(
    client: *mut ffi::wl_client,
    data: *mut std::ffi::c_void,
    version: u32,
    id: u32,
) {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return;
    }

    let mut pid = 0;
    let mut uid = 0;
    let mut gid = 0;
    ffi::wl_client_get_credentials(client, &mut pid, &mut uid, &mut gid);
    let cmdline = std::fs::read_to_string(format!("/proc/{}/cmdline", pid))
        .unwrap_or_default()
        .replace('\0', " ");
    log::info!("Client binding zcce_window_manager_v1: PID={}, cmdline='{}'", pid, cmdline);

    let resource = ffi::wl_resource_create(client, &ffi::zcce_window_manager_v1_interface, version as i32, id);
    if resource.is_null() {
        ffi::wl_client_post_no_memory(client);
        log::error!("out of memory binding zcce_window_manager_v1");
        return;
    }

    // We do not set (*wm).object = resource, so the built-in window manager remains active.
    // We just set the implementation to WM_INTERFACE so the client can call get_cce_toplevel.
    ffi::wl_resource_set_implementation(
        resource,
        &WM_INTERFACE as *const _ as *const _,
        wm as *mut _,
        Some(handle_destroy_wm_resource),
    );
}

unsafe extern "C" fn handle_destroy_wm_resource(resource: *mut ffi::wl_resource) {
    let wm = ffi::wl_resource_get_user_data(resource) as *mut WindowManager;
    if wm.is_null() {
        return;
    }
    if (*wm).object != resource {
        return;
    }
    log::debug!("active zcce_window_manager_v1 destroyed");
    (*wm).object = std::ptr::null_mut();

    let server = (*wm).server;

    // Iterate over outputs and make inert
    let outputs_list = &mut (*server).om.outputs as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*outputs_list).next;
    while curr != outputs_list {
        let next = (*curr).next;
        let output = crate::container_of!(curr, crate::output::Output, link);
        (*output).make_inert();
        curr = next;
    }

    // Iterate over seats and make inert
    let seats_list = &mut (*server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
    curr = (*seats_list).next;
    while curr != seats_list {
        let next = (*curr).next;
        let seat = crate::container_of!(curr, crate::seat::Seat, link);
        (*seat).make_inert();
        
        let bindings_head = &mut (*seat).xkb_bindings as *mut ffi::wl_list as *mut WlList;
        let mut curr_b = (*bindings_head).next;
        while curr_b != bindings_head {
            let next_b = (*curr_b).next;
            let binding = crate::container_of!(curr_b, crate::xkb_bindings::XkbBinding, link);
            (*binding).wm_scheduled.state_changes.clear();
            curr_b = next_b;
        }
        
        curr = next;
    }

    // Iterate over windows and make inert
    for &window in (*wm).windows.iter() {
        (*window).make_inert();
    }

    match (*wm).state {
        WindowManagerState::Idle | WindowManagerState::InflightConfigures(_) => {}
        WindowManagerState::Manage => (*wm).manage_finish(),
        WindowManagerState::Render => (*wm).render_finish(),
    }
}

/// Debounce, in ms, between the last viewport motion and blur being restored.
/// Long enough to outlast the ~16ms gaps between discrete pan/zoom updates (so
/// blur is not restored mid-gesture), short enough that blur returns promptly.
const VIEWPORT_SETTLE_MS: i32 = 120;

/// One-shot timer callback: the viewport has been still for `VIEWPORT_SETTLE_MS`,
/// so restore the blurred render state.
pub(crate) unsafe extern "C" fn handle_viewport_settle_tick(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    (*wm).finish_viewport_settle();
    0
}

/// The mechanism half of the trait boundary: apply one policy `Command`
/// against the scene/seat world. Command order matters — the apply loop is a
/// flat sequence, mirroring the arrange-plan convention.
impl crate::policy::api::Compositor for WindowManager {
    fn apply(&mut self, cmd: &crate::policy::api::Command) {
        use crate::policy::api::Command;
        unsafe {
            match *cmd {
                Command::Spawn(ref cmdline) => {
                    self.execute_action(&crate::config::Action::Spawn, Some(cmdline));
                }
                Command::SetCamera { camera, overview, animate } => {
                    // The mode flips immediately either way, so a re-toggle
                    // mid-flight exits/enters rather than re-entering.
                    if let Some(overview) = overview {
                        self.set_mode(if overview { WindowManagerMode::Overview } else { WindowManagerMode::Normal });
                    }
                    if animate {
                        if let Some((_, duration_ms)) = self.layout.overview_anim {
                            // Ramp-driven: a fixed-duration transition from
                            // the camera as it stands (a re-toggle mid-flight
                            // restarts the ramp from here). The exponential
                            // targets stay clear — the ramp owns the camera.
                            self.camera_ramp_anim = Some(CameraRampAnim {
                                start: self.camera(),
                                target: camera,
                                started_ns: crate::util::timestamp_ns(),
                                duration_ms,
                            });
                            self.target_desk_pan_x = None;
                            self.target_desk_pan_y = None;
                            self.target_desk_zoom = None;
                        } else {
                            self.camera_ramp_anim = None;
                            self.target_desk_pan_x = Some(camera.pan_x);
                            self.target_desk_pan_y = Some(camera.pan_y);
                            self.target_desk_zoom = Some(camera.zoom);
                        }
                        self.start_panning_animation();
                    } else {
                        self.desk_pan_x = camera.pan_x;
                        self.desk_pan_y = camera.pan_y;
                        self.desk_zoom = camera.zoom;
                        // An instant write cancels any easing in flight — the
                        // stale targets would otherwise drag the camera back.
                        self.target_desk_pan_x = None;
                        self.target_desk_pan_y = None;
                        self.target_desk_zoom = None;
                        self.camera_ramp_anim = None;
                    }
                }
                Command::PanTo { x, y } => {
                    if let Some(x) = x {
                        self.target_desk_pan_x = Some(x);
                    }
                    if let Some(y) = y {
                        self.target_desk_pan_y = Some(y);
                    }
                    self.start_panning_animation();
                }
                Command::StopPanAnimation => self.stop_panning_animation(),
                Command::FocusNextVisible => {
                    if let Some(seat) = self.first_seat() {
                        self.focus_next_visible_window(seat);
                    }
                }
                Command::Raise(id) => {
                    if let Some(&win) = self.windows.get(id.0) {
                        if !win.is_null() && !(*win).closed {
                            self.raise_window(win);
                        }
                    }
                }
                Command::CloseWindow(id) => {
                    if let Some(&win) = self.windows.get(id.0) {
                        if !win.is_null() && !(*win).closed {
                            (*win).close();
                        }
                    }
                }
                Command::SetMinimized { id, minimized } => {
                    if let Some(&win) = self.windows.get(id.0) {
                        if !win.is_null() && !(*win).closed {
                            (*win).minimized = minimized;
                        }
                    }
                }
                Command::SetWindowMode { id, mode, locked } => {
                    if let Some(&win) = self.windows.get(id.0) {
                        if !win.is_null() && !(*win).closed {
                            // Remember what Fullscreen replaced, so the
                            // toggle's exit can put it back (policy
                            // `actions::fullscreen`). A re-lock while
                            // already Fullscreen keeps the first record.
                            if mode == crate::tiling::TilingMode::Fullscreen {
                                if (*win).tiling_mode != crate::tiling::TilingMode::Fullscreen {
                                    (*win).pre_fullscreen = Some(((*win).tiling_mode, (*win).mode_locked));
                                }
                            } else {
                                // Coming back Tiled from Fullscreen is a
                                // RETURN, not a fresh entry: the arrange
                                // pass's Enter transition would snapshot
                                // the current box — still the output-sized
                                // fullscreen box at this point — as the
                                // window's floating geometry, and a later
                                // un-tile would pop it to screen size.
                                // Restoring `was_tiled` with the mode keeps
                                // the floating geometry saved before the
                                // window was tiled in the first place.
                                if (*win).tiling_mode == crate::tiling::TilingMode::Fullscreen
                                    && mode == crate::tiling::TilingMode::Tiled
                                    && matches!((*win).pre_fullscreen, Some((crate::tiling::TilingMode::Tiled, _)))
                                {
                                    (*win).was_tiled = true;
                                }
                                (*win).pre_fullscreen = None;
                            }
                            (*win).tiling_mode = mode;
                            (*win).mode_locked = locked;
                        }
                    }
                }
                Command::Focus(id) => {
                    if let Some(&win) = self.windows.get(id.0) {
                        if !win.is_null() && !(*win).closed {
                            if let Some(seat) = self.first_seat() {
                                (*seat).focus(crate::seat::Focus::Window(win));
                                if !(*seat).object.is_null() && !(*win).object.is_null() {
                                    ffi::wl_resource_post_event((*seat).object, 4, (*win).object);
                                }
                            }
                        }
                    }
                }
                Command::MoveWindow { id, x, y } => {
                    if let Some(&win) = self.windows.get(id.0) {
                        if !win.is_null() && !(*win).closed {
                            (*win).virtual_x = x;
                            (*win).virtual_y = y;
                        }
                    }
                }
                Command::SetOverlayPosition(side) => {
                    self.layout.overlay_position = match side {
                        crate::policy::api::OverlaySide::Left => "left".to_string(),
                        crate::policy::api::OverlaySide::Right => "right".to_string(),
                    };
                }
                Command::Relayout => self.dirty_windowing(),
                Command::RefreshCamera => {
                    if matches!(self.state, WindowManagerState::Idle) {
                        self.update_viewport_local();
                    } else {
                        self.dirty_windowing();
                    }
                }
            }
        }
    }
}

/// One dim frame standing in for a restored window until its program maps.
pub struct RestorePlaceholder {
    pub rect: *mut ffi::wlr_scene_rect,
    pub app_id: String,
    pub title: String,
    pub vx: f64,
    pub vy: f64,
    pub w: u32,
    pub h: u32,
}

/// Sweep placeholders whose programs never came back.
pub(crate) unsafe extern "C" fn handle_restore_placeholder_timeout(
    data: *mut std::ffi::c_void,
) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if !wm.is_null() {
        log::info!(
            "[Restore] Sweeping {} placeholder(s) whose windows never mapped",
            (*wm).restore_placeholders.len()
        );
        (*wm).clear_restore_placeholders();
    }
    0
}

/// 16ms edge auto-pan tick: scroll the desktop by the current velocity, then
/// re-run the seat op at its last cursor position so the dragged window keeps
/// tracking the (pinned) cursor — the op's pan-delta term turns the scroll
/// into window motion. op_update re-derives the velocity and re-arms this
/// timer, so the loop sustains itself until the op ends or the cursor leaves
/// the edge bands; then it stops without re-arming.
pub(crate) unsafe extern "C" fn handle_edge_pan_tick(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    let (vx, vy) = ((*wm).edge_pan_vx, (*wm).edge_pan_vy);
    if vx == 0.0 && vy == 0.0 {
        return 0;
    }
    let seat = match (*wm).first_seat() {
        Some(seat) if (*seat).op.is_some() => seat,
        _ => {
            (*wm).edge_pan_vx = 0.0;
            (*wm).edge_pan_vy = 0.0;
            return 0;
        }
    };
    let (ox, oy) = {
        let op = (*seat).op.as_ref().unwrap();
        (op.x, op.y)
    };
    let dt = 0.016;
    let zoom = (*wm).desk_zoom.max(0.01);
    (*wm).desk_pan_x += vx * dt / zoom;
    (*wm).desk_pan_y += vy * dt / zoom;
    (*seat).op_update(ox, oy);
    0
}

/// A duration-based camera transition timed through the configured
/// overview speed ramp. Interpolation is anchor-stable
/// (`camera::anchored_interp`): the whole flight is a zoom about the one
/// point that keeps the same screen position under both cameras, so the
/// transition reads as a direct zoom rather than a slide-while-zooming.
pub struct CameraRampAnim {
    pub start: crate::policy::camera::Camera,
    pub target: crate::policy::camera::Camera,
    /// Presentation-clock start (CLOCK_MONOTONIC ns); progress is read
    /// against each frame's predicted present time, never the wall clock.
    pub started_ns: u64,
    pub duration_ms: f64,
}

/// Interval of the camera-animation watchdog. The camera steps in the
/// output frame handler; this timer only re-requests a frame while an
/// animation is live, so a frame pipeline that goes quiet (nothing else
/// damaged, an output that stopped delivering frames) cannot strand it.
const CAMERA_WATCHDOG_MS: i32 = 50;

pub(crate) unsafe extern "C" fn handle_panning_animation_tick(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if wm.is_null() {
        return 0;
    }
    if (*wm).camera_anim_active {
        (*wm).schedule_frame_all_outputs();
        if !(*wm).animation_timer.is_null() {
            ffi::wl_event_source_timer_update((*wm).animation_timer, CAMERA_WATCHDOG_MS);
        }
    }
    0
}

/// Steps every window's border hover fade — and any in-flight
/// fullscreen-toggle animation — until all of them have settled. Windows at
/// rest cost one comparison per zone and no repaint, so leaving this running
/// for the tail of a fade is cheap.
unsafe extern "C" fn handle_border_fade_tick(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    let mut moving = false;
    let windows: Vec<*mut crate::window::Window> = (*wm).windows.iter().copied().collect();
    for window in windows {
        if window.is_null() || (*window).closed {
            continue;
        }
        if (*window).step_border_fade() {
            moving = true;
        }
        if (*window).step_fs_anim() {
            (*window).render_finish();
            moving = true;
        }
    }

    if moving {
        ffi::wl_event_source_timer_update((*wm).border_fade_timer, 16);
        let outputs_list = &mut (*(*wm).server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs_list).next;
        while curr != outputs_list {
            let next = (*curr).next;
            let output = crate::container_of!(curr, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                ffi::wlr_output_schedule_frame((*output).wlr_output);
            }
            curr = next;
        }
    } else {
        (*wm).border_fade_running = false;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_in_view_regions_is_half_open() {
        let r = [[10.0, 20.0, 100.0, 50.0], [500.0, 0.0, 10.0, 10.0]];
        assert!(point_in_view_regions(&r, 10.0, 20.0));
        assert!(point_in_view_regions(&r, 109.9, 69.9));
        assert!(!point_in_view_regions(&r, 110.0, 30.0));
        assert!(!point_in_view_regions(&r, 50.0, 70.0));
        assert!(point_in_view_regions(&r, 505.0, 5.0));
        assert!(!point_in_view_regions(&r, 0.0, 0.0));
    }

    /// An app that reports no view pane at all gets no drag: that is not
    /// the same as never having reported (`None`), which keeps the whole
    /// window.
    #[test]
    fn point_in_view_regions_empty_matches_nothing() {
        assert!(!point_in_view_regions(&[], 0.0, 0.0));
    }

    #[test]
    fn app_id_matches_is_exact_without_a_star() {
        assert!(app_id_matches("claude-desktop", "claude-desktop"));
        assert!(!app_id_matches("claude-desktop", "com.anthropic.Claude"));
        // An exact pattern must not match a longer id that merely contains it,
        // or `rounded_apps "foot"` would take in "footbar".
        assert!(!app_id_matches("foot", "footbar"));
        assert!(!app_id_matches("oot", "foot"));
    }

    #[test]
    fn app_id_matches_ignores_case() {
        assert!(app_id_matches("com.anthropic.claude", "com.anthropic.Claude"));
        assert!(app_id_matches("*CLAUDE*", "com.anthropic.Claude"));
    }

    /// The regression this matcher exists for: one pattern spanning an app's
    /// rename, so the window does not silently lose its decoration.
    #[test]
    fn app_id_matches_spans_a_rename() {
        for id in ["claude-desktop", "com.anthropic.Claude", "Claude"] {
            assert!(app_id_matches("*claude*", id), "{id} should match *claude*");
        }
        assert!(!app_id_matches("*claude*", "org.inkscape.Inkscape"));
    }

    #[test]
    fn app_id_matches_anchors_the_ends() {
        assert!(app_id_matches("com.anthropic.*", "com.anthropic.Claude"));
        assert!(!app_id_matches("com.anthropic.*", "org.example.anthropic"));
        assert!(app_id_matches("*.Claude", "com.anthropic.Claude"));
        assert!(!app_id_matches("*.Claude", "com.anthropic.ClaudeX"));
        // A trailing anchor may not re-consume what the leading one took.
        assert!(!app_id_matches("ab*ab", "ab"));
        assert!(app_id_matches("ab*ab", "abab"));
    }

    #[test]
    fn app_id_matches_handles_degenerate_patterns() {
        assert!(app_id_matches("*", "anything"));
        assert!(app_id_matches("*", ""));
        assert!(app_id_matches("**", "anything"));
        assert!(!app_id_matches("", "anything"));
        assert!(app_id_matches("", ""));
        // Interior segments consume left to right and may repeat.
        assert!(app_id_matches("a*b*c", "axxbyyc"));
        assert!(!app_id_matches("a*b*c", "acb"));
    }

    #[test]
    #[allow(invalid_value)]
    fn test_last_window_state_matching() {
        let mut wm = unsafe { std::mem::MaybeUninit::<WindowManager>::zeroed().assume_init() };
        unsafe {
            std::ptr::write(&mut wm.last_window_states, Vec::new());
        }
        
        wm.last_window_states.push(SavedWindowState {
            app_id: "test-app".to_string(),
            title: "My App Window".to_string(),
            tiling_mode: crate::tiling::TilingMode::Floating,
            minimized: false,
            virtual_x: 100.0,
            virtual_y: 200.0,
            scale: 1.0,
            width: 800,
            height: 600,
            cmdline: "test-app".to_string(),
            focused: false,
        });

        unsafe {
            // Test exact match
            let matched = wm.match_last_window_state("test-app", "My App Window");
            assert!(matched.is_some());
            let m = matched.unwrap();
            assert_eq!(m.app_id, "test-app");
            assert_eq!(m.virtual_x, 100.0);
            assert_eq!(m.virtual_y, 200.0);

            // Test fuzzy title match
            let matched_fuzzy = wm.match_last_window_state("test-app", "My App Window*");
            assert!(matched_fuzzy.is_some());

            // Test app_id only match
            let matched_appid = wm.match_last_window_state("test-app", "Different Title");
            assert!(matched_appid.is_some());
            assert_eq!(matched_appid.unwrap().width, 800);

            // Test no match
            let no_match = wm.match_last_window_state("other-app", "My App Window");
            assert!(no_match.is_none());
        }

        std::mem::forget(wm);
    }

    /// A scratch dir holding one executable (or not) file per name.
    struct BinDir(std::path::PathBuf);
    impl BinDir {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!(
                "cce-fx-shadowed-{}-{}-{}",
                std::process::id(),
                tag,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&d).unwrap();
            BinDir(d)
        }
        fn file(&self, name: &str, executable: bool) -> String {
            use std::os::unix::fs::PermissionsExt;
            let p = self.0.join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 })).unwrap();
            p.to_string_lossy().into_owned()
        }
        fn path(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }
    }
    impl Drop for BinDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_wrapper_first_on_path_restores_by_name() {
        let wrappers = BinDir::new("wrap");
        let system = BinDir::new("sys");
        wrappers.file("inkscape", true);
        let real = system.file("inkscape", true);
        let path = format!("{}:{}", wrappers.path(), system.path());
        assert_eq!(path_shadowed_name(&real, &path), Some("inkscape".to_string()));
    }

    #[test]
    fn the_same_binary_first_on_path_keeps_the_absolute_path() {
        let system = BinDir::new("sys");
        let real = system.file("inkscape", true);
        // Directly...
        assert_eq!(path_shadowed_name(&real, &system.path()), None);
        // ...and through a symlink farm ahead of it on PATH.
        let links = BinDir::new("links");
        std::os::unix::fs::symlink(&real, links.0.join("inkscape")).unwrap();
        let path = format!("{}:{}", links.path(), system.path());
        assert_eq!(path_shadowed_name(&real, &path), None);
    }

    #[test]
    fn a_non_executable_namesake_does_not_count() {
        let junk = BinDir::new("junk");
        let system = BinDir::new("sys");
        junk.file("inkscape", false);
        let real = system.file("inkscape", true);
        let path = format!("{}:{}", junk.path(), system.path());
        assert_eq!(path_shadowed_name(&real, &path), None);
    }

    #[test]
    fn only_absolute_argv0_of_an_existing_binary_is_considered() {
        let wrappers = BinDir::new("wrap");
        wrappers.file("inkscape", true);
        assert_eq!(path_shadowed_name("inkscape", &wrappers.path()), None);
        let gone = wrappers.0.join("nope/inkscape").to_string_lossy().into_owned();
        assert_eq!(path_shadowed_name(&gone, &wrappers.path()), None);
    }

    #[test]
    fn secret_service_gating_picks_only_keyring_clients() {
        // The real restored cmdline that lost the race against the keyring.
        assert!(WindowManager::needs_secret_service(
            "/usr/lib/claude-desktop/claude-desktop --password-store=gnome-libsecret"
        ));
        assert!(WindowManager::needs_secret_service(
            "bitwarden --password-store=kwallet6 --ozone-platform=wayland"
        ));
        // Electron's plaintext fallback never reaches the Secret Service, so
        // gating it would delay the app for nothing.
        assert!(!WindowManager::needs_secret_service(
            "some-app --password-store=basic"
        ));
        // Everything else starts immediately. KeePassXC is an ordinary app now
        // that gnome-keyring provides the Secret Service — it reaches for no
        // keyring of its own at startup, so it is not gated.
        assert!(!WindowManager::needs_secret_service("/usr/bin/keepassxc"));
        assert!(!WindowManager::needs_secret_service(
            "/home/lsgalante/.local/bin/cce-terminal"
        ));
        assert!(!WindowManager::needs_secret_service(""));
    }

    /// The barrier must actually hold while the collection reports locked, and
    /// release promptly once it flips — that ordering is the whole fix, so it
    /// is exercised here against a stub `busctl` rather than reasoned about.
    #[test]
    fn keyring_barrier_holds_until_unlocked_then_releases() {
        let dir = std::env::temp_dir().join(format!("cce-barrier-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let flag = dir.join("unlocked");
        let shim = dir.join("busctl");
        // Reports the name owned, and the collection locked until `flag` exists.
        std::fs::write(
            &shim,
            format!(
                "#!/bin/sh\n\
                 case \"$*\" in\n\
                 *NameHasOwner*) echo 'b true' ;;\n\
                 *Locked*) [ -e {flag} ] && echo 'b false' || echo 'b true' ;;\n\
                 esac\n",
                flag = flag.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shim, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{old_path}", dir.display()));

        // Flip the stub to "unlocked" shortly after the wait begins.
        let flag_writer = flag.clone();
        let unlock_at = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            std::fs::write(&flag_writer, b"").unwrap();
        });

        let started = std::time::Instant::now();
        WindowManager::wait_for_secret_service(1);
        let waited = started.elapsed();
        t.join().unwrap();
        std::env::set_var("PATH", old_path);
        let _ = std::fs::remove_dir_all(&dir);

        // Held for the locked window...
        assert!(
            started + waited >= unlock_at,
            "barrier released before the collection unlocked (waited {waited:?})"
        );
        // ...and did not sit there afterwards.
        assert!(
            waited < std::time::Duration::from_secs(5),
            "barrier did not release promptly after unlock (waited {waited:?})"
        );
    }
}
