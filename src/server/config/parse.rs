//! `parse_kdl_config`: config.kdl text into a `Config`, block by block. Split out
//! of config.rs on 2026-10-10.

use super::*;

pub(crate) fn parse_kdl_config(content: &str) -> Result<Config, String> {
    let doc: kdl::KdlDocument = content.parse().map_err(|e| format!("KDL parse error: {}", e))?;
    
    // 1. layout & style
    let mut layout = LayoutConfig::default();
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "layout") {
        layout.gap = get_child_arg_i64(node, "gap", default_gap());
        layout.gap_top = get_child_arg_i64(node, "gap_top", default_gap_top());
        layout.gap_left = get_child_arg_i64(node, "gap_left", default_gap_left());
        layout.gap_right = get_child_arg_i64(node, "gap_right", default_gap_right());
        layout.gap_bottom = get_child_arg_i64(node, "gap_bottom", default_gap_bottom());
        layout.cascade_offset = get_child_arg_i64(node, "cascade_offset", default_cascade_offset());
        layout.bar_height = get_child_arg_i64(node, "bar_height", default_bar_height());
        layout.grid_gap = get_child_arg_i64(node, "grid_gap", default_grid_gap());
    }
    
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "style") {
        layout.transition_duration = get_nested_prop_i64(node, "window", "transition_duration", default_transition_duration());
        layout.window_backdrop_blur_ignore_transparent = get_nested_prop_bool(node, "window", "backdrop_blur_ignore_transparent", default_window_backdrop_blur_ignore_transparent());
        
        layout.overlay_behavior = get_nested_prop_string(node, "overlay", "behavior", &default_overlay_behavior());
        layout.overlay_width = get_nested_prop_i64(node, "overlay", "width", default_overlay_width());
        layout.overlay_position = get_nested_prop_string(node, "overlay", "position", &default_overlay_position());
        layout.overlay_border_gap = get_nested_prop_i64(node, "overlay", "border_gap", default_overlay_border_gap());
        
        layout.status_normal_color = get_nested_prop_string(node, "status", "normal_color", &default_status_normal_color());
        layout.status_background_blur = get_nested_prop_f64(node, "status", "background_blur", default_status_background_blur());
        layout.status_backdrop_blur_ignore_transparent = get_nested_prop_bool(node, "status", "backdrop_blur_ignore_transparent", default_status_backdrop_blur_ignore_transparent());
        layout.status_module_hide_mode_preview = get_nested_prop_i64(node, "status", "module_hide_mode_preview", default_status_module_hide_mode_preview());
        layout.status_module_spacing = get_nested_prop_i64(node, "status", "module_spacing", default_status_module_spacing());
    }

    // The status bar's own config file wins over the shared status keys:
    // ~/.config/cce/cce-status-interface/config.kdl, `module { spacing height }`.
    // Re-read on every config (re)load, so `ccectl reload` picks up edits.
    {
        let app_cfg = cce_core::config::get_app_config_path("cce-status-interface");
        if let Ok(content) = std::fs::read_to_string(&app_cfg) {
            if let Ok(app_doc) = content.parse::<kdl::KdlDocument>() {
                if let Some(module) = app_doc.nodes().iter().find(|n| n.name().value() == "module") {
                    // A module key in either KDL spelling: `spacing=(f64)12`
                    // prop on the module node, or a `spacing 12` child node.
                    let module_i64 = |key: &str| -> Option<i64> {
                        module
                            .entries()
                            .iter()
                            .find(|e| e.name().map(|id| id.value()) == Some(key))
                            .map(|e| e.value())
                            .or_else(|| {
                                module.children().and_then(|c| {
                                    c.nodes()
                                        .iter()
                                        .find(|n| n.name().value() == key)
                                        .and_then(|n| n.entries().first().map(|e| e.value()))
                                })
                            })
                            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)))
                    };
                    if let Some(i) = module_i64("spacing") {
                        layout.status_module_spacing = i;
                    }
                    if let Some(i) = module_i64("height") {
                        layout.bar_height = i;
                    }
                    // The droplet spec string (presence enables the style;
                    // empty = all defaults). Same either-spelling lookup.
                    let module_str = |key: &str| -> Option<String> {
                        module
                            .entries()
                            .iter()
                            .find(|e| e.name().map(|id| id.value()) == Some(key))
                            .map(|e| e.value())
                            .or_else(|| {
                                module.children().and_then(|c| {
                                    c.nodes()
                                        .iter()
                                        .find(|n| n.name().value() == key)
                                        .and_then(|n| n.entries().first().map(|e| e.value()))
                                })
                            })
                            .and_then(|v| v.as_string().map(|s| s.to_string()))
                    };
                    layout.status_droplet = module_str("droplet");
                    // Backdrop compression: the contrast ratio the text must
                    // hold, against the text color the bar draws in (its
                    // own fallback is the shared status normal_color).
                    let module_f64 = |key: &str| -> Option<f64> {
                        module
                            .entries()
                            .iter()
                            .find(|e| e.name().map(|id| id.value()) == Some(key))
                            .map(|e| e.value())
                            .or_else(|| {
                                module.children().and_then(|c| {
                                    c.nodes()
                                        .iter()
                                        .find(|n| n.name().value() == key)
                                        .and_then(|n| n.entries().first().map(|e| e.value()))
                                })
                            })
                            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                    };
                    let text_color = module_str("text_color")
                        .unwrap_or_else(|| layout.status_normal_color.clone());
                    layout.status_backdrop_compress = module_f64("backdrop_compress")
                        .and_then(|ratio| backdrop_compress_params(&text_color, ratio));
                }
            }
        }
    }

    // 2. env
    let mut env = HashMap::new();
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "env") {
        if let Some(children) = node.children() {
            for child in children.nodes() {
                if let Some(entry) = child.entries().first() {
                    if let Some(val) = entry.value().as_string() {
                        env.insert(child.name().value().to_string(), val.to_string());
                    }
                }
            }
        }
    }

    // 3. lists
    let mut key_bindings = Vec::new();
    let mut pointer_bind = Vec::new();
    let mut gesture_bind = Vec::new();
    let mut mode_rule = Vec::new();
    let mut tag_layout = Vec::new();
    let mut startup = Vec::new();
    let mut device = Vec::new();
    
    for node in doc.nodes() {
        match node.name().value() {
            "key_bindings" => {
                if let Some(children) = node.children() {
                    for child in children.nodes() {
                        if child.name().value() == "bind" {
                            let mods = get_prop_string(child, "mods", "");
                            let key = get_prop_string(child, "key", "");
                            let action = get_prop_string(child, "action", "");
                            let command = get_prop_string_opt(child, "command");
                            key_bindings.push(KeybindConfig { mods, key, action, command });
                        }
                    }
                } else {
                    let mods = get_prop_string(node, "mods", "");
                    let key = get_prop_string(node, "key", "");
                    let action = get_prop_string(node, "action", "");
                    let command = get_prop_string_opt(node, "command");
                    key_bindings.push(KeybindConfig { mods, key, action, command });
                }
            }
            "pointer_bind" => {
                let mods = get_prop_string(node, "mods", "");
                let button = get_prop_string(node, "button", "");
                let action = get_prop_string(node, "action", "");
                pointer_bind.push(PointerBindConfig { mods, button, action });
            }
            "gesture_bind" => {
                let mods = get_prop_string_opt(node, "mods");
                let gesture_type = get_prop_string(node, "type", "");
                let fingers = get_prop_i64(node, "fingers", 0) as u32;
                let direction = get_prop_string(node, "direction", "");
                let action = get_prop_string(node, "action", "");
                let command = get_prop_string_opt(node, "command");
                gesture_bind.push(GestureBindConfig { mods, gesture_type, fingers, direction, action, command });
            }
            "mode_rule" => {
                let mode = get_prop_string(node, "mode", "");
                let app_id = get_prop_string(node, "app_id", "");
                let title = get_prop_string_opt(node, "title");
                let single = get_prop_bool_opt(node, "single");
                let tag = get_prop_i64_opt(node, "tag");
                let circular = get_prop_bool_opt(node, "circular");
                let ssd = get_prop_bool_opt(node, "ssd");
                let over_sibling = get_prop_bool_opt(node, "over_sibling");
                let center = get_prop_bool_opt(node, "center");
                mode_rule.push(ModeRuleConfig { mode, app_id, title, single, tag, circular, ssd, over_sibling, center });
            }
            "tag_layout" => {
                let tag = get_prop_i64(node, "tag", 0);
                let mode = get_prop_string(node, "mode", "");
                tag_layout.push(TagLayoutConfig { tag, mode });
            }
            "startup" => {
                let exec = get_prop_string(node, "exec", "");
                let once = get_prop_bool(node, "once", false);
                let restart = get_prop_bool(node, "restart", false);
                startup.push(StartupConfig { exec, once, restart });
            }
            "device" => {
                let name = get_prop_string(node, "name", "");
                let scroll_factor = get_prop_f64_opt(node, "scroll_factor");
                device.push(InputDeviceConfigRule { name, scroll_factor });
            }
            _ => {}
        }
    }

    if key_bindings.is_empty() {
        if let Some(input_node) = doc.nodes().iter().find(|n| n.name().value() == "input") {
            if let Some(children) = input_node.children() {
                for child_node in children.nodes() {
                    if child_node.name().value() == "key_bindings" {
                        if let Some(bind_children) = child_node.children() {
                            for child in bind_children.nodes() {
                                if child.name().value() == "bind" {
                                    let mods = get_prop_string(child, "mods", "");
                                    let key = get_prop_string(child, "key", "");
                                    let action = get_prop_string(child, "action", "");
                                    let command = get_prop_string_opt(child, "command");
                                    key_bindings.push(KeybindConfig { mods, key, action, command });
                                }
                            }
                        } else {
                            let mods = get_prop_string(child_node, "mods", "");
                            let key = get_prop_string(child_node, "key", "");
                            let action = get_prop_string(child_node, "action", "");
                            let command = get_prop_string_opt(child_node, "command");
                            key_bindings.push(KeybindConfig { mods, key, action, command });
                        }
                    }
                }
            }
        }
    }

    // 3b. idle timeouts
    let mut idle = crate::idle::IdleConfig::default();
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "idle") {
        idle.display_off_s = get_child_arg_i64(node, "display_off", 0);
        idle.sleep_s = get_child_arg_i64(node, "sleep", 0);
        idle.sleep_command = get_child_arg_string_opt(node, "sleep_command");
    }

    // 4. output
    let mut output = None;
    let mut display = HashMap::new();
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "output") {
        let scenefx_optimized_blur = get_child_arg_bool(node, "scenefx_optimized_blur", true);
        output = Some(OutputConfig { scenefx_optimized_blur });

        if let Some(children) = node.children() {
            for child in children.nodes() {
                let name = child.name().value();
                let mut parsed_scale = None;
                if let Some(entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("scale")) {
                    if let Some(num) = entry.value().as_f64() {
                        parsed_scale = Some(num);
                    }
                }
                // `size_mm="344x215"`: the panel's real size, overriding
                // the EDID figure the backend read (TVs and projectors
                // lie; some panels report nothing). Forwarded into the
                // wl_output geometry every client sees, so cce-ui's metric
                // measures against it.
                let mut parsed_size_mm = None;
                if let Some(entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("size_mm")) {
                    parsed_size_mm = entry.value().as_string().and_then(parse_size_mm);
                }
                let mut parsed_interval = None;
                if let Some(entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("brightness_interval")) {
                    if let Some(num) = entry.value().as_i64() {
                        parsed_interval = Some(num);
                    }
                }
                let mut parsed_up = None;
                if let Some(entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("brightness_up")) {
                    if let Some(key_val) = entry.value().as_string() {
                        parsed_up = Some(key_val.to_string());
                    }
                }
                let mut parsed_down = None;
                if let Some(entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("brightness_down")) {
                    if let Some(key_val) = entry.value().as_string() {
                        parsed_down = Some(key_val.to_string());
                    }
                }

                if let Some(display_children) = child.children() {
                    if parsed_scale.is_none() {
                        if let Some(scale_node) = display_children.nodes().iter().find(|n| n.name().value() == "scale") {
                            if let Some(entry) = scale_node.entries().first() {
                                if let Some(num) = entry.value().as_f64() {
                                    parsed_scale = Some(num);
                                }
                            }
                        }
                    }
                    if parsed_size_mm.is_none() {
                        if let Some(size_node) = display_children.nodes().iter().find(|n| n.name().value() == "size_mm") {
                            if let Some(entry) = size_node.entries().first() {
                                parsed_size_mm = entry.value().as_string().and_then(parse_size_mm);
                            }
                        }
                    }
                    if parsed_interval.is_none() {
                        if let Some(interval_node) = display_children.nodes().iter().find(|n| n.name().value() == "brightness_interval") {
                            if let Some(entry) = interval_node.entries().first() {
                                if let Some(num) = entry.value().as_i64() {
                                    parsed_interval = Some(num);
                                }
                            }
                        }
                    }
                    if parsed_up.is_none() {
                        if let Some(up_node) = display_children.nodes().iter().find(|n| n.name().value() == "brightness_up") {
                            if let Some(entry) = up_node.entries().first() {
                                if let Some(key_val) = entry.value().as_string() {
                                    parsed_up = Some(key_val.to_string());
                                }
                            }
                        }
                    }
                    if parsed_down.is_none() {
                        if let Some(down_node) = display_children.nodes().iter().find(|n| n.name().value() == "brightness_down") {
                            if let Some(entry) = down_node.entries().first() {
                                if let Some(key_val) = entry.value().as_string() {
                                    parsed_down = Some(key_val.to_string());
                                }
                            }
                        }
                    }
                }

                if let Some(num) = parsed_scale {
                    display.insert(format!("scale_{}", name), num);
                }
                if let Some((w, h)) = parsed_size_mm {
                    display.insert(format!("mm_w_{}", name), w);
                    display.insert(format!("mm_h_{}", name), h);
                }
                let interval = parsed_interval.unwrap_or(10);
                if parsed_interval.is_some() {
                    display.insert(format!("brightness_interval_{}", name), interval as f64);
                }
                if let Some(key_val) = parsed_up {
                    key_bindings.push(KeybindConfig {
                        mods: "".to_string(),
                        key: key_val,
                        action: "spawn".to_string(),
                        command: Some(format!("brightnessctl set {}%+", interval)),
                    });
                }
                if let Some(key_val) = parsed_down {
                    key_bindings.push(KeybindConfig {
                        mods: "".to_string(),
                        key: key_val,
                        action: "spawn".to_string(),
                        command: Some(format!("brightnessctl set {}%-", interval)),
                    });
                }
            }
        }
    }

    // 6. input
    let mut input = None;
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "input") {
        let accel_speed = get_child_arg_f64_opt(node, "accel_speed");
        let accel_profile = get_child_arg_string_opt(node, "accel_profile");
        let scroll_factor = get_child_arg_f64_opt(node, "scroll_factor");
        let scroll_ease = get_child_arg_f64_opt(node, "scroll_ease");
        let kinetic_scroll = get_child_arg_bool_opt(node, "kinetic_scroll");
        let scroll_friction = get_child_arg_f64_opt(node, "scroll_friction");
        let repeat_rate = get_child_arg_i64_opt(node, "repeat_rate");
        let repeat_delay = get_child_arg_i64_opt(node, "repeat_delay");

        let mut touchpad = None;
        if let Some(children) = node.children() {
            // `trackpad` is the input.kdl spelling, `touchpad` the legacy one.
            if let Some(tp_node) = children
                .nodes()
                .iter()
                .find(|n| n.name().value() == "touchpad" || n.name().value() == "trackpad")
            {
                let tap_to_click = get_child_arg_bool_opt(tp_node, "tap_to_click");
                let natural_scroll = get_child_arg_bool_opt(tp_node, "natural_scroll");
                let dwt = get_child_arg_bool_opt(tp_node, "dwt");
                let dwtp = get_child_arg_bool_opt(tp_node, "dwtp");

                let mut gestures = None;
                if let Some(tp_children) = tp_node.children() {
                    if let Some(gestures_node) = tp_children.nodes().iter().find(|n| n.name().value() == "gestures") {
                        let swipe = get_child_arg_bool_opt(gestures_node, "swipe");
                        let pinch = get_child_arg_bool_opt(gestures_node, "pinch");
                        gestures = Some(GesturesConfig { swipe, pinch });
                    }
                }

                touchpad = Some(TouchpadConfig {
                    tap_to_click,
                    natural_scroll,
                    dwt,
                    dwtp,
                    gestures,
                    accel_speed: get_child_arg_f64_opt(tp_node, "accel_speed"),
                    accel_profile: get_child_arg_string_opt(tp_node, "accel_profile"),
                    scroll_factor: get_child_arg_f64_opt(tp_node, "scroll_factor"),
                });
            }
        }

        let mut trackpoint = None;
        if let Some(children) = node.children() {
            if let Some(tp_node) = children.nodes().iter().find(|n| n.name().value() == "trackpoint") {
                let accel_speed = get_child_arg_f64_opt(tp_node, "accel_speed");
                let accel_profile = get_child_arg_string_opt(tp_node, "accel_profile");
                trackpoint = Some(TrackpointConfig {
                    accel_speed,
                    accel_profile,
                    scroll_factor: get_child_arg_f64_opt(tp_node, "scroll_factor"),
                    scroll_method: get_child_arg_string_opt(tp_node, "scroll_method"),
                });
            }
        }

        let mut mouse = None;
        if let Some(children) = node.children() {
            if let Some(m_node) = children.nodes().iter().find(|n| n.name().value() == "mouse") {
                mouse = Some(MouseConfig {
                    accel_speed: get_child_arg_f64_opt(m_node, "accel_speed"),
                    accel_profile: get_child_arg_string_opt(m_node, "accel_profile"),
                    scroll_factor: get_child_arg_f64_opt(m_node, "scroll_factor"),
                    scroll_method: get_child_arg_string_opt(m_node, "scroll_method"),
                });
            }
        }

        input = Some(InputConfig {
            accel_speed,
            accel_profile,
            scroll_factor,
            scroll_ease,
            kinetic_scroll,
            scroll_friction,
            repeat_rate,
            repeat_delay,
            mouse,
            touchpad,
            trackpoint,
        });
    }

    // 7. transparency
    let mut transparency = None;
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "transparency") {
        let opacity = get_child_arg_f64(node, "opacity", 0.9);
        transparency = Some(TransparencyConfig { opacity: Some(opacity) });
    }

    // 8. surface
    let mut surface = SurfaceConfig::default();
    let mut found_nested = false;
    if let Some(style_node) = doc.nodes().iter().find(|n| n.name().value() == "style") {
        if let Some(style_children) = style_node.children() {
            if let Some(surface_node) = style_children.nodes().iter().find(|n| n.name().value() == "surface") {
                if let Some(surface_children) = surface_node.children() {
                    if let Some(desktop_node) = surface_children.nodes().iter().find(|n| n.name().value() == "desktop") {
                        found_nested = true;
                        for entry in desktop_node.entries() {
                            if let Some(id) = entry.name() {
                                match id.value() {
                                    "gap_color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.desktop_gap_color = val.to_string();
                                        }
                                    }
                                    "cell_color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.desktop_cell_color = val.to_string();
                                        }
                                    }
                                    "gap_width" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_gap_width = val;
                                        }
                                    }
                                    "line_relief" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_line_relief = val;
                                        } else if let Some(s) = entry.value().as_string() {
                                            // A (relief) value: the fallback
                                            // honors its width — the client
                                            // installs the full material,
                                            // but the scenefx chamfer has no
                                            // custom profile to install.
                                            if let Some(spec) = cce_core::relief_spec::ReliefSpec::parse(s) {
                                                surface.desktop_line_relief = spec.width.round() as i64;
                                            }
                                        }
                                    }
                                    "cell_fade_inset" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_cell_fade_inset = val;
                                        }
                                    }
                                    "cell_labels" => {
                                        if let Some(val) = entry.value().as_bool() {
                                            surface.desktop_cell_labels = val;
                                        }
                                    }
                                    "grid_fade_mode" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.desktop_grid_fade_mode = val.to_string();
                                        }
                                    }
                                    "grid_cell_size" | "desktop_grid_scale" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_grid_scale = val;
                                        }
                                    }
                                    "grid_cell_width" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.grid_cell_width = Some(val);
                                        }
                                    }
                                    "grid_cell_height" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.grid_cell_height = Some(val);
                                        }
                                    }
                                    "snap" => {
                                        if let Some(val) = entry.value().as_bool() {
                                            surface.desktop_snap = val;
                                        }
                                    }
                                    "overview_ramp" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.desktop_overview_ramp = val.to_string();
                                        }
                                    }
                                    "overview_ms" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_overview_ms = val;
                                        }
                                    }
                                    "snap_threshold" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_snap_threshold = val;
                                        }
                                    }
                                    "edge_pan" => {
                                        if let Some(val) = entry.value().as_bool() {
                                            surface.desktop_edge_pan = val;
                                        }
                                    }
                                    "edge_pan_band" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_edge_pan_band = val;
                                        }
                                    }
                                    "edge_pan_speed" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.desktop_edge_pan_speed = val;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    // RFC Phase 7a (cce-ui): `plate { root ... }` is the one
                    // spelling of the root-plate style; the compositor reads
                    // the silhouette values from this block. The legacy
                    // `root plate` read-alias was removed 2026-09-06 after
                    // every live config had migrated.
                    let root_plate_node = surface_children
                        .nodes()
                        .iter()
                        .find(|n| n.name().value() == "plate")
                        .and_then(|n| n.children())
                        .and_then(|c| c.nodes().iter().find(|n| n.name().value() == "root"));
                    if let Some(root_node) = root_plate_node {
                        found_nested = true;
                        // Both spellings — properties on the `root` line and
                        // child nodes inside `root { … }` — see
                        // `node_keyed_values`. This block is the one the
                        // window silhouette comes from, and cce-ui reads the
                        // same keys either way; reading one spelling here put
                        // the compositor's clip and the apps' root plates on
                        // different radii.
                        for (key, entry) in node_keyed_values(root_node) {
                            match key.as_str() {
                                "color" => {
                                    if let Some(val) = entry.value().as_string() {
                                        surface.root_plate_color = val.to_string();
                                    }
                                }
                                "blur" => {
                                    if let Some(mut val) = entry.value().as_f64() {
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
                                        surface.root_plate_blur = val;
                                    }
                                }
                                "corner_radius" => {
                                    if let Some(val) = entry.value().as_i64() {
                                        surface.root_plate_corner_radius = val;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    // `surface { fade in_ms=140 out_ms=120 }` — the DE-wide
                    // open/close dissolve, read here so both halves of it
                    // (the compositor's scene-node ramp and the deadline a
                    // closing client waits on) come from one place.
                    if let Some(fade_node) = surface_children.nodes().iter().find(|n| n.name().value() == "fade") {
                        found_nested = true;
                        for entry in fade_node.entries() {
                            if let Some(id) = entry.name() {
                                match id.value() {
                                    "in_ms" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.fade_in_ms = val;
                                        }
                                    }
                                    "out_ms" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.fade_out_ms = val;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if let Some(border_node) = surface_children.nodes().iter().find(|n| n.name().value() == "border") {
                        found_nested = true;
                        for entry in border_node.entries() {
                            if let Some(id) = entry.name() {
                                match id.value() {
                                    "width" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.border_width = val;
                                        }
                                    }
                                    "color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.border_color = val.to_string();
                                        }
                                    }
                                    "color_focused" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.border_color_focused = Some(val.to_string());
                                        }
                                    }
                                    "color_hover" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.border_color_hover = Some(val.to_string());
                                        }
                                    }
                                    // No `corner_radius` here: the border
                                    // ring, its discs and the frame take the
                                    // ONE window radius, `plate.root.
                                    // corner_radius` (see `Layout::
                                    // root_plate_corner_radius`). The key was
                                    // read into dead config until 2026-09-28
                                    // and is ignored now, like any unknown one.
                                    "segment_gap" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.border_segment_gap = val;
                                        }
                                    }
                                    "taper" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.border_taper = val;
                                        } else if let Some(val) = entry.value().as_i64() {
                                            surface.border_taper = val as f64;
                                        }
                                    }
                                    "handle_width" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.border_handle_width = val;
                                        } else if let Some(val) = entry.value().as_i64() {
                                            surface.border_handle_width = val as f64;
                                        }
                                    }
                                    "overlap_opacity" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.border_overlap_opacity = val;
                                        } else if let Some(val) = entry.value().as_i64() {
                                            surface.border_overlap_opacity = val as f64;
                                        }
                                    }
                                    "swell_curve" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.border_swell_curve = val;
                                        } else if let Some(val) = entry.value().as_i64() {
                                            surface.border_swell_curve = val as f64;
                                        }
                                    }
                                    "bulge" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.border_corner_bulge = val;
                                        } else if let Some(val) = entry.value().as_i64() {
                                            surface.border_corner_bulge = val as f64;
                                        }
                                    }
                                    "corner_length" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.border_corner_length = val;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if let Some(bevel_node) = surface_children.nodes().iter().find(|n| n.name().value() == "bevel") {
                        found_nested = true;
                        for entry in bevel_node.entries() {
                            if let Some(id) = entry.name() {
                                match id.value() {
                                    "enabled" => {
                                        if let Some(val) = entry.value().as_bool() {
                                            surface.bevel_enabled = val;
                                        }
                                    }
                                    "thickness" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.bevel_thickness = val;
                                        } else if let Some(val) = entry.value().as_i64() {
                                            surface.bevel_thickness = val as f64;
                                        }
                                    }
                                    "light" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.bevel_light = val.to_string();
                                        }
                                    }
                                    "light_intensity" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.bevel_light_intensity = val;
                                        }
                                    }
                                    "shade_intensity" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.bevel_shade_intensity = val;
                                        }
                                    }
                                    "shoulder" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.bevel_shoulder = val;
                                        }
                                    }
                                    "color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.bevel_color = val.to_string();
                                        }
                                    }
                                    "focus_sharpness" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.bevel_focus_sharpness = val;
                                        } else if let Some(val) = entry.value().as_i64() {
                                            surface.bevel_focus_sharpness = val as f64;
                                        }
                                    }
                                    "focus_color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.bevel_focus_color = val.to_string();
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if let Some(shadow_node) = surface_children.nodes().iter().find(|n| n.name().value() == "shadow") {
                        found_nested = true;
                        for entry in shadow_node.entries() {
                            if let Some(id) = entry.name() {
                                match id.value() {
                                    "enabled" => {
                                        if let Some(val) = entry.value().as_bool() {
                                            surface.shadow_enabled = val;
                                        }
                                    }
                                    // Named `blur` to match the status/root plate blur keys.
                                    "blur" => {
                                        if let Some(val) = entry.value().as_f64() {
                                            surface.shadow_sigma = val;
                                        } else if let Some(val) = entry.value().as_i64() {
                                            surface.shadow_sigma = val as f64;
                                        }
                                    }
                                    "color" => {
                                        if let Some(val) = entry.value().as_string() {
                                            surface.shadow_color = val.to_string();
                                        }
                                    }
                                    "offset_x" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.shadow_offset_x = val;
                                        }
                                    }
                                    "offset_y" => {
                                        if let Some(val) = entry.value().as_i64() {
                                            surface.shadow_offset_y = val;
                                        }
                                    }
                                    "tiled" => {
                                        if let Some(val) = entry.value().as_bool() {
                                            surface.shadow_tiled = val;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if let Some(cloud_node) = surface_children.nodes().iter().find(|n| n.name().value() == "cloud") {
                        found_nested = true;
                        if let Some(pos) = get_child_arg_vec2i_opt(cloud_node, "position_default") {
                            surface.cloud_position_default = Some(pos);
                        }
                    }
                }
            }
        }
    }
    if !found_nested {
        if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "surface") {
            surface.desktop_gap_color = get_child_arg_string(node, "desktop_gap_color", &default_desktop_gap_color());
            surface.desktop_cell_color = get_child_arg_string(node, "desktop_cell_color", &default_desktop_cell_color());
            surface.desktop_grid_scale = get_child_arg_i64(node, "grid_cell_size", get_child_arg_i64(node, "desktop_grid_scale", default_desktop_grid_scale()));
            surface.grid_cell_width = match get_child_arg_i64(node, "grid_cell_width", i64::MIN) {
                i64::MIN => None,
                v => Some(v),
            };
            surface.grid_cell_height = match get_child_arg_i64(node, "grid_cell_height", i64::MIN) {
                i64::MIN => None,
                v => Some(v),
            };
            surface.desktop_gap_width = get_child_arg_i64(node, "desktop_gap_width", default_desktop_gap_width());
            surface.desktop_cell_fade_inset = get_child_arg_i64(node, "desktop_cell_fade_inset", default_desktop_cell_fade_inset());
            surface.desktop_grid_fade_mode = get_child_arg_string(node, "grid_fade_mode", &default_desktop_grid_fade_mode());
            surface.desktop_snap = get_child_arg_bool(node, "desktop_snap", default_desktop_snap());
            surface.desktop_overview_ramp = get_child_arg_string(node, "desktop_overview_ramp", "");
            surface.desktop_overview_ms = get_child_arg_i64(node, "desktop_overview_ms", default_desktop_overview_ms());
            surface.desktop_snap_threshold = get_child_arg_i64(node, "desktop_snap_threshold", default_desktop_snap_threshold());
            surface.desktop_edge_pan = get_child_arg_bool(node, "desktop_edge_pan", default_desktop_edge_pan());
            surface.desktop_edge_pan_band = get_child_arg_i64(node, "desktop_edge_pan_band", default_desktop_edge_pan_band());
            surface.desktop_edge_pan_speed = get_child_arg_i64(node, "desktop_edge_pan_speed", default_desktop_edge_pan_speed());
            surface.root_plate_color = get_child_arg_string(node, "root_plate_color", &default_root_plate_color());
            surface.root_plate_blur = get_child_arg_f64(node, "root_plate_blur", default_root_plate_blur());
            surface.root_plate_corner_radius = get_child_arg_i64(node, "root_plate_corner_radius", default_root_plate_corner_radius());
            surface.border_width = get_child_arg_i64(node, "border_width", default_border_width());
            surface.border_color = get_child_arg_string(node, "border_color", &default_border_color());
            surface.border_color_focused = get_child_arg_string_opt(node, "border_color_focused");
            surface.border_color_hover = get_child_arg_string_opt(node, "border_color_hover");
            surface.border_segment_gap = get_child_arg_i64(node, "border_segment_gap", default_border_segment_gap());
            surface.border_corner_length = get_child_arg_i64(node, "border_corner_length", 0);
            surface.cloud_position_default = get_child_arg_vec2i_opt(node, "cloud_position_default");
        }
    }

    // window manager
    let mut window_manager = None;
    if let Some(node) = doc.nodes().iter().find(|n| n.name().value() == "window manager" || n.name().value() == "window_manager") {
        let close_window = get_child_arg_string_opt(node, "close_window");
        let toggle_fullscreen = get_child_arg_string_opt(node, "toggle_fullscreen");
        let toggle_overview = get_child_arg_string_opt(node, "toggle_overview");
        let window_switcher = get_child_arg_string_opt(node, "window_switcher");
        let window_switcher_prev = get_child_arg_string_opt(node, "window_switcher_prev");
        let center_on_spawn = get_child_arg_bool_opt(node, "center_on_spawn");
        let on_app_exit = get_child_arg_string_opt(node, "on_app_exit");
        let corner_shape = get_child_arg_f64_opt(node, "corner_shape");
        let rounded_apps = get_child_args_string_vec_opt(node, "rounded_apps");
        let bevel_apps = get_child_args_string_vec_opt(node, "bevel_apps");
        let xwayland_hidpi = get_child_arg_bool_opt(node, "xwayland_hidpi");
        let xwayland_hidpi_except = get_child_args_string_vec_opt(node, "xwayland_hidpi_except");
        let touchpad_view_apps = get_child_args_string_vec_opt(node, "touchpad_view_apps");
        let touchpad_view_swipe = get_child_arg_string_opt(node, "touchpad_view_swipe");
        let touchpad_view_sensitivity = get_child_arg_f64_opt(node, "touchpad_view_sensitivity");
        let swipe_peek = get_child_arg_f64_opt(node, "swipe_peek");
        let swipe_repeat_peek = get_child_arg_f64_opt(node, "swipe_repeat_peek");
        let swipe_focus_cone = get_child_arg_f64_opt(node, "swipe_focus_cone");
        let swipe_threshold = get_child_arg_f64_opt(node, "swipe_threshold");
        let swipe_repeat_threshold = get_child_arg_f64_opt(node, "swipe_repeat_threshold");
        let touchpad_view_invert = get_child_arg_bool_opt(node, "touchpad_view_invert");
        let touchpad_hscroll_shift_apps = get_child_args_string_vec_opt(node, "touchpad_hscroll_shift_apps");
        let osk_on_touch = get_child_arg_bool_opt(node, "osk_on_touch");
        window_manager = Some(WindowManagerConfig { close_window, toggle_fullscreen, toggle_overview, window_switcher, window_switcher_prev, center_on_spawn, on_app_exit, corner_shape, rounded_apps, bevel_apps, xwayland_hidpi, xwayland_hidpi_except, touchpad_view_apps, touchpad_view_swipe, touchpad_view_sensitivity, swipe_peek, swipe_repeat_peek, swipe_focus_cone, swipe_threshold, swipe_repeat_threshold, touchpad_view_invert, touchpad_hscroll_shift_apps, osk_on_touch });
    }

    Ok(Config {
        layout,
        env,
        key_bindings,
        pointer_bind,
        mode_rule,
        tag_layout,
        startup,
        output,
        display,
        device,
        input,
        gesture_bind,
        transparency,
        surface,
        window_manager,
        idle,
    })
}
