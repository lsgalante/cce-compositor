// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlList, wl_list_insert, wl_list_remove, wl_list_remove_and_reinit, WlListener, wl_signal_add};
use crate::wm_node::WmNode;
use crate::xdg_toplevel::ConfigureState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    Init,
    Ready,
    Initialized,
    Mapped,
    Closing,
}

#[derive(Clone, Copy)]
pub enum WindowImpl {
    Toplevel(*mut crate::xdg_toplevel::XdgToplevel),
    Xwayland(*mut crate::xwayland_window::XwaylandWindow),
    Destroying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullscreenRequest {
    NoRequest,
    Fullscreen(*mut crate::output::Output),
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaximizeRequest {
    NoRequest,
    Maximize,
    Unmaximize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DimensionsHint {
    pub min_width: u32,
    pub min_height: u32,
    pub max_width: u32,
    pub max_height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Edges {
    pub fn new() -> Self {
        Self { top: false, bottom: false, left: false, right: false }
    }
    pub fn from_u32(val: u32) -> Self {
        Self {
            top: (val & 1) != 0,
            bottom: (val & 2) != 0,
            left: (val & 4) != 0,
            right: (val & 8) != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub edges: Edges,
    pub width: u32,
    /// Premultiplied-alpha RGBA, 0.0–1.0 per channel (scenefx convention).
    pub color: [f32; 4],
    /// Color while the pointer hovers the border (the grab surface).
    pub hover_color: [f32; 4],
    pub corner_radius: i32,
}

impl Border {
    pub fn none() -> Self {
        Self { edges: Edges::new(), width: 0, color: [0.0; 4], hover_color: [0.0; 4], corner_radius: 0 }
    }
}

/// One of the 8 interactive border zones. Each draws as its own visual
/// element (corners as two-rect Ls) and highlights independently on hover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderElement {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Length of a corner zone, measured from the outer corner along each band.
/// Shared by the visual segments (draw_borders) and the pointer zones
/// (cursor.rs get_border_zone) so they always agree. `configured` comes from
/// `border { corner_length= }`; 0 picks the auto formula. Never shorter than
/// the band width, so a corner is at least its diagonal square.
pub fn border_corner_len(bw: f64, configured: i32) -> f64 {
    if configured > 0 {
        (configured as f64).max(bw)
    } else {
        (2.0 * bw).max(16.0)
    }
}

// Indices into BorderRects.segments: 4 edge bars + 2 L-arm rects per corner.
const SEG_TOP: usize = 0;
const SEG_BOTTOM: usize = 1;
const SEG_LEFT: usize = 2;
const SEG_RIGHT: usize = 3;
const SEG_TL_H: usize = 4;
const SEG_TL_V: usize = 5;
const SEG_TR_H: usize = 6;
const SEG_TR_V: usize = 7;
const SEG_BL_H: usize = 8;
const SEG_BL_V: usize = 9;
const SEG_BR_H: usize = 10;
const SEG_BR_V: usize = 11;

pub struct BorderRects {
    /// Invisible full-band rects kept as scene hit-test catchers, so the
    /// pointer never falls through the visual gaps between segments.
    pub left: *mut ffi::wlr_scene_rect,
    pub right: *mut ffi::wlr_scene_rect,
    pub top: *mut ffi::wlr_scene_rect,
    pub bottom: *mut ffi::wlr_scene_rect,
    /// The visible zone segments, indexed by the SEG_* constants.
    pub segments: [*mut ffi::wlr_scene_rect; 12],
}

pub struct ShowWindowMenuRequest {
    pub x: i32,
    pub y: i32,
}

pub struct PointerResizeRequest {
    pub seat: *mut crate::seat::Seat,
    pub edges: u32,
}

pub struct WmScheduledState {
    pub dimensions_hint: DimensionsHint,
    pub decoration_hint: ffi::zcce_window_v1_decoration_hint,
    pub show_window_menu_requested: Option<ShowWindowMenuRequest>,
    pub fullscreen_requested: FullscreenRequest,
    pub maximize_requested: MaximizeRequest,
    pub minimize_requested: bool,
    pub dirty_app_id: bool,
    pub dirty_title: bool,
    pub pointer_move_requested: *mut crate::seat::Seat,
    pub pointer_resize_requested: Option<PointerResizeRequest>,
}

pub struct WmSentState {
    pub dimensions_hint: DimensionsHint,
    pub decoration_hint: ffi::zcce_window_v1_decoration_hint,
    pub parent: Option<crate::slotmap::Key>,
}

pub struct WmRequestedState {
    pub dimensions: Option<Dimensions>,
    pub bounds: Dimensions,
    pub ssd: bool,
    pub tiled: u32,
    pub capabilities: u32,
    pub resizing: bool,
    pub maximized: bool,
    pub fullscreen: *mut crate::output::Output,
    pub inform_fullscreen: bool,
    pub close: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configure {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bounds: Dimensions,
    pub activated: bool,
    pub ssd: bool,
    pub tiled: u32,
    pub capabilities: u32,
    pub maximized: bool,
    pub inform_fullscreen: bool,
    pub resizing: bool,
}

impl Configure {
    pub fn new() -> Self {
        Self {
            width: None,
            height: None,
            bounds: Dimensions { width: 0, height: 0 },
            activated: false,
            ssd: false,
            tiled: 0,
            capabilities: 0,
            maximized: false,
            inform_fullscreen: false,
            resizing: false,
        }
    }
}

pub struct WindowRenderingScheduled {
    pub width: u32,
    pub height: u32,
    pub resend_dimensions: bool,
}

pub struct WindowRenderingSent {
    pub width: u32,
    pub height: u32,
    pub presentation_hint: ffi::zcce_output_v1_presentation_mode,
}

pub struct WindowRenderingRequested {
    pub x: i32,
    pub y: i32,
    pub hidden: bool,
    pub border: Border,
    pub clip: ffi::wlr_box,
    pub content_clip: ffi::wlr_box,
    pub opacity: f32,
    pub circular: bool,
    pub blur: bool,
}

pub struct Window {
    pub ref_key: crate::slotmap::Key,
    pub server: *mut Server,
    pub object: *mut ffi::wl_resource, // zcce_window_v1
    pub node: WmNode,
    pub state: WindowState,
    pub impl_type: WindowImpl,

    pub tree: *mut ffi::wlr_scene_tree,
    pub fullscreen_background: *mut ffi::wlr_scene_rect,
    pub window_background: *mut ffi::wlr_scene_rect,
    pub decorations_below: ffi::wl_list,
    pub decorations_below_tree: *mut ffi::wlr_scene_tree,
    pub surfaces: crate::scene::SaveableSurfaces,
    pub border: BorderRects,
    /// The border zone the pointer is over (set by cursor.rs); that segment
    /// draws in `hover_color` while set.
    pub hovered_border_element: Option<BorderElement>,
    pub decorations_above: ffi::wl_list,
    pub decorations_above_tree: *mut ffi::wlr_scene_tree,
    pub popup_tree: *mut ffi::wlr_scene_tree,
    pub capture_scene: *mut ffi::wlr_scene,
    pub capture_source: *mut ffi::wlr_ext_image_capture_source_v1,
    pub tiling_mode: crate::tiling::TilingMode,
    pub mode_locked: bool,
    pub is_new: bool,
    pub restored: bool,
    pub restored_focused: bool,
    pub closed: bool,
    pub has_parent: bool,
    pub minimized: bool,
    pub anim_x: Option<f64>,
    pub anim_y: Option<f64>,
    pub anim_w: Option<f64>,
    pub anim_h: Option<f64>,
    pub anim_opacity: Option<f64>,
    pub circular: bool,
    pub blur: bool,
    pub scale: f64,
    pub last_applied_scale: f64,
    pub virtual_x: f64,
    pub virtual_y: f64,
    pub resize_start_vx: f64,
    pub resize_start_vy: f64,
    pub resize_start_w: u32,
    pub resize_start_h: u32,
    pub resize_edges: Option<Edges>,
    pub commit: ffi::wl_listener,
    pub was_fullscreen: bool,
    pub saved_width: i32,
    pub saved_height: i32,
    pub saved_virtual_x: f64,
    pub saved_virtual_y: f64,
    pub was_maximized: bool,
    pub saved_maximized_width: i32,
    pub saved_maximized_height: i32,
    pub saved_maximized_virtual_x: f64,
    pub saved_maximized_virtual_y: f64,

    pub wm_scheduled: WmScheduledState,
    pub wm_sent: WmSentState,
    pub wm_requested: WmRequestedState,
    pub configure_scheduled: Configure,
    pub configure_sent: Configure,
    pub rendering_scheduled: WindowRenderingScheduled,
    pub rendering_sent: WindowRenderingSent,
    pub rendering_requested: WindowRenderingRequested,
    pub box_geom: ffi::wlr_box,
    pub margin_x: i32,
    pub margin_y: i32,
    pub last_decor_w: i32,
    pub last_decor_h: i32,
    pub foreign_toplevel_handle: *mut ffi::wlr_ext_foreign_toplevel_handle_v1,
    pub wlr_toplevel_handle: *mut ffi::wlr_foreign_toplevel_handle_v1,
    pub csd_buffer_size_bug: bool,
    pub status_edge: StatusEdge,
}

pub use crate::policy::arrange::StatusEdge;

impl Window {
    pub unsafe fn is_wine(&self) -> bool {
        false
    }

    pub unsafe fn is_fullscreen(&self) -> bool {
        self.tiling_mode == crate::tiling::TilingMode::Fullscreen
            || !self.wm_requested.fullscreen.is_null()
    }

    pub unsafe fn role(&self) -> crate::policy::api::WindowRole {
        crate::policy::api::WindowRole::from_app_id(self.get_app_id_string().as_deref())
    }

    pub unsafe fn is_status_bar(&self) -> bool {
        self.role() == crate::policy::api::WindowRole::StatusBar
    }

    pub unsafe fn is_wallpaper(&self) -> bool {
        self.role() == crate::policy::api::WindowRole::Background
    }

    pub unsafe fn is_linked(&self) -> bool {
        let prev = self.node.link.prev;
        let next = self.node.link.next;
        if prev.is_null() || next.is_null() {
            return false;
        }
        let self_ptr = &self.node.link as *const ffi::wl_list as *mut ffi::wl_list;
        prev != self_ptr
    }


    pub unsafe fn create(impl_type: WindowImpl, server: *mut Server) -> Result<*mut Self, &'static str> {
        let hidden_tree = (*server).scene.hidden_tree;
        let tree = ffi::wlr_scene_tree_create(hidden_tree);
        if tree.is_null() {
            return Err("Failed to create tree");
        }

        let popup_tree = ffi::wlr_scene_tree_create(hidden_tree);
        if popup_tree.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create popup_tree");
        }

        let capture_scene = ffi::wlr_scene_create();
        if capture_scene.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create capture_scene");
        }
        // SceneFX 0.4 does not support restack_xwayland_surfaces
        // (*capture_scene).restack_xwayland_surfaces = false;

        let black_color = [0.0f32, 0.0f32, 0.0f32, 1.0f32];
        let fullscreen_background = ffi::wlr_scene_rect_create(tree, 0, 0, black_color.as_ptr());
        if fullscreen_background.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(&mut (*capture_scene).tree as *mut ffi::wlr_scene_tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create fullscreen rect");
        }

        let decorations_below_tree = ffi::wlr_scene_tree_create(tree);

        let clear_color = [0.0f32, 0.0f32, 0.0f32, 0.0f32];
        let window_background = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());
        if window_background.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
            ffi::wlr_scene_node_destroy(&mut (*capture_scene).tree as *mut ffi::wlr_scene_tree as *mut ffi::wlr_scene_node);
            return Err("Failed to create window background rect");
        }

        let surfaces = match crate::scene::SaveableSurfaces::init(tree) {
            Ok(s) => s,
            Err(e) => {
                ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
                ffi::wlr_scene_node_destroy(popup_tree as *mut ffi::wlr_scene_node);
                ffi::wlr_scene_node_destroy(&mut (*capture_scene).tree as *mut ffi::wlr_scene_tree as *mut ffi::wlr_scene_node);
                return Err(e);
            }
        };

        let border_left = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());
        let border_right = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());
        let border_top = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());
        let border_bottom = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());
        let mut border_segments = [std::ptr::null_mut(); 12];
        for seg in border_segments.iter_mut() {
            *seg = ffi::wlr_scene_rect_create(tree, 0, 0, clear_color.as_ptr());
        }

        let decorations_above_tree = ffi::wlr_scene_tree_create(tree);

        let mut window = Box::new(Window {
            ref_key: crate::slotmap::Key { generation: 0, index: 0 },
            server,
            object: std::ptr::null_mut(),
            node: std::mem::zeroed(),
            state: WindowState::Init,
            impl_type,
            tree,
            fullscreen_background,
            window_background,
            decorations_below: std::mem::zeroed(),
            decorations_below_tree,
            surfaces,
            border: BorderRects {
                left: border_left,
                right: border_right,
                top: border_top,
                bottom: border_bottom,
                segments: border_segments,
            },
            hovered_border_element: None,
            decorations_above: std::mem::zeroed(),
            decorations_above_tree,
            popup_tree,
            capture_scene,
            capture_source: std::ptr::null_mut(),
            tiling_mode: crate::tiling::TilingMode::Floating,
            mode_locked: false,
            is_new: true,
            restored: false,
            restored_focused: false,
            closed: false,
            has_parent: false,
            minimized: false,
            anim_x: None,
            anim_y: None,
            anim_w: None,
            anim_h: None,
            anim_opacity: None,
            circular: false,
            blur: false,
            scale: 1.0,
            last_applied_scale: 1.0,
            virtual_x: unsafe { (*server).wm.desk_pan_x + 100.0 },
            virtual_y: unsafe { (*server).wm.desk_pan_y + 100.0 },
            resize_start_vx: 0.0,
            resize_start_vy: 0.0,
            resize_start_w: 0,
            resize_start_h: 0,
            resize_edges: None,
            commit: std::mem::zeroed(),
            was_fullscreen: false,
            saved_width: 0,
            saved_height: 0,
            saved_virtual_x: 0.0,
            saved_virtual_y: 0.0,
            was_maximized: false,
            saved_maximized_width: 0,
            saved_maximized_height: 0,
            saved_maximized_virtual_x: 0.0,
            saved_maximized_virtual_y: 0.0,
            wm_scheduled: WmScheduledState {
                dimensions_hint: DimensionsHint { min_width: 0, min_height: 0, max_width: 0, max_height: 0 },
                decoration_hint: ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_ONLY_SUPPORTS_CSD,
                show_window_menu_requested: None,
                fullscreen_requested: FullscreenRequest::NoRequest,
                maximize_requested: MaximizeRequest::NoRequest,
                minimize_requested: false,
                dirty_app_id: false,
                dirty_title: false,
                pointer_move_requested: std::ptr::null_mut(),
                pointer_resize_requested: None,
            },
            wm_sent: WmSentState {
                dimensions_hint: DimensionsHint { min_width: 0, min_height: 0, max_width: 0, max_height: 0 },
                decoration_hint: ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_ONLY_SUPPORTS_CSD,
                parent: None,
            },
            wm_requested: WmRequestedState {
                dimensions: None,
                bounds: Dimensions { width: 0, height: 0 },
                ssd: false,
                tiled: 0,
                capabilities: 1 | 2 | 4 | 8,
                resizing: false,
                maximized: false,
                fullscreen: std::ptr::null_mut(),
                inform_fullscreen: false,
                close: false,
            },
            configure_scheduled: Configure::new(),
            configure_sent: Configure::new(),
            rendering_scheduled: WindowRenderingScheduled {
                width: 0,
                height: 0,
                resend_dimensions: false,
            },
            rendering_sent: WindowRenderingSent {
                width: 0,
                height: 0,
                presentation_hint: ffi::zcce_output_v1_presentation_mode_ZCCE_OUTPUT_V1_PRESENTATION_MODE_VSYNC,
            },
            rendering_requested: WindowRenderingRequested {
                x: 0,
                y: 0,
                hidden: false,
                border: Border::none(),
                clip: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
                content_clip: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
                opacity: 1.0f32,
                circular: false,
                blur: false,
            },
            box_geom: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
            margin_x: 0,
            margin_y: 0,
            last_decor_w: 0,
            last_decor_h: 0,
            foreign_toplevel_handle: std::ptr::null_mut(),
            wlr_toplevel_handle: std::ptr::null_mut(),
            csd_buffer_size_bug: false,
            status_edge: StatusEdge::Unspecified,
        });

        ffi::wl_list_init(&mut window.decorations_below);
        ffi::wl_list_init(&mut window.decorations_above);

        let raw = Box::into_raw(window);
        let key = (*(*raw).server).wm.windows.put(raw);
        (*raw).ref_key = key;
        (*raw).node.init(crate::wm_node::WmNodeTag::Window);

        ffi::wlr_scene_node_set_enabled(tree as *mut ffi::wlr_scene_node, false);
        ffi::wlr_scene_node_set_enabled(popup_tree as *mut ffi::wlr_scene_node, false);
        ffi::wlr_scene_node_set_enabled(fullscreen_background as *mut ffi::wlr_scene_node, false);

        crate::scene_node_data::SceneNodeData::attach(
            tree as *mut ffi::wlr_scene_node,
            crate::scene_node_data::SceneNodeDataVal::Window(raw),
        );
        crate::scene_node_data::SceneNodeData::attach(
            popup_tree as *mut ffi::wlr_scene_node,
            crate::scene_node_data::SceneNodeDataVal::Window(raw),
        );

        Ok(raw)
    }

    pub unsafe fn set_impl(&mut self, impl_type: WindowImpl) {
        self.impl_type = impl_type;
    }

    pub unsafe fn impl_destroying(&mut self) {
        self.impl_type = WindowImpl::Destroying;
    }

    pub unsafe fn get_title(&self) -> *const libc::c_char {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    std::ptr::null()
                } else {
                    ffi::river_wlr_xdg_toplevel_get_title((*toplevel).wlr_toplevel)
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    std::ptr::null()
                } else {
                    (*(*xwindow).xsurface).title
                }
            }
            WindowImpl::Destroying => std::ptr::null(),
        }
    }

    pub unsafe fn get_app_id(&self) -> *const libc::c_char {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    std::ptr::null()
                } else {
                    ffi::river_wlr_xdg_toplevel_get_app_id((*toplevel).wlr_toplevel)
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    std::ptr::null()
                } else {
                    (*(*xwindow).xsurface).class
                }
            }
            WindowImpl::Destroying => std::ptr::null(),
        }
    }

    pub unsafe fn get_app_id_string(&self) -> Option<String> {
        let ptr = self.get_app_id();
        if ptr.is_null() {
            None
        } else {
            Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned())
        }
    }

    pub unsafe fn get_title_string(&self) -> Option<String> {
        let ptr = self.get_title();
        if ptr.is_null() {
            None
        } else {
            Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned())
        }
    }

    pub unsafe fn get_parent(&self) -> *mut Window {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    std::ptr::null_mut()
                } else {
                    let wlr_parent = ffi::river_wlr_xdg_toplevel_get_parent((*toplevel).wlr_toplevel);
                    if wlr_parent.is_null() {
                        std::ptr::null_mut()
                    } else {
                        let base = ffi::river_wlr_xdg_toplevel_get_base(wlr_parent);
                        let parent_xdg = ffi::river_wlr_xdg_surface_get_data(base) as *mut crate::xdg_toplevel::XdgToplevel;
                        if parent_xdg.is_null() {
                            std::ptr::null_mut()
                        } else {
                            (*parent_xdg).window
                        }
                    }
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    std::ptr::null_mut()
                } else {
                    let parent_xsurface = (*(*xwindow).xsurface).parent;
                    if parent_xsurface.is_null() {
                        std::ptr::null_mut()
                    } else {
                        let parent_data = (*parent_xsurface).data;
                        if parent_data.is_null() {
                            std::ptr::null_mut()
                        } else {
                            let parent_xwindow = parent_data as *mut crate::xwayland_window::XwaylandWindow;
                            (*parent_xwindow).window
                        }
                    }
                }
            }
            WindowImpl::Destroying => std::ptr::null_mut(),
        }
    }

    pub unsafe fn unreliable_pid(&self) -> i32 {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    0
                } else {
                    let base = ffi::river_wlr_xdg_toplevel_get_base((*toplevel).wlr_toplevel);
                    let surface = ffi::river_wlr_xdg_surface_get_surface(base);
                    if surface.is_null() {
                        0
                    } else {
                        let res = ffi::river_wlr_surface_get_resource(surface);
                        if res.is_null() {
                            0
                        } else {
                            let client = ffi::wl_resource_get_client(res);
                            if client.is_null() {
                                0
                            } else {
                                let mut pid = 0;
                                let mut uid = 0;
                                let mut gid = 0;
                                ffi::wl_client_get_credentials(client, &mut pid, &mut uid, &mut gid);
                                pid
                            }
                        }
                    }
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    0
                } else {
                    (*(*xwindow).xsurface).pid
                }
            }
            WindowImpl::Destroying => 0,
        }
    }

    pub unsafe fn try_restore(&mut self) {
        if self.restored {
            return;
        }
        let app_id_str = self.get_app_id_string().unwrap_or_default();
        if app_id_str.is_empty() || app_id_str.starts_with("cce-status") || app_id_str == "cce-wallpaper" {
            return;
        }
        let title_str = self.get_title_string().unwrap_or_default();
        let mut saved_opt = (*self.server).wm.match_and_remove_restore_state(&app_id_str, &title_str);
        if saved_opt.is_none() {
            saved_opt = (*self.server).wm.match_last_window_state(&app_id_str, &title_str);
        }
        if let Some(saved) = saved_opt {
            log::info!("Restoring saved state for window: app_id={}, title={}. Position: ({}, {}), Size: {}x{}", app_id_str, title_str, saved.virtual_x, saved.virtual_y, saved.width, saved.height);
            self.tiling_mode = saved.tiling_mode;
            self.minimized = saved.minimized;
            self.virtual_x = saved.virtual_x;
            self.virtual_y = saved.virtual_y;
            self.scale = saved.scale;
            self.box_geom.width = saved.width as i32;
            self.box_geom.height = saved.height as i32;
            
            self.wm_requested.dimensions = Some(crate::window::Dimensions {
                width: saved.width,
                height: saved.height,
            });
            self.wm_requested.bounds = crate::window::Dimensions {
                width: saved.width,
                height: saved.height,
            };
            
            self.rendering_scheduled.width = saved.width;
            self.rendering_scheduled.height = saved.height;
            self.rendering_sent.width = saved.width;
            self.rendering_sent.height = saved.height;

            match self.impl_type {
                WindowImpl::Toplevel(toplevel) => {
                    if !toplevel.is_null() {
                        (*toplevel).geometry.width = saved.width as i32;
                        (*toplevel).geometry.height = saved.height as i32;
                    }
                }
                WindowImpl::Xwayland(xwindow) => {
                    if !xwindow.is_null() && !(*xwindow).xsurface.is_null() {
                        (*(*xwindow).xsurface).width = saved.width as u16;
                        (*(*xwindow).xsurface).height = saved.height as u16;
                    }
                }
                _ => {}
            }

            self.restored = true;
            self.restored_focused = saved.focused;
        }
    }

    pub unsafe fn map(&mut self) -> Result<(), &'static str> {
        log::debug!("window '{:?}' mapped", self.get_title());
        assert!(!matches!(self.impl_type, WindowImpl::Destroying));
        assert_eq!(self.state, WindowState::Initialized);
        self.state = WindowState::Mapped;

        self.try_restore();

        let surface = self.root_surface();
        if !surface.is_null() {
            let commit_listener = &mut self.commit as *mut ffi::wl_listener as *mut WlListener;
            (*commit_listener).notify = Some(handle_window_commit);
            wl_signal_add(ffi::river_wlr_surface_get_commit_signal(surface), &mut self.commit);
        }

        let app_id_ptr = self.get_app_id();
        let (is_status_bar, is_wallpaper) = if !app_id_ptr.is_null() {
            let app_id = std::ffi::CStr::from_ptr(app_id_ptr).to_string_lossy();
            (app_id.starts_with("cce-status"), app_id.as_ref() == "cce-wallpaper")
        } else {
            (false, false)
        };

        if is_status_bar || is_wallpaper {
            self.tiling_mode = crate::tiling::TilingMode::Status;
            if is_status_bar && self.status_edge == StatusEdge::Unspecified {
                let app_id = std::ffi::CStr::from_ptr(app_id_ptr).to_string_lossy();
                let name = if let Some(stripped) = app_id.strip_prefix("cce-status-interface-left-").or_else(|| app_id.strip_prefix("cce-status-left-")) {
                    stripped
                } else if let Some(stripped) = app_id.strip_prefix("cce-status-interface-right-").or_else(|| app_id.strip_prefix("cce-status-right-")) {
                    stripped
                } else {
                    &app_id
                };
                let mut loaded_edge = None;
                if name == "light_source" {
                    let mut light_pos = 2.356194490192345_f32; // Default 135 deg in rad
                    if let Ok(content) = std::fs::read_to_string(cce_ui::config::get_config_path()) {
                        let val = cce_ui::config::parse_kdl_to_json(&content);
                        if let Some(wm_obj) = val.get("window_manager") {
                            if let Some(pos_val) = wm_obj.get("light_source_position") {
                                if let Some(f) = pos_val.as_f64() {
                                    light_pos = f as f32;
                                } else if let Some(i) = pos_val.as_i64() {
                                    let deg = i as f32;
                                    if deg > 2.0 * std::f32::consts::PI {
                                        light_pos = deg.to_radians();
                                    } else {
                                        light_pos = deg;
                                    }
                                }
                            }
                        }
                    }
                    
                    let two_pi = 2.0 * std::f32::consts::PI;
                    let mut angle = light_pos % two_pi;
                    if angle < 0.0 {
                        angle += two_pi;
                    }
                    
                    let pi = std::f32::consts::PI;
                    let edge = if angle < pi / 8.0 || angle >= 15.0 * pi / 8.0 {
                        StatusEdge::Right
                    } else if angle < 3.0 * pi / 8.0 {
                        StatusEdge::TopRight
                    } else if angle < 5.0 * pi / 8.0 {
                        StatusEdge::TopCenter
                    } else if angle < 7.0 * pi / 8.0 {
                        StatusEdge::TopLeft
                    } else if angle < 9.0 * pi / 8.0 {
                        StatusEdge::Left
                    } else if angle < 11.0 * pi / 8.0 {
                        StatusEdge::BottomLeft
                    } else if angle < 13.0 * pi / 8.0 {
                        StatusEdge::BottomCenter
                    } else {
                        StatusEdge::BottomRight
                    };
                    loaded_edge = Some(edge);
                } else if let Ok(content) = std::fs::read_to_string(cce_ui::config::get_config_path()) {
                    let val = cce_ui::config::parse_kdl_to_json(&content);
                    if let Some(layout_obj) = val.get("layout") {
                        if let Some(status_bar_obj) = layout_obj.get("status_bar") {
                            if let Some(edge_val) = status_bar_obj.get(name) {
                                if let Some(edge_str) = edge_val.as_str() {
                                    loaded_edge = match edge_str.to_lowercase().as_str() {
                                        "left" => Some(StatusEdge::Left),
                                        "right" => Some(StatusEdge::Right),
                                        "top-left" => Some(StatusEdge::TopLeft),
                                        "top-center" => Some(StatusEdge::TopCenter),
                                        "top-right" => Some(StatusEdge::TopRight),
                                        "bottom-left" => Some(StatusEdge::BottomLeft),
                                        "bottom-center" => Some(StatusEdge::BottomCenter),
                                        "bottom-right" => Some(StatusEdge::BottomRight),
                                        _ => None,
                                    };
                                }
                            }
                        }
                    }
                }
                self.status_edge = if let Some(edge) = loaded_edge {
                    edge
                } else {
                    if app_id.contains("viewport") {
                        StatusEdge::TopLeft
                    } else if app_id.contains("window") {
                        StatusEdge::TopCenter
                    } else {
                        StatusEdge::TopRight
                    }
                };
            }
        } else {
            let mut should_focus = true;
            if self.restored {
                if self.restored_focused {
                    (*self.server).wm.restored_focused_window_mapped = true;
                    log::info!("[FocusRestore] Restored focused window mapped: {:?}", self.get_title());
                } else {
                    if (*self.server).wm.has_restored_focused_window && (*self.server).wm.restored_focused_window_mapped {
                        log::info!("[FocusRestore] Blocking focus to non-focused restored window {:?} because restored focused window is already mapped", self.get_title());
                        should_focus = false;
                    }
                }
            }

            if should_focus {
                let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
                let mut curr = (*seats).next;
                while curr != seats {
                    let next = (*curr).next;
                    let seat = crate::container_of!(curr, crate::seat::Seat, link);
                    (*seat).focus(crate::seat::Focus::Window(self as *mut Window));
                    curr = next;
                }
            }
        }

        (*self.server).wm.dirty_windowing();
        Ok(())
    }

    pub unsafe fn set_closing(&mut self) {
        if self.state != WindowState::Closing {
            self.state = WindowState::Closing;
            if self.is_linked() {
                wl_list_remove_and_reinit(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
            }
        }
    }

    pub unsafe fn unmap(&mut self) {
        log::debug!("window '{:?}' unmapped", self.get_title());
        if self.state != WindowState::Mapped {
            return;
        }
        wl_listener_remove_safe(&mut self.commit);
        self.surfaces.save();
        assert!(!matches!(self.impl_type, WindowImpl::Destroying));
        self.set_closing();
        (*self.server).wm.dirty_windowing();

        if !self.foreign_toplevel_handle.is_null() {
            ffi::wlr_ext_foreign_toplevel_handle_v1_destroy(self.foreign_toplevel_handle);
            self.foreign_toplevel_handle = std::ptr::null_mut();
        }
        if !self.wlr_toplevel_handle.is_null() {
            ffi::wlr_foreign_toplevel_handle_v1_destroy(self.wlr_toplevel_handle);
            self.wlr_toplevel_handle = std::ptr::null_mut();
        }


    }

    pub unsafe fn close(&mut self) {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if !toplevel.is_null() {
                    ffi::wlr_xdg_toplevel_send_close((*toplevel).wlr_toplevel);
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if !xwindow.is_null() {
                    ffi::wlr_xwayland_surface_close((*xwindow).xsurface);
                }
            }
            WindowImpl::Destroying => {}
        }
    }

    pub unsafe fn destroy(window: *mut Window) {
        assert!(matches!((*window).impl_type, WindowImpl::Destroying));
        match (*window).state {
            WindowState::Init => {}
            WindowState::Closing => {
                (*(*window).server).wm.dirty_windowing();
                return;
            }
            _ => unreachable!(),
        }
        assert!((*window).object.is_null());

        let seats = &mut (*(*window).server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            if let crate::seat::Focus::Window(w) = (*seat).focused {
                if w == window {
                    (*seat).focus(crate::seat::Focus::None);
                    (*(*window).server).wm.focus_next_visible_window(seat);
                }
            }
            if let Some(ref op) = (*seat).op {
                if op.window_ptr == window {
                    (*seat).op = None;
                }
            }
            curr = next;
        }



        // Destroy decorations
        for decorations in [&mut (*window).decorations_above as *mut ffi::wl_list, &mut (*window).decorations_below as *mut ffi::wl_list] {
            let list_head = decorations as *mut WlList;
            let mut curr = (*list_head).next;
            while curr != list_head {
                let next = (*curr).next;
                let dec = crate::container_of!(curr, Decoration, link);
                (*dec).destroy();
                curr = next;
            }
        }

        wl_listener_remove_safe(&mut (*window).commit);
        ffi::wlr_scene_node_destroy((*window).tree as *mut ffi::wlr_scene_node);
        ffi::wlr_scene_node_destroy((*window).popup_tree as *mut ffi::wlr_scene_node);
        ffi::wlr_scene_node_destroy(&mut (*(*window).capture_scene).tree as *mut ffi::wlr_scene_tree as *mut ffi::wlr_scene_node);

        (*window).node.deinit();

        (*(*window).server).wm.remove_from_history(window);
        (*(*window).server).wm.windows.remove((*window).ref_key);
        (*(*window).server).wm.check_clean_exit_progress();

        let _ = Box::from_raw(window);
    }

    pub unsafe fn set_dimensions_hint(&mut self, hint: DimensionsHint) {
        self.wm_scheduled.dimensions_hint = hint;
        if self.wm_sent.dimensions_hint != hint {
            if matches!(self.tiling_mode, crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Popup | crate::tiling::TilingMode::Status) {
                (*self.server).wm.dirty_windowing();
            }
            self.wm_sent.dimensions_hint = hint;
        }
    }

    pub unsafe fn set_dimensions(&mut self, width: u32, height: u32) {
        self.rendering_scheduled.width = width;
        self.rendering_scheduled.height = height;

        if self.rendering_scheduled.resend_dimensions ||
           self.rendering_scheduled.width != self.rendering_sent.width ||
           self.rendering_scheduled.height != self.rendering_sent.height {
            (*self.server).wm.dirty_rendering();
        }
    }

    pub unsafe fn set_decoration_hint(&mut self, hint: ffi::zcce_window_v1_decoration_hint) {
        self.wm_scheduled.decoration_hint = hint;
        if hint != self.wm_sent.decoration_hint {
            (*self.server).wm.dirty_windowing();
            self.wm_sent.decoration_hint = hint;
        }
    }

    pub unsafe fn root_surface(&self) -> *mut ffi::wlr_surface {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    std::ptr::null_mut()
                } else {
                    let base = ffi::river_wlr_xdg_toplevel_get_base((*toplevel).wlr_toplevel);
                    ffi::river_wlr_xdg_surface_get_surface(base)
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() || (*xwindow).xsurface.is_null() {
                    std::ptr::null_mut()
                } else {
                    (*(*xwindow).xsurface).surface
                }
            }
            _ => std::ptr::null_mut(),
        }
    }

    pub unsafe fn get_decorations_size(&self) -> (i32, i32) {
        if self.wm_requested.ssd {
            return (0, 0);
        }
        self.measure_decorations()
    }

    /// Raw client-side decoration size (surface minus geometry), regardless
    /// of the current SSD setting. Callers that honor SSD gate on it
    /// themselves.
    pub unsafe fn measure_decorations(&self) -> (i32, i32) {
        let surface = self.root_surface();
        if surface.is_null() {
            return (0, 0);
        }
        let surf_w = ffi::river_wlr_surface_get_width(surface);
        let surf_h = ffi::river_wlr_surface_get_height(surface);
        
        let (geom_w, geom_h) = match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    (surf_w, surf_h)
                } else {
                    ((*toplevel).geometry.width, (*toplevel).geometry.height)
                }
            }
            _ => (surf_w, surf_h),
        };
        
        let dec_w = (surf_w - geom_w).max(0);
        let dec_h = (surf_h - geom_h).max(0);
        (dec_w, dec_h)
    }

    pub unsafe fn send_frame_done(&self) {
        assert_eq!(self.state, WindowState::Mapped);
        if !matches!(self.impl_type, WindowImpl::Destroying) {
            let mut now = std::mem::zeroed();
            clock_gettime(libc::CLOCK_MONOTONIC, &mut now);
            let now_ffi = ffi::timespec {
                tv_sec: now.tv_sec as _,
                tv_nsec: now.tv_nsec as _,
            };
            ffi::wlr_surface_send_frame_done(self.root_surface(), &now_ffi);
        }
    }

    pub unsafe fn manage_start(&mut self) {
        match self.state {
            WindowState::Init => {}
            WindowState::Closing => {
                self.state = WindowState::Init;
                self.wm_sent = WmSentState {
                    dimensions_hint: DimensionsHint { min_width: 0, min_height: 0, max_width: 0, max_height: 0 },
                    decoration_hint: ffi::zcce_window_v1_decoration_hint_ZCCE_WINDOW_V1_DECORATION_HINT_ONLY_SUPPORTS_CSD,
                    parent: None,
                };
                self.wm_requested = WmRequestedState {
                    dimensions: None,
                    bounds: Dimensions { width: 0, height: 0 },
                    ssd: false,
                    tiled: 0,
                    capabilities: 1 | 2 | 4 | 8,
                    resizing: false,
                    maximized: false,
                    fullscreen: std::ptr::null_mut(),
                    inform_fullscreen: false,
                    close: false,
                };
                self.rendering_sent = WindowRenderingSent {
                    width: 0,
                    height: 0,
                    presentation_hint: ffi::zcce_output_v1_presentation_mode_ZCCE_OUTPUT_V1_PRESENTATION_MODE_VSYNC,
                };
                self.rendering_requested = WindowRenderingRequested {
                    x: 0,
                    y: 0,
                    hidden: false,
                    border: Border::none(),
                    clip: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
                    content_clip: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
                    opacity: 1.0f32,
                    circular: false,
                    blur: false,
                };

                if self.is_linked() {
                    wl_list_remove_and_reinit(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
                }

                self.make_inert();
            }
            WindowState::Ready | WindowState::Initialized | WindowState::Mapped => {
                let wm_v1 = (*self.server).wm.object;
                if wm_v1.is_null() {
                    let is_linked = self.is_linked();
                    if !is_linked {
                        if !self.node.link.prev.is_null() && !self.node.link.next.is_null() {
                            wl_list_remove_and_reinit(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
                        }
                        let rendering_list = &mut (*self.server).wm.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
                        wl_list_insert((*rendering_list).prev, &mut self.node.link as *mut ffi::wl_list as *mut WlList);

                        if self.foreign_toplevel_handle.is_null() {
                            let list = (*self.server).foreign_toplevel_list;
                            let title = self.get_title();
                            let app_id = self.get_app_id();
                            let state = ffi::wlr_ext_foreign_toplevel_handle_v1_state {
                                title,
                                app_id,
                            };
                            let handle = ffi::wlr_ext_foreign_toplevel_handle_v1_create(list, &state);
                            if !handle.is_null() {
                                self.foreign_toplevel_handle = handle;
                                (*handle).data = self as *mut Window as *mut _;
                            }
                        }

                        if self.wlr_toplevel_handle.is_null() {
                            let manager = (*self.server).wlr_foreign_toplevel_manager;
                            let handle = ffi::wlr_foreign_toplevel_handle_v1_create(manager);
                            if !handle.is_null() {
                                self.wlr_toplevel_handle = handle;
                                let title = self.get_title();
                                if !title.is_null() {
                                    ffi::wlr_foreign_toplevel_handle_v1_set_title(handle, title);
                                }
                                let app_id = self.get_app_id();
                                if !app_id.is_null() {
                                    ffi::wlr_foreign_toplevel_handle_v1_set_app_id(handle, app_id);
                                }
                            }
                        }
                        self.rendering_scheduled.resend_dimensions = true;
                    }
                    return;
                }
                let new_resource = self.object.is_null();
                let window_v1 = if new_resource {
                    let client = ffi::wl_resource_get_client(wm_v1);
                    let res = ffi::wl_resource_create(client, &ffi::zcce_window_v1_interface, ffi::wl_resource_get_version(wm_v1), 0);
                    if res.is_null() {
                        log::error!("out of memory");
                        return;
                    }
                    self.object = res;
                    self.rendering_scheduled.resend_dimensions = true;
                    ffi::wl_resource_set_implementation(
                        res,
                        &WINDOW_INTERFACE as *const _ as *const _,
                        self as *mut Window as *mut _,
                        Some(handle_destroy_resource),
                    );
                    
                    // Send window to manager
                    ffi::wl_resource_post_event(wm_v1, ffi::ZCCE_WINDOW_MANAGER_V1_WINDOW, res); // zcce_window_manager_v1.window
                    res
                } else {
                    self.object
                };

                let is_linked = self.is_linked();
                if !is_linked {
                    if !self.node.link.prev.is_null() && !self.node.link.next.is_null() {
                        wl_list_remove_and_reinit(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
                    }
                    let rendering_list = &mut (*self.server).wm.rendering_requested.list as *mut ffi::wl_list as *mut WlList;
                    wl_list_insert((*rendering_list).prev, &mut self.node.link as *mut ffi::wl_list as *mut WlList);

                    if self.foreign_toplevel_handle.is_null() {
                        let list = (*self.server).foreign_toplevel_list;
                        let title = self.get_title();
                        let app_id = self.get_app_id();
                        let state = ffi::wlr_ext_foreign_toplevel_handle_v1_state {
                            title,
                            app_id,
                        };
                        let handle = ffi::wlr_ext_foreign_toplevel_handle_v1_create(list, &state);
                        if !handle.is_null() {
                            self.foreign_toplevel_handle = handle;
                            (*handle).data = self as *mut Window as *mut _;
                        }
                    }

                    if self.wlr_toplevel_handle.is_null() {
                        let manager = (*self.server).wlr_foreign_toplevel_manager;
                        let handle = ffi::wlr_foreign_toplevel_handle_v1_create(manager);
                        if !handle.is_null() {
                            self.wlr_toplevel_handle = handle;
                            let title = self.get_title();
                            if !title.is_null() {
                                ffi::wlr_foreign_toplevel_handle_v1_set_title(handle, title);
                            }
                            let app_id = self.get_app_id();
                            if !app_id.is_null() {
                                ffi::wlr_foreign_toplevel_handle_v1_set_app_id(handle, app_id);
                            }
                        }
                    }
                };

                if new_resource {
                    let version = ffi::wl_resource_get_version(window_v1);
                    if version >= 2 {
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_UNRELIABLE_PID, self.unreliable_pid()); // sendUnreliablePid
                    }
                    if version >= 4 {
                        if !self.foreign_toplevel_handle.is_null() {
                            let identifier = (*self.foreign_toplevel_handle).identifier;
                            ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_IDENTIFIER, identifier);
                        }
                    }
                }

                if new_resource || self.wm_scheduled.dimensions_hint != self.wm_sent.dimensions_hint {
                    ffi::wl_resource_post_event(
                        window_v1,
                        ffi::ZCCE_WINDOW_V1_DIMENSIONS_HINT, // sendDimensionsHint
                        self.wm_scheduled.dimensions_hint.min_width as i32,
                        self.wm_scheduled.dimensions_hint.min_height as i32,
                        self.wm_scheduled.dimensions_hint.max_width as i32,
                        self.wm_scheduled.dimensions_hint.max_height as i32,
                    );
                    self.wm_sent.dimensions_hint = self.wm_scheduled.dimensions_hint;
                }

                if new_resource || self.wm_scheduled.decoration_hint != self.wm_sent.decoration_hint {
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_DECORATION_HINT, self.wm_scheduled.decoration_hint); // sendDecorationHint
                    self.wm_sent.decoration_hint = self.wm_scheduled.decoration_hint;
                }

                if let Some(ref offset) = self.wm_scheduled.show_window_menu_requested {
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_SHOW_WINDOW_MENU_REQUESTED, offset.x, offset.y); // sendShowWindowMenuRequested
                    self.wm_scheduled.show_window_menu_requested = None;
                }

                match self.wm_scheduled.fullscreen_requested {
                    FullscreenRequest::NoRequest => {}
                    FullscreenRequest::Fullscreen(output) => {
                        let mut out_resource = if output.is_null() { std::ptr::null_mut() } else { (*output).object };
                        if !window_v1.is_null() && !out_resource.is_null() {
                            let client_win = ffi::wl_resource_get_client(window_v1);
                            let client_out = ffi::wl_resource_get_client(out_resource);
                            if client_win != client_out {
                                log::error!(
                                    "Fullscreen output client mismatch: win_client={:?}, out_client={:?}. Fallback to null_mut",
                                    client_win,
                                    client_out
                                );
                                out_resource = std::ptr::null_mut();
                            }
                        }
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_FULLSCREEN_REQUESTED, out_resource); // sendFullscreenRequested
                    }
                    FullscreenRequest::Exit => {
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_EXIT_FULLSCREEN_REQUESTED); // sendExitFullscreenRequested
                    }
                }
                self.wm_scheduled.fullscreen_requested = FullscreenRequest::NoRequest;

                match self.wm_scheduled.maximize_requested {
                    MaximizeRequest::NoRequest => {}
                    MaximizeRequest::Maximize => {
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_MAXIMIZE_REQUESTED); // sendMaximizeRequested
                    }
                    MaximizeRequest::Unmaximize => {
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_UNMAXIMIZE_REQUESTED); // sendUnmaximizeRequested
                    }
                }
                self.wm_scheduled.maximize_requested = MaximizeRequest::NoRequest;

                if self.wm_scheduled.minimize_requested {
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_MINIMIZE_REQUESTED); // sendMinimizeRequested
                }
                self.wm_scheduled.minimize_requested = false;

                let parent = self.get_parent();
                if !parent.is_null() {
                    let parent_ref = Some((*parent).ref_key);
                    if self.wm_sent.parent.is_none() || self.wm_sent.parent != parent_ref {
                        let parent_obj = (*parent).object;
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_PARENT, parent_obj); // sendParent
                        self.wm_sent.parent = parent_ref;
                    }
                } else if self.wm_sent.parent.is_some() {
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_PARENT, std::ptr::null_mut::<ffi::wl_resource>()); // sendParent
                    self.wm_sent.parent = None;
                }

                if new_resource || self.wm_scheduled.dirty_app_id {
                    let app_id = self.get_app_id();
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_APP_ID, app_id); // sendAppId
                    self.wm_scheduled.dirty_app_id = false;
                }

                if new_resource || self.wm_scheduled.dirty_title {
                    let title = self.get_title();
                    ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_TITLE, title); // sendTitle
                    self.wm_scheduled.dirty_title = false;
                }

                if let Some(seat) = self.wm_scheduled.pointer_move_requested.as_mut() {
                    if !seat.object.is_null() {
                        ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_POINTER_MOVE_REQUESTED, seat.object); // sendPointerMoveRequested
                    }
                }
                self.wm_scheduled.pointer_move_requested = std::ptr::null_mut();

                if let Some(ref data) = self.wm_scheduled.pointer_resize_requested {
                    if let Some(seat) = unsafe { data.seat.as_ref() } {
                        if !seat.object.is_null() {
                            ffi::wl_resource_post_event(window_v1, ffi::ZCCE_WINDOW_V1_POINTER_RESIZE_REQUESTED, seat.object, data.edges); // sendPointerResizeRequested
                        }
                    }
                }
                self.wm_scheduled.pointer_resize_requested = None;
            }
        }
    }

    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_V1_CLOSED); // sendClosed // sendClosed
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_WINDOW_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
            (*self.server).wm.dirty_windowing();
            self.node.make_inert();

            for decorations in [&mut self.decorations_above as *mut ffi::wl_list, &mut self.decorations_below as *mut ffi::wl_list] {
                let list_head = decorations as *mut WlList;
                let mut curr = (*list_head).next;
                while curr != list_head {
                    let next = (*curr).next;
                    let dec = crate::container_of!(curr, Decoration, link);
                    (*dec).make_inert();
                    curr = next;
                }
            }

            let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*seats).next;
            while curr != seats {
                let next = (*curr).next;
                let seat = crate::container_of!(curr, crate::seat::Seat, link);
                if let crate::seat::Focus::Window(w) = (*seat).focused {
                    if w == self as *mut Window {
                        (*seat).focus(crate::seat::Focus::None);
                    }
                }
                curr = next;
            }
        }
    }

    pub unsafe fn manage_finish(&mut self) -> bool {
        if matches!(self.impl_type, WindowImpl::Destroying) {
            assert_eq!(self.state, WindowState::Closing);
            return false;
        }

        match self.state {
            WindowState::Init => unreachable!(),
            WindowState::Ready => {
                if self.wm_requested.dimensions.is_none() && self.wm_requested.fullscreen.is_null() {
                    return false;
                }
                self.state = WindowState::Initialized;
            }
            WindowState::Initialized | WindowState::Mapped => {}
            WindowState::Closing => return false,
        }

        if self.wm_requested.close {
            self.close();
            self.wm_requested.close = false;
        }

        let mut activated = false;
        let seats = &mut (*self.server).wm.sent.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats).next;
        while curr != seats {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link_sent);
            if let crate::seat::Focus::Window(w) = (*seat).focused {
                if w == self as *mut Window {
                    activated = true;
                    break;
                }
            }
            curr = next;
        }

        if !self.wlr_toplevel_handle.is_null() {
            ffi::wlr_foreign_toplevel_handle_v1_set_activated(self.wlr_toplevel_handle, activated);
        }

        let output = if !self.wm_requested.fullscreen.is_null() {
            self.wm_requested.fullscreen
        } else if self.is_fullscreen() {
            let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*outputs_list).next;
            let mut found_output = std::ptr::null_mut();
            while curr != outputs_list {
                let out = crate::container_of!(curr, crate::output::Output, link);
                if (*out).sent.state == crate::output::OutputStateValue::Enabled {
                    found_output = out;
                    break;
                }
                curr = (*curr).next;
            }
            found_output
        } else {
            std::ptr::null_mut()
        };

        let new_fullscreen = !output.is_null();
        if new_fullscreen && !self.was_fullscreen {
            if self.box_geom.width > 0 && self.box_geom.height > 0 {
                self.saved_width = self.box_geom.width;
                self.saved_height = self.box_geom.height;
                self.saved_virtual_x = self.virtual_x;
                self.saved_virtual_y = self.virtual_y;
                self.was_fullscreen = true;
                log::info!("[Fullscreen] Saved window {:?} geometry: {}x{} at ({}, {})", self.get_title_string().as_deref().unwrap_or(""), self.saved_width, self.saved_height, self.saved_virtual_x, self.saved_virtual_y);
            }
        } else if !new_fullscreen && self.was_fullscreen {
            if self.saved_width > 0 && self.saved_height > 0 {
                self.box_geom.width = self.saved_width;
                self.box_geom.height = self.saved_height;
                self.virtual_x = self.saved_virtual_x;
                self.virtual_y = self.saved_virtual_y;
                self.was_fullscreen = false;

                self.wm_requested.dimensions = Some(crate::window::Dimensions {
                    width: self.saved_width as u32,
                    height: self.saved_height as u32,
                });
                self.wm_requested.bounds = crate::window::Dimensions {
                    width: self.saved_width as u32,
                    height: self.saved_height as u32,
                };

                (*self.server).wm.dirty_windowing();
                log::info!("[Fullscreen] Restored window {:?} geometry: {}x{} at ({}, {})", self.get_title_string().as_deref().unwrap_or(""), self.saved_width, self.saved_height, self.saved_virtual_x, self.saved_virtual_y);
            }
        }

        let (width, height) = if !output.is_null() {
            let (w, h) = (*output).sent.dimensions();
            if self.configure_sent.width != Some(w as u32) || self.configure_sent.height != Some(h as u32) {
                self.configure_scheduled.width = Some(w as u32);
                self.configure_scheduled.height = Some(h as u32);
                self.rendering_scheduled.resend_dimensions = true;
                (Some(w as u32), Some(h as u32))
            } else {
                (None, None)
            }
        } else if let Some(dimensions) = self.wm_requested.dimensions {
            self.rendering_scheduled.resend_dimensions = true;
            (Some(dimensions.width), Some(dimensions.height))
        } else {
            (None, None)
        };
        self.wm_requested.dimensions = None;

        let is_maximized_layout = self.tiling_mode == crate::tiling::TilingMode::Cascade
            || self.tiling_mode == crate::tiling::TilingMode::Grid
            || self.tiling_mode == crate::tiling::TilingMode::Maximized;
        self.configure_scheduled = Configure {
            width,
            height,
            bounds: self.wm_requested.bounds,
            activated,
            ssd: self.wm_requested.ssd,
            tiled: self.wm_requested.tiled,
            capabilities: self.wm_requested.capabilities,
            maximized: self.wm_requested.maximized || is_maximized_layout,
            inform_fullscreen: self.wm_requested.inform_fullscreen || self.is_fullscreen(),
            resizing: self.wm_requested.resizing,
        };

        let track_configure = match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    false
                } else {
                    (*toplevel).configure()
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    false
                } else {
                    (*xwindow).configure()
                }
            }
            WindowImpl::Destroying => unreachable!(),
        };

        if track_configure && matches!(self.state, WindowState::Mapped) {
            self.surfaces.save();
            self.send_frame_done();
        }

        track_configure
    }

    pub unsafe fn render_start(&mut self) {
        match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if !toplevel.is_null() {
                    match (*toplevel).configure_state {
                        ConfigureState::Inflight(serial) => {
                            (*toplevel).configure_state = ConfigureState::TimedOut(serial);
                        }
                        ConfigureState::Acked => {
                            (*toplevel).configure_state = ConfigureState::TimedOutAcked;
                        }
                        ConfigureState::Committed => {
                            (*toplevel).configure_state = ConfigureState::Idle;
                        }
                        _ => {}
                    }
                    self.rendering_scheduled.width = (*toplevel).geometry.width as u32;
                    self.rendering_scheduled.height = (*toplevel).geometry.height as u32;
                }
            }
            WindowImpl::Xwayland(xwindow) => {
                if !xwindow.is_null() {
                    let mut w = (*(*xwindow).xsurface).width as u32;
                    let mut h = (*(*xwindow).xsurface).height as u32;
                    let has_parent = !(*(*xwindow).xsurface).parent.is_null();
                    if self.is_wine() && !has_parent && !self.is_fullscreen() {
                        w = w.saturating_sub(32);
                        h = h.saturating_sub(32);
                    }
                    self.rendering_scheduled.width = w;
                    self.rendering_scheduled.height = h;
                }
            }
            WindowImpl::Destroying => {}
        }

        let presentation_hint = self.presentation_hint();
        let sent = &mut self.rendering_sent;
        let scheduled = &mut self.rendering_scheduled;

        if matches!(self.state, WindowState::Mapped) &&
           (scheduled.resend_dimensions ||
            scheduled.width != sent.width || scheduled.height != sent.height) {
            if !self.object.is_null() {
                ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_V1_DIMENSIONS, scheduled.width as i32, scheduled.height as i32); // sendDimensions
                scheduled.resend_dimensions = false;
            }
        }
        sent.width = scheduled.width;
        sent.height = scheduled.height;
        if sent.presentation_hint != presentation_hint {
            if !self.object.is_null() {
                let version = ffi::wl_resource_get_version(self.object);
                if version >= 4 {
                    ffi::wl_resource_post_event(self.object, ffi::ZCCE_WINDOW_V1_PRESENTATION_HINT, presentation_hint); // sendPresentationHint
                }
            }
            sent.presentation_hint = presentation_hint;
        }
    }

    pub unsafe fn presentation_hint(&self) -> ffi::zcce_output_v1_presentation_mode {
        let root = self.root_surface();
        if root.is_null() {
            return ffi::zcce_output_v1_presentation_mode_ZCCE_OUTPUT_V1_PRESENTATION_MODE_VSYNC;
        }
        
        // tearing control check stub:
        // switch (server.tearing_control_manager.hintFromSurface(root)) {
        //     .async => .async,
        //     .vsync => .vsync,
        // }
        // For now, return VSYNC by default.
        ffi::zcce_output_v1_presentation_mode_ZCCE_OUTPUT_V1_PRESENTATION_MODE_VSYNC
    }

    pub unsafe fn notify_title(&mut self) {
        self.wm_scheduled.dirty_title = true;
        self.try_restore();
        (*self.server).wm.dirty_windowing();

        if !self.foreign_toplevel_handle.is_null() {
            let title = self.get_title();
            let app_id = self.get_app_id();
            let state = ffi::wlr_ext_foreign_toplevel_handle_v1_state {
                title,
                app_id,
            };
            ffi::wlr_ext_foreign_toplevel_handle_v1_update_state(self.foreign_toplevel_handle, &state);
        }

        if !self.wlr_toplevel_handle.is_null() {
            let title = self.get_title();
            if !title.is_null() {
                ffi::wlr_foreign_toplevel_handle_v1_set_title(self.wlr_toplevel_handle, title);
            }
        }
    }

    pub unsafe fn notify_app_id(&mut self) {
        self.wm_scheduled.dirty_app_id = true;
        let app_id_str = self.get_app_id_string();
        if app_id_str.as_deref().map_or(false, |id| id.starts_with("cce-status") || id == "cce-wallpaper") {
            self.tiling_mode = crate::tiling::TilingMode::Status;
        }
        self.try_restore();
        (*self.server).wm.dirty_windowing();

        if !self.foreign_toplevel_handle.is_null() {
            let title = self.get_title();
            let app_id = self.get_app_id();
            let state = ffi::wlr_ext_foreign_toplevel_handle_v1_state {
                title,
                app_id,
            };
            ffi::wlr_ext_foreign_toplevel_handle_v1_update_state(self.foreign_toplevel_handle, &state);
        }

        if !self.wlr_toplevel_handle.is_null() {
            let app_id = self.get_app_id();
            if !app_id.is_null() {
                ffi::wlr_foreign_toplevel_handle_v1_set_app_id(self.wlr_toplevel_handle, app_id);
            }
        }
    }

    pub unsafe fn render_finish(&mut self) {
        let requested = &self.rendering_requested;
        let enabled = !requested.hidden && (matches!(self.state, WindowState::Mapped) || matches!(self.state, WindowState::Closing));

        let title_ptr = match self.impl_type {
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() { std::ptr::null() } else { (*(*xwindow).xsurface).title }
            }
            _ => std::ptr::null(),
        };
        let title = if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") };
        if title.contains("Ubisoft") {
            log::info!("render_finish for '{}' (addr={:p}): enabled={} hidden={} state={:?}", title, self as *const Window, enabled, requested.hidden, self.state);
        }

        ffi::wlr_scene_node_set_enabled(self.tree as *mut ffi::wlr_scene_node, enabled);
        ffi::wlr_scene_node_set_enabled(self.popup_tree as *mut ffi::wlr_scene_node, enabled);

        if enabled {
            let app_id = self.get_app_id_string().unwrap_or_default();
            let is_status = self.tiling_mode == crate::tiling::TilingMode::Status ||
                            app_id.starts_with("cce-status");
            let is_cce_app = app_id.starts_with("cce-");
            let blur_enabled = requested.blur && (self.wm_requested.ssd || is_cce_app || is_status);
            let mut ignore_transparent = (*self.server).wm.layout.window_backdrop_blur_ignore_transparent;
            if is_status {
                ignore_transparent = (*self.server).wm.layout.status_backdrop_blur_ignore_transparent;
            }
            let use_optimized = if is_status { false } else { (*self.server).wm.layout.scenefx_optimized_blur };
            let toplevel_w = match self.impl_type {
                WindowImpl::Toplevel(toplevel) => {
                    if toplevel.is_null() { 0 } else { (*toplevel).geometry.width }
                }
                _ => 0,
            };
            let toplevel_h = match self.impl_type {
                WindowImpl::Toplevel(toplevel) => {
                    if toplevel.is_null() { 0 } else { (*toplevel).geometry.height }
                }
                _ => 0,
            };
            let actual_w = if self.rendering_sent.width > 0 { self.rendering_sent.width } else { toplevel_w as u32 };
            let actual_h = if self.rendering_sent.height > 0 { self.rendering_sent.height } else { toplevel_h as u32 };
            let width = (actual_w as f64 * self.scale) as i32;
            let height = (actual_h as f64 * self.scale) as i32;
            ffi::river_scene_node_enable_blur(
                self.tree as *mut ffi::wlr_scene_node,
                blur_enabled,
                use_optimized,
                ignore_transparent,
                0,
                0,
                width,
                height,
            );
            ffi::river_scene_node_set_opacity(self.tree as *mut ffi::wlr_scene_node, requested.opacity);

            let radius = if self.is_fullscreen() {
                0
            } else if requested.circular {
                let w = self.rendering_sent.width as i32;
                let h = self.rendering_sent.height as i32;
                w.min(h) / 2
            } else if self.wm_requested.ssd || is_cce_app {
                (*self.server).wm.layout.backplate_corner_radius
            } else {
                0
            };

            ffi::river_scene_node_set_corner_radius(
                self.surfaces.tree as *mut ffi::wlr_scene_node,
                radius,
            );
            ffi::river_scene_rect_set_corner_radius(
                self.window_background,
                radius,
            );

            struct ScaleData {
                scale: f64,
                ancestor: *mut ffi::wlr_scene_node,
            }

            unsafe extern "C" fn set_expose_scale_iterator(
                buffer: *mut ffi::wlr_scene_buffer,
                sx: i32,
                sy: i32,
                user_data: *mut std::ffi::c_void,
            ) {
                let data = &*(user_data as *const ScaleData);
                let node = buffer as *mut ffi::wlr_scene_node;

                let surface = ffi::river_scene_node_get_surface(node);
                if !surface.is_null() {
                    let w = ffi::river_wlr_surface_get_width(surface);
                    let h = ffi::river_wlr_surface_get_height(surface);
                    if data.scale == 1.0 {
                        ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                        ffi::river_scene_node_set_position_if_changed(node, 0, 0);
                    } else {
                        let dest_w = (w as f64 * data.scale) as i32;
                        let dest_h = (h as f64 * data.scale) as i32;
                        ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                        let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                        let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                        let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                        ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                    }
                } else if data.scale == 1.0 {
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, 0, 0);
                    ffi::river_scene_node_set_position_if_changed(node, sx, sy);
                } else {
                    let w = ffi::river_scene_buffer_get_width(buffer);
                    let h = ffi::river_scene_buffer_get_height(buffer);
                    let dest_w = (w as f64 * data.scale) as i32;
                    let dest_h = (h as f64 * data.scale) as i32;
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                    let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                    let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                    let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                    ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                }
            }

            let scale_data_surfaces = ScaleData { scale: self.scale, ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.tree as *mut ffi::wlr_scene_node,
                Some(set_expose_scale_iterator),
                &scale_data_surfaces as *const ScaleData as *mut std::ffi::c_void,
            );

            if self.surfaces.saved {
                let scale_data_saved = ScaleData { scale: self.scale, ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node };
                ffi::wlr_scene_node_for_each_buffer(
                    self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                    Some(set_expose_scale_iterator),
                    &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
                );
            }
            
            let scale_data_popup = ScaleData { scale: self.scale, ancestor: self.popup_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.popup_tree as *mut ffi::wlr_scene_node,
                Some(set_expose_scale_iterator),
                &scale_data_popup as *const ScaleData as *mut std::ffi::c_void,
            );
            self.last_applied_scale = self.scale;
        }

        // During an interactive resize, size the box from the client's
        // CURRENT committed geometry instead of the render-start snapshot
        // (rendering_sent): commits land between render_start and
        // render_finish, and the anchored position (rendering_requested.x,
        // updated by the commit handler) always tracks the newest commit.
        // Pairing it with the older snapshot size clips the surface short
        // and makes the anchored edge bounce every cycle.
        let mut resize_synced = false;
        if self.resize_edges.is_some() {
            if let WindowImpl::Toplevel(toplevel) = self.impl_type {
                if !toplevel.is_null() {
                    self.box_geom.width = (*toplevel).geometry.width;
                    self.box_geom.height = (*toplevel).geometry.height;
                    resize_synced = true;
                }
            }
        }
        if !resize_synced {
            if self.rendering_sent.width > 0 {
                self.box_geom.width = self.rendering_sent.width as i32;
            }
            if self.rendering_sent.height > 0 {
                self.box_geom.height = self.rendering_sent.height as i32;
            }
        }

        let mut clip = requested.clip;
        let mut content_clip = requested.content_clip;

        let output = if !self.wm_requested.fullscreen.is_null() {
            self.wm_requested.fullscreen
        } else if self.is_fullscreen() {
            let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*outputs_list).next;
            let mut found_output = std::ptr::null_mut();
            while curr != outputs_list {
                let out = crate::container_of!(curr, crate::output::Output, link);
                if (*out).sent.state == crate::output::OutputStateValue::Enabled {
                    found_output = out;
                    break;
                }
                curr = (*curr).next;
            }
            found_output
        } else {
            std::ptr::null_mut()
        };

        if !output.is_null() {
            self.box_geom.x = (*output).sent.x;
            self.box_geom.y = (*output).sent.y;

            let app_id_ptr = self.get_app_id();
            let (is_status_bar, is_wallpaper) = if !app_id_ptr.is_null() {
                let app_id = std::ffi::CStr::from_ptr(app_id_ptr).to_string_lossy();
                (app_id.starts_with("cce-status"), app_id.as_ref() == "cce-wallpaper")
            } else {
                (false, false)
            };

            ffi::wlr_scene_node_set_enabled(self.fullscreen_background as *mut ffi::wlr_scene_node, !is_status_bar && !is_wallpaper);
            let (width, height) = (*output).sent.dimensions();
            ffi::wlr_scene_rect_set_size(self.fullscreen_background, width as i32, height as i32);
            clip = ffi::wlr_box { x: 0, y: 0, width: width as i32, height: height as i32 };
            content_clip = ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 };

            ffi::wlr_scene_node_set_enabled(self.border.left as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.right as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.top as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.bottom as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.window_background as *mut ffi::wlr_scene_node, false);
        } else {
            self.box_geom.x = requested.x;
            self.box_geom.y = requested.y;
            ffi::wlr_scene_node_set_enabled(self.fullscreen_background as *mut ffi::wlr_scene_node, false);
            self.draw_borders();
        }

        ffi::river_scene_node_set_position_if_changed(self.tree as *mut ffi::wlr_scene_node, self.box_geom.x, self.box_geom.y);
        ffi::river_scene_node_set_position_if_changed(self.popup_tree as *mut ffi::wlr_scene_node, self.box_geom.x, self.box_geom.y);

        let (geom_x, geom_y) = match self.impl_type {
            WindowImpl::Toplevel(toplevel) => {
                if toplevel.is_null() {
                    (0, 0)
                } else {
                    let mut x = (*toplevel).geometry.x;
                    let mut y = (*toplevel).geometry.y;
                    if self.wm_requested.ssd {
                        x = 0;
                        y = 0;
                    }

                    (x, y)
                }
            }
            _ => (0, 0),
        };
        ffi::river_scene_node_set_position_if_changed(self.surfaces.tree as *mut ffi::wlr_scene_node, -geom_x, -geom_y);

        self.apply_surface_clip(&clip, &content_clip);

        for decorations in [&mut self.decorations_above as *mut ffi::wl_list, &mut self.decorations_below as *mut ffi::wl_list] {
            let list_head = decorations as *mut WlList;
            let mut curr = (*list_head).next;
            while curr != list_head {
                let next = (*curr).next;
                let dec = crate::container_of!(curr, Decoration, link);
                (*dec).render_finish(&clip);
                curr = next;
            }
        }

        match self.impl_type {
            WindowImpl::Xwayland(xwindow) => {
                if !xwindow.is_null() {
                    if !(*xwindow).surface_tree.is_null() {
                        let has_parent = !(*(*xwindow).xsurface).parent.is_null();
                        if self.is_wine() && !has_parent && !self.is_fullscreen() {
                            ffi::wlr_scene_node_set_position((*xwindow).surface_tree as *mut ffi::wlr_scene_node, -16, -16);
                        } else {
                            ffi::wlr_scene_node_set_position((*xwindow).surface_tree as *mut ffi::wlr_scene_node, 0, 0);
                        }
                    }
                    (*xwindow).configure();
                }
            }
            _ => {}
        }
    }

    pub unsafe fn scale_only_render_finish(&mut self) {
        if self.scale == 1.0 {
            self.last_applied_scale = 1.0;
            return;
        }

        if self.scale == self.last_applied_scale {
            return;
        }

        self.last_applied_scale = self.scale;

        struct ScaleData {
            scale: f64,
            ancestor: *mut ffi::wlr_scene_node,
        }

        unsafe extern "C" fn set_expose_scale_iterator(
            buffer: *mut ffi::wlr_scene_buffer,
            sx: i32,
            sy: i32,
            user_data: *mut std::ffi::c_void,
        ) {
            let data = &*(user_data as *const ScaleData);
            let node = buffer as *mut ffi::wlr_scene_node;

            let surface = ffi::river_scene_node_get_surface(node);
            if !surface.is_null() {
                let w = ffi::river_wlr_surface_get_width(surface);
                let h = ffi::river_wlr_surface_get_height(surface);
                if data.scale == 1.0 {
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                    ffi::river_scene_node_set_position_if_changed(node, 0, 0);
                } else {
                    let dest_w = (w as f64 * data.scale) as i32;
                    let dest_h = (h as f64 * data.scale) as i32;
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                    let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                    let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                    let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                    ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                }
            } else if data.scale == 1.0 {
                ffi::river_scene_buffer_set_dest_size_if_changed(buffer, 0, 0);
                ffi::river_scene_node_set_position_if_changed(node, sx, sy);
            } else {
                let w = ffi::river_scene_buffer_get_width(buffer);
                let h = ffi::river_scene_buffer_get_height(buffer);
                let dest_w = (w as f64 * data.scale) as i32;
                let dest_h = (h as f64 * data.scale) as i32;
                ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
            }
        }

        let scale_data_surfaces = ScaleData { scale: self.scale, ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.surfaces.tree as *mut ffi::wlr_scene_node,
            Some(set_expose_scale_iterator),
            &scale_data_surfaces as *const ScaleData as *mut std::ffi::c_void,
        );

        if self.surfaces.saved {
            let scale_data_saved = ScaleData { scale: self.scale, ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                Some(set_expose_scale_iterator),
                &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
            );
        }

        let scale_data_popup = ScaleData { scale: self.scale, ancestor: self.popup_tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.popup_tree as *mut ffi::wlr_scene_node,
            Some(set_expose_scale_iterator),
            &scale_data_popup as *const ScaleData as *mut std::ffi::c_void,
        );

        for decorations in [&mut self.decorations_above as *mut ffi::wl_list, &mut self.decorations_below as *mut ffi::wl_list] {
            let list_head = decorations as *mut WlList;
            let mut curr = (*list_head).next;
            while curr != list_head {
                let next = (*curr).next;
                let dec = crate::container_of!(curr, Decoration, link);
                (*dec).scale_only_render_finish();
                curr = next;
            }
        }
    }

    pub unsafe fn render_viewport_update(&mut self) {
        let requested = &self.rendering_requested;
        let enabled = !requested.hidden && (matches!(self.state, WindowState::Mapped) || matches!(self.state, WindowState::Closing));

        ffi::wlr_scene_node_set_enabled(self.tree as *mut ffi::wlr_scene_node, enabled);
        ffi::wlr_scene_node_set_enabled(self.popup_tree as *mut ffi::wlr_scene_node, enabled);

        if enabled {
            self.box_geom.x = requested.x;
            self.box_geom.y = requested.y;
            ffi::river_scene_node_set_position_if_changed(self.tree as *mut ffi::wlr_scene_node, self.box_geom.x, self.box_geom.y);
            ffi::river_scene_node_set_position_if_changed(self.popup_tree as *mut ffi::wlr_scene_node, self.box_geom.x, self.box_geom.y);

            // Disable backdrop blur during active viewport zoom/pan for maximum performance,
            // EXCEPT for cce-* apps, which we keep blurred during the pan so their translucent
            // backgrounds don't flicker as blur toggles on/off across motion frames.
            let app_id = self.get_app_id_string().unwrap_or_default();
            if app_id.starts_with("cce-") {
                let is_status = self.tiling_mode == crate::tiling::TilingMode::Status ||
                                app_id.starts_with("cce-status");
                let is_cce_app = app_id.starts_with("cce-");
                let blur_enabled = requested.blur && (self.wm_requested.ssd || is_cce_app || is_status);
                let mut ignore_transparent = (*self.server).wm.layout.window_backdrop_blur_ignore_transparent;
                if is_status {
                    ignore_transparent = (*self.server).wm.layout.status_backdrop_blur_ignore_transparent;
                }
                let use_optimized = if is_status { false } else { (*self.server).wm.layout.scenefx_optimized_blur };
                let toplevel_w = match self.impl_type {
                    WindowImpl::Toplevel(toplevel) => {
                        if toplevel.is_null() { 0 } else { (*toplevel).geometry.width }
                    }
                    _ => 0,
                };
                let toplevel_h = match self.impl_type {
                    WindowImpl::Toplevel(toplevel) => {
                        if toplevel.is_null() { 0 } else { (*toplevel).geometry.height }
                    }
                    _ => 0,
                };
                let actual_w = if self.rendering_sent.width > 0 { self.rendering_sent.width } else { toplevel_w as u32 };
                let actual_h = if self.rendering_sent.height > 0 { self.rendering_sent.height } else { toplevel_h as u32 };
                let width = (actual_w as f64 * self.scale) as i32;
                let height = (actual_h as f64 * self.scale) as i32;
                ffi::river_scene_node_enable_blur(
                    self.tree as *mut ffi::wlr_scene_node,
                    blur_enabled,
                    use_optimized,
                    ignore_transparent,
                    0,
                    0,
                    width,
                    height,
                );
            } else {
                ffi::river_scene_node_enable_blur(self.tree as *mut ffi::wlr_scene_node, false, (*self.server).wm.layout.scenefx_optimized_blur, true, 0, 0, 0, 0);
            }

            self.scale_only_render_finish();
            self.draw_borders();
        }
    }

    pub unsafe fn draw_borders(&mut self) {
        let requested = &self.rendering_requested;

        let border = &requested.border;
        let border_color = border.color;
        ffi::river_scene_node_set_position_if_changed(self.window_background as *mut ffi::wlr_scene_node, 0, 0);
        let bg_width = (self.box_geom.width as f64 * self.scale) as i32;
        let bg_height = (self.box_geom.height as f64 * self.scale) as i32;
        ffi::river_scene_rect_set_size_if_changed(self.window_background, bg_width, bg_height);
        ffi::wlr_scene_rect_set_color(self.window_background, border_color.as_ptr());
        ffi::river_scene_rect_set_corner_radius(self.window_background, (border.corner_radius as f64 * self.scale) as i32);
        ffi::wlr_scene_node_set_enabled(self.window_background as *mut ffi::wlr_scene_node, !requested.hidden && self.wm_requested.ssd);

        // The border draws as 8 zone segments (4 edge bars + 4 two-rect L
        // corners) with BORDER_SEGMENT_GAP between them; the hovered zone
        // draws in hover_color. Underneath, the 4 full-band rects stay
        // enabled but transparent as scene hit-test catchers, so the pointer
        // never falls through the gaps (and width 0 keeps the legacy
        // invisible 8px virtual resize zones).
        let is_virtual_border = border.width == 0;
        if requested.circular {
            ffi::wlr_scene_node_set_enabled(self.border.left as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.right as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.top as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.bottom as *mut ffi::wlr_scene_node, false);
            for &seg in self.border.segments.iter() {
                ffi::wlr_scene_node_set_enabled(seg as *mut ffi::wlr_scene_node, false);
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
            let band = if is_virtual_border { 8 } else { border.width as i32 };
            let transparent = [0.0f32; 4];

            // The rounded-frame path used to leave radius/clip state on the
            // top band rect; keep it reset.
            ffi::river_scene_rect_set_corner_radius(self.border.top, 0);
            ffi::wlr_scene_rect_set_clipped_region(self.border.top, ffi::clipped_region_get_default());

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

            // Full-band hit catchers: sides span the corners vertically.
            let b = ffi::wlr_box { x: -band, y: -band, width: band, height: content.height + 2 * band };
            apply(self.border.left, b, &transparent, true);
            let b = ffi::wlr_box { x: content.width, y: -band, width: band, height: content.height + 2 * band };
            apply(self.border.right, b, &transparent, true);
            let b = ffi::wlr_box { x: 0, y: -band, width: content.width, height: band };
            apply(self.border.top, b, &transparent, true);
            let b = ffi::wlr_box { x: 0, y: content.height, width: content.width, height: band };
            apply(self.border.bottom, b, &transparent, true);

            if is_virtual_border {
                for &seg in self.border.segments.iter() {
                    ffi::wlr_scene_node_set_enabled(seg as *mut ffi::wlr_scene_node, false);
                }
                return;
            }

            let bw = border.width as i32;
            let layout = &(*self.server).wm.layout;
            let cl = border_corner_len(bw as f64, layout.border_corner_length) as i32;
            let g = layout.border_segment_gap;
            let arm = cl - bw;
            // Edge bars span between the corner zones, inset by the gap.
            let bar_x = cl - bw + g;
            let bar_w = content.width + 2 * bw - 2 * cl - 2 * g;
            let bar_y = cl - bw + g;
            let bar_h = content.height + 2 * bw - 2 * cl - 2 * g;

            let color_for = |elem: BorderElement| -> [f32; 4] {
                if self.hovered_border_element == Some(elem) {
                    border.hover_color
                } else {
                    border_color
                }
            };
            let e = &border.edges;
            use BorderElement::*;

            let segs: [(usize, ffi::wlr_box, BorderElement, bool); 12] = [
                (SEG_TOP, ffi::wlr_box { x: bar_x, y: -bw, width: bar_w, height: bw }, Top, e.top),
                (SEG_BOTTOM, ffi::wlr_box { x: bar_x, y: content.height, width: bar_w, height: bw }, Bottom, e.bottom),
                (SEG_LEFT, ffi::wlr_box { x: -bw, y: bar_y, width: bw, height: bar_h }, Left, e.left),
                (SEG_RIGHT, ffi::wlr_box { x: content.width, y: bar_y, width: bw, height: bar_h }, Right, e.right),
                (SEG_TL_H, ffi::wlr_box { x: -bw, y: -bw, width: cl, height: bw }, TopLeft, e.top && e.left),
                (SEG_TL_V, ffi::wlr_box { x: -bw, y: 0, width: bw, height: arm }, TopLeft, e.top && e.left),
                (SEG_TR_H, ffi::wlr_box { x: content.width + bw - cl, y: -bw, width: cl, height: bw }, TopRight, e.top && e.right),
                (SEG_TR_V, ffi::wlr_box { x: content.width, y: 0, width: bw, height: arm }, TopRight, e.top && e.right),
                (SEG_BL_H, ffi::wlr_box { x: -bw, y: content.height, width: cl, height: bw }, BottomLeft, e.bottom && e.left),
                (SEG_BL_V, ffi::wlr_box { x: -bw, y: content.height - arm, width: bw, height: arm }, BottomLeft, e.bottom && e.left),
                (SEG_BR_H, ffi::wlr_box { x: content.width + bw - cl, y: content.height, width: cl, height: bw }, BottomRight, e.bottom && e.right),
                (SEG_BR_V, ffi::wlr_box { x: content.width, y: content.height - arm, width: bw, height: arm }, BottomRight, e.bottom && e.right),
            ];
            for (idx, bx, elem, enabled) in segs {
                apply(self.border.segments[idx], bx, &color_for(elem), enabled);
            }
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
            WindowImpl::Xwayland(xwindow) => {
                if !xwindow.is_null() {
                    let title_ptr = (*(*xwindow).xsurface).title;
                    let title = if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") };
                    if title.contains("Ubisoft") {
                        log::info!(
                            "XWayland window clip check: title='{}' box_geom=({}, {}, {}, {}) xsurface=({}, {}, {}, {})",
                            title,
                            self.box_geom.x,
                            self.box_geom.y,
                            self.box_geom.width,
                            self.box_geom.height,
                            (*(*xwindow).xsurface).x,
                            (*(*xwindow).xsurface).y,
                            (*(*xwindow).xsurface).width,
                            (*(*xwindow).xsurface).height,
                        );
                    }
                }
            }
            _ => {}
        }

        let children_head = ffi::river_scene_tree_get_children(self.surfaces.tree) as *mut WlList;
        if (*children_head).next != children_head {
            ffi::wlr_scene_subsurface_tree_set_clip(self.surfaces.tree as *mut ffi::wlr_scene_node, std::ptr::null());
        }
    }
}

