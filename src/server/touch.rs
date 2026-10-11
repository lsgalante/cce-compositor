//! Touchscreen input: where each finger goes, and the gestures the
//! compositor keeps for itself.
//!
//! **Routing.** A finger is routed once, at touch-down (`TouchRoute`): a
//! surface whose client bound `wl_touch` gets real touch events; anything
//! else gets the pointer, a left button held where the finger is. The
//! pointer's press waits until the finger has moved past `TAP_SLOP` (then
//! it is pressed where the finger went down, and the drag follows) or has
//! lifted (a tap: press and release). Nothing is lost by the wait — the
//! press lands at the down point either way — and it is what lets a second
//! or third finger turn the touch into a gesture with no half-made click
//! to take back.
//!
//! **Gestures** (`Claim`). While one is live, every finger is the
//! compositor's; fingers already given to clients are cancelled
//! (`wl_touch.cancel`) when it begins.
//!
//! - **Edge swipe**: one finger landing within `EDGE_ZONE` of a screen edge
//!   that has an `edge_<side>` bind in input.kdl, and moving `EDGE_FIRE`
//!   inward, fires that bind once. A finger that goes along or back out
//!   instead is handed to the normal route, from its down point; one that
//!   lifts without moving is delivered as the tap it was.
//! - **Desk pan and zoom**: a finger dragged on the bare desk (in normal
//!   mode — in overview a one-finger drag there is the selection band)
//!   pans the desk under it, and two fingers that start on the desk pan it
//!   and pinch-zoom about their midpoint; the lift coasts, like a trackpad
//!   pan's.
//! - **Three and four fingers**, anywhere: once the fingers have moved
//!   enough to tell (`decide`), a swipe runs through the touchpad's swipe
//!   handling — the same `swipe3_*` / `swipe4_*` binds, lean, repeat steps
//!   and focus aim — and a pinch fires the `pinch3_*` / `pinch4_*` binds.
//!   **A touchscreen swipe is natural**: the desk follows the fingers, so
//!   the bind fires for the direction the camera goes, which is opposite
//!   the fingers — dragging the desk right reveals what is to the left, and
//!   fires `swipe3_left`. On a touchpad the bind is the direction the
//!   fingers move.

use crate::cursor::{gesture_bind, gesture_mods, pinch_hits, run_gesture_action, Cursor};
use crate::ffi;
use crate::scene_node_data::SceneNodeDataVal;
use crate::seat::Focus;

/// Travel (layout px) a finger may wander and still be a tap.
pub const TAP_SLOP: f64 = 10.0;
/// How near a screen edge a finger must land to start an edge swipe.
pub const EDGE_ZONE: f64 = 24.0;
/// Inward travel that fires an edge swipe's bind.
pub const EDGE_FIRE: f64 = 60.0;
/// Centroid travel that decides three or more fingers are swiping.
pub const DECIDE_TRAVEL: f64 = 24.0;
/// Spread change (as a ratio) that decides they are pinching.
pub const DECIDE_SCALE: f64 = 1.15;

const BTN_LEFT: u32 = 0x110;

/// One finger on a touchscreen (`Cursor::touch_points`).
#[derive(Clone, Copy, Debug)]
pub struct TouchPoint {
    pub lx: f64,
    pub ly: f64,
    pub route: TouchRoute,
}

/// Where a finger's events go, fixed at touch-down for its lifetime — a
/// finger that slides onto another window keeps talking to the one it went
/// down on, exactly like a held button's implicit grab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TouchRoute {
    /// A client that bound `wl_touch` gets real touch events. Positions are
    /// surface-local through the frame frozen at down: `origin` is the
    /// surface's layout origin and `scale` its surface-per-layout-pixel
    /// ratio, the same mapping the pointer's implicit grab uses
    /// (`grab_origin` / `grab_scale`).
    Client { origin: (f64, f64), scale: f64 },
    /// This finger drives the pointer. Everything that is not a
    /// touch-capable client goes this way — the desktop, and every press
    /// the compositor itself handles (overview, the adjust-mode handles,
    /// drag-selection). `pressed` stays false until the finger has moved
    /// past `TAP_SLOP` or lifted (see the module comment); `on_desk` is a
    /// down on the bare desk, which a drag or a second finger turns into
    /// a desk pan.
    Pointer { start: (f64, f64), pressed: bool, on_desk: bool },
    /// Owned by the live gesture (`Cursor::touch_claim`).
    Claimed,
    /// Swallowed until it lifts: a second finger while another drives the
    /// pointer, or a touch while a real button is held.
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    /// The chord's spelling (`edge_left`) without the prefix.
    pub fn name(self) -> &'static str {
        match self {
            Edge::Left => "left",
            Edge::Right => "right",
            Edge::Top => "top",
            Edge::Bottom => "bottom",
        }
    }

    /// Unit vector pointing away from the edge, into the screen.
    pub fn inward(self) -> (f64, f64) {
        match self {
            Edge::Left => (1.0, 0.0),
            Edge::Right => (-1.0, 0.0),
            Edge::Top => (0.0, 1.0),
            Edge::Bottom => (0.0, -1.0),
        }
    }
}

