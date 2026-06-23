// SPDX-FileCopyrightText: © 2020 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlList, wl_list_insert, wl_list_remove, WlListener, wl_signal_add};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Border {
    pub edges: Edges,
    pub width: u32,
    pub r: u32,
    pub g: u32,
    pub b: u32,
    pub a: u32,
}

pub struct BorderRects {
    pub left: *mut ffi::wlr_scene_rect,
    pub right: *mut ffi::wlr_scene_rect,
    pub top: *mut ffi::wlr_scene_rect,
    pub bottom: *mut ffi::wlr_scene_rect,
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
    pub decorations_above: ffi::wl_list,
    pub decorations_above_tree: *mut ffi::wlr_scene_tree,
    pub popup_tree: *mut ffi::wlr_scene_tree,
    pub capture_scene: *mut ffi::wlr_scene,
    pub capture_source: *mut ffi::wlr_ext_image_capture_source_v1,
    pub tags: u32,
    pub tiling_mode: crate::tiling::TilingMode,
    pub mode_locked: bool,
    pub is_new: bool,
    pub restored: bool,
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
    pub virtual_x: f64,
    pub virtual_y: f64,
    pub commit: ffi::wl_listener,

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
}

impl Window {
    pub unsafe fn is_wine(&self) -> bool {
        match self.impl_type {
            WindowImpl::Xwayland(xwindow) => {
                if xwindow.is_null() {
                    return false;
                }
                let class_ptr = (*(*xwindow).xsurface).class;
                let class = if class_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(class_ptr).to_str().unwrap_or("") };
                let title_ptr = (*(*xwindow).xsurface).title;
                let title = if title_ptr.is_null() { "" } else { std::ffi::CStr::from_ptr(title_ptr).to_str().unwrap_or("") };
                
                let class_lower = class.to_lowercase();
                let title_lower = title.to_lowercase();
                class_lower.contains("steam_proton")
                    || class_lower.contains("steam_app")
                    || class_lower.contains("wine")
                    || class_lower.contains("upc.exe")
                    || class_lower.contains("trackmania")
                    || title_lower.contains("ubisoft")
                    || title_lower.contains("trackmania")
            }
            _ => false,
        }
    }

    pub unsafe fn is_fullscreen(&self) -> bool {
        self.tiling_mode == crate::tiling::TilingMode::Fullscreen
            || !self.wm_requested.fullscreen.is_null()
    }

    pub unsafe fn is_status_bar(&self) -> bool {
        self.get_app_id_string().as_deref() == Some("cce-status-interface")
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
            },
            decorations_above: std::mem::zeroed(),
            decorations_above_tree,
            popup_tree,
            capture_scene,
            capture_source: std::ptr::null_mut(),
            tags: 1,
            tiling_mode: crate::tiling::TilingMode::Floating,
            mode_locked: false,
            is_new: true,
            restored: false,
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
            virtual_x: unsafe { (*server).wm.desk_pan_x + 100.0 },
            virtual_y: unsafe { (*server).wm.desk_pan_y + 100.0 },
            commit: std::mem::zeroed(),
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
                border: Border { edges: Edges::new(), width: 0, r: 0, g: 0, b: 0, a: 0 },
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
        if app_id_str.is_empty() || app_id_str == "cce-status-interface" {
            return;
        }
        let title_str = self.get_title_string().unwrap_or_default();
        if let Some(saved) = (*self.server).wm.match_and_remove_restore_state(&app_id_str, &title_str) {
            log::info!("Restoring saved state for window: app_id={}, title={}. Position: ({}, {}), Size: {}x{}", app_id_str, title_str, saved.virtual_x, saved.virtual_y, saved.width, saved.height);
            self.tiling_mode = saved.tiling_mode;
            self.tags = saved.tags;
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
            
            self.restored = true;
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
        let is_status_bar = if !app_id_ptr.is_null() {
            let app_id = std::ffi::CStr::from_ptr(app_id_ptr).to_string_lossy();
            app_id == "cce-status-interface"
        } else {
            false
        };

        if !is_status_bar {
            let seats = &mut (*self.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
            let mut curr = (*seats).next;
            while curr != seats {
                let next = (*curr).next;
                let seat = crate::container_of!(curr, crate::seat::Seat, link);
                (*seat).focus(crate::seat::Focus::Window(self as *mut Window));
                curr = next;
            }
        }

        (*self.server).wm.dirty_windowing();
        Ok(())
    }

    pub unsafe fn unmap(&mut self) {
        log::debug!("window '{:?}' unmapped", self.get_title());
        wl_listener_remove_safe(&mut self.commit);
        self.surfaces.save();
        assert!(!matches!(self.impl_type, WindowImpl::Destroying));
        assert_eq!(self.state, WindowState::Mapped);
        self.state = WindowState::Closing;
        (*self.server).wm.dirty_windowing();

        if !self.foreign_toplevel_handle.is_null() {
            ffi::wlr_ext_foreign_toplevel_handle_v1_destroy(self.foreign_toplevel_handle);
            self.foreign_toplevel_handle = std::ptr::null_mut();
        }
        if !self.wlr_toplevel_handle.is_null() {
            ffi::wlr_foreign_toplevel_handle_v1_destroy(self.wlr_toplevel_handle);
            self.wlr_toplevel_handle = std::ptr::null_mut();
        }

        if (*self.server).wm.expose_hovered_window == self as *mut Window {
            (*self.server).wm.expose_hovered_window = std::ptr::null_mut();
        }
        if (*self.server).wm.expose_initial_focus == self as *mut Window {
            (*self.server).wm.expose_initial_focus = std::ptr::null_mut();
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
                }
            }
            curr = next;
        }

        if (*(*window).server).wm.expose_hovered_window == window {
            (*(*window).server).wm.expose_hovered_window = std::ptr::null_mut();
        }
        if (*(*window).server).wm.expose_initial_focus == window {
            (*(*window).server).wm.expose_initial_focus = std::ptr::null_mut();
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

        (*(*window).server).wm.windows.remove((*window).ref_key);

        let _ = Box::from_raw(window);
    }

    pub unsafe fn set_dimensions_hint(&mut self, hint: DimensionsHint) {
        self.wm_scheduled.dimensions_hint = hint;
        if self.wm_sent.dimensions_hint != hint {
            if matches!(self.tiling_mode, crate::tiling::TilingMode::Floating | crate::tiling::TilingMode::Popup | crate::tiling::TilingMode::Status) {
                (*self.server).wm.dirty_windowing();
            } else {
                self.wm_sent.dimensions_hint = hint;
            }
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
                    border: Border { edges: Edges::new(), width: 0, r: 0, g: 0, b: 0, a: 0 },
                    clip: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
                    content_clip: ffi::wlr_box { x: 0, y: 0, width: 0, height: 0 },
                    opacity: 1.0f32,
                    circular: false,
                    blur: false,
                };

                wl_list_remove(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
                self.node.link.prev = &mut self.node.link;
                self.node.link.next = &mut self.node.link;

                self.make_inert();
            }
            WindowState::Ready | WindowState::Initialized | WindowState::Mapped => {
                let wm_v1 = (*self.server).wm.object;
                if wm_v1.is_null() {
                    let is_linked = self.node.link.prev as *const _ != &self.node.link as *const _;
                    if !is_linked {
                        wl_list_remove(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
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

                    wl_list_remove(&mut self.node.link as *mut ffi::wl_list as *mut WlList);
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

                    res
                } else {
                    self.object
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

        let is_maximized_layout = self.tiling_mode == crate::tiling::TilingMode::Cascade || self.tiling_mode == crate::tiling::TilingMode::Grid;
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
            ffi::river_scene_node_enable_blur(self.surfaces.tree as *mut ffi::wlr_scene_node, requested.blur);
            ffi::river_scene_node_set_opacity(self.tree as *mut ffi::wlr_scene_node, requested.opacity);

            let radius = if requested.circular {
                let w = self.rendering_sent.width as i32;
                let h = self.rendering_sent.height as i32;
                w.min(h) / 2
            } else {
                0
            };

            unsafe extern "C" fn set_corner_radius_iterator(
                buffer: *mut ffi::wlr_scene_buffer,
                _sx: i32,
                _sy: i32,
                user_data: *mut std::ffi::c_void,
            ) {
                let radius = *(user_data as *const i32);
                ffi::wlr_scene_buffer_set_corner_radius(
                    buffer,
                    radius,
                    ffi::corner_location_CORNER_LOCATION_ALL,
                );
            }

            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.tree as *mut ffi::wlr_scene_node,
                Some(set_corner_radius_iterator),
                &radius as *const i32 as *mut std::ffi::c_void,
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
                }
            }

            let scale_data_surfaces = ScaleData { scale: self.scale, ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.surfaces.tree as *mut ffi::wlr_scene_node,
                Some(set_expose_scale_iterator),
                &scale_data_surfaces as *const ScaleData as *mut std::ffi::c_void,
            );
            
            let scale_data_popup = ScaleData { scale: self.scale, ancestor: self.popup_tree as *mut ffi::wlr_scene_node };
            ffi::wlr_scene_node_for_each_buffer(
                self.popup_tree as *mut ffi::wlr_scene_node,
                Some(set_expose_scale_iterator),
                &scale_data_popup as *const ScaleData as *mut std::ffi::c_void,
            );
        }

        self.box_geom.width = self.rendering_sent.width as i32;
        self.box_geom.height = self.rendering_sent.height as i32;

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
            let is_status_bar = if !app_id_ptr.is_null() {
                let app_id = std::ffi::CStr::from_ptr(app_id_ptr).to_string_lossy();
                app_id == "cce-status-interface"
            } else {
                false
            };

            ffi::wlr_scene_node_set_enabled(self.fullscreen_background as *mut ffi::wlr_scene_node, !is_status_bar);
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

    pub unsafe fn draw_borders(&mut self) {
        let requested = &self.rendering_requested;

        let border = &requested.border;
        let bg_color: [f32; 4] = [
            (border.r as f64 / u32::MAX as f64) as f32,
            (border.g as f64 / u32::MAX as f64) as f32,
            (border.b as f64 / u32::MAX as f64) as f32,
            (border.a as f64 / u32::MAX as f64) as f32,
        ];
        ffi::river_scene_node_set_position_if_changed(self.window_background as *mut ffi::wlr_scene_node, 0, 0);
        let bg_width = (self.box_geom.width as f64 * self.scale) as i32;
        let bg_height = (self.box_geom.height as f64 * self.scale) as i32;
        ffi::river_scene_rect_set_size_if_changed(self.window_background, bg_width, bg_height);
        ffi::wlr_scene_rect_set_color(self.window_background, bg_color.as_ptr());
        ffi::wlr_scene_node_set_enabled(self.window_background as *mut ffi::wlr_scene_node, !requested.hidden && self.wm_requested.ssd);

        if requested.circular || requested.border.width == 0 || !self.wm_requested.ssd {
            ffi::wlr_scene_node_set_enabled(self.border.left as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.right as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.top as *mut ffi::wlr_scene_node, false);
            ffi::wlr_scene_node_set_enabled(self.border.bottom as *mut ffi::wlr_scene_node, false);
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
            let color: [f32; 4] = [
                (border.r as f64 / u32::MAX as f64) as f32,
                (border.g as f64 / u32::MAX as f64) as f32,
                (border.b as f64 / u32::MAX as f64) as f32,
                (border.a as f64 / u32::MAX as f64) as f32,
            ];

            let mut left = ffi::wlr_box {
                x: -(border.width as i32),
                y: 0,
                width: border.width as i32,
                height: content.height,
            };
            let mut right = ffi::wlr_box {
                x: content.width,
                y: 0,
                width: border.width as i32,
                height: content.height,
            };
            let mut top = ffi::wlr_box {
                x: 0,
                y: -(border.width as i32),
                width: content.width,
                height: border.width as i32,
            };
            let mut bottom = ffi::wlr_box {
                x: 0,
                y: content.height,
                width: content.width,
                height: border.width as i32,
            };

            if border.edges.top {
                left.y -= border.width as i32;
                left.height += border.width as i32;
                right.y -= border.width as i32;
                right.height += border.width as i32;
            }
            if border.edges.bottom {
                left.height += border.width as i32;
                right.height += border.width as i32;
            }

            let mut edges = [
                ("left", &mut left, self.border.left, border.edges.left),
                ("right", &mut right, self.border.right, border.edges.right),
                ("top", &mut top, self.border.top, border.edges.top),
                ("bottom", &mut bottom, self.border.bottom, border.edges.bottom),
            ];

            for (_, edge_box, rect, enabled) in &mut edges {
                if requested.clip.width != 0 || requested.clip.height != 0 {
                    let mut clip_intersect = std::mem::zeroed();
                    ffi::wlr_box_intersection(&mut clip_intersect, *edge_box, &requested.clip);
                    **edge_box = clip_intersect;
                }
                ffi::wlr_scene_node_set_enabled(*rect as *mut ffi::wlr_scene_node, *enabled);
                let scaled_x = ((*edge_box).x as f64 * self.scale) as i32;
                let scaled_y = ((*edge_box).y as f64 * self.scale) as i32;
                let scaled_w = ((*edge_box).width as f64 * self.scale) as i32;
                let scaled_h = ((*edge_box).height as f64 * self.scale) as i32;
                ffi::river_scene_node_set_position_if_changed(*rect as *mut ffi::wlr_scene_node, scaled_x, scaled_y);
                ffi::river_scene_rect_set_size_if_changed(*rect, scaled_w, scaled_h);
                ffi::wlr_scene_rect_set_color(*rect, color.as_ptr());
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
    (*window).rendering_requested.border = Border {
        edges: Edges::from_u32(edges),
        width: width as u32,
        r,
        g,
        b,
        a,
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

        ffi::river_scene_node_enable_blur(self.surfaces.tree as *mut ffi::wlr_scene_node, self.rendering_requested.blur);

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
            }
        }

        let scale_data = ScaleData { scale, ancestor: self.surfaces.tree as *mut ffi::wlr_scene_node };
        ffi::wlr_scene_node_for_each_buffer(
            self.surfaces.tree as *mut ffi::wlr_scene_node,
            Some(set_expose_scale_iterator),
            &scale_data as *const ScaleData as *mut std::ffi::c_void,
        );

        let children_head = ffi::river_scene_tree_get_children(self.surfaces.tree) as *mut WlList;
        if (*children_head).next != children_head {
            ffi::wlr_scene_subsurface_tree_set_clip(self.surfaces.tree as *mut ffi::wlr_scene_node, std::ptr::null());
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
    (*window).render_finish();
}