unsafe fn clock_gettime(clk_id: libc::clockid_t, tp: &mut libc::timespec) -> libc::c_int {
    libc::clock_gettime(clk_id, tp)
}

unsafe extern "C" fn window_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn window_close(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.close = true;
}

unsafe extern "C" fn window_get_node(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    if !(*window).node.object.is_null() {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_NODE_EXISTS,
            b"window already has a node object\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).node.create_object(client, ffi::wl_resource_get_version(resource) as u32, id);
}

unsafe extern "C" fn window_propose_dimensions(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    width: i32,
    height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    if width < 0 || height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_DIMENSIONS,
            b"dimensions must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    if (*window).get_parent().is_null() {
        (*window).wm_requested.dimensions = Some(Dimensions {
            width: width as u32,
            height: height as u32,
        });
    }
}

unsafe extern "C" fn window_hide(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.hidden = true;
}

unsafe extern "C" fn window_show(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.hidden = false;
}

unsafe extern "C" fn window_use_csd(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.ssd = false;
    (*server).wm.dirty_windowing();
}

unsafe extern "C" fn window_use_ssd(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.ssd = true;
    (*server).wm.dirty_windowing();
}

unsafe extern "C" fn window_set_borders(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    edges: u32,
    width: i32,
    r: u32,
    g: u32,
    b: u32,
    a: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    if width < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_BORDER,
            b"border width must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    let alpha = (a as f64 / u32::MAX as f64) as f32;
    // Protocol channels are straight alpha; scene colors are premultiplied.
    let color = [
        (r as f64 / u32::MAX as f64) as f32 * alpha,
        (g as f64 / u32::MAX as f64) as f32 * alpha,
        (b as f64 / u32::MAX as f64) as f32 * alpha,
        alpha,
    ];
    (*window).rendering_requested.border = Border {
        edges: Edges::from_u32(edges),
        width: width as u32,
        color,
        // Protocol-set borders don't participate in hover highlighting.
        hover_color: color,
        corner_radius: 0,
    };
}

