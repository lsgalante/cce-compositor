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

It also installs the **`.desktop` entries** crates ship at their own root into
`$XDG_DATA_HOME/applications` (then `update-desktop-database`), discovered by
`desktop_entries()` and filtered per package exactly like units. Discovery is
`-maxdepth 2` — crate root only — so keep the file next to `Cargo.toml`; units get
`-maxdepth 3` because `cce-compositor/scripts/` holds one, which is what the shared
`file_crate_dir()` helper unwraps. These entries were unversioned hand-written files
in `~/.local/share/applications` until 2026-08-16; see `./WORKSPACE.md` for the
`Exec=`/`MimeType=` rules that go with them.

**App icons** install from any crate's `hicolor/` tree (`app_icons()`), mirrored
verbatim into `$XDG_DATA_HOME/icons/hicolor/` — so an icon's size and context are
its directory, not a rule in the script, and `48x48/apps` would need no edit here.
In practice the tree is `cce-icons/hicolor/`, whose files are all **symlinks** into
its own `svg/`; that makes `-type l` load-bearing in the `find`, because `-type f`
alone matches none of them and would report a clean install of nothing. The install
is deliberately *not* package-filtered: `cce-icons` has no `Cargo.toml`, so
`crate_selected()` can never match it. See `cce-icons/hicolor/README.md` for why the
target is `hicolor` rather than the `cce` theme, and why it ships no `index.theme`.

**Helper scripts** are installed from **any** crate's `scripts/` dir, not just
this one's (`crate_scripts()`, same per-package filtering). A script belongs in
the repo whose code it is about — `cce-keyring-selftest` reports on the keyring
chain, so it ships from `cce-display-manager/scripts/` — and anything installed
from outside a repo is unversioned and gone on a fresh clone, which is how that
script and the `.desktop` entries above both started out.

`ccebuild restart` deliberately cannot reach the compositor: `cce-fx` is not a user
unit (startcce launches it), and restarting it would tear down the session.

Building emits a harmless warning that per-package `[profile.*]` in this `Cargo.toml`
is ignored because profiles are only honored at the workspace root.

### `scripts/cce-shadow` — an invisible session to verify in

`cce-shadow start` runs a second `cce-fx` on the wlroots **headless** backend: a
real output, real scenefx rendering, real clients, but nothing is ever scanned
out, so it does not touch the screen, focus or input of whoever is using the
machine. It is the replacement for the nested (wayland-backend) approach, which
needed a visible window and had to be re-centred before every capture.

```sh
cce-shadow [--instance NAME] start [--new|--fresh|--restore|--scale N|--gpu PATH|--exec CMD]
cce-shadow ctl windows          # ccectl against the shadow
cce-shadow spawn cce-files
cce-shadow shot [name]          # PNG path on stdout
cce-shadow list | prune         # instances; reclaim stopped agent-N trees
cce-shadow status | logs | run <cmd> | env | stop [--all]
```

**Instances — how two agents share the machine.** Several shadows run at once,
selected by `--instance NAME` or `CCE_SHADOW_INSTANCE`; each is a directory
under `$CCE_SHADOW_BASE` (default `~/.local/state/cce-shadow`), and the default
name is `default`. `start --new` claims an unused `agent-N` and prints it — the
opening move for an agent that must not disturb another's run. The isolation
falls out of that one directory: separate homes mean separate windows, and a
`stop` sweep that cannot see the other session's clients. The display is not a
collision point either, since `cce-fx` picks its socket with
`wl_display_add_socket_auto` and the script reads the name back out of the log,
so the second compositor lands on a different one unprompted.

Without this, two agents share one session, and each one's `stop` — or plain
`start`, which clears saved window state — tears down the other's run *silently*,
because `start` reports an existing session as success. `prune` exists for the
same reason in reverse: an agent that dies never calls `stop`, and a leaked
headless compositor runs forever. It deletes stopped `agent-N` trees only;
instances named by hand are left alone, since pruning takes their `shots/` too.

**Ownership** closes the other half. Naming instances stops two sessions
sharing one by accident, but not `stop --all` and `prune` reaching across
deliberately — an `agent-1` did vanish mid-verification, tree and all, with
three sessions live on the machine. So `start` records who started the
instance in `run/owner`, and those two commands skip anything a *different
live* session owns, saying so rather than passing over it in silence.
`--force` overrides; targeting an instance by name is never restricted, since
that is deliberate. `list` shows the verdict as `me` / `other` / `orphan` /
`none`.