/// Where an edge swipe stands after the finger has moved by `(dx, dy)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeProgress {
    /// Not far enough to tell.
    Undecided,
    /// Inward past `EDGE_FIRE`, and more inward than along the edge.
    Fire,
    /// Past the slop, but along the edge or back out: not an edge swipe.
    NotEdge,
}

pub fn edge_progress(edge: Edge, dx: f64, dy: f64) -> EdgeProgress {
    let (ix, iy) = edge.inward();
    let inward = dx * ix + dy * iy;
    let along = (dx * iy - dy * ix).abs();
    if inward >= EDGE_FIRE && inward > along {
        EdgeProgress::Fire
    } else if dx.hypot(dy) >= TAP_SLOP && inward < along {
        EdgeProgress::NotEdge
    } else {
        EdgeProgress::Undecided
    }
}

/// The fingers as one shape: their centroid and their spread (mean
/// distance from it), which is what a pan follows and a pinch measures.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shape {
    pub centroid: (f64, f64),
    pub spread: f64,
    pub count: usize,
}

pub fn shape(points: &[(f64, f64)]) -> Shape {
    if points.is_empty() {
        return Shape::default();
    }
    let n = points.len() as f64;
    let cx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let cy = points.iter().map(|p| p.1).sum::<f64>() / n;
    let spread = points.iter().map(|p| (p.0 - cx).hypot(p.1 - cy)).sum::<f64>() / n;
    Shape { centroid: (cx, cy), spread, count: points.len() }
}

/// A gesture's motion, accumulated across finger sets: a finger landing or
/// lifting moves the centroid and the spread without anything having moved,
/// so a change of set rebases (`rebase`) instead of counting as travel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Track {
    last: Shape,
    /// Centroid travel since the gesture began.
    pub travel: (f64, f64),
    /// Product of the spread's ratios since the gesture began.
    pub scale: f64,
    /// The most fingers seen at once: what a swipe or pinch bind matches.
    pub fingers: usize,
}

impl Track {
    pub fn new(s: Shape) -> Self {
        Track { last: s, travel: (0.0, 0.0), scale: 1.0, fingers: s.count }
    }

    pub fn rebase(&mut self, s: Shape) {
        self.last = s;
        self.fingers = self.fingers.max(s.count);
    }

    /// One sample of the current fingers: the centroid's motion since the
    /// last one. A different finger count is a rebase, not motion.
    pub fn step(&mut self, s: Shape) -> (f64, f64) {
        if s.count != self.last.count {
            self.rebase(s);
            return (0.0, 0.0);
        }
        let d = (s.centroid.0 - self.last.centroid.0, s.centroid.1 - self.last.centroid.1);
        self.travel.0 += d.0;
        self.travel.1 += d.1;
        // A single finger has no spread to compare, and two fingers almost
        // on top of each other would turn a pixel of jitter into a zoom.
        if s.count >= 2 && self.last.spread > 4.0 && s.spread > 4.0 {
            self.scale *= s.spread / self.last.spread;
        }
        self.last = s;
        d
    }
}

/// What three or more fingers are doing, once it can be told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiKind {
    Undecided,
    Swipe,
    /// `fired`: its bind has run; the rest of the gesture is ignored.
    Pinch { fired: bool },
}

pub fn decide(track: &Track) -> MultiKind {
    if track.scale >= DECIDE_SCALE || track.scale <= 1.0 / DECIDE_SCALE {
        MultiKind::Pinch { fired: false }
    } else if track.travel.0.hypot(track.travel.1) >= DECIDE_TRAVEL {
        MultiKind::Swipe
    } else {
        MultiKind::Undecided
    }
}

/// The finger count a bind matches for a gesture tracked with `fingers`:
/// binds name two to four, and a fifth finger is still a four-finger swipe.
fn bind_fingers(fingers: usize) -> u32 {
    fingers.clamp(3, 4) as u32
}

/// The gesture the compositor holds (`Cursor::touch_claim`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Claim {
    None,
    /// One finger from a bound edge (see the module comment).
    Edge { id: i32, edge: Edge, start: (f64, f64), fired: bool },
    /// The desk follows the fingers. `start_zoom` is the zoom the pinch
    /// scales; `vel` (virtual px/s per axis) and `last_ms` feed the coast.
    Desk { track: Track, start_zoom: f64, vel: [f64; 2], last_ms: u32 },
    /// Three or more fingers.
    Multi { track: Track, kind: MultiKind },
}

impl Default for Claim {
    fn default() -> Self {
        Claim::None
    }
}