unsafe extern "C" fn window_set_tiled(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    edges: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.tiled = edges;
}

unsafe extern "C" fn window_get_decoration_above(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    wl_surface: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let wlr_surface = ffi::wlr_surface_from_resource(wl_surface);
    let decoration = match Decoration::create(
        client,
        ffi::wl_resource_get_version(resource) as u32,
        id,
        wlr_surface,
        (*window).decorations_above_tree,
        window,
    ) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to create decoration: {}", e);
            ffi::wl_client_post_no_memory(client);
            return;
        }
    };
    let list_head = &mut (*window).decorations_above as *mut ffi::wl_list as *mut WlList;
    wl_list_insert((*list_head).prev, &mut (*decoration).link as *mut ffi::wl_list as *mut WlList);
}

unsafe extern "C" fn window_get_decoration_below(
    client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    id: u32,
    wl_surface: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let wlr_surface = ffi::wlr_surface_from_resource(wl_surface);
    let decoration = match Decoration::create(
        client,
        ffi::wl_resource_get_version(resource) as u32,
        id,
        wlr_surface,
        (*window).decorations_below_tree,
        window,
    ) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to create decoration: {}", e);
            ffi::wl_client_post_no_memory(client);
            return;
        }
    };
    let list_head = &mut (*window).decorations_below as *mut ffi::wl_list as *mut WlList;
    wl_list_insert((*list_head).prev, &mut (*decoration).link as *mut ffi::wl_list as *mut WlList);
}

