//! Reading values out of the KDL tree: `get_child_arg_*`, `get_prop_*`,
//! `get_nested_prop_*`, keyed values, `(mm)` sizes, and `$VAR` expansion. Split
//! out of config.rs on 2026-10-10.

pub(crate) fn expand_env_vars(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() {
            if chars[i + 1] == '{' {
                // ${VAR} form
                if let Some(end) = chars[i + 2..].iter().position(|c| *c == '}') {
                    let var_name: String = chars[i + 2..i + 2 + end].iter().collect();
                    let val = std::env::var(&var_name).unwrap_or_default();
                    result.push_str(&val);
                    i = i + 2 + end + 1; // skip ${VAR}
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            } else if chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_' {
                // $VAR form — name is [A-Za-z_][A-Za-z0-9_]*
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_')
                {
                    end += 1;
                }
                let var_name: String = chars[start..end].iter().collect();
                let val = std::env::var(&var_name).unwrap_or_default();
                result.push_str(&val);
                i = end;
            } else {
                // $ followed by non-identifier char (e.g. $$, $:, $@) — keep as-is
                result.push(chars[i]);
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// A node's keyed values in BOTH KDL spellings, as `(key, entry)`: its
/// properties (`root color="…" blur=0.5 corner_radius=24`) and its child
/// nodes' first positional value (`root { color "…"; blur 0.5;
/// corner_radius 24 }`). The child form is what cce-data-editor writes and
/// what cce-ui's `kdl_to_json` reads; a block parsed from properties alone
/// silently keeps its defaults under it, which is how the compositor ran on
/// a root radius of 12 while every cce-ui app read the config's 24
/// (2026-09-28). Properties first, then children, so a key given both ways
/// takes the child's.
pub(crate) fn node_keyed_values(node: &kdl::KdlNode) -> Vec<(String, kdl::KdlEntry)> {
    let mut out: Vec<(String, kdl::KdlEntry)> = node
        .entries()
        .iter()
        .filter_map(|e| e.name().map(|n| (n.value().to_string(), e.clone())))
        .collect();
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if let Some(e) = child.entries().first() {
                if e.name().is_none() {
                    out.push((child.name().value().to_string(), e.clone()));
                }
            }
        }
    }
    out
}

pub(crate) fn get_child_arg_i64(node: &kdl::KdlNode, child_name: &str, default: i64) -> i64 {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_i64().unwrap_or(default);
                }
            }
        }
    }
    default
}

pub(crate) fn get_child_arg_i64_opt(node: &kdl::KdlNode, child_name: &str) -> Option<i64> {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_i64();
                }
            }
        }
    }
    None
}

pub(crate) fn get_child_arg_f64(node: &kdl::KdlNode, child_name: &str, default: f64) -> f64 {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    let mut val = entry.value().as_f64().unwrap_or(default);
                    if let Some(ty) = entry.ty() {
                        let ty_str = ty.value();
                        if ty_str.starts_with("f64:") {
                            let range_str = ty_str.trim_start_matches("f64:");
                            if let Some(dash_idx) = range_str.find('-') {
                                let min_str = &range_str[..dash_idx].trim();
                                let max_str = &range_str[dash_idx + 1..].trim();
                                if let (Ok(min_f), Ok(max_f)) = (min_str.parse::<f64>(), max_str.parse::<f64>()) {
                                    val = val.clamp(min_f, max_f);
                                }
                            }
                        }
                    }
                    return val;
                }
            }
        }
    }
    default
}

pub(crate) fn get_child_arg_bool(node: &kdl::KdlNode, child_name: &str, default: bool) -> bool {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_bool().unwrap_or(default);
                }
            }
        }
    }
    default
}

pub(crate) fn get_child_arg_string(node: &kdl::KdlNode, child_name: &str, default: &str) -> String {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_string().map(|s| s.to_string()).unwrap_or_else(|| default.to_string());
                }
            }
        }
    }
    default.to_string()
}

pub(crate) fn get_child_arg_bool_opt(node: &kdl::KdlNode, child_name: &str) -> Option<bool> {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_bool();
                }
            }
        }
    }
    None
}

pub(crate) fn get_child_arg_f64_opt(node: &kdl::KdlNode, child_name: &str) -> Option<f64> {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    let mut val = entry.value().as_f64()?;
                    if let Some(ty) = entry.ty() {
                        let ty_str = ty.value();
                        if ty_str.starts_with("f64:") {
                            let range_str = ty_str.trim_start_matches("f64:");
                            if let Some(dash_idx) = range_str.find('-') {
                                let min_str = &range_str[..dash_idx].trim();
                                let max_str = &range_str[dash_idx + 1..].trim();
                                if let (Ok(min_f), Ok(max_f)) = (min_str.parse::<f64>(), max_str.parse::<f64>()) {
                                    val = val.clamp(min_f, max_f);
                                }
                            }
                        }
                    }
                    return Some(val);
                }
            }
        }
    }
    None
}

pub(crate) fn get_child_arg_vec2i_opt(node: &kdl::KdlNode, child_name: &str) -> Option<[i32; 2]> {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                let entries = child.entries();
                if entries.len() >= 2 {
                    let has_tag = entries.first().and_then(|e| e.ty()).map_or(false, |t| t.value() == "vec2i");
                    if has_tag {
                        let x = entries[0].value().as_i64()? as i32;
                        let y = entries[1].value().as_i64()? as i32;
                        return Some([x, y]);
                    }
                }
            }
        }
    }
    None
}


