//! Drawing a window's server-side border, its handles and buttons, and the
//! surface clip. Split out of window.rs on 2026-10-10.

use super::*;

impl Window {
    pub unsafe fn draw_borders(&mut self) {
        // Taken before `requested` borrows self: `window_takes_handles` is
        // the shared predicate with cursor::get_border_zone and must not be
        // duplicated here just to satisfy borrowck.
        let self_ptr = self as *mut Window;
        let requested = &self.rendering_requested;

        let border = &requested.border;
        let border_color = border.color;
        self.window_background.set_position_if_changed(0, 0);
        let bg_width = (self.box_geom.width as f64 * self.scale) as i32;
        let bg_height = (self.box_geom.height as f64 * self.scale) as i32;
        self.window_background.set_size_if_changed(bg_width, bg_height);
        self.window_background.set_color(&border_color);
        // The background plate sits directly under the client's plate, so it
        // takes the ROOT_PLATE radius and the same span widening as the
        // blur/clip radius — not the border ring's radius, which is a
        // separate key describing a different edge.
        let bg_radius = widen_corner_radius(
            self.root_plate_radius_base(),
            self.box_geom.width,
            self.box_geom.height,
        );
        self.window_background.set_corner_radius((bg_radius as f64 * self.scale) as i32);
        self.window_background.set_enabled(!requested.hidden && self.wm_requested.ssd);

        // The handles draw as eight discs in one frame node; the hovered
        // disc draws in hover_color. Under each disc a transparent square
        // rect is a scene hit-test catcher (width 0 keeps the legacy
        // invisible 8px virtual resize zones).
        //
        // They live in `border.tree`, parented to the global border overlay
        // layer rather than to this window's tree, so it has to be
        // positioned and enabled in step with the window by hand.
        let is_virtual_border = border.width == 0;
        // Deliberately NOT gated on `wm_requested.ssd`: that flag defaults to
        // false and is only set by a client calling use_ssd, and the segments
        // have never depended on it — only `window_background` does.
        let borders_visible = !requested.hidden
            && !requested.circular
            && !is_virtual_border
            && self.border_reveal.iter().any(|&a| a > 0.0);
        self.border.tree.set_enabled(borders_visible);
        if borders_visible {
            self.border.tree.set_position_if_changed(self.box_geom.x, self.box_geom.y);
        }
        if requested.circular {
            self.border.left.set_enabled(false);
            self.border.right.set_enabled(false);
            self.border.top.set_enabled(false);
            self.border.bottom.set_enabled(false);
            for seg in self.border.segments.iter() {
                seg.set_enabled(false);
            }
            return;
        }
        let content = ffi::wlr_box {
            x: 0,
            y: 0,
            width: self.box_geom.width,
            height: self.box_geom.height,
        };

        let mut intersect = std::mem::zeroed();
        let clip_empty = requested.content_clip.width == 0 && requested.content_clip.height == 0;
        if clip_empty || ffi::wlr_box_intersection(&mut intersect, &content, &requested.content_clip) {
            let border = &requested.border;
            // The interactive band (doubled configured width, floored). The
            // per-side foam clipping this used to carry is gone with the
            // outside band: it split a gap SHARED with a neighbouring window,
            // and the inside ring shares nothing.
            let band_f = border_band_width(border.width);
            let band = band_f as i32;
            let transparent = [0.0f32; 4];

            // The rounded-frame path used to leave radius/clip state on the
            // top band rect; keep it reset.
            self.border.top.set_corner_radius(0);
            self.border.top.set_clipped_region(ffi::clipped_region_get_default());

            let apply = |rect: *mut ffi::wlr_scene_rect, bx: ffi::wlr_box, color: &[f32; 4], enabled: bool| {
                let mut bx = bx;
                if enabled && (requested.clip.width != 0 || requested.clip.height != 0) {
                    let mut clip_intersect = std::mem::zeroed();
                    ffi::wlr_box_intersection(&mut clip_intersect, &bx, &requested.clip);
                    bx = clip_intersect;
                }
                let enabled = enabled && bx.width > 0 && bx.height > 0;
                ffi::wlr_scene_node_set_enabled(rect as *mut ffi::wlr_scene_node, enabled);
                if !enabled {
                    return;
                }
                ffi::river_scene_node_set_position_if_changed(
                    rect as *mut ffi::wlr_scene_node,
                    (bx.x as f64 * self.scale) as i32,
                    (bx.y as f64 * self.scale) as i32,
                );
                ffi::river_scene_rect_set_size_if_changed(
                    rect,
                    (bx.width as f64 * self.scale) as i32,
                    (bx.height as f64 * self.scale) as i32,
                );
                ffi::wlr_scene_rect_set_color(rect, color.as_ptr());
            };

            // Handles live INSIDE the content rect, and only in overview
            // mode — see `cursor::get_border_zone`, which hit-tests the same
            // ring from the same band width and corner length. In normal
            // mode there is nothing to grab, so the catchers and the visible
            // segments are both disabled outright. The window's own border
            // (`window_background`, above) is untouched in either mode: this
            // moved the HANDLES inward, not the border.
            // Overview, or Super held: the same adjust mode at any zoom.
            let in_overview = (*crate::reentry::wm(self.server)).window_adjust_active();
            let bw = band;
            let layout_handle_w = (*crate::reentry::wm(self.server)).layout.border_handle_width;
            let sc = if self.scale > 0.0 { self.scale } else { 1.0 };
            let (cw, ch) = (content.width, content.height);
            // A window thinner than two bands has no interior left for a
            // ring; drawing one would be a solid block over the whole window.
            // Live handles: the mode is on and this is the adjust target
            // (the window under the pointer). Target-only,
            // like the reveal in step_border_fade: without this the
            // invisible catcher rects would keep intercepting scene hits on
            // windows whose ring is not even drawn.
            let handles_live = in_overview && self.is_adjust_target();
            // Drawn handles: live, OR still fading out — releasing Super (or
            // leaving overview, or losing focus) eases the ring away instead
            // of cutting it, so the ring stays drawn while any reveal is
            // above zero. The catchers below are gated on `handles_live`
            // alone: a fading ring is decoration, never a grab.
            let fading_out = !handles_live && self.border_reveal.iter().any(|&a| a > 0.0);
            let handles_on = (handles_live || fading_out)
                && window_takes_handles(self_ptr)
                && !is_virtual_border
                && bw > 0
                && (cw as f64 * sc) >= 12.0
                && (ch as f64 * sc) >= 12.0;
            if !handles_on {
                // Nothing is drawn, so nothing is stale: without this the
                // fade step would see a mismatch and repaint every tick.
                self.border_hover_drawn = self.hovered_border_element;
                for r in [&self.border.left, &self.border.right, &self.border.top, &self.border.bottom] {
                    r.set_enabled(false);
                }
                for seg in self.border.segments.iter() {
                    seg.set_enabled(false);
                }
                self.border.frame.set_enabled(false);
                return;
            }

            // The band is a SCREEN width, not a world one. Handles exist only
            // in overview, which is zoomed OUT, so a band that scaled with the
            // window would be at its thinnest exactly where it is the only way
            // to resize — 16px becomes 7 at a typical overview zoom, and the
            // thin corners 2.5. `apply` scales the boxes it is given, so the
            // catchers are sized in unscaled units that come back to
            // `band_screen` on screen. cursor::get_border_zone measures the
            // same width in layout px; the two must agree.
            // Screen thickness, but never more than a fifth of the smaller
            // on-screen side: a zoomed-out window would otherwise be mostly
            // ring. Shrinking beats the old hard cutoff, which dropped the
            // handles altogether below a threshold — a window you cannot
            // resize at all is worse than one with a slimmer grip.
            let short_side = (cw.min(ch) as f64 * sc).max(1.0);
            let band_screen = (layout_handle_w as f64)
                .max(crate::window::HOVER_BAND_MIN)
                .min(short_side * 0.2);
            let px = |v: i32| (v as f64 * sc) as i32;

            // The band catchers are retired: between two discs the pointer
            // must reach the app, not a catcher.
            for r in [&self.border.left, &self.border.right, &self.border.top, &self.border.bottom] {
                r.set_enabled(false);
            }

            let layout = &(*crate::reentry::wm(self.server)).layout;
            // The discs sit inside the window's own silhouette, so the
            // corner discs place against the content radius (the widened
            // root plate radius the corner clip uses).
            let r_in = bg_radius;

            // Hit catchers: one transparent square under each disc, laid out
            // by the same function the hit test uses. `apply` takes unscaled
            // boxes, so the screen-px layout is divided back out (a px of
            // rounding on an invisible catcher is nothing).
            let buttons = frame_buttons_value(self_ptr);
            let (centres, disc_r, live) = handle_disc_layout(
                cw as f64 * sc,
                ch as f64 * sc,
                px(r_in) as f64,
                band_screen,
                buttons > 0.0,
            );
            for (i, &(cx, cy)) in centres.iter().enumerate() {
                if i >= live {
                    self.border.segments[i].set_enabled(false);
                    continue;
                }
                let b = ffi::wlr_box {
                    x: ((cx - disc_r) / sc).floor() as i32,
                    y: ((cy - disc_r) / sc).floor() as i32,
                    width: (2.0 * disc_r / sc).ceil() as i32,
                    height: (2.0 * disc_r / sc).ceil() as i32,
                };
                apply(self.border.segments[i].raw(), b, &transparent, handles_live);
            }
            // corner_len and gap are retired by the wave profile (the
            // valleys place the seams now, a quarter along each side) and
            // ignored by the shader; still passed so the node API holds.
            let cl = border_corner_len(bw as f64, layout.border_corner_length, r_in as f64) as i32;
            let g = layout.border_segment_gap;

            // Handles rest invisible and fade in with the mode; one alpha for
            // the whole ring now that every zone reveals together in overview
            // (`step_border_fade`'s all_on branch). The Top slot carries it —
            // they are all equal while the ring is up, and taking one keeps
            // the fade a single number.
            let a = self.border_reveal[BorderElement::Top.index()].clamp(0.0, 1.0);
            let premul = |c: &[f32; 4]| [c[0] * a, c[1] * a, c[2] * a, c[3] * a];

            self.border.frame.set_size(px(cw), px(ch));
            self.border.frame.set_corner_radius(px(r_in));
            // band is the disc diameter; the rest is retired by the discs and
            // ignored by the shader, still passed so the node API holds.
            self.border.frame.set_shape(band_screen as f32, (band_screen as f32 * layout.border_taper.clamp(0.0, 1.0)).max(2.0), (px(cl) as f64).max(band_screen) as f32, px(g) as f32, layout.border_swell_curve, (layout.border_corner_bulge as f64).min(short_side * 0.3) as f32);
            // The popover hint arrives in surface-local LOGICAL px; the node
            // space is zoom-scaled device px like everything else here, so it
            // takes the same px() mapping. Zeroed when clear.
            let ex = match self.popover_region {
                Some(r) => [
                    px(r.x) as f32,
                    px(r.y) as f32,
                    px(r.width) as f32,
                    px(r.height) as f32,
                ],
                None => [0.0; 4],
            };
            self.border.frame.set_exclusion(&ex);
            self.border.frame.set_buttons(buttons);
            // The ring exists only for the SEAT-focused window — `handles_on`
            // above says so, and so does step_border_fade's reveal — so it
            // paints in the focused color, taken from the layout rather than
            // from `requested.border.color`.
            //
            // That field is a PLAN value, written by the arrange pass from the
            // focus it saw when it ran, and a focus change only schedules an
            // arrange when the newly focused window happens to be Floating
            // (Seat::focus). Every other focus change armed the border fade
            // and nothing else, so the ring eased in wearing the UNFOCUSED
            // color: hovering a tiled window in overview drew its resize ring
            // in the plain border gray instead of the focus color, and it
            // stayed gray until some unrelated transaction refreshed the plan.
            // Whether the ring is drawn and what color it is are the same
            // fact — focused — so both now read it live, the way update_bevel
            // already reads focus off the seats for the rim highlight.
            //
            // `window_background` above keeps the plan color: that one is the
            // window's own plate, not this compositor-drawn handle.
            self.border.frame.set_color(&premul(&layout.border_color_focused));
            let hovered = self
                .hovered_border_element
                .map(|e| e.index() as f32)
                .unwrap_or(-1.0);
            self.border_hover_drawn = self.hovered_border_element;
            self.border.frame.set_hover(hovered, &premul(&border.hover_color));
            self.border.frame.set_position_if_changed(0, 0);
            self.border.frame.set_enabled(a > 0.0);
        }
    }

