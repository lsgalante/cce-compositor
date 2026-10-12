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
    pub wlr_constraint: *mut ffi::wlr_pointer_constraint_v1,
    pub state: PointerConstraintState,
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

impl PointerConstraint {
    /// A client asked to lock or confine the pointer on one of its
    /// surfaces. It takes effect while that surface has keyboard focus
    /// (`Seat::sync_pointer_constraint` hands it over on focus changes) and
    /// the pointer is over its region.
    pub unsafe fn create(wm: &mut crate::window_manager::WindowManager, wlr_constraint: *mut ffi::wlr_pointer_constraint_v1) -> *mut Self {
        let constraint = Box::into_raw(Box::new(Self {
            wlr_constraint,
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
        let focused = seat.wlr_seat.keyboard_focused_surface();
        if !focused.is_null() && focused == (*wlr_constraint).surface && seat.cursor.constraint.is_null() {
            seat.cursor.constraint = constraint;
            (*constraint).maybe_activate(wm);
        }

        constraint
    }

    /// The seat this constraint is for, if it is still in the store.
    unsafe fn seat<'a>(&self, wm: &'a crate::window_manager::WindowManager) -> Option<&'a Seat> {
        Seat::id_of((*self.wlr_constraint).seat).and_then(|sid| wm.seats.get(sid))
    }

    pub unsafe fn maybe_activate(&mut self, wm: &crate::window_manager::WindowManager) {
        let Some(seat) = self.seat(wm) else { return };
        if seat.cursor.constraint != self as *mut Self {
            return;
        }
        if matches!(self.state, PointerConstraintState::Active { .. }) {
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
            if result.surface != (*self.wlr_constraint).surface {
                return;
            }

            let sx = result.sx;
            let sy = result.sy;

            // Check if within the pixman region
            if ffi::pixman_region32_contains_point(&mut (*self.wlr_constraint).region, sx as i32, sy as i32, std::ptr::null_mut()) == 0 {
                return;
            }

            self.state = PointerConstraintState::Active {
                node: result.node,
                sx,
                sy,
            };

            let destroy_signal = ffi::river_scene_node_get_destroy_signal(result.node);
            self.node_destroy_listener.connect(destroy_signal, handle_node_destroy);

            log::info!("activating pointer constraint");
            ffi::wlr_pointer_constraint_v1_send_activated(self.wlr_constraint);
        }
    }

    /// Whether it is active and locks (rather than confines) the pointer.
    pub unsafe fn locks(&self) -> bool {
        matches!(self.state, PointerConstraintState::Active { .. })
            && (*self.wlr_constraint).type_ == ffi::wlr_pointer_constraint_v1_type_WLR_POINTER_CONSTRAINT_V1_LOCKED
    }

    pub unsafe fn deactivate(&mut self, wm: &crate::window_manager::WindowManager) {
        let seat = self.seat(wm);
        self.deactivate_with(seat);
    }

    /// `deactivate` for the paths that have no window manager to look the
    /// seat up in (a scene node or the constraint going away): `seat`, if
    /// any, gets the cursor hint warp.
    unsafe fn deactivate_with(&mut self, seat: Option<&Seat>) {
        if !matches!(self.state, PointerConstraintState::Active { .. }) {
            return;
        }

        if let Some(seat) = seat {
            self.warp_to_hint_if_set(seat);
        }

        self.state = PointerConstraintState::Inactive;
        self.node_destroy_listener.disconnect();
        ffi::wlr_pointer_constraint_v1_send_deactivated(self.wlr_constraint);
    }

    pub unsafe fn update_state(&mut self, wm: &crate::window_manager::WindowManager) {
        if !applies_now() {
            self.deactivate(wm);
            return;
        }
        self.maybe_activate(wm);

        if let PointerConstraintState::Active { node, sx, sy } = self.state {
            let Some(seat) = self.seat(wm) else { return };

            let mut lx = 0;
            let mut ly = 0;
            if !wlr_scene_node_coords_safe(node, &mut lx, &mut ly) {
                log::info!("deactivating pointer constraint, scene node disabled");
                self.deactivate(wm);
                return;
            }

            let scale = self.surface_scale(node);
            let warp_lx = lx as f64 + sx / scale;
            let warp_ly = ly as f64 + sy / scale;

            if !seat.cursor.wlr_cursor.warp(warp_lx, warp_ly) {
                log::info!("deactivating pointer constraint, could not warp cursor");
                self.deactivate(wm);
                return;
            }

            if ffi::pixman_region32_contains_point(&mut (*self.wlr_constraint).region, sx as i32, sy as i32, std::ptr::null_mut()) == 0 {
                log::info!("deactivating pointer constraint, cursor outside region despite warp");
                self.deactivate(wm);
            }
        }
    }

    /// Surface px per layout px for the surface drawn by `node`: a window
    /// can be drawn zoomed, where a layout px is not a surface px.
    unsafe fn surface_scale(&self, node: *mut ffi::wlr_scene_node) -> f64 {
        if node.is_null() {
            return 1.0;
        }
        let dest_w = ffi::river_scene_buffer_get_dest_width(node as *mut ffi::wlr_scene_buffer);
        let surf_w = ffi::river_wlr_surface_get_width((*self.wlr_constraint).surface);
        if dest_w > 0 && surf_w > 0 { surf_w as f64 / dest_w as f64 } else { 1.0 }
    }

    /// Clamp a relative motion `(dx, dy)` (layout px) from the pointer at
    /// layout `(lx, ly)` to the confining region. The surface position is
    /// taken from where the pointer actually is, not from the activation:
    /// the compositor can warp the pointer in between (`ccectl`, an
    /// absolute device, a constraint's own re-anchoring).
    pub unsafe fn confine(&mut self, lx: f64, ly: f64, dx: &mut f64, dy: &mut f64) {
        if let PointerConstraintState::Active { node, .. } = self.state {
            let (mut nx, mut ny) = (0, 0);
            if !wlr_scene_node_coords_safe(node, &mut nx, &mut ny) {
                return;
            }
            let scale = self.surface_scale(node);
            let sx = (lx - nx as f64) * scale;
            let sy = (ly - ny as f64) * scale;
            let mut new_sx = 0.0;
            let mut new_sy = 0.0;
            if ffi::wlr_region_confine(&mut (*self.wlr_constraint).region, sx, sy, sx + *dx * scale, sy + *dy * scale, &mut new_sx, &mut new_sy) {
                *dx = (new_sx - sx) / scale;
                *dy = (new_sy - sy) / scale;
                self.state = PointerConstraintState::Active { node, sx: new_sx, sy: new_sy };
            }
        }
    }

    unsafe fn warp_to_hint_if_set(&self, seat: &Seat) {
        if (*self.wlr_constraint).current.cursor_hint.enabled {
            if let PointerConstraintState::Active { node, .. } = self.state {
                let mut lx = 0;
                let mut ly = 0;
                wlr_scene_node_coords_safe(node, &mut lx, &mut ly);

                let sx = (*self.wlr_constraint).current.cursor_hint.x;
                let sy = (*self.wlr_constraint).current.cursor_hint.y;
                let scale = self.surface_scale(node);
                seat.cursor.wlr_cursor.warp(lx as f64 + sx / scale, ly as f64 + sy / scale);
                // Through the field, which is unset once `Seat::destroy`
                // has taken it: `wlr_seat_destroy` destroys the seat's
                // constraints, and this runs from their destroy handler.
                seat.wlr_seat.pointer_warp(sx, sy);
            }
        }
    }
}

unsafe fn river_wlr_seat_get_data_safe(wlr_seat: *mut ffi::wlr_seat) -> *mut Seat {
    if wlr_seat.is_null() {
        std::ptr::null_mut()
    } else {
        ffi::river_wlr_seat_get_data(wlr_seat) as *mut Seat
    }
}

unsafe fn wlr_scene_node_coords_safe(node: *mut ffi::wlr_scene_node, x: *mut i32, y: *mut i32) -> bool {
    if node.is_null() {
        false
    } else {
        ffi::wlr_scene_node_coords(node, x, y)
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let constraint_ptr = crate::container_of!(listener, PointerConstraint, destroy_listener) as *mut PointerConstraint;
    let constraint = &mut *constraint_ptr;
    // The seat through wlroots' pointer, not the store: this also runs while
    // the server tears down, with no window manager to take.
    let seat = river_wlr_seat_get_data_safe((*constraint.wlr_constraint).seat);

    if matches!(constraint.state, PointerConstraintState::Active { .. }) {
        if !seat.is_null() {
            constraint.warp_to_hint_if_set(&*seat);
        }
        constraint.node_destroy_listener.disconnect();
    }

    constraint.destroy_listener.disconnect();
    constraint.commit_listener.disconnect();

    if !seat.is_null() && (*seat).cursor.constraint == constraint_ptr {
        (*seat).cursor.constraint = std::ptr::null_mut();
    }

    let _boxed = Box::from_raw(constraint_ptr);
}

unsafe extern "C" fn handle_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let wm = &*crate::reentry::wm(crate::shared::server_ptr());
    let constraint_ptr = crate::container_of!(listener, PointerConstraint, commit_listener) as *mut PointerConstraint;
    let constraint = &mut *constraint_ptr;

    match constraint.state {
        PointerConstraintState::Active { sx, sy, .. } => {
            if ffi::pixman_region32_contains_point(&mut (*constraint.wlr_constraint).region, sx as i32, sy as i32, std::ptr::null_mut()) == 0 {
                log::info!("deactivating pointer constraint, input region change left pointer outside");
                constraint.deactivate(wm);
            }
        }
        PointerConstraintState::Inactive => {
            if constraint.seat(wm).is_some_and(|seat| seat.cursor.constraint == constraint_ptr) {
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
    let seat = river_wlr_seat_get_data_safe((*(*constraint_ptr).wlr_constraint).seat);
    (*constraint_ptr).deactivate_with(seat.as_ref());
}
