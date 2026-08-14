# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`cce-fx` is a standalone Wayland compositor + tiling window manager written in Rust,
built directly on **wlroots 0.20** (via FFI) and a vendored **scenefx** for
blur/rounded-corner scene effects. It began as a Rust rewrite of the
[river](https://isaacfreund.com/software/river) compositor — hence the GPL-3.0
license, `SPDX-FileCopyrightText: © 2020 The River Developers` headers, and river
protocol XML files you'll see throughout `src/server/` and `protocol/`.

This crate lives inside a larger Cargo workspace (the workspace root is the **parent**
directory `../Cargo.toml`, which lists ~20 `cce-*` sibling apps). This crate is the
compositor; the siblings (`cce-status-interface`, `cce-system-interface`, etc.) are
clients that talk to it over its sockets. Intra-workspace dependencies: `cce-ui`
(`../cce-ui`, config helpers) and **`cce-window-manager`** (`../cce-window-manager`,
its own repo) — the pure-Rust window-management **policy layer** (arrange pass,
`TilingMode`, saved state, the `Policy`/`Compositor` trait boundary, slotmap). It was
extracted from this crate's `src/server/policy/`; `src/lib.rs` re-exports it as
`crate::policy` / `crate::tiling` / `crate::slotmap`, so mechanism code keeps using
the historical paths.

## Build / run / install

```sh
make build      # cargo build --release
make run        # cargo run --bin cce-fx
make install    # build + install to ~/.local/bin (see below)
make clean      # cargo clean
```

`make install` builds, then delegates to `./scripts/ccebuild install --no-build cce-fx`,
which installs `cce-fx` (symlinked as `cce`), `ccectl`, the `scripts/*` helpers, and
`gpu-watcher.service` into `~/.local/bin` / `~/.config/systemd/user`. It reads binaries
from `../target/release/` because the workspace target dir is at the parent. The recipe
invokes the in-repo `./scripts/ccebuild` rather than the one on `PATH`: this crate is
what *installs* ccebuild, so it cannot depend on it already being present.

### `scripts/ccebuild` — the DE-wide build/install tool

This crate owns **`ccebuild`**, the entry point for building and installing the whole
workspace (see the workspace guide `./WORKSPACE.md` for the full command list). It lives here
because this crate already ships helper scripts to `~/.local/bin`, and because the
workspace root is not a git repo so nothing there can be versioned.

It derives every binary from `cargo metadata` instead of hand-written lists — the
per-crate Makefiles used to name their binaries manually, which silently left crates
with extra `[[bin]]` targets uninstalled. All the crate Makefiles are now thin wrappers
around it. When touching it, keep two invariants:

- **`prune` detects dead crates from `.fingerprint/` only.** Those dirs are exactly
  `<pkg>-<hex hash>`. Deriving names from `deps/` instead picks up incremental
  artifacts like `cce_terminal-0qsvll1iqr9dj` whose non-hex suffix survives stripping
  and looks like an unknown crate — that false positive selected *live* caches for
  deletion. `incremental/` is excluded from deletion for the same reason: a pattern
  loose enough to match those suffixes also matches live siblings like
  `cce-authenticator`.
- **Never widen the artifact glob.** `cce-status*` also matches the live
  `cce-status-interface`, and `cce*` matches the entire tree (a 180G false reading).
  Matching is anchored: a basename must equal a dead crate name exactly, or that name
  plus a hex hash.

`ccebuild restart` deliberately cannot reach the compositor: `cce-fx` is not a user
unit (startcce launches it), and restarting it would tear down the session.

Building emits a harmless warning that per-package `[profile.*]` in this `Cargo.toml`
is ignored because profiles are only honored at the workspace root.

### Two binaries

- **`cce-fx`** (`src/bin/cce.rs`, symlinked to `cce`) — the compositor server.
  Any arg other than `client`/`help` just starts the server (`cce_fx::run_server()`).
- **`ccectl`** (`src/bin/ccectl.rs`) — thin IPC client; all logic is in
  `src/cce_ctl.rs` (`run_cce_ctl`). Run `ccectl` with no args to see the full command
  list (layout, mode, pointer-*, key*, bind, spawn, notify, exit, …).

### System dependencies (checked by `build.rs`)

Native libs via `pkg-config`: `wlroots-0.20`, `wayland-server`, `xkbcommon`,
`pixman-1`, `libinput`, `libevdev`; linked directly: `GLESv2`, `EGL`, `drm`, `gbm`,
`lcms2`. Also required at build time: `meson` + `ninja` (to compile the vendored
`scenefx/` statically on first build), `wayland-scanner`, and the system
`wayland-protocols` XML files under `/usr/share/wayland-protocols/`.

## Tests

Tests are sparse (unit tests in `config.rs`, `window_manager.rs`; the arrange/slotmap
tests live in the sibling `cce-window-manager` crate — run them with
`cargo test -p cce-window-manager`). The library crate name is `cce_fx` (underscored).

```sh
cargo test --lib                  # all library tests
cargo test --lib <name>           # single test by (substring) name
cargo test --lib config::         # tests in the config module
```

## Build pipeline (`build.rs`)

`build.rs` does a lot before Rust compiles:
1. Runs `meson setup build` (first time) + `meson compile` inside `scenefx/`, static.
2. Generates server headers for upstream protocols and header + `private-code` C for
   the custom `river-*` / `cce-*` protocols (`protocol/`), using `wayland-scanner`.
   `clean_xml` reorders files whose XML declaration follows a leading comment.
3. Compiles `src/server/wlroots_log_wrapper.c` + the generated protocol `.c` files
   into a static `wlroots_log_wrapper` lib.
4. Runs `bindgen` over `wrapper.h` → `$OUT_DIR/bindings.rs`, blocklisting a handful of
   types that are hand-defined `#[repr(C)]` in Rust instead.

## Architecture

Everything lives under `src/server/` and is re-exported flat from `src/lib.rs` via
`#[path = ...]` module declarations. FFI-heavy: expect large `unsafe` blocks, raw
pointers into wlroots C structs, and `wl_listener` callbacks throughout.

### Key FFI idiom — `container_of!`

`src/server/server.rs` defines the `container_of!` macro (the Rust equivalent of
Zig's `@fieldParentPtr` / the C `wl_container_of`). wlroots delivers events through
embedded `wl_listener` fields; callbacks use `container_of!(listener, Struct, field)`
to recover the owning Rust struct from a listener pointer. Many wlroots structs are
also redefined as hand-written `#[repr(C)]` mirrors in `server.rs` because bindgen
treats them as opaque.

### Central files (by size/importance)

- **`server.rs`** — `Server` struct: owns the wlroots backend, renderer, `wl_display`,
  xwayland, and all the manager sub-objects. `Server::init()` / `deinit()` wire up
  every wlroots global. `run_server.rs` is the entry point: parses args, inits the
  server, loads config + persisted state, adds the wayland socket, spawns the init
  program (`~/.config/cce/init` via `sh -c`) and the IPC + status servers, then
  `wl_display_run`.
- **`window_manager.rs`** (~3900 lines) — the heart of the mechanism side. Holds the
  WM state, the camera fields, window lists, the IPC command dispatcher
  `process_ipc_command()`, the `Policy::action` snapshot builder
  (`build_action_ctx`) and the `Compositor` command applier. IPC requests arrive on
  an mpsc channel drained by a wlroots event-loop timer (`handle_ipc_timer`) so all
  mutation happens on the main thread. Decision logic (camera math, action
  dispatch, snapping, refocus, grid geometry) lives in `cce-window-manager`.
- **`window.rs`** (~3900 lines) — per-window model and rendering (borders, blur,
  viewport transforms).
- **`crate::tiling`** (from `cce-window-manager`) — `TilingMode` enum: `Floating`,
  `Tiled` (grid-aligned; the window reports xdg maximized), `Fullscreen`,
  `Popup`, `Overlay`, `Status`. Tiled-ness is geometric: the seat op's end
  (`seat.rs::op_end`) promotes/demotes via `policy::snap::is_cell_aligned`.
- Input stack: `input_manager.rs`, `seat.rs`, `cursor.rs`, `keyboard*.rs`,
  `xkb_*.rs`, `libinput_*.rs`, `pointer_*.rs`, `tablet*.rs`, `text_input.rs`,
  `input_relay.rs`/`input_popup.rs` (IME).
- Shell/surface: `xdg_toplevel.rs`, `xdg_popup.rs`, `shell_surface.rs`,
  `layer_shell.rs`, `xwayland_window.rs`, `xwayland_override_redirect.rs`,
  `drag_icon.rs`, `wm_node.rs`.
- Output: `output.rs`, `output_manager.rs`. Session: `lock_manager.rs`,
  `idle_inhibit_manager.rs`. Rendering: `scene.rs`, `scene_node_data.rs`.

### Config

Loaded on startup from **`$XDG_CONFIG_HOME/cce/config.kdl`** (falls back to
`~/.config/cce/config.kdl`). An adjacent `input.kdl` is merged in for key bindings and
input settings. **The format is KDL** (via the `kdl` crate; `parse_kdl_config`).
`config.rs` maps parsed values onto `WindowManager` state (layout gaps,
border/blur/desktop styling, keybindings → `Action`s, startup programs, output/display
settings). Live reconfiguration comes in over IPC (`ccectl reload`, `bind`, `layout …`,
`config-done`, etc.).

Persistent window state is saved to **`~/.local/state/cce/state.json`**
(`XDG_STATE_HOME/cce/state.json`) on shutdown and restored on start
(`save_state` / `load_state` / `spawn_restored_windows`).

### IPC & status sockets

- **Control socket** `/tmp/cce-{WAYLAND_DISPLAY}.sock` (`ipc_server.rs`): line-oriented
  request/reply over a Unix socket. `ccectl` / `cce_ctl.rs` is the client.
- **Status socket** `/tmp/cce-status-{WAYLAND_DISPLAY}.sock` (`status_server.rs`): runs
  on its own thread; a client sends one subscription line (`layout`, `title`,
  `modifiers`, or `dismiss`) and receives text lines on every change. This feeds the
  status bar (`cce-status-interface`). The main loop pushes updates through a
  `StatusSender` mpsc handle.

## Conventions

- This is systems FFI code: raw pointers, `unsafe`, and manual wlroots listener wiring
  are the norm. When adding a wlroots event handler, follow the existing pattern —
  embed a `wl_listener`, register it, and recover `self` with `container_of!`.
- Keep river's SPDX/copyright headers on files that carry them.
- `scratch/` and `scratch/*` (and the many `.png`/`.log`/`patch*.py` files in the
  parent dir) are ad-hoc debugging artifacts, not part of the build.