impl Cursor {
    /// Decide where a new finger at `lx, ly` goes. In window-adjust mode
    /// (overview, or Super held) every press is the compositor's to spend
    /// — the handles, a window body's drag, the rubber band — and those live
    /// on the pointer path, so the finger becomes the pointer even over a
    /// touch-capable window.
    unsafe fn touch_route_at(&self, lx: f64, ly: f64) -> TouchRoute {
        let seat = &*self.seat;
        let server = seat.server;
        let result = crate::shared::scene().at(lx, ly);
        if let Some(ref result) = result {
            // Locked, only the lock surface may hear a finger, as only it
            // may hear the pointer (`passthrough`).
            let locked = (*server).lock_manager.state != crate::lock_manager::LockState::Unlocked;
            let lock_ok = !locked || matches!(result.data, SceneNodeDataVal::LockSurface(_));
            if lock_ok
                && !result.surface.is_null()
                && !crate::shared::window_adjust_active()
                && ffi::wlr_surface_accepts_touch(result.surface, seat.wlr_seat)
            {
                // The implicit grab's frame (see the button path): the
                // surface's scene buffer may be drawn scaled, so map through
                // its destination size rather than assume 1:1.
                let mut scale = 1.0;
                if !result.node.is_null() {
                    let dest_w = ffi::river_scene_buffer_get_dest_width(result.node as *mut ffi::wlr_scene_buffer);
                    let surf_w = ffi::river_wlr_surface_get_width(result.surface);
                    if dest_w > 0 && surf_w > 0 {
                        scale = surf_w as f64 / dest_w as f64;
                    }
                }
                let origin = (lx - result.sx / scale, ly - result.sy / scale);
                return TouchRoute::Client { origin, scale };
            }
        }
        // The pointer is singular: a second finger cannot also drive it, and
        // a real button held (or a seat op it started) owns it already.
        let pointer_busy = self.touch_points.values().any(|p| matches!(p.route, TouchRoute::Pointer { .. }))
            || !self.pressed.is_empty()
            || seat.op.is_some();
        if pointer_busy {
            return TouchRoute::Ignored;
        }
        let on_desk = match result {
            None => (*server).lock_manager.state == crate::lock_manager::LockState::Unlocked,
            Some(r) => matches!(r.data, SceneNodeDataVal::Window(w) if (*w).is_wallpaper()),
        };
        TouchRoute::Pointer { start: (lx, ly), pressed: false, on_desk }
    }

    pub unsafe fn touch_down(&mut self, wm: &mut crate::window_manager::WindowManager, id: i32, lx: f64, ly: f64, time_msec: u32) {
        (*self.seat).handle_activity();
        // A touch is deliberate input, like a press (see `Seat::focus`).
        wm.startup_input_seen = true;
        self.hide_for_touch();
        // A device that reuses a live id without lifting it first has lost
        // the up; finish the old point so its button or client is released.
        if self.touch_points.contains_key(&id) {
            log::warn!("touch: down for live touch id {id}; lifting the old point first");
            self.touch_up(wm, id, time_msec);
        }

        match self.touch_claim {
            // A second finger is not an edge swipe: hand the first back
            // before routing this one.
            Claim::Edge { .. } => self.edge_release(wm, time_msec),
            Claim::Desk { track, .. } => {
                self.touch_points.insert(id, TouchPoint { lx, ly, route: TouchRoute::Claimed });
                // Three fingers on a desk that has not moved yet were a
                // three-finger gesture landing one finger at a time.
                let still = track.travel.0.hypot(track.travel.1) < TAP_SLOP && (track.scale - 1.0).abs() < 0.05;
                if still && self.touch_points.len() >= 3 {
                    self.touch_claim = Claim::Multi { track: Track::new(self.claimed_shape()), kind: MultiKind::Undecided };
                } else {
                    self.claim_rebase();
                }
                return;
            }
            Claim::Multi { .. } => {
                self.touch_points.insert(id, TouchPoint { lx, ly, route: TouchRoute::Claimed });
                self.claim_rebase();
                return;
            }
            Claim::None => {}
        }

        let already = self.touch_points.len();
        if already == 0 {
            if let Some(edge) = self.bound_edge_at(lx, ly) {
                log::debug!("touch: down id={id} in the {} edge zone", edge.name());
                self.touch_points.insert(id, TouchPoint { lx, ly, route: TouchRoute::Claimed });
                self.touch_claim = Claim::Edge { id, edge, start: (lx, ly), fired: false };
                return;
            }
        }
        let pointer_pressed = self.touch_points.values().any(|p| matches!(p.route, TouchRoute::Pointer { pressed: true, .. }));
        if already + 1 >= 3 && !pointer_pressed {
            self.claim_all(wm);
            self.touch_points.insert(id, TouchPoint { lx, ly, route: TouchRoute::Claimed });
            self.touch_claim = Claim::Multi { track: Track::new(self.claimed_shape()), kind: MultiKind::Undecided };
            log::debug!("touch: {} fingers claimed for a gesture", self.touch_points.len());
            return;
        }
        if already == 1 {
            let first_on_desk = self
                .touch_points
                .values()
                .any(|p| matches!(p.route, TouchRoute::Pointer { pressed: false, on_desk: true, .. }));
            if first_on_desk {
                self.claim_all(wm);
                self.touch_points.insert(id, TouchPoint { lx, ly, route: TouchRoute::Claimed });
                self.start_desk();
                return;
            }
        }
        self.touch_begin(wm, id, lx, ly, time_msec);
    }

