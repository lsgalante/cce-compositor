// SPDX-License-Identifier: GPL-3.0-only

//! `ccectl debug-output add WxH | remove NAME`: plug and unplug HEADLESS
//! outputs, so a cce-shadow session (headless backend, which otherwise
//! keeps its outputs for life) can exercise output hotplug: the destroy
//! path a monitor unplug takes. Any other kind of output is refused, so it
//! cannot touch a real session's monitors.
//!
//! The change runs from an idle callback, an entry point of its own:
//! adding or destroying an output emits wlroots signals whose handlers take
//! the window manager, which the IPC dispatcher holds.

use crate::ffi;
use std::cell::RefCell;

enum Op {
    Add(u32, u32),
    Remove(String),
}

thread_local! {
    /// Changes `command` queued for `run`.
    static OPS: RefCell<Vec<Op>> = const { RefCell::new(Vec::new()) };
}

const USAGE: &str = "error: usage: debug-output add WxH | remove NAME\n";

/// The headless output named `name`, if one is in `outputs`.
fn headless_named(outputs: &crate::output::Outputs, name: &str) -> Result<*mut ffi::wlr_output, String> {
    let found = outputs.values().map(|o| o.wlr_output().ptr()).find(|&p| {
        // SAFETY: an output's held `wlr_output` is live (its destroy
        // handler lets go of it before wlroots frees it).
        !p.is_null() && unsafe { std::ffi::CStr::from_ptr(ffi::river_wlr_output_get_name(p)) }.to_string_lossy() == name
    });
    match found {
        // SAFETY: as above.
        Some(p) if unsafe { ffi::cce_output_is_headless(p) } => Ok(p),
        Some(_) => Err(format!("error: {name} is not a headless output\n")),
        None => Err(format!("error: no output {name}\n")),
    }
}

/// The `debug-output` command: checks it now, queues the change for after
/// this dispatch, and replies.
pub fn command(outputs: &crate::output::Outputs, args: &[&str]) -> String {
    let op = match args {
        ["add", size] => {
            let parsed = size.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
            match parsed {
                Some((w, h)) if w > 0 && h > 0 => Op::Add(w, h),
                _ => return USAGE.to_string(),
            }
        }
        ["remove", name] => match headless_named(outputs, name) {
            Ok(_) => Op::Remove(name.to_string()),
            Err(e) => return e,
        },
        _ => return USAGE.to_string(),
    };
    let first = OPS.with(|ops| {
        let mut ops = ops.borrow_mut();
        let first = ops.is_empty();
        ops.push(op);
        first
    });
    let event_loop = crate::shared::event_loop();
    if first && !event_loop.is_null() && crate::shared::pending().loop_live() {
        // SAFETY: the shared loop is live (`loop_live`), and the callback
        // takes no data; an idle source frees itself once it has run.
        unsafe { ffi::wl_event_loop_add_idle(event_loop, Some(run), std::ptr::null_mut()) };
    }
    "ok\n".to_string()
}

/// The idle callback `command` arms: make each queued change.
unsafe extern "C" fn run(_data: *mut std::ffi::c_void) {
    let ops = OPS.with(|ops| std::mem::take(&mut *ops.borrow_mut()));
    let server = crate::shared::server_ptr();
    if server.is_null() {
        return;
    }
    for op in ops {
        match op {
            Op::Add(w, h) => {
                let output = ffi::cce_headless_add_output(crate::shared::handles().backend.ptr(), w, h);
                if output.is_null() {
                    log::warn!("debug-output add: no headless backend");
                }
            }
            Op::Remove(name) => {
                // Looked up afresh, with no borrow held across the destroy:
                // its handlers take the window manager themselves.
                match headless_named(&(*crate::reentry::wm(server)).outputs, &name) {
                    Ok(wlr_output) => {
                        log::info!("debug-output: destroying {name}");
                        ffi::wlr_output_destroy(wlr_output);
                    }
                    Err(e) => log::warn!("debug-output remove: {}", e.trim_end()),
                }
            }
        }
    }
}
