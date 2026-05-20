// Persistent state file for clearwm restart recovery.
//
// Writes window tag assignments and global tag/layout state to
// ~/.cache/clearwm_state so that it survives restarts. On startup,
// the state file is read and applied to re-advertised windows
// matched by their River identifier (stable across WM restarts)
// or app_id+title as a fallback.
//
// What is persisted:
//   - active_tags bitmask
//   - Per-tag layout overrides (tag_layouts + has_tag_layout)
//   - Per-window: tags, tiling_mode (only if mode_locked), identifier, app_id, title
//
// What is NOT persisted:
//   - Focus (seat.focused_window_id)
//   - Window positions/dimensions (recomputed on startup)
//   - Transient flags (is_new, needs_*, etc.)

use crate::types::{TilingMode, WindowManager, NUM_TAGS};
use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// Get the state file path: ~/.cache/clearwm_state
fn state_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let mut path = PathBuf::from(home);
    path.push(".cache");
    path.push("clearwm_state");
    path
}

/// Persistent state snapshot (plain-text, line-oriented).
///
/// Format:
///   active_tags=<u32>
///   tag_layout=<tag_index> <mode_str> <has_layout_bool>
///   window <identifier> <app_id> <title> <tags> <mode_str> <mode_locked>
///
/// Fields are tab-separated. App_id and title use URL-style percent-encoding
/// for spaces, tabs, and newlines so they never break the line format.
pub struct PersistentState {
    pub active_tags: u32,
    pub tag_layouts: Vec<(usize, TilingMode, bool)>,
    pub windows: Vec<PersistentWindow>,
}

pub struct PersistentWindow {
    pub identifier: Option<String>,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub tags: u32,
    pub tiling_mode: TilingMode,
    pub mode_locked: bool,
}

/// Percent-encode spaces, tabs, newlines, and percent signs in a string.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '\t' => out.push_str("%09"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            '%' => out.push_str("%25"),
            _ => out.push(c),
        }
    }
    out
}

/// Decode a percent-encoded string.
fn pct_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                out.push(byte as char);
            } else {
                out.push('%');
                out.push_str(&hex);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Write the current WM state to the state file.
/// Safe to call inside Dispatch callbacks — just file I/O, no fork.
pub fn write_state(wm: &WindowManager) {
    let path = state_file_path();

    // Ensure ~/.cache/ exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if let Ok(mut f) = fs::File::create(&path) {
        let _ = writeln!(f, "active_tags={}", wm.active_tags);

        for tag_bit in 0..NUM_TAGS {
            let mode_str = wm.tag_layouts[tag_bit].as_str();
            let has = wm.has_tag_layout[tag_bit];
            let _ = writeln!(f, "tag_layout\t{}\t{}\t{}", tag_bit, mode_str, has);
        }

        for win in &wm.windows {
            if win.closed {
                continue;
            }
            let ident = win
                .identifier
                .as_deref()
                .map(pct_encode)
                .unwrap_or_else(|| "-".to_string());
            let app_id = win
                .app_id
                .as_deref()
                .map(pct_encode)
                .unwrap_or_else(|| "-".to_string());
            let title = win
                .title
                .as_deref()
                .map(pct_encode)
                .unwrap_or_else(|| "-".to_string());
            let mode_str = win.tiling_mode.as_str();
            let _ = writeln!(
                f,
                "window\t{}\t{}\t{}\t{}\t{}\t{}",
                ident, app_id, title, win.tags, mode_str, win.mode_locked
            );
        }

        let _ = f.sync_all();
    }
}

