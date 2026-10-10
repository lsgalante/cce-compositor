//! A window's effects on the scene: the root-plate radius, shadow, bevel,
//! droplet and backdrop compression, the open/close fade, the adjust dim, the
//! border-reveal fade and the fullscreen-toggle animation, and the border band's
//! extents. Split out of window.rs on 2026-10-10.

use super::*;

impl Window {
    /// The root plate / content-clip corner radius in logical px, before span
    /// widening (`widen_corner_radius`). THE source for every writer of that
    /// radius — `set_rendering_state`, `render_viewport_update`, the toplevel
    /// commit path in `xdg_toplevel.rs` and `draw_borders` — where until
    /// 2026-09-28 the first three each kept an inline copy of this decision
    /// "mirrored" by comment. They disagreed once before: draw_borders applied
    /// the BORDER ring's radius to the root plate node and, running last,
    /// silently overrode the value set_rendering_state had just written,
    /// making `root_plate_corner_radius` dead config.
    pub unsafe fn root_plate_radius_base(&self) -> i32 {
        if self.is_fullscreen() {
            return 0;
        }
        if self.rendering_requested.circular {
            let w = self.rendering_sent.width as i32;
            let h = self.rendering_sent.height as i32;
            return w.min(h) / 2;
        }
        let app_id = self.get_app_id_string().unwrap_or_default();
        let is_status = self.tiling_mode == crate::tiling::TilingMode::Status
            || app_id.starts_with("cce-status");
        if is_status {
            return 0;
        }
        let is_decorated = (*self.server).wm.is_decorated_app(&app_id);
        if self.wm_requested.ssd || is_decorated {
            (*self.server).wm.layout.root_plate_corner_radius
        } else {
            0
        }
    }

    /// Sync the drop shadow with the current geometry. `width`/`height` are the
    /// content size in device px, `radius` the corner radius in logical px (as
    /// computed for the blur/rounding paths). scenefx's box-shadow shader draws
    /// the shadow of a box inset by sigma on all sides of the node box, so the
    /// node is padded by sigma and offset so the casting box lands exactly on
    /// the window, displaced by the configured offset — which should point away
    /// from the light (down-right for the DE's default top-left light). The
    /// window's own box is punched out via the clipped region so the shadow
    /// darkens only the desktop around the window, never the (translucent)
    /// window itself.
    pub unsafe fn update_shadow(&self, width: i32, height: i32, radius: i32, want: bool) {
        if self.shadow.is_null() {
            return;
        }
        let node = &mut (*self.shadow).node as *mut ffi::wlr_scene_node;
        let layout = &(*self.server).wm.layout;
        let enabled = want && layout.shadow_enabled && width > 0 && height > 0;
        ffi::wlr_scene_node_set_enabled(node, enabled);
        if !enabled {
            return;
        }
        let sigma = (layout.shadow_sigma as f64 * self.scale) as f32;
        let pad = sigma.ceil() as i32;
        let ox = (layout.shadow_offset_x as f64 * self.scale) as i32;
        let oy = (layout.shadow_offset_y as f64 * self.scale) as i32;
        let radius_dev = (radius as f64 * self.scale) as i32;
        ffi::wlr_scene_shadow_set_color(self.shadow, layout.shadow_color.as_ptr());
        ffi::wlr_scene_shadow_set_blur_sigma(self.shadow, sigma);
        ffi::wlr_scene_shadow_set_corner_radius(self.shadow, radius_dev);
        ffi::wlr_scene_shadow_set_size(self.shadow, width + 2 * pad, height + 2 * pad);
        ffi::river_scene_node_set_position_if_changed(node, -pad + ox, -pad + oy);
        let r = radius_dev.clamp(0, u16::MAX as i32) as u16;
        ffi::wlr_scene_shadow_set_clipped_region(self.shadow, ffi::clipped_region {
            area: ffi::wlr_box { x: pad - ox, y: pad - oy, width, height },
            corners: ffi::fx_corner_radii {
                top_left: r, top_right: r, bottom_right: r, bottom_left: r,
            },
        });
    }

