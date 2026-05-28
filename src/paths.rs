use std::env;

pub fn get_socket_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-{}.sock", display)
    } else {
        "/tmp/clearwm.sock".to_string()
    }
}

pub fn get_status_socket_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-status-{}.sock", display)
    } else {
        "/tmp/clearwm-status.sock".to_string()
    }
}

pub fn get_windows_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-windows-{}", display)
    } else {
        "/tmp/clearwm-windows".to_string()
    }
}

pub fn get_tags_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-tags-{}", display)
    } else {
        "/tmp/clearwm-tags".to_string()
    }
}

pub fn get_layout_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-layout-{}", display)
    } else {
        "/tmp/clearwm-layout".to_string()
    }
}

pub fn get_title_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-title-{}", display)
    } else {
        "/tmp/clearwm-title".to_string()
    }
}

pub fn get_death_log_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-death-{}.log", display)
    } else {
        "/tmp/clearwm-death.log".to_string()
    }
}

pub fn get_bt_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-bt-{}.txt", display)
    } else {
        "/tmp/clearwm-bt.txt".to_string()
    }
}

pub fn get_input_coords_socket_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clear-input-coords-{}.sock", display)
    } else {
        "/tmp/clear-input-coords.sock".to_string()
    }
}

pub fn get_log_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-{}.log", display)
    } else {
        "/tmp/clearwm.log".to_string()
    }
}

pub fn get_prev_log_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-{}-prev.log", display)
    } else {
        "/tmp/clearwm-prev.log".to_string()
    }
}

pub fn get_xprop_path(wid: u64) -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/clearwm-xprop-{}-{}", wid, display)
    } else {
        format!("/tmp/clearwm-xprop-{}", wid)
    }
}
