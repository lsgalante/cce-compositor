// SPDX-License-Identifier: GPL-3.0-only
//! Portal global shortcuts — chords the compositor eats on behalf of the
//! `org.freedesktop.impl.portal.GlobalShortcuts` backend (`cce-shortcuts-portal`).
//!
//! A native Wayland client cannot grab keys; what it can do is ask the
//! desktop portal to bind a trigger for it (1Password's Quick Access does
//! exactly this). The portal frontend forwards that to a backend, and the
//! backend forwards it here over the control socket:
//!
//! ```text
//! shortcut bind <session> <id> <trigger>   -> ok <trigger_description> | error: …
//! shortcut unbind <session> [<id>]         -> ok
//! shortcut clear                           -> ok
//! shortcut list                            -> <session> <id> <trigger_description> per line
//! ```
//!
//! `<session>` is the portal's session object path (no whitespace, so it is
//! one token) and `<trigger>` is the shortcuts-spec string the app supplied
//! (`CTRL+SHIFT+space`). A bound chord is matched in `handle_group_key`
//! AFTER the builtins and the user's own keybinds — the user's config always
//! wins, and a bind for a chord the config already uses is refused rather
//! than silently shadowed, so the app is told it did not get it. Press and
//! release are reported as one-shot lines on the status socket's
//! `shortcuts` topic (`activated|deactivated <session> <id> <time_msec>`),
//! which is where the backend turns them into the portal's `Activated` /
//! `Deactivated` signals. The compositor never learns which app asked; the
//! session path is the only identity it carries.
//!
//! The table is process state, not config: nothing here is persisted, and a
//! backend that starts fresh sends `clear` first so a bind left by a dead
//! predecessor cannot keep eating a chord nobody listens for.

use crate::window_manager::WindowManager;

/// One bound chord. `mods` is the wlr modifier mask (`config::parse_modifiers`
/// values) and `keysym` an xkb keysym, exactly what `Keybind` carries, so the
/// same matcher serves both tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortalShortcut {
    pub session: String,
    pub id: String,
    pub mods: u32,
    pub keysym: u32,
    /// The `trigger_description` handed back to the app: `Ctrl+Shift+Space`.
    pub description: String,
}

/// A parsed shortcuts-spec trigger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub mods: u32,
    pub keysym: u32,
    pub description: String,
}

const MOD_SHIFT: u32 = 0x01;
const MOD_CTRL: u32 = 0x04;
const MOD_ALT: u32 = 0x08;
const MOD_LOGO: u32 = 0x40;

/// Parse a shortcuts-spec trigger: modifiers and one key joined by `+`, the
/// modifiers being `CTRL`, `ALT`, `SHIFT` and `LOGO` (case-insensitive;
/// `SUPER` and `META` are taken as `LOGO` since apps do write them) and the
/// key an xkb keysym name (`space`, `F5`, `q`). A trailing `+` is the plus
/// key itself, as in `CTRL++`.
pub fn parse_trigger(s: &str) -> Result<Trigger, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty trigger".into());
    }
    // `CTRL++` splits as ["CTRL", "", ""]: an empty last part after a `+`
    // means the key is `+` itself.
    let mut parts: Vec<&str> = s.split('+').collect();
    let key = match parts.pop() {
        Some("") if s.ends_with('+') => {
            // Drop the empty part before it too (the one between the two
            // plus signs), leaving just the modifiers.
            parts.pop();
            "plus"
        }
        Some(k) => k,
        None => return Err("empty trigger".into()),
    };
    let mut mods = 0u32;
    for m in parts {
        let bit = match m.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => MOD_CTRL,
            "ALT" => MOD_ALT,
            "SHIFT" => MOD_SHIFT,
            "LOGO" | "SUPER" | "META" => MOD_LOGO,
            "" => return Err(format!("empty modifier in {s:?}")),
            other => return Err(format!("unknown modifier {other:?}")),
        };
        mods |= bit;
    }
    let keysym: u32 = xkbcommon::xkb::keysym_from_name(key, xkbcommon::xkb::KEYSYM_CASE_INSENSITIVE).into();
    if keysym == 0 {
        return Err(format!("unknown key {key:?}"));
    }
    if unsafe { crate::keyboard::keysym_is_modifier(keysym) } {
        return Err(format!("{key:?} is a modifier, not a key"));
    }
    Ok(Trigger { mods, keysym, description: describe(mods, keysym) })
}

