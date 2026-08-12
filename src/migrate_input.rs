// `ccectl migrate-input` — one-time extraction of keybindings from
// config.kdl into the domain-scoped input.kdl.
//
// Reads only; config.kdl is never rewritten. Extracted bindings are merged
// into input.kdl (existing entries win), the previous input.kdl is backed up
// under ~/.config/cce/backups/, and a summary tells the user which config.kdl
// entries are now redundant and can be deleted by hand.
//
// What gets extracted:
//   - root-level / block-form `key_bindings` nodes  → `cce-window-manager`
//   - the `window_manager` section                  → `cce-window-manager`
//   - `style.data.list/tree` search-key props       → `cce-ui`
//
// The brightness spawn binds synthesized from the `display` section are NOT
// migrated — they stay derived from the display config at load time.

use cce_ui::input::{BindingEntry, InputConfig, UI_DOMAIN, WINDOW_MANAGER_DOMAIN};
use cce_window_manager::api::Action;
use cce_window_manager::bindings::parse_chord;

pub struct Extracted {
    pub wm: Vec<BindingEntry>,
    pub ui: Vec<BindingEntry>,
    pub warnings: Vec<String>,
}

fn prop_string(node: &kdl::KdlNode, key: &str) -> Option<String> {
    node.entries()
        .iter()
        .find(|e| e.name().map(|n| n.value()) == Some(key))
        .and_then(|e| e.value().as_string().map(str::to_string))
}

fn first_arg_string(node: &kdl::KdlNode) -> Option<String> {
    node.entries()
        .iter()
        .find(|e| e.name().is_none())
        .and_then(|e| e.value().as_string().map(str::to_string))
}

fn child<'a>(doc: &'a kdl::KdlDocument, name: &str) -> Option<&'a kdl::KdlNode> {
    doc.nodes().iter().find(|n| n.name().value() == name)
}

/// One legacy key_bindings entry (flat node or block child) → a wm-domain
/// BindingEntry, validated against the policy crate's vocabulary.
fn convert_key_binding(node: &kdl::KdlNode, out: &mut Extracted) {
    let mods = prop_string(node, "mods").unwrap_or_default();
    let key = prop_string(node, "key").unwrap_or_default();
    if key.is_empty() {
        out.warnings.push(format!("key_bindings entry without key= skipped: {}", node));
        return;
    }
    let chord = if mods.is_empty() { key } else { format!("{}+{}", mods, key) };
    let action = prop_string(node, "action").unwrap_or_default();

    if Action::from_name(&action).is_none() {
        out.warnings.push(format!("unknown action {:?} skipped (chord {:?})", action, chord));
        return;
    }
    if !valid_chord(&chord) {
        out.warnings.push(format!("invalid chord {:?} skipped (action {:?})", chord, action));
        return;
    }
    out.wm.push(BindingEntry { name: action, chord, command: prop_string(node, "command") });
}

/// A chord is migratable when its modifiers parse AND its key is a real XKB
/// keysym — this rejects gesture names like "swipe_down" that ride in
/// keybind-typed config slots.
fn valid_chord(chord: &str) -> bool {
    match parse_chord(chord) {
        Some(c) => crate::config::parse_keysym(&c.key) != 0,
        None => false,
    }
}