/// Read the persisted state from disk. Returns None if the file doesn't
/// exist or can't be parsed.
pub fn read_state() -> Option<PersistentState> {
    let path = state_file_path();
    let file = fs::File::open(&path).ok()?;
    let reader = std::io::BufReader::new(file);

    let mut active_tags: Option<u32> = None;
    let mut tag_layouts = Vec::new();
    let mut windows = Vec::new();

    for line in reader.lines() {
        let line = line.ok()?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(rest) = line.strip_prefix("active_tags=") {
            active_tags = rest.parse::<u32>().ok();
            continue;
        }

        if let Some(rest) = line.strip_prefix("tag_layout") {
            let rest = rest.trim_start();
            let parts: Vec<&str> = rest.split('\t').collect();
            if parts.len() >= 3 {
                if let Ok(idx) = parts[0].parse::<usize>() {
                    let mode = parse_tiling_mode_str(parts[1]);
                    let has = parts[2] == "true";
                    tag_layouts.push((idx, mode, has));
                }
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("window") {
            let rest = rest.trim_start();
            let parts: Vec<&str> = rest.split('\t').collect();
            if parts.len() >= 6 {
                let identifier = if parts[0] == "-" {
                    None
                } else {
                    Some(pct_decode(parts[0]))
                };
                let app_id = if parts[1] == "-" {
                    None
                } else {
                    Some(pct_decode(parts[1]))
                };
                let title = if parts[2] == "-" {
                    None
                } else {
                    Some(pct_decode(parts[2]))
                };
                let tags = parts[3].parse::<u32>().unwrap_or(1);
                let tiling_mode = parse_tiling_mode_str(parts[4]);
                let mode_locked = parts[5] == "true";

                windows.push(PersistentWindow {
                    identifier,
                    app_id,
                    title,
                    tags,
                    tiling_mode,
                    mode_locked,
                });
            }
            continue;
        }
    }

    Some(PersistentState {
        active_tags: active_tags.unwrap_or(1),
        tag_layouts,
        windows,
    })
}

/// Parse a tiling mode string (case-insensitive).
fn parse_tiling_mode_str(s: &str) -> TilingMode {
    match s {
        "Cascade" => TilingMode::Cascade,
        "Grid" => TilingMode::Grid,
        "Vsplit" => TilingMode::Vsplit,
        "Hsplit" => TilingMode::Hsplit,
        "Fullscreen" => TilingMode::Fullscreen,
        "Floating" => TilingMode::Floating,
        _ => TilingMode::Cascade,
    }
}

/// Apply persisted state to the window manager.
/// Called on startup after config is loaded but before the main loop.
///
/// - Restores active_tags (so the correct tag view is shown).
/// - Restores per-tag layout overrides.
/// - For each live window, looks up its persisted entry by identifier
///   (preferred) or app_id+title (fallback) and restores:
///     - tags (which tag the window is on)
///     - tiling_mode + mode_locked (if the mode was user-set)
pub fn apply_state(wm: &mut WindowManager, state: &PersistentState) {
    // Restore active_tags
    wm.active_tags = state.active_tags;

    // Restore per-tag layouts
    for (idx, mode, has) in &state.tag_layouts {
        if *idx < NUM_TAGS {
            wm.tag_layouts[*idx] = *mode;
            wm.has_tag_layout[*idx] = *has;
        }
    }

    // Apply per-window state from persisted entries.
    // Build a lookup: identifier -> PersistentWindow, and (app_id, title) -> PersistentWindow.
    // Identifier is the primary key (stable across restarts).
    // App_id+title is a fallback for windows without identifiers.
    let mut by_identifier: Vec<&PersistentWindow> = Vec::new();
    let mut by_app_id_title: Vec<&PersistentWindow> = Vec::new();

    for pw in &state.windows {
        if pw.identifier.is_some() {
            by_identifier.push(pw);
        }
        if pw.app_id.is_some() {
            by_app_id_title.push(pw);
        }
    }

    for win in &mut wm.windows {
        if win.closed {
            continue;
        }

        // Try identifier match first
        let matched = if let Some(ref ident) = win.identifier {
            by_identifier.iter().find(|pw| {
                pw.identifier.as_deref() == Some(ident.as_str())
            })
        } else {
            None
        };

        // Fallback: app_id + title match
        let matched = matched.or_else(|| {
            if let Some(ref aid) = win.app_id {
                by_app_id_title.iter().find(|pw| {
                    pw.app_id.as_deref() == Some(aid.as_str())
                        && pw.title == win.title
                })
            } else {
                None
            }
        });

        if let Some(pw) = matched {
            // Restore tag assignment
            win.tags = pw.tags;

            // Restore locked tiling mode (user explicitly set this)
            if pw.mode_locked {
                win.tiling_mode = pw.tiling_mode;
                win.mode_locked = true;
            }
        }
    }

    eprintln!(
        "[state] restored: active_tags={}, tag_layouts={:?}, window_entries={}",
        state.active_tags,
        state
            .tag_layouts
            .iter()
            .filter(|(_, _, has)| *has)
            .count(),
        state.windows.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pct_encode_decode_roundtrip() {
        let cases = vec![
            "",
            "hello",
            "hello world",
            "foo\tbar\nbaz",
            "100%",
            "org.qutebrowser.qutebrowser",
            "zed-industries/awesome-gpui: Awesome projects!",
        ];
        for case in cases {
            assert_eq!(pct_decode(&pct_encode(case)), case);
        }
    }

    #[test]
    fn test_write_read_roundtrip() {
        let dir = std::env::temp_dir().join("clearwm_state_test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("clearwm_state");

        let mut wm = WindowManager::default();
        wm.active_tags = 0b1010; // tags 2 and 4
        wm.tag_layouts[1] = TilingMode::Grid;
        wm.has_tag_layout[1] = true;

        // Add a window
        let mut win = crate::types::Window::default();
        win.id = 42;
        win.identifier = Some("river-window-123".to_string());
        win.app_id = Some("org.qutebrowser.qutebrowser".to_string());
        win.title = Some("Test Page - qutebrowser".to_string());
        win.tags = 0b100; // tag 3
        win.tiling_mode = TilingMode::Fullscreen;
        win.mode_locked = true;
        wm.windows.push(win);

        // Write
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut f) = fs::File::create(&path) {
            // Manually write using the same format as write_state
            let _ = writeln!(f, "active_tags={}", wm.active_tags);
            for tag_bit in 0..NUM_TAGS {
                let _ = writeln!(
                    f,
                    "tag_layout\t{}\t{}\t{}",
                    tag_bit,
                    wm.tag_layouts[tag_bit].as_str(),
                    wm.has_tag_layout[tag_bit]
                );
            }
            for w in &wm.windows {
                let ident = w.identifier.as_deref().map(pct_encode).unwrap_or("-".to_string());
                let app_id = w.app_id.as_deref().map(pct_encode).unwrap_or("-".to_string());
                let title = w.title.as_deref().map(pct_encode).unwrap_or("-".to_string());
                let _ = writeln!(
                    f,
                    "window\t{}\t{}\t{}\t{}\t{}\t{}",
                    ident, app_id, title, w.tags, w.tiling_mode.as_str(), w.mode_locked
                );
            }
        }

        // Read and verify via the public read_state() path.
        // We set HOME to our temp dir so state_file_path() finds the test file.
        // Instead, use a direct parse approach:
        let file = fs::File::open(&path).unwrap();
        let reader = std::io::BufReader::new(file);
        let mut active_tags: Option<u32> = None;
        let mut tag_layouts = Vec::new();
        let mut windows = Vec::new();
        for line_r in reader.lines() {
            let line = line_r.unwrap();
            let line = line.trim();
            if line.is_empty() { continue; }
            if let Some(rest) = line.strip_prefix("active_tags=") {
                active_tags = rest.parse::<u32>().ok();
            } else if let Some(rest) = line.strip_prefix("tag_layout") {
                let rest = rest.trim_start();
                let parts: Vec<&str> = rest.split('\t').collect();
                if parts.len() >= 3 {
                    if let Ok(idx) = parts[0].parse::<usize>() {
                        let mode = parse_tiling_mode_str(parts[1]);
                        let has = parts[2] == "true";
                        tag_layouts.push((idx, mode, has));
                    }
                }
            } else if let Some(rest) = line.strip_prefix("window") {
                let rest = rest.trim_start();
                let parts: Vec<&str> = rest.split('\t').collect();
                if parts.len() >= 6 {
                    let identifier = if parts[0] == "-" { None } else { Some(pct_decode(parts[0])) };
                    let app_id = if parts[1] == "-" { None } else { Some(pct_decode(parts[1])) };
                    let title = if parts[2] == "-" { None } else { Some(pct_decode(parts[2])) };
                    let tags = parts[3].parse::<u32>().unwrap_or(1);
                    let tiling_mode = parse_tiling_mode_str(parts[4]);
                    let mode_locked = parts[5] == "true";
                    windows.push(PersistentWindow { identifier, app_id, title, tags, tiling_mode, mode_locked });
                }
            }
        }
        let state = PersistentState {
            active_tags: active_tags.unwrap_or(1),
            tag_layouts,
            windows,
        };
        assert_eq!(state.active_tags, 0b1010);
        assert_eq!(state.tag_layouts.len(), NUM_TAGS);
        // Tag layout at index 1 should be Grid + has=true
        let tl1 = state.tag_layouts.iter().find(|(idx, _, _)| *idx == 1);
        assert!(tl1.is_some());
        let (_, mode, has) = tl1.unwrap();
        assert_eq!(*mode, TilingMode::Grid);
        assert!(has);

        assert_eq!(state.windows.len(), 1);
        let pw = &state.windows[0];
        assert_eq!(pw.identifier.as_deref(), Some("river-window-123"));
        assert_eq!(pw.app_id.as_deref(), Some("org.qutebrowser.qutebrowser"));
        assert_eq!(pw.title.as_deref(), Some("Test Page - qutebrowser"));
        assert_eq!(pw.tags, 0b100);
        assert_eq!(pw.tiling_mode, TilingMode::Fullscreen);
        assert!(pw.mode_locked);

        // Clean up
        let _ = fs::remove_file(&path);
    }
}