    #[allow(unused_assignments)]
    pub unsafe fn apply_surface_clip(&mut self, a: *const ffi::wlr_box, b: *const ffi::wlr_box) {
        let mut surface_clip = std::mem::zeroed::<ffi::wlr_box>();
        let a_empty = (*a).width == 0 && (*a).height == 0;
        let b_empty = (*b).width == 0 && (*b).height == 0;

        let layout_box = ffi::wlr_box {
            x: 0,
            y: 0,
            width: self.box_geom.width,
            height: self.box_geom.height,
        };

        if !a_empty && !b_empty {
            let mut temp_clip = std::mem::zeroed::<ffi::wlr_box>();
            if !ffi::wlr_box_intersection(&mut temp_clip, a, b) {
                self.surfaces.set_enabled(false);
                return;
            }
            if !ffi::wlr_box_intersection(&mut surface_clip, &temp_clip, &layout_box) {
                self.surfaces.set_enabled(false);
                return;
            }
        } else if !a_empty {
            if !ffi::wlr_box_intersection(&mut surface_clip, a, &layout_box) {
                self.surfaces.set_enabled(false);
                return;
            }
        } else if !b_empty {
            if !ffi::wlr_box_intersection(&mut surface_clip, b, &layout_box) {
                self.surfaces.set_enabled(false);
                return;
            }
        } else {
            surface_clip = layout_box;
        }

        self.surfaces.set_enabled(true);
        let margin = 0;
        surface_clip.x -= margin;
        surface_clip.y -= margin;
        surface_clip.width += 2 * margin;
        surface_clip.height += 2 * margin;

        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if !toplevel.is_null() {
                    let x = if self.wm_requested.ssd { 0 } else { (*toplevel).geometry.x };
                    let y = if self.wm_requested.ssd { 0 } else { (*toplevel).geometry.y };
                    surface_clip.x += x;
                    surface_clip.y += y;
                }
            }
            _ => {}
        }

        // Crop a CSD toplevel to its xdg geometry. Chromium-family clients
        // paint a translucent shadow band outside the geometry whenever they
        // are not maximized; the compositor draws its own shadow, and it
        // rounds corners per buffer at the buffer's edge, so uncropped the
        // rounding fell in that band and the visible window read
        // square-cornered (an Electron window un-tiled by a fullscreen round
        // trip). A geometry clip was set once and nulled in 34b3ae64: the
        // scaling passes rewrote every buffer's dest size from the full
        // surface each commit and stretched the crop back out — they go
        // through surface_buffer_extent now. Skipped while a cce-ui client
        // has a popover overhanging its geometry (set_popover_region): that
        // rim is live menu content, not a shadow. And skipped mid
        // fullscreen-toggle, where the animation owns the buffers' stretch.
        let mut crop = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };
        if let WindowImpl::Toplevel(toplevel) = self.impl_type {
            if !toplevel.is_null()
                && !self.wm_requested.ssd
                && self.popover_region.is_none()
                && self.fs_anim.is_none()
            {
                crop = (*toplevel).geometry;
            }
        }
        let clip = (crop.width > 0 && crop.height > 0).then_some(&crop);
        let children_head = ffi::river_scene_tree_get_children(self.surfaces.tree.raw()) as *mut WlList;
        if (*children_head).next != children_head {
            self.surfaces.tree.set_subsurface_clip(clip);
        }
    }
}

