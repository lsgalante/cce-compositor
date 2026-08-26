// SPDX-FileCopyrightText: © 2024 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::scene_node_data::{SceneNodeData, SceneNodeDataVal};

pub struct SceneLayers {
    pub background: *mut ffi::wlr_scene_tree,
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
        self.layers.bottom = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.wm = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.top = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.fullscreen = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.overlay = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.popups = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.override_redirect = ffi::wlr_scene_tree_create(normal_tree);
        self.layers.border_overlay = ffi::wlr_scene_tree_create(normal_tree);

        if self.layers.border_overlay.is_null()
            || self.layers.background.is_null()
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
        self.at_impl(lx, ly, false)
    }

    /// `at`, but the grid layer participates like any other surface. The only
    /// caller is the drag path: the grid client is the desktop's drop target
    /// (it draws the canvas, so it owns what "dropped here" means), and this
    /// resolves the drop point through wlroots so the surface-local
    /// coordinates account for the grid's buffer scale — hand-deriving them
    /// from box_geom would drift the moment the camera zoomed.
    pub unsafe fn at_including_grid(&self, lx: f64, ly: f64) -> Option<AtResult> {
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
                    // The grid layer is input-transparent: every input path
                    // (clicks, hover, overview background-exit) sees what is
                    // underneath it, exactly as if it were the backdrop.
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
            ffi::zwlr_layer_shell_v1_layer_ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND => self.layers.background,
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
