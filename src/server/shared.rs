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
//! - the display and its event loop;
//! - the pending work: whether a manage pass and a render pass are wanted,
//!   and the idle callback that runs them ([`pending`]).
//!
//! One `Shared` per thread, leaked on first use. Only the compositor thread
//! ever touches it — the IPC and stream threads talk to the main loop through
//! channels — and any other thread would see an inert instance of its own.

use crate::ffi;
use crate::scene::Scene;
use std::cell::{Cell, OnceCell};

pub struct Shared {
    scene: OnceCell<Scene>,
    display: Cell<*mut ffi::wl_display>,
    server: Cell<*mut crate::server::Server>,
    pending: Pending,
}

thread_local! {
    static SHARED: &'static Shared = Box::leak(Box::new(Shared {
        scene: OnceCell::new(),
        display: Cell::new(std::ptr::null_mut()),
        server: Cell::new(std::ptr::null_mut()),
        pending: Pending::new(),
    }));
}

/// This thread's `Shared`.
pub fn shared() -> &'static Shared {
    SHARED.with(|s| *s)
}

/// The scene. Panics before `Server::init` has built it.
pub fn scene() -> &'static Scene {
    shared().scene.get().expect("scene used before Server::init built it")
}

/// The pending manage and render work.
pub fn pending() -> &'static Pending {
    &shared().pending
}

impl Shared {
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

    /// Publish the scene once it is built. Panics if called twice.
    pub fn set_scene(&self, scene: Scene) {
        if self.scene.set(scene).is_err() {
            panic!("the scene is set once");
        }
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
    let wm = &mut (*server).wm;
    if matches!(wm.state, crate::window_manager::WindowManagerState::Idle) {
        if pending.windowing() {
            wm.manage_start();
        } else if pending.rendering() {
            wm.render_start();
        }
    }
}
