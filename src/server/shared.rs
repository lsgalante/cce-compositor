// SPDX-License-Identifier: GPL-3.0-only

//! Compositor state every subsystem may reach through a shared reference.
//!
//! The subsystems hold a raw `*mut Server` and reach each other through it:
//! `(*self.server).wm.dirty_windowing()` from a window, a seat, an output.
//! Each such call made a `&mut WindowManager` while the window manager was
//! often already mutably borrowed further up the stack — a manage pass
//! iterating windows whose methods mark more work pending — which is two
//! live `&mut` to one place.
//!
//! What lives here is reached through `&'static Shared` instead, and nothing
//! ever takes `&mut Shared`: every field is a `Cell` or a `OnceCell`, written
//! through `&self`. That is what makes [`shared`] a safe function. It holds:
//!
//! - the scene, set once when `Server::init` has built it and only read
//!   after ([`scene`]);
//! - the configuration, as an `Rc` snapshot ([`layout`]);
//! - the window manager's mode and whether Super is held, and so whether
//!   window adjusting is active ([`window_adjust_active`]);
//! - what the last manage pass applied: the outputs and seats in it, in
//!   order, and the output configuration it carries ([`sent`]);
//! - the display and its event loop;
//! - the pending work: whether a manage pass and a render pass are wanted,
//!   and the idle callback that runs them ([`pending`]).
//!
//! One `Shared` per thread, leaked on first use. Only the compositor thread
//! ever touches it — the IPC and stream threads talk to the main loop through
//! channels — and any other thread would see an inert instance of its own.

use crate::ffi;
use crate::scene::Scene;
use crate::config::Layout;
use crate::window_manager::WindowManagerMode;
use std::cell::{Cell, OnceCell, RefCell, UnsafeCell};
use std::rc::Rc;

pub struct Shared {
    scene: OnceCell<Scene>,
    layout: RefCell<Rc<Layout>>,
    display: Cell<*mut ffi::wl_display>,
    server: Cell<*mut crate::server::Server>,
    mode: Cell<WindowManagerMode>,
    adjust_held: Cell<bool>,
    sent: Sent,
    pending: Pending,
}

thread_local! {
    static SHARED: &'static Shared = Shared::leak(Shared {
        scene: OnceCell::new(),
        layout: RefCell::new(Rc::new(Layout::default())),
        display: Cell::new(std::ptr::null_mut()),
        server: Cell::new(std::ptr::null_mut()),
        mode: Cell::new(WindowManagerMode::Normal),
        adjust_held: Cell::new(false),
        sent: Sent::new(),
        pending: Pending::new(),
    });
}

/// This thread's `Shared`.
pub fn shared() -> &'static Shared {
    SHARED.with(|s| *s)
}

/// The scene. Panics before `Server::init` has built it.
pub fn scene() -> &'static Scene {
    shared().scene.get().expect("scene used before Server::init built it")
}

/// The configuration as it stands: a snapshot, cheap to take (an `Rc`), that
/// stays what it was if the configuration changes while it is held. Take it
/// once in a loop rather than per access.
pub fn layout() -> Rc<Layout> {
    shared().layout.borrow().clone()
}

/// Replace the configuration (a config load).
pub fn set_layout(layout: Layout) {
    *shared().layout.borrow_mut() = Rc::new(layout);
}

/// Change the configuration. `f` edits a copy and no borrow is held while it
/// runs, so `f` may itself read `layout()` (it sees the value before this
/// change); the copy is published when `f` returns. Snapshots taken before
/// keep the old value. Rare (a config load, an IPC `set`), so the copy is
/// cheap enough.
pub fn update_layout<R>(f: impl FnOnce(&mut Layout) -> R) -> R {
    let mut next = (*layout()).clone();
    let r = f(&mut next);
    set_layout(next);
    r
}