    /// Arm the on-screen keyboard (`osk.rs`) when the finger is on an
    /// app's window — not the board itself, a layer surface, nor the bar.
    unsafe fn note_touch_for_osk(&mut self, lx: f64, ly: f64) {
        let Some(result) = crate::shared::scene().at(lx, ly) else { return };
        if let SceneNodeDataVal::Window(window) = result.data {
            if !(*window).is_status_bar() && !(*window).is_wallpaper() {
                (*self.seat).relay.osk.note_touch();
            }
        }
    }

    /// Route a finger normally (`touch_route_at`) and deliver its down.
    unsafe fn touch_begin(&mut self, wm: &mut crate::window_manager::WindowManager, id: i32, lx: f64, ly: f64, time_msec: u32) {
        let server = (*self.seat).server;
        let route = self.touch_route_at(lx, ly);
        self.touch_points.insert(id, TouchPoint { lx, ly, route });
        log::debug!("touch: down id={id} at ({lx:.0}, {ly:.0}) -> {route:?}");

        match route {
            TouchRoute::Client { origin, scale } => {
                crate::cursor::press_dismissals(wm, server, lx, ly);
                let Some(result) = crate::shared::scene().at(lx, ly) else { return };
                // Focus as a click would (`handle_button`).
                let seat = &mut *self.seat;
                match result.data {
                    SceneNodeDataVal::Window(window) => {
                        if !(*window).is_status_bar() && !(*window).is_wallpaper() {
                            seat.focus(wm, Focus::Window(window));
                            seat.relay.osk.note_touch();
                        }
                    }
                    SceneNodeDataVal::LayerSurface(layer_surface) => {
                        if crate::cursor::layer_takes_click_focus(layer_surface) {
                            seat.focus(wm, Focus::LayerSurface(result.surface));
                        }
                    }
                    _ => {}
                }
                ffi::wlr_seat_touch_notify_down(
                    seat.wlr_seat,
                    result.surface,
                    time_msec,
                    id,
                    (lx - origin.0) * scale,
                    (ly - origin.1) * scale,
                );
            }
            // Hover now; the press waits (see `TouchRoute::Pointer`).
            TouchRoute::Pointer { .. } => self.warp_to(wm, lx, ly),
            TouchRoute::Claimed | TouchRoute::Ignored => {}
        }
    }

    pub unsafe fn touch_motion(&mut self, wm: &mut crate::window_manager::WindowManager, id: i32, lx: f64, ly: f64, time_msec: u32) {
        (*self.seat).handle_activity();
        let Some(point) = self.touch_points.get_mut(&id) else { return };
        point.lx = lx;
        point.ly = ly;
        let route = point.route;
        match route {
            TouchRoute::Claimed => self.claim_motion(wm, time_msec),
            TouchRoute::Client { origin, scale } => {
                ffi::wlr_seat_touch_notify_motion(
                    (*self.seat).wlr_seat,
                    time_msec,
                    id,
                    (lx - origin.0) * scale,
                    (ly - origin.1) * scale,
                );
                // A touch drag's icon follows the finger.
                self.update_drag_icons();
            }
            TouchRoute::Pointer { start, pressed: false, on_desk } => {
                if (lx - start.0).hypot(ly - start.1) < TAP_SLOP {
                    return;
                }
                let normal = crate::shared::mode() == crate::window_manager::WindowManagerMode::Normal;
                if on_desk && normal && self.touch_points.len() == 1 {
                    // A drag on the bare desk pans it. The track starts at
                    // the down point, so the desk catches up with the
                    // finger's whole travel, not just what follows the slop.
                    self.touch_points.get_mut(&id).unwrap().route = TouchRoute::Claimed;
                    let mut track = Track::new(shape(&[start]));
                    track.fingers = 1;
                    self.touch_claim = Claim::Desk {
                        track,
                        start_zoom: wm.desk_zoom,
                        vel: [0.0, 0.0],
                        last_ms: 0,
                    };
                    wm.stop_panning_animation();
                    self.claim_motion(wm, time_msec);
                    return;
                }
                // A drag: press where the finger went down, then follow it.
                self.touch_points.get_mut(&id).unwrap().route = TouchRoute::Pointer { start, pressed: true, on_desk };
                self.warp_to(wm, start.0, start.1);
                self.inject_button(wm, BTN_LEFT, true);
                self.warp_to(wm, lx, ly);
            }
            TouchRoute::Pointer { pressed: true, .. } => self.warp_to(wm, lx, ly),
            TouchRoute::Ignored => {}
        }
    }

