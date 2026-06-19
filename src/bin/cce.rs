use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "client" => {
                println!("cce: standalone client mode is deprecated in the monolithic architecture");
                std::process::exit(0);
            }
            "control" => {
                let mut control_args = vec!["cce-ctl".to_string()];
                control_args.extend(args.iter().skip(2).cloned());
                cce::run_cce_ctl(control_args);
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
    println!("  (default)          Start the monolithic compositor and window manager");
    println!("  control            Run IPC control commands (e.g. cce control layout gap 10)");
}
