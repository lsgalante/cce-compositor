//! The window switcher (cce-cloud's chooser): which windows it offers, launching
//! it, and the thread that waits for the pick. Split out of window_manager.rs on
//! 2026-10-10.

use super::*;

impl WindowManager {
    /// True for windows that should appear in the window switcher: mapped,
    /// non-closed, and not one of the desktop-shell surfaces (status bar,
    /// wallpaper, or the switcher's own cce-cloud overlay).
    pub(crate) unsafe fn is_switchable_window(&self, w: *mut Window) -> bool {
        crate::wm_scope!();
        if w.is_null() || (*w).closed {
            return false;
        }
        if !matches!((*w).state, crate::window::WindowState::Mapped) {
            return false;
        }
        match (*w).get_app_id_string().as_deref() {
            Some(id) if id.starts_with("cce-status") => false,
            Some("cce-wallpaper") | Some("cce-cloud") | Some("cce-grid") => false,
            _ => true,
        }
    }

    /// Open the alt-tab window switcher: spawn a `cce-cloud --switcher` overlay,
    /// feed it the currently switchable windows in most-recently-used order, and
    /// hand off to a background thread that focuses whatever the user commits to.
    ///
    /// cce-cloud is a `Layer::Overlay` surface with exclusive keyboard focus,
    /// and it commits on Super release / cancels on Escape by itself. Tab while
    /// Super is held, however, never reaches it — that chord matches this very
    /// keybinding — so a repeat press lands back here and is forwarded as a
    /// `__cce_switcher_next__` / `__cce_switcher_prev__` line down the held-open
    /// stdin pipe, moving the highlight. The committed entry is printed to
    /// stdout; we only build the list and map the selection back to a window id.
    ///
    /// `backwards` is the super+shift+tab direction: it cycles the highlight
    /// the other way, and opening with it lands on the least-recently-used
    /// window instead of the previously focused one.
    pub unsafe fn launch_window_switcher(&mut self, backwards: bool) {
        crate::wm_scope!(mut);
        let cycle_line: &[u8] = if backwards {
            b"__cce_switcher_prev__\n"
        } else {
            b"__cce_switcher_next__\n"
        };

        // A repeat super+(shift+)tab while the switcher is already up moves its
        // highlight instead of spawning a second switcher.
        {
            let mut active = ACTIVE_SWITCHER.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(handle) = active.as_mut() {
                use std::io::Write;
                if handle
                    .stdin
                    .write_all(cycle_line)
                    .and_then(|_| handle.stdin.flush())
                    .is_ok()
                {
                    return;
                }
                // Dead pipe: the child exited and we raced its cleanup thread.
                // Drop the stale handle and open a fresh switcher below.
                *active = None;
            }
        }
        // Most-recently-used order (focus_history is MRU-front). The focused
        // window lands first, so cce-cloud auto-selects index 1 — the previously
        // focused window — which is the classic alt-tab default.
        let mut ordered: Vec<*mut Window> = Vec::new();
        for &w in self.focus_history.iter() {
            if self.is_switchable_window(w) && !ordered.contains(&w) {
                ordered.push(w);
            }
        }
        for &w in self.windows.iter() {
            if self.is_switchable_window(w) && !ordered.contains(&w) {
                ordered.push(w);
            }
        }
        if ordered.is_empty() {
            return;
        }

        // cce-cloud echoes the committed entry back verbatim, so keep the mapping
        // from display string to window id to resolve the selection.
        let mut items: Vec<(String, String)> = Vec::new();
        let mut input = String::new();
        for &w in &ordered {
            let id = (*w).ref_key.index.to_string();
            let app_id = (*w).get_app_id_string().unwrap_or_default();
            let title = (*w).get_title_string().unwrap_or_default();
            let display = if title.is_empty() {
                app_id.clone()
            } else {
                format!("{} ({})", title, app_id)
            };
            input.push_str(&display);
            input.push('\n');
            items.push((id, display));
        }
        if backwards {
            // cce-cloud auto-highlights index 1 once the items land; two prev
            // steps from there wrap to the last entry, so a backwards open
            // starts on the least-recently-used window (classic alt+shift+tab).
            input.push_str("__cce_switcher_prev__\n__cce_switcher_prev__\n");
        }

        // Position near the top-centre of the enabled output. Passing explicit
        // -x/-y keeps cce-cloud a layer-shell overlay (omitting both would make
        // it an XDG toplevel, which would not grab keyboard the same way).
        let (mut vp_w, mut origin_x, mut origin_y) = (1920.0_f64, 0i32, 0i32);
        let outputs_list = &mut (*self.server).om.outputs as *mut ffi::wl_list as *mut WlList;
        let mut curr_out = (*outputs_list).next;
        while curr_out != outputs_list {
            let output = crate::container_of!(curr_out, crate::output::Output, link);
            if (*output).sent.state == crate::output::OutputStateValue::Enabled {
                let wlr_box = (*output).sent.box_layout();
                vp_w = wlr_box.width as f64;
                origin_x = wlr_box.x;
                origin_y = wlr_box.y;
                break;
            }
            curr_out = (*curr_out).next;
        }
        // cce-cloud's default logical width is 600; centre it horizontally.
        let x_pos = origin_x + (((vp_w - 600.0) / 2.0).max(0.0)) as i32;
        let y_pos = origin_y + 80;

        let display_env = std::env::var("WAYLAND_DISPLAY").ok();

        let mut child = match std::process::Command::new(cce_cloud_cmd())
            .args([
                "--switcher",
                "-p",
                "Windows:",
                "-x",
                &x_pos.to_string(),
                "-y",
                &y_pos.to_string(),
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                log::error!("window switcher: failed to spawn cce-cloud: {}", e);
                return;
            }
        };

        // Feed the item list but keep stdin open: repeat super+(shift+)tab
        // presses write cycle lines down the same pipe. cce-cloud streams
        // items in as they arrive and does not wait for EOF.
        let generation = SWITCHER_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(input.as_bytes());
            let _ = stdin.flush();
            *ACTIVE_SWITCHER.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(SwitcherHandle { generation, stdin });
        }

