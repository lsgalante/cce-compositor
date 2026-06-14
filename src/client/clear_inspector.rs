use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection, Dispatch, QueueHandle,
};
use serde_json::Value;

// Import generated client protocols from cce library
use crate::protocol::clear_inspector::client::zclear_inspector_v1::{self, ZclearInspectorV1};

struct InspectorState {
    inspector: Option<ZclearInspectorV1>,
    done: bool,
    surfaces: Vec<InspectedSurface>,
}

struct InspectedSurface {
    title: String,
    app_id: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    state: String,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for InspectorState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            println!("Debug Global: interface='{}', version={}, name={}", interface, version, name);
            if interface == "zclear_inspector_v1" {
                state.inspector = Some(registry.bind::<ZclearInspectorV1, _, _>(name, version, qh, ()));
            }
        }
    }
}

impl Dispatch<ZclearInspectorV1, ()> for InspectorState {
    fn event(
        state: &mut Self,
        _proxy: &ZclearInspectorV1,
        event: zclear_inspector_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        #[allow(unreachable_patterns)]
        match event {
            zclear_inspector_v1::Event::InspectedSurface { title, app_id, x, y, width, height, fd, len } => {
                let mut surface_state = String::new();
                if len > 0 {
                    use std::io::Read;
                    let file = std::fs::File::from(fd);
                    let _ = file.take(len as u64).read_to_string(&mut surface_state);
                }
                state.surfaces.push(InspectedSurface {
                    title,
                    app_id,
                    x,
                    y,
                    width,
                    height,
                    state: surface_state,
                });
            }
            zclear_inspector_v1::Event::InspectedSurfaceDone => {
                state.done = true;
            }
            _ => {}
        }
    }
}

fn get_socket_path() -> String {
    match std::env::var("WAYLAND_DISPLAY") {
        Ok(display) => format!("/tmp/cce-client-{}.sock", display),
        Err(_) => "/tmp/cce-client.sock".to_string(),
    }
}

fn send_ipc_commands(commands: &[String]) -> Result<Vec<String>, std::io::Error> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let mut stream = UnixStream::connect(get_socket_path())?;
    
    let mut combined_cmd = String::new();
    for cmd in commands {
        combined_cmd.push_str(cmd);
    }
    
    stream.write_all(combined_cmd.as_bytes())?;
    
    let mut buf = [0u8; 4096];
    let mut response = String::new();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                response.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
    
    let replies = response.lines().map(|line| format!("{}\n", line)).collect();
    Ok(replies)
}

fn find_widget_recursive(val: &Value, target_type: &str, target_label: Option<&str>) -> Option<(f64, f64, f64, f64)> {
    if let Some(obj) = val.as_object() {
        let type_match = obj.get("type")
            .and_then(|t| t.as_str())
            .map(|t| t == target_type)
            .unwrap_or(false);
            
        let label_match = match target_label {
            Some(lbl) => obj.get("label")
                .and_then(|l| l.as_str())
                .map(|l| l.eq_ignore_ascii_case(lbl) || l.to_lowercase().contains(&lbl.to_lowercase()))
                .unwrap_or(false),
            None => true,
        };
        
        if type_match && label_match {
            if let Some(rect_arr) = obj.get("rect").and_then(|r| r.as_array()) {
                if rect_arr.len() == 4 {
                    let rx = rect_arr[0].as_f64().unwrap_or(0.0);
                    let ry = rect_arr[1].as_f64().unwrap_or(0.0);
                    let rw = rect_arr[2].as_f64().unwrap_or(0.0);
                    let rh = rect_arr[3].as_f64().unwrap_or(0.0);
                    return Some((rx, ry, rw, rh));
                }
            }
        }
        
        // Search children
        if let Some(children) = obj.get("children").and_then(|c| c.as_array()) {
            for child in children {
                if let Some(coords) = find_widget_recursive(child, target_type, target_label) {
                    return Some(coords);
                }
            }
        }
    } else if let Some(arr) = val.as_array() {
        for item in arr {
            if let Some(coords) = find_widget_recursive(item, target_type, target_label) {
                return Some(coords);
            }
        }
    }
    None
}

fn print_usage(bin_name: &str) {
    println!("Usage:");
    println!("  {}                       List all inspected surfaces and their widget trees", bin_name);
    println!("  {} click-widget --app-id <app_id> --type <widget_type> [--label <widget_label>]", bin_name);
    println!();
    println!("Options:");
    println!("  -a, --app-id <app_id>       The Wayland application ID (e.g. clear-design-interface)");
    println!("  -t, --type <widget_type>    The widget type (e.g. MenuItem, Button)");
    println!("  -l, --label <widget_label>  Optional widget label to filter by");
}