/// Normal or overview. Changed only by `WindowManager::set_mode`, which
/// also starts the handle fade.
pub fn mode() -> WindowManagerMode {
    shared().mode.get()
}

/// Whether Super is held (a seat keyboard's live mask, or an injected one),
/// as `WindowManager::refresh_adjust_held` last read it.
pub fn adjust_held() -> bool {
    shared().adjust_held.get()
}

/// Whether window adjusting is active: in overview, or with Super held.
/// Resize handles, the adjust target and its overlap dim follow it.
pub fn window_adjust_active() -> bool {
    mode() == WindowManagerMode::Overview || adjust_held()
}

/// The server, for an entry point with no object that leads to it (a
/// keyboard group without a seat); null before `Server::init`. Reached
/// through `reentry::wm` like any other.
pub fn server_ptr() -> *mut crate::server::Server {
    shared().server.get()
}

/// What the last manage pass applied.
pub fn sent() -> &'static Sent {
    &shared().sent
}

/// The pending manage and render work.
pub fn pending() -> &'static Pending {
    &shared().pending
}

impl Shared {
    /// Leak `shared` and initialise its list heads, which must not move
    /// once a link points at them.
    fn leak(shared: Shared) -> &'static Shared {
        let shared: &'static Shared = Box::leak(Box::new(shared));
        shared.sent.outputs.init();
        shared.sent.seats.init();
        shared
    }

    /// The display; null before `Server::init`.
    pub fn display(&self) -> *mut ffi::wl_display {
        self.display.get()
    }

    /// Record the display and the server, so the idle callback can find the
    /// window manager.
    ///
    /// # Safety
    /// `display` and `server` must stay valid until [`Pending::shutdown`].
    pub unsafe fn set_server(&self, display: *mut ffi::wl_display, server: *mut crate::server::Server) {
        self.display.set(display);
        self.server.set(server);
        self.pending.event_loop.set(ffi::wl_display_get_event_loop(display));
        self.pending.live.set(true);
    }

    /// Store the mode. `WindowManager::set_mode` is the way to change it;
    /// this is its store.
    pub(crate) fn store_mode(&self, mode: WindowManagerMode) {
        self.mode.set(mode);
    }

    /// Store whether Super is held. `WindowManager::refresh_adjust_held`
    /// is the way to change it; this is its store.
    pub(crate) fn store_adjust_held(&self, held: bool) {
        self.adjust_held.set(held);
    }

    /// Publish the scene once it is built. Panics if called twice.
    pub fn set_scene(&self, scene: Scene) {
        if self.scene.set(scene).is_err() {
            panic!("the scene is set once");
        }
    }
}

/// An intrusive `wl_list` head that lives in `Shared`, so its address is
/// fixed for the life of the thread. The links in it are fields of the
/// structs on the list (`Output::link_sent`, `Seat::link_sent`), recovered
/// with `container_of!`.
pub struct ListHead(UnsafeCell<ffi::wl_list>);

impl ListHead {
    const fn new() -> Self {
        ListHead(UnsafeCell::new(ffi::wl_list { prev: std::ptr::null_mut(), next: std::ptr::null_mut() }))
    }

    fn init(&self) {
        // SAFETY: called once, from `Shared::leak`, before anything can link
        // into it.
        unsafe { ffi::wl_list_init(self.0.get()) };
    }

    /// The head, for a walk: `(*head).next` until it comes back to `head`.
    pub fn head(&self) -> *mut ffi::wl_list {
        self.0.get()
    }

    /// Move `link` to the back of this list, unlinking it from wherever it
    /// was.
    ///
    /// # Safety
    /// `link` must be an initialised `wl_list` (linked or self-looped) in a
    /// struct that unlinks it before it is freed.
    pub unsafe fn move_to_back(&self, link: *mut ffi::wl_list) {
        use crate::server::{wl_list_insert, wl_list_remove, WlList};
        wl_list_remove(link as *mut WlList);
        wl_list_insert((*self.head()).prev as *mut WlList, link as *mut WlList);
    }
}