/// Human form for the app to render: `Ctrl+Shift+Space`. Modifier order is
/// fixed regardless of how the trigger was written.
fn describe(mods: u32, keysym: u32) -> String {
    let mut out = Vec::new();
    if mods & MOD_CTRL != 0 {
        out.push("Ctrl".to_string());
    }
    if mods & MOD_ALT != 0 {
        out.push("Alt".to_string());
    }
    if mods & MOD_SHIFT != 0 {
        out.push("Shift".to_string());
    }
    if mods & MOD_LOGO != 0 {
        out.push("Super".to_string());
    }
    let name = xkbcommon::xkb::keysym_get_name(xkbcommon::xkb::Keysym::new(keysym));
    // Single letters read better upper-case; multi-letter names (`space`,
    // `Return`, `F5`) get an initial capital and are otherwise left alone.
    let mut chars = name.chars();
    let pretty = match chars.next() {
        Some(c) if name.chars().count() == 1 => c.to_uppercase().collect::<String>(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => name.clone(),
    };
    out.push(pretty);
    out.join("+")
}

/// The `shortcut …` control-socket command. `args` is everything after the
/// word `shortcut`.
pub fn ipc(wm: &mut WindowManager, args: &[&str]) -> String {
    match args.first().copied() {
        Some("bind") => {
            let [_, session, id, trigger] = args else {
                return "error: usage: shortcut bind <session> <id> <trigger>\n".to_string();
            };
            let t = match parse_trigger(trigger) {
                Ok(t) => t,
                Err(e) => return format!("error: {e}\n"),
            };
            // The user's config owns its chords: a portal bind never
            // shadows one, and the app hears that it was refused.
            if wm.keybinds.iter().any(|kb| kb.mods == t.mods && kb.keysym == t.keysym) {
                return format!("error: {} is a compositor keybind\n", t.description);
            }
            if let Some(other) = wm
                .portal_shortcuts
                .iter()
                .find(|s| s.mods == t.mods && s.keysym == t.keysym && !(s.session == *session && s.id == *id))
            {
                return format!("error: {} is already bound by {} {}\n", t.description, other.session, other.id);
            }
            // Re-binding the same (session, id) replaces its chord.
            wm.portal_shortcuts.retain(|s| !(s.session == *session && s.id == *id));
            log::info!("[shortcut] bind {} {} -> {}", session, id, t.description);
            wm.portal_shortcuts.push(PortalShortcut {
                session: session.to_string(),
                id: id.to_string(),
                mods: t.mods,
                keysym: t.keysym,
                description: t.description.clone(),
            });
            format!("ok {}\n", t.description)
        }
        Some("unbind") => {
            let (session, id) = match args {
                [_, session] => (*session, None),
                [_, session, id] => (*session, Some(*id)),
                _ => return "error: usage: shortcut unbind <session> [<id>]\n".to_string(),
            };
            let before = wm.portal_shortcuts.len();
            wm.portal_shortcuts.retain(|s| !(s.session == session && id.map_or(true, |id| s.id == id)));
            log::info!("[shortcut] unbind {} {}: {} removed", session, id.unwrap_or("*"), before - wm.portal_shortcuts.len());
            "ok\n".to_string()
        }
        Some("clear") => {
            log::info!("[shortcut] clear: {} removed", wm.portal_shortcuts.len());
            wm.portal_shortcuts.clear();
            "ok\n".to_string()
        }
        Some("list") => {
            let mut out = String::new();
            for s in &wm.portal_shortcuts {
                out.push_str(&format!("{} {} {}\n", s.session, s.id, s.description));
            }
            out
        }
        _ => "error: usage: shortcut bind|unbind|clear|list\n".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spec_triggers() {
        let t = parse_trigger("CTRL+SHIFT+space").unwrap();
        assert_eq!(t.mods, MOD_CTRL | MOD_SHIFT);
        assert_eq!(t.keysym, u32::from(xkbcommon::xkb::keysyms::KEY_space));
        assert_eq!(t.description, "Ctrl+Shift+Space");
    }

    #[test]
    fn modifier_spelling_is_lenient_and_order_fixed() {
        let a = parse_trigger("shift+logo+q").unwrap();
        let b = parse_trigger("SUPER+SHIFT+Q").unwrap();
        assert_eq!(a.mods, b.mods);
        assert_eq!(a.keysym, b.keysym, "keysym lookup is case-insensitive");
        assert_eq!(a.description, "Shift+Super+Q");
    }

    #[test]
    fn plus_key_and_bare_key() {
        assert_eq!(parse_trigger("CTRL++").unwrap().description, "Ctrl+Plus");
        let f5 = parse_trigger("F5").unwrap();
        assert_eq!(f5.mods, 0);
        assert_eq!(f5.description, "F5");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_trigger("").is_err());
        assert!(parse_trigger("HYPER+a").is_err());
        assert!(parse_trigger("CTRL+nosuchkey").is_err());
        assert!(parse_trigger("CTRL+Shift_L").is_err(), "a lone modifier key is not a chord");
    }
}
