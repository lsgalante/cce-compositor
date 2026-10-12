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
    unsafe fn touch_route_at(wm: &crate::window_manager::WindowManager, sid: crate::seat::SeatId, lx: f64, ly: f64) -> TouchRoute {
        let Some(seat) = wm.seats.get(sid) else { return TouchRoute::Ignored };
        let this = &seat.cursor;
        let result = crate::shared::scene().at(wm, lx, ly);
        if let Some(ref result) = result {
            // Locked, only the lock surface may hear a finger, as only it
            // may hear the pointer (`passthrough`).
            let locked = crate::shared::lock_state() != crate::lock_manager::LockState::Unlocked;
            let lock_ok = !locked || matches!(result.data, SceneNodeDataVal::LockSurface(_));
            if lock_ok
                && !result.surface.is_null()
                && !crate::shared::window_adjust_active()
                && ffi::wlr_surface_accepts_touch(result.surface, seat.wlr_seat.ptr())
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
        let pointer_busy = this.touch_points.values().any(|p| matches!(p.route, TouchRoute::Pointer { .. }))
            || !this.pressed.is_empty()
            || seat.op.is_some();
        if pointer_busy {
            return TouchRoute::Ignored;
        }
        let on_desk = match result {
            None => crate::shared::lock_state() == crate::lock_manager::LockState::Unlocked,
            Some(r) => matches!(r.data, SceneNodeDataVal::Window(id) if wm.windows.get(id).is_some_and(|w| w.is_wallpaper())),
        };
        TouchRoute::Pointer { start: (lx, ly), pressed: false, on_desk }
    }

    pub unsafe fn touch_down(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, id: i32, lx: f64, ly: f64, time_msec: u32, sys: &mut crate::ctx::Sys) {
        crate::seat::Seat::handle_activity(wm, sid, sys);
        // A touch is deliberate input, like a press (see `Seat::focus`).
        wm.startup_input_seen = true;
        let this = cursor_mut!(wm, sid, ());
        this.hide_for_touch();
        // A device that reuses a live id without lifting it first has lost
        // the up; finish the old point so its button or client is released.
        if this.touch_points.contains_key(&id) {
            log::warn!("touch: down for live touch id {id}; lifting the old point first");
            Cursor::touch_up(wm, sid, id, time_msec, sys);
        }

        let this = cursor_mut!(wm, sid, ());
        match this.touch_claim {
            // A second finger is not an edge swipe: hand the first back
            // before routing this one.
            Claim::Edge { .. } => Cursor::edge_release(wm, sid, time_msec, sys),
            Claim::Desk { track, .. } => {
                this.touch_points.insert(id, TouchPoint { lx, ly, route: TouchRoute::Claimed });
                // Three fingers on a desk that has not moved yet were a
                // three-finger gesture landing one finger at a time.
                let still = track.travel.0.hypot(track.travel.1) < TAP_SLOP && (track.scale - 1.0).abs() < 0.05;
                if still && this.touch_points.len() >= 3 {
                    this.touch_claim = Claim::Multi { track: Track::new(this.claimed_shape()), kind: MultiKind::Undecided };
                } else {
                    this.claim_rebase();
                }
                return;
            }
            Claim::Multi { .. } => {
                this.touch_points.insert(id, TouchPoint { lx, ly, route: TouchRoute::Claimed });
                this.claim_rebase();
                return;
            }
            Claim::None => {}
        }

        let this = cursor_mut!(wm, sid, ());
        let already = this.touch_points.len();
        if already == 0 {
            if let Some(edge) = this.bound_edge_at(lx, ly) {
                log::debug!("touch: down id={id} in the {} edge zone", edge.name());
                this.touch_points.insert(id, TouchPoint { lx, ly, route: TouchRoute::Claimed });
                this.touch_claim = Claim::Edge { id, edge, start: (lx, ly), fired: false };
                return;
            }
        }
        let pointer_pressed = this.touch_points.values().any(|p| matches!(p.route, TouchRoute::Pointer { pressed: true, .. }));
        if already + 1 >= 3 && !pointer_pressed {
            Cursor::claim_all(wm, sid, sys);
            let this = cursor_mut!(wm, sid, ());
            this.touch_points.insert(id, TouchPoint { lx, ly, route: TouchRoute::Claimed });
            this.touch_claim = Claim::Multi { track: Track::new(this.claimed_shape()), kind: MultiKind::Undecided };
            log::debug!("touch: {} fingers claimed for a gesture", this.touch_points.len());
            return;
        }
        if already == 1 {
            let first_on_desk = this
                .touch_points
                .values()
                .any(|p| matches!(p.route, TouchRoute::Pointer { pressed: false, on_desk: true, .. }));
            if first_on_desk {
                Cursor::claim_all(wm, sid, sys);
                let this = cursor_mut!(wm, sid, ());
                this.touch_points.insert(id, TouchPoint { lx, ly, route: TouchRoute::Claimed });
                Cursor::start_desk(wm, sid);
                return;
            }
        }
        Cursor::touch_begin(wm, sid, id, lx, ly, time_msec, sys);
    }

    /// Arm the on-screen keyboard (`osk.rs`) when the finger is on an
    /// app's window — not the board itself, a layer surface, nor the bar.
    fn note_touch_for_osk(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, lx: f64, ly: f64) {
        let Some(result) = crate::shared::scene().at(wm, lx, ly) else { return };
        if let SceneNodeDataVal::Window(window_id) = result.data {
            if let Some(window) = wm.windows.get(window_id) {
                if !window.is_status_bar() && !window.is_wallpaper() {
                    seat_mut!(wm, sid, ()).relay.osk.note_touch();
                }
            }
        }
    }

    /// Route a finger normally (`touch_route_at`) and deliver its down.
    unsafe fn touch_begin(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, id: i32, lx: f64, ly: f64, time_msec: u32, sys: &mut crate::ctx::Sys) {
        let route = Cursor::touch_route_at(wm, sid, lx, ly);
        let this = cursor_mut!(wm, sid, ());
        this.touch_points.insert(id, TouchPoint { lx, ly, route });
        log::debug!("touch: down id={id} at ({lx:.0}, {ly:.0}) -> {route:?}");

        match route {
            TouchRoute::Client { origin, scale } => {
                crate::cursor::press_dismissals(wm, lx, ly);
                let Some(result) = crate::shared::scene().at(wm, lx, ly) else { return };
                // Focus as a click would (`handle_button`).
                match result.data {
                    SceneNodeDataVal::Window(window_id) => {
                        if let Some(window) = wm.windows.get(window_id) {
                            if !window.is_status_bar() && !window.is_wallpaper() {
                                crate::seat::Seat::focus(wm, sid, Focus::Window(window_id));
                                seat_mut!(wm, sid, ()).relay.osk.note_touch();
                            }
                        }
                    }
                    SceneNodeDataVal::LayerSurface(layer_surface) => {
                        if crate::cursor::layer_takes_click_focus(layer_surface) {
                            crate::seat::Seat::focus(wm, sid, Focus::LayerSurface(result.surface));
                        }
                    }
                    _ => {}
                }
                ffi::wlr_seat_touch_notify_down(
                    seat_mut!(wm, sid, ()).wlr_seat.ptr(),
                    result.surface,
                    time_msec,
                    id,
                    (lx - origin.0) * scale,
                    (ly - origin.1) * scale,
                );
            }
            // Hover now; the press waits (see `TouchRoute::Pointer`).
            TouchRoute::Pointer { .. } => Cursor::warp_to(wm, sid, lx, ly, sys),
            TouchRoute::Claimed | TouchRoute::Ignored => {}
        }
    }

    pub unsafe fn touch_motion(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, id: i32, lx: f64, ly: f64, time_msec: u32, sys: &mut crate::ctx::Sys) {
        crate::seat::Seat::handle_activity(wm, sid, sys);
        let this = cursor_mut!(wm, sid, ());
        let Some(point) = this.touch_points.get_mut(&id) else { return };
        point.lx = lx;
        point.ly = ly;
        let route = point.route;
        match route {
            TouchRoute::Claimed => Cursor::claim_motion(wm, sid, time_msec, sys),
            TouchRoute::Client { origin, scale } => {
                this.wlr_seat.touch_notify_motion(time_msec, id, (lx - origin.0) * scale, (ly - origin.1) * scale);
                // A touch drag's icon follows the finger.
                this.update_drag_icons();
            }
            TouchRoute::Pointer { start, pressed: false, on_desk } => {
                if (lx - start.0).hypot(ly - start.1) < TAP_SLOP {
                    return;
                }
                let normal = crate::shared::mode() == crate::window_manager::WindowManagerMode::Normal;
                if on_desk && normal && this.touch_points.len() == 1 {
                    // A drag on the bare desk pans it. The track starts at
                    // the down point, so the desk catches up with the
                    // finger's whole travel, not just what follows the slop.
                    this.touch_points.get_mut(&id).unwrap().route = TouchRoute::Claimed;
                    let mut track = Track::new(shape(&[start]));
                    track.fingers = 1;
                    this.touch_claim = Claim::Desk {
                        track,
                        start_zoom: wm.desk_zoom,
                        vel: [0.0, 0.0],
                        last_ms: 0,
                    };
                    wm.stop_panning_animation();
                    Cursor::claim_motion(wm, sid, time_msec, sys);
                    return;
                }
                // A drag: press where the finger went down, then follow it.
                this.touch_points.get_mut(&id).unwrap().route = TouchRoute::Pointer { start, pressed: true, on_desk };
                Cursor::warp_to(wm, sid, start.0, start.1, sys);
                Cursor::inject_button(wm, sid, BTN_LEFT, true, sys);
                Cursor::warp_to(wm, sid, lx, ly, sys);
            }
            TouchRoute::Pointer { pressed: true, .. } => Cursor::warp_to(wm, sid, lx, ly, sys),
            TouchRoute::Ignored => {}
        }
    }

    pub unsafe fn touch_up(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, id: i32, time_msec: u32, sys: &mut crate::ctx::Sys) {
        crate::seat::Seat::handle_activity(wm, sid, sys);
        let this = cursor_mut!(wm, sid, ());
        let Some(point) = this.touch_points.remove(&id) else { return };
        match point.route {
            TouchRoute::Claimed => Cursor::claim_lift(wm, sid, id, point, time_msec, false, sys),
            TouchRoute::Client { .. } => {
                // A field that starts editing on the release is still this
                // touch's (`osk.rs`).
                Cursor::note_touch_for_osk(wm, sid, point.lx, point.ly);
                let this = cursor_mut!(wm, sid, ());
                this.wlr_seat.touch_notify_up(time_msec, id);
            }
            TouchRoute::Pointer { start, pressed: false, .. } => {
                // A tap.
                Cursor::note_touch_for_osk(wm, sid, start.0, start.1);
                Cursor::warp_to(wm, sid, start.0, start.1, sys);
                Cursor::inject_button(wm, sid, BTN_LEFT, true, sys);
                Cursor::inject_button(wm, sid, BTN_LEFT, false, sys);
            }
            TouchRoute::Pointer { pressed: true, .. } => Cursor::inject_button(wm, sid, BTN_LEFT, false, sys),
            TouchRoute::Ignored => {}
        }
    }

    /// The device gave up on a finger (a palm, a gesture the kernel took).
    /// A client is told `wl_touch.cancel`, which voids its whole sequence; a
    /// finger driving the pointer releases the button — a cancelled drag
    /// still drops where it was, but a held button must never be left
    /// behind — and one that never pressed clicks nothing.
    pub unsafe fn touch_cancel(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, id: i32, sys: &mut crate::ctx::Sys) {
        crate::seat::Seat::handle_activity(wm, sid, sys);
        let this = cursor_mut!(wm, sid, ());
        let Some(point) = this.touch_points.remove(&id) else { return };
        match point.route {
            TouchRoute::Claimed => Cursor::claim_lift(wm, sid, id, point, crate::util::msec_timestamp(), true, sys),
            TouchRoute::Client { .. } => this.wlr_seat.touch_cancel_point(id),
            TouchRoute::Pointer { pressed: true, .. } => Cursor::inject_button(wm, sid, BTN_LEFT, false, sys),
            TouchRoute::Pointer { pressed: false, .. } | TouchRoute::Ignored => {}
        }
    }

    pub fn touch_frame(&mut self) {
        // The emulated pointer frames each of its own events; only touch
        // clients are waiting on this one.
        if self.touch_points.values().any(|p| matches!(p.route, TouchRoute::Client { .. }))
            || self.wlr_seat.touch_num_points() > 0
        {
            self.wlr_seat.touch_notify_frame();
        }
    }

    // ── Gestures ────────────────────────────────────────────────────────

    /// Take every finger already down for a gesture: clients hear
    /// `wl_touch.cancel`, and a pointer finger that never pressed is simply
    /// dropped (the reason the press waits).
    unsafe fn claim_all(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, sys: &mut crate::ctx::Sys) {
        let this = cursor_mut!(wm, sid, ());
        let ids: Vec<i32> = this.touch_points.keys().copied().collect();
        for id in ids {
            let this = cursor_mut!(wm, sid, ());
            let point = this.touch_points.get_mut(&id).unwrap();
            let route = std::mem::replace(&mut point.route, TouchRoute::Claimed);
            match route {
                TouchRoute::Client { .. } => this.wlr_seat.touch_cancel_point(id),
                TouchRoute::Pointer { pressed: true, .. } => Cursor::inject_button(wm, sid, BTN_LEFT, false, sys),
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

    fn start_desk(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId) {
        wm.stop_panning_animation();
        let this = cursor_mut!(wm, sid, ());
        this.touch_claim = Claim::Desk {
            track: Track::new(this.claimed_shape()),
            start_zoom: wm.desk_zoom,
            vel: [0.0, 0.0],
            last_ms: 0,
        };
        log::debug!("touch: desk gesture with {} fingers", this.touch_points.len());
    }

    unsafe fn claim_motion(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, time_msec: u32, sys: &mut crate::ctx::Sys) {
        let this = cursor_mut!(wm, sid, ());
        let s = this.claimed_shape();
        match this.touch_claim {
            Claim::None => {}
            Claim::Edge { id, edge, start, fired } => {
                if fired {
                    return;
                }
                let Some(p) = this.touch_points.get(&id) else { return };
                match edge_progress(edge, p.lx - start.0, p.ly - start.1) {
                    EdgeProgress::Undecided => {}
                    EdgeProgress::NotEdge => Cursor::edge_release(wm, sid, time_msec, sys),
                    EdgeProgress::Fire => {
                        this.touch_claim = Claim::Edge { id, edge, start, fired: true };
                        let mods = gesture_mods(&seat_mut!(wm, sid, ()).wlr_seat);
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
                    let (ax, ay) = output_local(s.centroid);
                    wm.queue_pinch(zoom, ax, ay);
                }
                let this = cursor_mut!(wm, sid, ());
                this.touch_claim = Claim::Desk { track, start_zoom, vel, last_ms };
            }
            Claim::Multi { mut track, kind } => {
                let d = track.step(s);
                let fingers = bind_fingers(track.fingers);
                let kind = match kind {
                    MultiKind::Undecided => match decide(&track) {
                        MultiKind::Swipe => {
                            log::info!("touch: {fingers}-finger swipe");
                            // Natural: the camera goes against the fingers.
                            Cursor::touch_swipe(wm, sid, "begin", fingers, 0.0, 0.0, sys);
                            Cursor::touch_swipe(wm, sid, "update", fingers, -track.travel.0, -track.travel.1, sys);
                            MultiKind::Swipe
                        }
                        MultiKind::Pinch { .. } => {
                            log::info!("touch: {fingers}-finger pinch");
                            Cursor::touch_pinch(wm, sid, fingers, track.scale)
                        }
                        MultiKind::Undecided => MultiKind::Undecided,
                    },
                    MultiKind::Swipe => {
                        if d != (0.0, 0.0) {
                            Cursor::touch_swipe(wm, sid, "update", fingers, -d.0, -d.1, sys);
                        }
                        MultiKind::Swipe
                    }
                    MultiKind::Pinch { fired: false } => Cursor::touch_pinch(wm, sid, fingers, track.scale),
                    fired @ MultiKind::Pinch { fired: true } => fired,
                };
                let this = cursor_mut!(wm, sid, ());
                this.touch_claim = Claim::Multi { track, kind };
            }
        }
    }

    /// A claimed finger lifted (or was cancelled). The gesture ends with
    /// its last finger.
    unsafe fn claim_lift(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, id: i32, point: TouchPoint, time_msec: u32, cancelled: bool, sys: &mut crate::ctx::Sys) {
        let this = cursor_mut!(wm, sid, ());
        match this.touch_claim {
            Claim::None => {}
            Claim::Edge { id: edge_id, start, fired, .. } => {
                if edge_id != id {
                    return;
                }
                this.touch_claim = Claim::None;
                // Lifted where it landed: it was a tap in the edge zone (the
                // status bar's, say), so deliver it as one, late.
                if !fired && !cancelled && (point.lx - start.0).hypot(point.ly - start.1) < TAP_SLOP {
                    Cursor::touch_begin(wm, sid, id, start.0, start.1, time_msec, sys);
                    Cursor::touch_up(wm, sid, id, time_msec, sys);
                }
            }
            Claim::Desk { vel, last_ms, .. } => {
                if this.touch_points.values().any(|p| p.route == TouchRoute::Claimed) {
                    this.claim_rebase();
                    return;
                }
                this.touch_claim = Claim::None;
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
                if this.touch_points.values().any(|p| p.route == TouchRoute::Claimed) {
                    this.claim_rebase();
                    return;
                }
                this.touch_claim = Claim::None;
                if kind == MultiKind::Swipe {
                    Cursor::touch_swipe(wm, sid, "end", bind_fingers(track.fingers), 0.0, 0.0, sys);
                }
            }
        }
    }

    /// The edge finger is not an edge swipe after all: route it normally
    /// from where it went down, and catch it up to where it is.
    unsafe fn edge_release(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, time_msec: u32, sys: &mut crate::ctx::Sys) {
        let this = cursor_mut!(wm, sid, ());
        let Claim::Edge { id, start, .. } = this.touch_claim else { return };
        this.touch_claim = Claim::None;
        let Some(now) = this.touch_points.get(&id).map(|p| (p.lx, p.ly)) else { return };
        Cursor::touch_begin(wm, sid, id, start.0, start.1, time_msec, sys);
        Cursor::touch_motion(wm, sid, id, now.0, now.1, time_msec, sys);
    }

    /// The edge whose zone `lx, ly` is in, if an `edge_<side>` bind exists
    /// for it under the current modifiers. An edge is a screen edge only
    /// where no other output continues past it.
    unsafe fn bound_edge_at(&self, lx: f64, ly: f64) -> Option<Edge> {
        if !crate::shared::layout().gesture_binds.iter().any(|b| b.gesture_type == "edge") {
            return None;
        }
        let layout = crate::shared::handles().output_layout.ptr();
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
        let mods = gesture_mods(&self.wlr_seat);
        candidates
            .into_iter()
            .filter(|&(_, near)| near)
            .map(|(edge, _)| edge)
            .find(|edge| gesture_bind("edge", 1, mods, |d| d == edge.name()).is_some())
    }

    /// One stage of a touchscreen swipe, through the touchpad's handling.
    unsafe fn touch_swipe(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, stage: &str, fingers: u32, dx: f64, dy: f64, sys: &mut crate::ctx::Sys) {
        let this = cursor_mut!(wm, sid, ());
        this.gesture_from_touch = true;
        Cursor::inject_swipe_stage(wm, sid, stage, fingers, dx, dy, sys);
        let this = cursor_mut!(wm, sid, ());
        this.gesture_from_touch = false;
    }

    /// Fire a pinch bind if the spread has gone far enough; once per gesture.
    fn touch_pinch(wm: &mut crate::window_manager::WindowManager, sid: crate::seat::SeatId, fingers: u32, scale: f64) -> MultiKind {
        let mods = gesture_mods(&seat_mut!(wm, sid, MultiKind::Undecided).wlr_seat);
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
        ffi::wlr_cursor_absolute_to_layout_coords(self.wlr_cursor.ptr(), wlr_device, x, y, &mut lx, &mut ly);
        (lx, ly)
    }
}

/// A layout point in its output's own coordinates, the anchor a pinch zoom
/// takes (`WindowManager::queue_pinch`).
unsafe fn output_local((x, y): (f64, f64)) -> (f64, f64) {
    let wlr_output = crate::output_manager::output_at(x, y);
    if wlr_output.is_null() {
        return (x, y);
    }
    let mut b = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
    ffi::wlr_output_layout_get_box(crate::shared::handles().output_layout.ptr(), wlr_output, &mut b);
    (x - b.x as f64, y - b.y as f64)
}

pub(crate) unsafe extern "C" fn handle_touch_down(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_down_listener);
    let event = data as *mut ffi::wlr_touch_down_event;
    let (lx, ly) = cursor.touch_layout_coords((*event).touch, (*event).x, (*event).y);
    crate::cursor::Cursor::touch_down(&mut *crate::reentry::wm(crate::shared::server_ptr()), cursor.sid, (*event).touch_id, lx, ly, (*event).time_msec, sys);
}

pub(crate) unsafe extern "C" fn handle_touch_motion(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_motion_listener);
    let event = data as *mut ffi::wlr_touch_motion_event;
    let (lx, ly) = cursor.touch_layout_coords((*event).touch, (*event).x, (*event).y);
    crate::cursor::Cursor::touch_motion(&mut *crate::reentry::wm(crate::shared::server_ptr()), cursor.sid, (*event).touch_id, lx, ly, (*event).time_msec, sys);
}

pub(crate) unsafe extern "C" fn handle_touch_up(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_up_listener);
    let event = data as *mut ffi::wlr_touch_up_event;
    crate::cursor::Cursor::touch_up(&mut *crate::reentry::wm(crate::shared::server_ptr()), cursor.sid, (*event).touch_id, (*event).time_msec, sys);
}

pub(crate) unsafe extern "C" fn handle_touch_cancel(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let sys = &mut crate::ctx::Sys::of(crate::shared::server_ptr());
    let cursor = &mut *crate::container_of!(listener, Cursor, touch_cancel_listener);
    let event = data as *mut ffi::wlr_touch_cancel_event;
    crate::cursor::Cursor::touch_cancel(&mut *crate::reentry::wm(crate::shared::server_ptr()), cursor.sid, (*event).touch_id, sys);
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
