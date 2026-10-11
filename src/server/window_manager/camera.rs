//! The camera: pan and zoom state, edge panning, the queued pan / pinch / op
//! frames, and the eased camera animation stepped each frame. Split out of
//! window_manager.rs on 2026-10-10.

use super::*;

impl WindowManager {
    pub fn stop_panning_animation(&mut self) {
        crate::wm_scope!(mut);
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
        crate::wm_scope!();
        crate::shared::layout().input_config
            .scroll_ease
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(12.0)
    }

    /// Whether a trackpad flick coasts the desktop (`input { kinetic_scroll }`).
    pub fn kinetic_scroll(&self) -> bool {
        crate::wm_scope!();
        crate::shared::layout().input_config.kinetic_scroll.unwrap_or(true)
    }

    /// Coast decay, 1/s (`input { scroll_friction }`).
    pub fn scroll_friction(&self) -> f64 {
        crate::wm_scope!();
        crate::shared::layout().input_config
            .scroll_friction
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(6.0)
    }

    /// The current camera as the policy crate's plain-data snapshot.
    /// The camera as the LAYOUT sees it, plus the desk's sub-pixel render
    /// offset. Scene nodes sit on integer layout px, so the pan handed to
    /// placement is floored to a layout pixel (at the current zoom) and the
    /// remainder — each in (-1, 0] layout px — goes to scenefx, which
    /// shifts the desk trees by it in device px at render time. At output
    /// scale 2 that is what lets a slow pan move one device pixel per
    /// frame instead of two, the visible judder of HiDPI panning.
    pub fn layout_camera(&self) -> (crate::policy::camera::Camera, f64, f64) {
        crate::wm_scope!();
        let zoom = self.desk_zoom.max(1e-6);
        let split = |pan: f64| {
            let s = pan * zoom;
            let f = s.floor();
            (f / zoom, -(s - f))
        };
        let (pan_x, sub_x) = split(self.desk_pan_x);
        let (pan_y, sub_y) = split(self.desk_pan_y);
        (crate::policy::camera::Camera { pan_x, pan_y, zoom: self.desk_zoom }, sub_x, sub_y)
    }

    /// The largest enabled output scale: the device-pixel resolution a
    /// camera move is quantized at (see `update_viewport_local`).
    pub unsafe fn max_output_scale(&self) -> f64 {
        crate::wm_scope!();
        let mut best = 1.0f64;
        if self.server.is_null() {
            return best;
        }
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs_list).next;
        while curr != outputs_list {
            let output = crate::container_of!(curr, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                best = best.max((*output).current.scale as f64);
            }
            curr = (*curr).next;
        }
        best
    }

    pub fn camera(&self) -> crate::policy::camera::Camera {
        crate::wm_scope!();
        crate::policy::camera::Camera {
            pan_x: self.desk_pan_x,
            pan_y: self.desk_pan_y,
            zoom: self.desk_zoom,
        }
    }

    /// Record the edge auto-pan velocity (screen px/s) and arm its 16ms tick
    /// when nonzero. A zero velocity just parks: the armed tick sees it and
    /// stops itself without re-arming.
    pub unsafe fn set_edge_pan_velocity(&mut self, vx: f64, vy: f64) {
        crate::wm_scope!(mut);
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
        crate::wm_scope!(mut);
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
        crate::wm_scope!(mut);
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
        crate::wm_scope!(mut);
        self.pan_pending[0] += dx;
        self.pan_pending[1] += dy;
        self.schedule_frame_all_outputs();
    }

    /// Queue a pinch zoom about an output-local anchor for the next output
    /// frame (see `pinch_pending`).
    pub unsafe fn queue_pinch(&mut self, zoom: f64, ax: f64, ay: f64) {
        crate::wm_scope!(mut);
        self.pinch_pending = Some((zoom, ax, ay));
        self.schedule_frame_all_outputs();
    }

    /// Queue the interactive move/resize's configure and relayout for the
    /// next output frame (see `op_frame_pending`).
    pub unsafe fn queue_op_frame(&mut self) {
        crate::wm_scope!(mut);
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
        crate::wm_scope!(mut);
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
                    (*win).manage_finish(self);
                }
            }
            curr = (*curr).next;
        }
        if matches!(self.state, WindowManagerState::Idle) {
            crate::shared::pending().mark_windowing();
            self.manage_start();
        } else {
            crate::shared::pending().dirty_windowing();
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
        crate::wm_scope!(mut);
        let has_pending = self.pan_pending != [0.0, 0.0] || self.pinch_pending.is_some();
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
            if let Some((zoom, ax, ay)) = self.pinch_pending.take() {
                // Like the wheel, pinch pivots about the cursor: the virtual
                // point under it stays put on screen.
                let cam = crate::policy::camera::zoom_about_anchor(self.camera(), ax, ay, zoom);
                self.desk_pan_x = cam.pan_x;
                self.desk_pan_y = cam.pan_y;
                self.desk_zoom = cam.zoom;
                self.set_mode(if crate::policy::camera::is_overview(cam.zoom) { WindowManagerMode::Overview } else { WindowManagerMode::Normal });
            }
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
            crate::shared::pending().dirty_windowing();
        }
    }

    /// Advance the camera animation by `dt` seconds. Returns true when
    /// nothing is left to animate.
    pub(crate) fn advance_camera_animation(&mut self, dt: f64, frame_target_ns: u64) -> bool {
        crate::wm_scope!(mut);
        let mut done = true;
        // Animations off (`cce_core::motion`): every ease below covers its
        // whole distance in this step, the ramp lands, and a flick does not
        // coast — the camera still goes where it was sent, just at once.
        let animate = cce_core::motion::enabled();
        // Frame-rate independent exponential approach: the same fraction of
        // the remaining distance per unit time whatever the frame pacing.
        let factor = if animate { 1.0 - (-self.scroll_ease_rate() * dt).exp() } else { 1.0 };

        // Ramp-driven transition: position is a pure function of elapsed
        // time, so a stalled frame never changes where the camera lands.
        let ramp = self.camera_ramp_anim.as_ref().map(|a| {
            (a.start, a.target, frame_target_ns.saturating_sub(a.started_ns) as f64 / 1e6 / a.duration_ms)
        });
        if let Some((start, target, t)) = ramp {
            if t >= 1.0 || !animate {
                self.desk_pan_x = target.pan_x;
                self.desk_pan_y = target.pan_y;
                self.desk_zoom = target.zoom;
                self.camera_ramp_anim = None;
            } else if let Some((ramp, _)) = &crate::shared::layout().overview_anim {
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
        if !animate {
            self.pan_coast_vx = 0.0;
            self.pan_coast_vy = 0.0;
        }
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
}