unsafe extern "C" fn window_inform_resize_start(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.resizing = true;
}

unsafe extern "C" fn window_inform_resize_end(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.resizing = false;
}

unsafe extern "C" fn window_set_capabilities(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    caps: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.capabilities = caps;
}

unsafe extern "C" fn window_inform_maximized(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.maximized = true;
}

unsafe extern "C" fn window_inform_unmaximized(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.maximized = false;
}

unsafe extern "C" fn window_inform_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.inform_fullscreen = true;
}

unsafe extern "C" fn window_inform_not_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.inform_fullscreen = false;
}

unsafe extern "C" fn window_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    output: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    let out = if output.is_null() {
        std::ptr::null_mut()
    } else {
        let wlr_output = ffi::wlr_output_from_resource(output);
        if wlr_output.is_null() {
            std::ptr::null_mut()
        } else {
            ffi::river_wlr_output_get_data(wlr_output) as *mut crate::output::Output
        }
    };
    (*window).wm_requested.fullscreen = out;
}

unsafe extern "C" fn window_exit_fullscreen(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    (*window).wm_requested.fullscreen = std::ptr::null_mut();
}

unsafe extern "C" fn window_set_clip_box(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    if width < 0 || height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_CLIP_BOX,
            b"width/height must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).rendering_requested.clip = ffi::wlr_box {
        x,
        y,
        width,
        height,
    };
}