pub fn run_inspector(args: Vec<String>) {
    let bin_name = args.get(0).map(|s| s.as_str()).unwrap_or("clear-inspector");
    
    let mut app_id = None;
    let mut widget_type = None;
    let mut widget_label = None;
    let mut is_click_widget = false;

    if args.len() > 1 {
        if args[1] == "click-widget" {
            is_click_widget = true;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--app-id" | "-a" => {
                        if i + 1 < args.len() {
                            app_id = Some(args[i+1].clone());
                            i += 2;
                        } else {
                            eprintln!("Error: missing value for --app-id");
                            std::process::exit(1);
                        }
                    }
                    "--type" | "-t" => {
                        if i + 1 < args.len() {
                            widget_type = Some(args[i+1].clone());
                            i += 2;
                        } else {
                            eprintln!("Error: missing value for --type");
                            std::process::exit(1);
                        }
                    }
                    "--label" | "-l" => {
                        if i + 1 < args.len() {
                            widget_label = Some(args[i+1].clone());
                            i += 2;
                        } else {
                            eprintln!("Error: missing value for --label");
                            std::process::exit(1);
                        }
                    }
                    "-h" | "--help" | "help" => {
                        print_usage(bin_name);
                        std::process::exit(0);
                    }
                    _ => {
                        eprintln!("Error: unknown argument '{}'", args[i]);
                        print_usage(bin_name);
                        std::process::exit(1);
                    }
                }
            }
            if app_id.is_none() || widget_type.is_none() {
                eprintln!("Error: --app-id and --type are required for click-widget command");
                print_usage(bin_name);
                std::process::exit(1);
            }
        } else if args[1] == "-h" || args[1] == "--help" || args[1] == "help" {
            print_usage(bin_name);
            std::process::exit(0);
        } else {
            eprintln!("Error: unknown command '{}'", args[1]);
            print_usage(bin_name);
            std::process::exit(1);
        }
    }

    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to Wayland display socket: {:?}", e);
            std::process::exit(1);
        }
    };

    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let mut state = InspectorState {
        inspector: None,
        done: false,
        surfaces: Vec::new(),
    };

    let inspector = match globals.bind::<ZclearInspectorV1, _, _>(&qh, 1..=1, ()) {
        Ok(ins) => ins,
        Err(e) => {
            eprintln!("Error: zclear_inspector_v1 global protocol not found on Wayland registry: {:?}", e);
            eprintln!("Ensure clear-river is running and supports the inspector protocol.");
            std::process::exit(1);
        }
    };

    // Request active inspected surfaces
    inspector.get_inspected_surfaces();

    // Dispatch until completed
    while !state.done {
        event_queue.blocking_dispatch(&mut state).unwrap();
    }

    if is_click_widget {
        let target_app_id = app_id.as_deref().unwrap();
        let target_type = widget_type.as_deref().unwrap();
        let matched_surface = state.surfaces.iter().find(|s| s.app_id.eq_ignore_ascii_case(target_app_id));
        if let Some(surface) = matched_surface {
            if let Ok(json_val) = serde_json::from_str::<Value>(&surface.state) {
                let mut max_logical_w = 0.0;
                if let Some(arr) = json_val.as_array() {
                    for item in arr {
                        if let Some(obj) = item.as_object() {
                            if let Some(rect_arr) = obj.get("rect").and_then(|r| r.as_array()) {
                                if rect_arr.len() == 4 {
                                    let rx_val = rect_arr[0].as_f64().unwrap_or(0.0);
                                    let rw_val = rect_arr[2].as_f64().unwrap_or(0.0);
                                    if rx_val == 0.0 && rw_val > max_logical_w {
                                        max_logical_w = rw_val;
                                    }
                                }
                            }
                        }
                    }
                }
                let scale = if max_logical_w > 0.0 {
                    (surface.width as f64) / max_logical_w
                } else {
                    1.0
                };

                if let Some((rx, ry, rw, rh)) = find_widget_recursive(&json_val, target_type, widget_label.as_deref()) {
                    let target_x = (surface.x as f64 + (rx + rw / 2.0) * scale) as i32;
                    let target_y = (surface.y as f64 + (ry + rh / 2.0) * scale) as i32;
                    println!("Found matching widget! Coordinate local bounds: [{}, {}, {}, {}], global target: ({}, {}), scale: {}", rx, ry, rw, rh, target_x, target_y, scale);
                    
                    let commands = vec![
                        format!("focus-window {}\n", target_app_id),
                        format!("pointer-move-to {} {}\n", target_x, target_y),
                        format!("pointer-click left\n"),
                    ];
                    match send_ipc_commands(&commands) {
                        Ok(replies) => {
                            for r in replies {
                                print!("{}", r);
                            }
                        }
                        Err(e) => {
                            eprintln!("Error sending IPC commands to cce-client socket: {:?}", e);
                            std::process::exit(1);
                        }
                    }
                } else {
                    eprintln!("Error: Element of type '{}'{} not found in surface '{}'", 
                        target_type, 
                        widget_label.as_ref().map(|l| format!(" with label '{}'", l)).unwrap_or_default(),
                        target_app_id
                    );
                    std::process::exit(1);
                }
            } else {
                eprintln!("Error: Failed to parse widget state JSON for surface '{}'", target_app_id);
                std::process::exit(1);
            }
        } else {
            eprintln!("Error: Surface with app_id '{}' not found", target_app_id);
            std::process::exit(1);
        }
        return;
    }

    // Print results
    println!("Found {} active inspected surface(s):", state.surfaces.len());
    println!("{}", "=".repeat(60));

    for (idx, surface) in state.surfaces.iter().enumerate() {
        println!("Surface #{}:", idx + 1);
        println!("  Title:       {}", surface.title);
        println!("  App ID:      {}", surface.app_id);
        println!("  Position:    (x: {}, y: {})", surface.x, surface.y);
        println!("  Dimensions:  {}x{} px", surface.width, surface.height);
        
        // Pretty print widget state if it's JSON
        print!("  Element Tree: ");
        if let Ok(json_val) = serde_json::from_str::<Value>(&surface.state) {
            if let Ok(pretty_json) = serde_json::to_string_pretty(&json_val) {
                // Indent pretty JSON for clean display
                let indented = pretty_json.lines()
                    .map(|line| format!("    {}", line))
                    .collect::<Vec<String>>()
                    .join("\n");
                println!("\n{}", indented);
            } else {
                println!("{}", surface.state);
            }
        } else {
            println!("{}", surface.state);
        }
        println!("{}", "=".repeat(60));
    }
}
