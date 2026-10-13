// SPDX-FileCopyrightText: © 2026 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use crate::ffi;
use crate::server::{Server};
use crate::scene_node_data::SceneNodeData;

pub struct IdleInhibitManager {
    pub wlr_manager: *mut ffi::wlr_idle_inhibit_manager_v1,
    new_idle_inhibitor: crate::listener::Listener,
    /// Private: `holders` trusts that a registered inhibitor is live, which
    /// holds only while nothing outside this module can replace or swap it.
    inhibitors: crate::registry::Registry<IdleInhibitor>,
}

// SAFETY: a field of the one `Server`, which only the unsafe `Server::zeroed`
// makes and `run_server` never frees. It has no safe constructor, so safe code
// holding the one `&mut` to it has nothing to swap it with or replace it by.
unsafe impl crate::listener::Anchored for IdleInhibitManager {}

impl IdleInhibitManager {
    /// Write an empty registry into a zero-filled manager (`Server::zeroed`).
    ///
    /// # Safety
    /// As `InputManager::init_registry_in_place`.
    pub unsafe fn init_registry_in_place(this: *mut Self) {
        std::ptr::write(std::ptr::addr_of_mut!((*this).inhibitors), crate::registry::Registry::new());
    }

    /// # Safety
    /// `self` must be the `idle_inhibit_manager` field of the live `Server`
    /// `server` points to, whose display is up, and [`deinit`](Self::deinit)
    /// must run before that display is destroyed.
    pub unsafe fn init(&mut self, server: *mut Server) -> Result<(), &'static str> {
        let wlr_manager = ffi::wlr_idle_inhibit_v1_create((*server).wl_server);
        if wlr_manager.is_null() {
            return Err("Failed to create wlr_idle_inhibit_manager_v1");
        }
        self.wlr_manager = wlr_manager;

        self.new_idle_inhibitor.connect(&mut (*self.wlr_manager).events.new_inhibitor, handle_new_idle_inhibitor);

        Ok(())
    }

    /// # Safety
    /// Every registered inhibitor is freed here through its
    /// `inhibit_manager`, which must still be this manager at this address.
    pub unsafe fn deinit(&mut self) {
        self.new_idle_inhibitor.disconnect();
        
        for inhibitor in self.inhibitors.ptrs() {
            // Out of the registry through `self`, not through the
            // inhibitor's own pointer to this manager (`IdleInhibitor::
            // destroy`): that would make a second `&mut` to it while `self`
            // is live. At shutdown: nothing is left to keep awake, so no
            // recount.
            self.inhibitors.remove(inhibitor);
            IdleInhibitor::free(inhibitor);
        }
    }

    /// Who holds a live inhibitor, for [`publish`]. Every inhibitor is
    /// visited (no early exit): two apps holding one are both reported.
    pub fn holders(&self, wm: &crate::window_manager::WindowManager) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();

        // SAFETY: an inhibitor is in the registry from `create` until
        // `destroy` takes it out, before freeing it, on its wlroots
        // inhibitor's destroy — which wlroots emits before the surface it
        // inhibits goes. A surface's scene data lives as long as its scene
        // node, which a layer surface destroys before itself.
        for inhibitor in self.inhibitors.ptrs() { unsafe {

            let surface = (*(*inhibitor).wlr_inhibitor).surface;
            if let Some(node_data) = SceneNodeData::from_surface(surface) {
                use crate::scene_node_data::SceneNodeDataVal::*;
                let name = match node_data.data {
                    Window(id) => match wm.windows.get(id) {
                        Some(w) => inhibitor_name("window", w.get_app_id_string().as_deref(), w.get_title_string().as_deref()),
                        None => inhibitor_name("window", None, None),
                    },
                    LockSurface(_) => inhibitor_name("lock-surface", None, None),
                    LayerSurface(l) => {
                        let ns = (*l).wlr_layer_surface().with_namespace(|ns| (!ns.is_empty()).then(|| ns.to_string()));
                        inhibitor_name("layer", ns.as_deref(), None)
                    }
                    OverrideRedirect(_) => inhibitor_name("xwayland-override-redirect", None, None),
                };
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        } }

        names
    }
}

