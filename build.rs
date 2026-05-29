use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/wlroots_log_wrapper.c");
    println!("cargo:rerun-if-changed=wrapper.h");

    // Probe system libraries
    let wlroots = pkg_config::Config::new()
        .atleast_version("0.20.0")
        .probe("wlroots-0.20")
        .expect("wlroots-0.20 is required");

    let scenefx = pkg_config::Config::new()
        .atleast_version("0.4.0")
        .probe("scenefx-0.4")
        .expect("scenefx-0.4 is required");

    let wl_server = pkg_config::probe_library("wayland-server")
        .expect("wayland-server is required");

    let xkb = pkg_config::probe_library("xkbcommon")
        .expect("xkbcommon is required");

    let pixman = pkg_config::probe_library("pixman-1")
        .expect("pixman-1 is required");

    let libinput = pkg_config::probe_library("libinput")
        .expect("libinput is required");

    let libevdev = pkg_config::probe_library("libevdev")
        .expect("libevdev is required");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Helper to fix XML files starting with comments instead of the XML declaration
    let clean_xml = |src: &str, dst: &std::path::Path| {
        let content = std::fs::read_to_string(src).expect("Failed to read XML source");
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        if lines.len() > 1 && lines[0].starts_with("<!--") && lines[1].starts_with("<?xml") {
            lines.swap(0, 1);
        }
        let cleaned = lines.join("\n");
        std::fs::write(dst, cleaned).expect("Failed to write cleaned XML");
    };

    // Generate upstream protocol headers (header-only)
    let upstream_protocols = vec![
        ("wlr-layer-shell-unstable-v1.xml", "protocol/upstream/wlr-layer-shell-unstable-v1.xml"),
        ("wlr-output-power-management-unstable-v1.xml", "protocol/upstream/wlr-output-power-management-unstable-v1.xml"),
        ("virtual-keyboard-unstable-v1.xml", "protocol/upstream/virtual-keyboard-unstable-v1.xml"),
    ];

    for (name, path) in upstream_protocols {
        let temp_xml = out_dir.join(format!("{}-temp.xml", name));
        clean_xml(path, &temp_xml);
        let header_name = name.replace(".xml", "-protocol.h");
        let status = std::process::Command::new("wayland-scanner")
            .args(&[
                "server-header",
                temp_xml.to_str().unwrap(),
                out_dir.join(&header_name).to_str().unwrap(),
            ])
            .status()
            .expect("failed to execute wayland-scanner");
        assert!(status.success(), "wayland-scanner server-header failed for {}", name);
    }

    // Generate custom river protocol headers and private-code C files
    let custom_protocols = vec![
        ("river-window-management-v1.xml", "protocol/river-window-management-v1.xml"),
        ("river-xkb-bindings-v1.xml", "protocol/river-xkb-bindings-v1.xml"),
        ("river-layer-shell-v1.xml", "protocol/river-layer-shell-v1.xml"),
        ("river-input-management-v1.xml", "protocol/river-input-management-v1.xml"),
        ("river-libinput-config-v1.xml", "protocol/river-libinput-config-v1.xml"),
        ("river-xkb-config-v1.xml", "protocol/river-xkb-config-v1.xml"),
    ];

    let mut generated_c_files = Vec::new();

    for (name, path) in custom_protocols {
        let temp_xml = out_dir.join(format!("{}-temp.xml", name));
        clean_xml(path, &temp_xml);

        let header_name = name.replace(".xml", "-protocol.h");
        let status_h = std::process::Command::new("wayland-scanner")
            .args(&[
                "server-header",
                temp_xml.to_str().unwrap(),
                out_dir.join(&header_name).to_str().unwrap(),
            ])
            .status()
            .expect("failed to execute wayland-scanner");
        assert!(status_h.success(), "wayland-scanner server-header failed for {}", name);

        let code_name = name.replace(".xml", "-protocol.c");
        let code_path = out_dir.join(&code_name);
        let status_c = std::process::Command::new("wayland-scanner")
            .args(&[
                "private-code",
                temp_xml.to_str().unwrap(),
                code_path.to_str().unwrap(),
            ])
            .status()
            .expect("failed to execute wayland-scanner");
        assert!(status_c.success(), "wayland-scanner private-code failed for {}", name);

        generated_c_files.push(code_path);
    }

    // Compile the C wrapper and protocol C files
    let mut build = cc::Build::new();
    build.file("src/wlroots_log_wrapper.c")
        .define("WLR_USE_UNSTABLE", None)
        .flag("-std=c99")
        .flag("-O2")
        .include(&out_dir);

    for c_file in generated_c_files {
        build.file(c_file);
    }

    // Add include paths for scenefx, wlroots, and wayland-server
    for path in &scenefx.include_paths {
        if path.to_string_lossy().contains("wlroots-0.19") {
            continue;
        }
        build.include(path);
    }
    for path in &wlroots.include_paths {
        build.include(path);
    }
    for path in &wl_server.include_paths {
        build.include(path);
    }
    build.compile("wlroots_log_wrapper");


    // Setup bindgen
    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg("-DWLR_USE_UNSTABLE")
        .layout_tests(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    // Pass include paths to bindgen clang argument parser
    let mut include_paths = vec![out_dir.clone()];
    for path in scenefx.include_paths.clone() {
        if path.to_string_lossy().contains("wlroots-0.19") {
            continue;
        }
        include_paths.push(path);
    }
    include_paths.extend(wlroots.include_paths.clone());
    include_paths.extend(wl_server.include_paths.clone());
    include_paths.extend(xkb.include_paths.clone());
    include_paths.extend(pixman.include_paths.clone());
    include_paths.extend(libinput.include_paths.clone());
    include_paths.extend(libevdev.include_paths.clone());

    for path in include_paths {
        builder = builder.clang_arg(format!("-I{}", path.display()));
    }

    let bindings = builder
        .blocklist_item("FP_NAN")
        .blocklist_item("FP_INFINITE")
        .blocklist_item("FP_ZERO")
        .blocklist_item("FP_SUBNORMAL")
        .blocklist_item("FP_NORMAL")
        .blocklist_item("wl_listener")
        .blocklist_item("wlr_addon")
        .blocklist_item("wlr_input_device")
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
