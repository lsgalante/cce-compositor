use std::env;

pub fn get_socket_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/ccec-{}.sock", display)
    } else {
        "/tmp/ccec.sock".to_string()
    }
}

pub fn get_status_socket_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/ccec-status-{}.sock", display)
    } else {
        "/tmp/ccec-status.sock".to_string()
    }
}

pub fn get_windows_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/ccec-windows-{}", display)
    } else {
        "/tmp/ccec-windows".to_string()
    }
}

pub fn get_tags_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/ccec-tags-{}", display)
    } else {
        "/tmp/ccec-tags".to_string()
    }
}

pub fn get_layout_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/ccec-layout-{}", display)
    } else {
        "/tmp/ccec-layout".to_string()
    }
}

pub fn get_title_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/ccec-title-{}", display)
    } else {
        "/tmp/ccec-title".to_string()
    }
}

pub fn get_death_log_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/ccec-death-{}.log", display)
    } else {
        "/tmp/ccec-death.log".to_string()
    }
}

pub fn get_bt_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/ccec-bt-{}.txt", display)
    } else {
        "/tmp/ccec-bt.txt".to_string()
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
        format!("/tmp/ccec-{}.log", display)
    } else {
        "/tmp/ccec.log".to_string()
    }
}

pub fn get_prev_log_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/ccec-{}-prev.log", display)
    } else {
        "/tmp/ccec-prev.log".to_string()
    }
}

pub fn get_xprop_path(wid: u64) -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/ccec-xprop-{}-{}", wid, display)
    } else {
        format!("/tmp/ccec-xprop-{}", wid)
    }
}
