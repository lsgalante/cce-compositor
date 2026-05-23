// clearctl — IPC client for clearwm

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process;

const SOCKET_PATH: &str = "/tmp/clearwm.sock";

fn usage(name: &str) {
    eprintln!("usage: {} <command> [args...]", name);
    eprintln!();
    eprintln!("commands:");
    eprintln!("  layout <gap|gap_top|gap_left|gap_right|gap_bottom|offset|bar_height|border_width|fullscreen_border_width|border_color> <value>");
    eprintln!("  view <1-4>");
    eprintln!("  toggle <1-4>");
    eprintln!("  close");
    eprintln!("  focus-next");
    eprintln!("  windows");
    eprintln!("  exit");
    eprintln!("  restart");
    eprintln!("  reload");
    eprintln!("  repeat <rate> <delay>");
    eprintln!("  config-done");
    eprintln!("  spawn <command>");
    eprintln!("  notify <title> [body]");
    eprintln!("  bind <mods> <keysym> <action> [args...]");
    eprintln!("  pbind <mods> <button> <action>");
    eprintln!("  retile");
    eprintln!("  set-tag <1-4>");
    eprintln!("  mode <cascade|grid|vsplit|hsplit|fullscreen|floating> <app_id> [title]");
    eprintln!("  tag-layout <1-4> <cascade|grid|vsplit|hsplit|fullscreen|floating>");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage(&args[0]);
        process::exit(1);
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
