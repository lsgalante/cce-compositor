// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlList};
use crate::slotmap::SlotMap;
use std::hash::{Hash, Hasher};

pub use crate::window::Window;

pub use crate::xwayland_override_redirect::XwaylandOverrideRedirect;

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

/// Why `update_grid_patches` is issuing a grid patch — the log tag, and
/// whether an identical patch may be skipped (every reason but a style
/// reload, which re-renders the same rect on purpose).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PatchReason {
    /// The displayed patch no longer covers the viewport, or no longer
    /// suits its resolution.
    Coverage,
    /// A camera flight's destination, sized to land before the ramp does.
    Flight,
    /// At rest after a flight: the roomy rect a pan wants back.
    Upgrade,
    /// The style config changed under the rendered pixels.
    StyleReload,
}

impl PatchReason {
    fn tag(self) -> &'static str {
        match self {
            PatchReason::Coverage => "coverage",
            PatchReason::Flight => "flight",
            PatchReason::Upgrade => "rest upgrade",
            PatchReason::StyleReload => "style reload",
        }
    }
}

pub struct WindowManagerScheduled {
    pub dirty: bool,
    pub dirty_lazy: bool,
    pub output_config: *mut ffi::wlr_output_configuration_v1,
}

pub struct WindowManagerSent {
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

/// The longest `focus-window --wait` holds its reply. A focus pan lands in
/// well under a second; this only bounds a window that never holds still.
pub const SETTLE_TIMEOUT_MS: u64 = 3000;
/// How often a held `focus-window --wait` looks at its window.
const SETTLE_POLL_MS: i32 = 16;
/// Consecutive polls with nothing moving that count as settled: one poll
/// can fall between two steps of an animation that is not the camera's.
const SETTLE_STILL_POLLS: u32 = 3;

/// A `focus-window --wait` caller, answered once its window has stopped
/// moving on screen.
///
/// Focusing a window can move it: a window hanging off the view is panned
/// into it, and the pan is animated. A script that clicks right after
/// `focus-window` clicked where the window was going to be, mid-flight —
/// cce-fonts' search box "did not take focus" in a shadow, while its large
/// preview box, which a mid-flight click still lands in, did.
pub struct SettleWaiter {
    pub tx: std::sync::mpsc::Sender<String>,
    pub window: crate::slotmap::Key,
    pub started_ns: u64,
    last_box: Option<(i32, i32, i32, i32)>,
    still: u32,
}

impl SettleWaiter {
    pub fn new(tx: std::sync::mpsc::Sender<String>, window: crate::slotmap::Key, started_ns: u64) -> Self {
        Self { tx, window, started_ns, last_box: None, still: 0 }
    }

    /// One poll: the window's box on screen now, and whether anything that
    /// moves windows is still in flight (the camera, a pending relayout).
    /// True once the box has held still for [`SETTLE_STILL_POLLS`] polls.
    fn observe(&mut self, screen_box: (i32, i32, i32, i32), moving: bool) -> bool {
        if moving || self.last_box != Some(screen_box) {
            self.still = 0;
        } else {
            self.still += 1;
        }
        self.last_box = Some(screen_box);
        self.still >= SETTLE_STILL_POLLS
    }
}

/// Whether a relayout is pending or under way. `manage_start` clears
/// `scheduled.dirty` as the sequence begins, but a window's on-screen box
/// only moves at `render_finish`, so a transaction waiting on a client's
/// configure ack reads as a window at rest unless the state counts too.
fn layout_in_flight(state: WindowManagerState, dirty: bool) -> bool {
    dirty || !matches!(state, WindowManagerState::Idle)
}

#[path = "window_manager/session.rs"]
mod session;
pub(crate) use session::*;
#[path = "window_manager/camera.rs"]
mod camera;
#[path = "window_manager/transaction.rs"]
mod transaction;
#[path = "window_manager/grid_patches.rs"]
mod grid_patches;
#[path = "window_manager/switcher.rs"]
mod switcher;
#[path = "window_manager/ipc_commands.rs"]
mod ipc_commands;
#[cfg(test)]
use ipc_commands::parse_finite;
#[path = "window_manager/config_apply.rs"]
mod config_apply;

pub struct WindowManager {
    pub server: *mut Server,
    pub global: *mut ffi::wl_global,
    pub server_destroy: crate::listener::Listener,
    pub state: WindowManagerState,
    pub windows: SlotMap<*mut Window>,
    /// The overview drag-selection: the selected windows, the rubber band
    /// while one is being dragged out, and the nodes that draw both. See
    /// [`crate::selection`].
    pub selection: crate::selection::Selection,
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
    /// Armed by a camera flight (overview enter/exit, a zoom target): keeps
    /// the fallback cells on after landing until the grid client's LATCHED
    /// patch reaches the whole viewport — see the gate in `arrange_views`.
    pub grid_cells_hold: bool,
    pub layout: crate::config::Layout,
    pub mode_rules: Vec<crate::config::ModeRule>,
    pub keybinds: Vec<crate::config::Keybind>,
    pub pointer_binds: Vec<crate::config::PointerBind>,
    pub gesture_binds: Vec<crate::config::GestureBind>,
    /// Chords bound through the GlobalShortcuts portal backend — see
    /// `global_shortcuts`. Matched after `keybinds`, never persisted.
    pub portal_shortcuts: Vec<crate::global_shortcuts::PortalShortcut>,
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
    /// PID of the client that sent the IPC command currently being
    /// dispatched, alongside `pending_ipc_reply`. `fade-out` resolves "the
    /// caller's own window" with it. 0 outside a dispatch.
    pub pending_ipc_peer_pid: i32,
    /// `focus-window --wait` callers whose reply is held until the window
    /// stops moving on screen (see [`SettleWaiter`]).
    pub settle_waiters: Vec<SettleWaiter>,
    /// Polls `settle_waiters`; created on the first wait, armed while any
    /// waiter is left.
    pub settle_timer: *mut ffi::wl_event_source,
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
    /// `window_manager { osk_on_touch }` (`osk.rs`).
    pub osk_on_touch: bool,
    /// `window_manager { swipe_peek }`: the desktop's lean toward a
    /// directional swipe bind at its threshold, screen px (default 60;
    /// 0 disables). See `cursor::swipe_peek_for`.
    pub swipe_peek_px: f64,
    /// `window_manager { swipe_repeat_peek }`: the lean toward each further
    /// step once a swipe has switched focus, screen px at
    /// `swipe_repeat_threshold` (default half of `swipe_peek_px`).
    pub swipe_repeat_peek_px: f64,
    /// `window_manager { swipe_focus_cone }`: degrees off a focus swipe's
    /// direction within which a window center can take focus (default 45).
    /// See `focus_toward`.
    pub swipe_focus_cone_deg: f64,
    /// `window_manager { swipe_threshold }`: accumulated swipe travel
    /// (libinput units) at which a swipe bind fires (default 70).
    pub swipe_threshold: f64,
    /// `window_manager { swipe_repeat_threshold }`: the travel each FURTHER
    /// fire of the same swipe needs after its first (default four times
    /// `swipe_threshold`) — the resistance that keeps a swipe from
    /// running on through a second window.
    pub swipe_repeat_threshold: f64,
    /// See `WindowManagerConfig::touchpad_hscroll_shift_apps`.
    pub touchpad_hscroll_shift_apps: Vec<String>,
    /// Live override-redirect X11 surfaces (menus, tooltips, combo lists),
    /// so the per-frame pass can re-apply their 1/scale dest size — the
    /// scene's own commit listener resets it on every commit.
    pub override_redirects: Vec<*mut XwaylandOverrideRedirect>,
    pub display: std::collections::HashMap<String, f64>,
    pub input_rules: Vec<crate::config::InputDeviceConfigRule>,
    pub input_config: crate::config::InputConfig,
    pub last_status_update: std::cell::RefCell<Option<crate::status_server::StatusUpdate>>,
    pub status_hide_mode: bool,
    pub adjust_position_mode: bool,
    /// Window-adjust mode: Super is held. The focused window shows its
    /// frame (handles) and its body drags it, as in overview — the two
    /// are one predicate, `window_adjust_active`. Set from the keyboard's
    /// modifier state (`refresh_adjust_held`), never assigned directly.
    pub adjust_held: bool,
    /// A Super held through `ccectl key-down 125|126`, which bypasses the
    /// keyboard device the modifier mask is read from.
    pub injected_super_held: bool,
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
    ///
    /// Compared as compact JSON: the pretty form is only built for a write.
    pub last_saved_state_json: Option<String>,
    /// `proc_args` per window pid. A save reads every window's argv, and each
    /// read is `/proc/<pid>/cmdline` plus a stat per `PATH` entry and two
    /// canonicalizes (`path_shadowed_name`) — once a second while anything
    /// moves. A live window's pid cannot be reused, and the entries of pids
    /// no longer on a window are dropped at each save.
    pub proc_args_cache: std::collections::HashMap<i32, Vec<String>>,
    /// X11 apps' learned minimum sizes, persisted in `min-sizes.json`.
    pub min_sizes: crate::min_sizes::MinSizes,
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
    /// A pinch zoom waiting for the next output frame: (zoom, anchor x,
    /// anchor y) in output-local px. libinput delivers pinch updates faster
    /// than the refresh rate; the last one before a frame wins, so each
    /// frame samples the gesture once instead of relaying out per event.
    pub pinch_pending: Option<(f64, f64, f64)>,
    /// The zoom when the current viewport gesture froze the blur bakes;
    /// settling re-bakes only if the zoom moved away from it.
    pub viewport_freeze_zoom: f64,
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
    /// to close, with the program that owned it (`proc_args` argv[0], when
    /// still readable) and when. A client that loses its Wayland connection
    /// lands here and reappears a moment later having rebuilt its surface;
    /// see `take_recent_vanish`.
    pub vanished_windows: Vec<(String, Option<String>, std::time::Instant)>,
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
        self.swipe_peek_px = 60.0;
        self.swipe_repeat_peek_px = 30.0;
        self.swipe_focus_cone_deg = 45.0;
        self.swipe_threshold = 70.0;
        self.swipe_repeat_threshold = 280.0;
        self.touchpad_view_invert = false;
        self.osk_on_touch = true;
        self.touchpad_hscroll_shift_apps = Vec::new();
        self.display = std::collections::HashMap::new();
        self.input_rules = Vec::new();
        self.input_config = crate::config::InputConfig::default();
        self.mode = WindowManagerMode::Normal;
        Ok(())
    }

