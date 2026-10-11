//! Where a new window maps: its saved spot (`try_restore`), beside a sibling,
//! cascaded off siblings, a client's size hint, clear of the tiles, on the grid
//! cell it was invoked at, or centred on the view for a modal. Split out of
//! window.rs on 2026-10-10.

use super::*;

impl Window {
    pub unsafe fn try_restore(&mut self) {
        if self.restored {
            return;
        }
        // No geometry is ever saved for a Utility window, so none may be
        // restored over it — a pre-Utility state.json entry for the same
        // app_id would otherwise dictate a stale size to a self-sizing
        // client. (Belt over suspenders: the arrange pass restates the
        // "you choose" 0x0 for Utility anyway, so even a slipped-through
        // restore heals on the client's next commit.)
        if self.tiling_mode == crate::tiling::TilingMode::Utility {
            return;
        }
        // A transient — an xdg toplevel with a parent, or an X11 window with
        // WM_TRANSIENT_FOR — is a dialog of the window it hangs off, and is
        // never what a saved entry describes. It shares its app_id with the
        // main window, so the app_id-only third pass of the state matchers
        // (kept for a relaunched main window whose title has changed) would
        // hand it the MAIN window's geometry: Houdini's Preferences opened at
        // the full 1856x1141 of the session it belongs to, and hkey's
        // "Redeem Result" at the administrator's size. The save pass skips
        // transients for the same reason, so there is nothing of their own to
        // restore either; they size themselves.
        if !self.get_parent().is_null() {
            return;
        }
        // For an X11 window that check is only meaningful once its properties
        // are all in: they arrive one PropertyNotify at a time, and WM_CLASS
        // (the app_id) lands before WM_TRANSIENT_FOR, so on the app_id notify
        // a dialog still looks parentless and the app_id-only match below
        // restored it anyway — first match wins, and the restore overwrites
        // the client's own requested size, so it cannot be undone when the
        // parent turns up. (Waiting for the wl_surface was not enough: GTK's
        // dialog was still title-less and parentless at association.) Wait
        // for `map`, which calls back in here; by then every property the
        // client set before mapping has been read.
        if matches!(self.impl_type, WindowImpl::Xwayland(_)) && self.state != WindowState::Mapped {
            return;
        }
        // A full-screen X11 game (`xwayland_hidpi_except`) sizes itself to
        // the screen; restoring a saved size onto it is what shrank
        // Trackmania to the launcher's 1214x689 — the game then pinned that
        // size in its hints and no fullscreen could take. Mark it restored
        // so nothing else tries. Where on the desk it was fullscreen is the
        // compositor's to remember, though, not the game's: that alone is
        // taken from its entry (`restore_fullscreen_at`).
        if crate::xwayland_window::window_is_hidpi_exempt(self as *const Window) {
            let app_id = self.get_app_id_string().unwrap_or_default();
            let title = self.get_title_string().unwrap_or_default();
            let program = crate::window_manager::proc_args(self.unreliable_pid()).into_iter().next();
            let wm = &mut (*crate::reentry::wm(self.server));
            let saved = wm
                .match_and_remove_restore_state(&app_id, &title, program.as_deref())
                .or_else(|| wm.match_last_window_state(&app_id, &title, program.as_deref()));
            self.restore_fullscreen_at = saved.and_then(|s| s.fullscreen_at);
            log::info!(
                "Not restoring saved state for {:?}: named in xwayland_hidpi_except, it places itself (saved fullscreen spot: {:?})",
                title,
                self.restore_fullscreen_at
            );
            self.restored = true;
            return;
        }
        // A shy helper window (no-activate, skip-taskbar) is placed by its
        // app, relative to the app's own windows — see `is_shy`.
        if self.is_shy() {
            log::info!(
                "Not restoring saved state for {:?} ({}): a no-activate helper window, its app places it",
                self.get_title_string().unwrap_or_default(),
                self.get_app_id_string().unwrap_or_default()
            );
            self.restored = true;
            return;
        }
        let app_id_str = self.get_app_id_string().unwrap_or_default();
        if app_id_str.is_empty()
            || app_id_str.starts_with("cce-status")
            || app_id_str == "cce-wallpaper"
            || app_id_str == "cce-grid"
        {
            return;
        }
        let title_str = self.get_title_string().unwrap_or_default();
        // A `mode_rule` with `title=` names one window of an app, and it can
        // only be judged once the title is in. Chromium/Electron set the
        // app_id first, and restoring on that notify handed Obsidian's
        // Settings window the MAIN window's entry by app_id alone — Tiled,
        // latched, at the main window's size — before the rule that floats
        // it could match. So an untitled window of an app some title rule
        // names waits for its title; `map` calls back in here regardless.
        if title_str.is_empty() && self.state != WindowState::Mapped {
            let wm = &(*crate::reentry::wm(self.server));
            if wm.mode_rules.iter().any(|r| {
                r.title_pattern.is_some()
                    && (r.app_id_pattern == "*" || app_id_str.contains(&r.app_id_pattern))
            }) {
                return;
            }
        }
        // With the title in, a title rule outranks an entry that is not this
        // window's own: the rule is about this window, the entry about
        // another one of the same app. An `over_sibling` rule outranks its
        // own entry too while a sibling is up — the window goes where the
        // sibling is, at the size it asks for.
        {
            let wm = &(*crate::reentry::wm(self.server));
            let rule = wm
                .get_rule_for_window(self as *mut Window)
                .filter(|r| r.title_pattern.is_some())
                .map(|r| r.over_sibling);
            if let Some(over_sibling) = rule {
                let has_sibling = over_sibling && !self.find_sibling(&app_id_str).is_null();
                let own_entry = wm.has_titled_saved_entry(&app_id_str, &title_str);
                if rule_skips_restore(has_sibling, own_entry) {
                    log::info!(
                        "Not restoring saved state for {:?} ({}): a title rule matches it{}",
                        title_str,
                        app_id_str,
                        if has_sibling { ", and it opens over its sibling" } else { "" }
                    );
                    self.satellite = has_sibling;
                    self.restored = true;
                    return;
                }
            }
        }
        // Which program this window belongs to, so an entry matched by
        // app_id alone is only borrowed from a run of the same one — see
        // `window_manager::same_program`.
        let program = crate::window_manager::proc_args(self.unreliable_pid()).into_iter().next();
        let program = program.as_deref();
        let mut saved_opt = (*crate::reentry::wm(self.server)).match_and_remove_restore_state(&app_id_str, &title_str, program);
        let from_session = saved_opt.is_some();
        if saved_opt.is_none() {
            saved_opt = (*crate::reentry::wm(self.server)).match_last_window_state(&app_id_str, &title_str, program);
        }
        if let Some(saved) = saved_opt {
            log::info!("Restoring saved state for window: app_id={}, title={}. Position: ({}, {}), Size: {}x{}", app_id_str, title_str, saved.virtual_x, saved.virtual_y, saved.width, saved.height);
            self.tiling_mode = saved.tiling_mode;
            // `minimized` is session state, not app memory: a window the
            // user just opened must never be born hidden. On a
            // `last_window_states` borrow the flag is whatever the sibling
            // (or the app's last incarnation) happened to be doing — and a
            // parentless dialog matched by app_id alone inherits it from
            // the LIVE main window, which the user may well have minimized
            // to get it out of the way. Focused, listed, and invisible.
            if from_session {
                self.minimized = saved.minimized;
            }
            self.virtual_x = saved.virtual_x;
            self.virtual_y = saved.virtual_y;
            self.restore_fullscreen_at = saved.fullscreen_at;
            self.scale = saved.scale;
            self.box_geom.width = saved.width as i32;
            self.box_geom.height = saved.height as i32;
            
            self.wm_requested.dimensions = Some(crate::window::Dimensions {
                width: saved.width,
                height: saved.height,
            });
            self.wm_requested.bounds = crate::window::Dimensions {
                width: saved.width,
                height: saved.height,
            };
            
            self.rendering_scheduled.width = saved.width;
            self.rendering_scheduled.height = saved.height;
            self.rendering_sent.width = saved.width;
            self.rendering_sent.height = saved.height;

            match self.impl_type {
                WindowImpl::Toplevel(toplevel) => {
                    if !toplevel.is_null() {
                        (*toplevel).geometry.width = saved.width as i32;
                        (*toplevel).geometry.height = saved.height as i32;
                    }
                }
                WindowImpl::Xwayland(xwindow) => {
                    // This pre-writes the wlroots mirror so `render_finish`
                    // reports the saved size from the first frame; X itself
                    // is still at the window's natural size until the
                    // arrange pass configures it. That configure must not
                    // be deduplicated against this mirror — see
                    // `xwayland_window::needs_configure`, which also checks
                    // the geometry the compositor has actually sent.
                    if !xwindow.is_null() && !(*xwindow).xsurface.is_null() {
                        let s = crate::xwayland_window::x11_scale_for(self.server, (*xwindow).xsurface);
                        (*(*xwindow).xsurface).width = crate::xwayland_window::to_x11(saved.width as i32, s) as u16;
                        (*(*xwindow).xsurface).height = crate::xwayland_window::to_x11(saved.height as i32, s) as u16;
                    }
                }
                _ => {}
            }

            // A restored non-Floating mode is EXPLICIT state, and has to be
            // latched to survive. `get_mode_for_window` returns the window's own
            // mode only when `mode_locked`; unlocked, it resolves from the config
            // rules and falls through to Floating — and the arrange pass writes
            // that resolution straight back into `tiling_mode`
            // (`window_manager.rs`, the `wp.tiling_mode` apply). So a window
            // restored Tiled but unlocked was demoted by the very next arrange,
            // which is why a relaunched app came back floating however exactly
            // its geometry had been restored: position, size and cell were all
            // right, and the mode was gone before the first frame.
            //
            // Both sibling promotions already pair the mode with the lock — the
            // seat's op_end detection, and the geometric one just below, which is
            // why a window saved Floating-but-aligned survived while one saved
            // Tiled did not. Only Floating is left unlatched here, so a window
            // with no explicit mode still resolves from the rules as before.
            if saved.tiling_mode != crate::tiling::TilingMode::Floating {
                self.mode_locked = true;
            }

            // Geometric promotion at restore time: a window whose saved
            // geometry sits cell-aligned IS tiled, even if an older session
            // saved it as Floating (pre-rework state, or a session that
            // never touched it after it landed on the grid). Same test and
            // lock as the op_end detection. No demotion here — a saved
            // Tiled window off the current grid is re-snapped by the Tiled
            // arrange arm instead.
            if self.tiling_mode == crate::tiling::TilingMode::Floating {
                let sp = crate::shared::layout().snap_params();
                if crate::policy::snap::is_cell_aligned(
                    self.virtual_x,
                    self.virtual_y,
                    saved.width as f64,
                    saved.height as f64,
                    &sp,
                    1.0,
                ) {
                    self.tiling_mode = crate::tiling::TilingMode::Tiled;
                    self.mode_locked = true;
                }
            }

            // A remembered FLOATING position is only worth keeping if it is
            // where the user can see it. The camera at restore is wherever
            // the session left it (or wherever the user has panned since a
            // relaunch), and a floating window a screen away from that is
            // lost, not remembered: Inkscape's start screen came back a full
            // viewport above the desk every login, at the cell its previous
            // incarnation had been saved in, with nothing on screen to say
            // it existed. Tiled windows are the grid's and stay put.
            //
            // Unless it is on the tiled desk: a window within a viewport of
            // the tiled windows' bounding box (`tiled_desk_bounds`, the
            // session's tiled entries still to restore plus the tiled
            // windows already up) is placed beside content the user pans
            // along, and stays where it was put — cce-data-editor parked
            // left of the first column came back mid-view every login.
            if self.tiling_mode == crate::tiling::TilingMode::Floating && !self.minimized {
                let (_, _, vp_w, vp_h) = self.first_enabled_output_box();
                let wm = &(*crate::reentry::wm(self.server));
                let cam = crate::policy::camera::Camera {
                    pan_x: wm.desk_pan_x,
                    pan_y: wm.desk_pan_y,
                    zoom: wm.desk_zoom,
                };
                let desk = wm.tiled_desk_bounds();
                if let Some((nx, ny)) = crate::policy::camera::recalled_origin(
                    self.virtual_x,
                    self.virtual_y,
                    saved.width as f64,
                    saved.height as f64,
                    cam,
                    vp_w,
                    vp_h,
                    desk,
                ) {
                    log::info!(
                        "Recalling off-view floating window into view: app_id={} remembered=({:.0},{:.0}) -> ({:.0},{:.0}) (tiled desk: {:?})",
                        app_id_str, self.virtual_x, self.virtual_y, nx, ny, desk
                    );
                    self.virtual_x = nx;
                    self.virtual_y = ny;
                }
            }

            // A borrowed origin is a live sibling's origin whenever the
            // app_id-only pass matched a window of an app that is still
            // running: a parentless dialog (1Password's CLI "Authorize"
            // prompt, a second browser window) lands exactly on the main
            // window's top-left corner, where it reads as part of that
            // window rather than a new one. Cascade it off any mapped
            // sibling already sitting there, the way every stacking WM
            // offsets a new window from the last. Session entries are
            // exempt: a restored layout is where the user left it.
            if !from_session && self.tiling_mode == crate::tiling::TilingMode::Floating {
                let (nx, ny) = self.cascade_off_siblings(&app_id_str, self.virtual_x, self.virtual_y);
                if (nx, ny) != (self.virtual_x, self.virtual_y) {
                    log::info!(
                        "Cascading new {} window off a sibling at ({:.0},{:.0}) -> ({:.0},{:.0})",
                        app_id_str, self.virtual_x, self.virtual_y, nx, ny
                    );
                    self.virtual_x = nx;
                    self.virtual_y = ny;
                }
            }

            self.restored = true;
            self.session_restored = from_session;
            // The saved `focused` flag only means something for the startup
            // restore queue; on a `last_window_states` borrow it is stale
            // (whether the app happened to be focused when last closed) and
            // must not feed the settle-phase focus gates.
            self.restored_focused = from_session && saved.focused;
        }
    }

