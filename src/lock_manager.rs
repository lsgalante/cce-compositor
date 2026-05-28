// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, wl_signal_add, wl_listener_remove, WlList, wl_list_insert, wl_list_remove};
use crate::scene_node_data::{SceneNodeData, SceneNodeDataVal};
use crate::seat::Focus;

pub struct LockManager {
    pub wlr_manager: *mut ffi::wlr_session_lock_manager_v1,
    pub state: LockState,
    pub lock: *mut ffi::wlr_session_lock_v1,
    pub lock_surfaces_timer: *mut ffi::wl_event_source,
    pub server: *mut Server,

    pub new_lock: ffi::wl_listener,
    pub unlock: ffi::wl_listener,
    pub destroy: ffi::wl_listener,
    pub new_surface: ffi::wl_listener,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LockState {
    Unlocked,
    Locked,
    WaitingForBlank,
    WaitingForLockSurfaces,
}

impl Default for LockManager {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl LockManager {
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        self.state = LockState::Unlocked;

        let wlr_manager = ffi::wlr_session_lock_manager_v1_create((*server).wl_server);
        if wlr_manager.is_null() {
            return Err("Failed to create wlr_session_lock_manager_v1");
        }
        self.wlr_manager = wlr_manager;

        let event_loop = ffi::wl_display_get_event_loop((*server).wl_server);
        let timer = ffi::wl_event_loop_add_timer(
            event_loop,
            Some(handle_lock_surfaces_timeout),
            self as *mut LockManager as *mut _,
        );
        if timer.is_null() {
            return Err("Failed to create lock surfaces timer");
        }
        self.lock_surfaces_timer = timer;

        let new_lock_ptr = &mut self.new_lock as *mut ffi::wl_listener as *mut WlListener;
        (*new_lock_ptr).notify = Some(handle_new_lock);
        wl_signal_add(&mut (*self.wlr_manager).events.new_lock, &mut self.new_lock);

        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        if !self.lock_surfaces_timer.is_null() {
            ffi::wl_event_source_remove(self.lock_surfaces_timer);
            self.lock_surfaces_timer = std::ptr::null_mut();
        }
        wl_listener_remove(&mut self.new_lock);
    }

    pub unsafe fn lock_surface_from_output(
        &self,
        output: *mut crate::output::Output,
    ) -> Option<*mut LockSurface> {
        if self.lock.is_null() {
            return None;
        }

        let surfaces_head = &mut (*self.lock).surfaces as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*surfaces_head).next;
        while curr != surfaces_head {
            let next = (*curr).next;
            let wlr_lock_surface = crate::container_of!(curr, ffi::wlr_session_lock_surface_v1, link);
            let lock_surface = (*wlr_lock_surface).data as *mut LockSurface;
            if !lock_surface.is_null() && (*lock_surface).get_output() == output {
                return Some(lock_surface);
            }
            curr = next;
        }

        None
    }

    pub unsafe fn maybe_lock(&mut self) {
        let mut all_outputs_blanked = true;
        let mut all_outputs_rendered_lock_surface = true;

        let outputs_head = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*outputs_head).next;
        while curr != outputs_head {
            let next = (*curr).next;
            let output = crate::container_of!(curr, crate::output::Output, link);
            let wlr_output = (*output).wlr_output;
            if !wlr_output.is_null() && ffi::river_wlr_output_get_enabled(wlr_output) {
                match (*output).lock_render_state {
                    crate::output::LockRenderState::PendingUnlock
                    | crate::output::LockRenderState::Unlocked
                    | crate::output::LockRenderState::PendingBlank
                    | crate::output::LockRenderState::PendingLockSurface => {
                        all_outputs_blanked = false;
                        all_outputs_rendered_lock_surface = false;
                    }
                    crate::output::LockRenderState::Blanked => {
                        all_outputs_rendered_lock_surface = false;
                    }
                    crate::output::LockRenderState::LockSurface => {}
                }
            }
            curr = next;
        }

