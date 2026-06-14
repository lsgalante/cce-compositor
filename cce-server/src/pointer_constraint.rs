use crate::ffi;
use crate::seat::{Seat, Focus};
use crate::server::{WlListener, wl_listener_remove, wl_signal_add};

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
    pub destroy_listener: ffi::wl_listener,
    pub commit_listener: ffi::wl_listener,
    pub node_destroy_listener: ffi::wl_listener,
}

impl PointerConstraint {
    pub unsafe fn create(wlr_constraint: *mut ffi::wlr_pointer_constraint_v1) -> *mut Self {
        let constraint = Box::into_raw(Box::new(Self {
            wlr_constraint,
            state: PointerConstraintState::Inactive,
            destroy_listener: std::mem::zeroed(),
            commit_listener: std::mem::zeroed(),
            node_destroy_listener: std::mem::zeroed(),
        }));

        (*wlr_constraint).data = constraint as *mut _;

        let destroy_listener_ptr = &mut (*constraint).destroy_listener as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_listener_ptr).notify = Some(handle_destroy);
        
        let commit_listener_ptr = &mut (*constraint).commit_listener as *mut ffi::wl_listener as *mut WlListener;
        (*commit_listener_ptr).notify = Some(handle_commit);

        let node_destroy_listener_ptr = &mut (*constraint).node_destroy_listener as *mut ffi::wl_listener as *mut WlListener;
        (*node_destroy_listener_ptr).notify = Some(handle_node_destroy);

        wl_signal_add(&mut (*wlr_constraint).events.destroy, &mut (*constraint).destroy_listener);
        let commit_signal = ffi::river_wlr_surface_get_commit_signal((*wlr_constraint).surface);
        wl_signal_add(commit_signal, &mut (*constraint).commit_listener);

        let seat = river_wlr_seat_get_data_safe((*wlr_constraint).seat);
        if !seat.is_null() {
            if let Focus::LayerSurface(surface) = (*seat).focused {
                if surface == (*wlr_constraint).surface {
                    (*seat).cursor.constraint = constraint;
                    (*constraint).maybe_activate();
                }
            }
        }

