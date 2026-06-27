// cce-ctl — IPC client for cce
 
use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process;
 
fn get_socket_path() -> String {
    match env::var("WAYLAND_DISPLAY") {
        Ok(display) => format!("/tmp/cce-{}.sock", display),
        Err(_) => "/tmp/cce.sock".to_string(),
    }
}
 
#[allow(dead_code)]
fn get_windows_path() -> String {
    match env::var("WAYLAND_DISPLAY") {
        Ok(display) => format!("/tmp/cce-windows-{}", display),
        Err(_) => "/tmp/cce-windows".to_string(),
    }
}
 
fn usage(name: &str, to_stderr: bool) {
    let print = |s: &str| {
        if to_stderr {
            eprintln!("{}", s);
        } else {
            println!("{}", s);
        }
    };
    print(&format!("usage: {} <command> [args...]", name));
    print("");
    print("commands:");
    print("  layout <gap|gap_top|gap_left|gap_right|gap_bottom|offset|grid_gap|bar_height|border_width|fullscreen_border_width|border_color> <value>");
    print("  view <1-4>");
    print("  close");
    print("  minimize");
    print("  focus-next");
    print("  focus-window <app_id>");
    print("  expose");
    print("  windows");
    print("  exit");
    print("  restart");
    print("  reload");
    print("  repeat <rate> <delay>");
    print("  input <device_name|*> scroll-factor <value>");
    print("  config-done");
    print("  spawn <command>");
    print("  notify <title> [body]");
    print("  bind <mods> <keysym> <action> [args...]");
    print("  pbind <mods> <button> <action>");
    print("  retile");
    print("  set-viewport <1-4>");
    print("  mode <cascade|grid|fullscreen|floating|popup|maximized> <app_id> [title]");
    print("  viewport-layout <1-4> <cascade|grid|fullscreen|floating|popup|maximized>");
    print("  pointer-location");
    print("  pointer-move-to <x> <y>");
    print("  pointer-move-by <dx> <dy>");
    print("  pointer-scroll <dx> <dy>");
    print("  pointer-click <button>");
    print("  pointer-press <button>");
    print("  pointer-release <button>");
    print("  keypress <key>");
    print("  key-press <key>");
    print("  key-release <key>");
}
 
pub fn run_cce_ctl(args: Vec<String>) {
    if args.len() < 2 {
        usage(&args[0], true);
        process::exit(1);
    }
 
    if args[1] == "--help" || args[1] == "-h" || args[1] == "help" {
        usage(&args[0], false);
        return;
    }
 
 
    // Connect to IPC socket
    let stream = match UnixStream::connect(get_socket_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("connect: {}", e);
            process::exit(1);
        }
    };
 
    let mut stream = stream;
    // Build command string from args
    let cmd = args[1..].join(" ") + "\n";
    if let Err(e) = stream.write_all(cmd.as_bytes()) {
        eprintln!("write: {}", e);
        process::exit(1);
    }
 
    // Read response
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let s = String::from_utf8_lossy(&buf[..n]);
                print!("{}", s);
            }
            Err(e) => {
                eprintln!("read: {}", e);
                break;
            }
        }
    }
}
