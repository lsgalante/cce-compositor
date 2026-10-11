// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::scene_node_data::{SceneNodeData, SceneNodeDataVal};
use crate::scene_handle::SceneTree;

pub struct SceneLayers {
    pub background: SceneTree,
    /// Client-provided backgrounds — wlr-layer-shell Background surfaces and
    /// the `cce-wallpaper` window — inside `background`, ABOVE each output's
    /// base rect and grid backdrop and BELOW its grid cells (`Output::draw_grid`
    /// keeps the backdrop under this tree and the cell tree above it). A client
    /// wallpaper therefore replaces the flat backdrop colour and keeps the cell
    /// lattice; before this tree the grid tree, re-raised on every redraw, buried
    /// every client background under its opaque backdrop.
    pub background_clients: SceneTree,
    pub bottom: SceneTree,
    pub wm: SceneTree,
    pub top: SceneTree,
    pub fullscreen: SceneTree,
    pub overlay: SceneTree,
    pub popups: SceneTree,
    pub override_redirect: SceneTree,
    /// Hover-revealed window borders. Borders draw outside the content box, so
    /// with the content filling its grid cell they overhang into the gap and
    /// over the neighbouring window. Hosting them above every other layer
    /// keeps a revealed edge visible instead of letting the neighbour occlude
    /// it. Each window parents its own `border_tree` here.
    pub border_overlay: SceneTree,
}

