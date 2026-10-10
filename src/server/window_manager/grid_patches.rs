//! The world-anchored grid client's patches: which region of the desk cce-grid
//! must cover, sending it, and whether the current patch covers the viewport.
//! Split out of window_manager.rs on 2026-10-10.

use super::*;

impl WindowManager {
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
        // A ZOOM flight (an overview ramp, a zoom target) is the one camera
        // move that holds the compositor's own cell lattice on for its whole
        // duration (`grid_cells_hold`, in `arrange_views`) — and that
        // lattice is analytic: crisp at every zoom, and drawn from the same
        // style keys, so it stands in for this client's rendering without a
        // visible seam. So a zoom flight's patch is issued for its
        // DESTINATION alone, and whatever it does not reach while the camera
        // is still travelling is carried by the fallback.
        //
        // That is what lets it go out on the flight's FIRST frame instead of
        // waiting for the current viewport to shrink inside the buffer cap:
        // an overview exit's union is the zoomed-out viewport at the
        // destination's resolution, several times the cap, so the wait ran
        // most of the ramp and the client then rendered into the landing —
        // the exit came to rest on a 2x-magnified patch (4x in a deep
        // overview) and snapped sharp a beat AFTER the animation was over.
        //
        // A pan keeps the union: its fallback stays off (enabling the cell
        // pool re-bakes every blur, which is not worth paying per pan), so
        // its patch has to cover where the camera is as well as where it is
        // heading.
        let zoom_flight = target_cam.is_some()
            && (self.camera_ramp_anim.is_some() || self.target_desk_zoom.is_some());

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
            // A zoom flight is judged by its destination alone — the same
            // reason it is ISSUED for the destination alone. Demanding the
            // current viewport here would reject the patch just sent on
            // every animation frame and re-send it, the storm the comments
            // above are all about.
            (zoom_flight || now_ok) && target_ok
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
            // A flight patch is sized for its destination, not for pan
            // headroom (see the `zoom_flight` arm below). Once the camera is
            // at rest and the client has actually latched it, re-issue the
            // roomy cap-filling rect: a pan outruns the small one within a
            // fraction of a viewport, and a pan has no fallback lattice to
            // cover what it outruns. Rendered at rest behind a patch that
            // already displays correctly, so the swap is invisible.
            let upgrade = (*w).grid_patch_flight
                && !zoom_flight
                && (*w).grid_patch_pending.is_none()
                && (*w).grid_patch_acked.is_none();
            // A stale patch (style changed since it was rendered) is
            // re-issued regardless of coverage; a covering patch already in
            // flight is left to latch first, and the flag then re-sends
            // once it has become current.
            let stale = (*w).grid_patch_stale;
            if !stale && !upgrade && (*w).grid_patch_current.as_ref().map_or(false, &covers) {
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
            // The cap is a MEMORY knob, shared by both sizings below: the
            // client's framebuffer is (patch * q)^2 * 4B per swapchain image
            // (q carries the output scale), and it is scale-aware (4096 *
            // out_scale keeps the same VIRTUAL coverage at every scale) so
            // margins can never shrink below what covers() demands.
            let max_buf: f64 = 4096.0 * out_scale;
            if let Some((tx, ty, tw, th, _)) = target_rect.filter(|_| zoom_flight) {
                // A zoom flight's patch is sized for its DESTINATION, not
                // for the cap — it has to be RENDERED before the ramp lands,
                // and the cap-filling rect is three times the pixels: 250ms
                // of client render at this output's scale, longer than the
                // ramp itself, which is how it used to arrive late. 0.25 of
                // the destination viewport per side keeps the resting
                // `covers()` margin (0.15) intact so landing does not
                // immediately re-patch, and `grid_patch_flight` upgrades to
                // the roomy rect once at rest.
                let span = |lo: f64, hi: f64, mid: f64, period: f64| -> (f64, f64) {
                    let cap = ((max_buf / q) / period).floor().max(1.0) * period;
                    let p0 = (lo / period).floor() * period;
                    let p1 = (hi / period).ceil() * period;
                    if p1 - p0 <= cap {
                        return (p0, p1 - p0);
                    }
                    // A destination too wide to cover at this resolution —
                    // a deep overview ENTER, whose q is raised for the zoom
                    // the camera is still at. Cap-sized and centered on it;
                    // the fallback lattice draws the rest until the rest
                    // band re-patches at the destination's own resolution.
                    (((mid - cap * 0.5) / period).floor() * period, cap)
                };
                let (x0, pw) = span(tx - 0.25 * tw, tx + 1.25 * tw, tx + tw * 0.5, period_x);
                let (y0, ph) = span(ty - 0.25 * th, ty + 1.25 * th, ty + th * 0.5, period_y);
                let patch = crate::policy::api::GridPatch { x: x0, y: y0, w: pw, h: ph, scale: q };
                let reason = if stale { PatchReason::StyleReload } else { PatchReason::Flight };
                self.send_grid_patch_to(w, patch, reason, true);
                continue;
            }
            // Coverage: the current viewport, unioned with a predicted
            // destination (+0.10 comfort) when one is known. Then the
            // remaining buffer budget spreads as margin per side and the
            // rect period-aligns outward so the client draws whole cells.
            // The original 3x3-viewport margin cost ~340MB per image, for
            // scroll headroom that the 0.15 comfort margin rarely used.
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
            if uw * q > max_buf || uh * q > max_buf {
                // The union doesn't fit (a pan whose predicted destination
                // is a viewport away at native resolution): keep displaying
                // the old patch and retry as the camera approaches it.
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
            let reason = if stale {
                PatchReason::StyleReload
            } else if upgrade {
                PatchReason::Upgrade
            } else {
                PatchReason::Coverage
            };
            self.send_grid_patch_to(w, patch, reason, false);
        }
    }