    /// The window a satellite opens over: a mapped, visible window of the
    /// same app_id that is not itself a satellite — the focused one when it
    /// qualifies, since that is where the user asked for the settings.
    pub(crate) unsafe fn find_sibling(&self, app_id: &str) -> *mut Window {
        let me = self as *const Window;
        let wm = &(*crate::reentry::wm(self.server));
        let qualifies = |w: *mut Window| {
            !w.is_null()
                && w as *const Window != me
                && !(*w).closed
                && !(*w).minimized
                && !(*w).satellite
                && matches!((*w).state, WindowState::Mapped)
                && (*w).get_app_id_string().as_deref() == Some(app_id)
        };
        let focused = wm.focused_window();
        if qualifies(focused) {
            return focused;
        }
        wm.windows.iter().copied().find(|&w| qualifies(w)).unwrap_or(std::ptr::null_mut())
    }

    /// Centre a satellite over its sibling. Like `try_center_on_view` this
    /// owns the POSITION only, and latches a redo for the commit that
    /// brings the window's real size (`pending_view_center`).
    pub(crate) unsafe fn try_center_on_sibling(&mut self) {
        if !self.satellite {
            return;
        }
        self.minimized = false;
        self.pending_view_center = self.box_geom.width <= 0 || self.box_geom.height <= 0;
        self.apply_sibling_centering();
    }

