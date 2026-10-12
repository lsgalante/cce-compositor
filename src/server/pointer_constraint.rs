use crate::ffi;
use crate::seat::Seat;

#[derive(Clone, Copy)]
pub enum PointerConstraintState {
    Inactive,
    Active {
        node: *mut ffi::wlr_scene_node,
        sx: f64,
        sy: f64,
    },
}

pub struct PointerConstraint {
    /// The wlroots constraint; the destroy handler takes it before the box
    /// goes. Private, as every `Wlr` field.
    wlr_constraint: crate::wlr::Wlr<ffi::wlr_pointer_constraint_v1>,
    /// Private: an `Active` node is live exactly while
    /// `node_destroy_listener` is connected to it (`maybe_activate` connects
    /// it on the node a hit test just returned; the node's destroy, and
    /// every deactivation, disconnect it first), which the safe methods
    /// below rely on.
    state: PointerConstraintState,
    pub destroy_listener: crate::listener::Listener,
    pub commit_listener: crate::listener::Listener,
    pub node_destroy_listener: crate::listener::Listener,
}

/// Whether a pointer constraint may hold the pointer now: in normal mode
/// with no Super-held window adjust. Overview and adjust mode drive the
/// pointer themselves, and a game keeping keyboard focus must not freeze
/// them.
pub fn applies_now() -> bool {
    crate::shared::mode() == crate::window_manager::WindowManagerMode::Normal && !crate::shared::window_adjust_active()
}

/// Run `f` on seat `sid`'s pointer constraint, if it holds one.
pub fn with_held<R>(
    wm: &crate::window_manager::WindowManager,
    sid: crate::seat::SeatId,
    f: impl FnOnce(&mut PointerConstraint, &crate::window_manager::WindowManager) -> R,
) -> Option<R> {
    let c = wm.seats.get(sid)?.cursor.constraint();
    if c.is_null() {
        return None;
    }
    // SAFETY: `Cursor::constraint` is null or a live constraint
    // (`set_constraint`'s contract). It is its own allocation, apart from the
    // window manager `f` also gets, and no other reference to it is live:
    // its handlers run only from wlroots signals, which nothing in a
    // constraint method raises synchronously.
    Some(f(unsafe { &mut *c }, wm))
}

/// What a pointer motion does under seat `sid`'s constraint.
pub enum MotionUnder {
    /// Nothing holds the pointer (or the constraint just let go).
    Free,
    /// Locked: the pointer stays where it is.
    Locked,
}

/// Apply seat `sid`'s constraint to a motion by (`dx`, `dy`) from layout
/// (`lx`, `ly`): deactivate it where it no longer applies, report a lock,
/// or clamp the motion to a confining region.
pub fn constrain_motion(
    wm: &crate::window_manager::WindowManager,
    sid: crate::seat::SeatId,
    lx: f64,
    ly: f64,
    dx: &mut f64,
    dy: &mut f64,
) -> MotionUnder {
    with_held(wm, sid, |c, wm| {
        if !c.is_active() {
            MotionUnder::Free
        } else if !applies_now() {
            c.deactivate(wm);
            MotionUnder::Free
        } else if c.locks() {
            MotionUnder::Locked
        } else {
            c.confine(lx, ly, dx, dy);
            MotionUnder::Free
        }
    })
    .unwrap_or(MotionUnder::Free)
}

impl PointerConstraint {
    /// A client asked to lock or confine the pointer on one of its
    /// surfaces. It takes effect while that surface has keyboard focus
    /// (`Seat::sync_pointer_constraint` hands it over on focus changes) and
    /// the pointer is over its region.
    pub unsafe fn create(wm: &mut crate::window_manager::WindowManager, wlr_constraint: *mut ffi::wlr_pointer_constraint_v1) -> *mut Self {
        let constraint = Box::into_raw(Box::new(Self {
            // SAFETY: the destroy handler takes it before the box goes,
            // inside wlroots' destroy of the constraint.
            wlr_constraint: crate::wlr::Wlr::from_raw(wlr_constraint),
            state: PointerConstraintState::Inactive,
            destroy_listener: crate::listener::Listener::default(),
            commit_listener: crate::listener::Listener::default(),
            node_destroy_listener: crate::listener::Listener::default(),
        }));

        (*wlr_constraint).data = constraint as *mut _;

        (*constraint).destroy_listener.connect(&mut (*wlr_constraint).events.destroy, handle_destroy);
        let commit_signal = ffi::river_wlr_surface_get_commit_signal((*wlr_constraint).surface);
        (*constraint).commit_listener.connect(commit_signal, handle_commit);

        ffi::river_wlr_pointer_constraint_default_region(wlr_constraint);

        let Some(sid) = Seat::id_of((*wlr_constraint).seat) else { return constraint };
        let Some(seat) = wm.seats.get_mut(sid) else { return constraint };
        let focused = seat.wlr_seat().keyboard_focused_surface();
        if !focused.is_null() && focused == (*wlr_constraint).surface && seat.cursor.constraint().is_null() {
            seat.cursor.set_constraint(constraint);
            (*constraint).maybe_activate(wm);
        }

        constraint
    }