        constraint
    }

    pub unsafe fn maybe_activate(&mut self) {
        let seat = river_wlr_seat_get_data_safe((*self.wlr_constraint).seat);
        if seat.is_null() {
            return;
        }

        if (*seat).cursor.constraint != self {
            return;
        }

        if matches!(self.state, PointerConstraintState::Active { .. }) {
            return;
        }

        // Retrieve cursor coordinates
        let cx = (*seat).cursor.x();
        let cy = (*seat).cursor.y();

        let server = (*seat).server;
        if let Some(result) = (*server).scene.at(cx, cy) {
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
            wl_signal_add(destroy_signal, &mut self.node_destroy_listener);

            log::info!("activating pointer constraint");
            ffi::wlr_pointer_constraint_v1_send_activated(self.wlr_constraint);
        }
    }

    pub unsafe fn deactivate(&mut self) {
        if !matches!(self.state, PointerConstraintState::Active { .. }) {
            return;
        }

        self.warp_to_hint_if_set();

        self.state = PointerConstraintState::Inactive;
        wl_listener_remove(&mut self.node_destroy_listener);
        ffi::wlr_pointer_constraint_v1_send_deactivated(self.wlr_constraint);
    }

    pub unsafe fn update_state(&mut self) {
        self.maybe_activate();

        if let PointerConstraintState::Active { node, sx, sy } = self.state {
            let seat = river_wlr_seat_get_data_safe((*self.wlr_constraint).seat);
            if seat.is_null() {
                return;
            }

            let mut lx = 0;
            let mut ly = 0;
            // Get coordinates of scene node relative to layout
            // In wlroots, scene node coords are retrieved
            if !wlr_scene_node_coords_safe(node, &mut lx, &mut ly) {
                log::info!("deactivating pointer constraint, scene node disabled");
                self.deactivate();
                return;
            }

            let warp_lx = lx as f64 + sx;
            let warp_ly = ly as f64 + sy;

            if !ffi::wlr_cursor_warp((*seat).cursor.wlr_cursor, std::ptr::null_mut(), warp_lx, warp_ly) {
                log::info!("deactivating pointer constraint, could not warp cursor");
                self.deactivate();
                return;
            }

            if ffi::pixman_region32_contains_point(&mut (*self.wlr_constraint).region, sx as i32, sy as i32, std::ptr::null_mut()) == 0 {
                log::info!("deactivating pointer constraint, cursor outside region despite warp");
                self.deactivate();
            }
        }
    }

    pub unsafe fn confine(&mut self, dx: &mut f64, dy: &mut f64) {
        if let PointerConstraintState::Active { sx, sy, node } = self.state {
            let mut new_sx = 0.0;
            let mut new_sy = 0.0;

            // Call wlroots / pixman confine helper
            // river does: wlr.region.confine(region, sx, sy, sx + dx, sy + dy, &new_sx, &new_sy)
            // If the region contains the line segment or we can confine it.
            // Let's implement confinement using wlroots wlr_region_confine (or similar, or custom simple confinement).
            // Actually, wlroots has: wlr_region_confine(struct pixman_region32 *region, double sx1, double sy1, double sx2, double sy2, double *ak_sx, double *ak_sy)
            // Let's call it:
            if ffi::wlr_region_confine(&mut (*self.wlr_constraint).region, sx, sy, sx + *dx, sy + *dy, &mut new_sx, &mut new_sy) {
                *dx = new_sx - sx;
                *dy = new_sy - sy;

                self.state = PointerConstraintState::Active {
                    node,
                    sx: new_sx,
                    sy: new_sy,
                };
            }
        }
    }

    unsafe fn warp_to_hint_if_set(&mut self) {
        let seat = river_wlr_seat_get_data_safe((*self.wlr_constraint).seat);
        if seat.is_null() {
            return;
        }

        if (*self.wlr_constraint).current.cursor_hint.enabled {
            if let PointerConstraintState::Active { node, .. } = self.state {
                let mut lx = 0;
                let mut ly = 0;
                wlr_scene_node_coords_safe(node, &mut lx, &mut ly);

                let sx = (*self.wlr_constraint).current.cursor_hint.x;
                let sy = (*self.wlr_constraint).current.cursor_hint.y;
                ffi::wlr_cursor_warp((*seat).cursor.wlr_cursor, std::ptr::null_mut(), lx as f64 + sx, ly as f64 + sy);
                ffi::wlr_seat_pointer_warp((*seat).wlr_seat, sx, sy);
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
    let seat = river_wlr_seat_get_data_safe((*constraint.wlr_constraint).seat);

    if matches!(constraint.state, PointerConstraintState::Active { .. }) {
        constraint.warp_to_hint_if_set();
        wl_listener_remove(&mut constraint.node_destroy_listener);
    }

    wl_listener_remove(&mut constraint.destroy_listener);
    wl_listener_remove(&mut constraint.commit_listener);

    if !seat.is_null() && (*seat).cursor.constraint == constraint_ptr {
        (*seat).cursor.constraint = std::ptr::null_mut();
    }

    let _boxed = Box::from_raw(constraint_ptr);
}

unsafe extern "C" fn handle_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let constraint_ptr = crate::container_of!(listener, PointerConstraint, commit_listener) as *mut PointerConstraint;
    let constraint = &mut *constraint_ptr;
    let seat = river_wlr_seat_get_data_safe((*constraint.wlr_constraint).seat);

    match constraint.state {
        PointerConstraintState::Active { sx, sy, .. } => {
            if ffi::pixman_region32_contains_point(&mut (*constraint.wlr_constraint).region, sx as i32, sy as i32, std::ptr::null_mut()) == 0 {
                log::info!("deactivating pointer constraint, input region change left pointer outside");
                constraint.deactivate();
            }
        }
        PointerConstraintState::Inactive => {
            if !seat.is_null() && (*seat).cursor.constraint == constraint_ptr {
                constraint.maybe_activate();
            }
        }
    }
}

unsafe extern "C" fn handle_node_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let constraint_ptr = crate::container_of!(listener, PointerConstraint, node_destroy_listener) as *mut PointerConstraint;
    log::info!("deactivating pointer constraint, scene node destroyed");
    (*constraint_ptr).deactivate();
}