    pub(crate) unsafe fn apply_sibling_centering(&mut self) {
        let app_id = self.get_app_id_string().unwrap_or_default();
        let sibling = self.find_sibling(&app_id);
        if sibling.is_null() {
            return;
        }
        let (_, _, vp_w, vp_h) = self.first_enabled_output_box();
        let wm = &(*crate::reentry::wm(self.server));
        let zoom = wm.desk_zoom.max(0.01);
        let (w, h) = self.mapped_size_hint();
        let (sw, sh) = (*sibling).mapped_size_hint();
        let (x, y) = centered_over(
            ((*sibling).virtual_x, (*sibling).virtual_y, sw, sh),
            (w, h),
            (wm.desk_pan_x, wm.desk_pan_y, vp_w / zoom, vp_h / zoom),
        );
        self.virtual_x = x;
        self.virtual_y = y;
        self.hint_placed = true;
        log::info!(
            "satellite centred over sibling: app_id={} size=({:.0}x{:.0}) sibling={:?} virtual=({:.1},{:.1})",
            app_id,
            w,
            h,
            (*sibling).get_title_string().unwrap_or_default(),
            x,
            y
        );
    }

    /// Step an origin diagonally until no mapped sibling of `app_id` (any
    /// window but this one) has its top-left within a few pixels of it.
    /// Bounded, so a pathological pile of siblings cannot walk a window off
    /// the desk: after `MAX_STEPS` the last candidate is taken as is.
    pub(crate) unsafe fn cascade_off_siblings(&self, app_id: &str, x: f64, y: f64) -> (f64, f64) {
        const STEP: f64 = 40.0;
        const NEAR: f64 = 4.0;
        const MAX_STEPS: usize = 8;
        let me = self as *const Window;
        let origins: Vec<(f64, f64)> = (*self.server)
            .wm
            .windows
            .iter()
            .copied()
            .filter(|&w| !w.is_null() && w as *const Window != me && !(*w).closed)
            .filter(|&w| matches!((*w).state, WindowState::Mapped))
            .filter(|&w| (*w).get_app_id_string().as_deref() == Some(app_id))
            .map(|w| ((*w).virtual_x, (*w).virtual_y))
            .collect();
        let taken = |cx: f64, cy: f64| {
            origins
                .iter()
                .any(|&(ox, oy)| (ox - cx).abs() <= NEAR && (oy - cy).abs() <= NEAR)
        };
        let (mut cx, mut cy) = (x, y);
        for _ in 0..MAX_STEPS {
            if !taken(cx, cy) {
                break;
            }
            cx += STEP;
            cy += STEP;
        }
        (cx, cy)
    }