/// Pure extraction pass over config.kdl content.
pub fn extract_from_config(content: &str) -> Result<Extracted, String> {
    let doc: kdl::KdlDocument = content.parse().map_err(|e| format!("{}", e))?;
    let mut out = Extracted { wm: Vec::new(), ui: Vec::new(), warnings: Vec::new() };

    // key_bindings nodes live at the root or nested one level down (the
    // `input` section) — mirror parse_kdl_config and scan both.
    let mut kb_nodes: Vec<&kdl::KdlNode> = Vec::new();
    for node in doc.nodes() {
        if node.name().value() == "key_bindings" {
            kb_nodes.push(node);
        } else if let Some(children) = node.children() {
            kb_nodes.extend(children.nodes().iter().filter(|n| n.name().value() == "key_bindings"));
        }
    }
    for node in kb_nodes {
        match node.children() {
            Some(children) => {
                for c in children.nodes() {
                    convert_key_binding(c, &mut out);
                }
            }
            None => convert_key_binding(node, &mut out),
        }
    }

    if let Some(wm_node) = child(&doc, "window_manager") {
        if let Some(children) = wm_node.children() {
            for (prop, name) in [
                ("close_window", "close_window"),
                ("toggle_fullscreen", "toggle_fullscreen"),
                ("window_switcher", "window_switcher"),
                ("toggle_overview", "overview"),
            ] {
                let Some(c) = child(children, prop) else { continue };
                let Some(chord) = first_arg_string(c) else { continue };
                let normalized = chord.to_lowercase().replace('-', "_");
                if normalized.starts_with("swipe") || normalized.starts_with("pinch") {
                    // A gesture binding (e.g. toggle_overview "swipe_down"),
                    // consumed by the compositor's gesture path — not a
                    // keybind, nothing to migrate.
                    continue;
                }
                if !valid_chord(&chord) {
                    out.warnings.push(format!("window_manager.{}: invalid chord {:?} skipped", prop, chord));
                    continue;
                }
                out.wm.push(BindingEntry { name: name.to_string(), chord, command: None });
            }
        }
    }

    // Widget search keys from style.data.{list,tree} props → cce-ui domain.
    let data = child(&doc, "style").and_then(|n| n.children()).and_then(|c| child(c, "data"));
    if let Some(data) = data {
        let data_children = data.children();
        let list = data_children.and_then(|c| child(c, "list"));
        let tree = data_children.and_then(|c| child(c, "tree"));
        let list_open = list.and_then(|n| prop_string(n, "open_search"));
        let tree_open = tree.and_then(|n| prop_string(n, "open_search"));
        let close = list.and_then(|n| prop_string(n, "close_search"));

        match (&list_open, &tree_open) {
            (Some(l), Some(t)) if l != t => out.warnings.push(format!(
                "list.open_search ({:?}) and tree.open_search ({:?}) differ; migrating the list value — the tree keeps its config.kdl prop",
                l, t
            )),
            _ => {}
        }
        if let Some(chord) = list_open.or(tree_open) {
            out.ui.push(BindingEntry { name: "open_search".into(), chord, command: None });
        }
        if let Some(chord) = close {
            out.ui.push(BindingEntry { name: "close_search".into(), chord, command: None });
        }
    }

    Ok(out)
}

/// Merge extracted entries into a domain's existing list. Existing entries
/// always win: an extracted entry is dropped when its name is already
/// configured (or, for repeatable spawn/toggle, when the same chord is).
pub fn merge_into(existing: &[BindingEntry], extracted: Vec<BindingEntry>) -> (Vec<BindingEntry>, usize) {
    let mut merged = existing.to_vec();
    let mut added = 0;
    for e in extracted {
        let repeatable = e.name == "spawn" || e.name == "toggle";
        let taken = merged.iter().any(|m| {
            if repeatable {
                m.chord == e.chord
            } else {
                m.name == e.name
            }
        });
        if !taken {
            merged.push(e);
            added += 1;
        }
    }
    (merged, added)
}

fn backup(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if !path.exists() {
        return None;
    }
    let backups = cce_ui::config::cce_config_dir().join("backups");
    let _ = std::fs::create_dir_all(&backups);
    for n in 1..1000 {
        let candidate = backups.join(format!("input.kdl.{}.bak", n));
        if !candidate.exists() {
            return std::fs::copy(path, &candidate).ok().map(|_| candidate);
        }
    }
    None
}

