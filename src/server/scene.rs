// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::scene_node_data::{SceneNodeData, SceneNodeDataVal};

pub struct SceneLayers {
    pub background: *mut ffi::wlr_scene_tree,
    /// Client-provided backgrounds — wlr-layer-shell Background surfaces and
    /// the `cce-wallpaper` window — inside `background`, ABOVE each output's
    /// base rect and grid backdrop and BELOW its grid cells (`Output::draw_grid`
    /// keeps the backdrop under this tree and the cell tree above it). A client
    /// wallpaper therefore replaces the flat backdrop colour and keeps the cell
    /// lattice; before this tree the grid tree, re-raised on every redraw, buried
    /// every client background under its opaque backdrop.
    pub background_clients: *mut ffi::wlr_scene_tree,
    pub bottom: *mut ffi::wlr_scene_tree,
    pub wm: *mut ffi::wlr_scene_tree,
    pub top: *mut ffi::wlr_scene_tree,
    pub fullscreen: *mut ffi::wlr_scene_tree,
    pub overlay: *mut ffi::wlr_scene_tree,
    pub popups: *mut ffi::wlr_scene_tree,
    pub override_redirect: *mut ffi::wlr_scene_tree,
    /// Hover-revealed window borders. Borders draw outside the content box, so
    /// with the content filling its grid cell they overhang into the gap and
    /// over the neighbouring window. Hosting them above every other layer
    /// keeps a revealed edge visible instead of letting the neighbour occlude
    /// it. Each window parents its own `border_tree` here.
    pub border_overlay: *mut ffi::wlr_scene_tree,
}

pub struct Scene {
    pub wlr_scene: *mut ffi::wlr_scene,
    pub interactive_tree: *mut ffi::wlr_scene_tree,
    pub drag_icons: *mut ffi::wlr_scene_tree,
    pub hidden_tree: *mut ffi::wlr_scene_tree,
    pub normal_tree: *mut ffi::wlr_scene_tree,
    pub locked_tree: *mut ffi::wlr_scene_tree,
    pub layers: SceneLayers,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            wlr_scene: std::ptr::null_mut(),
            interactive_tree: std::ptr::null_mut(),
            drag_icons: std::ptr::null_mut(),
            hidden_tree: std::ptr::null_mut(),
            normal_tree: std::ptr::null_mut(),
            locked_tree: std::ptr::null_mut(),
            layers: SceneLayers {
                background: std::ptr::null_mut(),
                background_clients: std::ptr::null_mut(),
                bottom: std::ptr::null_mut(),
                wm: std::ptr::null_mut(),
                top: std::ptr::null_mut(),
                fullscreen: std::ptr::null_mut(),
                overlay: std::ptr::null_mut(),
                popups: std::ptr::null_mut(),
                override_redirect: std::ptr::null_mut(),
                border_overlay: std::ptr::null_mut(),
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

        let interactive_tree = ffi::wlr_scene_tree_create(&mut (*wlr_scene).tree);
        let drag_icons = ffi::wlr_scene_tree_create(&mut (*wlr_scene).tree);
        let hidden_tree = ffi::wlr_scene_tree_create(&mut (*wlr_scene).tree);
        if interactive_tree.is_null() || drag_icons.is_null() || hidden_tree.is_null() {
            return Err("Failed to create root scene trees");
        }
        self.interactive_tree = interactive_tree;
        self.drag_icons = drag_icons;
        self.hidden_tree = hidden_tree;

        ffi::wlr_scene_node_set_enabled(hidden_tree as *mut ffi::wlr_scene_node, false);

        let normal_tree = ffi::wlr_scene_tree_create(interactive_tree);
        let locked_tree = ffi::wlr_scene_tree_create(interactive_tree);
        if normal_tree.is_null() || locked_tree.is_null() {
            return Err("Failed to create normal/locked scene trees");
        }
        self.normal_tree = normal_tree;
        self.locked_tree = locked_tree;

        ffi::wlr_scene_node_set_enabled(locked_tree as *mut ffi::wlr_scene_node, false);

        self.layers.background = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.background_clients = ffi::wlr_scene_tree_create(self.layers.background);
        self.layers.bottom = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.wm = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.top = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.fullscreen = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.overlay = ffi::wlr_scene_tree_create(normal_tree);
        // Window decorations (the resize ring) sit above every window but
        // BELOW popups: a cce-ui dropdown is a separate Popup-mode window in
        // layers.popups, and a menu must never be drawn under the chrome of
        // the window that opened it. Creation order is stacking order.
        self.layers.border_overlay = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.popups = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.override_redirect = ffi::wlr_scene_tree_create(normal_tree);

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

    pub unsafe fn deinit(&mut self) {}

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
                self.interactive_tree as *mut ffi::wlr_scene_node,
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
                        let tree_node = (*window).tree as *mut ffi::wlr_scene_node;
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
                            let tree_node = (*window).tree as *mut ffi::wlr_scene_node;
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

    pub unsafe fn layer_surface_tree(&self, layer: u32) -> *mut ffi::wlr_scene_tree {
        // layer is zwlr_layer_shell_v1_layer enum values
        match layer {
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND => self.layers.background_clients,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM => self.layers.bottom,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_TOP => self.layers.top,
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY => self.layers.overlay,
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
    pub tree: *mut ffi::wlr_scene_tree,
    pub saved_tree: *mut ffi::wlr_scene_tree,
}

impl SaveableSurfaces {
    pub unsafe fn init(parent: *mut ffi::wlr_scene_tree) -> Result<Self, &'static str> {
        let tree = ffi::wlr_scene_tree_create(parent);
        let saved_tree = ffi::wlr_scene_tree_create(parent);
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

    pub unsafe fn sync_enabled(&self) {
        ffi::wlr_scene_node_set_enabled(self.tree as *mut ffi::wlr_scene_node, self.enabled && !self.saved);
        ffi::wlr_scene_node_set_enabled(self.saved_tree as *mut ffi::wlr_scene_node, self.enabled && self.saved);
    }

    pub unsafe fn set_enabled(&mut self, enabled: bool) {
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
        ffi::river_scene_tree_save_buffers(self.tree, self.saved_tree);
        self.saved = true;
        self.sync_enabled();
    }

    pub unsafe fn drop_saved(&mut self) {
        if !self.saved {
            return;
        }
        ffi::river_scene_tree_clear_children(self.saved_tree);
        self.saved = false;
        self.sync_enabled();
    }
}