    /// Apply a one-shot `place-next` hint: land the window's top-left just
    /// below-right of the hinted layout position (the control that spawned
    /// it), clamped to the output so it stays fully on-screen. Runs after
    /// `try_restore` so the remembered SIZE is kept — only the position is
    /// overridden — and marks `hint_placed` so the spawn viewport pan is
    /// skipped (the window is already under the user's pointer).
    /// Layout box of the first enabled output — `(phys_x, phys_y, width,
    /// height)`, the viewport every placement decision is measured against.
    /// Falls back to a 1920x1080 box at the origin before any output is up.
    pub(crate) unsafe fn first_enabled_output_box(&self) -> (f64, f64, f64, f64) {
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let b = (*output).sent.box_layout();
                return (b.x as f64, b.y as f64, b.width as f64, b.height as f64);
            }
            curr_out = (*curr_out).next;
        }
        (0.0, 0.0, 1920.0, 1080.0)
    }

    /// Virtual position to layout (screen) position, ROUNDED — the same
    /// conversion the arrange pass makes (`PlacementCtx::virtual_to_screen`).
    /// Every writer of a window's screen origin has to agree on the
    /// rounding: the seat op and the resize-commit anchoring truncated while
    /// the arrange pass rounds, so whenever the fractional part was .5 or
    /// more the window stepped a pixel back and forth between a commit and
    /// the next arrange — a twitch on every resize step at overview zoom,
    /// and a one-pixel hop on grab and release.
    pub unsafe fn virtual_to_screen(&self, vx: f64, vy: f64) -> (i32, i32) {
        let wm = &(*crate::reentry::wm(self.server));
        let (cam, _, _) = wm.layout_camera();
        let (out_x, out_y, _, _) = self.first_enabled_output_box();
        (
            out_x as i32 + ((vx - cam.pan_x) * cam.zoom).round() as i32,
            out_y as i32 + ((vy - cam.pan_y) * cam.zoom).round() as i32,
        )
    }

    /// Layout (screen) position back to a virtual position — the inverse of
    /// `virtual_to_screen`. A client that repositions itself hands us a
    /// SCREEN origin, but the arrange pass places a floating window from its
    /// VIRTUAL one, so a screen origin written on its own survives exactly
    /// until the next transaction and is then recomputed away.
    pub unsafe fn screen_to_virtual(&self, wm: &crate::window_manager::WindowManager, sx: i32, sy: i32) -> (f64, f64) {
        let (cam, _, _) = wm.layout_camera();
        let zoom = cam.zoom.max(0.01);
        let (out_x, out_y, _, _) = self.first_enabled_output_box();
        (
            cam.pan_x + (sx as f64 - out_x) / zoom,
            cam.pan_y + (sy as f64 - out_y) / zoom,
        )
    }

    /// Best-known window size in VIRTUAL units at map time. `box_geom` is the
    /// render pass's size and is only filled in once a frame has been drawn
    /// (or by `try_restore` from the saved geometry), so a first-ever launch
    /// falls back to the client's committed toplevel geometry.
    pub(crate) unsafe fn mapped_size_hint(&self) -> (f64, f64) {
        if self.box_geom.width > 0 && self.box_geom.height > 0 {
            return (self.box_geom.width as f64, self.box_geom.height as f64);
        }
        if let WindowImpl::Toplevel(toplevel) = self.impl_type {
            if !toplevel.is_null() {
                let g = (*toplevel).geometry;
                if g.width > 0 && g.height > 0 {
                    return (g.width as f64, g.height as f64);
                }
            }
        }
        (400.0, 400.0)
    }

    pub(crate) unsafe fn try_hint_placement(&mut self) {
        let app_id = self.get_app_id_string().unwrap_or_default();
        if app_id.is_empty() {
            return;
        }
        // Claimed before the mode is judged, so a hint aimed at this window
        // does not linger and land on the next one to open.
        let Some((hx, hy, cell_anchored)) = (*crate::reentry::wm(self.server)).take_pending_placement(&app_id)
        else {
            return;
        };
        if cell_anchored {
            // TILED IS THE POINT here, unlike the position-only hint below: a
            // window that reopens filling four squares is exactly the case
            // this exists for. Only the modes that do not own a position at
            // all are excluded.
            if matches!(
                self.tiling_mode,
                crate::tiling::TilingMode::Fullscreen
                    | crate::tiling::TilingMode::Popup
                    | crate::tiling::TilingMode::Overlay
                    | crate::tiling::TilingMode::Status
            ) {
                return;
            }
            self.place_on_invocation_cell(&app_id, hx, hy);
            return;
        }
        // Utility included: the hint moves only the POSITION, which a utility
        // window does not own — only its size is the client's.
        if !matches!(
            self.tiling_mode,
            crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Utility
        ) {
            return;
        }

        let (phys_x, phys_y, vp_w, vp_h) = self.first_enabled_output_box();

        let wm = &(*crate::reentry::wm(self.server));
        let zoom = wm.desk_zoom.max(0.01);
        let (vw, vh) = self.mapped_size_hint();
        let (w, h) = (vw * zoom, vh * zoom);

        const OFFSET: f64 = 12.0; // context-menu-style drop below-right of the control
        const MARGIN: f64 = 8.0;
        let sx = (hx + OFFSET)
            .min(phys_x + vp_w - w - MARGIN)
            .max(phys_x + MARGIN);
        let sy = (hy + OFFSET)
            .min(phys_y + vp_h - h - MARGIN)
            .max(phys_y + MARGIN);

        // screen = phys + (virtual - desk_pan) * zoom  →  invert for virtual.
        self.virtual_x = wm.desk_pan_x + (sx - phys_x) / zoom;
        self.virtual_y = wm.desk_pan_y + (sy - phys_y) / zoom;
        self.hint_placed = true;
        log::info!(
            "place-next hint applied: app_id={} screen=({:.0},{:.0}) virtual=({:.1},{:.1})",
            app_id, sx, sy, self.virtual_x, self.virtual_y
        );
    }

    /// Step a freshly-spawned TILED window off any tiled window it would open
    /// on top of, keeping its size and staying as close to its intended spot
    /// as possible (`policy::spawn::nearest_free`).
    ///
    /// The remembered-position path has no idea whether that position is still
    /// free — it was when the window closed, and something else may have taken
    /// it since. Two tiled windows stacked on the same squares is never what
    /// was meant: tiled windows are the ones laid out to sit side by side.
    ///
    /// Deliberately narrow:
    /// - Only TILED windows are moved, and only tiled windows count as
    ///   obstacles. Floating windows overlap by nature; that is the difference
    ///   between the two modes, not a fault to correct.
    /// - Session restore is exempt. A restored layout is a layout the user
    ///   arranged and saved, and mapping order is arbitrary, so nudging there
    ///   would rearrange a deliberate desktop at every login.
    pub(crate) unsafe fn avoid_tiled_overlap(&mut self) {
        if self.session_restored || self.tiling_mode != crate::tiling::TilingMode::Tiled {
            return;
        }
        let wm = &(*crate::reentry::wm(self.server));
        let sp = crate::shared::layout().snap_params();
        if sp.cell_w <= 0.5 || sp.cell_h <= 0.5 {
            return;
        }
        let (vw, vh) = self.mapped_size_hint();
        if vw <= 0.0 || vh <= 0.0 {
            return;
        }
        let (c0, r0, c1, r1) = crate::policy::cells::window_span(
            self.virtual_x, self.virtual_y, vw, vh, sp.cell_w, sp.cell_h, sp.gap_width,
        );
        let want = crate::policy::spawn::CellBlock::new(c0, r0, c1, r1);

        let mut occupied = Vec::new();
        for &w in wm.windows.iter() {
            if w.is_null() || w == (self as *mut Window) || (*w).closed || (*w).minimized {
                continue;
            }
            if !matches!((*w).state, WindowState::Mapped) {
                continue;
            }
            if (*w).tiling_mode != crate::tiling::TilingMode::Tiled {
                continue;
            }
            let (ow, oh) = ((*w).box_geom.width as f64, (*w).box_geom.height as f64);
            if ow <= 0.0 || oh <= 0.0 {
                continue;
            }
            let (oc0, or0, oc1, or1) = crate::policy::cells::window_span(
                (*w).virtual_x, (*w).virtual_y, ow, oh, sp.cell_w, sp.cell_h, sp.gap_width,
            );
            occupied.push(crate::policy::spawn::CellBlock::new(oc0, or0, oc1, or1));
        }
        if occupied.is_empty() {
            return;
        }

        // Bounded: a window that cannot find room nearby stays put rather than
        // being flung to an empty region of a desktop that has no edges.
        const SEARCH_SQUARES: i32 = 12;
        let free = crate::policy::spawn::nearest_free(want, &occupied, SEARCH_SQUARES);
        if free == want {
            return;
        }
        let (bx, by, _, _) = crate::policy::cells::block_rect(
            free.col0, free.row0, free.col1, free.row1,
            sp.cell_w, sp.cell_h, sp.gap_width, sp.cell_inset,
        );
        log::info!(
            "spawn overlap: {} would open on a tiled window at {} -> moved to {}",
            self.get_app_id_string().unwrap_or_default(),
            crate::policy::cells::span_label(want.col0, want.row0, want.col1, want.row1),
            crate::policy::cells::span_label(free.col0, free.row0, free.col1, free.row1),
        );
        self.virtual_x = bx;
        self.virtual_y = by;
    }

    /// Place this window on the grid square the user invoked it from, keeping
    /// its remembered SIZE and growing away from the windows already there
    /// (`policy::spawn::place_at_cell`).
    ///
    /// The size comes from the remembered geometry `try_restore` just applied,
    /// measured in whole squares: a window last seen filling four squares
    /// opens filling four squares, at the corner of the invocation square that
    /// leaves it clear of its neighbours.
    pub(crate) unsafe fn place_on_invocation_cell(&mut self, app_id: &str, hx: f64, hy: f64) {
        let wm = &(*crate::reentry::wm(self.server));
        let sp = crate::shared::layout().snap_params();
        if sp.cell_w <= 0.5 || sp.cell_h <= 0.5 {
            return;
        }
        let (phys_x, phys_y, vp_w, vp_h) = self.first_enabled_output_box();
        let zoom = wm.desk_zoom.max(0.01);
        // The hint is a layout point; the grid is in virtual coordinates.
        let inv_vx = wm.desk_pan_x + (hx - phys_x) / zoom;
        let inv_vy = wm.desk_pan_y + (hy - phys_y) / zoom;
        let col = crate::policy::cells::cell_index(inv_vx, sp.cell_w, sp.gap_width);
        let row = crate::policy::cells::cell_index(inv_vy, sp.cell_h, sp.gap_width);

        // Size in squares, from the geometry `try_restore` left in place.
        let (vw, vh) = self.mapped_size_hint();
        let (c0, r0, c1, r1) = crate::policy::cells::window_span(
            0.0, 0.0, vw, vh, sp.cell_w, sp.cell_h, sp.gap_width,
        );
        let (cols, rows) = (c1 - c0 + 1, r1 - r0 + 1);

        // Everything else already on the desktop, in squares. Chrome and the
        // canvas itself are not obstacles.
        let mut occupied = Vec::new();
        for &w in wm.windows.iter() {
            if w.is_null() || w == (self as *mut Window) || (*w).closed || (*w).minimized {
                continue;
            }
            if !matches!((*w).state, WindowState::Mapped) {
                continue;
            }
            if (*w).is_status_bar() || (*w).is_wallpaper() || (*w).is_grid() {
                continue;
            }
            let (ow, oh) = ((*w).box_geom.width as f64, (*w).box_geom.height as f64);
            if ow <= 0.0 || oh <= 0.0 {
                continue;
            }
            let (oc0, or0, oc1, or1) = crate::policy::cells::window_span(
                (*w).virtual_x, (*w).virtual_y, ow, oh, sp.cell_w, sp.cell_h, sp.gap_width,
            );
            occupied.push(crate::policy::spawn::CellBlock::new(oc0, or0, oc1, or1));
        }

        // Visible squares, so a tie between two clear corners goes to the one
        // on screen.
        let view = {
            let (vx0, vy0) = (wm.desk_pan_x, wm.desk_pan_y);
            let (vx1, vy1) = (vx0 + vp_w / zoom, vy0 + vp_h / zoom);
            let c0 = crate::policy::cells::cell_index(vx0, sp.cell_w, sp.gap_width);
            let r0 = crate::policy::cells::cell_index(vy0, sp.cell_h, sp.gap_width);
            let c1 = crate::policy::cells::cell_index(vx1, sp.cell_w, sp.gap_width);
            let r1 = crate::policy::cells::cell_index(vy1, sp.cell_h, sp.gap_width);
            crate::policy::spawn::CellBlock::new(c0, r0, c1, r1)
        };

        let block = crate::policy::spawn::place_at_cell(col, row, cols, rows, &occupied, Some(view));
        let (bx, by, bw, bh) = crate::policy::cells::block_rect(
            block.col0, block.row0, block.col1, block.row1,
            sp.cell_w, sp.cell_h, sp.gap_width, sp.cell_inset,
        );
        self.virtual_x = bx;
        self.virtual_y = by;
        // A window that was filling whole squares keeps doing so — it is the
        // same window, in the same shape, somewhere else. One that was not
        // keeps its own size and simply starts at the square's corner.
        if self.tiling_mode == crate::tiling::TilingMode::Tiled {
            self.box_geom.width = bw.round() as i32;
            self.box_geom.height = bh.round() as i32;
            self.wm_requested.dimensions = Some(crate::window::Dimensions {
                width: bw.round() as u32,
                height: bh.round() as u32,
            });
        }
        self.hint_placed = true;
        log::info!(
            "place-next-cell: {} -> {} ({}x{} squares) at virtual ({:.0}, {:.0})",
            app_id,
            crate::policy::cells::span_label(block.col0, block.row0, block.col1, block.row1),
            cols, rows, bx, by
        );
    }

    /// Open a session modal in the middle of what the user is looking at,
    /// ignoring wherever it last sat.
    ///
    /// On a panning desktop a remembered position is actively wrong for these
    /// windows: the camera has almost always moved since the last time, so
    /// the window maps somewhere off-view and the prompt reads as never
    /// having appeared — which for the polkit agent means the privileged
    /// action silently times out.
    ///
    /// Runs after `try_restore`, so the remembered SIZE is still available
    /// and only the position is overridden — the same split
    /// `try_hint_placement` uses — and marks `hint_placed` so the spawn
    /// viewport pan is skipped: the window is already centered in view, and
    /// panning the camera to it would move the desktop out from under the
    /// user for a dialog that is about to close again.
    /// Windows that open centered on the current view rather than wherever
    /// they last were: DE session modals whose whole job is to interrupt, and
    /// which the user must be able to answer immediately.
    ///
    /// Hardcoded by app_id like the compositor's other DE-internal window
    /// classes (`cce-status*`/`cce-wallpaper`/`cce-grid` in `try_restore`,
    /// `cce-notifier`/`cce-cloud` in `get_mode_for_window`). A third-party
    /// prompt asks for the same treatment through a `mode_rule` with
    /// `center` (`wants_view_center`): 1Password's authorization popup is a
    /// parentless Electron toplevel under the vault window's app_id, told
    /// apart by its bare title, and it restored Tiled to wherever it was
    /// last answered.
    pub(crate) fn is_view_centered_modal(app_id: &str) -> bool {
        // The polkit prompt, and the file chooser cce-files runs in --select/
        // --save mode: both are spawned BY an action in the current view and
        // must be answered immediately — a remembered position is actively
        // wrong for them (the chooser used to map wherever the file manager
        // was last used, squares away from the app that opened it).
        app_id == "cce-authenticator" || app_id == "cce-filesystem-chooser"
    }

    /// The built-in modal list, or a matching `mode_rule` that says `center`.
    pub(crate) unsafe fn wants_view_center(&mut self) -> bool {
        let app_id = self.get_app_id_string().unwrap_or_default();
        if Self::is_view_centered_modal(&app_id) {
            return true;
        }
        (*crate::reentry::wm(self.server)).get_rule_for_window(self as *mut Window).map_or(false, |r| r.center)
    }

    pub(crate) unsafe fn try_center_on_view(&mut self) {
        if !self.wants_view_center() {
            return;
        }

        // Whatever history says, a modal has to be visible and free-floating:
        // a restored Tiled mode would re-snap it onto a grid cell (undoing
        // the centering) and a restored `minimized` would hide the prompt
        // outright. `mode_locked` is the "explicit beats heuristic" latch, so
        // the arrange pass cannot geometrically re-promote it either.
        //
        // Utility is exempt from the mode forcing ONLY — like
        // `try_hint_placement`, this owns the window's POSITION, never its
        // size. A Utility window already satisfies everything the forcing is
        // for: it always floats, never tiles, and both the grid snap and the
        // overview displacement skip it. Overwriting the field would silently
        // strip the mode — `set_utility` arrives before map, and every Utility
        // gate reads `tiling_mode` RAW — leaving the modal resizable, its
        // geometry saved, and a stale size restored over it next time.
        if self.tiling_mode != crate::tiling::TilingMode::Utility {
            self.tiling_mode = crate::tiling::TilingMode::Floating;
        }
        self.mode_locked = true;
        self.minimized = false;

        // A self-sizing modal has not committed its geometry yet, so
        // `mapped_size_hint` here is still the 400x400 floor — centering
        // against that misses by half the difference from the real size (a
        // 640x360 prompt landed 120px right and 20px high). Center anyway so
        // the first frame is not wildly off, and latch a redo for the commit
        // that brings the truth.
        //
        // Any mode with unknown geometry latches the redo — not Utility only.
        // The file chooser disproved the old Utility-only reasoning: a
        // FLOATING self-sizer on its first ever run has no restored geometry
        // and no arrange-given size either, so it was centered against the
        // 400x400 floor and stuck there, ~250px off for a 900x500 dialog.
        // A Floating modal with restored geometry still skips the latch
        // (box_geom is already filled by the time we run).
        self.pending_view_center = self.box_geom.width <= 0 || self.box_geom.height <= 0;
        self.apply_view_centering();
    }

    /// The centering itself, split out so the self-sizing commit path can redo
    /// it once the client's real size lands.
    pub(crate) unsafe fn apply_view_centering(&mut self) {
        if self.satellite {
            self.apply_sibling_centering();
            return;
        }
        let (_, _, vp_w, vp_h) = self.first_enabled_output_box();
        let wm = &(*crate::reentry::wm(self.server));
        let zoom = wm.desk_zoom.max(0.01);
        let (w, h) = self.mapped_size_hint();

        // Policy owns the camera math; the output's origin cancels out of the
        // centering, so only the extent is needed per axis.
        self.virtual_x = crate::policy::camera::centered_window_origin(wm.desk_pan_x, vp_w, zoom, w);
        self.virtual_y = crate::policy::camera::centered_window_origin(wm.desk_pan_y, vp_h, zoom, h);
        self.hint_placed = true;
        log::info!(
            "view-centered modal: app_id={} size=({:.0}x{:.0}) zoom={:.2} virtual=({:.1},{:.1})",
            self.get_app_id_string().unwrap_or_default(),
            w, h, zoom, self.virtual_x, self.virtual_y
        );
    }

    /// Redo a latched view-centering now that a self-sizing modal's real
    /// geometry has arrived. One-shot: a later commit (or a user dragging the
    /// window) must not snap it back to the middle.
    pub unsafe fn take_pending_view_center(&mut self) {
        if !self.pending_view_center || self.box_geom.width <= 0 || self.box_geom.height <= 0 {
            return;
        }
        self.pending_view_center = false;
        self.apply_view_centering();
    }
}
