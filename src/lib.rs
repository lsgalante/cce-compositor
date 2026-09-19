// cce — Unified compositor server and window manager client

// ==========================================
// Compositor Server Modules
// ==========================================
#[path = "server/ffi.rs"]
pub mod ffi;
#[path = "server/server.rs"]
pub mod server;
#[path = "server/process.rs"]
pub mod process;
#[path = "server/util.rs"]
pub mod util;
pub use cce_window_manager::slotmap;
#[path = "server/window_manager.rs"]
pub mod window_manager;
#[path = "server/xkb_bindings.rs"]
pub mod xkb_bindings;
#[path = "server/layer_shell.rs"]
pub mod layer_shell;
#[path = "server/scene.rs"]
pub mod scene;
#[path = "server/text.rs"]
pub mod text;
// The window-management policy layer lives in the sibling crate
// `cce-window-manager` (pure Rust, no FFI). The aliases keep the historical
// `crate::policy::…` / `crate::tiling` / `crate::slotmap` paths working.
pub use cce_window_manager as policy;
pub use cce_window_manager::tiling;
#[path = "server/backdrop.rs"]
pub mod backdrop;
#[path = "server/config.rs"]
pub mod config;
#[path = "server/ipc_server.rs"]
pub mod ipc_server;
#[path = "server/screenshot.rs"]
pub mod screenshot;
#[path = "server/status_server.rs"]
pub mod status_server;
#[path = "server/stream_server.rs"]
pub mod stream_server;
#[path = "server/scene_node_data.rs"]
pub mod scene_node_data;
#[path = "server/output.rs"]
pub mod output;
#[path = "server/output_manager.rs"]
pub mod output_manager;
#[path = "server/input_manager.rs"]
pub mod input_manager;
#[path = "server/libinput_config.rs"]
pub mod libinput_config;
#[path = "server/libinput_device.rs"]
pub mod libinput_device;
#[path = "server/libinput_accel_config.rs"]
pub mod libinput_accel_config;
#[path = "server/xkb_keyboard.rs"]
pub mod xkb_keyboard;
#[path = "server/xkb_config.rs"]
pub mod xkb_config;
#[path = "server/idle_inhibit_manager.rs"]
pub mod idle_inhibit_manager;
#[path = "server/idle.rs"]
pub mod idle;
#[path = "server/lock_manager.rs"]
pub mod lock_manager;
#[path = "server/input_device.rs"]
pub mod input_device;
#[path = "server/pointer_constraint.rs"]
pub mod pointer_constraint;
#[path = "server/keyboard.rs"]
pub mod keyboard;
#[path = "server/cursor.rs"]
pub mod cursor;
#[path = "server/seat.rs"]
pub mod seat;
#[path = "server/tablet.rs"]
pub mod tablet;
#[path = "server/tablet_tool.rs"]
pub mod tablet_tool;
#[path = "server/window.rs"]
pub mod window;
#[path = "server/xdg_toplevel.rs"]
pub mod xdg_toplevel;
#[path = "server/xdg_popup.rs"]
pub mod xdg_popup;
#[path = "server/shell_surface.rs"]
pub mod shell_surface;
#[path = "server/wm_node.rs"]
pub mod wm_node;
#[path = "server/xwayland_window.rs"]
pub mod xwayland_window;
#[path = "server/xwayland_override_redirect.rs"]
pub mod xwayland_override_redirect;
#[path = "server/text_input.rs"]
pub mod text_input;
#[path = "server/input_relay.rs"]
pub mod input_relay;
#[path = "server/input_popup.rs"]
pub mod input_popup;
#[path = "server/drag_icon.rs"]
pub mod drag_icon;
#[path = "server/pointer_binding.rs"]
pub mod pointer_binding;
#[path = "server/keyboard_group.rs"]
pub mod keyboard_group;
#[path = "server/inspector.rs"]
pub mod inspector;
#[path = "server/cce_window_management.rs"]
pub mod cce_window_management;

#[path = "server/run_server.rs"]
pub mod run_server;
pub use run_server::run_server;


// ==========================================
// IPC Client Modules
// ==========================================
#[path = "cce_ctl.rs"]
pub mod cce_ctl;
pub use cce_ctl::run_cce_ctl;

#[path = "migrate_input.rs"]
pub mod migrate_input;