    pub unsafe fn touch_up(&mut self, wm: &mut crate::window_manager::WindowManager, id: i32, time_msec: u32) {
        (*self.seat).handle_activity();
        let Some(point) = self.touch_points.remove(&id) else { return };
        match point.route {
            TouchRoute::Claimed => self.claim_lift(wm, id, point, time_msec, false),
            TouchRoute::Client { .. } => {
                // A field that starts editing on the release is still this
                // touch's (`osk.rs`).
                self.note_touch_for_osk(point.lx, point.ly);
                ffi::wlr_seat_touch_notify_up((*self.seat).wlr_seat, time_msec, id);
            }
            TouchRoute::Pointer { start, pressed: false, .. } => {
                // A tap.
                self.note_touch_for_osk(start.0, start.1);
                self.warp_to(wm, start.0, start.1);
                self.inject_button(wm, BTN_LEFT, true);
                self.inject_button(wm, BTN_LEFT, false);
            }
            TouchRoute::Pointer { pressed: true, .. } => self.inject_button(wm, BTN_LEFT, false),
            TouchRoute::Ignored => {}
        }
    }

    /// The device gave up on a finger (a palm, a gesture the kernel took).
    /// A client is told `wl_touch.cancel`, which voids its whole sequence; a
    /// finger driving the pointer releases the button — a cancelled drag
    /// still drops where it was, but a held button must never be left
    /// behind — and one that never pressed clicks nothing.
    pub unsafe fn touch_cancel(&mut self, wm: &mut crate::window_manager::WindowManager, id: i32) {
        (*self.seat).handle_activity();
        let Some(point) = self.touch_points.remove(&id) else { return };
        match point.route {
            TouchRoute::Claimed => self.claim_lift(wm, id, point, crate::util::msec_timestamp(), true),
            TouchRoute::Client { .. } => ffi::river_wlr_seat_touch_cancel_point((*self.seat).wlr_seat, id),
            TouchRoute::Pointer { pressed: true, .. } => self.inject_button(wm, BTN_LEFT, false),
            TouchRoute::Pointer { pressed: false, .. } | TouchRoute::Ignored => {}
        }
    }

    pub unsafe fn touch_frame(&mut self) {
        // The emulated pointer frames each of its own events; only touch
        // clients are waiting on this one.
        if self.touch_points.values().any(|p| matches!(p.route, TouchRoute::Client { .. }))
            || ffi::wlr_seat_touch_num_points((*self.seat).wlr_seat) > 0
        {
            ffi::wlr_seat_touch_notify_frame((*self.seat).wlr_seat);
        }
    }

    // ── Gestures ────────────────────────────────────────────────────────

    /// Take every finger already down for a gesture: clients hear
    /// `wl_touch.cancel`, and a pointer finger that never pressed is simply
    /// dropped (the reason the press waits).
    unsafe fn claim_all(&mut self, wm: &mut crate::window_manager::WindowManager) {
        let seat = (*self.seat).wlr_seat;
        let ids: Vec<i32> = self.touch_points.keys().copied().collect();
        for id in ids {
            let point = self.touch_points.get_mut(&id).unwrap();
            let route = std::mem::replace(&mut point.route, TouchRoute::Claimed);
            match route {
                TouchRoute::Client { .. } => ffi::river_wlr_seat_touch_cancel_point(seat, id),
                TouchRoute::Pointer { pressed: true, .. } => self.inject_button(wm, BTN_LEFT, false),
                _ => {}
            }
        }
    }

    fn claimed_shape(&self) -> Shape {
        let points: Vec<(f64, f64)> = self
            .touch_points
            .values()
            .filter(|p| p.route == TouchRoute::Claimed)
            .map(|p| (p.lx, p.ly))
            .collect();
        shape(&points)
    }

    fn claim_rebase(&mut self) {
        let s = self.claimed_shape();
        match &mut self.touch_claim {
            Claim::Desk { track, .. } | Claim::Multi { track, .. } => track.rebase(s),
            _ => {}
        }
    }

    unsafe fn start_desk(&mut self) {
        let wm = &mut (*crate::reentry::wm((*self.seat).server));
        wm.stop_panning_animation();
        self.touch_claim = Claim::Desk {
            track: Track::new(self.claimed_shape()),
            start_zoom: wm.desk_zoom,
            vel: [0.0, 0.0],
            last_ms: 0,
        };
        log::debug!("touch: desk gesture with {} fingers", self.touch_points.len());
    }