    /// Sync the edge bevel with the current geometry. `width`/`height` are the
    /// content size in device px and `radius` the corner radius in logical px,
    /// exactly as `update_shadow` takes them. The rim is drawn INSIDE that box
    /// (see the shader), so it overlays the client's outermost pixels and needs
    /// no room of its own.
    ///
    /// The light direction is the DE's convention — the same top-left source
    /// the drop shadow is offset away from — so a window reads as a slab lit
    /// from the same place as everything else on the desktop.
    /// Is this window any seat's keyboard focus? The window's `activated`
    /// field is a configure-time snapshot, not live state, so live answers
    /// come from the seats.
    pub unsafe fn is_seat_focused(&self) -> bool {
        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            if let crate::seat::Focus::Window(w) = (*seat).focused {
                if w == self as *const Window as *mut Window {
                    return true;
                }
            }
            curr = (*curr).next;
        }
        false
    }

    /// Whether this is the window the adjust-mode handles belong to: the
    /// toplevel under some seat's pointer (`Cursor::adjust_hover`), focused
    /// or not, in overview and with Super held alike — the handles are
    /// shown on what the pointer is over and on nothing else, so a pointer
    /// on the background shows none. (Overview's hover-to-focus still moves
    /// focus with the pointer, but focus is not what the ring keys on: a
    /// pointer resting on the background would otherwise keep the last
    /// window's ring up.) The reveal (`step_border_fade`), the drawn handles
    /// and catchers (`draw_borders`) and the hit test
    /// (`cursor::get_border_zone`) all ask this one predicate, so the ring
    /// cannot be drawn on one window and grabbed on another.
    pub unsafe fn is_adjust_target(&self) -> bool {
        let me = self as *const Window as *mut Window;
        let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            if (*seat).cursor.adjust_hover == me {
                return true;
            }
            curr = (*curr).next;
        }
        false
    }

    pub unsafe fn update_bevel(&self, width: i32, height: i32, radius: i32, want: bool, want_focus: bool) {
        if self.bevel.is_null() {
            return;
        }
        let node = &mut (*self.bevel).node as *mut ffi::wlr_scene_node;
        let layout = &(*self.server).wm.layout;
        // Focused-window treatment: the rim highlight wraps all four sides
        // in the accent (the DE focus glint). Focus is read off the seats —
        // the window's `activated` field is a configure-time snapshot, not
        // live state — and this runs on both render paths, so a focus switch
        // restyles on the next frame. A focused window that is NOT in the
        // bevel app list still enables the node: the shader's focus branch
        // draws ONLY the glint, so it lays cleanly over a cce-ui app's own
        // client-side bevel instead of doubling its shading.
        let focused = self.is_seat_focused();
        let enabled = (want || (focused && want_focus))
            && layout.bevel_enabled
            && layout.bevel_thickness > 0.0
            && width > 0
            && height > 0;
        ffi::wlr_scene_node_set_enabled(node, enabled);
        if !enabled {
            return;
        }

        // Device px, like the blur radius and shadow sigma: the content is
        // scaled to its dest size, so an unscaled rim would keep its zoom-1
        // width while the window shrinks.
        let thickness = (layout.bevel_thickness as f64 * self.scale) as f32;
        let radius_dev = (radius as f64 * self.scale) as i32;

        // Light from the top-left, matching shadow_offset_x/y pointing away
        // from it. Normalized here so the shader can take it as-is.
        let (lx, ly) = (layout.bevel_light_x, layout.bevel_light_y);
        let len = (lx * lx + ly * ly).sqrt();
        let (lx, ly) = if len > 1e-6 { (lx / len, ly / len) } else { (-0.7071, -0.7071) };

        ffi::wlr_scene_bevel_set_size(self.bevel, width, height);
        ffi::wlr_scene_bevel_set_corner_radius(self.bevel, radius_dev);
        ffi::wlr_scene_bevel_set_thickness(self.bevel, thickness.max(1.0));
        ffi::wlr_scene_bevel_set_light(
            self.bevel,
            lx,
            ly,
            layout.bevel_light_intensity,
            layout.bevel_shade_intensity,
        );
        ffi::wlr_scene_bevel_set_shoulder(self.bevel, layout.bevel_shoulder);
        ffi::wlr_scene_bevel_set_color(self.bevel, layout.bevel_color.as_ptr());
        ffi::wlr_scene_bevel_set_focus(
            self.bevel,
            if focused { 1.0 } else { 0.0 },
            layout.bevel_focus_sharpness,
            layout.bevel_focus_color.as_ptr(),
        );
        ffi::river_scene_node_set_position_if_changed(node, 0, 0);
    }
    /// Push the status bar's backdrop compression (`module {
    /// backdrop_compress }`) onto this segment's backdrop: the blur node and
    /// the droplet lens, whichever is live. Off for everything that is not a
    /// status segment. Call after `river_scene_node_enable_blur`, which can
    /// recreate the blur node with compression off — from every path that
    /// calls it for a status segment.
    pub unsafe fn sync_backdrop_compress(&self) {
        let is_status = self.tiling_mode == crate::tiling::TilingMode::Status;
        let (ceil, knee, invert) = if is_status {
            (*self.server).wm.layout.status_backdrop_compress.unwrap_or((0.0, 0.0, false))
        } else {
            (0.0, 0.0, false)
        };
        ffi::river_scene_node_set_blur_compress(self.tree as *mut ffi::wlr_scene_node, ceil, knee, invert);
        if !self.droplet.is_null() {
            ffi::wlr_scene_droplet_set_compress(self.droplet, ceil, knee, invert);
        }
    }
    /// Sync the droplet backdrop-refraction node for a droplet-styled status
    /// segment. Called from BOTH render paths, like update_bevel — one-path
    /// effects freeze at the pre-gesture zoom (the shadow's old trap).
    pub unsafe fn update_droplet(&self, width: i32, height: i32) {
        if self.droplet.is_null() {
            return;
        }
        let node = &mut (*self.droplet).node as *mut ffi::wlr_scene_node;
        let layout = &(*self.server).wm.layout;
        let is_status = self.tiling_mode == crate::tiling::TilingMode::Status;
        // Only bar-strip segments: an expanded (menu) segment is taller than
        // the bar and draws its own grown drop client-side — refracting the
        // collapsed silhouette beneath it would be wrong.
        let enabled = is_status
            && layout.status_droplet.is_some()
            && width > 0
            && height > 0
            && height <= layout.bar_height as i32;
        if !enabled {
            ffi::wlr_scene_node_set_enabled(node, false);
            return;
        }
        let spec = cce_core::droplet::DropletSpec::parse(
            layout.status_droplet.as_deref().unwrap_or(""),
        );
        if spec.refr <= 0.0 && spec.ghost <= 0.0 {
            ffi::wlr_scene_node_set_enabled(node, false);
            return;
        }
        ffi::wlr_scene_node_set_enabled(node, true);

        // Match the client's drop box: inset 1px from the surface bottom.
        // Camera-zoom scaling like the bevel; output scale is applied by the
        // render pass itself.
        let w = width as f32;
        let h = (height as f32 - 1.0).max(1.0);
        let (sr, ar, bow) = spec.resolve_silhouette(w, h);
        let k = (spec.blend.max(0.0) * h).max(1.0);
        let band = (spec.band.max(0.05) * h).max(1.0);
        let zs = self.scale as f32;
        ffi::wlr_scene_droplet_set_size(self.droplet, width, height);
        ffi::wlr_scene_droplet_set_silhouette(
            self.droplet,
            ar * zs,
            sr * zs,
            bow * zs,
            k * zs,
            spec.curve.clamp(2.0, 6.0),
        );
        ffi::wlr_scene_droplet_set_lens(self.droplet, band * zs, spec.refr * zs, spec.ghost.clamp(0.0, 1.0));
        ffi::river_scene_node_set_position_if_changed(node, 0, 0);
    }
    /// True when this status segment's droplet backdrop node is live. The
    /// per-window blur must yield to it: the blur pass would composite the
    /// UNREFRACTED cached backdrop over the lens output.
    pub unsafe fn droplet_backdrop_on(&self) -> bool {
        if self.droplet.is_null() || self.tiling_mode != crate::tiling::TilingMode::Status {
            return false;
        }
        match (*self.server).wm.layout.status_droplet.as_deref() {
            Some(raw) => {
                let spec = cce_core::droplet::DropletSpec::parse(raw);
                spec.refr > 0.0 || spec.ghost > 0.0
            }
            None => false,
        }
    }



    /// The opacity the scene tree gets: the requested one, scaled down by the
    /// adjust-mode overlap dim (`adjust_dim`, 0..1) toward
    /// `border.overlap_opacity`, and again by the map/close fade
    /// (`map_fade`), which rests at 1.0 whenever no fade is in flight.
    pub unsafe fn effective_opacity(&self) -> f32 {
        let floor = (*self.server).wm.layout.border_overlap_opacity;
        self.rendering_requested.opacity
            * (1.0 - self.adjust_dim.clamp(0.0, 1.0) * (1.0 - floor))
            * self.map_fade.clamp(0.0, 1.0)
    }

    /// Whether this window takes the map/close fade at all. Surfaces that are
    /// part of the desktop itself rather than something the user opened — the
    /// status segments, the wallpaper, the grid layer — are left alone: they
    /// map once at login and a dissolve there reads as the desktop failing to
    /// draw. Same exclusion list `adjust_dim_wanted` uses, for the same
    /// reason: these are not windows the user thinks of as opening.
    pub unsafe fn wants_map_fade(&self) -> bool {
        !self.is_status_bar() && !self.is_wallpaper() && !self.is_grid()
    }

    /// Begin a fade toward `target` (0.0 out, 1.0 in) over `ms`, and arm the
    /// timer that steps it. A `ms` of 0 (or fading disabled) snaps instead,
    /// so every caller can treat this as "put the window at `target`".
    pub unsafe fn start_map_fade(&mut self, target: f32, ms: u32) {
        self.map_fade_target = target.clamp(0.0, 1.0);
        if ms == 0 || !self.wants_map_fade() {
            self.map_fade = self.map_fade_target;
            ffi::river_scene_node_set_opacity(
                self.tree as *mut ffi::wlr_scene_node,
                self.effective_opacity(),
            );
            return;
        }
        // Ticks at 16 ms; at least one step, so a sub-frame duration still
        // lands on the target rather than dividing by zero.
        let ticks = ((ms as f32) / 16.0).max(1.0);
        self.map_fade_step = ((self.map_fade_target - self.map_fade).abs() / ticks).max(1.0e-4);
        ffi::river_scene_node_set_opacity(
            self.tree as *mut ffi::wlr_scene_node,
            self.effective_opacity(),
        );
        (*self.server).wm.arm_border_fade();
    }

    /// Advance the map/close fade one tick toward `map_fade_target`, applying
    /// the opacity as it goes. Returns true while still in motion, like
    /// `step_adjust_dim`.
    pub unsafe fn step_map_fade(&mut self) -> bool {
        let delta = self.map_fade_target - self.map_fade;
        if delta.abs() <= self.map_fade_step {
            if self.map_fade == self.map_fade_target {
                return false;
            }
            self.map_fade = self.map_fade_target;
        } else {
            self.map_fade += self.map_fade_step * delta.signum();
        }
        ffi::river_scene_node_set_opacity(
            self.tree as *mut ffi::wlr_scene_node,
            self.effective_opacity(),
        );
        true
    }

    /// Whether this window should be dimmed right now: adjust mode is on,
    /// this is a Floating window, and it lies ABOVE the adjust target in the
    /// render stack while overlapping it on screen — where it would cover
    /// the target's handles. Windows under the target are left alone; they
    /// hide nothing.
    pub unsafe fn adjust_dim_wanted(&self) -> bool {
        let wm = &(*self.server).wm;
        if !wm.window_adjust_active()
            || self.closed
            || self.tiling_mode != crate::tiling::TilingMode::Floating
            || self.is_status_bar()
            || self.is_wallpaper()
            || self.is_grid()
        {
            return false;
        }
        let me = self as *const Window as *mut Window;
        let on_screen = |w: *mut Window| -> (f64, f64, f64, f64) {
            let sc = if (*w).scale > 0.0 { (*w).scale } else { 1.0 };
            let g = (*w).box_geom;
            (g.x as f64, g.y as f64, g.width as f64 * sc, g.height as f64 * sc)
        };
        let (mx, my, mw, mh) = on_screen(me);
        // The render list runs bottom to top (raise_window moves to the
        // tail), so a target met before this window sits beneath it.
        let list = &wm.rendering_requested.list as *const ffi::wl_list as *mut WlList;
        let mut curr = (*list).next;
        let mut covered = false;
        while curr != list {
            let node = crate::container_of!(curr, crate::wm_node::WmNode, link);
            let w = (*node).window();
            if w == me {
                return covered;
            }
            if !w.is_null()
                && !(*w).closed
                && window_takes_handles(w)
                && (*w).is_adjust_target()
            {
                let (tx, ty, tw, th) = on_screen(w);
                if mx < tx + tw && tx < mx + mw && my < ty + th && ty < my + mh {
                    covered = true;
                }
            }
            curr = (*curr).next;
        }
        false
    }

    /// Advance the overlap dim one tick toward where `adjust_dim_wanted`
    /// says it should rest, applying the opacity as it goes. Returns true
    /// while still in motion, like `step_border_fade`.
    pub unsafe fn step_adjust_dim(&mut self) -> bool {
        let target = if self.adjust_dim_wanted() { 1.0 } else { 0.0 };
        let delta = target - self.adjust_dim;
        let moving;
        if delta.abs() <= BORDER_FADE_EPSILON {
            if self.adjust_dim == target {
                return false;
            }
            self.adjust_dim = target;
            moving = false;
        } else {
            self.adjust_dim += delta * border_fade_step();
            moving = true;
        }
        ffi::river_scene_node_set_opacity(self.tree as *mut ffi::wlr_scene_node, self.effective_opacity());
        moving
    }

    /// Advance the hover fade one tick. Every zone eases toward 1.0 if it is
    /// the one under the pointer and 0.0 otherwise. Returns true while any
    /// zone is still in motion, so the caller knows to schedule another tick.
    pub unsafe fn step_border_fade(&mut self) -> bool {
        let mut moving = false;
        let mut changed = false;
        // The adjust TARGET — the window under the pointer — shows its whole
        // ring for as long as the mode is on; other windows show nothing.
        // The ring follows the pointer from window to window, each swap
        // easing through this same fade. Hover still reads through on the revealed ring, as
        // `color_for` paints the hovered zone in hover_color over the full
        // reveal.
        let all_on = (*self.server).wm.window_adjust_active()
            && window_takes_handles(self as *mut Window)
            && self.is_adjust_target();
        for elem in BorderElement::ALL {
            let i = elem.index();
            let target = if all_on || self.hovered_border_element == Some(elem) { 1.0 } else { 0.0 };
            let delta = target - self.border_reveal[i];
            if delta.abs() <= BORDER_FADE_EPSILON {
                if self.border_reveal[i] != target {
                    self.border_reveal[i] = target;
                    changed = true;
                }
                continue;
            }
            self.border_reveal[i] += delta * border_fade_step();
            moving = true;
            changed = true;
        }
        // A hover swap on a fully revealed ring moves nothing above, but the
        // shader still has to be told which zone to paint.
        if self.hovered_border_element != self.border_hover_drawn {
            changed = true;
        }
        if changed {
            self.draw_borders();
        }
        moving
    }

    /// The black backdrop under a fullscreen surface, at the output's size
    /// — scaled with the window when it sits on a zoomed-out desk, since
    /// the surface's buffers shrink with `scale` and the backdrop does not.
    pub(crate) unsafe fn size_fullscreen_background(&mut self, width: i32, height: i32) {
        let s = if self.fs_on_desk { self.scale } else { 1.0 };
        ffi::wlr_scene_rect_set_size(
            self.fullscreen_background,
            (width as f64 * s).round() as i32,
            (height as f64 * s).round() as i32,
        );
    }

    /// The output a fullscreen window fills: the one the WM pinned it to, or
    /// the first enabled output (the same fallback manage/render use).
    pub unsafe fn fullscreen_output(&self) -> *mut crate::output::Output {
        if !self.wm_requested.fullscreen.is_null() {
            return self.wm_requested.fullscreen;
        }
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs_list).next;
        while curr != outputs_list {
            let out = crate::container_of!(curr, crate::output::Output, link);
            if (*out).sent.state == crate::output::OutputStateValue::Enabled {
                return out;
            }
            curr = (*curr).next;
        }
        std::ptr::null_mut()
    }

    /// Arms the fullscreen-toggle animation at the window's current on-screen
    /// rect. Called from manage_finish on the enter/exit transition, before
    /// the settled geometry is rewritten; a re-toggle mid-flight continues
    /// from wherever the previous animation had reached. Sized with
    /// last_applied_scale (the scale actually drawn) because self.scale has
    /// already been rewritten to the destination state's scale by the arrange
    /// pass in this same cycle.
    pub(crate) unsafe fn start_fs_anim(&mut self) {
        // Animations off: the window is simply drawn at its new rect.
        if !cce_core::motion::enabled() {
            self.fs_anim = None;
            return;
        }
        if !matches!(self.impl_type, WindowImpl::Toplevel(_))
            || !matches!(self.state, WindowState::Mapped)
            || self.box_geom.width <= 0
            || self.box_geom.height <= 0
        {
            return;
        }
        let (x, y, w, h) = if let Some(a) = self.fs_anim {
            (a.x, a.y, a.w, a.h)
        } else {
            let s = if self.last_applied_scale > 0.0 { self.last_applied_scale } else { 1.0 };
            (
                self.box_geom.x as f64,
                self.box_geom.y as f64,
                self.box_geom.width as f64 * s,
                self.box_geom.height as f64 * s,
            )
        };
        self.fs_anim = Some(FsAnim { x, y, w, h, moved: false, ticks: 0 });
        (*self.server).wm.arm_border_fade();
    }

    /// One tick of the fullscreen-toggle animation. Returns true while the
    /// caller should re-render (including the final settling frame). The
    /// target rect is recomputed live every tick — the output box while
    /// fullscreen, else the arranged position at the last configured size —
    /// so it tracks the client's asynchronous resize instead of freezing a
    /// stale goal on the first frame.
    pub unsafe fn step_fs_anim(&mut self) -> bool {
        let Some(mut anim) = self.fs_anim else {
            return false;
        };
        // Switched off mid-flight: land now, with the settling frame.
        if !cce_core::motion::enabled() {
            self.fs_anim = None;
            return true;
        }

        let (tx, ty, tw, th) = if self.is_fullscreen() {
            let output = self.fullscreen_output();
            if output.is_null() {
                self.fs_anim = None;
                return true;
            }
            let (w, h) = (*output).sent.dimensions();
            if self.fs_on_desk {
                // Toggled in overview: it grows into its slab on the desk
                // (`place_fullscreen_windows`), not over the whole output.
                (
                    self.rendering_requested.x as f64,
                    self.rendering_requested.y as f64,
                    w as f64 * self.scale,
                    h as f64 * self.scale,
                )
            } else {
                ((*output).sent.x as f64, (*output).sent.y as f64, w as f64, h as f64)
            }
        } else {
            let w = self.configure_sent.width.map(|w| w as i32).unwrap_or(self.box_geom.width);
            let h = self.configure_sent.height.map(|h| h as i32).unwrap_or(self.box_geom.height);
            (
                self.rendering_requested.x as f64,
                self.rendering_requested.y as f64,
                w as f64 * self.scale,
                h as f64 * self.scale,
            )
        };

        anim.ticks += 1;
        let dx = tx - anim.x;
        let dy = ty - anim.y;
        let dw = tw - anim.w;
        let dh = th - anim.h;
        let settled = dx.abs() < FS_ANIM_EPSILON
            && dy.abs() < FS_ANIM_EPSILON
            && dw.abs() < FS_ANIM_EPSILON
            && dh.abs() < FS_ANIM_EPSILON;
        if !settled {
            anim.moved = true;
        }
        if (settled && anim.moved) || anim.ticks > FS_ANIM_MAX_TICKS {
            self.fs_anim = None;
            return true;
        }
        anim.x += dx * FS_ANIM_STEP;
        anim.y += dy * FS_ANIM_STEP;
        anim.w += dw * FS_ANIM_STEP;
        anim.h += dh * FS_ANIM_STEP;
        self.fs_anim = Some(anim);
        true
    }

    /// Outward extent (unscaled px) the interactive border may reach on each
    /// side — `[left, right, top, bottom]` — after the foam rule against the
    /// other windows: where two windows' bands would overlap across a gap,
    /// each band stops at the gap's midline (the ramp key-ring behavior,
    /// rectangular — the wall is equidistant from the two content edges).
    /// Stacked windows (content rects overlapping) do not clip each other,
    /// mirroring the rings' degenerate-distance guard. Per-side, not
    /// per-span: one near neighbor claims the whole facing side.
    pub unsafe fn border_side_extents(&self, band_unscaled: f64) -> [f64; 4] {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let band = band_unscaled * scale;
        let ax0 = self.box_geom.x as f64;
        let ay0 = self.box_geom.y as f64;
        let ax1 = ax0 + self.box_geom.width as f64 * scale;
        let ay1 = ay0 + self.box_geom.height as f64 * scale;
        let mut ext = [band; 4]; // left, right, top, bottom (layout px)

        let self_ptr = self as *const Window as *mut Window;
        for &other in (*self.server).wm.windows.iter() {
            if other.is_null() || other == self_ptr {
                continue;
            }
            let o = &*other;
            if o.closed
                || o.minimized
                || o.rendering_requested.hidden
                || o.rendering_requested.circular
                || matches!(
                    o.tiling_mode,
                    crate::tiling::TilingMode::Popup
                        | crate::tiling::TilingMode::Fullscreen
                        | crate::tiling::TilingMode::Status
                )
                || o.is_status_bar()
                || o.is_wallpaper()
            {
                continue;
            }
            let os = if o.scale > 0.0 { o.scale } else { 1.0 };
            let bx0 = o.box_geom.x as f64;
            let by0 = o.box_geom.y as f64;
            let bx1 = bx0 + o.box_geom.width as f64 * os;
            let by1 = by0 + o.box_geom.height as f64 * os;
            // Stacked: keep the full band.
            if bx0 < ax1 && bx1 > ax0 && by0 < ay1 && by1 > ay0 {
                continue;
            }
            let ob = border_band_width(o.rendering_requested.border.width) * os;
            // Spans (including bands) must overlap for a wall to exist.
            let v_overlap = by0 - ob < ay1 + band && by1 + ob > ay0 - band;
            let h_overlap = bx0 - ob < ax1 + band && bx1 + ob > ax0 - band;
            if v_overlap {
                if bx0 >= ax1 {
                    let gap = bx0 - ax1;
                    if gap < band + ob {
                        ext[1] = ext[1].min((gap / 2.0).max(0.0));
                    }
                } else if bx1 <= ax0 {
                    let gap = ax0 - bx1;
                    if gap < band + ob {
                        ext[0] = ext[0].min((gap / 2.0).max(0.0));
                    }
                }
            }
            if h_overlap {
                if by0 >= ay1 {
                    let gap = by0 - ay1;
                    if gap < band + ob {
                        ext[3] = ext[3].min((gap / 2.0).max(0.0));
                    }
                } else if by1 <= ay0 {
                    let gap = ay0 - by1;
                    if gap < band + ob {
                        ext[2] = ext[2].min((gap / 2.0).max(0.0));
                    }
                }
            }
        }
        [ext[0] / scale, ext[1] / scale, ext[2] / scale, ext[3] / scale]
    }
}