pub struct Scene {
    pub wlr_scene: *mut ffi::wlr_scene,
    pub interactive_tree: SceneTree,
    pub drag_icons: SceneTree,
    pub hidden_tree: SceneTree,
    pub normal_tree: SceneTree,
    pub locked_tree: SceneTree,
    pub layers: SceneLayers,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            wlr_scene: std::ptr::null_mut(),
            interactive_tree: SceneTree::none(),
            drag_icons: SceneTree::none(),
            hidden_tree: SceneTree::none(),
            normal_tree: SceneTree::none(),
            locked_tree: SceneTree::none(),
            layers: SceneLayers {
                background: SceneTree::none(),
                background_clients: SceneTree::none(),
                bottom: SceneTree::none(),
                wm: SceneTree::none(),
                top: SceneTree::none(),
                fullscreen: SceneTree::none(),
                overlay: SceneTree::none(),
                popups: SceneTree::none(),
                override_redirect: SceneTree::none(),
                border_overlay: SceneTree::none(),
            },
        }
    }

    pub unsafe fn init(
        &mut self,
        linux_dmabuf: *mut ffi::wlr_linux_dmabuf_v1,
        _color_manager: *mut ffi::wlr_color_manager_v1,
    ) -> Result<(), &'static str> {
        let wlr_scene = ffi::wlr_scene_create();
        if wlr_scene.is_null() {
            return Err("Failed to create wlr_scene");
        }
        self.wlr_scene = wlr_scene;

        if !linux_dmabuf.is_null() {
            ffi::wlr_scene_set_linux_dmabuf_v1(wlr_scene, linux_dmabuf);
        }
        // SceneFX 0.4 does not support set_color_manager_v1
        // if !color_manager.is_null() {
        //     ffi::wlr_scene_set_color_manager_v1(wlr_scene, color_manager);
        // }

        ffi::wlr_scene_set_blur_data(wlr_scene, 3, 5, 0.0, 1.0, 1.0, 1.0);

        let root = &mut (*wlr_scene).tree as *mut ffi::wlr_scene_tree;
        self.interactive_tree = SceneTree::create_in(root);
        self.drag_icons = SceneTree::create_in(root);
        self.hidden_tree = SceneTree::create_in(root);
        if self.interactive_tree.is_null() || self.drag_icons.is_null() || self.hidden_tree.is_null() {
            return Err("Failed to create root scene trees");
        }

        self.hidden_tree.set_enabled(false);

        self.normal_tree = SceneTree::create(&self.interactive_tree);
        self.locked_tree = SceneTree::create(&self.interactive_tree);
        if self.normal_tree.is_null() || self.locked_tree.is_null() {
            return Err("Failed to create normal/locked scene trees");
        }

        self.locked_tree.set_enabled(false);

        let normal_tree = &self.normal_tree;
        self.layers.background = SceneTree::create(normal_tree);
        self.layers.background_clients = SceneTree::create(&self.layers.background);
        self.layers.bottom = SceneTree::create(normal_tree);
        self.layers.wm = SceneTree::create(normal_tree);
        self.layers.top = SceneTree::create(normal_tree);
        self.layers.fullscreen = SceneTree::create(normal_tree);
        self.layers.overlay = SceneTree::create(normal_tree);
        // Window decorations (the resize ring) sit above every window but
        // BELOW popups: a cce-ui dropdown is a separate Popup-mode window in
        // layers.popups, and a menu must never be drawn under the chrome of
        // the window that opened it. Creation order is stacking order.
        self.layers.border_overlay = SceneTree::create(normal_tree);
        self.layers.popups = SceneTree::create(normal_tree);
        self.layers.override_redirect = SceneTree::create(normal_tree);

        // Desk content renders with the camera's sub-pixel offset
        // (`WindowManager::layout_camera`): windows, their borders, their
        // popups and X11 menus, and the grid (flagged where it is built).
        // Layer shells, fullscreen and the lock screen stay put.
        for tree in [&self.layers.wm, &self.layers.border_overlay, &self.layers.popups, &self.layers.override_redirect] {
            if !tree.is_null() {
                tree.set_desk_offset(true);
            }
        }

        if self.layers.border_overlay.is_null()
            || self.layers.background.is_null()
            || self.layers.background_clients.is_null()
            || self.layers.bottom.is_null()
            || self.layers.wm.is_null()
            || self.layers.top.is_null()
            || self.layers.fullscreen.is_null()
            || self.layers.overlay.is_null()
            || self.layers.popups.is_null()
            || self.layers.override_redirect.is_null()
        {
            return Err("Failed to create scene layer trees");
        }

        Ok(())
    }

    pub fn deinit(&mut self) {}

    pub unsafe fn at(&self, lx: f64, ly: f64) -> Option<AtResult> {
        self.at_impl(lx, ly, true)
    }

    unsafe fn at_impl(&self, lx: f64, ly: f64, include_grid: bool) -> Option<AtResult> {
        let mut disabled_nodes = Vec::new();
        let mut result = None;

        loop {
            let mut sx: f64 = 0.0;
            let mut sy: f64 = 0.0;
            let node = ffi::wlr_scene_node_at(
                self.interactive_tree.node(),
                lx,
                ly,
                &mut sx,
                &mut sy,
            );

            if node.is_null() {
                break;
            }

            if let Some(scene_node_data) = SceneNodeData::from_node(node) {
                if let SceneNodeDataVal::Window(window) = scene_node_data.data {
                    // The grid layer used to be skipped here to keep it
                    // input-transparent. It now advertises an input region
                    // covering exactly its desktop items, so wlr_scene_node_at
                    // already misses it over bare canvas and every other input
                    // path (clicks, hover, overview background-exit) still sees
                    // through it — while a click ON an item reaches the client
                    // that owns it. The parameter stays for the callers that
                    // must never see the grid at all.
                    if !include_grid && (*window).is_grid() {
                        let tree_node = (*window).tree.node();
                        ffi::wlr_scene_node_set_enabled(tree_node, false);
                        disabled_nodes.push(tree_node);
                        continue;
                    }
                    if (*window).rendering_requested.circular {
                        // Check if outside the circle
                        let w = (*window).box_geom.width as f64 * (*window).scale;
                        let h = (*window).box_geom.height as f64 * (*window).scale;
                        let cx = (*window).box_geom.x as f64 + w / 2.0;
                        let cy = (*window).box_geom.y as f64 + h / 2.0;
                        let r = w.min(h) / 2.0;
                        let dx = lx - cx;
                        let dy = ly - cy;
                        if dx * dx + dy * dy > r * r {
                            // Outside the circle! Disable the window tree node temporarily and try again.
                            let tree_node = (*window).tree.node();
                            ffi::wlr_scene_node_set_enabled(tree_node, false);
                            disabled_nodes.push(tree_node);
                            continue;
                        }
                    }
                }

                let surface = ffi::river_scene_node_get_surface(node);
                // An X11 surface under xwayland_hidpi is a physical-pixel
                // buffer drawn at 1/scale, and wlr_scene_node_at maps the
                // point through the buffer's CURRENT dest size. That size
                // is reset to natural on every commit and restored by the
                // commit hook — but any other writer of dest sizes (a
                // transaction's frozen copy, a fullscreen tick) opens the
                // same gap, in which a pointer event reaches the client at
                // half its coordinates: Houdini's hover jumping up-left for
                // a frame. Derive the surface point from what is INVARIANT
                // instead — the node's layout origin and the scale the
                // buffer is meant to be shown at — so input never depends
                // on the dest state.
                let (mut sx, mut sy) = (sx, sy);
                if !surface.is_null() {
                    let ratio: Option<f64> = match scene_node_data.data {
                        SceneNodeDataVal::Window(window)
                            if !window.is_null()
                                && matches!((*window).impl_type, crate::window::WindowImpl::Xwayland(_)) =>
                        {
                            let xsurface = match (*window).impl_type {
                                crate::window::WindowImpl::Xwayland(xw) if !xw.is_null() => (*xw).xsurface as *const _,
                                _ => std::ptr::null(),
                            };
                            let s = crate::xwayland_window::x11_scale_for((*window).server, xsurface) as f64;
                            let zoom = if (*window).scale > 0.0 { (*window).scale } else { 1.0 };
                            (s != 1.0).then_some(s / zoom)
                        }
                        SceneNodeDataVal::OverrideRedirect(or) if !or.is_null() => {
                            let s = crate::xwayland_window::x11_scale_for((*or).server, (*or).xsurface) as f64;
                            (s != 1.0).then_some(s)
                        }
                        _ => None,
                    };
                    if let Some(ratio) = ratio {
                        let (mut nx, mut ny) = (0, 0);
                        if ffi::wlr_scene_node_coords(node, &mut nx, &mut ny) {
                            sx = (lx - nx as f64) * ratio;
                            sy = (ly - ny as f64) * ratio;
                        }
                    }
                }
                result = Some(AtResult {
                    node,
                    surface,
                    sx,
                    sy,
                    data: scene_node_data.data,
                });
                break;
            } else {
                break;
            }
        }

        // Re-enable all disabled nodes
        for node in disabled_nodes {
            ffi::wlr_scene_node_set_enabled(node, true);
        }

        result
    }

    pub fn layer_surface_tree(&self, layer: u32) -> *mut ffi::wlr_scene_tree {
        // layer is zwlr_layer_shell_v1_layer enum values
        match layer {
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND => self.layers.background_clients.raw(),
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM => self.layers.bottom.raw(),
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_TOP => self.layers.top.raw(),
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY => self.layers.overlay.raw(),
            _ => std::ptr::null_mut(),
        }
    }
}

