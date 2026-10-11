// SPDX-License-Identifier: GPL-3.0-only

//! Borrowing a window from the store (`wm.windows`) by id. Declared ahead of
//! every module that uses them (`#[macro_use]` in lib.rs).

/// The window `id` names, borrowed mutably from `wm.windows`, or an early
/// return of `$ret` (default `()`) when it has gone. Borrowing only the
/// `windows` field, it leaves `wm`'s other fields free to read beside it;
/// a call to a `wm` method ends it, and the window is fetched again after.
macro_rules! window_mut {
    ($wm:expr, $id:expr) => {
        window_mut!($wm, $id, ())
    };
    ($wm:expr, $id:expr, $ret:expr) => {
        match $wm.windows.get_mut($id) {
            Some(w) => w,
            None => return $ret,
        }
    };
}

/// `window_mut!`'s shared twin: the window borrowed from `wm.windows`
/// alongside a shared `wm`, for the `&self` methods that read the window
/// manager too.
macro_rules! window_ref {
    ($wm:expr, $id:expr, $ret:expr) => {
        match $wm.windows.get($id) {
            Some(w) => w,
            None => return $ret,
        }
    };
}

/// The window's address from the store, or an early return of `$ret` when
/// it has gone: for a call that still takes `*mut Window`. Borrowing
/// `wm.windows` for the lookup, it cannot sit inside a live `window_mut!`
/// borrow, which keeps the raw pointer and the reference apart.
macro_rules! window_ptr {
    ($wm:expr, $id:expr, $ret:expr) => {
        match $wm.windows.ptr($id) {
            Some(p) => p,
            None => return $ret,
        }
    };
}
