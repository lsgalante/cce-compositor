use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection, Dispatch, QueueHandle,
};
use serde_json::Value;

// Import generated client protocols from ccec crate
mod protocol;
use protocol::clear_inspector::client::zclear_inspector_v1::{self, ZclearInspectorV1};

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
        match event {
            zclear_inspector_v1::Event::InspectedSurface { title, app_id, x, y, width, height, state: surface_state } => {
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
        }
    }
}

fn main() {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to Wayland display socket: {:?}", e);
            std::process::exit(1);
        }
    };

    let (_globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let _qh = event_queue.handle();

    let mut state = InspectorState {
        inspector: None,
        done: false,
        surfaces: Vec::new(),
    };

    // Populate registry globals
    event_queue.roundtrip(&mut state).unwrap();

    let inspector = match state.inspector.take() {
        Some(ins) => ins,
        None => {
            eprintln!("Error: zclear_inspector_v1 global protocol not found on Wayland registry.");
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
        print!("  Widget Tree: ");
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