unsafe extern "C" fn window_set_content_clip_box(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    if width < 0 || height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_CLIP_BOX,
            b"width/height must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).rendering_requested.content_clip = ffi::wlr_box {
        x,
        y,
        width,
        height,
    };
}

unsafe extern "C" fn window_set_dimension_bounds(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    max_width: i32,
    max_height: i32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_windowing() {
        return;
    }
    if max_width < 0 || max_height < 0 {
        ffi::wl_resource_post_error(
            resource,
            ffi::zcce_window_v1_error_ZCCE_WINDOW_V1_ERROR_INVALID_DIMENSIONS,
            b"dimensions must be greater than or equal to 0\0".as_ptr() as *const _,
        );
        return;
    }
    (*window).wm_requested.bounds = Dimensions {
        width: max_width as u32,
        height: max_height as u32,
    };
}

unsafe extern "C" fn window_set_opacity(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    opacity: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    let opacity_f32 = opacity as f32 / u32::MAX as f32;
    (*window).rendering_requested.opacity = opacity_f32;
}

unsafe extern "C" fn window_set_circular(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    circular: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.circular = circular != 0;
}

unsafe extern "C" fn window_set_blur(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    blur: u32,
) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if window.is_null() {
        return;
    }
    let server = (*window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*window).rendering_requested.blur = blur != 0;
}