/// All positional string args of a child node, e.g. `rounded_apps "a" "b"`.
/// `Some` when the child node is present (even with no args), `None` when absent.
pub(crate) fn get_child_args_string_vec_opt(node: &kdl::KdlNode, child_name: &str) -> Option<Vec<String>> {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                return Some(
                    child
                        .entries()
                        .iter()
                        .filter(|e| e.name().is_none())
                        .filter_map(|e| e.value().as_string().map(|s| s.to_string()))
                        .collect(),
                );
            }
        }
    }
    None
}

pub(crate) fn get_child_arg_string_opt(node: &kdl::KdlNode, child_name: &str) -> Option<String> {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                if let Some(entry) = child.entries().first() {
                    return entry.value().as_string().map(|s| s.to_string());
                }
            }
        }
    }
    None
}

pub(crate) fn get_prop_string(node: &kdl::KdlNode, key: &str, default: &str) -> String {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_string().map(|s| s.to_string()).unwrap_or_else(|| default.to_string());
            }
        }
    }
    default.to_string()
}

pub(crate) fn get_prop_string_opt(node: &kdl::KdlNode, key: &str) -> Option<String> {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_string().map(|s| s.to_string());
            }
        }
    }
    None
}

pub(crate) fn get_prop_i64(node: &kdl::KdlNode, key: &str, default: i64) -> i64 {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_i64().unwrap_or(default);
            }
        }
    }
    default
}

pub(crate) fn get_prop_bool(node: &kdl::KdlNode, key: &str, default: bool) -> bool {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_bool().unwrap_or(default);
            }
        }
    }
    default
}

pub(crate) fn get_prop_bool_opt(node: &kdl::KdlNode, key: &str) -> Option<bool> {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_bool();
            }
        }
    }
    None
}

pub(crate) fn get_prop_i64_opt(node: &kdl::KdlNode, key: &str) -> Option<i64> {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                return entry.value().as_i64();
            }
        }
    }
    None
}

pub(crate) fn get_prop_f64_opt(node: &kdl::KdlNode, key: &str) -> Option<f64> {
    for entry in node.entries() {
        if let Some(id) = entry.name() {
            if id.value() == key {
                let mut val = entry.value().as_f64()?;
                if let Some(ty) = entry.ty() {
                    let ty_str = ty.value();
                    if ty_str.starts_with("f64:") {
                        let range_str = ty_str.trim_start_matches("f64:");
                        if let Some(dash_idx) = range_str.find('-') {
                            let min_str = &range_str[..dash_idx].trim();
                            let max_str = &range_str[dash_idx + 1..].trim();
                            if let (Ok(min_f), Ok(max_f)) = (min_str.parse::<f64>(), max_str.parse::<f64>()) {
                                val = val.clamp(min_f, max_f);
                            }
                        }
                    }
                }
                return Some(val);
            }
        }
    }
    None
}

pub(crate) fn get_nested_prop_string(node: &kdl::KdlNode, child_name: &str, prop_name: &str, default: &str) -> String {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                for entry in child.entries() {
                    if let Some(id) = entry.name() {
                        if id.value() == prop_name {
                            return entry.value().as_string().map(|s| s.to_string()).unwrap_or_else(|| default.to_string());
                        }
                    }
                }
            }
        }
    }
    default.to_string()
}

pub(crate) fn get_nested_prop_i64(node: &kdl::KdlNode, child_name: &str, prop_name: &str, default: i64) -> i64 {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                for entry in child.entries() {
                    if let Some(id) = entry.name() {
                        if id.value() == prop_name {
                            return entry.value().as_i64().unwrap_or(default);
                        }
                    }
                }
            }
        }
    }
    default
}

pub(crate) fn get_nested_prop_bool(node: &kdl::KdlNode, child_name: &str, prop_name: &str, default: bool) -> bool {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                for entry in child.entries() {
                    if let Some(id) = entry.name() {
                        if id.value() == prop_name {
                            return entry.value().as_bool().unwrap_or(default);
                        }
                    }
                }
            }
        }
    }
    default
}

pub(crate) fn get_nested_prop_f64(node: &kdl::KdlNode, child_name: &str, prop_name: &str, default: f64) -> f64 {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() == child_name {
                for entry in child.entries() {
                    if let Some(id) = entry.name() {
                        if id.value() == prop_name {
                            let mut val = entry.value().as_f64().unwrap_or(default);
                            if let Some(ty) = entry.ty() {
                                let ty_str = ty.value();
                                if ty_str.starts_with("f64:") {
                                    let range_str = ty_str.trim_start_matches("f64:");
                                    if let Some(dash_idx) = range_str.find('-') {
                                        let min_str = &range_str[..dash_idx].trim();
                                        let max_str = &range_str[dash_idx + 1..].trim();
                                        if let (Ok(min_f), Ok(max_f)) = (min_str.parse::<f64>(), max_str.parse::<f64>()) {
                                            val = val.clamp(min_f, max_f);
                                        }
                                    }
                                }
                            }
                            return val;
                        }
                    }
                }
            }
        }
    }
    default
}

/// `"344x215"` (also `344,215` / `344 215`) → (w, h) in mm, both positive.
pub(crate) fn parse_size_mm(s: &str) -> Option<(f64, f64)> {
    let mut it = s.split(|c: char| c == 'x' || c == 'X' || c == ',' || c.is_whitespace()).filter(|p| !p.is_empty());
    let w = it.next()?.trim().parse::<f64>().ok()?;
    let h = it.next()?.trim().parse::<f64>().ok()?;
    (w > 0.0 && h > 0.0 && it.next().is_none()).then_some((w, h))
}