        match self.state {
            LockState::WaitingForLockSurfaces => {
                if all_outputs_rendered_lock_surface {
                    self.send_locked();
                    ffi::wlr_scene_node_set_enabled((*self.server).scene.normal_tree as *mut ffi::wlr_scene_node, false);
                    ffi::wl_event_source_timer_update(self.lock_surfaces_timer, 0);
                }
            }
            LockState::WaitingForBlank => {
                if all_outputs_blanked {
                    self.send_locked();
                }
            }
            _ => {}
        }
    }

    pub unsafe fn send_locked(&mut self) {
        log::info!("session locked");
        if !self.lock.is_null() {
            ffi::wlr_session_lock_v1_send_locked(self.lock);
        }
        self.state = LockState::Locked;
        (*self.server).wm.dirty_windowing();
    }
}

pub struct LockSurface {
    pub tree: *mut ffi::wlr_scene_tree,
    pub wlr_lock_surface: *mut ffi::wlr_session_lock_surface_v1,
    pub lock: *mut ffi::wlr_session_lock_v1,
    pub manager: *mut LockManager,

    pub idle_update_focus: *mut ffi::wl_event_source,

    pub map: ffi::wl_listener,
    pub surface_destroy: ffi::wl_listener,
}

impl LockSurface {
    pub unsafe fn create(
        wlr_lock_surface: *mut ffi::wlr_session_lock_surface_v1,
        lock: *mut ffi::wlr_session_lock_v1,
        manager: *mut LockManager,
    ) -> Result<*mut Self, &'static str> {
        let tree = ffi::wlr_scene_subsurface_tree_create(
            (*(*manager).server).scene.locked_tree,
            (*wlr_lock_surface).surface,
        );
        if tree.is_null() {
            return Err("Failed to create subsurface tree for lock surface");
        }

        let lock_surface = Box::into_raw(Box::new(Self {
            tree,
            wlr_lock_surface,
            lock,
            manager,
            idle_update_focus: std::ptr::null_mut(),
            map: std::mem::zeroed(),
            surface_destroy: std::mem::zeroed(),
        }));

        (*wlr_lock_surface).data = lock_surface as *mut _;

        SceneNodeData::attach(tree as *mut ffi::wlr_scene_node, SceneNodeDataVal::LockSurface(lock_surface));
        ffi::river_wlr_surface_set_data((*wlr_lock_surface).surface, tree as *mut ffi::wlr_scene_node as *mut _);

        let map_ptr = &mut (*lock_surface).map as *mut ffi::wl_listener as *mut WlListener;
        (*map_ptr).notify = Some(handle_lock_surface_map);
        wl_signal_add(
            ffi::river_wlr_surface_get_map_signal((*wlr_lock_surface).surface),
            &mut (*lock_surface).map,
        );