        std::thread::spawn(move || {
            run_window_switcher(child, items, display_env);
            // The switcher is gone; release its stdin unless a newer switcher
            // already replaced it.
            let mut active = ACTIVE_SWITCHER.lock().unwrap_or_else(|e| e.into_inner());
            if active.as_ref().map_or(false, |h| h.generation == generation) {
                *active = None;
            }
        });
    }
}

/// Prefer the installed `~/.local/bin/cce-cloud`, falling back to PATH lookup.
pub(crate) fn cce_cloud_cmd() -> String {
    if let Ok(home) = std::env::var("HOME") {
        let path = format!("{}/.local/bin/cce-cloud", home);
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }
    "cce-cloud".to_string()
}

/// Stdin of the currently open `cce-cloud --switcher` child. Held open (in
/// `ACTIVE_SWITCHER`) so repeat super+tab presses can advance the highlight via
/// cce-cloud's magic `__cce_switcher_next__` stdin line; the generation lets the
/// per-switcher cleanup thread avoid clearing a newer switcher's handle.
pub(crate) struct SwitcherHandle {
    generation: u64,
    stdin: std::process::ChildStdin,
}

pub(crate) static ACTIVE_SWITCHER: std::sync::Mutex<Option<SwitcherHandle>> = std::sync::Mutex::new(None);
pub(crate) static SWITCHER_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Tail of the window switcher, run on a detached thread: waits for the
/// committed selection on the already-spawned child's stdout, and asks the
/// compositor to focus it via the control socket (so the actual focus change
/// happens on the main thread through the IPC dispatcher).
pub(crate) fn run_window_switcher(
    mut child: std::process::Child,
    items: Vec<(String, String)>,
    display_env: Option<String>,
) {
    use std::io::{Read, Write};

    let mut selected = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut selected);
    }
    let _ = child.wait();

    let selected = selected.trim();
    if selected.is_empty() {
        return; // cancelled (Escape) or empty selection
    }

    let id = match items.iter().find(|(_, display)| display == selected) {
        Some((id, _)) => id.clone(),
        None => return,
    };

    let sock = cce_core::ipc::ctl::control_socket_for(display_env.as_deref());
    if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&sock) {
        let _ = stream.write_all(format!("focus-window {}\n", id).as_bytes());
        let _ = stream.flush();
        let mut resp = String::new();
        let _ = stream.read_to_string(&mut resp);
    }
}