pub fn run() {
    let config_path = cce_ui::config::get_config_path();
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot read {}: {}", config_path.display(), e);
            std::process::exit(1);
        }
    };
    let extracted = match extract_from_config(&content) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("cannot parse {}: {}", config_path.display(), e);
            std::process::exit(1);
        }
    };
    for w in &extracted.warnings {
        eprintln!("warning: {}", w);
    }
    if extracted.wm.is_empty() && extracted.ui.is_empty() {
        println!("nothing to migrate: no keybindings found in {}", config_path.display());
        return;
    }

    let input_path = cce_ui::input::get_input_path();
    let existing_content = std::fs::read_to_string(&input_path).unwrap_or_default();
    let existing = match InputConfig::parse(&existing_content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot parse existing {}: {} — fix or remove it first", input_path.display(), e);
            std::process::exit(1);
        }
    };

    let (wm_merged, wm_added) = merge_into(existing.domain(WINDOW_MANAGER_DOMAIN), extracted.wm);
    let (ui_merged, ui_added) = merge_into(existing.domain(UI_DOMAIN), extracted.ui);
    if wm_added == 0 && ui_added == 0 {
        println!("nothing to migrate: input.kdl already covers every config.kdl binding");
        return;
    }

    if let Some(bak) = backup(&input_path) {
        println!("backed up {} -> {}", input_path.display(), bak.display());
    }
    let write = |domain: &str, entries: &[BindingEntry], added: usize| {
        if added == 0 {
            return;
        }
        match cce_ui::input::write_domain(&input_path, domain, entries) {
            Ok(()) => println!("{}: migrated {} binding(s)", domain, added),
            Err(e) => {
                eprintln!("failed to write {}: {}", input_path.display(), e);
                std::process::exit(1);
            }
        }
    };
    write(WINDOW_MANAGER_DOMAIN, &wm_merged, wm_added);
    write(UI_DOMAIN, &ui_merged, ui_added);

    println!();
    println!("wrote {}", input_path.display());
    println!("config.kdl was NOT modified. The migrated `key_bindings` nodes and the");
    println!("`window_manager` section are now shadowed by input.kdl and can be deleted.");
    if ui_added > 0 {
        println!("Migrated widget search keys only take effect once the corresponding");
        println!("style.data.list/tree props are removed from config.kdl (per-widget");
        println!("props stay more specific than cce-ui domain defaults).");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
input {
    accel_speed (f64)1.0
    key_bindings action="spawn" command="cce-cloud --apps" key=(keybind)"super+d"
    key_bindings action="spawn" command="cce control keypress 69" key=(keybind)"super+slash"
}
key_bindings {
    bind action="toggle" command="foot" key=(keybind)"super+t"
    bind action="bogus" key=(keybind)"super+b"
    bind action="spawn" command="x" key=(keybind)"hyper+x"
}
style {
    data {
        list close_search=(keybind)"escape" open_search=(keybind)"/"
        tree open_search=(keybind)"ctrl+f"
    }
}
window_manager {
    close_window (keybind)"super+q"
    toggle_fullscreen (keybind)"super+f"
    window_switcher (keybind)"super+tab"
    toggle_overview (keybind)"swipe_down"
}
"#;

    #[test]
    fn extracts_all_legacy_sources() {
        let x = extract_from_config(CONFIG).unwrap();
        // 2 nested spawns + 1 block toggle + 3 window_manager entries; the
        // unknown action and the bad modifier warn, the gesture value
        // (toggle_overview "swipe_down") is silently left to the gesture path.
        assert_eq!(x.wm.len(), 6);
        assert!(!x.wm.iter().any(|e| e.chord == "swipe_down"));
        assert_eq!(x.warnings.len(), 3); // bogus action, hyper chord, list/tree mismatch
        let spawn = x.wm.iter().find(|e| e.chord == "super+d").unwrap();
        assert_eq!(spawn.name, "spawn");
        assert_eq!(spawn.command.as_deref(), Some("cce-cloud --apps"));
        let toggle = x.wm.iter().find(|e| e.name == "toggle").unwrap();
        assert_eq!(toggle.chord, "super+t");
        assert!(x.wm.iter().any(|e| e.name == "close_window" && e.chord == "super+q"));
        assert!(x.wm.iter().any(|e| e.name == "window_switcher" && e.chord == "super+tab"));
        // list wins the open_search mismatch; close_search comes along.
        assert!(x.ui.iter().any(|e| e.name == "open_search" && e.chord == "/"));
        assert!(x.ui.iter().any(|e| e.name == "close_search" && e.chord == "escape"));
    }

    #[test]
    fn merge_never_overrides_existing() {
        let existing = vec![
            BindingEntry { name: "close_window".into(), chord: "super+w".into(), command: None },
            BindingEntry { name: "spawn".into(), chord: "super+d".into(), command: Some("a".into()) },
        ];
        let extracted = vec![
            // Same name, different chord: dropped (name already configured).
            BindingEntry { name: "close_window".into(), chord: "super+q".into(), command: None },
            // Repeatable, same chord: dropped.
            BindingEntry { name: "spawn".into(), chord: "super+d".into(), command: Some("b".into()) },
            // Repeatable, new chord: added.
            BindingEntry { name: "spawn".into(), chord: "super+t".into(), command: Some("c".into()) },
        ];
        let (merged, added) = merge_into(&existing, extracted);
        assert_eq!(added, 1);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].chord, "super+w");
        assert!(merged.iter().any(|e| e.chord == "super+t"));
    }
}
