//! Minimum sizes X11 apps have revealed, kept across restarts.
//!
//! Wine carries no minimum into WM_NORMAL_HINTS for a resizable window, so
//! the compositor learns one from a refusal: during an interactive resize,
//! an app asking for more than it was just given on an axis would not go
//! that small (`xwayland_window::learned_min`). Learned once per launch,
//! every first drag past the minimum snapped back once, so the minimum is
//! stored in `$XDG_STATE_HOME/cce/min-sizes.json` and handed to the window
//! again when it maps.
//!
//! An entry is keyed by app_id, program (argv[0] — every Proton window is
//! `steam_proton`, its program is the exe) and title, since one program's
//! windows have their own minimums. Sizes are X11 pixels, what the app
//! itself measured, so a change of output scale still converts correctly.
//! A width or height of 0 is "not learned".

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MinSize {
    pub app_id: String,
    pub program: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
pub struct MinSizes {
    entries: Vec<MinSize>,
    loaded: bool,
}

fn file_path() -> Option<std::path::PathBuf> {
    crate::config::default_state_path().map(|p| std::path::Path::new(&p).with_file_name("min-sizes.json"))
}

impl MinSizes {
    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Some(path) = file_path() else { return };
        let Ok(json) = std::fs::read_to_string(&path) else { return };
        match serde_json::from_str(&json) {
            Ok(entries) => self.entries = entries,
            Err(e) => log::warn!("min-sizes: ignoring unreadable {}: {}", path.display(), e),
        }
    }

    /// The stored minimum for this window, X11 pixels.
    pub fn get(&mut self, app_id: &str, program: &str, title: &str) -> Option<(u32, u32)> {
        self.ensure_loaded();
        self.entries
            .iter()
            .find(|e| e.app_id == app_id && e.program == program && e.title == title)
            .map(|e| (e.width, e.height))
    }

    /// Store this window's minimum, X11 pixels; written to disk at once
    /// (it changes only on a refusal, a handful of times per app, ever).
    /// 0x0 forgets the entry.
    pub fn set(&mut self, app_id: &str, program: &str, title: &str, width: u32, height: u32) {
        self.ensure_loaded();
        if !update(&mut self.entries, app_id, program, title, width, height) {
            return;
        }
        let Some(path) = file_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self.entries) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    log::error!("min-sizes: failed to write {}: {}", path.display(), e);
                }
            }
            Err(e) => log::error!("min-sizes: failed to serialize: {}", e),
        }
    }
}

/// Apply one entry's new size to the table; whether anything changed.
fn update(entries: &mut Vec<MinSize>, app_id: &str, program: &str, title: &str, width: u32, height: u32) -> bool {
    let pos = entries.iter().position(|e| e.app_id == app_id && e.program == program && e.title == title);
    match (pos, width == 0 && height == 0) {
        (Some(i), true) => {
            entries.remove(i);
            true
        }
        (Some(i), false) => {
            let e = &mut entries[i];
            let changed = (e.width, e.height) != (width, height);
            e.width = width;
            e.height = height;
            changed
        }
        (None, true) => false,
        (None, false) => {
            entries.push(MinSize {
                app_id: app_id.to_string(),
                program: program.to_string(),
                title: title.to_string(),
                width,
                height,
            });
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPC: &str = r"C:\Program Files (x86)\Ubisoft\Ubisoft Game Launcher\upc.exe";

    #[test]
    fn entries_are_added_changed_and_forgotten() {
        let mut t = Vec::new();
        assert!(update(&mut t, "steam_proton", UPC, "Ubisoft Connect", 2428, 0));
        assert!(!update(&mut t, "steam_proton", UPC, "Ubisoft Connect", 2428, 0));
        assert!(update(&mut t, "steam_proton", UPC, "Ubisoft Connect", 2428, 1608));
        // Another window of the same program is its own entry.
        assert!(update(&mut t, "steam_proton", UPC, "Settings", 800, 600));
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].width, t[0].height), (2428, 1608));
        assert!(update(&mut t, "steam_proton", UPC, "Ubisoft Connect", 0, 0));
        assert!(!update(&mut t, "steam_proton", UPC, "Ubisoft Connect", 0, 0));
        assert_eq!(t.len(), 1);
    }
}