        let destroy_ptr = &mut (*lock_surface).surface_destroy as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_ptr).notify = Some(handle_lock_surface_destroy);
        wl_signal_add(
            &mut (*wlr_lock_surface).events.destroy,
            &mut (*lock_surface).surface_destroy,
        );

        (*lock_surface).configure();

        Ok(lock_surface)
    }

    pub unsafe fn destroy(lock_surface: *mut Self) {
        let mut new_focus = Focus::None;
        let surfaces_head = &mut (*(*lock_surface).lock).surfaces as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*surfaces_head).next;
        while curr != surfaces_head {
            let next = (*curr).next;
            let wlr_lock_surface = crate::container_of!(curr, ffi::wlr_session_lock_surface_v1, link);
            if wlr_lock_surface != (*lock_surface).wlr_lock_surface {
                let other_surf = (*wlr_lock_surface).data as *mut LockSurface;
                if !other_surf.is_null() {
                    new_focus = Focus::LockSurface(other_surf);
                    break;
                }
            }
            curr = next;
        }

        let server = (*(*lock_surface).manager).server;
        let seats_head = &mut (*server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats_head).next;
        while curr != seats_head {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            if let Focus::LockSurface(focused_surf) = (*seat).focused {
                if focused_surf == lock_surface {
                    (*seat).focus(new_focus);
                }
            }
            (*seat).cursor.update_state();
            curr = next;
        }

        if !(*lock_surface).idle_update_focus.is_null() {
            ffi::wl_event_source_remove((*lock_surface).idle_update_focus);
        }

        wl_listener_remove(&mut (*lock_surface).map);
        wl_listener_remove(&mut (*lock_surface).surface_destroy);

        ffi::river_wlr_surface_set_data((*(*lock_surface).wlr_lock_surface).surface, std::ptr::null_mut());

        let _ = Box::from_raw(lock_surface);
    }

    pub unsafe fn get_output(&self) -> *mut crate::output::Output {
        ffi::river_wlr_output_get_data((*self.wlr_lock_surface).output) as *mut crate::output::Output
    }

    pub unsafe fn configure(&self) {
        let mut width: i32 = 0;
        let mut height: i32 = 0;
        ffi::wlr_output_effective_resolution((*self.wlr_lock_surface).output, &mut width, &mut height);
        ffi::wlr_session_lock_surface_v1_configure(self.wlr_lock_surface, width as u32, height as u32);
    }
}

unsafe extern "C" fn handle_lock_surfaces_timeout(data: *mut std::ffi::c_void) -> std::os::raw::c_int {
    let manager = &mut *(data as *mut LockManager);
    log::error!("waiting for lock surfaces timed out, imperfect frames may be shown");

    assert!(manager.state == LockState::WaitingForLockSurfaces);
    manager.state = LockState::WaitingForBlank;

    ffi::wlr_scene_node_set_enabled((*manager.server).scene.normal_tree as *mut ffi::wlr_scene_node, false);

    manager.maybe_lock();

    0
}

unsafe extern "C" fn handle_new_lock(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let manager = &mut *crate::container_of!(listener, LockManager, new_lock);
    let lock = data as *mut ffi::wlr_session_lock_v1;

    log::debug!("session lock client made lock request");

    if !manager.lock.is_null() {
        log::info!("denying new session lock client, an active one already exists");
        ffi::wlr_session_lock_v1_destroy(lock);
        return;
    }

    manager.lock = lock;

    if manager.state == LockState::Unlocked {
        manager.state = LockState::WaitingForLockSurfaces;

        ffi::wlr_scene_node_set_enabled((*manager.server).scene.locked_tree as *mut ffi::wlr_scene_node, true);

        ffi::wl_event_source_timer_update(manager.lock_surfaces_timer, 200);

        let seats_head = &mut (*manager.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
        let mut curr = (*seats_head).next;
        while curr != seats_head {
            let next = (*curr).next;
            let seat = crate::container_of!(curr, crate::seat::Seat, link);
            (*seat).focus(Focus::None);
            curr = next;
        }
    } else {
        if manager.state == LockState::Locked {
            ffi::wlr_session_lock_v1_send_locked(lock);
        }
        log::info!("new session lock client given control of already locked session");
    }

    let unlock_ptr = &mut manager.unlock as *mut ffi::wl_listener as *mut WlListener;
    (*unlock_ptr).notify = Some(handle_unlock);
    wl_signal_add(&mut (*lock).events.unlock, &mut manager.unlock);

    let destroy_ptr = &mut manager.destroy as *mut ffi::wl_listener as *mut WlListener;
    (*destroy_ptr).notify = Some(handle_destroy);
    wl_signal_add(&mut (*lock).events.destroy, &mut manager.destroy);

    let new_surface_ptr = &mut manager.new_surface as *mut ffi::wl_listener as *mut WlListener;
    (*new_surface_ptr).notify = Some(handle_surface);
    wl_signal_add(&mut (*lock).events.new_surface, &mut manager.new_surface);
}

unsafe extern "C" fn handle_unlock(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let manager = &mut *crate::container_of!(listener, LockManager, unlock);

    manager.state = LockState::Unlocked;
    log::info!("session unlocked");

    ffi::wlr_scene_node_set_enabled((*manager.server).scene.normal_tree as *mut ffi::wlr_scene_node, true);
    ffi::wlr_scene_node_set_enabled((*manager.server).scene.locked_tree as *mut ffi::wlr_scene_node, false);

    let seats_head = &mut (*manager.server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*seats_head).next;
    while curr != seats_head {
        let next = (*curr).next;
        let seat = crate::container_of!(curr, crate::seat::Seat, link);
        (*seat).focus(Focus::None);
        curr = next;
    }

    handle_destroy(&mut manager.destroy, std::ptr::null_mut());

    (*manager.server).wm.dirty_windowing();
}

unsafe extern "C" fn handle_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let manager = &mut *crate::container_of!(listener, LockManager, destroy);

    log::debug!("ext_session_lock_v1 destroyed");

    wl_listener_remove(&mut manager.new_surface);
    wl_listener_remove(&mut manager.unlock);
    wl_listener_remove(&mut manager.destroy);

    manager.lock = std::ptr::null_mut();
    if manager.state == LockState::WaitingForLockSurfaces {
        manager.state = LockState::WaitingForBlank;
        ffi::wl_event_source_timer_update(manager.lock_surfaces_timer, 0);
    }
}

