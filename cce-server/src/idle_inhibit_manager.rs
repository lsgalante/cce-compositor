// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server, WlListener, wl_signal_add, wl_listener_remove};
use crate::scene_node_data::SceneNodeData;

pub struct IdleInhibitManager {
    pub wlr_manager: *mut ffi::wlr_idle_inhibit_manager_v1,
    pub new_idle_inhibitor: ffi::wl_listener,
    pub inhibitors: ffi::wl_list,
    pub server: *mut Server,
}

impl IdleInhibitManager {
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        self.server = server;
        ffi::wl_list_init(&mut self.inhibitors);

        let wlr_manager = ffi::wlr_idle_inhibit_v1_create((*server).wl_server);
        if wlr_manager.is_null() {
            return Err("Failed to create wlr_idle_inhibit_manager_v1");
        }
        self.wlr_manager = wlr_manager;

        let new_inhibitor_listener = &mut self.new_idle_inhibitor as *mut ffi::wl_listener as *mut WlListener;
        (*new_inhibitor_listener).notify = Some(handle_new_idle_inhibitor);
        wl_signal_add(
            &mut (*self.wlr_manager).events.new_inhibitor,
            &mut self.new_idle_inhibitor,
        );

        Ok(())
    }

    pub unsafe fn deinit(&mut self) {
        wl_listener_remove(&mut self.new_idle_inhibitor);
        
        let inhibitors_head = &mut self.inhibitors as *mut ffi::wl_list as *mut crate::server::WlList;
        let mut curr = (*inhibitors_head).next;
        while curr != inhibitors_head {
            let next = (*curr).next;
            let inhibitor = crate::container_of!(curr, IdleInhibitor, link);
            IdleInhibitor::destroy(inhibitor);
            curr = next;
        }
    }

    pub unsafe fn check_active(&self) {
        let mut inhibited = false;
        
        let inhibitors_head = &self.inhibitors as *const ffi::wl_list as *mut crate::server::WlList;
        let mut curr = (*inhibitors_head).next;
        while curr != inhibitors_head {
            let next = (*curr).next;
            let inhibitor = crate::container_of!(curr, IdleInhibitor, link);
            
            let surface = (*(*inhibitor).wlr_inhibitor).surface;
            if let Some(node_data) = SceneNodeData::from_surface(surface) {
                match node_data.data {
                    crate::scene_node_data::SceneNodeDataVal::Window(_) |
                    crate::scene_node_data::SceneNodeDataVal::ShellSurface(_) |
                    crate::scene_node_data::SceneNodeDataVal::LockSurface(_) |
                    crate::scene_node_data::SceneNodeDataVal::LayerSurface(_) |
                    crate::scene_node_data::SceneNodeDataVal::OverrideRedirect(_) => {
                        inhibited = true;
                        break;
                    }
                }
            }
            curr = next;
        }

        let notifier = (*self.server).input_manager.idle_notifier;
        if !notifier.is_null() {
            ffi::wlr_idle_notifier_v1_set_inhibited(notifier, inhibited);
        }
    }
}

pub struct IdleInhibitor {
    pub inhibit_manager: *mut IdleInhibitManager,
    pub wlr_inhibitor: *mut ffi::wlr_idle_inhibitor_v1,
    pub listen_destroy: ffi::wl_listener,
    pub link: ffi::wl_list,
}

impl IdleInhibitor {
    pub unsafe fn create(
        wlr_inhibitor: *mut ffi::wlr_idle_inhibitor_v1,
        inhibit_manager: *mut IdleInhibitManager,
    ) -> Result<*mut Self, &'static str> {
        let inhibitor = Box::into_raw(Box::new(Self {
            inhibit_manager,
            wlr_inhibitor,
            listen_destroy: std::mem::zeroed(),
            link: std::mem::zeroed(),
        }));

        let destroy_listener = &mut (*inhibitor).listen_destroy as *mut ffi::wl_listener as *mut WlListener;
        (*destroy_listener).notify = Some(handle_inhibitor_destroy);
        wl_signal_add(
            &mut (*wlr_inhibitor).events.destroy,
            &mut (*inhibitor).listen_destroy,
        );

        let list_head = &mut (*inhibit_manager).inhibitors as *mut ffi::wl_list as *mut crate::server::WlList;
        crate::server::wl_list_insert((*list_head).prev, &mut (*inhibitor).link as *mut ffi::wl_list as *mut crate::server::WlList);

        (*inhibit_manager).check_active();

        Ok(inhibitor)
    }

    pub unsafe fn destroy(inhibitor: *mut Self) {
        wl_listener_remove(&mut (*inhibitor).listen_destroy);
        crate::server::wl_list_remove(&mut (*inhibitor).link as *mut ffi::wl_list as *mut crate::server::WlList);
        
        let manager = (*inhibitor).inhibit_manager;
        let _boxed = Box::from_raw(inhibitor);
        
        (*manager).check_active();
    }
}

unsafe extern "C" fn handle_new_idle_inhibitor(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let manager = &mut *crate::container_of!(listener, IdleInhibitManager, new_idle_inhibitor);
    let inhibitor = data as *mut ffi::wlr_idle_inhibitor_v1;

    if IdleInhibitor::create(inhibitor, manager).is_err() {
        log::error!("failed to create idle inhibitor");
    }
}

unsafe extern "C" fn handle_inhibitor_destroy(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let inhibitor = crate::container_of!(listener, IdleInhibitor, listen_destroy);
    IdleInhibitor::destroy(inhibitor);
}
