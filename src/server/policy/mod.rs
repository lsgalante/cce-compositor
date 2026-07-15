// The window-management policy layer.
//
// This module is the seam for the planned compositor / window-manager split:
// everything under `policy/` is pure Rust — no FFI, no raw wlroots pointers —
// and is intended to eventually move to a separate `cce-window-manager` crate.
// The mechanism side (scene graph, seats, shells, sockets) stays in this crate
// and talks to policy code only through the types in `api`.
//
// Migration status:
//   - `tiling`: pure layout formulas (moved here from `src/server/tiling.rs`).
//   - `state`: persisted session state (moved here from `window_manager.rs`).
//   - `arrange`: pure pieces of `arrange_views()`; currently the status-bar
//     layout engine. `StatusEdge` lives here (re-exported via `window.rs`).
//   - `api`: the `Policy` / `Compositor` trait boundary. Skeleton — defined but
//     not yet driven by `window_manager.rs`.

pub mod api;
pub mod arrange;
pub mod state;
pub mod tiling;