unsafe extern "C" fn handle_surface(listener: *mut ffi::wl_listener, data: *mut std::ffi::c_void) {
    let manager = &mut *crate::container_of!(listener, LockManager, new_surface);
    let wlr_lock_surface = data as *mut ffi::wlr_session_lock_surface_v1;

    log::debug!("new ext_session_lock_surface_v1 created");

    assert!(manager.state != LockState::Unlocked);
    assert!(!manager.lock.is_null());

    if LockSurface::create(wlr_lock_surface, manager.lock, manager).is_err() {
        log::error!("out of memory");
        ffi::wl_resource_post_no_memory((*wlr_lock_surface).resource);
    }
}

unsafe extern "C" fn update_focus(data: *mut std::ffi::c_void) {
    let lock_surface = data as *mut LockSurface;
    let manager = (*lock_surface).manager;

    let seats_head = &mut (*(*manager).server).input_manager.seats as *mut ffi::wl_list as *mut WlList;
    let mut curr = (*seats_head).next;
    while curr != seats_head {
        let next = (*curr).next;
        let seat = crate::container_of!(curr, crate::seat::Seat, link);
        if !matches!((*seat).focused, Focus::LockSurface(s) if s == lock_surface) {
            (*seat).focus(Focus::LockSurface(lock_surface));
        }
        (*seat).cursor.update_state();
        curr = next;
    }

    (*lock_surface).idle_update_focus = std::ptr::null_mut();
}

unsafe extern "C" fn handle_lock_surface_map(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let lock_surface = crate::container_of!(listener, LockSurface, map);

    let output = (*lock_surface).get_output();
    let x = (*output).sent.x;
    let y = (*output).sent.y;
    ffi::wlr_scene_node_set_position((*lock_surface).tree as *mut ffi::wlr_scene_node, x, y);

    let server = (*(*lock_surface).manager).server;
    let event_loop = ffi::wl_display_get_event_loop((*server).wl_server);
    assert!((*lock_surface).idle_update_focus.is_null());

    let idle = ffi::wl_event_loop_add_idle(
        event_loop,
        Some(update_focus),
        lock_surface as *mut _,
    );
    if idle.is_null() {
        log::error!("Failed to create idle update focus event source");
        return;
    }
    (*lock_surface).idle_update_focus = idle;
}

unsafe extern "C" fn handle_lock_surface_destroy(listener: *mut ffi::wl_listener, _data: *mut std::ffi::c_void) {
    let lock_surface = crate::container_of!(listener, LockSurface, surface_destroy);
    LockSurface::destroy(lock_surface);
}
