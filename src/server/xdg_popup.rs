// SPDX-FileCopyrightText: © 2023 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::scene_node_data::SceneNodeDataVal;

pub struct XdgPopup {
    /// Private, as every `Wlr` field reachable through `&mut` is; let go
    /// by the destroy handler.
    wlr_popup: crate::wlr::Wlr<ffi::wlr_xdg_popup>,
    /// Watched, as is `capture_tree`: wlroots destroys the xdg-surface trees
    /// with the popup.
    pub tree: crate::scene_handle::SceneTree,
    pub capture_tree: crate::scene_handle::SceneTree,
    /// The scene tree of the popup's ROOT — the window's or layer
    /// surface's popup tree, which sits at that surface's origin — shared
    /// by every submenu under it. `handle_reposition` measures the screen
    /// from here, because wlroots wants the unconstrain box in the root
    /// toplevel surface's coordinates, not the immediate parent's.
    /// Watched: the root window's (or layer surface's) tree, not ours.
    pub root_tree: crate::scene_handle::SceneTree,
    /// Whether the root is a window, found once at creation through the
    /// root's scene node — a destroyed popup's own tree may already be gone
    /// when its destroy signal runs, so nothing can be looked up then. Only
    /// a window's popup asks for a pointer refresh.
    pub under_window: bool,
    /// Whether the root is a window or a layer surface, found the same way:
    /// only such a popup is positioned on screen and blurred. (Until
    /// 2026-10-12 a layer surface's popup was neither, so a bar's tray menu
    /// never got its first configure.)
    pub root_is_shell: bool,
    /// Where the popup stood and how big it was at its last commit, layout
    /// coordinates: to tell a map, a move or a resize.
    pub last_box: (i32, i32, i32, i32),

    pub destroy: crate::listener::Listener,
    pub commit: crate::listener::Listener,
    pub new_popup: crate::listener::Listener,
    pub reposition: crate::listener::Listener,
}

impl XdgPopup {
    /// `root` is the tree the popup's root surface's popups live in: for a
    /// top-level menu the same tree as `parent`, for a submenu the one its
    /// parent popup carries.
    pub unsafe fn create(
        wlr_popup: *mut ffi::wlr_xdg_popup,
        parent: *mut ffi::wlr_scene_tree,
        capture_parent: *mut ffi::wlr_scene_tree,
        root: *mut ffi::wlr_scene_tree,
    ) -> Result<*mut Self, &'static str> {
        let base_surface = ffi::river_wlr_xdg_popup_get_base(wlr_popup);
        let tree = ffi::wlr_scene_xdg_surface_create(parent, base_surface);
        if tree.is_null() {
            return Err("wlr_scene_xdg_surface_create failed");
        }

        let mut capture_tree = std::ptr::null_mut();
        if !capture_parent.is_null() {
            capture_tree = ffi::wlr_scene_xdg_surface_create(capture_parent, base_surface);
            if capture_tree.is_null() {
                ffi::wlr_scene_node_destroy(tree as *mut ffi::wlr_scene_node);
                return Err("wlr_scene_xdg_surface_create for capture parent failed");
            }
        }

        let root_tree = crate::scene_handle::SceneTree::watch(root);
        let root_kind = root_tree.node_data();
        let popup = Box::into_raw(Box::new(XdgPopup {
            wlr_popup: crate::wlr::Wlr::from_raw(wlr_popup),
            tree: crate::scene_handle::SceneTree::watch(tree),
            capture_tree: crate::scene_handle::SceneTree::watch(capture_tree),
            root_tree,
            under_window: matches!(root_kind, Some(SceneNodeDataVal::Window(_))),
            root_is_shell: matches!(root_kind, Some(SceneNodeDataVal::Window(_) | SceneNodeDataVal::LayerSurface(_))),
            last_box: (0, 0, 0, 0),
            destroy: std::mem::zeroed(),
            commit: std::mem::zeroed(),
            new_popup: std::mem::zeroed(),
            reposition: std::mem::zeroed(),
        }));

        (*popup).destroy.connect(ffi::river_wlr_xdg_popup_get_destroy_signal(wlr_popup), handle_destroy);

        let wlr_surface = ffi::river_wlr_xdg_surface_get_surface(base_surface);
        (*popup).commit.connect(ffi::river_wlr_surface_get_commit_signal(wlr_surface), handle_commit);

        (*popup).new_popup.connect(ffi::river_wlr_xdg_surface_get_new_popup_signal(base_surface), handle_new_popup);

        (*popup).reposition.connect(ffi::river_wlr_xdg_popup_get_reposition_signal(wlr_popup), handle_reposition);

        Ok(popup)
    }
}