/// CLAUDE.md's `(~Nk lines)` figures, checked against the files they describe.
///
/// Those numbers exist to set expectations before opening a file — "this is
/// the big one" — and they drift in total silence, because a stale number
/// reads exactly like a fresh one. A workspace sweep on 2026-09-19 found
/// EVERY size figure in every CLAUDE.md stale, all of them undercounts, the
/// worst by 49% (cce-designer's app.rs, written ~5.4k at 8046 lines).
///
/// Tolerance is 10%: loose enough that ordinary work does not trip it, tight
/// enough that a file cannot quietly double. When it fails, write the number
/// it reports — that is the whole fix.
///
/// Deliberately MIRRORED into each crate that carries such a figure rather
/// than shared from a helper: every crate here is its own git repository and
/// must build standalone, and this needs nothing but `std`. Same call
/// `ramp.rs` makes about its cce-ui parser.
#[cfg(test)]
mod doc_size_claims {
    use std::path::{Path, PathBuf};

    /// One `(~Nk lines)` claim: the name as CLAUDE.md spells it, the figure,
    /// and whether the claim also calls it the largest file.
    fn claims(doc: &str) -> Vec<(String, f64, bool)> {
        const TAIL: &str = " lines)";
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(p) = doc[i..].find(TAIL) {
            let end = i + p;
            i = end + TAIL.len();
            let Some(open) = doc[..end].rfind('(') else { continue };
            let inner = &doc[open + 1..end];
            // "~8k", or "largest file, ~6.9k" — take the last word.
            let largest = inner.contains("largest file");
            let word = inner.rsplit([' ', ',']).next().unwrap_or("").trim();
            let digits = word.trim_start_matches('~');
            let value = match digits.strip_suffix('k') {
                Some(k) => k.parse::<f64>().ok().map(|v| v * 1000.0),
                None => digits.parse::<f64>().ok(),
            };
            // The backticked name immediately before the parenthetical.
            let before = &doc[..open];
            let name = before.rfind('`').and_then(|e| {
                before[..e].rfind('`').map(|s| before[s + 1..e].to_string())
            });
            if let (Some(v), Some(n)) = (value, name) {
                if v > 0.0 {
                    out.push((n, v, largest));
                }
            }
        }
        out
    }

    /// Every `.rs` file under `src/`, as (path, line count).
    fn sources(root: &Path) -> Vec<(PathBuf, usize)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, usize)>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                    if let Ok(s) = std::fs::read_to_string(&p) {
                        out.push((p, s.lines().count()));
                    }
                }
            }
        }
        let mut v = Vec::new();
        walk(&root.join("src"), &mut v);
        v
    }

    #[test]
    fn test_claude_md_line_counts_match_the_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let doc = std::fs::read_to_string(root.join("CLAUDE.md"))
            .expect("CLAUDE.md is missing next to Cargo.toml");
        let files = sources(root);
        let claims = claims(&doc);
        assert!(
            !claims.is_empty(),
            "no `(~N lines)` figure found in CLAUDE.md — either the syntax changed \
             and this scan needs updating, or the figures were removed and so \
             should this test"
        );

        let mut bad: Vec<String> = Vec::new();
        for (name, claimed, largest) in &claims {
            // A path relative to the crate root, else a unique basename.
            let hits: Vec<&(PathBuf, usize)> = if root.join(name).is_file() {
                files.iter().filter(|(p, _)| *p == root.join(name)).collect()
            } else {
                files
                    .iter()
                    .filter(|(p, _)| p.file_name().and_then(|x| x.to_str()) == Some(name.as_str()))
                    .collect()
            };
            let [(path, actual)] = hits[..] else {
                bad.push(format!("`{name}`: names {} files under src/, cannot check", hits.len()));
                continue;
            };
            let actual = *actual as f64;
            let drift = (actual - claimed) / claimed;
            if drift.abs() > 0.10 {
                bad.push(format!(
                    "`{name}` is documented as ~{} lines but has {} ({:+.0}%) — write ~{}",
                    round_k(*claimed),
                    actual as usize,
                    drift * 100.0,
                    round_k(actual)
                ));
            }
            if *largest {
                if let Some((big, n)) = files.iter().max_by_key(|(_, n)| *n) {
                    if big != path {
                        bad.push(format!(
                            "`{name}` is called the largest file, but {} has {n} lines",
                            big.strip_prefix(root).unwrap_or(big).display()
                        ));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "CLAUDE.md size claims are stale:\n  {}", bad.join("\n  "));
    }

    /// "~8k" for 8046, "~3.1k" for 3134, "~950" for 950 — the spelling the
    /// docs already use, so the failure message can be pasted straight in.
    fn round_k(n: f64) -> String {
        if n < 1000.0 {
            return format!("{}", n.round() as usize);
        }
        let k = n / 1000.0;
        if (k - k.round()).abs() < 0.05 {
            format!("{}k", k.round() as usize)
        } else {
            format!("{k:.1}k")
        }
    }
}