    pub unsafe fn init_with_server(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        self.global = std::ptr::null_mut();
        self.state = WindowManagerState::Idle;
        self.windows = SlotMap::new();
        std::ptr::write(&mut self.selection, crate::selection::Selection::default());
        self.focus_history = Vec::new();
        self.scheduled = WindowManagerScheduled {
            dirty: false,
            dirty_lazy: false,
            output_config: std::ptr::null_mut(),
        };
        self.sent = WindowManagerSent {
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
        self.pinch_pending = None;
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
        self.pending_ipc_peer_pid = 0;
        self.settle_timer = std::ptr::null_mut();
        self.mode = WindowManagerMode::Normal;
        self.on_app_exit = crate::config::OnAppExit::FocusPrevious;
        self.grid_cells_enabled = true;
        self.grid_cells_hold = false;
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
        self.swipe_peek_px = 60.0;
        self.swipe_repeat_peek_px = 30.0;
        self.swipe_focus_cone_deg = 45.0;
        self.swipe_threshold = 70.0;
        self.swipe_repeat_threshold = 280.0;
        self.touchpad_view_invert = false;
        self.osk_on_touch = true;
        self.touchpad_hscroll_shift_apps = Vec::new();
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
        self.portal_shortcuts = Vec::new();
        self.ipc_rx = None;
        self.ipc_source = std::ptr::null_mut();
        self.ipc_wake = None;
        self.save_state_timer = std::ptr::null_mut();
        self.save_state_pending = false;
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
        self.status_hide_mode = false;
        self.adjust_position_mode = false;
        self.adjust_held = false;
        self.injected_super_held = false;
        self.injected_key_mods = 0;

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

        ffi::wl_display_add_destroy_listener((*server).wl_server, self.server_destroy.prepare(handle_server_destroy));

        Ok(())
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

    pub unsafe fn start_ipc(&mut self, display_socket: Option<String>) {
        if self.ipc_rx.is_none() {
            // Lock before logind's sleeps (lid, power key) — a real seat only.
            if !(*self.server).session.is_null() {
                crate::sleep_lock::spawn(display_socket.clone());
            }
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
        if !self.settle_timer.is_null() {
            ffi::wl_event_source_remove(self.settle_timer);
            self.settle_timer = std::ptr::null_mut();
        }
        // Dropping a waiter drops its reply channel: the caller is told the
        // command timed out rather than left hanging.
        self.settle_waiters.clear();
        if !self.restore_placeholder_timer.is_null() {
            ffi::wl_event_source_remove(self.restore_placeholder_timer);
            self.restore_placeholder_timer = std::ptr::null_mut();
        }
        // Placeholder rects go down with the scene; only the bookkeeping.
        // Released, not dropped: dropping a handle would destroy its rect
        // here, mid display teardown.
        for mut p in self.restore_placeholders.drain(..) {
            p.rect.release();
        }
        if !self.viewport_settle_timer.is_null() {
            ffi::wl_event_source_remove(self.viewport_settle_timer);
            self.viewport_settle_timer = std::ptr::null_mut();
        }
        self.server_destroy.disconnect();
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
            let window = (*node).window();
            if !window.is_null() {
                rendered.insert(window as usize);
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
                && !(*w).is_overlay_ui()
                && !(*w).is_shy();
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

    /// Poll the held `focus-window --wait` replies in [`SETTLE_POLL_MS`].
    unsafe fn arm_settle_timer(&mut self) {
        if self.settle_timer.is_null() {
            let event_loop = ffi::wl_display_get_event_loop((*self.server).wl_server);
            self.settle_timer = ffi::wl_event_loop_add_timer(
                event_loop,
                Some(handle_settle_tick),
                self as *mut WindowManager as *mut _,
            );
        }
        if self.settle_timer.is_null() {
            // No timer: answer now rather than never.
            for w in self.settle_waiters.drain(..) {
                let _ = w.tx.send("ok\n".to_string());
            }
            return;
        }
        ffi::wl_event_source_timer_update(self.settle_timer, SETTLE_POLL_MS);
    }

    /// Answer each `focus-window --wait` whose window has stopped moving
    /// on screen, or has waited [`SETTLE_TIMEOUT_MS`], or has closed.
    unsafe fn poll_settle_waiters(&mut self) {
        let moving = self.camera_anim_active
            || self.pan_pending != [0.0, 0.0]
            || self.pinch_pending.is_some()
            || layout_in_flight(self.state, self.scheduled.dirty);
        let now = crate::util::timestamp_ns();
        let mut waiters = std::mem::take(&mut self.settle_waiters);
        let mut kept = Vec::new();
        for mut w in waiters.drain(..) {
            let win = self.windows.get(w.window).copied().filter(|&p| !p.is_null() && !(*p).closed);
            let Some(win) = win else {
                let _ = w.tx.send("error: window closed before it settled\n".to_string());
                continue;
            };
            let b = (*win).box_geom;
            if w.observe((b.x, b.y, b.width, b.height), moving) {
                let _ = w.tx.send("ok\n".to_string());
            } else if now.saturating_sub(w.started_ns) >= SETTLE_TIMEOUT_MS * 1_000_000 {
                let _ = w.tx.send(format!("ok (still moving after {} ms)\n", SETTLE_TIMEOUT_MS));
            } else {
                kept.push(w);
            }
        }
        self.settle_waiters = kept;
        if !self.settle_waiters.is_empty() {
            self.arm_settle_timer();
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
/// Whether `rule` names a window with this app_id and title. Both match
/// as substrings. A window that never set a title matches as the empty
/// string, so `title=""` (which every title contains) reaches an untitled
/// popup too — the Claude app's quick-entry window sets none, and without
/// this it fell through to its main window's saved entry.
pub fn mode_rule_matches(rule: &crate::config::ModeRule, app_id: Option<&str>, title: Option<&str>) -> bool {
    let match_app = rule.app_id_pattern == "*" || app_id.map_or(false, |aid| aid.contains(&rule.app_id_pattern));
    let match_title = rule.title_pattern.as_deref().map_or(true, |tp| title.unwrap_or("").contains(tp));
    match_app && match_title
}

impl WindowManager {
    // Add legacy fields so structural offsets are preserved if layout-based code is compiled
    pub fn sent_outputs_compat(&self) {}

    pub unsafe fn get_rule_for_window(&self, win: *mut Window) -> Option<&crate::config::ModeRule> {
        self.rule_for((*win).app_id_str(), (*win).title_str())
    }

    /// The first mode rule matching this app_id and title. Borrowed: the
    /// arrange snapshot asks once per window per transaction — once a vblank
    /// during a drag — and used to allocate both strings, twice.
    fn rule_for(&self, app_id: Option<&str>, title: Option<&str>) -> Option<&crate::config::ModeRule> {
        self.mode_rules.iter().find(|rule| mode_rule_matches(rule, app_id, title))
    }

    pub unsafe fn get_mode_for_window(&self, win: *mut Window) -> crate::tiling::TilingMode {
        self.mode_for_window(win, (*win).app_id_str(), || self.get_rule_for_window(win))
    }

    /// `get_mode_for_window` with the app_id already in hand and the mode
    /// rule asked for only if it is needed.
    unsafe fn mode_for_window<'a>(
        &'a self,
        win: *mut Window,
        app_id: Option<&str>,
        rule: impl FnOnce() -> Option<&'a crate::config::ModeRule>,
    ) -> crate::tiling::TilingMode {
        if (*win).is_status_bar() {
            return crate::tiling::TilingMode::Status;
        }
        if app_id == Some("cce-notifier") || app_id == Some("cce-notification-daemon") || app_id == Some("clear-notification-daemon") {
            return crate::tiling::TilingMode::Popup;
        }
        // An explicit set_popup via the cce window-management protocol beats the
        // app_id heuristic below: a cce-cloud toplevel that flagged itself a popup
        // sizes itself (dmenu-style) instead of taking the overlay dock's
        // full-height fresh slot.
        if (*win).tiling_mode == crate::tiling::TilingMode::Popup {
            return crate::tiling::TilingMode::Popup;
        }
        if app_id.map_or(false, |id| id.starts_with("cce-cloud")) {
            return crate::tiling::TilingMode::Overlay;
        }



        if (*win).mode_locked {
            return (*win).tiling_mode;
        }

        if (*win).has_parent {
            return crate::tiling::TilingMode::Floating;
        }

        if let Some(rule) = rule() {
            return rule.mode;
        }

        crate::tiling::TilingMode::Floating
    }

    /// How far a pointer op has carried, in virtual units
    /// (`policy::drag::virtual_delta`): its travel over the zoom, plus the
    /// camera's pan since the grab.
    pub(crate) fn op_virtual_delta(&self, op: &crate::seat::SeatOp) -> (f64, f64) {
        crate::policy::drag::virtual_delta(
            op.x - op.start_x,
            op.y - op.start_y,
            self.desk_zoom,
            (self.desk_pan_x, self.desk_pan_y),
            (op.start_pan_x, op.start_pan_y),
        )
    }

    /// The size a resize op gives its window (`policy::drag::resize_to`),
    /// before the client's min/max hint is applied. The seat op's Resize arm
    /// and `get_active_resize_dimensions` both ask this, so the arrange
    /// snapshot cannot disagree with the drag.
    pub(crate) fn op_resize_size(&self, op: &crate::seat::SeatOp, edges: crate::window::Edges) -> (u32, u32) {
        // Zoom-aware: the felt grab distance stays constant in screen px.
        let sp = self.layout.snap_params().for_zoom(self.desk_zoom);
        crate::policy::drag::resize_to(
            (op.start_win_virtual_x, op.start_win_virtual_y),
            (op.start_win_w, op.start_win_h),
            self.op_virtual_delta(op),
            edges.into(),
            op.start_was_tiled,
            &sp,
        )
    }

    pub unsafe fn get_active_resize_dimensions(&self, win_ptr: *mut Window) -> Option<(u32, u32)> {
        let seats_list = &(*self.server).input_manager.seats as *const ffi::wl_list as *const WlList as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if let Some(ref op) = (*seat).op {
                if op.window_ptr == win_ptr {
                    if let crate::seat::PointerOpType::Resize { edges } = op.op_type {
                        let (new_w, new_h) = self.op_resize_size(op, edges);
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
                // A window carried along by a group move is being moved as
                // much as the one under the pointer — or as the image under
                // it, when the group was grabbed by one.
                if matches!(
                    op.op_type,
                    crate::seat::PointerOpType::Move | crate::seat::PointerOpType::GroupMove
                ) {
                    if (*seat).group_move.iter().any(|&(w, _, _)| w == win_ptr) {
                        return true;
                    }
                }
            }
            curr_seat = (*curr_seat).next;
        }
        false
    }



    /// Pan the overview camera, at its current zoom, just far enough to
    /// show all of `win` — the map-time treatment for a window SPAWNED
    /// during overview, which leaves the mode alone. A camera ramp (an
    /// overview enter still flying) owns the camera, and its mid-flight
    /// sample would give a stale target, so the window is left to land
    /// wherever the flight shows it.
    pub unsafe fn pan_overview_to_window(&mut self, win: *mut Window) {
        if win.is_null() || self.camera_ramp_anim.is_some() {
            return;
        }
        let win_w = if (*win).box_geom.width > 0 { (*win).box_geom.width as f64 } else { 800.0 };
        let win_h = if (*win).box_geom.height > 0 { (*win).box_geom.height as f64 } else { 600.0 };
        self.pan_to_virtual_rect((*win).virtual_x, (*win).virtual_y, win_w, win_h);
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
            // Read once per window: the rule lookup, the status test and the
            // mode below all need them (they each re-read and allocated).
            let app_id = (*win_ptr).app_id_str();
            let rule = self.rule_for(app_id, (*win_ptr).title_str());
            let rule_ssd = if !(*win_ptr).mode_locked {
                rule.and_then(|rule| rule.ssd)
            } else {
                None
            };
            // Status segments: refresh the frozen collapsed slot length
            // while at bar thickness; while EXPANDED (in-surface menu, the
            // surface is thicker than the bar) keep the frozen value and
            // raise the segment above its siblings and the windows the open
            // menu now overlaps.
            if app_id.map_or(false, |id| id.starts_with("cce-status")) {
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
                mode: self.mode_for_window(win_ptr, app_id, || rule),
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

        // Placement sees the pan floored to a layout pixel; the remainder
        // shifts the desk trees at render time (`layout_camera`).
        let (layout_cam, sub_x, sub_y) = self.layout_camera();
        ffi::river_scene_set_desk_subpixel((*self.server).scene.wlr_scene, sub_x, sub_y);
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
            pan_x: layout_cam.pan_x,
            pan_y: layout_cam.pan_y,
            zoom: self.desk_zoom,
        };

        let plan = crate::policy::arrange::arrange(&window_snaps, &output_snaps, &params);

        for &output in &active_outputs {
            (*output).background_rect.set_enabled(plan.background_rect_enabled);
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
        // And AFTER the flight, until the client's patch has actually
        // landed: the ramp's last frame used to switch the cells off
        // regardless, and a client still rendering the flight's (large)
        // patch — a 4096²-buffer re-render, sometimes longer than the
        // 250ms ramp — left bare backdrop at the screen edges until its
        // commit latched: "the edge cells appear a moment after the
        // animation ends". So at rest the cells also stay on while the
        // LATCHED patch does not reach the whole viewport, which the
        // latch's own arrange then switches off. Not during a pan gesture
        // (`viewport_is_active`), per the re-bake cost above.
        // The hold is ARMED by a flight, not by coverage alone, so a pure
        // pan that outruns its patch never toggles the pool; and it is
        // keyed on the latched patch rather than on `viewport_is_active`,
        // because the ramp's last frame arranges while the viewport still
        // counts as moving, and the settle that clears that flag 120ms
        // later never arranges — the only arrange after landing is the
        // latch's own, which is exactly the one that should switch the
        // cells off.
        let flight = self.camera_ramp_anim.is_some() || self.target_desk_zoom.is_some();
        if flight {
            self.grid_cells_hold = true;
        }
        // Coverage is geometry only, so the patch a flight was issued for
        // can still be rendering while its PREDECESSOR covers the landed
        // viewport — the cells would switch off a beat before the client
        // swaps buffers, and anything the swap costs a frame (a swapchain
        // rebuilt for the new buffer extent) would show as bare backdrop.
        // So the hold also spans a patch still in the air.
        let in_air = self.grid_patch_in_air();
        let uncovered = self.grid_cells_hold && (in_air || !self.grid_patch_covers_viewport());
        if self.grid_cells_hold && !flight && !uncovered {
            self.grid_cells_hold = false;
        }
        let cells_wanted = plan.grid_cells_enabled || flight || uncovered;
        if self.grid_cells_enabled != cells_wanted {
            log::info!(
                "[Grid] fallback cells {} (no_client={} flight={} held_uncovered={})",
                if cells_wanted { "on" } else { "off" },
                plan.grid_cells_enabled,
                flight,
                uncovered,
            );
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
                ffi::wlr_scene_node_set_enabled((*win_ptr).tree.node(), enabled);
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

        self.place_fullscreen_windows();

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
                    ffi::river_scene_node_set_position_if_changed((*win_ptr).tree.node(), x, y);
                    ffi::river_scene_node_set_position_if_changed((*win_ptr).popup_tree.node(), x, y);
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
        // A window that just moved, resized or restacked may now cover the
        // adjust target, or no longer: let the overlap dim re-evaluate.
        if self.window_adjust_active() {
            self.arm_border_fade();
        }
    }

    /// A fullscreen window is pinned to its output only while it owns the
    /// top of the stack outside overview. Stepped aside
    /// (`Window::fullscreen_yields`), or in overview, it
    /// sits on the desk at the spot it covered — `virtual_x/y`, at output
    /// size, scaled with the zoom — so a focus chord's pan leaves it behind
    /// like any other window. Until 2026-10-03 it stayed pinned behind the
    /// windows the camera panned to, as if it were the backdrop. It also
    /// rides the desk while the camera eases back onto it after a refocus
    /// (`Seat::focus_follow_pan` aims at `fullscreen_anchor_pan`), so it
    /// slides in and lands pinned with nothing to jump.
    ///
    /// While pinned, its desk spot follows the camera: whatever the camera
    /// shows is what the window covers, so a pan made while it is on top
    /// moves where it will be left. The policy reads the same spot, so
    /// directional focus measures from it. Not before `was_fullscreen`:
    /// `manage_finish` saves `virtual_x/y` as the restore position on the
    /// entering transition (and sets the first desk spot there), which
    /// this would overwrite.
    unsafe fn place_fullscreen_windows(&mut self) {
        let zoom = self.desk_zoom;
        let mut spot_moved = false;
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed {
                continue;
            }
            let fullscreen = rendered_fullscreen(w)
                && (*w).was_fullscreen
                && matches!((*w).state, crate::window::WindowState::Mapped);
            if !fullscreen {
                (*w).fs_on_desk = false;
                continue;
            }
            let returning = match ((*w).fullscreen_anchor_pan(), self.target_desk_pan_x, self.target_desk_pan_y) {
                (Some((ax, ay)), Some(tx), Some(ty)) => (tx - ax).abs() < 0.5 && (ty - ay).abs() < 0.5,
                _ => false,
            };
            // Overview shows the desk, so the window is on it there; and
            // through a camera flight (the overview ramp, or an eased zoom)
            // it flies with the desk rather than snapping to the output at
            // either end — an exit onto it lands exactly on its spot
            // (`exit_onto_window` centres its output-sized rect).
            let flying = self.camera_ramp_anim.is_some() || self.target_desk_zoom.is_some();
            let on_desk = returning
                || flying
                || self.mode == WindowManagerMode::Overview
                || (*w).fullscreen_yields();
            if on_desk {
                let (sx, sy) = (*w).virtual_to_screen((*w).virtual_x, (*w).virtual_y);
                (*w).rendering_requested.x = sx;
                (*w).rendering_requested.y = sy;
                (*w).scale = zoom;
            } else {
                let output = (*w).fullscreen_output();
                if !output.is_null() {
                    let (vx, vy) = (*w).screen_to_virtual((*output).sent.x, (*output).sent.y);
                    // The spot is saved (`fullscreen_at`), and a camera
                    // pan relays out without a transaction, which is
                    // what normally schedules the save — so a game closed
                    // after a pan would reopen where the pan began.
                    spot_moved |= (vx - (*w).virtual_x).abs() >= 0.5 || (vy - (*w).virtual_y).abs() >= 0.5;
                    (*w).virtual_x = vx;
                    (*w).virtual_y = vy;
                }
            }
            (*w).fs_on_desk = on_desk;
        }
        if spot_moved {
            self.schedule_save_state();
        }
    }

    pub unsafe fn update_viewport_local(&mut self) {
        let zoom_changed = self.desk_zoom != self.last_viewport_zoom;
        // A pan counts as motion only once it moves a DEVICE pixel: the
        // desk renders on integer layout px plus a device-px sub-pixel
        // shift (`layout_camera`), so the tail of an eased pan below that
        // changes nothing on screen, and repainting the whole output for
        // it was pure cost.
        let zoom = self.desk_zoom;
        let scale = self.max_output_scale();
        let px = |pan: f64| (pan * zoom * scale).round();
        let pan_changed = px(self.desk_pan_x) != px(self.last_viewport_pan_x)
            || px(self.desk_pan_y) != px(self.last_viewport_pan_y);
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
            if !self.viewport_is_active {
                self.viewport_freeze_zoom = self.last_viewport_zoom;
            }
            self.viewport_is_active = true;
            // Every motion frame moves the screen-sized backdrop under every
            // blurred window; without this, scenefx re-bakes every optimized
            // blur every frame of the pan. Through a pure pan the bakes are
            // frozen and each window samples the cache where its own bake
            // lives (the coordinates it last baked at), reading exactly its
            // own bake — correct, not stale, because the backdrop moved with
            // it. A zoom changes the scale under the window, which no shift
            // can compensate — but re-baking every blur on every frame of a
            // zoom was the most expensive thing the desktop did, and the
            // mismatch is a low-frequency blur under a window in flight for
            // a few hundred ms. So the bakes stay frozen through zooms too,
            // and `finish_viewport_settle` re-bakes once if the zoom moved.
            let scene = (*self.server).scene.wlr_scene;
            ffi::river_scene_set_blur_frozen(scene, true);
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
        // Thaw the blur caches so the settled frame re-bakes against the
        // final backdrop — every bake, if the gesture changed the zoom
        // (they were sampled at the old scale in flight); otherwise only
        // the ones the thaw itself distrusts.
        let scene = (*self.server).scene.wlr_scene;
        ffi::river_scene_set_blur_frozen(scene, false);
        if self.desk_zoom != self.viewport_freeze_zoom {
            ffi::river_scene_mark_optimized_blur_dirty(scene);
        }
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
        self.windows.iter().any(|&w| w != except && self.is_expanded_status_segment(w))
    }

    /// A mapped status segment thicker than the bar — one whose in-surface
    /// menu is open. The thickness IS the signal: the bar grows its own
    /// surface into the menu and shrinks it back on close.
    pub unsafe fn is_expanded_status_segment(&self, w: *mut crate::window::Window) -> bool {
        let bar_h = self.layout.bar_height;
        !w.is_null()
            && !(*w).closed
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

    /// Record that this app_id's window disappeared unbidden, and which
    /// program owned it (`None` once the process is gone).
    pub fn note_vanished(&mut self, app_id: String, program: Option<String>) {
        let now = std::time::Instant::now();
        self.vanished_windows
            .retain(|(_, _, at)| now.duration_since(*at) < RECONNECT_FOCUS_GRACE);
        self.vanished_windows.push((app_id, program, now));
    }

    /// Whether this window's program recently lost one of its windows
    /// unbidden within the grace — the reconnect case — consuming the record
    /// so one disappearance excuses exactly one re-map: a client that
    /// crashes twice does not get a standing exemption.
    pub fn take_recent_vanish(&mut self, app_id: &str, program: Option<&str>) -> bool {
        let now = std::time::Instant::now();
        self.vanished_windows
            .retain(|(_, _, at)| now.duration_since(*at) < RECONNECT_FOCUS_GRACE);
        match self
            .vanished_windows
            .iter()
            .position(|(id, prog, _)| is_reconnect(id, prog.as_deref(), app_id, program))
        {
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
        let wm_layer = (*self.server).scene.layers.wm.raw();
        if wm_layer.is_null()
            || ffi::river_scene_node_get_parent((*window).popup_tree.node()) != wm_layer
        {
            return;
        }
        ffi::wlr_scene_node_raise_to_top((*window).popup_tree.node());
    }

    pub unsafe fn raise_window(&mut self, window: *mut Window) {
        if window.is_null() {
            return;
        }
        // A shy helper window stays where its app stacked it: beneath.
        if (*window).is_shy() {
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
        // Restacking changes who covers the adjust target.
        if self.window_adjust_active() {
            self.arm_border_fade();
        }
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

    /// Focus toward a free direction `v` (virtual units, y down): the
    /// three-finger focus swipe's path, which has a vector where a key has
    /// one of four directions (`policy::focus::vector_focus`). The nearest
    /// window center within `swipe_focus_cone_deg` of the ray from the
    /// focused window's center takes focus; with none, focus stays. With
    /// no focused window in the focus ring there is no ray, so `fallback`
    /// (the four-way action the swipe was bound to) runs instead, for its
    /// entry rule.
    pub unsafe fn focus_toward(&mut self, v: (f64, f64), fallback: &crate::config::Action) {
        use crate::policy::api::Compositor;
        self.stop_panning_animation();
        let ctx = self.build_action_ctx();
        let has_ray = ctx.windows.iter().any(|w| w.focus_cyclable && Some(w.id) == ctx.focused);
        if !has_ray {
            self.execute_action(fallback, None);
            return;
        }
        let cmds = crate::policy::actions::focus_toward(&ctx, v, self.swipe_focus_cone_deg);
        if cmds.is_empty() {
            log::info!("focus_toward {:?}: no window within {}°", v, self.swipe_focus_cone_deg);
        }
        for cmd in &cmds {
            self.apply(cmd);
        }
    }

    /// Whether a focus swipe's step would move focus — `focus_toward(v,
    /// action)` with a vector, `execute_action(action)` without one — asked
    /// of the policy without applying anything or touching the camera. A
    /// directional focus with nowhere to go is an empty command list on
    /// both paths, and the legacy arms do nothing with it either.
    pub unsafe fn focus_toward_lands(&mut self, v: Option<(f64, f64)>, action: &crate::config::Action) -> bool {
        use crate::policy::api::Policy;
        let ctx = self.build_action_ctx();
        let has_ray = ctx.windows.iter().any(|w| w.focus_cyclable && Some(w.id) == ctx.focused);
        let cmds = match v {
            Some(v) if has_ray => crate::policy::actions::focus_toward(&ctx, v, self.swipe_focus_cone_deg),
            _ => crate::policy::actions::DefaultPolicy.action(&ctx, *action, None),
        };
        !cmds.is_empty()
    }

    /// A click on one of a window's buttons — the discs left of its
    /// top-right handle (`window::window_takes_buttons`). Minimize and
    /// maximize run the keyboard's own actions, which act on the focused
    /// window, so the window is focused first: a click is a choice of
    /// window. Maximize is the fullscreen toggle — a Tiled window already
    /// reports xdg maximized here, so the step up from it is fullscreen —
    /// and the toggle flips Floating and Tiled exactly as `set-mode` does.
    pub unsafe fn press_window_button(&mut self, window: *mut Window, elem: crate::window::BorderElement) {
        use crate::policy::api::{Command, Compositor, WindowId};
        use crate::window::BorderElement;
        if !crate::window::window_takes_buttons(window) {
            return;
        }
        match elem {
            BorderElement::Minimize | BorderElement::Maximize => {
                if let Some(seat) = self.first_seat() {
                    (*seat).focus(crate::seat::Focus::Window(window));
                }
                let action = if elem == BorderElement::Minimize {
                    crate::config::Action::Minimize
                } else {
                    crate::config::Action::Fullscreen
                };
                self.execute_action(&action, None);
            }
            BorderElement::ToggleTile => {
                let mode = if (*window).tiling_mode == crate::tiling::TilingMode::Tiled {
                    crate::tiling::TilingMode::Floating
                } else {
                    crate::tiling::TilingMode::Tiled
                };
                let id = WindowId((*window).ref_key);
                self.apply(&Command::SetWindowMode { id, mode, locked: true });
                self.apply(&Command::Relayout);
            }
            _ => {}
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
            (*wm).pending_ipc_peer_pid = req.peer_pid;
            // A panic here would unwind out of this extern "C" callback,
            // which aborts the process — the whole desktop — over one bad
            // command. Contain it to the command: the caller gets an error.
            let reply = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (*wm).process_ipc_command(&req.command)
            })) {
                Ok(reply) => reply,
                Err(_) => {
                    log::error!("[ipc] command panicked and was abandoned: {:?}", req.command);
                    "error: the compositor failed running that command\n".to_string()
                }
            };
            (*wm).pending_ipc_peer_pid = 0;
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

/// A fullscreen window that owns the top of the stack: rendered fullscreen,
/// not stepped aside for another focused window (`fullscreen_yields`), and
/// not in overview — where it is a slab on the desk like a stepped-aside
/// one (`place_fullscreen_windows`) and stacks behind every window with
/// it, so it never hides the windows overview is there to show.
unsafe fn fullscreen_on_top(window: *mut Window) -> bool {
    rendered_fullscreen(window)
        && (*(*window).server).wm.mode != WindowManagerMode::Overview
        && !(*window).fullscreen_yields()
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
        // Manage and Render finish synchronously in the built-in policy,
        // so only an in-flight configure ever waits on this timer.
        WindowManagerState::Manage | WindowManagerState::Render | WindowManagerState::Idle => {}
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

// The global is kept for one request: clients (cce-ui, cce-cloud) bind it to
// call `get_cce_toplevel`. The river-style management requests it once
// carried (manage/render handshakes, shell surfaces, exit_session) were for an
// external window-manager client, and the built-in policy replaced that; they
// are unimplemented here, so a client sending one is a protocol error rather
// than, say, a way for any client to end the session.
unsafe extern "C" fn wm_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

static WM_INTERFACE: ffi::zcce_window_manager_v1_interface = ffi::zcce_window_manager_v1_interface {
    stop: None,
    destroy: Some(wm_destroy),
    manage_finish: None,
    manage_dirty: None,
    render_finish: None,
    get_shell_surface: None,
    exit_session: None,
    get_cce_toplevel: Some(crate::cce_window_management::cce_wm_get_cce_toplevel),
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

    ffi::wl_resource_set_implementation(resource, &WM_INTERFACE as *const _ as *const _, wm as *mut _, None);
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
    pub rect: crate::scene_handle::SceneRect,
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

/// The `focus-window --wait` poll (`WindowManager::poll_settle_waiters`).
unsafe extern "C" fn handle_settle_tick(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let wm = data as *mut WindowManager;
    if !wm.is_null() {
        (*wm).poll_settle_waiters();
    }
    0
}

/// Steps every window's border hover fade, map/close dissolve, and any
/// in-flight fullscreen-toggle animation — until all of them have settled. Windows at
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
        if (*window).step_adjust_dim() {
            moving = true;
        }
        if (*window).step_map_fade() {
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

    fn rule(app_id: &str, title: Option<&str>) -> crate::config::ModeRule {
        crate::config::ModeRule {
            mode: crate::tiling::TilingMode::Floating,
            app_id_pattern: app_id.to_string(),
            title_pattern: title.map(str::to_string),
            single_instance: false,
            tag: -1,
            circular: false,
            ssd: None,
            over_sibling: false,
            center: true,
        }
    }

    #[test]
    fn a_window_settles_once_its_box_holds_still_with_nothing_in_flight() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let key = crate::slotmap::Key { generation: 0, index: 0 };
        let mut w = SettleWaiter::new(tx, key, 0);
        // A pan in flight: the box moves, never settled.
        for x in [100, 95, 62, 57, 56] {
            assert!(!w.observe((x, 24, 1200, 720), true));
        }
        // The camera has landed but the box has not been seen still yet.
        assert!(!w.observe((56, 24, 1200, 720), false));
        assert!(!w.observe((56, 24, 1200, 720), false));
        // A step of some other animation restarts the count.
        assert!(!w.observe((57, 24, 1200, 720), false));
        assert!(!w.observe((57, 24, 1200, 720), false));
        assert!(!w.observe((57, 24, 1200, 720), false));
        assert!(w.observe((57, 24, 1200, 720), false));
    }

    #[test]
    fn a_transaction_awaiting_acks_counts_as_layout_in_flight() {
        // The sequence has cleared `dirty`, but the box has not moved yet.
        assert!(layout_in_flight(WindowManagerState::InflightConfigures(1), false));
        assert!(layout_in_flight(WindowManagerState::Manage, false));
        assert!(layout_in_flight(WindowManagerState::Render, false));
        assert!(layout_in_flight(WindowManagerState::Idle, true));
        assert!(!layout_in_flight(WindowManagerState::Idle, false));
    }

    #[test]
    fn an_empty_title_rule_reaches_an_untitled_window() {
        let any = rule("com.anthropic.Claude", Some(""));
        assert!(mode_rule_matches(&any, Some("com.anthropic.Claude"), None), "never set a title");
        assert!(mode_rule_matches(&any, Some("com.anthropic.Claude"), Some("")));
        assert!(mode_rule_matches(&any, Some("com.anthropic.Claude"), Some("Claude")), "substring: every title");
        let named = rule("com.anthropic.Claude", Some("Claude"));
        assert!(!mode_rule_matches(&named, Some("com.anthropic.Claude"), None), "a named title needs a title");
        assert!(mode_rule_matches(&named, Some("com.anthropic.Claude"), Some("Claude")));
        assert!(!mode_rule_matches(&named, Some("other"), Some("Claude")));
        assert!(mode_rule_matches(&rule("*", None), None, None));
    }

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
            argv: None,
            fullscreen_at: None,
        });

        unsafe {
            // Test exact match
            let matched = wm.match_last_window_state("test-app", "My App Window", None);
            assert!(matched.is_some());
            let m = matched.unwrap();
            assert_eq!(m.app_id, "test-app");
            assert_eq!(m.virtual_x, 100.0);
            assert_eq!(m.virtual_y, 200.0);

            // Test fuzzy title match
            let matched_fuzzy = wm.match_last_window_state("test-app", "My App Window*", None);
            assert!(matched_fuzzy.is_some());

            // Test app_id only match
            let matched_appid = wm.match_last_window_state("test-app", "Different Title", None);
            assert!(matched_appid.is_some());
            assert_eq!(matched_appid.unwrap().width, 800);

            // Test no match
            let no_match = wm.match_last_window_state("other-app", "My App Window", None);
            assert!(no_match.is_none());
        }

        std::mem::forget(wm);
    }

    fn proton_entry(title: &str, cmdline: &str) -> SavedWindowState {
        SavedWindowState {
            app_id: "steam_proton".to_string(),
            title: title.to_string(),
            tiling_mode: crate::tiling::TilingMode::Floating,
            minimized: false,
            virtual_x: -5002.0,
            virtual_y: -1517.0,
            scale: 1.0,
            width: 1214,
            height: 689,
            cmdline: cmdline.to_string(),
            focused: false,
            argv: None,
            fullscreen_at: None,
        }
    }

    fn entry_with(cmdline: &str, argv: Option<&[&str]>) -> SavedWindowState {
        SavedWindowState {
            app_id: "viewer".to_string(),
            cmdline: cmdline.to_string(),
            argv: argv.map(|a| a.iter().map(|s| s.to_string()).collect()),
            ..proton_entry("t", "")
        }
    }

    /// What `sh -c` makes of a command: the argv it would exec.
    fn shell_argv(cmd: &str) -> Vec<String> {
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", &format!("printf '%s\\0' {cmd}")])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().split('\0').filter(|s| !s.is_empty()).map(String::from).collect()
    }

    #[test]
    fn control_socket_numbers_must_be_finite() {
        assert_eq!(parse_finite("1.5"), Ok(1.5));
        assert_eq!(parse_finite("-20"), Ok(-20.0));
        for bad in ["NaN", "nan", "inf", "-inf", "infinity", "1e999", "x", ""] {
            assert!(parse_finite(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_restore_relaunches_the_saved_argv_exactly() {
        // Every argument comes back from the shell as it went in: the
        // substitution, the `&`, the quote and the space are text.
        let argv = ["zathura", "/home/u/Downloads/x$(touch /tmp/pwned).pdf", "a&b", "it's", "two words", "--working-directory=/tmp/a b"];
        let cmd = restore_command(&entry_with("ignored", Some(&argv))).unwrap();
        assert_eq!(shell_argv(&cmd), argv);
        // Plain arguments stay readable in the log.
        assert_eq!(shell_quote("/usr/bin/foot"), "/usr/bin/foot");
        assert_eq!(shell_quote("--app-id=cce-terminal"), "--app-id=cce-terminal");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn a_legacy_entry_relaunches_only_when_its_cmdline_is_plain() {
        // Saved before argv: plain words still relaunch as before...
        assert_eq!(
            restore_command(&entry_with("cce-terminal --app-id=cce-terminal", None)).as_deref(),
            Some("cce-terminal --app-id=cce-terminal")
        );
        // ...but anything the shell would act on is not run at all.
        for bad in ["zathura x$(id).pdf", "chromium https://a/?x=1&y=2", "foot --working-directory='/tmp'", "a;b", "a`id`"] {
            assert!(restore_command(&entry_with(bad, None)).is_none(), "{bad:?} must not be relaunched");
        }
        // Wine's Windows paths never relaunch, argv or not.
        assert!(restore_command(&entry_with(r"C:\x.exe", Some(&[r"C:\x.exe"]))).is_none());
    }

    #[test]
    fn a_rewritten_cmdline_relaunches_as_its_words() {
        // Chromium/Electron leave one space-joined string in /proc cmdline;
        // it must come back as three words, not one missing program name.
        let joined = "/usr/lib/claude-desktop/claude-desktop --ozone-platform=wayland --password-store=gnome-libsecret";
        let cmd = restore_command(&entry_with(joined, Some(&[joined]))).unwrap();
        assert_eq!(
            shell_argv(&cmd),
            ["/usr/lib/claude-desktop/claude-desktop", "--ozone-platform=wayland", "--password-store=gnome-libsecret"]
        );
        // Shell characters in a joined line are still never run.
        let url = "/opt/google/chrome/chrome https://a/?x=1&y=2";
        assert!(restore_command(&entry_with(url, Some(&[url]))).is_none());
        // A real program whose path holds a space stays one quoted word.
        let dir = std::env::temp_dir().join(format!("cce restore test {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let prog = dir.join("my app");
        std::fs::write(&prog, "").unwrap();
        let prog = prog.to_str().unwrap();
        let cmd = restore_command(&entry_with(prog, Some(&[prog]))).unwrap();
        assert_eq!(shell_argv(&cmd), [prog]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    const UPC: &str = r"C:\Program Files (x86)\Ubisoft\Ubisoft Game Launcher\upc.exe";
    const EXPLORER: &str = r"C:\windows\system32\explorer.exe";

    /// Trackmania and the Ubisoft Connect it launches from are both
    /// `steam_proton`; each keeps its own `last_window_states` slot, while
    /// an entry or window with no program still shares the app_id's one.
    #[test]
    fn last_state_slots_are_per_program() {
        const TM: &str = "C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Trackmania/Trackmania.exe";
        let upc = proton_entry("Ubisoft Connect", &format!("{UPC} -upc_desktop_mode"));
        let tm = proton_entry("Trackmania", &format!("{TM}      "));
        assert!(last_state_slot(&upc, "steam_proton", Some(UPC)));
        assert!(!last_state_slot(&upc, "steam_proton", Some(TM)));
        assert!(last_state_slot(&tm, "steam_proton", Some(TM)));
        assert!(!last_state_slot(&tm, "steam_proton", Some(UPC)));
        assert!(!last_state_slot(&tm, "other", Some(TM)));
        assert!(last_state_slot(&tm, "steam_proton", None));
        assert!(last_state_slot(&proton_entry("x", "steam_proton"), "steam_proton", Some(TM)));
    }

    #[test]
    fn same_program_compares_argv0_by_prefix() {
        let saved = proton_entry("Ubisoft Connect", &format!("{UPC} -upc_desktop_mode --disable-gpu"));
        assert!(same_program(&saved, Some(UPC)));
        assert!(!same_program(&saved, Some(EXPLORER)));
        // A prefix of argv[0] is not argv[0].
        assert!(!same_program(&saved, Some(UPC.trim_end_matches(".exe"))));
        // Unknown on either side never vetoes.
        assert!(same_program(&saved, None));
        assert!(same_program(&saved, Some("")));
        assert!(same_program(&proton_entry("x", "steam_proton"), Some(EXPLORER)));
        assert!(same_program(&proton_entry("x", ""), Some(EXPLORER)));
        // A bare argv with no arguments.
        assert!(same_program(&proton_entry("x", UPC), Some(UPC)));
    }

    /// The scrub of a shy window's own entries deletes only on positive
    /// evidence: an entry `save_state` could label with just the app_id
    /// names no program, so it is kept.
    #[test]
    fn saved_by_program_needs_a_recorded_cmdline() {
        let tray = proton_entry("", &format!("{EXPLORER} /desktop      "));
        assert!(saved_by_program(&tray, EXPLORER));
        assert!(!saved_by_program(&tray, UPC));
        assert!(!saved_by_program(&tray, ""));
        assert!(!saved_by_program(&proton_entry("", "steam_proton"), EXPLORER));
    }

    #[test]
    fn windows_paths_and_empty_commands_are_not_relaunched() {
        assert!(relaunchable("/usr/bin/cce-files"));
        assert!(relaunchable("cce-terminal --working-directory='/tmp'"));
        assert!(!relaunchable(&format!("{UPC} -upc_desktop_mode")));
        assert!(!relaunchable("D:/Games/thing.exe"));
        assert!(!relaunchable("   "));
        assert!(!relaunchable(""));
    }

    #[test]
    fn wine_argv0_is_a_windows_path() {
        assert!(is_windows_path(UPC));
        assert!(is_windows_path("D:/Games/thing.exe"));
        assert!(!is_windows_path("/usr/bin/wine"));
        assert!(!is_windows_path("C:"));
        assert!(!is_windows_path(""));
    }

    #[test]
    fn a_reconnect_is_the_same_program_under_the_same_app_id() {
        let upc = Some(UPC);
        let game = Some(r"C:\Program Files\Ubisoft\Trackmania\Trackmania.exe");
        // The launcher's own window coming back is a reconnect...
        assert!(is_reconnect("steam_proton", upc, "steam_proton", upc));
        // ...a game it started, sharing only the app_id, is not.
        assert!(!is_reconnect("steam_proton", upc, "steam_proton", game));
        // Unknown program on either side: the app_id decides, as before.
        assert!(is_reconnect("steam_proton", None, "steam_proton", game));
        assert!(is_reconnect("cce-files", Some("/usr/bin/cce-files"), "cce-files", None));
        assert!(!is_reconnect("cce-files", None, "cce-mail", None));
    }

    #[test]
    fn empty_titles_match_nothing_exactly() {
        assert!(titles_match("Ubisoft Connect", "Ubisoft Connect"));
        assert!(!titles_match("", ""));
        assert!(!titles_match("Ubisoft Connect", "Ubisoft"));
    }

    #[test]
    fn empty_titles_resemble_nothing() {
        assert!(titles_resemble("Doc.txt*", "Doc.txt"));
        assert!(titles_resemble("Ubisoft Connect", "Ubisoft"));
        assert!(!titles_resemble("Ubisoft Connect", ""));
        assert!(!titles_resemble("", "Ubisoft Connect"));
        assert!(!titles_resemble("*", ""));
    }

    /// Wine's fallback tray window is untitled, owned by the prefix's
    /// explorer.exe, and shares `steam_proton` with every Proton app; it
    /// must not borrow the launcher's saved geometry. Nor may the launcher
    /// borrow the tray's, once the tray has been saved.
    #[test]
    #[allow(invalid_value)]
    fn app_id_only_match_requires_same_program() {
        let mut wm = unsafe { std::mem::MaybeUninit::<WindowManager>::zeroed().assume_init() };
        unsafe {
            std::ptr::write(&mut wm.last_window_states, Vec::new());
        }
        wm.last_window_states
            .push(proton_entry("Ubisoft Connect", &format!("{UPC} -upc_desktop_mode")));
        unsafe {
            assert!(wm.match_last_window_state("steam_proton", "", Some(EXPLORER)).is_none());
            // The launcher itself, under a changed title, still borrows.
            assert!(wm.match_last_window_state("steam_proton", "Library", Some(UPC)).is_some());
            // And with its pid unknown, as before.
            assert!(wm.match_last_window_state("steam_proton", "", None).is_some());
        }
        wm.last_window_states[0] = proton_entry("", &format!("{EXPLORER} /desktop"));
        unsafe {
            // Untitled on both sides is not an exact match, so the program
            // check applies to it too.
            assert!(wm.match_last_window_state("steam_proton", "", Some(UPC)).is_none());
            assert!(wm.match_last_window_state("steam_proton", "Ubisoft Connect", Some(UPC)).is_none());
            // Nor the tray itself: an untitled entry is never borrowed.
            assert!(wm.match_last_window_state("steam_proton", "", Some(EXPLORER)).is_none());
            assert!(wm.match_last_window_state("steam_proton", "", None).is_none());
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