/// A popup coming, going, moving or changing size under a pointer that is
/// standing still changes what is under it, and nothing else would say so:
/// wlroots moves pointer focus on MOTION. So a menu closed under the pointer
/// left the window beneath it without focus until the pointer moved — the
/// designer's network menu turning into its Add Node list, a swipe back on
/// the list going nowhere — and a menu opened under it took no focus either.
/// The same deferred re-evaluation a toplevel's commit asks for when its
/// mapping moves (`InputManager::schedule_pointer_refresh`), coalesced and
/// run once the scene has caught up.
fn refresh_pointer_under(popup: &XdgPopup) {
    let server = crate::shared::server_ptr();
    if popup.under_window && !server.is_null() {
        // SAFETY: the shared server is live while the compositor runs, and
        // scheduling borrows the input manager only to arm a deferred call.
        unsafe { (*server).input_manager.schedule_pointer_refresh() };
    }
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let popup = crate::container_of!(listener, XdgPopup, destroy);
    refresh_pointer_under(&*popup);

    (*popup).destroy.disconnect();
    (*popup).commit.disconnect();
    (*popup).new_popup.disconnect();
    (*popup).reposition.disconnect();

    (*popup).wlr_popup.take();
    let _ = Box::from_raw(popup);
}

unsafe extern "C" fn handle_commit(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let popup = crate::container_of!(listener, XdgPopup, commit);
    if (*popup).wlr_popup.initial_commit() {
        handle_reposition((*popup).reposition.as_ptr(), std::ptr::null_mut());
        return;
    }
    update_blur(&*popup);
    // Mapped (its first buffer), moved or resized: what is under the
    // pointer changed. Not on every commit — a refresh sends the client a
    // motion, and an animating menu commits every frame.
    let (lx, ly) = (*popup).tree.coords().map_or((0, 0), |(x, y, _)| (x, y));
    let (w, h) = (*popup).wlr_popup.surface_size();
    let at = (lx, ly, w, h);
    if at != (*popup).last_box {
        (*popup).last_box = at;
        refresh_pointer_under(&*popup);
    }
}

/// Blur behind the popup as behind a window: a translucent menu is frosted
/// glass, and with no blur node it is a clear pane over whatever it opened
/// above. cce-ui paints its context-menu popup as the surface's ROOT plate
/// for exactly this — translucent, with the frost left to the compositor,
/// since the client has no backdrop to frost from inside its own popup.
///
/// Masked by the surface's own alpha (`ignore_transparent`), so the menu's
/// rounded corners and its shadow margin stay clear; never the optimized
/// (cached) blur, which re-bakes on every change beneath a surface stacked
/// above windows — see `handle_layer_surface_commit`. Sized from the
/// surface, so it runs every commit: a popup is resized by its configure.
fn update_blur(popup: &XdgPopup) {
    if !popup.root_is_shell || popup.wlr_popup.surface().is_null() {
        return;
    }
    let (w, h) = popup.wlr_popup.surface_size();
    let layout = crate::shared::layout();
    popup.tree.enable_blur(layout.window_blur, false, layout.window_backdrop_blur_ignore_transparent, 0, 0, w, h, 0);
}

unsafe extern "C" fn handle_new_popup(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let popup = crate::container_of!(listener, XdgPopup, new_popup);
    let wlr_xdg_popup = data as *mut ffi::wlr_xdg_popup;

    if let Err(e) = XdgPopup::create(wlr_xdg_popup, (*popup).tree.raw(), (*popup).capture_tree.raw(), (*popup).root_tree.raw()) {
        log::error!("Failed to create nested popup: {}", e);
        ffi::wl_resource_post_no_memory((*wlr_xdg_popup).resource);
    }
}

unsafe extern "C" fn handle_reposition(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let popup = crate::container_of!(listener, XdgPopup, reposition);
    let server = crate::shared::server_ptr();
    if !(*popup).root_is_shell || server.is_null() {
        return;
    }

    let mut parent_lx: i32 = 0;
    let mut parent_ly: i32 = 0;
    let parent_tree = ffi::river_wlr_scene_tree_get_parent((*popup).tree.raw());
    if parent_tree.is_null() {
        return;
    }

    ffi::wlr_scene_node_coords(parent_tree as *mut ffi::wlr_scene_node, &mut parent_lx, &mut parent_ly);

    let mut anchor = (*popup).wlr_popup.anchor_rect();
    anchor.x += parent_lx;
    anchor.y += parent_ly;

    let Some(output_box) = (*crate::reentry::wm(server))
        .outputs
        .max_overlap_output(&anchor)
        .map(|output| output.wlr_output().layout_box())
    else {
        return;
    };

    // The box goes to wlroots in the ROOT surface's coordinates. For a
    // top-level menu the parent is the root; for a submenu it is the
    // menu, and measuring from the menu's corner (as this did until
    // 2026-09-25) shifted the screen up and left by the menu's offset in
    // the window, so a tall submenu slid past the real top edge and the
    // flip decisions were made against the wrong right edge.
    let (root_lx, root_ly) = (*popup).root_tree.coords().map_or((0, 0), |(x, y, _)| (x, y));

    let mut constraint = output_box;
    constraint.x -= root_lx;
    constraint.y -= root_ly;

    (*popup).wlr_popup.unconstrain_and_configure(&constraint);
}
