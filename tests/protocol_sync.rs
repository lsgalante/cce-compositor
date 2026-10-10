//! The custom protocol XMLs the compositor shares with cce-ui, checked
//! byte-identical against cce-ui's copies.
//!
//! Both crates need their own copy (each builds standalone from its own git
//! repository: cce-ui generates its client code from `protocol/` with
//! wayland-scanner's macros, the compositor its server header in build.rs),
//! so the copies are kept in step by this test rather than by a shared path.
//! cce-ui's copy is canonical. They had already drifted once: the
//! compositor's carried `enum="river_output_v1.presentation_mode"`, naming an
//! interface the file no longer defines, which cce-ui's Rust scanner could
//! not resolve and had dropped.
//!
//! Skips (passes) when there is no sibling cce-ui checkout, as in a
//! standalone clone of this repository.

use std::path::Path;

const SHARED: &[&str] = &["cce-window-management-v1.xml", "cce-inspector-v1.xml"];

#[test]
fn shared_protocols_match_cce_ui() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui = here.join("../cce-ui/protocol");
    if !ui.is_dir() {
        eprintln!("no sibling cce-ui checkout; skipping");
        return;
    }
    let mut drifted = Vec::new();
    for name in SHARED {
        let ours = std::fs::read(here.join("protocol").join(name)).expect(name);
        let theirs = std::fs::read(ui.join(name)).expect(name);
        if ours != theirs {
            drifted.push(*name);
        }
    }
    assert!(
        drifted.is_empty(),
        "protocol XMLs differ from cce-ui's (canonical) copies: {drifted:?}; \
         copy cce-ui/protocol/<name> over cce-compositor/protocol/<name>"
    );
}
