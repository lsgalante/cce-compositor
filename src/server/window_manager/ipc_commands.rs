//! The control socket's dispatcher: `process_ipc_command`, one arm per command
//! `ccectl` and the clients send, and the helpers only it uses. The cross-crate
//! request lines are cce-core's `ipc::ctl`. Split out of window_manager.rs on
//! 2026-10-10.

use super::*;

impl WindowManager {
    pub unsafe fn process_ipc_command(&mut self, cmd: &str) -> String {
        crate::wm_scope!(mut);
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
            // "I am closing — dissolve me out, and tell me how long that
            // takes." The client keeps its surface alive for the reply's
            // worth of milliseconds and then exits; the compositor ramps the
            // scene node's opacity down in the meantime, which takes the
            // backdrop blur and the window's shadow and bevel with it. A
            // client fading its OWN pixels cannot do that — its surface stays
            // fully present to the compositor, so the blur behind it hangs at
            // full strength over a dissolving window (what cce-cloud's
            // hand-rolled close fade looked like).
            //
            // The target is the CALLER, resolved through SO_PEERCRED rather
            // than a name in the command: the kernel vouches for the pid, and
            // a client always knows its own even when it has no app_id. The
            // reply is the duration in ms, always — a client that gets "0"
            // simply exits at once, which is what a disabled fade means.
            "fade-out" => {
                // Animations off answers 0 like a disabled fade: the client
                // exits at once and nothing ramps.
                let ms = if cce_core::motion::enabled() { crate::shared::layout().fade_out_ms } else { 0 };
                let pid = self.pending_ipc_peer_pid;
                if pid <= 0 {
                    return "0\n".to_string();
                }
                let mut faded = false;
                let windows: Vec<*mut crate::window::Window> =
                    self.windows.iter().copied().collect();
                for window in windows {
                    if window.is_null() || (*window).closed {
                        continue;
                    }
                    if (*window).unreliable_pid() == pid && (*window).wants_map_fade() {
                        (*window).start_map_fade(0.0, ms);
                        faded = true;
                    }
                }
                // The same client may own layer surfaces instead of (or as
                // well as) windows — cce-cloud's launcher is one.
                let surfaces: Vec<*mut crate::layer_shell::LayerSurface> =
                    (*self.server).layer_shell.surfaces.iter().copied().collect();
                for surface in surfaces {
                    if surface.is_null() {
                        continue;
                    }
                    if (*surface).client_pid() == pid {
                        (*surface).start_fade(0.0, ms);
                        faded = true;
                    }
                }
                if !faded {
                    // Nothing of the caller's is on screen — no reason to
                    // make it wait.
                    return "0\n".to_string();
                }
                return format!("{}\n", ms);
            }
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
                let scene = crate::shared::scene();
                let mut out = String::new();
                let layers: [(&str, *mut ffi::wlr_scene_tree); 12] = [
                    ("background", scene.layers.background.raw()),
                    ("bottom", scene.layers.bottom.raw()),
                    ("wm", scene.layers.wm.raw()),
                    ("top", scene.layers.top.raw()),
                    ("fullscreen", scene.layers.fullscreen.raw()),
                    ("overlay", scene.layers.overlay.raw()),
                    ("popups", scene.layers.popups.raw()),
                    ("override_redirect", scene.layers.override_redirect.raw()),
                    ("border_overlay", scene.layers.border_overlay.raw()),
                    ("drag_icons", scene.drag_icons.raw()),
                    ("hidden", scene.hidden_tree.raw()),
                    ("locked", scene.locked_tree.raw()),
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
                    (*win).tree.is_enabled(),
                    (*win).surfaces.tree.is_enabled(),
                    (*win).surfaces.saved_tree.is_enabled(),
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
                    ("surfaces", (*win).surfaces.tree.node()),
                    ("saved", (*win).surfaces.saved_tree.node()),
                    ("popup", (*win).popup_tree.node()),
                    ("whole-tree", (*win).tree.node()),
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
            // Re-entrancy tally (`reentry.rs`): where code outside the window
            // manager reached it while one of its methods was running.
            // `debug-reentry reset` clears it.
            "debug-reentry" => {
                if parts.get(1) == Some(&"reset") {
                    crate::reentry::reset();
                    return "ok\n".to_string();
                }
                return crate::reentry::report();
            }
            "debug-windows" => {
                let mut out = format!(
                    "wm state={:?} dirty={} rendering_dirty={} dirty_idle_armed={}\n",
                    self.state,
                    crate::shared::pending().windowing(),
                    crate::shared::pending().rendering(),
                    crate::shared::pending().armed(),
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
                crate::shared::pending().dirty_windowing();
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
                // The state reaches the bar on the status socket's `adjust`
                // topic. A fixed-name flag file in shared /tmp was also
                // written here until 2026-10-02; nothing read it.
                self.adjust_position_mode = enable;
                crate::shared::pending().dirty_windowing();
                return format!("ok {}\n", enable);
            }
            // The desktop grid reporting its images: `grid-items
            // <id>:<x>:<y>:<w>:<h> ...` in virtual units, the whole list on
            // every change (none at all clears it). See `crate::selection`.
            "grid-items" => {
                let items = crate::selection::parse_desktop_items(&parts[1..]);
                let n = items.len();
                self.set_desktop_items(items);
                return format!("ok {}\n", n);
            }
            // The overview drag-selection: the selected window ids and the
            // selected desktop images' ids, each on one line (`-` for
            // none), and the rubber band's virtual rect while one is being
            // dragged out.
            "selection" => {
                let ids: Vec<String> = self
                    .selection
                    .windows
                    .iter()
                    .map(|&w| (*w).ref_key.index.to_string())
                    .collect();
                let items: Vec<String> =
                    self.selection.items.iter().map(|id| id.to_string()).collect();
                // Every image the grid has reported, so a shadow can see
                // the report landed and where a group move left them.
                let desk: Vec<String> = self
                    .selection
                    .desktop_items
                    .iter()
                    .map(|i| format!("{}@{:.0},{:.0},{:.0}x{:.0}", i.id, i.x, i.y, i.w, i.h))
                    .collect();
                let band = match self.selection.marquee {
                    Some(m) => {
                        let (x, y, w, h) = m.rect();
                        format!("{:.0},{:.0},{:.0}x{:.0}", x, y, w, h)
                    }
                    None => "-".to_string(),
                };
                format!(
                    "selected={} items={} band={} desk={}\n",
                    if ids.is_empty() { "-".to_string() } else { ids.join(",") },
                    if items.is_empty() { "-".to_string() } else { items.join(",") },
                    band,
                    if desk.is_empty() { "-".to_string() } else { desk.join(";") },
                )
            }
            // The camera as it stands and where it is easing to — what a
            // shadow reads back to assert a swipe's peek and its return.
            "camera" => {
                let fmt = |v: Option<f64>| v.map_or("-".to_string(), |v| format!("{:.1}", v));
                format!(
                    "pan_x={:.1} pan_y={:.1} zoom={:.3} target_x={} target_y={} anim={} mode={:?}\n",
                    self.desk_pan_x + self.pan_pending[0],
                    self.desk_pan_y + self.pan_pending[1],
                    self.desk_zoom,
                    fmt(self.target_desk_pan_x),
                    fmt(self.target_desk_pan_y),
                    self.camera_anim_active,
                    crate::shared::mode()
                )
            }
            "pan-by" => {
                if parts.len() < 3 { return "error: missing dx or dy\n".to_string(); }
                if let (Ok(dx), Ok(dy)) = (parse_finite(parts[1]), parse_finite(parts[2])) {
                    self.desk_pan_x += dx;
                    self.desk_pan_y += dy;
                    if matches!(self.state, WindowManagerState::Idle) {
                        self.update_viewport_local();
                    } else {
                        crate::shared::pending().dirty_windowing();
                    }
                    return "ok\n".to_string();
                }
                "error: invalid dx or dy\n".to_string()
            }
            "pan-to" => {
                if parts.len() < 3 { return "error: missing x or y\n".to_string(); }
                if let (Ok(x), Ok(y)) = (parse_finite(parts[1]), parse_finite(parts[2])) {
                    self.desk_pan_x = x;
                    self.desk_pan_y = y;
                    if matches!(self.state, WindowManagerState::Idle) {
                        self.update_viewport_local();
                    } else {
                        crate::shared::pending().dirty_windowing();
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
                if let Ok(factor) = parse_finite(parts[1]) {
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
                    crate::shared::pending().dirty_windowing();
                    return "ok\n".to_string();
                }
                "error: invalid zoom factor\n".to_string()
            }
            "set-coords" => {
                if parts.len() < 3 { return "error: missing x or y\n".to_string(); }
                if let (Ok(x), Ok(y)) = (parse_finite(parts[1]), parse_finite(parts[2])) {
                    if let Some(seat) = self.first_seat() {
                        if let crate::seat::Focus::Window(fw) = (*seat).focused {
                            (*fw).virtual_x = x;
                            (*fw).virtual_y = y;
                            crate::shared::pending().dirty_windowing();
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
                if let (Ok(x), Ok(y)) = (parse_finite(parts[2]), parse_finite(parts[3])) {
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
                        crate::shared::pending().dirty_windowing();
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
                    return format!("{:?}\n", crate::shared::mode()).to_lowercase();
                }
                let target = parts[1].to_lowercase();
                // Same pointer-less halves as the "overview" command above.
                if target == "normal" {
                    if crate::shared::mode() == WindowManagerMode::Overview {
                        self.execute_action(&crate::config::Action::OverviewExit, None);
                    }
                    return "ok\n".to_string();
                } else if target == "overview" {
                    if crate::shared::mode() == WindowManagerMode::Normal {
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
                // `--wait` holds the reply until the window has stopped
                // moving on screen (`SettleWaiter`) — ccectl sends it. Apps
                // forwarding a launch (`cce_core::ipc::focus_window`) do not:
                // they wait a second at most and need no settling.
                let Some(cce_core::ipc::ctl::Request::FocusWindow { query, wait }) = cce_core::ipc::ctl::Request::parse(cmd) else {
                    return "error: missing app_id/id\n".to_string();
                };
                if let Some(seat) = self.first_seat() {
                    let best_target = self.find_window_by_query(&query);
                    if !best_target.is_null() {
                        if (*best_target).minimized {
                            (*best_target).minimized = false;
                        }
                        (*seat).focus(crate::seat::Focus::Window(best_target));
                        self.raise_window(best_target);
                        crate::shared::pending().dirty_windowing();
                        if wait {
                            if let Some(tx) = self.pending_ipc_reply.take() {
                                let started = crate::util::timestamp_ns();
                                self.settle_waiters.push(SettleWaiter::new(tx, (*best_target).ref_key, started));
                                self.arm_settle_timer();
                                return String::new();
                            }
                        }
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
                let sp = crate::shared::layout().snap_params();
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
                crate::shared::pending().dirty_windowing();

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
            "resize-window" => {
                // resize-window <w> <h> [app_id|id] — set a window's content
                // size in logical px, top-left fixed; the focused window with
                // no query. Clamped to the client's min/max hints like a drag
                // (`DimensionsHint::clamp`). A Tiled window then covers the
                // cells that size touches, as a client maximize does. The
                // reply is the size asked for: the client answers in its own
                // time (an X11 app may insist on its minimum), so read the
                // outcome back with `windows`.
                let usage = "error: usage: resize-window <w> <h> [app_id|id]\n";
                let (Some(w), Some(h)) = (
                    parts.get(1).and_then(|s| s.parse::<u32>().ok()).filter(|&v| v > 0),
                    parts.get(2).and_then(|s| s.parse::<u32>().ok()).filter(|&v| v > 0),
                ) else {
                    return usage.to_string();
                };
                let target: *mut Window = if parts.len() >= 4 {
                    self.find_window_by_query(&parts[3..].join(" "))
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
                if matches!(
                    (*target).tiling_mode,
                    crate::tiling::TilingMode::Fullscreen
                        | crate::tiling::TilingMode::Utility
                        | crate::tiling::TilingMode::Status
                ) {
                    return "error: a fullscreen, utility or status window sizes itself\n".to_string();
                }
                let (w, h) = (*target).wm_scheduled.dimensions_hint.clamp(w, h);
                (*target).box_geom.width = w as i32;
                (*target).box_geom.height = h as i32;
                crate::shared::pending().dirty_windowing();
                format!("ok w={} h={}\n", w, h)
            }
            "min-size" => {
                // The minimum sizes X11 apps revealed by refusing a smaller
                // configure (`min_sizes`). `list` numbers the stored entries;
                // `forget <app_id|id>` drops an open window's entry and the
                // minimum it is held to now; `forget-entry <n>` drops one by
                // its `list` number, for an app that is not running. A
                // forgotten minimum is learned again on the next drag the
                // app refuses.
                match parts.get(1).copied().unwrap_or("list") {
                    "list" => {
                        let mut out = String::new();
                        for (i, e) in self.min_sizes.entries().iter().enumerate() {
                            out.push_str(&format!(
                                "{} {}x{} app_id={} title={:?} program={:?}\n",
                                i, e.width, e.height, e.app_id, e.title, e.program
                            ));
                        }
                        if out.is_empty() { "none\n".to_string() } else { out }
                    }
                    "forget" if parts.len() >= 3 => {
                        let target = self.find_window_by_query(&parts[2..].join(" "));
                        if target.is_null() {
                            return "error: window not found\n".to_string();
                        }
                        let crate::window::WindowImpl::Xwayland(xw) = (*target).impl_type else {
                            return "error: not an X11 window; only those have a learned minimum\n".to_string();
                        };
                        if xw.is_null() {
                            return "error: window not found\n".to_string();
                        }
                        match (*xw).forget_min_size() {
                            Some((w, h)) => format!("ok forgot {}x{}\n", w, h),
                            None => "ok none stored\n".to_string(),
                        }
                    }
                    "forget-entry" if parts.len() == 3 => {
                        let Some(e) = parts[2]
                            .parse::<usize>()
                            .ok()
                            .and_then(|i| self.min_sizes.entries().get(i).cloned())
                        else {
                            return format!("error: no entry '{}' (see min-size list)\n", parts[2]);
                        };
                        self.min_sizes.set(&e.app_id, &e.program, &e.title, 0, 0);
                        format!("ok forgot {}x{} app_id={} title={:?}\n", e.width, e.height, e.app_id, e.title)
                    }
                    _ => "error: usage: min-size [list] | forget <app_id|id> | forget-entry <n>\n".to_string(),
                }
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
                crate::shared::pending().dirty_windowing();

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
                // Never from a locked session: the display manager relaunches
                // the session greeter-free, so a restart sent while locked
                // (only a process of the user's can send one then) came back
                // unlocked. The user unlocks first.
                if unsafe { (*self.server).lock_manager.state } != crate::lock_manager::LockState::Unlocked {
                    return "error: the session is locked; unlock before restarting the compositor\n".to_string();
                }
                // Leave the restart flag for cce-display-manager's daemon (it
                // checks after the session worker exits, verifies the file is
                // owned by the session user, and relaunches this same session
                // greeter-free), then exit cleanly — which saves window state,
                // so the restored compositor brings the session back.
                let user = std::env::var("USER")
                    .unwrap_or_else(|_| format!("uid{}", unsafe { libc::getuid() }));
                let flag = cce_core::ipc::ctl::restart_flag(&user);
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
                crate::shared::pending().dirty_windowing();
                "ok\n".to_string()
            }
            "idle" => unsafe { (*self.server).idle.ipc(&parts[1..]) },
            // Lock the session (`LockManager::lock_now`). The reply waits for
            // the lock to complete, which is what the lock-before-sleep
            // thread (`sleep_lock`) holds logind's sleep for: `ok locked`
            // once every output shows the locked scene.
            "lock" => unsafe {
                let lock = &mut (*self.server).lock_manager;
                lock.lock_now();
                match self.pending_ipc_reply.take() {
                    Some(tx) => {
                        lock.reply_when_locked(tx);
                        String::new()
                    }
                    None => "ok\n".to_string(),
                }
            },
            // Live key repeat for every hardware keyboard, like `idle
            // timeouts`: it lasts until the next config load, which puts
            // `input { repeat_rate repeat_delay }` back.
            "repeat" => {
                if parts.len() == 3 {
                    let (Ok(rate), Ok(delay)) = (parts[1].parse::<u32>(), parts[2].parse::<u32>()) else {
                        return "error: rate and delay must be non-negative integers\n".to_string();
                    };
                    crate::shared::update_layout(|l| l.input_config.repeat_rate = Some(rate as i64));
                    crate::shared::update_layout(|l| l.input_config.repeat_delay = Some(delay as i64));
                } else if parts.len() != 1 {
                    return "error: usage: repeat [<rate> <delay>]\n".to_string();
                }
                let keyboards = unsafe { self.apply_key_repeat() };
                let (rate, delay) = crate::shared::layout().input_config.repeat_info();
                format!("rate={} delay={} keyboards={}\n", rate, delay, keyboards)
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

                // Render-list position, bottom (0) to top: the stacking
                // order the reorder pass applies within a layer, which a
                // window list keyed by slot id cannot show. -1 = not linked.
                let mut stack_of: std::collections::HashMap<*mut Window, i64> = std::collections::HashMap::new();
                {
                    let render_list = &mut self.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
                    let mut curr = (*render_list).next;
                    let mut i = 0i64;
                    while !curr.is_null() && curr != render_list {
                        let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
                        let win = (*node).window();
                        stack_of.insert(win, i);
                        i += 1;
                        curr = (*curr).next;
                    }
                }
                let mut out = String::new();
                let sp = crate::shared::layout().snap_params();
                for &w in self.windows.iter() {
                    if !w.is_null() && !(*w).closed && !matches!((*w).state, crate::window::WindowState::Closing | crate::window::WindowState::Init) {
                        let stack = stack_of.get(&w).copied().unwrap_or(-1);
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
                            // The line is cce-core's `ctl::WindowInfo`, which
                            // the status bar, its OSD and cce-remote parse.
                            let info = cce_core::ipc::ctl::WindowInfo {
                                id: (*w).ref_key.index as u64,
                                app_id: app_id.clone(),
                                title: title.clone(),
                                mode: (*w).tiling_mode.as_str().to_string(),
                                x: (*w).box_geom.x,
                                y: (*w).box_geom.y,
                                w: (*w).box_geom.width,
                                h: (*w).box_geom.height,
                                vx: (*w).virtual_x,
                                vy: (*w).virtual_y,
                                x11: match (*w).impl_type {
                                    crate::window::WindowImpl::Xwayland(xw) if !xw.is_null() => Some((*(*xw).xsurface).window_id),
                                    _ => None,
                                },
                                cell: cell.clone(),
                                minimized: (*w).minimized,
                                has_parent: (*w).has_parent,
                                focused: w == focused_window,
                                stack,
                                ssd: (*w).wm_requested.ssd,
                                // Why a window has (or lacks) rounded corners,
                                // blur and shadow. Without it the only way to
                                // tell is a full-output screenshot: a
                                // per-window capture reads the client's
                                // dmabuf, which is pre-composite and never
                                // shows the compositor's clip.
                                decorated: crate::shared::layout().is_decorated_app(&app_id),
                                beveled: crate::shared::layout().is_beveled_app(&app_id),
                            };
                            out.push_str(&info.to_json_line());
                            out.push('\n');
                        } else {
                            out.push_str(&format!(
                                "window id={} app_id={} title=\"{}\" mode={} x={} y={} w={} h={} vx={:.1} vy={:.1} cell={} minimized={} has_parent={} focused={} stack={} ssd={} decorated={} beveled={}\n",
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
                                stack,
                                (*w).wm_requested.ssd,
                                crate::shared::layout().is_decorated_app(&app_id),
                                crate::shared::layout().is_beveled_app(&app_id),
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
                let old_sp = crate::shared::layout().snap_params();
                match key {
                    "desktop_gap_color" => {
                        crate::shared::update_layout(|l| l.desktop_gap_color = val.to_string());
                        let parsed_color = crate::config::parse_hex_color(val);
                        crate::shared::update_layout(|l| l.background_r = ((parsed_color >> 16) & 0xFF) * 0x01010101);
                        crate::shared::update_layout(|l| l.background_g = ((parsed_color >> 8) & 0xFF) * 0x01010101);
                        crate::shared::update_layout(|l| l.background_b = (parsed_color & 0xFF) * 0x01010101);
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
                        crate::shared::update_layout(|l| l.desktop_cell_color = crate::config::parse_hex_color_rgba(val));
                    }
                    "desktop_grid_scale" | "grid_cell_size" => {
                        if let Ok(v) = parse_finite(val) {
                            crate::shared::update_layout(|l| l.desktop_cell_width = v);
                            crate::shared::update_layout(|l| l.desktop_cell_height = v);
                        }
                    }
                    "grid_cell_width" => {
                        if let Ok(v) = parse_finite(val) {
                            crate::shared::update_layout(|l| l.desktop_cell_width = v);
                        }
                    }
                    "grid_cell_height" => {
                        if let Ok(v) = parse_finite(val) {
                            crate::shared::update_layout(|l| l.desktop_cell_height = v);
                        }
                    }
                    "desktop_gap_width" => {
                        if let Ok(v) = val.parse::<i32>() {
                            crate::shared::update_layout(|l| l.desktop_gap_width = v);
                        }
                    }
                    "desktop_cell_fade_inset" => {
                        if let Ok(v) = val.parse::<i64>() {
                            crate::shared::update_layout(|l| l.desktop_cell_fade_inset = v);
                        }
                    }
                    "desktop_cell_labels" => {
                        if let Ok(v) = val.parse::<bool>() {
                            crate::shared::update_layout(|l| l.desktop_cell_labels = v);
                        }
                    }
                    "desktop_grid_fade_mode" => {
                        crate::shared::update_layout(|l| l.desktop_grid_fade_mode = val.to_string());
                    }
                    "gap" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.gap = v); } }
                    "gap_top" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.gap_top = v); } }
                    "gap_left" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.gap_left = v); } }
                    "gap_right" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.gap_right = v); } }
                    "gap_bottom" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.gap_bottom = v); } }
                    "offset" | "cascade_offset" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.cascade_offset = v); } }
                    "grid_gap" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.grid_gap = v); } }
                    "transition_duration" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.transition_duration = v); } }
                    "bar_height" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.bar_height = v); } }

                    "side_panel_width" | "pinned_width" | "overlay_width" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.overlay_width = v); } }
                    "side_panel_behavior" | "pinned_behavior" | "overlay_behavior" => { crate::shared::update_layout(|l| l.overlay_behavior = val.to_string()); }
                    "side_panel_position" | "pinned_position" | "overlay_position" => { crate::shared::update_layout(|l| l.overlay_position = val.to_string()); }
                    "side_panel_border_gap" | "pinned_border_gap" | "overlay_border_gap" => { if let Ok(v) = val.parse::<i32>() { crate::shared::update_layout(|l| l.overlay_border_gap = v); } }
                    _ => return format!("error: unknown layout key: {}\n", key),
                }
                self.retile_for_grid_change(old_sp);
                self.remap_saved_entries(&old_sp);
                if key.starts_with("desktop_") || key.starts_with("grid_cell") {
                    self.invalidate_grid_patches();
                }
                crate::shared::pending().dirty_windowing();
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
                    over_sibling: false,
                    center: false,
                });
                crate::shared::pending().dirty_windowing();
                "ok\n".to_string()
            }
            "input" => {
                if parts.len() < 4 { return "error: usage: input <device_name|*> scroll-factor <value>\n".to_string(); }
                let device_name = parts[1];
                let key = parts[2];
                let val = parts[3];
                if key == "scroll-factor" {
                    if let Ok(factor) = parse_finite(val) {
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
                if let (Ok(x), Ok(y)) = (parse_finite(parts[1]), parse_finite(parts[2])) {
                    self.for_each_cursor(|cursor| cursor.inject_motion_to(x, y));
                    "ok\n".to_string()
                } else {
                    "error: invalid x or y\n".to_string()
                }
            }
            // ── Synthetic touch (`ccectl touch …`), in layout pixels. Runs the
            // same `Cursor::touch_*` routing a touchscreen does; the first use
            // offers the touch capability as a touchscreen would, so clients
            // in a headless shadow bind `wl_touch` and the client route can be
            // exercised too.
            "touch" => {
                let usage = "error: usage: touch down <id> <x> <y> | motion <id> <x> <y> | up <id> | cancel <id> | tap <x> <y>\n";
                let id = |i: usize| parts.get(i).and_then(|v| v.parse::<i32>().ok());
                let num = |i: usize| parts.get(i).and_then(|v| parse_finite(v).ok());
                let stage = parts.get(1).copied().unwrap_or("");
                let time = crate::util::msec_timestamp();
                let ok = match stage {
                    "down" | "motion" => match (id(2), num(3), num(4)) {
                        (Some(id), Some(x), Some(y)) => {
                            self.for_each_seat_touch(|cursor| {
                                if stage == "down" {
                                    cursor.touch_down(id, x, y, time);
                                } else {
                                    cursor.touch_motion(id, x, y, time);
                                }
                                cursor.touch_frame();
                            });
                            true
                        }
                        _ => false,
                    },
                    "up" | "cancel" => match id(2) {
                        Some(id) => {
                            self.for_each_seat_touch(|cursor| {
                                if stage == "up" {
                                    cursor.touch_up(id, time);
                                } else {
                                    cursor.touch_cancel(id);
                                }
                                cursor.touch_frame();
                            });
                            true
                        }
                        None => false,
                    },
                    "tap" => match (num(2), num(3)) {
                        (Some(x), Some(y)) => {
                            self.for_each_seat_touch(|cursor| {
                                cursor.touch_down(0, x, y, time);
                                cursor.touch_frame();
                                cursor.touch_up(0, time);
                                cursor.touch_frame();
                            });
                            true
                        }
                        _ => false,
                    },
                    _ => false,
                };
                if ok { "ok\n".to_string() } else { usage.to_string() }
            }
            "pointer-move-by" => {
                if parts.len() < 3 { return "error: usage: pointer-move-by <dx> <dy>\n".to_string(); }
                if let (Ok(dx), Ok(dy)) = (parse_finite(parts[1]), parse_finite(parts[2])) {
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
                let dy = parse_finite(parts[1]);
                let dx = parts.get(2).filter(|p| **p != "finger" && **p != "natural").map(|v| parse_finite(v)).unwrap_or(Ok(0.0));
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
                // pointer-swipe begin <fingers> | update <dx> <dy> | end   (paced by the caller)
                let usage = "error: usage: pointer-swipe <fingers> <dx> <dy> [steps] | begin <fingers> | update <dx> <dy> | end\n";
                if parts.len() >= 2 && matches!(parts[1], "begin" | "update" | "end") {
                    let stage = parts[1].to_string();
                    let num = |i: usize| parts.get(i).and_then(|v| parse_finite(v).ok());
                    let (fingers, dx, dy) = match parts[1] {
                        "begin" => (num(2).map(|f| f as u32).unwrap_or(3), 0.0, 0.0),
                        "update" => match (num(2), num(3)) {
                            (Some(dx), Some(dy)) => (3, dx, dy),
                            _ => return usage.to_string(),
                        },
                        _ => (3, 0.0, 0.0),
                    };
                    // The begin fixes the finger count; updates reuse it.
                    self.for_each_cursor(|cursor| cursor.inject_swipe_stage(&stage, fingers, dx, dy));
                    return "ok\n".to_string();
                }
                if parts.len() < 4 { return usage.to_string(); }
                let fingers = parts[1].parse::<u32>();
                let dx = parse_finite(parts[2]);
                let dy = parse_finite(parts[3]);
                let steps = parts.get(4).map(|v| v.parse::<u32>()).unwrap_or(Ok(10)).map(|n| n.clamp(1, MAX_INJECTED_STEPS));
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
                    let scale = parts.get(2).and_then(|v| parse_finite(v).ok()).unwrap_or(1.0);
                    let rotation = parts.get(3).and_then(|v| parse_finite(v).ok()).unwrap_or(0.0);
                    let stage = parts[1].to_string();
                    self.for_each_cursor(|cursor| cursor.inject_pinch_stage(&stage, scale, rotation));
                    return "ok\n".to_string();
                }
                let scale = parse_finite(parts[1]);
                let rotation = parts.get(2).map(|v| parse_finite(v)).unwrap_or(Ok(0.0));
                let steps = parts.get(3).map(|v| v.parse::<u32>()).unwrap_or(Ok(10)).map(|n| n.clamp(1, MAX_INJECTED_STEPS));
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
                // The grammar is cce-core's (`ctl::Request::PlaceNext`),
                // shared with the widgets that send it; `cell` is false here.
                let Some(cce_core::ipc::ctl::Request::PlaceNext { app_id, x, y, cell }) = cce_core::ipc::ctl::Request::parse(cmd) else {
                    return "error: usage: place-next <app_id> <x> <y> (x/y finite numbers)\n".to_string();
                };
                self.pending_placements.retain(|(id, _, _, _, _)| id != &app_id);
                self.pending_placements.push((app_id, x, y, cell, std::time::Instant::now()));
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
                let Some(cce_core::ipc::ctl::Request::PlaceNext { app_id, x, y, cell }) = cce_core::ipc::ctl::Request::parse(cmd) else {
                    return "error: usage: place-next-cell <app_id> <x> <y> (x/y finite numbers)\n".to_string();
                };
                self.pending_placements.retain(|(id, _, _, _, _)| id != &app_id);
                self.pending_placements.push((app_id, x, y, cell, std::time::Instant::now()));
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
                        (*seat).handle_activity();
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
                    // The compositor's own Super state (window-adjust mode)
                    // reads the keyboard device, which injection bypasses.
                    if matches!(keycode, 125 | 126) {
                        self.injected_super_held = pressed;
                        self.refresh_adjust_held();
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
                        (*seat).handle_activity();
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
            // Portal global shortcuts (the `cce-shortcuts-portal` backend's
            // half of the contract lives in `global_shortcuts`).
            "shortcut" => crate::global_shortcuts::ipc(self, &parts[1..]),
            _ => format!("error: unknown command: {}\n", action),
        }
    }

    /// Run `f` on every seat's cursor (the synthetic-input commands act on all seats,
    /// like the pre-existing pointer-move-to loop did).
    pub(crate) unsafe fn for_each_cursor(&mut self, mut f: impl FnMut(&mut crate::cursor::Cursor)) {
        crate::wm_scope!(mut);
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            f(&mut (*seat).cursor);
            curr_seat = next_seat;
        }
    }

    /// `for_each_cursor` for `ccectl touch`: marks each seat as having had
    /// touch injected first, which offers clients the touch capability.
    pub(crate) unsafe fn for_each_seat_touch(&mut self, mut f: impl FnMut(&mut crate::cursor::Cursor)) {
        crate::wm_scope!(mut);
        let seats_list = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr_seat = (*seats_list).next;
        while curr_seat != seats_list {
            let next_seat = (*curr_seat).next;
            let seat = crate::container_of!(curr_seat, crate::seat::Seat, link);
            if !(*seat).touch_injected {
                (*seat).touch_injected = true;
                (*seat).update_capabilities();
            }
            f(&mut (*seat).cursor);
            curr_seat = next_seat;
        }
    }

    /// Button-name/evdev-code parsing for the pointer commands; a missing argument
    /// means the left button, like wlrctl.
    pub(crate) fn parse_pointer_button(arg: Option<&str>) -> Option<u32> {
        match arg {
            None | Some("left") => Some(0x110),
            Some("right") => Some(0x111),
            Some("middle") => Some(0x112),
            Some("back") | Some("side") => Some(0x113),
            Some("forward") | Some("extra") => Some(0x114),
            Some(other) => other.parse::<u32>().ok(),
        }
    }
}

/// Whether the session restore can relaunch a window from this saved
/// command. A Wine/Proton window records its WINDOWS-side exe path
/// (`C:\...` or `C:/...`) — /bin/sh can never run it — and an empty
/// command has nothing to run. `spawn_restored_one` skips such entries and
/// `create_restore_placeholders` draws no plate for them: Ubisoft Connect's
/// stood a minute over the empty desk every login (2026-09-26), waiting for
/// a window nothing had started.
/// A control-socket number: `str::parse::<f64>` accepts "NaN", "inf" and
/// "infinity", which no command means and which would reach pointer and
/// camera math as positions (cce-remote filters the same for its frames).
pub(crate) fn parse_finite(s: &str) -> Result<f64, ()> {
    s.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or(())
}

/// Most synthetic steps one `pointer-swipe` / `pointer-pinch` may take. The
/// steps run in one loop on the main thread, so a count like 4000000000 held
/// the whole session frozen; a real gesture is tens of events.
pub(crate) const MAX_INJECTED_STEPS: u32 = 1000;
