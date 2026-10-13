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

/// `window_mut!` for seats: the seat `$id` borrowed from `$wm.seats`,
/// returning `$ret` (default `()`) when it has gone. For the `(wm, SeatId)`
/// seat functions, which re-take it after each call that needs `wm`.
macro_rules! seat_mut {
    ($wm:expr, $id:expr) => {
        seat_mut!($wm, $id, ())
    };
    ($wm:expr, $id:expr, $ret:expr) => {
        match $wm.seats.get_mut($id) {
            Some(s) => s,
            None => return $ret,
        }
    };
}

/// The cursor of seat `$id`, borrowed from `$wm.seats` as `seat_mut!` does.
macro_rules! cursor_mut {
    ($wm:expr, $id:expr) => {
        cursor_mut!($wm, $id, ())
    };
    ($wm:expr, $id:expr, $ret:expr) => {
        &mut seat_mut!($wm, $id, $ret).cursor
    };
}

/// In a wlroots callback on a seat, its cursor or its relay: the id of the
/// seat whose keyed listener at `$($field)+` — a path from the seat,
/// `cursor.motion_listener` — was called (`crate::listener_id!`, which
/// aborts if the seat its id names does not hold that listener). Unsafe:
/// it reaches the window manager through the shared server.
macro_rules! listener_seat {
    ($listener:expr, $($field:tt)+) => {
        crate::listener_id!(
            $listener,
            (*crate::reentry::wm(crate::shared::server_ptr())).seats,
            crate::seat::SeatId,
            $($field)+
        )
    };
}

/// `listener_seat!` as the seat's raw place in the store, for a callback
/// that holds the seat beside the window manager (the store keeps raw
/// pointers for that); returns early if the seat is gone.
macro_rules! listener_seat_ptr {
    ($listener:expr, $($field:tt)+) => {{
        let sid = listener_seat!($listener, $($field)+);
        match (*crate::reentry::wm(crate::shared::server_ptr())).seats.ptr(sid) {
            Some(seat) => seat,
            None => return,
        }
    }};
}

/// The window manager's output and seat stores as a `crate::ctx::Stores`,
/// borrowed through their own field places — so it can be taken while a
/// `window_mut!` borrow of `wm.windows` is live. Safe: the borrow is tied
/// to `wm`, so take it again after any call that needs `&mut wm`.
macro_rules! stores {
    ($wm:expr) => {
        crate::ctx::Stores {
            outputs: &$wm.outputs,
            seats: &$wm.seats,
        }
    };
}

/// `window_mut!`'s shared twin: the window borrowed from `wm.windows`
/// alongside a shared `wm`, for the `&self` methods that read the window
/// manager too.
macro_rules! window_ref {
    ($wm:expr, $id:expr) => {
        window_ref!($wm, $id, ())
    };
    ($wm:expr, $id:expr, $ret:expr) => {
        match $wm.windows.get($id) {
            Some(w) => w,
            None => return $ret,
        }
    };
}
