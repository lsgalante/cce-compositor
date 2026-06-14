use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "client" => {
                cce::run_client();
            }
            "control" => {
                let mut control_args = vec!["clearctl".to_string()];
                control_args.extend(args.iter().skip(2).cloned());
                cce::run_clearctl(control_args);
            }
            "inspect" | "inspector" => {
                let mut inspect_args = vec!["clear-inspector".to_string()];
                inspect_args.extend(args.iter().skip(2).cloned());
                cce::run_inspector(inspect_args);
            }
            "--help" | "-h" | "help" => {
                print_help();
            }
            _ => {
                cce::run_server();
            }
        }
    } else {
        cce::run_server();
    }
}

fn print_help() {
    println!("usage: cce <subcommand> [options]");
    println!();
    println!("subcommands:");
    println!("  (default)          Start the compositor server and embedded client");
    println!("  client             Start the standalone Wayland window manager client");
    println!("  control            Run IPC control commands (e.g. cce control layout gap 10)");
    println!("  inspect            Run the widget tree inspector");
}