    unsafe fn claim_motion(&mut self, wm: &mut crate::window_manager::WindowManager, time_msec: u32) {
        let s = self.claimed_shape();
        let server = (*self.seat).server;
        match self.touch_claim {
            Claim::None => {}
            Claim::Edge { id, edge, start, fired } => {
                if fired {
                    return;
                }
                let Some(p) = self.touch_points.get(&id) else { return };
                match edge_progress(edge, p.lx - start.0, p.ly - start.1) {
                    EdgeProgress::Undecided => {}
                    EdgeProgress::NotEdge => self.edge_release(wm, time_msec),
                    EdgeProgress::Fire => {
                        self.touch_claim = Claim::Edge { id, edge, start, fired: true };
                        let mods = gesture_mods(&*self.seat);
                        if let Some((action, command)) = gesture_bind("edge", 1, mods, |d| d == edge.name()) {
                            log::info!("touch: edge_{} fired {action:?}", edge.name());
                            run_gesture_action(wm, action, command.as_deref());
                        }
                    }
                }
            }
            Claim::Desk { mut track, start_zoom, mut vel, mut last_ms } => {
                let d = track.step(s);
                if d != (0.0, 0.0) {
                    // The desk follows the fingers: the camera goes the
                    // other way, in virtual units.
                    let step = (-d.0 / wm.desk_zoom, -d.1 / wm.desk_zoom);
                    wm.queue_pan(step.0, step.1);
                    let dt = time_msec.wrapping_sub(last_ms).clamp(4, 100) as f64 / 1000.0;
                    for (axis, v) in [step.0, step.1].into_iter().enumerate() {
                        let sample = v / dt;
                        vel[axis] = if last_ms == 0 { sample } else { vel[axis] * 0.65 + sample * 0.35 };
                    }
                    last_ms = time_msec;
                    wm.pan_finger_v = vel;
                }
                if s.count >= 2 && track.scale != 1.0 {
                    let zoom = crate::policy::camera::pinch_zoom(start_zoom, track.scale);
                    let (ax, ay) = output_local(server, s.centroid);
                    wm.queue_pinch(zoom, ax, ay);
                }
                self.touch_claim = Claim::Desk { track, start_zoom, vel, last_ms };
            }
            Claim::Multi { mut track, kind } => {
                let d = track.step(s);
                let fingers = bind_fingers(track.fingers);
                let kind = match kind {
                    MultiKind::Undecided => match decide(&track) {
                        MultiKind::Swipe => {
                            log::info!("touch: {fingers}-finger swipe");
                            // Natural: the camera goes against the fingers.
                            self.touch_swipe(wm, "begin", fingers, 0.0, 0.0);
                            self.touch_swipe(wm, "update", fingers, -track.travel.0, -track.travel.1);
                            MultiKind::Swipe
                        }
                        MultiKind::Pinch { .. } => {
                            log::info!("touch: {fingers}-finger pinch");
                            self.touch_pinch(fingers, track.scale)
                        }
                        MultiKind::Undecided => MultiKind::Undecided,
                    },
                    MultiKind::Swipe => {
                        if d != (0.0, 0.0) {
                            self.touch_swipe(wm, "update", fingers, -d.0, -d.1);
                        }
                        MultiKind::Swipe
                    }
                    MultiKind::Pinch { fired: false } => self.touch_pinch(fingers, track.scale),
                    fired @ MultiKind::Pinch { fired: true } => fired,
                };
                self.touch_claim = Claim::Multi { track, kind };
            }
        }
    }

    /// A claimed finger lifted (or was cancelled). The gesture ends with
    /// its last finger.
    unsafe fn claim_lift(&mut self, wm: &mut crate::window_manager::WindowManager, id: i32, point: TouchPoint, time_msec: u32, cancelled: bool) {
        match self.touch_claim {
            Claim::None => {}
            Claim::Edge { id: edge_id, start, fired, .. } => {
                if edge_id != id {
                    return;
                }
                self.touch_claim = Claim::None;
                // Lifted where it landed: it was a tap in the edge zone (the
                // status bar's, say), so deliver it as one, late.
                if !fired && !cancelled && (point.lx - start.0).hypot(point.ly - start.1) < TAP_SLOP {
                    self.touch_begin(wm, id, start.0, start.1, time_msec);
                    self.touch_up(wm, id, time_msec);
                }
            }
            Claim::Desk { vel, last_ms, .. } => {
                if self.touch_points.values().any(|p| p.route == TouchRoute::Claimed) {
                    self.claim_rebase();
                    return;
                }
                self.touch_claim = Claim::None;
                wm.pan_finger_v = [0.0, 0.0];
                // Fling on the last velocity unless the fingers had come to
                // rest first, as a trackpad pan's lift does (`handle_axis`).
                let resting = last_ms == 0 || time_msec.wrapping_sub(last_ms) > 80;
                if !cancelled && !resting && wm.kinetic_scroll() && vel != [0.0, 0.0] {
                    wm.pan_coast_vx = vel[0];
                    wm.pan_coast_vy = vel[1];
                    wm.start_panning_animation();
                }
            }
            Claim::Multi { kind, track } => {
                if self.touch_points.values().any(|p| p.route == TouchRoute::Claimed) {
                    self.claim_rebase();
                    return;
                }
                self.touch_claim = Claim::None;
                if kind == MultiKind::Swipe {
                    self.touch_swipe(wm, "end", bind_fingers(track.fingers), 0.0, 0.0);
                }
            }
        }
    }

