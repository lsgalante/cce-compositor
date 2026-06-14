use std::env;
 
pub fn get_socket_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/cce-client-{}.sock", display)
    } else {
        "/tmp/cce-client.sock".to_string()
    }
}
 
pub fn get_status_socket_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/cce-client-status-{}.sock", display)
    } else {
        "/tmp/cce-client-status.sock".to_string()
    }
}
 
pub fn get_windows_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/cce-client-windows-{}", display)
    } else {
        "/tmp/cce-client-windows".to_string()
    }
}
 
pub fn get_tags_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/cce-client-tags-{}", display)
    } else {
        "/tmp/cce-client-tags".to_string()
    }
}
 
pub fn get_layout_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/cce-client-layout-{}", display)
    } else {
        "/tmp/cce-client-layout".to_string()
    }
}
 
pub fn get_title_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/cce-client-title-{}", display)
    } else {
        "/tmp/cce-client-title".to_string()
    }
}
 
pub fn get_death_log_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/cce-client-death-{}.log", display)
    } else {
        "/tmp/cce-client-death.log".to_string()
    }
}
 
pub fn get_bt_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/cce-client-bt-{}.txt", display)
    } else {
        "/tmp/cce-client-bt.txt".to_string()
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
        format!("/tmp/cce-client-{}.log", display)
    } else {
        "/tmp/cce-client.log".to_string()
    }
}
 
pub fn get_prev_log_path() -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/cce-client-{}-prev.log", display)
    } else {
        "/tmp/cce-client-prev.log".to_string()
    }
}
 
pub fn get_xprop_path(wid: u64) -> String {
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        format!("/tmp/cce-client-xprop-{}-{}", wid, display)
    } else {
        format!("/tmp/cce-client-xprop-{}", wid)
    }
}
