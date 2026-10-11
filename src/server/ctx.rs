// SPDX-License-Identifier: GPL-3.0-only

//! What a function reaches besides its own object, handed down from the
//! entry point instead of fetched through a `server` back-pointer (the
//! architecture RFC's step 4, "Ctx").

use crate::output::Outputs;
use crate::seat::Seats;
use crate::server::Server;

/// The output and seat stores, borrowed to read: which output is the first
/// enabled one, which seat focuses, hovers or resizes which window.
///
/// For the window's drawing, effect and placement code, which runs with the
/// window borrowed from `wm.windows` and so cannot also take the window
/// manager. Both stores are `Server` fields beside the window manager, not
/// inside it, so a `Stores` sits next to that borrow. Copy: hand it down by
/// value.
#[derive(Clone, Copy)]
pub struct Stores<'a> {
    pub outputs: &'a Outputs,
    pub seats: &'a Seats,
}

impl<'a> Stores<'a> {
    /// Borrow the server's stores.
    ///
    /// # Safety
    /// `server` is the live server, and no output or seat is inserted into
    /// or taken out of its store while the borrow lasts — a destroy path
    /// runs from the manage pass, never under a drawing or effect pass.
    pub unsafe fn of(server: *mut Server) -> Stores<'a> {
        let wm = crate::reentry::wm(server);
        Stores {
            outputs: &*std::ptr::addr_of!((*wm).outputs),
            seats: &*std::ptr::addr_of!((*wm).seats),
        }
    }
}

/// The server's subsystems other than the window manager, borrowed together
/// at an entry point and handed down to the window-manager paths that call
/// into them — the RFC's `Ctx` without its `wm`, which those paths already
/// hold (as `self` or a `wm` parameter). Each field is a disjoint `Server`
/// field, so the borrows coexist with each other and with the window
/// manager's. The input manager is NOT in it: the input path's own
/// teardown (`InputManager::deinit`, detaching devices) runs code that
/// takes a `Sys`, so the few that need the input manager (the config
/// reload, the IPC device commands) take it as a parameter of its own.
pub struct Sys<'a> {
    pub om: &'a mut crate::output_manager::OutputManager,
    pub idle: &'a mut crate::idle::IdleManager,
    pub lock: &'a mut crate::lock_manager::LockManager,
    pub inhibit: &'a mut crate::idle_inhibit_manager::IdleInhibitManager,
}

impl<'a> Sys<'a> {
    /// Borrow the server's subsystems, at an entry point.
    ///
    /// # Safety
    /// `server` is the live server, and nothing else reaches these
    /// subsystems while the borrow lasts: an entry point takes one `Sys`
    /// and hands it down, as it takes one window manager.
    pub unsafe fn of(server: *mut Server) -> Sys<'a> {
        Sys {
            om: &mut *std::ptr::addr_of_mut!((*server).om),
            idle: &mut *std::ptr::addr_of_mut!((*server).idle),
            lock: &mut *std::ptr::addr_of_mut!((*server).lock_manager),
            inhibit: &mut *std::ptr::addr_of_mut!((*server).idle_inhibit_manager),
        }
    }
}
