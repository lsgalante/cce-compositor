use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "client" => {
                println!("cce: standalone client mode is deprecated in the monolithic architecture");
                std::process::exit(0);
            }
            "--help" | "-h" | "help" => {
                print_help();
            }
            _ => {
                cce_fx::run_server();
            }
        }
    } else {
        cce_fx::run_server();
    }
}

fn print_help() {
    println!("usage: cce [options]");
    println!();
    println!("  (default)          Start the monolithic compositor and window manager");
}
