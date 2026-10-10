//! `parse_config`: load config.kdl and apply it to the window manager — layout,
//! surface, desktop, input, rules, keybinds and the rest. Split out of config.rs
//! on 2026-10-10.

use super::*;

pub fn parse_config(path: &str, state: &mut crate::window_manager::WindowManager) -> Result<(), String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return Err(format!("cannot open {}: {}", path, e)),
    };

    let mut config: Config = parse_kdl_config(&content)?;

    let path_buf = std::path::Path::new(path);
    let input_path = path_buf.parent().unwrap_or_else(|| std::path::Path::new(".")).join("input.kdl");
    let mut wm_domain_entries: Vec<cce_core::input::BindingEntry> = Vec::new();
    if input_path.exists() {
        if let Ok(input_content) = fs::read_to_string(&input_path) {
            // New domain-scoped format: a `cce-window-manager { ... }` block
            // of `<action_name> "<chord>"` bindings. Other domains belong to
            // clients/widgets and are ignored here.
            match cce_core::input::InputConfig::parse(&input_content) {
                Ok(ic) => {
                    wm_domain_entries = ic.domain(cce_core::input::WINDOW_MANAGER_DOMAIN).to_vec();
                }
                Err(e) => eprintln!("[WARNING] {}: {}", input_path.display(), e),
            }
            // Legacy input.kdl contents: root-level key_bindings nodes and
            // the input section.
            if let Ok(input_config) = parse_kdl_config(&input_content) {
                config.key_bindings.extend(input_config.key_bindings);
                if input_config.input.is_some() {
                    config.input = input_config.input;
                }
            }
        }
    }

    state.output_scale = 1.0f32;
    state.xwayland_hidpi = config
        .window_manager
        .as_ref()
        .and_then(|wm| wm.xwayland_hidpi)
        .unwrap_or(true);
    state.xwayland_hidpi_except = config
        .window_manager
        .as_ref()
        .and_then(|wm| wm.xwayland_hidpi_except.clone())
        .unwrap_or_default();
    {
        let tv = config.window_manager.as_ref();
        state.touchpad_view_apps = tv.and_then(|w| w.touchpad_view_apps.clone()).unwrap_or_default();
        state.touchpad_view_swipe_tumble = tv
            .and_then(|w| w.touchpad_view_swipe.as_deref())
            .map_or(false, |s| s.eq_ignore_ascii_case("tumble"));
        state.touchpad_view_sensitivity = tv.and_then(|w| w.touchpad_view_sensitivity).unwrap_or(1.0);
        state.swipe_peek_px = tv
            .and_then(|w| w.swipe_peek)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(60.0);
        state.swipe_repeat_peek_px = tv
            .and_then(|w| w.swipe_repeat_peek)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(state.swipe_peek_px * 0.5);
        state.swipe_focus_cone_deg = tv
            .and_then(|w| w.swipe_focus_cone)
            .filter(|v| v.is_finite())
            .map_or(45.0, |v| v.clamp(1.0, 89.0));
        state.swipe_threshold = tv
            .and_then(|w| w.swipe_threshold)
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(70.0);
        state.swipe_repeat_threshold = tv
            .and_then(|w| w.swipe_repeat_threshold)
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(state.swipe_threshold * 4.0);
        state.touchpad_view_invert = tv.and_then(|w| w.touchpad_view_invert).unwrap_or(false);
        state.touchpad_hscroll_shift_apps = tv.and_then(|w| w.touchpad_hscroll_shift_apps.clone()).unwrap_or_default();
        state.osk_on_touch = tv.and_then(|w| w.osk_on_touch).unwrap_or(true);
    }
    state.display = config.display.clone();
    state.on_app_exit = config
        .window_manager
        .as_ref()
        .and_then(|wm| wm.on_app_exit.as_deref())
        .map(parse_on_app_exit)
        .unwrap_or(OnAppExit::FocusPrevious);
    state.center_on_spawn = config
        .window_manager
        .as_ref()
        .and_then(|wm| wm.center_on_spawn)
        .unwrap_or(true);
    state.rounded_apps = config
        .window_manager
        .as_ref()
        .and_then(|wm| wm.rounded_apps.clone())
        .unwrap_or_default();
    // Unset means "same apps as rounded_apps" — which deliberately excludes
    // the implicit cce-* set, since those draw their own bevels.
    state.bevel_apps = config
        .window_manager
        .as_ref()
        .and_then(|wm| wm.bevel_apps.clone())
        .unwrap_or_else(|| state.rounded_apps.clone());

    // Feed scenefx's rounded-corner shaders the DE-wide corner-shape exponent
    // (clamped like cce-ui's corner_shape()). Plain C state, safe pre-renderer
    // and on live reload.
    let corner_shape = config
        .window_manager
        .as_ref()
        .and_then(|wm| wm.corner_shape)
        .unwrap_or(2.0)
        .clamp(2.0, 16.0);
    unsafe {
        crate::ffi::fx_renderer_set_corner_shape(corner_shape as f32);
    }
    let span_factor = if corner_shape > 2.001 {
        (corner_shape - 1.0) * 2f64.powf(1.0 / corner_shape) / std::f64::consts::SQRT_2
    } else {
        1.0
    };
    CORNER_SPAN_FACTOR.store(span_factor.to_bits(), std::sync::atomic::Ordering::Relaxed);

    state.layout.gap = config.layout.gap as i32;
    state.layout.gap_top = config.layout.gap_top as i32;
    state.layout.gap_left = config.layout.gap_left as i32;
    state.layout.gap_right = config.layout.gap_right as i32;
    state.layout.gap_bottom = config.layout.gap_bottom as i32;
    state.layout.cascade_offset = config.layout.cascade_offset as i32;
    state.layout.bar_height = config.layout.bar_height as i32;
    state.layout.border_width = config.surface.border_width as i32;
    state.layout.fullscreen_border_width = 0;
    state.layout.cascade_border_width = 0;
    state.layout.grid_border_width = 0;
    state.layout.floating_border_width = 0;

    state.layout.border_color = parse_hex_color_rgba(&config.surface.border_color);
    state.layout.border_color_focused = config
        .surface
        .border_color_focused
        .as_deref()
        .map(parse_hex_color_rgba)
        .unwrap_or(state.layout.border_color);
    state.layout.border_color_hover = config
        .surface
        .border_color_hover
        .as_deref()
        .map(parse_hex_color_rgba)
        .unwrap_or_else(|| lighten_premultiplied(state.layout.border_color_focused, HOVER_LIGHTEN));
    state.layout.border_segment_gap = config.surface.border_segment_gap.max(0) as i32;
    // Clamped at 1: past that the corners would be THICKER than the middle,
    // which is the moulding inside out.
    state.layout.border_taper = config.surface.border_taper.clamp(0.05, 1.0) as f32;
    state.layout.border_handle_width = config.surface.border_handle_width.max(4.0) as f32;
    state.layout.border_overlap_opacity = config.surface.border_overlap_opacity.clamp(0.0, 1.0) as f32;
    // Capped at 2s: the close fade is a deadline a client blocks on before it
    // exits, so a mistyped 20000 would hang every quit for 20 seconds.
    state.layout.fade_in_ms = config.surface.fade_in_ms.clamp(0, 2000) as u32;
    state.layout.fade_out_ms = config.surface.fade_out_ms.clamp(0, 2000) as u32;
    state.layout.border_swell_curve = config.surface.border_swell_curve.clamp(0.1, 6.0) as f32;
    state.layout.border_corner_bulge = config.surface.border_corner_bulge.max(0.0) as f32;
    state.layout.border_corner_length = config.surface.border_corner_length.max(0) as i32;

    state.layout.desktop_gap_color = config.surface.desktop_gap_color.clone();

    let background_color_val = parse_hex_color(&config.surface.desktop_gap_color);
    state.layout.background_r = ((background_color_val >> 16) & 0xFF) * 0x01010101;
    state.layout.background_g = ((background_color_val >> 8) & 0xFF) * 0x01010101;
    state.layout.background_b = (background_color_val & 0xFF) * 0x01010101;
    state.layout.background_a = 0xFFFFFFFF;

    state.layout.desktop_cell_color = parse_hex_color_rgba(&config.surface.desktop_cell_color);
    state.layout.desktop_cell_width =
        config.surface.grid_cell_width.unwrap_or(config.surface.desktop_grid_scale) as f64;
    state.layout.desktop_cell_height =
        config.surface.grid_cell_height.unwrap_or(config.surface.desktop_grid_scale) as f64;
    state.layout.desktop_snap = config.surface.desktop_snap;
    state.layout.overview_anim = if config.surface.desktop_overview_ramp.is_empty() {
        None
    } else {
        match crate::policy::ramp::SpeedRamp::from_spec(&config.surface.desktop_overview_ramp) {
            Some(ramp) => Some((ramp, (config.surface.desktop_overview_ms.max(16)) as f64)),
            None => {
                log::warn!("overview_ramp {:?} is invalid or all-zero; falling back to the exponential camera animation", config.surface.desktop_overview_ramp);
                None
            }
        }
    };
    state.layout.desktop_snap_threshold = config.surface.desktop_snap_threshold.max(0) as f64;
    state.layout.desktop_edge_pan = config.surface.desktop_edge_pan;
    state.layout.desktop_edge_pan_band = config.surface.desktop_edge_pan_band.max(1) as f64;
    state.layout.desktop_edge_pan_speed = config.surface.desktop_edge_pan_speed.max(0) as f64;
    state.layout.desktop_gap_width = config.surface.desktop_gap_width as i32;
    state.layout.desktop_line_relief = if config.surface.desktop_line_relief < 0 {
        None
    } else {
        Some(config.surface.desktop_line_relief as f64)
    };
    state.layout.desktop_cell_fade_inset = config.surface.desktop_cell_fade_inset;
    state.layout.desktop_cell_labels = config.surface.desktop_cell_labels;
    state.layout.desktop_grid_fade_mode = config.surface.desktop_grid_fade_mode.clone();

    state.layout.border_font_size = 11;
    state.layout.transition_duration = config.layout.transition_duration as i32;
    state.layout.grid_gap = config.layout.grid_gap as i32;
    state.layout.border_blur = false;
    state.layout.window_blur = config.surface.root_plate_blur > 0.001;
    state.layout.root_plate_corner_radius = config.surface.root_plate_corner_radius as i32;
    state.layout.overlay_behavior = config.layout.overlay_behavior;
    state.layout.overlay_width = config.layout.overlay_width as i32;
    state.layout.overlay_position = config.layout.overlay_position;
    state.layout.overlay_border_gap = config.layout.overlay_border_gap as i32;
    state.layout.status_normal_color = config.layout.status_normal_color.clone();
    state.layout.status_background_blur = config.layout.status_background_blur as f32;
    state.layout.transparency_opacity = config.transparency.as_ref().and_then(|t| t.opacity).unwrap_or(0.9) as f32;
    let root_plate_rgba = parse_hex_color_rgba(&config.surface.root_plate_color);
    state.layout.window_opacity = root_plate_rgba[3] < 0.999;
    state.layout.scenefx_optimized_blur = config.output.as_ref().map(|o| o.scenefx_optimized_blur).unwrap_or(true);
    // Idle timeouts live on the server, not the window manager; a reload
    // re-arms them from now with the new figures.
    if !state.server.is_null() {
        unsafe { (*state.server).idle.configure(&config.idle); }
    }
    state.layout.status_backdrop_blur_ignore_transparent = config.layout.status_backdrop_blur_ignore_transparent;
    state.layout.window_backdrop_blur_ignore_transparent = config.layout.window_backdrop_blur_ignore_transparent;
    state.layout.status_module_hide_mode_preview = config.layout.status_module_hide_mode_preview;
    state.layout.status_module_spacing = config.layout.status_module_spacing;
    state.layout.status_droplet = config.layout.status_droplet.clone();
    state.layout.status_backdrop_compress = config.layout.status_backdrop_compress;
    state.layout.cloud_position_default = config.surface.cloud_position_default;
    state.layout.shadow_enabled = config.surface.shadow_enabled;
    state.layout.shadow_sigma = config.surface.shadow_sigma.max(0.0) as f32;
    state.layout.shadow_color = parse_hex_color_rgba(&config.surface.shadow_color);
    state.layout.shadow_offset_x = config.surface.shadow_offset_x as i32;
    state.layout.shadow_offset_y = config.surface.shadow_offset_y as i32;
    state.layout.shadow_tiled = config.surface.shadow_tiled;
    state.layout.bevel_enabled = config.surface.bevel_enabled;
    state.layout.bevel_thickness = config.surface.bevel_thickness.max(0.0) as f32;
    let (bevel_lx, bevel_ly) = parse_light_direction(&config.surface.bevel_light);
    state.layout.bevel_light_x = bevel_lx;
    state.layout.bevel_light_y = bevel_ly;
    state.layout.bevel_light_intensity = config.surface.bevel_light_intensity.clamp(0.0, 1.0) as f32;
    state.layout.bevel_shade_intensity = config.surface.bevel_shade_intensity.clamp(0.0, 1.0) as f32;
    state.layout.bevel_shoulder = config.surface.bevel_shoulder.clamp(0.0, 1.0) as f32;
    state.layout.bevel_color = parse_hex_color_rgba(&config.surface.bevel_color);
    let fc = parse_hex_color_rgba(&config.surface.bevel_focus_color);
    state.layout.bevel_focus_color = [fc[0], fc[1], fc[2]];
    // Clamped low at 1: below that the glint would spread WIDER than the
    // rim's own slope, which is what `thickness` is for.
    state.layout.bevel_focus_sharpness =
        config.surface.bevel_focus_sharpness.clamp(1.0, 64.0) as f32;

    for (key, val) in &config.env {
        let expanded = expand_env_vars(val);
        std::env::set_var(key, &expanded);
    }

    state.input_rules = config.device.clone();
    state.input_config = config.input.clone().unwrap_or_default();
    unsafe {
        // Config first (its per-class scroll factors are defaults), then the
        // name-based device rules so they stay the most specific override.
        state.apply_input_config();
        state.apply_input_rules();
    }

    state.keybinds.clear();
    let mut table = cce_window_manager::bindings::BindingTable::new();
    // Gesture entries from the same domain (`focus_left "swipe3_left"`);
    // they go ahead of config.kdl's `gesture_bind` nodes below.
    let mut input_gesture_binds: Vec<GestureBind> = Vec::new();

    // Primary source: the `cce-window-manager` domain of input.kdl.
    for entry in &wm_domain_entries {
        let Some(action) = Action::from_name(&entry.name) else {
            eprintln!("[WARNING] input.kdl: unknown window-manager action {:?}", entry.name);
            continue;
        };
        let command = match action {
            Action::Spawn | Action::Toggle => {
                warn_if_command_missing(entry.command.as_deref());
                entry.command.clone()
            }
            // Media-key actions have stock commands (policy-side
            // `media_command`); a `command=` property overrides.
            Action::VolumeUp | Action::VolumeDown | Action::VolumeMute | Action::MicMute
            | Action::BrightnessUp | Action::BrightnessDown => {
                warn_if_command_missing(entry.command.as_deref());
                entry.command.clone()
            }
            _ => None,
        };
        // A touchscreen edge swipe: `edge_left` fires on a finger swiped in
        // from the left edge (`touch::Claim::Edge`).
        if let Some((mods, edge)) = parse_edge_gesture(&entry.chord) {
            if input_gesture_binds.iter().any(|b| b.mods == mods && b.gesture_type == "edge" && b.direction == edge) {
                eprintln!("[WARNING] input.kdl: {:?} is bound more than once", entry.chord);
            }
            input_gesture_binds.push(GestureBind {
                mods,
                gesture_type: "edge".to_string(),
                fingers: 1,
                direction: edge,
                action,
                command: command.clone(),
            });
            continue;
        }
        // A touchpad gesture rides in the chord slot: `swipe3_left`,
        // `super+pinch_out`. The fingerless spelling binds three AND four
        // fingers, as `toggle_overview "swipe_down"` always has.
        if let Some(g) = cce_window_manager::bindings::parse_gesture(&entry.chord) {
            let fingers: Vec<u32> = g.fingers.map(|n| vec![n]).unwrap_or_else(|| vec![3, 4]);
            for fingers in fingers {
                let dup = input_gesture_binds.iter().any(|b| {
                    b.mods == g.mods && b.gesture_type == g.kind.as_str() && b.fingers == fingers && b.direction == g.direction
                });
                if dup {
                    eprintln!("[WARNING] input.kdl: {:?} is bound more than once", entry.chord);
                }
                input_gesture_binds.push(GestureBind {
                    mods: g.mods,
                    gesture_type: g.kind.as_str().to_string(),
                    fingers,
                    direction: g.direction.clone(),
                    action,
                    command: command.clone(),
                });
            }
            continue;
        }
        let Some(chord) = cce_window_manager::bindings::parse_chord(&entry.chord) else {
            eprintln!("[WARNING] input.kdl: invalid chord {:?} for {}", entry.chord, entry.name);
            continue;
        };
        let keysym = parse_keysym(&chord.key);
        if keysym == 0 {
            eprintln!("[WARNING] input.kdl: unknown key {:?} in chord {:?}", chord.key, entry.chord);
            continue;
        }
        if table.add(Keybind { mods: chord.mods, keysym, action, command }) {
            eprintln!("[WARNING] input.kdl: {:?} is bound more than once", entry.chord);
        }
    }

    // Legacy sources: config.kdl `key_bindings` nodes (including the
    // synthesized brightness binds) and the `window_manager` section.
    // input.kdl wins on chord conflicts via add_default.
    let mut seen = std::collections::HashSet::new();
    for kb in &config.key_bindings {
        let (mods_str, key_str) = if kb.mods.is_empty() {
            if let Some(last_plus) = kb.key.rfind('+') {
                (kb.key[..last_plus].to_string(), kb.key[last_plus+1..].to_string())
            } else {
                ("".to_string(), kb.key.clone())
            }
        } else {
            (kb.mods.clone(), kb.key.clone())
        };
        let mods = parse_modifiers(&mods_str);
        let keysym = parse_keysym(&key_str);

        if !seen.insert((mods, keysym)) {
            eprintln!("[WARNING] Keybinding conflict: multiple actions mapped to mods={:?}, key={:?}", mods_str, key_str);
        }

        let action = parse_action(&kb.action);
        let command = if action == Action::Spawn || action == Action::Toggle {
            warn_if_command_missing(kb.command.as_deref());
            kb.command.clone()
        } else {
            None
        };
        table.add_default(Keybind { mods, keysym, action, command });
    }

    if let Some(ref wm_config) = config.window_manager {
        let wm_section_binds = [
            (&wm_config.close_window, Action::Close),
            (&wm_config.toggle_fullscreen, Action::Fullscreen),
            (&wm_config.window_switcher, Action::WindowSwitcher),
            (&wm_config.window_switcher_prev, Action::WindowSwitcherPrev),
        ];
        for (chord_str, action) in wm_section_binds {
            let Some(chord_str) = chord_str else { continue };
            if let Some(chord) = cce_window_manager::bindings::parse_chord(chord_str) {
                let keysym = parse_keysym(&chord.key);
                table.add_default(Keybind { mods: chord.mods, keysym, action, command: None });
            }
        }
    }

    // Stock defaults from the policy crate; never shadow configured chords.
    for d in cce_window_manager::bindings::DEFAULT_BINDINGS {
        let keysym = parse_keysym(d.key);
        table.add_default(Keybind { mods: d.mods, keysym, action: d.action, command: None });
    }

    state.keybinds = table.into_bindings();

    state.pointer_binds.clear();
    for pb in &config.pointer_bind {
        let mods = parse_modifiers(&pb.mods);
        let button = parse_button(&pb.button);
        let action = parse_action(&pb.action);
        state.pointer_binds.push(PointerBind {
            mods,
            button,
            action,
        });
    }

    // Gesture table, first match wins in the cursor's swipe/pinch handlers:
    // input.kdl entries, then config.kdl `gesture_bind` nodes, then the
    // legacy `window_manager { toggle_overview "swipe_down" }`.
    state.gesture_binds.clear();
    state.gesture_binds.extend(input_gesture_binds);
    for gb in &config.gesture_bind {
        let mods = gb.mods.as_ref().map(|m| parse_modifiers(m)).unwrap_or(0);
        let action = parse_action(&gb.action);
        let command = if action == Action::Spawn || action == Action::Toggle {
            gb.command.clone()
        } else {
            None
        };
        state.gesture_binds.push(GestureBind {
            mods,
            gesture_type: gb.gesture_type.clone(),
            fingers: gb.fingers,
            direction: gb.direction.clone(),
            action,
            command,
        });
    }

    if let Some(ref wm_config) = config.window_manager {
        if let Some(ref toggle_ov_str) = wm_config.toggle_overview {
            let normalized = toggle_ov_str.to_lowercase().replace('-', "_");
            let gesture_type = if normalized.starts_with("swipe") {
                Some("swipe")
            } else if normalized.starts_with("pinch") {
                Some("pinch")
            } else {
                None
            };
            if let Some(g_type) = gesture_type {
                let direction = normalized.trim_start_matches(g_type).trim_start_matches('_').to_string();
                for fingers in [3, 4] {
                    state.gesture_binds.push(GestureBind {
                        mods: 0,
                        gesture_type: g_type.to_string(),
                        fingers,
                        direction: direction.clone(),
                        action: Action::Overview,
                        command: None,
                    });
                }
            }
        }
    }

    state.mode_rules.clear();
    for rule in config.mode_rule {
        state.mode_rules.push(ModeRule {
            mode: parse_tiling_mode(&rule.mode),
            app_id_pattern: rule.app_id,
            title_pattern: rule.title,
            single_instance: rule.single.unwrap_or(false),
            tag: rule.tag.unwrap_or(-1) as i32,
            circular: rule.circular.unwrap_or(false),
            ssd: rule.ssd,
            over_sibling: rule.over_sibling.unwrap_or(false),
            center: rule.center.unwrap_or(false),
        });
    }

    for _tag_layout in config.tag_layout {
        // Tag layouts are ignored in the pannable coordinate system.
    }

    state.startup.clear();
    for st in config.startup {
        state.startup.push(st);
    }

    log::info!(
        "Parsed config: {} keybinds, {} pointer binds, {} gesture binds, {} startup programs",
        state.keybinds.len(),
        state.pointer_binds.len(),
        state.gesture_binds.len(),
        state.startup.len()
    );

    unsafe {
        if !state.server.is_null() {
            let outputs_head = &mut (*state.server).om.outputs as *mut crate::ffi::wl_list as *mut crate::server::WlList;
            let mut curr = (*outputs_head).next;
            while curr != outputs_head {
                let next = (*curr).next;
                let output = &mut *crate::container_of!(curr, crate::output::Output, link);
                output.update_background_color();
                curr = next;
            }
        }
    }

    Ok(())
}