The token is `<pid>:<starttime>` of the first ancestor that is not a shell —
an agent's `claude`, or a human's terminal emulator. Neither the script nor
its parent works: each invocation is a fresh setsid'd session leader, and
`$PPID` is the throwaway shell of one tool call, dead by the next, so an
instance would read as an orphan to the very session that started it. The
start time is what stops a recycled pid from inheriting someone's ownership.
An unowned instance (one from before this change) or an orphaned one is fair
game — that is the leak `prune` is for.
What stays global across instances is the D-Bus name claims in the script's "Do
not run" list — those are one-at-a-time for the whole machine.

`CCE_SHADOW_DIR` still overrides the tree wholesale, bypassing instance
resolution. A pre-instance tree (`home/` `run/` `shots/` directly under the
base) is migrated into `default` on first use — but *not* while it is still
running, since its pidfile is at the old path and moving it would strand a live
compositor no command could reach again.

Four things in the tree are load-bearing, and each was a bug before it was a
feature:

- **`HOME` is isolated** because screenshots go to a hardcoded
  `$HOME/Pictures/screenshots` and ignore XDG entirely.
- **`XDG_STATE_HOME` is isolated** because `state.json` otherwise restores the
  *live* session's windows, respawning a duplicate of every open app. `start`
  additionally discards the shadow's own `state.json` unless `--restore`, so a
  run never inherits the previous one's windows.
- **`stop` sweeps clients by environment**, matching `HOME=$SHADOW_HOME` in
  `/proc/<pid>/environ`. They cannot be found by process group (the compositor
  `setsid`s what it spawns) and must not be found by name (the live session runs
  the same binaries — matching `cce-files` would kill the user's file manager).
  Skipping the sweep leaves clients alive that reattach when the next `start`
  reuses the display name, which looks exactly like session restore gone wrong.
- **A `notifications { screenshots (bool)false }` key is written into the
  seeded config.** The compositor only defaults this off when the config is
  *unreadable*; a config that exists but omits the key defaults it ON, and the
  seeded config is a copy of the user's, which omits it. Without it every
  capture fires a `notify-send` toast onto the user's real screen, because the
  D-Bus session bus is necessarily shared.

The GPU pin (`--gpu`, default: first non-NVIDIA render node) is not cosmetic:
full-output capture works anywhere, but `screenshot window` reads the client's
imported dmabuf and reports read format `0x0` when the compositor is on the
NVIDIA node and the client rendered elsewhere.

Not reachable this way, so still live-session work: real DRM/KMS modesetting and
page-flip timing, suspend/resume, and libinput hardware paths (gestures, accel)
— injected events do not exercise them.

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
  `modifiers`, `dismiss`, or `backdrop <app_id>`) and receives text lines on every
  change. This feeds the status bar (`cce-status-interface`). The main loop pushes
  updates through a `StatusSender` mpsc handle.

  **`backdrop` is the one per-subscriber topic** — it names the asking segment,
  because the whole point is that the two ends of a bar sit over different things.
  Lines are `<luma> <spread>` (0-100 each) or `unknown`. It answers a question a
  Wayland client cannot: what its translucent module boxes are composited *over*,
  so it can raise its text contrast to match. The measurement is geometry, not a
  readback — the desktop background is drawn from a declarative spec, so
  `backdrop.rs` computes cell-vs-gap coverage under each segment rect on the CPU
  (`Output::measure_status_backdrops`, per frame, gated by `update_status`'s
  equality check). A window overlapping a segment reports maximum spread, since
  its pixels are not knowable from here.

## Conventions

- This is systems FFI code: raw pointers, `unsafe`, and manual wlroots listener wiring
  are the norm. When adding a wlroots event handler, follow the existing pattern —
  embed a `wl_listener`, register it, and recover `self` with `container_of!`.
- Keep river's SPDX/copyright headers on files that carry them.
- `scratch/` and `scratch/*` (and the many `.png`/`.log`/`patch*.py` files in the
  parent dir) are ad-hoc debugging artifacts, not part of the build.