    /// The edge finger is not an edge swipe after all: route it normally
    /// from where it went down, and catch it up to where it is.
    unsafe fn edge_release(&mut self, wm: &mut crate::window_manager::WindowManager, time_msec: u32) {
        let Claim::Edge { id, start, .. } = self.touch_claim else { return };
        self.touch_claim = Claim::None;
        let Some(now) = self.touch_points.get(&id).map(|p| (p.lx, p.ly)) else { return };
        self.touch_begin(wm, id, start.0, start.1, time_msec);
        self.touch_motion(wm, id, now.0, now.1, time_msec);
    }

    /// The edge whose zone `lx, ly` is in, if an `edge_<side>` bind exists
    /// for it under the current modifiers. An edge is a screen edge only
    /// where no other output continues past it.
    unsafe fn bound_edge_at(&self, lx: f64, ly: f64) -> Option<Edge> {
        let server = (*self.seat).server;
        if !crate::shared::layout().gesture_binds.iter().any(|b| b.gesture_type == "edge") {
            return None;
        }
        let layout = (*server).om.output_layout;
        let output = ffi::wlr_output_layout_output_at(layout, lx, ly);
        if output.is_null() {
            return None;
        }
        let mut b = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
        ffi::wlr_output_layout_get_box(layout, output, &mut b);
        let (x0, y0) = (b.x as f64, b.y as f64);
        let (x1, y1) = (x0 + b.width as f64, y0 + b.height as f64);
        let beyond_is_empty = |x: f64, y: f64| ffi::wlr_output_layout_output_at(layout, x, y).is_null();
        let candidates = [
            (Edge::Left, lx - x0 < EDGE_ZONE && beyond_is_empty(x0 - 1.0, ly)),
            (Edge::Right, x1 - lx <= EDGE_ZONE && beyond_is_empty(x1 + 1.0, ly)),
            (Edge::Top, ly - y0 < EDGE_ZONE && beyond_is_empty(lx, y0 - 1.0)),
            (Edge::Bottom, y1 - ly <= EDGE_ZONE && beyond_is_empty(lx, y1 + 1.0)),
        ];
        let mods = gesture_mods(&*self.seat);
        candidates
            .into_iter()
            .filter(|&(_, near)| near)
            .map(|(edge, _)| edge)
            .find(|edge| gesture_bind("edge", 1, mods, |d| d == edge.name()).is_some())
    }

    /// One stage of a touchscreen swipe, through the touchpad's handling.
    unsafe fn touch_swipe(&mut self, wm: &mut crate::window_manager::WindowManager, stage: &str, fingers: u32, dx: f64, dy: f64) {
        self.gesture_from_touch = true;
        self.inject_swipe_stage(wm, stage, fingers, dx, dy);
        self.gesture_from_touch = false;
    }

    /// Fire a pinch bind if the spread has gone far enough; once per gesture.
    unsafe fn touch_pinch(&mut self, fingers: u32, scale: f64) -> MultiKind {
        let mods = gesture_mods(&*self.seat);
        let wm = &mut (*crate::reentry::wm((*self.seat).server));
        match gesture_bind("pinch", fingers, mods, |d| pinch_hits(d, scale)) {
            Some((action, command)) => {
                log::info!("touch: pinch fired {action:?}");
                run_gesture_action(wm, action, command.as_deref());
                MultiKind::Pinch { fired: true }
            }
            None => MultiKind::Pinch { fired: false },
        }
    }

    /// Layout coordinates of a touch event's normalized `x, y`, through the
    /// device's output mapping (`map_to_output` / `map_to_region`).
    unsafe fn touch_layout_coords(&self, touch: *mut ffi::wlr_touch, x: f64, y: f64) -> (f64, f64) {
        let wlr_device = if touch.is_null() {
            std::ptr::null_mut()
        } else {
            &mut (*touch).base as *mut ffi::wlr_input_device
        };
        let mut lx = 0.0;
        let mut ly = 0.0;
        ffi::wlr_cursor_absolute_to_layout_coords(self.wlr_cursor, wlr_device, x, y, &mut lx, &mut ly);
        (lx, ly)
    }
}

/// A layout point in its output's own coordinates, the anchor a pinch zoom
/// takes (`WindowManager::queue_pinch`).
unsafe fn output_local(server: *mut crate::server::Server, (x, y): (f64, f64)) -> (f64, f64) {
    let wlr_output = (*server).om.output_at(x, y);
    if wlr_output.is_null() {
        return (x, y);
    }
    let mut b = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
    ffi::wlr_output_layout_get_box((*server).om.output_layout, wlr_output, &mut b);
    (x - b.x as f64, y - b.y as f64)
}

pub(crate) unsafe extern "C" fn handle_touch_down(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_down_listener);
    let event = data as *mut ffi::wlr_touch_down_event;
    let (lx, ly) = cursor.touch_layout_coords((*event).touch, (*event).x, (*event).y);
    cursor.touch_down(&mut *crate::reentry::wm((*cursor.seat).server), (*event).touch_id, lx, ly, (*event).time_msec);
}

