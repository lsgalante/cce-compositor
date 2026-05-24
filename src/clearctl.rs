// clearctl — IPC client for clearwm

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process;

const SOCKET_PATH: &str = "/tmp/clearwm.sock";

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
    print("  layout <gap|gap_top|gap_left|gap_right|gap_bottom|offset|bar_height|border_width|fullscreen_border_width|border_color> <value>");
    print("  view <1-4>");
    print("  toggle <1-4>");
    print("  close");
    print("  focus-next");
    print("  windows");
    print("  exit");
    print("  restart");
    print("  reload");
    print("  repeat <rate> <delay>");
    print("  config-done");
    print("  spawn <command>");
    print("  notify <title> [body]");
    print("  bind <mods> <keysym> <action> [args...]");
    print("  pbind <mods> <button> <action>");
    print("  retile");
    print("  set-tag <1-4>");
    print("  mode <cascade|grid|vsplit|hsplit|fullscreen|floating|popup> <app_id> [title]");
    print("  tag-layout <1-4> <cascade|grid|vsplit|hsplit|fullscreen|floating|popup>");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage(&args[0], true);
        process::exit(1);
    }

    if args[1] == "--help" || args[1] == "-h" || args[1] == "help" {
        usage(&args[0], false);
        return;
    }

    // Special case: "windows" reads the status file directly
    if args[1] == "windows" {
        match fs::read_to_string("/tmp/clearwm-windows") {
            Ok(content) => print!("{}", content),
            Err(_) => eprintln!("No windows info (clearwm may not be running)"),
        }
        return;
    }

    // Connect to IPC socket
    let stream = match UnixStream::connect(SOCKET_PATH) {
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
