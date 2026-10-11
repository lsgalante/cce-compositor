// SPDX-License-Identifier: GPL-3.0-only

//! Re-entrancy tracer for the window manager.
//!
//! A window, seat or output reaches the window manager through its server
//! pointer, `(*server).wm`. When that happens while a window-manager method
//! is already running further up the stack — a manage pass iterating windows
//! whose methods call back into it — there are two live borrows of one
//! `WindowManager`, which Rust's aliasing rules forbid. This measures how
//! often, and where, to size the change that removes it.
//!
//! Two instruments:
//!
//! - `wm_scope!` opens every `WindowManager` method: a per-thread depth
//!   count, plus the name of the outermost method and whether it borrowed
//!   `&mut`. Always on; two `Cell` writes.
//! - [`wm`] is how an entry point (a wlroots listener, timer or idle
//!   callback) takes the window manager, once, before handing it down as a
//!   parameter — nothing else calls it since the context-passing work. It
//!   returns the raw place, built without creating a reference, and with
//!   `CCE_REENTRY_TRACE` set it records an access made at depth > 0 (an
//!   entry reached from inside a window-manager method): the outer method,
//!   the access site, and whether the outer borrow was `&mut`.
//!
//! `ccectl debug-reentry` prints the tally, most frequent first.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::panic::Location;

/// Whether `CCE_REENTRY_TRACE` is set (read once).
pub fn enabled() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCE_REENTRY_TRACE").is_some())
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Hit {
    outer: &'static str,
    outer_mut: bool,
    file: &'static str,
    line: u32,
}

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static OUTER: Cell<(&'static str, bool)> = const { Cell::new(("", false)) };
    static HITS: RefCell<HashMap<Hit, u64>> = RefCell::new(HashMap::new());
}

/// An open window-manager method; closes on drop.
pub struct Scope(());

impl Drop for Scope {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// Open a window-manager method named `name`; `mutable` when it took
/// `&mut self`. Use `wm_scope!` rather than calling this.
pub fn enter(name: &'static str, mutable: bool) -> Scope {
    DEPTH.with(|d| {
        if d.get() == 0 {
            OUTER.with(|o| o.set((name, mutable)));
        }
        d.set(d.get() + 1);
    });
    Scope(())
}

/// Open the enclosing `WindowManager` method for the tracer: `wm_scope!(mut)`
/// in a `&mut self` method, `wm_scope!()` in a `&self` one.
#[macro_export]
macro_rules! wm_scope {
    (@name) => {{
        fn __f() {}
        let n = ::std::any::type_name_of_val(&__f);
        &n[..n.len() - "::__f".len()]
    }};
    (mut) => {
        let _wm_scope = $crate::reentry::enter($crate::wm_scope!(@name), true);
    };
    () => {
        let _wm_scope = $crate::reentry::enter($crate::wm_scope!(@name), false);
    };
}

/// The window manager inside `server`, as a raw place, for code outside the
/// window manager. Records a re-entry when traced; see the module docs.
///
/// # Safety
/// `server` must point to the live `Server`.
#[track_caller]
#[inline]
pub unsafe fn wm(server: *mut crate::server::Server) -> *mut crate::window_manager::WindowManager {
    if enabled() {
        record(Location::caller());
    }
    std::ptr::addr_of_mut!((*server).wm)
}

fn record(site: &'static Location<'static>) {
    let depth = DEPTH.with(|d| d.get());
    if depth == 0 {
        return;
    }
    let (outer, outer_mut) = OUTER.with(|o| o.get());
    let hit = Hit { outer, outer_mut, file: site.file(), line: site.line() };
    let first = HITS.with(|h| {
        let mut h = h.borrow_mut();
        let n = h.entry(hit).or_insert(0);
        *n += 1;
        *n == 1
    });
    if first {
        log::info!(
            "[reentry] {}:{} reaches the window manager inside {} ({})",
            site.file(),
            site.line(),
            outer,
            if outer_mut { "&mut self" } else { "&self" }
        );
    }
}

/// The tally, most frequent first: count, outer method, its borrow, site.
pub fn report() -> String {
    if !enabled() {
        return "re-entrancy tracing is off: start the compositor with CCE_REENTRY_TRACE=1\n".to_string();
    }
    let mut rows: Vec<(Hit, u64)> = HITS.with(|h| h.borrow().iter().map(|(k, v)| (*k, *v)).collect());
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.outer.cmp(b.0.outer)).then(a.0.file.cmp(b.0.file)).then(a.0.line.cmp(&b.0.line)));
    let total: u64 = rows.iter().map(|r| r.1).sum();
    let mut out = format!("{} re-entries at {} sites\n", total, rows.len());
    for (hit, n) in rows {
        out.push_str(&format!(
            "{n}\t{}\t{}\t{}:{}\n",
            hit.outer.trim_start_matches("cce_fx::"),
            if hit.outer_mut { "mut" } else { "ref" },
            hit.file,
            hit.line
        ));
    }
    out
}

/// Clear the tally.
pub fn reset() {
    HITS.with(|h| h.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_counts_nested_scopes_and_keeps_the_outermost_name() {
        {
            let _a = enter("outer", true);
            {
                let _b = enter("inner", false);
                assert_eq!(DEPTH.with(|d| d.get()), 2);
            }
            assert_eq!(OUTER.with(|o| o.get()), ("outer", true));
        }
        assert_eq!(DEPTH.with(|d| d.get()), 0);
    }

    #[test]
    fn the_scope_macro_names_its_function() {
        fn probe() -> &'static str {
            crate::wm_scope!(@name)
        }
        assert!(probe().ends_with("::probe"), "{}", probe());
    }
}