    /// The wlroots constraint.
    pub fn wlr_constraint(&self) -> &crate::wlr::Wlr<ffi::wlr_pointer_constraint_v1> {
        &self.wlr_constraint
    }

    /// Whether it holds the pointer now.
    pub fn is_active(&self) -> bool {
        matches!(self.state, PointerConstraintState::Active { .. })
    }

    /// The seat this constraint is for, if it is still in the store.
    fn seat<'a>(&self, wm: &'a crate::window_manager::WindowManager) -> Option<&'a Seat> {
        self.wlr_constraint.seat_id().and_then(|sid| wm.seats.get(sid))
    }

    pub fn maybe_activate(&mut self, wm: &crate::window_manager::WindowManager) {
        let Some(seat) = self.seat(wm) else { return };
        if seat.cursor.constraint() != self as *mut Self {
            return;
        }
        if self.is_active() {
            return;
        }
        // The compositor owns the pointer during an op, and in overview or
        // adjust mode (`applies_now`).
        if seat.op.is_some() || !applies_now() {
            return;
        }

        let cx = seat.cursor.x();
        let cy = seat.cursor.y();

        if let Some(result) = crate::shared::scene().at(wm, cx, cy) {
            if result.surface != self.wlr_constraint.surface() {
                return;
            }

            let sx = result.sx;
            let sy = result.sy;

            if !self.wlr_constraint.region_contains(sx, sy) {
                return;
            }

            self.state = PointerConstraintState::Active {
                node: result.node,
                sx,
                sy,
            };

            // SAFETY: `result.node` is the live node the hit test just
            // returned; the listener is disconnected (by the node's destroy,
            // a deactivation, or the constraint's destroy) before either
            // goes.
            unsafe {
                let destroy_signal = ffi::river_scene_node_get_destroy_signal(result.node);
                self.node_destroy_listener.connect(destroy_signal, handle_node_destroy);
            }

            log::info!("activating pointer constraint");
            self.wlr_constraint.send_activated();
        }
    }

    /// Whether it is active and locks (rather than confines) the pointer.
    pub fn locks(&self) -> bool {
        self.is_active() && self.wlr_constraint.is_lock()
    }

    pub fn deactivate(&mut self, wm: &crate::window_manager::WindowManager) {
        let seat = self.seat(wm);
        self.deactivate_with(seat);
    }

    /// `deactivate` for the paths that have no window manager to look the
    /// seat up in (a scene node or the constraint going away): `seat`, if
    /// any, gets the cursor hint warp.
    fn deactivate_with(&mut self, seat: Option<&Seat>) {
        if !self.is_active() {
            return;
        }

        if let Some(seat) = seat {
            self.warp_to_hint_if_set(seat);
        }

        self.state = PointerConstraintState::Inactive;
        self.node_destroy_listener.disconnect();
        self.wlr_constraint.send_deactivated();
    }

    pub fn update_state(&mut self, wm: &crate::window_manager::WindowManager) {
        if !applies_now() {
            self.deactivate(wm);
            return;
        }
        self.maybe_activate(wm);

        if let PointerConstraintState::Active { sx, sy, .. } = self.state {
            let Some(seat) = self.seat(wm) else { return };

            let Some((lx, ly, true)) = self.active_node_coords() else {
                log::info!("deactivating pointer constraint, scene node disabled");
                self.deactivate(wm);
                return;
            };

            let scale = self.surface_scale();
            let warp_lx = lx as f64 + sx / scale;
            let warp_ly = ly as f64 + sy / scale;

            if !seat.cursor.wlr_cursor().warp(warp_lx, warp_ly) {
                log::info!("deactivating pointer constraint, could not warp cursor");
                self.deactivate(wm);
                return;
            }

            if !self.wlr_constraint.region_contains(sx, sy) {
                log::info!("deactivating pointer constraint, cursor outside region despite warp");
                self.deactivate(wm);
            }
        }
    }

    /// The active node's layout position and whether it and every ancestor
    /// are enabled; `None` when inactive.
    fn active_node_coords(&self) -> Option<(i32, i32, bool)> {
        let PointerConstraintState::Active { node, .. } = self.state else { return None };
        if node.is_null() {
            return None;
        }
        let (mut x, mut y) = (0, 0);
        // SAFETY: an active node is live (the `state` field's invariant).
        let enabled = unsafe { ffi::wlr_scene_node_coords(node, &mut x, &mut y) };
        Some((x, y, enabled))
    }

    /// Surface px per layout px for the surface the active node draws: a
    /// window can be drawn zoomed, where a layout px is not a surface px.
    /// 1 when inactive.
    fn surface_scale(&self) -> f64 {
        let PointerConstraintState::Active { node, .. } = self.state else { return 1.0 };
        let surface = self.wlr_constraint.surface();
        if node.is_null() || surface.is_null() {
            return 1.0;
        }
        // SAFETY: an active node is live (the `state` field's invariant) and,
        // as a hit with a surface, a buffer node; the constrained surface
        // lives as long as the constraint.
        let (dest_w, surf_w) = unsafe {
            (
                ffi::river_scene_buffer_get_dest_width(node as *mut ffi::wlr_scene_buffer),
                ffi::river_wlr_surface_get_width(surface),
            )
        };
        if dest_w > 0 && surf_w > 0 { surf_w as f64 / dest_w as f64 } else { 1.0 }
    }

    /// Clamp a relative motion `(dx, dy)` (layout px) from the pointer at
    /// layout `(lx, ly)` to the confining region. The surface position is
    /// taken from where the pointer actually is, not from the activation:
    /// the compositor can warp the pointer in between (`ccectl`, an
    /// absolute device, a constraint's own re-anchoring).
    pub fn confine(&mut self, lx: f64, ly: f64, dx: &mut f64, dy: &mut f64) {
        let PointerConstraintState::Active { node, .. } = self.state else { return };
        let Some((nx, ny, true)) = self.active_node_coords() else { return };
        let scale = self.surface_scale();
        let sx = (lx - nx as f64) * scale;
        let sy = (ly - ny as f64) * scale;
        if let Some((new_sx, new_sy)) = self.wlr_constraint.region_confine(sx, sy, sx + *dx * scale, sy + *dy * scale) {
            *dx = (new_sx - sx) / scale;
            *dy = (new_sy - sy) / scale;
            self.state = PointerConstraintState::Active { node, sx: new_sx, sy: new_sy };
        }
    }

    fn warp_to_hint_if_set(&self, seat: &Seat) {
        let Some((sx, sy)) = self.wlr_constraint.cursor_hint() else { return };
        if !self.is_active() {
            return;
        }
        let (lx, ly) = self.active_node_coords().map_or((0, 0), |(x, y, _)| (x, y));
        let scale = self.surface_scale();
        seat.cursor.wlr_cursor().warp(lx as f64 + sx / scale, ly as f64 + sy / scale);
        // Through the field, which is unset once `Seat::destroy` has taken
        // it: `wlr_seat_destroy` destroys the seat's constraints, and this
        // runs from their destroy handler.
        seat.wlr_seat().pointer_warp(sx, sy);
    }
}