// zcce_window_v1 implementation
static WINDOW_INTERFACE: ffi::zcce_window_v1_interface = ffi::zcce_window_v1_interface {
    destroy: Some(window_destroy),
    close: Some(window_close),
    get_node: Some(window_get_node),
    propose_dimensions: Some(window_propose_dimensions),
    hide: Some(window_hide),
    show: Some(window_show),
    use_csd: Some(window_use_csd),
    use_ssd: Some(window_use_ssd),
    set_borders: Some(window_set_borders),
    set_tiled: Some(window_set_tiled),
    get_decoration_above: Some(window_get_decoration_above),
    get_decoration_below: Some(window_get_decoration_below),
    inform_resize_start: Some(window_inform_resize_start),
    inform_resize_end: Some(window_inform_resize_end),
    set_capabilities: Some(window_set_capabilities),
    inform_maximized: Some(window_inform_maximized),
    inform_unmaximized: Some(window_inform_unmaximized),
    inform_fullscreen: Some(window_inform_fullscreen),
    inform_not_fullscreen: Some(window_inform_not_fullscreen),
    fullscreen: Some(window_fullscreen),
    exit_fullscreen: Some(window_exit_fullscreen),
    set_clip_box: Some(window_set_clip_box),
    set_content_clip_box: Some(window_set_content_clip_box),
    set_dimension_bounds: Some(window_set_dimension_bounds),
    set_opacity: Some(window_set_opacity),
    set_circular: Some(window_set_circular),
    set_blur: Some(window_set_blur),
};