/// What the last manage pass applied: the outputs and seats in it, each
/// appended as its `manage_start` ran, and the output configuration a
/// client asked for, answered when the outputs commit (or fail to). It was
/// `WindowManager::sent`, reached through the server pointer by every
/// output and seat — and by every window, to find the seats focusing it —
/// from inside the manage pass that held the window manager.
pub struct Sent {
    pub outputs: ListHead,
    pub seats: ListHead,
    output_config: Cell<*mut ffi::wlr_output_configuration_v1>,
}

impl Sent {
    const fn new() -> Self {
        Sent {
            outputs: ListHead::new(),
            seats: ListHead::new(),
            output_config: Cell::new(std::ptr::null_mut()),
        }
    }

    /// The output configuration being applied, or null.
    pub fn output_config(&self) -> *mut ffi::wlr_output_configuration_v1 {
        self.output_config.get()
    }

    /// Take the output configuration being applied, leaving null.
    pub fn take_output_config(&self) -> *mut ffi::wlr_output_configuration_v1 {
        self.output_config.replace(std::ptr::null_mut())
    }

    pub fn set_output_config(&self, config: *mut ffi::wlr_output_configuration_v1) {
        self.output_config.set(config);
    }
}

/// Whether a manage pass and a render pass are wanted, and the idle callback
/// that runs them. Marking work pending used to need the window manager
/// mutably; it is two flags and an event source, so it is `Cell`s here.
pub struct Pending {
    windowing: Cell<bool>,
    rendering: Cell<bool>,
    idle: Cell<*mut ffi::wl_event_source>,
    event_loop: Cell<*mut ffi::wl_event_loop>,
    /// Between `set_server` and `shutdown`: the event loop exists.
    live: Cell<bool>,
}

impl Pending {
    const fn new() -> Self {
        Pending {
            windowing: Cell::new(false),
            rendering: Cell::new(false),
            idle: Cell::new(std::ptr::null_mut()),
            event_loop: Cell::new(std::ptr::null_mut()),
            live: Cell::new(false),
        }
    }

    /// Ask for a manage pass (and the render pass after it).
    #[track_caller]
    pub fn dirty_windowing(&self) {
        // Capturing and symbolizing a backtrace costs far more than the event it
        // annotates, and this fires on routine commits — the session runs at
        // --log-level debug, so keying it on Debug meant ~160 log lines/second
        // and most of a 19MB session log. Behind its own switch now.
        if crate::window_manager::dirty_backtrace_debug() {
            let bt = std::backtrace::Backtrace::force_capture();
            log::debug!("dirty_windowing called from backtrace:\n{}", bt);
        }
        if crate::window_manager::dirty_trace() {
            log::debug!("dirty_windowing from {}", std::panic::Location::caller());
        }
        self.windowing.set(true);
        self.arm();
    }

    /// Ask for a render pass.
    #[track_caller]
    pub fn dirty_rendering(&self) {
        if crate::window_manager::dirty_trace() {
            log::debug!("dirty_rendering from {}", std::panic::Location::caller());
        }
        self.rendering.set(true);
        self.arm();
    }

    /// Mark a manage pass wanted without arming the idle callback: for a
    /// caller about to run the pass itself.
    pub fn mark_windowing(&self) {
        self.windowing.set(true);
    }

    /// Mark a render pass wanted without arming the idle callback.
    pub fn mark_rendering(&self) {
        self.rendering.set(true);
    }

    /// The manage pass has started: clear its request, and drop the idle
    /// callback if nothing else is wanted.
    pub fn clean_windowing(&self) {
        self.windowing.set(false);
        self.disarm();
    }

    /// As `clean_windowing`, for the render pass.
    pub fn clean_rendering(&self) {
        self.rendering.set(false);
        self.disarm();
    }