unsafe fn river_wlr_seat_get_data_safe(wlr_seat: *mut ffi::wlr_seat) -> *mut Seat {
    if wlr_seat.is_null() {
        std::ptr::null_mut()
    } else {
        ffi::river_wlr_seat_get_data(wlr_seat) as *mut Seat
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let constraint_ptr = crate::container_of!(listener, PointerConstraint, destroy_listener) as *mut PointerConstraint;
    let constraint = &mut *constraint_ptr;
    // The seat through wlroots' pointer, not the store: this also runs while
    // the server tears down, with no window manager to take.
    let seat = river_wlr_seat_get_data_safe((*constraint.wlr_constraint.ptr()).seat);

    if constraint.is_active() {
        if !seat.is_null() {
            constraint.warp_to_hint_if_set(&*seat);
        }
        constraint.node_destroy_listener.disconnect();
    }

    constraint.destroy_listener.disconnect();
    constraint.commit_listener.disconnect();

    if !seat.is_null() && (*seat).cursor.constraint() == constraint_ptr {
        (*seat).cursor.set_constraint(std::ptr::null_mut());
    }

    constraint.wlr_constraint.take();
    let _boxed = Box::from_raw(constraint_ptr);
}

unsafe extern "C" fn handle_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let wm = &*crate::reentry::wm(crate::shared::server_ptr());
    let constraint_ptr = crate::container_of!(listener, PointerConstraint, commit_listener) as *mut PointerConstraint;
    let constraint = &mut *constraint_ptr;

    match constraint.state {
        PointerConstraintState::Active { sx, sy, .. } => {
            if !constraint.wlr_constraint.region_contains(sx, sy) {
                log::info!("deactivating pointer constraint, input region change left pointer outside");
                constraint.deactivate(wm);
            }
        }
        PointerConstraintState::Inactive => {
            if constraint.seat(wm).is_some_and(|seat| seat.cursor.constraint() == constraint_ptr) {
                constraint.maybe_activate(wm);
            }
        }
    }
}

unsafe extern "C" fn handle_node_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let constraint_ptr = crate::container_of!(listener, PointerConstraint, node_destroy_listener) as *mut PointerConstraint;
    log::info!("deactivating pointer constraint, scene node destroyed");
    // A scene node goes inside window-manager calls (a window unmapping),
    // so this must not take the window manager: the seat comes from
    // wlroots' pointer.
    let seat = river_wlr_seat_get_data_safe((*(*constraint_ptr).wlr_constraint.ptr()).seat);
    (*constraint_ptr).deactivate_with(seat.as_ref());
}