static INERT_WINDOW_INTERFACE: ffi::zcce_window_v1_interface = ffi::zcce_window_v1_interface {
    destroy: Some(window_destroy),
    close: None,
    get_node: None,
    propose_dimensions: None,
    hide: None,
    show: None,
    use_csd: None,
    use_ssd: None,
    set_borders: None,
    set_tiled: None,
    get_decoration_above: None,
    get_decoration_below: None,
    inform_resize_start: None,
    inform_resize_end: None,
    set_capabilities: None,
    inform_maximized: None,
    inform_unmaximized: None,
    inform_fullscreen: None,
    inform_not_fullscreen: None,
    fullscreen: None,
    exit_fullscreen: None,
    set_clip_box: None,
    set_content_clip_box: None,
    set_dimension_bounds: None,
    set_opacity: None,
    set_circular: None,
    set_blur: None,
};

unsafe extern "C" fn handle_destroy_resource(resource: *mut ffi::wl_resource) {
    let window = ffi::wl_resource_get_user_data(resource) as *mut Window;
    if !window.is_null() {
        if (*window).object != resource {
            return;
        }
        (*window).object = std::ptr::null_mut();
        (*window).node.make_inert();
        
        for decorations in [&mut (*window).decorations_above as *mut ffi::wl_list, &mut (*window).decorations_below as *mut ffi::wl_list] {
            let list_head = decorations as *mut WlList;
            let mut curr = (*list_head).next;
            while curr != list_head {
                let next = (*curr).next;
                let dec = crate::container_of!(curr, Decoration, link);
                (*dec).make_inert();
                curr = next;
            }
        }
    }
}

// zcce_decoration_v1 implementation
pub struct DecorationRenderingRequested {
    pub offset_x: i32,
    pub offset_y: i32,
    pub sync_next_commit: bool,
    pub blur: bool,
}

pub struct Decoration {
    pub object: *mut ffi::wl_resource, // zcce_decoration_v1
    pub surface: *mut ffi::wlr_surface,
    pub tree: *mut ffi::wlr_scene_tree,
    pub surfaces: crate::scene::SaveableSurfaces,
    pub link: ffi::wl_list,
    pub window: *mut Window,
    pub rendering_requested: DecorationRenderingRequested,
}

impl Decoration {
    pub unsafe fn create(
        client: *mut ffi::wl_client,
        version: u32,
        id: u32,
        surface: *mut ffi::wlr_surface,
        parent: *mut ffi::wlr_scene_tree,
        window: *mut Window,
    ) -> Result<*mut Self, &'static str> {
        let decoration_v1 = ffi::wl_resource_create(client, &ffi::zcce_decoration_v1_interface, version as i32, id);
        if decoration_v1.is_null() {
            ffi::wl_client_post_no_memory(client);
            return Err("wl_resource_create failed");
        }

        if !ffi::wlr_surface_set_role(
            surface,
            &raw const DECORATION_ROLE,
            decoration_v1,
            ffi::zcce_window_manager_v1_error_ZCCE_WINDOW_MANAGER_V1_ERROR_ROLE,
        ) {
            return Err("wlr_surface_set_role failed");
        }
        ffi::river_wlr_surface_set_role_object(surface, decoration_v1);

        let tree = ffi::wlr_scene_tree_create(parent);
        if tree.is_null() {
            return Err("wlr_scene_tree_create failed");
        }

        let surfaces = crate::scene::SaveableSurfaces::init(tree)?;
        let subsurface_tree = ffi::wlr_scene_subsurface_tree_create(surfaces.tree, surface);
        if subsurface_tree.is_null() {
            ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
            return Err("wlr_scene_subsurface_tree_create failed");
        }