/// Hand a recount ([`IdleInhibitManager::holders`]) to the idle notifier,
/// whose inhibited flag clients see, and to the idle manager, which holds
/// its timers while anyone inhibits and names them in `ccectl idle status`.
pub fn publish(
    names: Vec<String>,
    notifier: &crate::wlr::Wlr<ffi::wlr_idle_notifier_v1>,
    idle: &mut crate::idle::IdleManager,
) {
    notifier.set_inhibited(!names.is_empty());
    idle.set_inhibitors(names);
}

/// Recount and publish, from an entry point.
unsafe fn recount(server: *mut Server, wm: &crate::window_manager::WindowManager) {
    let names = (*server).idle_inhibit_manager.holders(wm);
    publish(names, &crate::shared::handles().idle_notifier, &mut (*server).idle);
}

/// What the status line calls one inhibitor: a window's app id, its title
/// when the client never set one, and the bare kind when it set neither; a
/// layer surface shows as `layer:<namespace>`. Kinds are stable words, so a
/// script can tell a browser holding the display from the lock screen.
pub fn inhibitor_name(kind: &str, id: Option<&str>, title: Option<&str>) -> String {
    let id = id.filter(|s| !s.is_empty());
    let title = title.filter(|s| !s.is_empty());
    match (kind, id, title) {
        ("window", Some(id), _) => id.to_string(),
        ("window", None, Some(title)) => format!("window:{}", title),
        (kind, Some(id), _) => format!("{}:{}", kind, id),
        (kind, None, _) => kind.to_string(),
    }
}

pub struct IdleInhibitor {
    pub inhibit_manager: *mut IdleInhibitManager,
    pub wlr_inhibitor: *mut ffi::wlr_idle_inhibitor_v1,
    listen_destroy: crate::listener::Listener,
}

// SAFETY: made by `Box::into_raw` and freed only by its own destroy path,
// after its listeners are gone, and reached only through raw pointers: no safe
// API returns a `&mut`, a `Box` or a value of it, so nothing safe can move it.
unsafe impl crate::listener::Anchored for IdleInhibitor {}

impl IdleInhibitor {
    /// # Safety
    /// `wlr_inhibitor` must be a live inhibitor, and `inhibit_manager` the
    /// live manager to register it in, which must outlive it: the result is
    /// freed by [`destroy`](Self::destroy), which reaches the manager through it.
    pub unsafe fn create(
        wlr_inhibitor: *mut ffi::wlr_idle_inhibitor_v1,
        inhibit_manager: *mut IdleInhibitManager,
    ) -> Result<*mut Self, &'static str> {
        let inhibitor = Box::into_raw(Box::new(Self {
            inhibit_manager,
            wlr_inhibitor,
            listen_destroy: std::mem::zeroed(),
        }));

        (*inhibitor).listen_destroy.connect(&mut (*wlr_inhibitor).events.destroy, handle_inhibitor_destroy);

        (*inhibit_manager).inhibitors.push(inhibitor);

        Ok(inhibitor)
    }

    /// Take the inhibitor out of its manager's registry and free it. The
    /// caller recounts after (not at shutdown, where nothing is left to
    /// keep awake).
    ///
    /// # Safety
    /// `inhibitor` must be one [`create`](Self::create) returned and not yet
    /// destroyed, and its manager still live, with nothing holding a
    /// reference to it: this reaches the manager through the inhibitor's
    /// pointer to it. The inhibitor is freed here.
    pub unsafe fn destroy(inhibitor: *mut Self) {
        (*(*inhibitor).inhibit_manager).inhibitors.remove(inhibitor);
        Self::free(inhibitor);
    }

    /// Disconnect and free an inhibitor already out of the registry.
    ///
    /// # Safety
    /// `inhibitor` is live and in no registry; nothing uses it after.
    unsafe fn free(inhibitor: *mut Self) {
        (*inhibitor).listen_destroy.disconnect();
        drop(Box::from_raw(inhibitor));
    }
}

