fn main() {
    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg("-DWLR_USE_UNSTABLE")
        .allowlist_type("pixman_region32")
        .allowlist_type("pixman_box32")
        .layout_tests(false);

    // Let's query pkg-config for the paths programmatically
    let pixman = pkg_config::probe_library("pixman-1").unwrap();
    let wlroots = pkg_config::probe_library("wlroots-0.19").unwrap();
    let scenefx = pkg_config::probe_library("scenefx-0.4").unwrap();
    let libinput = pkg_config::probe_library("libinput").unwrap();
    let libevdev = pkg_config::probe_library("libevdev").unwrap();
    let wayland_server = pkg_config::probe_library("wayland-server").unwrap();
    let xkbcommon = pkg_config::probe_library("xkbcommon").unwrap();

    let mut include_paths = vec![std::path::PathBuf::from("target/debug/build/cce-84dd41093ae4212e/out")];
    include_paths.extend(pixman.include_paths);
    include_paths.extend(wlroots.include_paths);
    include_paths.extend(scenefx.include_paths);
    include_paths.extend(libinput.include_paths);
    include_paths.extend(libevdev.include_paths);
    include_paths.extend(wayland_server.include_paths);
    include_paths.extend(xkbcommon.include_paths);

    for path in include_paths {
        builder = builder.clang_arg(format!("-I{}", path.display()));
    }

    let bindings = builder.generate().expect("Failed to generate bindings");
    let code = bindings.to_string();
    if let Some(idx) = code.find("pub struct pixman_region32 ") {
        println!("=== pixman_region32 ===");
        println!("{}", &code[idx..idx+500]);
    } else {
        println!("pixman_region32 not found in generated bindings!");
    }
}