pub struct AtResult {
    pub node: *mut ffi::wlr_scene_node,
    pub surface: *mut ffi::wlr_surface,
    pub sx: f64,
    pub sy: f64,
    pub data: SceneNodeDataVal,
}

pub struct SaveableSurfaces {
    pub enabled: bool,
    pub saved: bool,
    pub tree: crate::scene_handle::SceneTree,
    pub saved_tree: crate::scene_handle::SceneTree,
}

impl SaveableSurfaces {
    pub unsafe fn init(parent: *mut ffi::wlr_scene_tree) -> Result<Self, &'static str> {
        let tree = crate::scene_handle::SceneTree::create_in(parent);
        let saved_tree = crate::scene_handle::SceneTree::create_in(parent);
        if tree.is_null() || saved_tree.is_null() {
            return Err("Failed to create saveable surfaces trees");
        }
        let surfaces = SaveableSurfaces {
            enabled: true,
            saved: false,
            tree,
            saved_tree,
        };
        surfaces.sync_enabled();
        Ok(surfaces)
    }

    pub fn sync_enabled(&self) {
        self.tree.set_enabled(self.enabled && !self.saved);
        self.saved_tree.set_enabled(self.enabled && self.saved);
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.sync_enabled();
    }

    pub unsafe fn save(&mut self) {
        if self.saved {
            return;
        }
        if self.tree.is_null() || self.saved_tree.is_null() {
            return;
        }
        ffi::river_scene_tree_save_buffers(self.tree.raw(), self.saved_tree.raw());
        self.saved = true;
        self.sync_enabled();
    }

    pub unsafe fn drop_saved(&mut self) {
        if !self.saved {
            return;
        }
        if !self.saved_tree.is_null() {
            ffi::river_scene_tree_clear_children(self.saved_tree.raw());
        }
        self.saved = false;
        self.sync_enabled();
    }
}
