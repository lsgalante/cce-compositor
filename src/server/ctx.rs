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