        let dec = Box::new(Decoration {
            object: decoration_v1,
            surface,
            tree,
            surfaces,
            link: std::mem::zeroed(),
            window,
            rendering_requested: DecorationRenderingRequested {
                offset_x: 0,
                offset_y: 0,
                sync_next_commit: false,
                blur: false,
            },
        });
        let raw = Box::into_raw(dec);

        ffi::wl_resource_set_implementation(
            decoration_v1,
            &DECORATION_INTERFACE as *const _ as *const _,
            raw as *mut _,
            Some(handle_dec_destroy_resource),
        );

        Ok(raw)
    }

    pub unsafe fn destroy(&mut self) {
        assert!(self.object.is_null());
        ffi::wlr_scene_node_destroy(self.tree as *mut ffi::wlr_scene_node);
        wl_list_remove(&mut self.link as *mut ffi::wl_list as *mut WlList);
        let _ = Box::from_raw(self);
    }

    pub unsafe fn make_inert(&mut self) {
        if !self.object.is_null() {
            ffi::wl_resource_set_implementation(
                self.object,
                &INERT_DECORATION_INTERFACE as *const _ as *const _,
                std::ptr::null_mut(),
                None,
            );
            self.object = std::ptr::null_mut();
        }
        if !self.surface.is_null() {
            ffi::river_wlr_surface_set_role_object(self.surface, std::ptr::null_mut());
        }
        self.surfaces.save();
    }

    pub unsafe fn render_finish(&mut self, _window_clip: *const ffi::wlr_box) {
        if self.rendering_requested.sync_next_commit {
            self.rendering_requested.sync_next_commit = false;

            if !self.surfaces.saved {
                if !self.object.is_null() {
                    ffi::wl_resource_post_error(
                        self.object,
                        ffi::zcce_decoration_v1_error_ZCCE_DECORATION_V1_ERROR_NO_COMMIT,
                        b"no wl_surface.commit after sync_next_commit and before update_rendering_finish\0".as_ptr() as *const _,
                    );
                }
            }
        }

        self.surfaces.drop_saved();

        let server = (*self.window).server;
        let app_id = (*self.window).get_app_id_string().unwrap_or_default();
        let mut ignore_transparent = (*server).wm.layout.window_backdrop_blur_ignore_transparent;
        let is_status = (*self.window).tiling_mode == crate::tiling::TilingMode::Status ||
                        app_id.starts_with("cce-status");
        if is_status {
            ignore_transparent = (*server).wm.layout.status_backdrop_blur_ignore_transparent;
        }
        let is_cce_app = app_id.starts_with("cce-");
        let blur_enabled = self.rendering_requested.blur && ((*self.window).wm_requested.ssd || is_cce_app || is_status);
        ffi::river_scene_node_enable_blur(self.surfaces.tree as *mut ffi::wlr_scene_node, blur_enabled, (*server).wm.layout.scenefx_optimized_blur, ignore_transparent, 0, 0, 0, 0);

        let scale = (*self.window).scale;
        let scaled_x = (self.rendering_requested.offset_x as f64 * scale) as i32;
        let scaled_y = (self.rendering_requested.offset_y as f64 * scale) as i32;
        ffi::river_scene_node_set_position_if_changed(self.tree as *mut ffi::wlr_scene_node, scaled_x, scaled_y);

        struct ScaleData {
            scale: f64,
            ancestor: *mut ffi::wlr_scene_node,
        }

        unsafe extern "C" fn set_expose_scale_iterator(
            buffer: *mut ffi::wlr_scene_buffer,
            sx: i32,
            sy: i32,
            user_data: *mut std::ffi::c_void,
        ) {
            let data = &*(user_data as *const ScaleData);
            let node = buffer as *mut ffi::wlr_scene_node;

            let surface = ffi::river_scene_node_get_surface(node);
            if !surface.is_null() {
                let w = ffi::river_wlr_surface_get_width(surface);
                let h = ffi::river_wlr_surface_get_height(surface);
                if data.scale == 1.0 {
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                    ffi::river_scene_node_set_position_if_changed(node, 0, 0);
                } else {
                    let dest_w = (w as f64 * data.scale) as i32;
                    let dest_h = (h as f64 * data.scale) as i32;
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                    let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                    let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                    let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                    ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                }
            } else if data.scale == 1.0 {
                ffi::river_scene_buffer_set_dest_size_if_changed(buffer, 0, 0);
                ffi::river_scene_node_set_position_if_changed(node, sx, sy);
            } else {
                let w = ffi::river_scene_buffer_get_width(buffer);
                let h = ffi::river_scene_buffer_get_height(buffer);
                let dest_w = (w as f64 * data.scale) as i32;
                let dest_h = (h as f64 * data.scale) as i32;
                ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
            }
        }

        let scale_data = ScaleData { scale, ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.surfaces.tree as *mut ffi::wlr_scene_node,
            Some(set_expose_scale_iterator),
            &scale_data as *const ScaleData as *mut std::ffi::c_void,
        );

        if self.surfaces.saved {
            let scale_data_saved = ScaleData { scale, ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                Some(set_expose_scale_iterator),
                &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
            );
        }

        let children_head = ffi::river_scene_tree_get_children(self.surfaces.tree) as *mut WlList;
        if (*children_head).next != children_head {
            ffi::wlr_scene_subsurface_tree_set_clip(self.surfaces.tree as *mut ffi::wlr_scene_node, std::ptr::null());
        }
    }

    pub unsafe fn scale_only_render_finish(&mut self) {
        let scale = (*self.window).scale;
        if scale == 1.0 {
            return;
        }

        struct ScaleData {
            scale: f64,
            ancestor: *mut ffi::wlr_scene_node,
        }

        unsafe extern "C" fn set_expose_scale_iterator(
            buffer: *mut ffi::wlr_scene_buffer,
            sx: i32,
            sy: i32,
            user_data: *mut std::ffi::c_void,
        ) {
            let data = &*(user_data as *const ScaleData);
            let node = buffer as *mut ffi::wlr_scene_node;

            let surface = ffi::river_scene_node_get_surface(node);
            if !surface.is_null() {
                let w = ffi::river_wlr_surface_get_width(surface);
                let h = ffi::river_wlr_surface_get_height(surface);
                if data.scale == 1.0 {
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, w, h);
                    ffi::river_scene_node_set_position_if_changed(node, 0, 0);
                } else {
                    let dest_w = (w as f64 * data.scale) as i32;
                    let dest_h = (h as f64 * data.scale) as i32;
                    ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                    let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                    let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                    let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                    ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
                }
            } else if data.scale == 1.0 {
                ffi::river_scene_buffer_set_dest_size_if_changed(buffer, 0, 0);
                ffi::river_scene_node_set_position_if_changed(node, sx, sy);
            } else {
                let w = ffi::river_scene_buffer_get_width(buffer);
                let h = ffi::river_scene_buffer_get_height(buffer);
                let dest_w = (w as f64 * data.scale) as i32;
                let dest_h = (h as f64 * data.scale) as i32;
                ffi::river_scene_buffer_set_dest_size_if_changed(buffer, dest_w, dest_h);

                let (px, py) = get_parent_position_relative_to(node, data.ancestor);
                let dest_x = (px as f64 * (data.scale - 1.0)) as i32;
                let dest_y = (py as f64 * (data.scale - 1.0)) as i32;
                ffi::river_scene_node_set_position_if_changed(node, dest_x, dest_y);
            }
        }

        let scale_data = ScaleData { scale, ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.surfaces.tree as *mut ffi::wlr_scene_node,
            Some(set_expose_scale_iterator),
            &scale_data as *const ScaleData as *mut std::ffi::c_void,
        );

        if self.surfaces.saved {
            let scale_data_saved = ScaleData { scale, ancestor: self.surfaces.saved_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.saved_tree as *mut ffi::wlr_scene_node,
                Some(set_expose_scale_iterator),
                &scale_data_saved as *const ScaleData as *mut std::ffi::c_void,
            );
        }
    }
}

pub unsafe fn decoration_from_wlr_surface(surface: *mut ffi::wlr_surface) -> *mut Decoration {
    if surface.is_null() {
        return std::ptr::null_mut();
    }
    let role_ptr = ffi::river_wlr_surface_get_role(surface);
    if role_ptr != &raw const DECORATION_ROLE {
        return std::ptr::null_mut();
    }
    let resource = ffi::river_wlr_surface_get_role_resource(surface);
    if resource.is_null() {
        return std::ptr::null_mut();
    }
    ffi::wl_resource_get_user_data(resource) as *mut Decoration
}

unsafe extern "C" fn dec_client_commit(surface: *mut ffi::wlr_surface) {
    let dec = decoration_from_wlr_surface(surface);
    if dec.is_null() {
        return;
    }
    if (*dec).rendering_requested.sync_next_commit {
        (*dec).surfaces.save();
    }
}

unsafe extern "C" fn dec_commit(surface: *mut ffi::wlr_surface) {
    if ffi::wlr_surface_has_buffer(surface) {
        ffi::wlr_surface_map(surface);
    }
}

unsafe extern "C" fn dec_destroy(_client: *mut ffi::wl_client, resource: *mut ffi::wl_resource) {
    ffi::wl_resource_destroy(resource);
}

unsafe extern "C" fn dec_set_offset(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    x: i32,
    y: i32,
) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if dec.is_null() {
        return;
    }
    let server = (*(*dec).window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*dec).rendering_requested.offset_x = x;
    (*dec).rendering_requested.offset_y = y;
}

unsafe extern "C" fn dec_sync_next_commit(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if dec.is_null() {
        return;
    }
    let server = (*(*dec).window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*dec).rendering_requested.sync_next_commit = true;
}

unsafe extern "C" fn dec_set_blur(
    _client: *mut ffi::wl_client,
    resource: *mut ffi::wl_resource,
    blur: u32,
) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if dec.is_null() {
        return;
    }
    let server = (*(*dec).window).server;
    if !(*server).wm.ensure_rendering() {
        return;
    }
    (*dec).rendering_requested.blur = blur != 0;
}

static DECORATION_INTERFACE: ffi::zcce_decoration_v1_interface = ffi::zcce_decoration_v1_interface {
    destroy: Some(dec_destroy),
    set_offset: Some(dec_set_offset),
    sync_next_commit: Some(dec_sync_next_commit),
    set_blur: Some(dec_set_blur),
};

static INERT_DECORATION_INTERFACE: ffi::zcce_decoration_v1_interface = ffi::zcce_decoration_v1_interface {
    destroy: Some(dec_destroy),
    set_offset: None,
    sync_next_commit: None,
    set_blur: None,
};

unsafe extern "C" fn handle_dec_destroy_resource(resource: *mut ffi::wl_resource) {
    let dec = ffi::wl_resource_get_user_data(resource) as *mut Decoration;
    if !dec.is_null() {
        ffi::river_wlr_surface_set_role_object((*dec).surface, std::ptr::null_mut());
        (*dec).object = std::ptr::null_mut();
        (*dec).destroy();
    }
}

unsafe extern "C" fn dec_role_destroy(surface: *mut ffi::wlr_surface) {
    let dec = decoration_from_wlr_surface(surface);
    if dec.is_null() {
        return;
    }
    ffi::river_wlr_surface_set_role_object(surface, std::ptr::null_mut());
    if !(*dec).object.is_null() {
        ffi::wl_resource_set_user_data((*dec).object, std::ptr::null_mut());
        ffi::wl_resource_destroy((*dec).object);
        (*dec).object = std::ptr::null_mut();
    }
    (*dec).destroy();
}

#[no_mangle]
pub static mut DECORATION_ROLE: ffi::wlr_surface_role = ffi::wlr_surface_role {
    name: b"zcce_decoration_v1\0".as_ptr() as *const _,
    no_object: false,
    client_commit: Some(dec_client_commit),
    commit: Some(dec_commit),
    map: None,
    unmap: None,
    destroy: Some(dec_role_destroy),
};

unsafe fn get_parent_position_relative_to(
    node: *mut ffi::wlr_scene_node,
    ancestor: *mut ffi::wlr_scene_node,
) -> (i32, i32) {
    let mut x = 0;
    let mut y = 0;
    if !node.is_null() {
        let mut curr = ffi::river_scene_node_get_parent(node) as *mut ffi::wlr_scene_node;
        while !curr.is_null() && curr != ancestor {
            x += ffi::river_scene_node_get_x(curr);
            y += ffi::river_scene_node_get_y(curr);
            curr = ffi::river_scene_node_get_parent(curr) as *mut ffi::wlr_scene_node;
        }
    }
    (x, y)
}

unsafe fn wl_listener_remove_safe(listener: *mut ffi::wl_listener) {
    let prev = (*listener).link.prev;
    let next = (*listener).link.next;
    if !prev.is_null() && !next.is_null() && prev != listener as *mut ffi::wl_list && next != listener as *mut ffi::wl_list {
        ffi::wl_list_remove(&mut (*listener).link);
        (*listener).link.prev = std::ptr::null_mut();
        (*listener).link.next = std::ptr::null_mut();
    }
}

unsafe extern "C" fn handle_window_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let window = crate::container_of!(listener, Window, commit);
    let was_status = (*window).is_status_bar();
    (*window).render_finish();
    if was_status {
        (*(*window).server).wm.dirty_windowing();
    }
}
