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
    print("  layout <gap|gap_top|gap_left|gap_right|gap_bottom|offset|grid_gap|bar_height> <value>");
    print("  close");
    print("  minimize");
    print("  focus-next");
    print("  focus-prev");
    print("  focus-up | focus-down | focus-left | focus-right");
    print("  window-switcher            # open the alt-tab window switcher (cce-cloud overlay)");
    print("  fullscreen");
    print("  mode-next");
    print("  mode-next-shared");
    print("  zoom-in");
    print("  zoom-out");
    print("  zoom-reset");
    print("  pan-left");
    print("  pan-right");
    print("  pan-up");
    print("  pan-down");
    print("  overlay-left");
    print("  overlay-right");
    print("  focus-window <app_id>");
    print("  close-window <app_id|id> [title-substring]  # close a specific window");
    print("  center-window [<app_id>]   # pan focused/named window on-screen; replies x= y= w= h=");
    print("  move-window <square> [<app_id>] # put focused/named window on a desktop square (e.g. C-9)");
    print("  place-next <app_id> <x> <y> # one-shot: next map of app_id lands near this layout pos");
    print("  overview");
    print("  windows [--json]           # list windows; --json emits one JSON object per line");
    print("  status-hide-mode [true|false]");
    print("  adjust-position-mode [true|false|query]");
    print("  exit");
    print("  restart");
    print("  restart-compositor        # exit cleanly; cce-display-manager relaunches the session");
    print("  reload");
    print("  repeat <rate> <delay>");
    print("  input <device_name|*> scroll-factor <value>");
    print("  config-done");
    print("  spawn <command>");
    print("  screenshot                          # capture the screen to ~/Pictures/screenshots");
    print("  screenshot region <x> <y> <w> <h>   # capture an on-screen region (logical px)");
    print("  screenshot window [app_id|id]       # capture a window (focused if omitted; works off-screen)");
    print("  debug-buffers [app_id|id]           # dump a window's scene buffers (pos/dest/natural/surface)");
    print("  notify <title> [body]");
    print("  bind <mods> <keysym> <action> [args...]");
    print("  pbind <mods> <button> <action>");
    print("  retile");
    print("  mode <floating|tiled|fullscreen|popup|overlay> <app_id> [title]");
    print("  pointer-location");
    print("  pointer-move-to <x> <y>            (layout pixels)");
    print("  pointer-move-by <dx> <dy>");
    print("  pointer-scroll <dy> [dx]           (positive dy scrolls down; 15 = one notch)");
    print("  pointer-click [button]             (left|right|middle|back|forward or evdev code)");
    print("  pointer-press [button]             (held until pointer-release — drives drags)");
    print("  pointer-release [button]");
    print("  migrate-input                      (local: move config.kdl keybindings to input.kdl)");
    print("  keypress <keycode>                 (evdev code; press+release to the focused client)");
    print("  key-down <keycode>                 (modifier codes — ctrl 29/97, shift 42/54,");
    print("  key-up <keycode>                    alt 56/100, super 125/126 — update client");
    print("                                      xkb state, so e.g. 29+36 lands as ctrl+j)");
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

    // Local file operation — no compositor needed.
    if args[1] == "migrate-input" {
        crate::migrate_input::run();
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