unsafe extern "C" fn handle_new_idle_inhibitor(
    listener: *mut ffi::wl_listener,
    data: *mut std::ffi::c_void,
) {
    let manager = &mut *crate::listener_owner!(listener, IdleInhibitManager, new_idle_inhibitor);
    let inhibitor = data as *mut ffi::wlr_idle_inhibitor_v1;

    if IdleInhibitor::create(inhibitor, manager).is_err() {
        log::error!("failed to create idle inhibitor");
    }
    let server = crate::shared::server_ptr();
    recount(server, &*crate::reentry::wm(server));
}

unsafe extern "C" fn handle_inhibitor_destroy(
    listener: *mut ffi::wl_listener,
    _data: *mut std::ffi::c_void,
) {
    let inhibitor = crate::listener_owner!(listener, IdleInhibitor, listen_destroy);
    IdleInhibitor::destroy(inhibitor);
    let server = crate::shared::server_ptr();
    recount(server, &*crate::reentry::wm(server));
}

#[cfg(test)]
mod tests {
    use super::{inhibitor_name, IdleInhibitManager, IdleInhibitor};
    use crate::ffi;
    use crate::server::WlList;

    /// A wlroots inhibitor with only its destroy signal set up.
    fn fake_inhibitor() -> Box<ffi::wlr_idle_inhibitor_v1> {
        let mut inhibitor: Box<ffi::wlr_idle_inhibitor_v1> = Box::new(unsafe { std::mem::zeroed() });
        let head = &mut inhibitor.events.destroy.listener_list as *mut ffi::wl_list as *mut WlList;
        unsafe {
            (*head).prev = head;
            (*head).next = head;
        }
        inhibitor
    }

    fn listeners(inhibitor: &ffi::wlr_idle_inhibitor_v1) -> usize {
        let head = &inhibitor.events.destroy.listener_list as *const ffi::wl_list;
        let mut n = 0;
        let mut cur = inhibitor.events.destroy.listener_list.next as *const ffi::wl_list;
        while cur != head {
            n += 1;
            cur = unsafe { (*cur).next };
        }
        n
    }

    #[test]
    fn deinit_frees_every_inhibitor_and_unlinks_its_listener() {
        // As `Server::zeroed` makes it: zero bytes, then a real registry.
        let mut manager = std::mem::MaybeUninit::<IdleInhibitManager>::zeroed();
        let mut manager = unsafe {
            IdleInhibitManager::init_registry_in_place(manager.as_mut_ptr());
            manager.assume_init()
        };
        let mut wlr: Vec<_> = (0..3).map(|_| fake_inhibitor()).collect();
        for inhibitor in &mut wlr {
            unsafe { IdleInhibitor::create(&mut **inhibitor, &mut manager) }.unwrap();
        }
        assert_eq!(manager.inhibitors.ptrs().len(), 3);
        assert!(wlr.iter().all(|i| listeners(i) == 1));

        // Each inhibitor leaves the registry through the manager this
        // borrows, not through its own pointer to it.
        unsafe { manager.deinit() };

        assert!(manager.inhibitors.ptrs().is_empty());
        assert!(wlr.iter().all(|i| listeners(i) == 0), "a freed inhibitor's listener is still linked");
    }

    #[test]
    fn a_window_is_its_app_id_then_its_title_then_the_kind() {
        assert_eq!(inhibitor_name("window", Some("cce-browser"), Some("Video")), "cce-browser");
        assert_eq!(inhibitor_name("window", None, Some("Video")), "window:Video");
        assert_eq!(inhibitor_name("window", Some(""), None), "window");
    }

    #[test]
    fn other_surfaces_are_their_kind_with_an_optional_id() {
        assert_eq!(inhibitor_name("layer", Some("bar"), None), "layer:bar");
        assert_eq!(inhibitor_name("layer", None, None), "layer");
        assert_eq!(inhibitor_name("lock-surface", None, None), "lock-surface");
    }
}