/// Does this window get resize handles at all?
///
/// The single answer for both halves — `cursor::get_border_zone`'s hit test
/// and `draw_borders`' visuals — so a window can never show a handle it
/// would not honour, or honour one it does not show. Excluded: the internal
/// roles that are not user-geometry (Popup, Fullscreen, Status), Utility
/// (self-sizing by definition — the client owns its size), circular windows
/// (no rectangular ring to hug), and hidden ones.
pub unsafe fn window_takes_handles(window: *mut Window) -> bool {
    !matches!(
        (*window).tiling_mode,
        crate::tiling::TilingMode::Popup
            | crate::tiling::TilingMode::Fullscreen
            | crate::tiling::TilingMode::Status
            | crate::tiling::TilingMode::Utility
    ) && !(*window).rendering_requested.circular
        && !(*window).rendering_requested.hidden
}

/// Does this window get the minimize / maximize / float-tile buttons beside
/// its top-right handle? Only a window with handles, and only a Floating or
/// Tiled one: those are the modes the toggle flips between, and an Overlay
/// dock has no business being minimized or tiled from its chrome. Asked by
/// `draw_borders` and `cursor::get_border_zone` alike, like
/// [`window_takes_handles`].
pub unsafe fn window_takes_buttons(window: *mut Window) -> bool {
    window_takes_handles(window)
        && matches!(
            (*window).tiling_mode,
            crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Tiled
        )
}

/// The frame shader's `buttons` value for `window`: 0 none, 1 Floating,
/// 2 Tiled (the toggle's glyph shows the mode a click goes to).
pub(crate) unsafe fn frame_buttons_value(window: *mut Window) -> f32 {
    if !window_takes_buttons(window) {
        0.0
    } else if (*window).tiling_mode == crate::tiling::TilingMode::Tiled {
        2.0
    } else {
        1.0
    }
}