    pub fn windowing(&self) -> bool {
        self.windowing.get()
    }

    pub fn rendering(&self) -> bool {
        self.rendering.get()
    }

    /// Whether the idle callback is armed.
    pub fn armed(&self) -> bool {
        !self.idle.get().is_null()
    }

    /// Arm the idle callback if work is wanted and it is not armed yet.
    pub fn arm(&self) {
        if !self.live.get() || !self.idle.get().is_null() {
            return;
        }
        if self.windowing.get() || self.rendering.get() {
            // SAFETY: `live` means `set_server`'s display, and so its event
            // loop, is still valid (`shutdown` clears it before the display
            // goes).
            let source = unsafe {
                ffi::wl_event_loop_add_idle(self.event_loop.get(), Some(handle_idle), std::ptr::null_mut())
            };
            self.idle.set(source);
        }
    }

    /// Drop the idle callback if nothing is wanted.
    pub fn disarm(&self) {
        if self.windowing.get() || self.rendering.get() {
            return;
        }
        let source = self.idle.replace(std::ptr::null_mut());
        if !source.is_null() {
            // SAFETY: an armed source belongs to the live event loop
            // (`shutdown` removes it before the display goes).
            unsafe { ffi::wl_event_source_remove(source) };
        }
    }

    /// The display is about to be destroyed: remove the idle callback and
    /// arm nothing more.
    pub fn shutdown(&self) {
        let source = self.idle.replace(std::ptr::null_mut());
        if !source.is_null() && self.live.get() {
            unsafe { ffi::wl_event_source_remove(source) };
        }
        self.live.set(false);
    }
}

/// The idle callback: run the manage pass if one is wanted, else the render
/// pass, when the window manager is between transactions.
unsafe extern "C" fn handle_idle(_data: *mut std::ffi::c_void) {
    let pending = pending();
    // wayland frees an idle source once it has run.
    pending.idle.set(std::ptr::null_mut());
    let server = shared().server.get();
    if server.is_null() {
        return;
    }
    let wm = &mut (*crate::reentry::wm(server));
    if matches!(wm.state, crate::window_manager::WindowManagerState::Idle) {
        if pending.windowing() {
            wm.manage_start();
        } else if pending.rendering() {
            wm.render_start();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(head: &ListHead, links: &[*mut ffi::wl_list]) -> Vec<usize> {
        let mut out = Vec::new();
        unsafe {
            let mut cur = (*head.head()).next;
            while cur != head.head() {
                out.push(links.iter().position(|&l| l == cur).unwrap());
                cur = (*cur).next;
            }
        }
        out
    }

    #[test]
    fn move_to_back_appends_and_moves_without_duplicating() {
        let head = Box::new(ListHead::new());
        head.init();
        let mut a: Box<ffi::wl_list> = Box::new(unsafe { std::mem::zeroed() });
        let mut b: Box<ffi::wl_list> = Box::new(unsafe { std::mem::zeroed() });
        let links = [&mut *a as *mut ffi::wl_list, &mut *b as *mut ffi::wl_list];
        unsafe {
            ffi::wl_list_init(links[0]);
            ffi::wl_list_init(links[1]);
            head.move_to_back(links[0]);
            head.move_to_back(links[1]);
            assert_eq!(order(&head, &links), [0, 1]);
            // The next manage pass appends them again, in its own order.
            head.move_to_back(links[0]);
            assert_eq!(order(&head, &links), [1, 0]);
            crate::server::wl_list_remove(links[0] as *mut crate::server::WlList);
            crate::server::wl_list_remove(links[1] as *mut crate::server::WlList);
        }
        assert_eq!(order(&head, &links), Vec::<usize>::new());
    }

    #[test]
    fn the_shared_lists_start_empty_and_initialised() {
        let head = sent().outputs.head();
        assert_eq!(unsafe { (*head).next }, head);
        assert!(sent().output_config().is_null());
    }
}
