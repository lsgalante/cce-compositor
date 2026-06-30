use std::env;

fn main() {
    println!("cce-wallpaper: standalone background renderer service starting...");
    if let Ok(display) = env::var("WAYLAND_DISPLAY") {
        println!("Connected to compositor WAYLAND_DISPLAY: {}", display);
    } else {
        println!("Error: WAYLAND_DISPLAY environment variable not set.");
    }
}