pub(crate) unsafe extern "C" fn handle_touch_motion(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_motion_listener);
    let event = data as *mut ffi::wlr_touch_motion_event;
    let (lx, ly) = cursor.touch_layout_coords((*event).touch, (*event).x, (*event).y);
    cursor.touch_motion(&mut *crate::reentry::wm((*cursor.seat).server), (*event).touch_id, lx, ly, (*event).time_msec);
}

pub(crate) unsafe extern "C" fn handle_touch_up(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_up_listener);
    let event = data as *mut ffi::wlr_touch_up_event;
    cursor.touch_up(&mut *crate::reentry::wm((*cursor.seat).server), (*event).touch_id, (*event).time_msec);
}

pub(crate) unsafe extern "C" fn handle_touch_cancel(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_cancel_listener);
    let event = data as *mut ffi::wlr_touch_cancel_event;
    cursor.touch_cancel(&mut *crate::reentry::wm((*cursor.seat).server), (*event).touch_id);
}

pub(crate) unsafe extern "C" fn handle_touch_frame(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_frame_listener);
    cursor.touch_frame();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edge_swipe_fires_inward_and_lets_go_along_the_edge() {
        // From the left edge, straight in.
        assert_eq!(edge_progress(Edge::Left, 30.0, 2.0), EdgeProgress::Undecided);
        assert_eq!(edge_progress(Edge::Left, EDGE_FIRE, 5.0), EdgeProgress::Fire);
        // From the bottom, inward is up.
        assert_eq!(edge_progress(Edge::Bottom, 0.0, -EDGE_FIRE), EdgeProgress::Fire);
        assert_eq!(edge_progress(Edge::Bottom, 0.0, EDGE_FIRE), EdgeProgress::NotEdge);
        // Along the top edge (a drag in the status bar) is not an edge swipe.
        assert_eq!(edge_progress(Edge::Top, 40.0, 3.0), EdgeProgress::NotEdge);
        // Diagonal but mostly inward still fires.
        assert_eq!(edge_progress(Edge::Right, -70.0, 40.0), EdgeProgress::Fire);
        // Jitter inside the slop decides nothing.
        assert_eq!(edge_progress(Edge::Top, 5.0, -3.0), EdgeProgress::Undecided);
    }

    #[test]
    fn shape_is_centroid_and_mean_spread() {
        let s = shape(&[(0.0, 0.0), (10.0, 0.0)]);
        assert_eq!(s.centroid, (5.0, 0.0));
        assert_eq!(s.spread, 5.0);
        assert_eq!(s.count, 2);
        assert_eq!(shape(&[]).count, 0);
    }

    #[test]
    fn a_track_rebases_when_a_finger_lands_or_lifts() {
        let mut t = Track::new(shape(&[(0.0, 0.0), (100.0, 0.0)]));
        assert_eq!(t.step(shape(&[(10.0, 0.0), (110.0, 0.0)])), (10.0, 0.0));
        // A third finger lands far away: the centroid jumps, but nothing moved.
        assert_eq!(t.step(shape(&[(10.0, 0.0), (110.0, 0.0), (60.0, 300.0)])), (0.0, 0.0));
        assert_eq!(t.travel, (10.0, 0.0));
        assert_eq!(t.fingers, 3);
        // Spreading to twice the distance scales by two.
        let mut p = Track::new(shape(&[(0.0, 0.0), (100.0, 0.0)]));
        p.step(shape(&[(-50.0, 0.0), (150.0, 0.0)]));
        assert!((p.scale - 2.0).abs() < 1e-9);
        assert_eq!(p.travel, (0.0, 0.0));
    }

    #[test]
    fn three_fingers_decide_between_swipe_and_pinch() {
        let fingers = [(0.0, 0.0), (60.0, 0.0), (30.0, 50.0)];
        let mut t = Track::new(shape(&fingers));
        let moved = |dx: f64, dy: f64| shape(&fingers.map(|(x, y)| (x + dx, y + dy)));
        t.step(moved(10.0, 0.0));
        assert_eq!(decide(&t), MultiKind::Undecided);
        t.step(moved(DECIDE_TRAVEL, 0.0));
        assert_eq!(decide(&t), MultiKind::Swipe);

        let mut p = Track::new(shape(&fingers));
        p.step(shape(&fingers.map(|(x, y)| ((x - 30.0) * 0.6 + 30.0, (y - 16.0) * 0.6 + 16.0))));
        assert_eq!(decide(&p), MultiKind::Pinch { fired: false });
    }

    #[test]
    fn a_fifth_finger_still_matches_four_finger_binds() {
        assert_eq!(bind_fingers(3), 3);
        assert_eq!(bind_fingers(4), 4);
        assert_eq!(bind_fingers(5), 4);
    }
}