    /// Issue one patch to one grid client and record it as pending.
    /// `flight` marks a patch sized for a camera flight's destination, which
    /// `update_grid_patches` upgrades to the roomy resting rect once the
    /// camera has landed and the client has latched it.
    pub(crate) unsafe fn send_grid_patch_to(
        &self,
        w: *mut crate::window::Window,
        patch: crate::policy::api::GridPatch,
        reason: PatchReason,
        flight: bool,
    ) {
        // Re-sending the patch the client already has renders the same
        // pixels again and latches to the same anchor — nothing changes, so
        // the next arrange asks for it again: a silent re-render loop for as
        // long as the camera sits there. It is reachable wherever the
        // resolution the caps allow does not satisfy the rest band (deep
        // overview, where `q` is already at its 0.125 floor and the patch is
        // necessarily oversampled). A style reload is the one caller that
        // MEANS the same rect — the pixels are what changed.
        if reason != PatchReason::StyleReload {
            let same = |p: &crate::policy::api::GridPatch| *p == patch;
            if (*w).grid_patch_pending.as_ref().map_or(false, |(_, p)| same(p))
                || (*w).grid_patch_acked.as_ref().map_or(false, |(_, p)| same(p))
                || (*w).grid_patch_current.as_ref().map_or(false, same)
            {
                // The upgrade would otherwise ask again on every arrange.
                (*w).grid_patch_flight = flight;
                return;
            }
        }
        (*w).grid_patch_serial = (*w).grid_patch_serial.wrapping_add(1);
        let serial = (*w).grid_patch_serial;
        if (*self.server)
            .cce_window_management
            .send_grid_patch((*w).ref_key, serial, patch)
        {
            log::info!("[Grid] sent patch #{serial}: {:.0},{:.0} {:.0}x{:.0} @{:.3} ({})",
                patch.x, patch.y, patch.w, patch.h, patch.scale, reason.tag());
            (*w).grid_patch_pending = Some((serial, patch));
            (*w).grid_patch_stale = false;
            (*w).grid_patch_flight = flight;
        } else {
            log::info!("[Grid] patch #{serial} not sent (no toplevel resource yet)");
        }
    }

    /// Hand a frame callback to every grid client with a patch it has not
    /// rendered yet, whether or not the scene thinks its surface is visible
    /// — see the call site in the output frame handler.
    pub unsafe fn send_frame_done_to_grid_clients_awaiting_patch(&self) {
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || !(*w).is_grid() {
                continue;
            }
            if !matches!((*w).state, crate::window::WindowState::Mapped) {
                continue;
            }
            if (*w).grid_patch_pending.is_some() {
                (*w).send_frame_done();
            }
        }
    }

    /// Whether any mapped grid client has a patch issued but not yet
    /// latched — sent and unacked, or acked and awaiting the commit that
    /// carries its buffer.
    pub unsafe fn grid_patch_in_air(&self) -> bool {
        self.windows.iter().any(|&w| {
            !w.is_null()
                && !(*w).closed
                && (*w).is_grid()
                && matches!((*w).state, crate::window::WindowState::Mapped)
                && ((*w).grid_patch_pending.is_some() || (*w).grid_patch_acked.is_some())
        })
    }

    /// Whether every mapped grid client's LATCHED patch reaches the whole
    /// current viewport — bare coverage, no comfort margin. False while the
    /// patch a camera flight needed is still being rendered by the client,
    /// or wherever the camera has come to rest beyond the latched patch
    /// (a union that did not fit the buffer cap). True with no grid client
    /// at all: that case is the arrange plan's `grid_cells_enabled`.
    pub unsafe fn grid_patch_covers_viewport(&self) -> bool {
        let mut out_box: Option<ffi::wlr_box> = None;
        let outputs_list = &(*self.server).om.outputs as *const ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                out_box = Some((*output).sent.box_layout());
                break;
            }
            curr_out = (*curr_out).next;
        }
        let Some(out) = out_box else { return true };
        let zoom = crate::policy::background::sanitized_zoom(self.desk_zoom);
        let (vx, vy) = (self.desk_pan_x, self.desk_pan_y);
        let (vw, vh) = (out.width as f64 / zoom, out.height as f64 / zoom);
        for &w in self.windows.iter() {
            if w.is_null() || (*w).closed || !(*w).is_grid() {
                continue;
            }
            if !matches!((*w).state, crate::window::WindowState::Mapped) {
                continue;
            }
            let Some(p) = &(*w).grid_patch_current else { return false };
            // Half a virtual unit of tolerance: the anchor is rounded to a
            // screen pixel, and a patch edge exactly on the viewport edge
            // must not read as a gap.
            let covered = p.x <= vx + 0.5
                && p.y <= vy + 0.5
                && p.x + p.w >= vx + vw - 0.5
                && p.y + p.h >= vy + vh - 0.5;
            if !covered {
                return false;
            }
        }
        true
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
}
