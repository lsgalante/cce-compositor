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

**Portal declarations** (`<crate>/portals/*.portal`, the file that tells
xdg-desktop-portal a backend's bus name and interfaces) install to
`$XDG_DATA_HOME/xdg-desktop-portal/portals/` the same filtered way
(`portal_files()`); `cce-shortcuts-portal` ships the first. `file_crate_dir()`
must know every such subdirectory name (`scripts`, `dbus`, `portals`) or the
package filter reads the subdirectory as the crate and drops the file.

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
cce-shadow shot-window [name]   # one window rather than the whole output
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

Eighteen modules carry unit tests — `window_manager.rs` (the most of any, among
them the saved-state matchers: same-program borrowing, untitled entries),
`config.rs` (among them `backdrop_compress_params`), `idle.rs`,
`idle_inhibit_manager.rs`, `xwayland_window.rs`, `screenshot.rs`, `window.rs`, `migrate_input.rs`,
`text.rs`, `global_shortcuts.rs` (trigger parsing),
`cursor.rs` (the swipe lean's direction, `swipe_lean`), `min_sizes.rs`,
`selection.rs` (the rubber band's rect and its hit rule),
`touch.rs` (edge-swipe progress, finger centroid/spread tracking, swipe vs pinch),
`ipc_server.rs` (command framing and cutting off a stalled subscriber),
`status_server.rs` (a slow reader, and a client that never reads),
`keyboard_group.rs` (the keys a locked session keeps) and
`sleep_lock.rs` (logind's sleep delay). They cluster where the logic is
pure and the FFI is not, which is the only kind of thing testable in a crate
this deep in wlroots. The arrange/slotmap tests live in the sibling
`cce-window-manager` crate — run them with `cargo test -p cce-window-manager`.
The library crate name is `cce_fx` (underscored).

```sh
cargo test --lib                  # all library tests
cargo test --lib <name>           # single test by (substring) name
cargo test --lib config::         # tests in the config module
```

### `verify/` — behavioral tests in a shadow session

For behavior the unit tests cannot reach (it needs a running compositor),
`verify/` holds self-contained test drivers that start a private `cce-shadow`
instance, drive it with real Wayland clients, and assert on what the
compositor observably does. `verify/clients/` is the shared client crate —
deliberately **not** a workspace member (severed with an empty `[workspace]`,
own `target/`, invisible to ccebuild), built on demand by the drivers:

- **`vkey`** — injects key events through `zwp_virtual_keyboard_v1`
  (wtype-style; evdev keycodes plus `mod:MASK` args for held modifiers).
  This exercises the same `KeyboardGroup::handle_group_key` path hardware
  keys take, so keybindings and builtins fire for injected keys. `vkey hold`
  keeps the virtual keyboard alive until killed: a headless seat has no
  keyboard otherwise, and a Chromium/Electron client that gains focus there
  crashes on a modifiers event with no keymap before it.
- **`float-pair`** — one client, two parentless Floating toplevels with one
  app_id: a 1024x800 main window, then an "Authorize" dialog that insists on
  400x370 (min == max, ignores the configure) and is activated with a token
  BEFORE its first buffer, the way Chromium/Electron open a dialog.
  `--reactivate SECS` later activates the by-then-unfocused main window — an
  activation for an already-mapped window. Prints one milestone per line.
- **`status-stub`** — maps an xdg toplevel with a `cce-status*` app_id 400px
  tall, which `any_expanded_status_segment` reads as an open in-surface menu
  (expanded is geometric: thicker than `layout.bar_height`). It subscribes to
  the status socket's `dismiss` topic, prints one line per push, and shrinks
  to a bar strip on the first one — reacting the way the real bar does.

- **`popup-nest`** — a fixed-size window with a menu (`xdg_popup`) and a
  submenu nested on it, both asking to flip sideways and slide vertically
  the way Chrome's three-dot menu does, printing each popup's configured
  position relative to its parent. `--menu-at`, `--menu`, `--sub-at` and
  `--sub` move and size them.

- **`xembed-icon`** — a legacy X11 tray icon docked the way Wine's systray
  docks one: waits for a `_NET_SYSTEM_TRAY_S0` owner, draws in the visual it
  advertises, sends SYSTEM_TRAY_REQUEST_DOCK, paints one solid colour and
  prints each milestone (docked, embedded, every button it receives).
  `--recolor SECS COLOR` and `--exit-after SECS` exercise the icon updating
  and leaving. `--popup WxH` makes a right-click open an override-redirect
  popup the way a Windows tray app does — bottom-aligned at the click and
  clamped to the screen top, i.e. over a top bar — which reports every move
  and closes on a press outside it: the bridge's popup placement and the
  `clickaway` topic, together. Needs `cce-shadow start --xwayland`; see
  `../cce-status-interface/CLAUDE.md` for the bridge it tests.

- **`or-flip`** — an X11 window that maps override-redirect, then is
  unmapped, has the flag cleared and maps again (`--cycles N`), so wlroots
  emits `set_override_redirect` and the record changes kind. Until
  2026-09-26 the override-redirect side freed its record without dropping it
  from `wm.override_redirects`, and the next frame's `apply_x11_scale`
  segfaulted the compositor: a Wine tray icon handed back to the root window
  by the XEmbed bridge did exactly this and took the live session down. The
  compositor surviving a run is the assertion.

`./verify/popup-constrain-test` drives `popup-nest` to prove submenus are
fitted to the real screen (`xdg_popup.rs::handle_reposition`): a tall
submenu low in the window slides up to fit, and one near the right edge
flips left. wlroots wants the unconstrain box in the ROOT window's surface
coordinates, which each popup finds through `XdgPopup::root_tree`; until
2026-09-25 the box was measured from a submenu's parent menu, which pushed
Chrome's tall submenus past the top of the screen and stopped them flipping
at the right edge.

`./verify/image-selection-test` needs no client of its own: cce-grid and
cce-terminal are the clients, and the desktop images it seeds are what the
overview band and a group move have to pick up (see "Overview
drag-selection" below).

`./verify/escape-dismiss-test` composes the two to prove all three gates of
the Escape-closes-status-menus arm (`handle_builtin_binding`): a chorded
Escape stays out of the arm, a plain Escape while expanded pushes exactly one
dismiss (and the stub's shrink is visible in `ctl windows`), and a plain
Escape with nothing expanded stays quiet. The compositor binary is whatever
`cce-shadow` resolves (installed first, then `target/release`); extra args
pass through to `cce-shadow start`, so `--bin ../target/release/cce-fx` pins
the tree's own build.

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

**"First time" means the guard is `!Path::new("scenefx/build").exists()`** (step
1, `build.rs`) — so once that directory exists `setup` never runs again, and
every later build is `meson compile -C build` alone, which cannot reconfigure.
Meson bakes absolute paths into a configured build dir, so **relocating the
workspace root kills it permanently**: the dir still points at where the tree
used to be, and nothing in the build recovers it. `cargo clean` and `make
clean` both only clear `../target/`; `scenefx/build/` is gitignored
(`.gitignore:6`, `scenefx/.gitignore:2`), so a fresh clone never has one and is
fine, while a *moved* tree carries the dead one along.

It surfaces in `meson compile`, not `meson setup`, which is what makes it
confusing — ninja goes to regenerate `build.ninja` and meson dies with

```
ERROR: Neither source directory '<old absolute path>' nor build directory '.' contain a build file meson.build
```

The old path in that message is the entire diagnosis: it names where the
workspace used to live. Recovery is `rm -rf cce-compositor/scenefx/build` and
one more build to reconfigure, about a minute. Cost an afternoon on
2026-08-28, when a build dir configured under the workspace's former
`~/Dropbox/cce` path survived the move to `~/projects/cce`.

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
- **`window_manager.rs`** (~8.4k lines) — the heart of the mechanism side. Holds the
  WM state, the camera fields, window lists, the IPC command dispatcher
  `process_ipc_command()`, the `Policy::action` snapshot builder
  (`build_action_ctx`) and the `Compositor` command applier. IPC requests arrive on
  an mpsc channel; the IPC thread bumps an eventfd after each send, and that fd is a
  `wl_event_loop_add_fd` source (`handle_ipc_event`) which drains the channel, so all
  mutation happens on the main thread and the loop sleeps until a command exists.
  (It was a 10 ms polling timer until 2026-09-10 — 100 wakeups/s at total idle. The
  status server thread had the same shape, a `try_recv` loop with a 20 ms sleep; it
  now `poll()`s its sockets plus a wake eventfd. Nothing in the compositor should
  tick while idle: a timer that re-arms itself unconditionally is a bug.) Decision logic (camera math, action
  dispatch, snapping, refocus, grid geometry) lives in `cce-window-manager`.
- **`window.rs`** (~5.8k lines) — per-window model and rendering (borders, blur,
  viewport transforms).
- **`crate::tiling`** (from `cce-window-manager`) — `TilingMode` enum: `Floating`,
  `Tiled` (grid-aligned; the window reports xdg maximized), `Fullscreen`,
  `Popup`, `Overlay`, `Status`, `Utility`. A Wine window answers "maximized"
  by maximizing itself, which for a captionless one (Ubisoft Connect) is the
  whole monitor and so a FULLSCREEN request; `XwaylandWindow::absorbs_wine_echo`
  swallows that echo and the tile's held size lets Wine settle on MAXIMIZED.
  The same FULLSCREEN is what such an app's OWN maximize button sends; a
  captionless Wine window whose `_MOTIF_WM_HINTS` functions offer maximize
  (`is_wine_maximize`, read over the XWM's xcb connection) is tiled instead,
  while a WS_POPUP game — no maximize function — still goes fullscreen.
  Tiled-ness is geometric: the seat
  op's end (`seat.rs::op_end`) promotes/demotes via
  `policy::snap::is_cell_aligned`. **A window grabbed Tiled snaps HARD
  through the whole drag** — its move lands on cell starts
  (`snap::snap_move_tiled`) and its resize lands each dragged edge on a cell
  edge, whole cells only (`snap::resize_axis_tiled`, used by both the seat
  op and `get_active_resize_dimensions`) — so it comes out of the drag still
  Tiled; the magnetic pull (`snap_move`, `resize_axis`) is for Floating
  windows deciding whether to tile. `Utility` is the one mode a client asks for
  outright — `cce_window_management.rs` sets it on `set_utility` — and it is a
  self-sizing float: no resize affordance, no saved geometry (see
  `xdg_toplevel.rs`, which sizes it and `Status` from their own content, and
  `xwayland_window.rs`, which excludes it from the tiled report alongside
  `Floating`/`Popup`).
- Input stack: `input_manager.rs`, `seat.rs`, `cursor.rs`, `keyboard*.rs`,
  `xkb_*.rs`, `libinput_*.rs`, `pointer_*.rs`, `tablet*.rs`, `text_input.rs`,
  `input_relay.rs`/`input_popup.rs` (IME).
- Shell/surface: `xdg_toplevel.rs`, `xdg_popup.rs`, `shell_surface.rs`,
  `layer_shell.rs`, `xwayland_window.rs`, `xwayland_override_redirect.rs`,
  `drag_icon.rs`, `wm_node.rs`. An override-redirect window whose WM_CLASS
  class is `cce-xembed-tray` gets no scene node at all
  (`is_xembed_tray_container`): it is the tray bridge's container for a
  legacy X11 tray icon, which X must have mapped for the icon to draw but
  which is shown in the status bar instead (`cce-status-interface`'s
  `cce-xembed-tray`).
  An override-redirect window never takes the keyboard from ANOTHER client
  (`focus_if_desired`): it gets it only when the seat holds nothing or a
  window/popup of its own process. wlroots' `override_redirect_wants_focus`
  cannot tell a Wine tooltip from a Wine menu — both are
  `_NET_WM_WINDOW_TYPE_DIALOG` with `WM_TAKE_FOCUS` and the same Win32
  styles — so until 2026-09-27 the tooltip Wine's `explorer.exe` shows on
  every forwarded tray click took focus from whatever the user was typing
  in, and kept it after closing. The price is keyboard navigation in the
  menu of an app with no focused window (a tray menu); the pointer still
  drives it.
- Output: `output.rs`, `output_manager.rs`. Session: `lock_manager.rs`,
  `idle_inhibit_manager.rs`. Rendering: `scene.rs`, `scene_node_data.rs`.

### A node must draw everything it claims as opaque

`scene_node_opaque_region` (scenefx `wlr_scene.c`) is a promise: whatever a
node reports there is culled from every node beneath it AND from the black
background clear, so a pixel inside it that the node's shader draws at less
than full alpha blends over whatever the buffer last held. An alpha-1
`wlr_scene_rect` with a corner radius reports its box minus the corner
squares, and until 2026-09-28 `quad_round.frag` did not honour that: its
inlined distance was one pixel off vertically (top row alpha 0, the next
0.5) and its AA ramp left every straight edge pixel at 0.5. On the desktop
grid — opaque black rounded cells over the gap-coloured backdrop — that was
a tint of whatever had last covered a cell's top row, constant in the first
row and halving per repaint in the second, invisible after a full redraw
only because the clear colour and the cells are both black. The shader now
takes its distance from `corner_dist`, like the clip and the buffer corner
cut. To look for this kind of residue, screenshot a shadow before and after
drawing over the grid and diff; every desktop pixel should be a multiple of
the gap colour.

### Window move/resize handles

Pointer move and resize exist **only in adjust mode** (overview, or Super
held), and the resize handles are **eight discs inside** the content rect —
one at the midpoint of each side, one on each corner — where the old band
sat outside the edges and the ring that followed hugged them.

- `cursor::get_border_zone` is the hit test: it returns `BorderZone::None`
  outright unless `wm.mode == Overview`, so in normal mode a window cannot be
  dragged or resized at all. Only the pointer is gated — `move_window_*`,
  `ccectl move-window`, and a client repositioning itself all still work in
  normal mode.
- Inside the ring, **all four edges resize**, the top included. Dragging the
  window's body is what moves it in overview, so the top edge no longer has
  to be spent on moving the way the outside band's did.
- The handles are drawn by **one scenefx node**, `wlr_scene_frame`
  (`scenefx/render/fx_renderer/shaders/frame.frag`): eight discs of
  diameter `band` (= `border.handle_width`, screen px), each its own zone.
  The side discs are tangent to their side; a corner disc sits on the
  corner's 45° diagonal, tangent to the rounded corner arc when that arc is
  wider than the disc and tucked into the two straight edges otherwise.
  **`window::handle_disc_layout` is the one layout function**: `draw_borders`
  places the eight invisible square catchers (`border.segments`) from it,
  `cursor::get_border_zone` hit-tests the discs from it (a pixel of slack
  for the rim), and the shader repeats the same arithmetic from the same
  inputs (size, corner radius, band) — keep the three in step. Between two
  discs the pointer reaches the app; the old four full-band catchers are
  disabled for exactly that reason. Not eight rounded scene rects: a scene
  rect takes the renderer's global corner shape, a squircle, so a rect with
  radius half its size is not a circle.
  The shader's zone logic works in TOP-DOWN box-local coordinates
  (`gl_FragCoord` minus the box position, unflipped): `corner_dist` flips
  its own copy, and mirroring the zone coordinate the same way once
  swapped every zone label vertically — the top edge lit the bottom. And a
  hover swap must repaint even when no reveal value moves: in adjust mode
  the discs are already fully revealed, so `step_border_fade` compares the
  hovered zone against the one last drawn (`border_hover_drawn`), or the
  shader keeps showing the previous zone until an unrelated commit repaints.
- **The disc diameter is a SCREEN size, not a world one**, floored at
  `HOVER_BAND_MIN` and capped at a fifth of the window's shorter on-screen
  side. Overview is zoomed *out*, so a handle that scaled with the window
  would be smallest exactly where it is the only way to resize; the cap
  keeps a zoomed-out window from being mostly handle. `draw_borders` and
  `cursor::get_border_zone` each derive it the same way and must stay in
  step.
- **A Floating window lying over the adjust target is dimmed** to
  `border.overlap_opacity` (default 0.4; 1.0 disables) while the mode is
  on, so it does not hide the handles. `Window::adjust_dim_wanted` walks
  the render list bottom-up: only windows ABOVE the target that overlap it
  on screen qualify (one beneath hides nothing). `step_adjust_dim` eases
  `adjust_dim` on the same border-fade timer as the ring, and
  `effective_opacity` folds it into the scene-tree opacity `render_finish`
  sets — set the tree opacity through that, never from
  `rendering_requested.opacity` directly, or the dim is clobbered on the
  next commit. Anything that can change who covers whom re-arms the fade:
  `arrange_views`, `raise_window`, and every `op_update` step.
- The **open/close dissolve** is a third multiplier on the same machinery:
  `Window::map_fade`, stepped by `step_map_fade` on the border-fade timer and
  folded into `effective_opacity` beside `adjust_dim`. `Window::map` starts the
  open ramp (`start_map_fade`; `wants_map_fade` excludes status segments, the
  wallpaper and the grid), and the `fade-out` control-socket command starts the
  close ramp for whichever windows and Overlay layer surfaces belong to the
  CALLER — resolved from `IpcRequest::peer_pid` (SO_PEERCRED), never from a
  name in the command. Layer surfaces run the same ramp on their own timer
  (`LayerSurface::start_fade`) because they are not in `wm.windows`. The ramp
  is LINEAR, unlike the borders' exponential approach: an exponential close
  fade never reaches zero, and the client is holding its surface open against a
  deadline. Durations are `surface { fade in_ms out_ms }`; see
  "Window fades" in WORKSPACE.md for the client half of the contract.
- `handle_width` under `border` in config.kdl is the diameter. `taper`,
  `swell_curve`, `bulge`, `corner_length` and `segment_gap` belonged to the
  retired ring profiles (an even ring, then a wave of hills and valleys):
  still parsed and passed to the node, no longer drawn.
- The shader's zone numbering MUST match `BorderElement::index()`; it is what
  the hovered-zone uniform selects on.
- `window::window_takes_handles` is the single predicate for which windows get
  handles (excluding Popup, Fullscreen, Status, Utility, circular, hidden),
  used by both the hit test and the drawing. Keep those in step: a handle that
  is drawn but not honoured — or honoured but not drawn — is the failure mode
  this arrangement exists to prevent. The grab zone IS the disc (plus a pixel
  of rim), not a band: a press between two discs is a body press and moves.
- **Holding Super is window-adjust mode at zoom 1**: the same handles and
  body-drag as overview, gated by one predicate,
  `WindowManager::window_adjust_active()` (overview OR `adjust_held`).
  But NOT hover-to-focus: the ring lands on the window **under the
  pointer** (`Cursor::adjust_hover`, set by `passthrough` — the same
  target overview uses), focused or not, and focus stays put — so pressing
  Super arms whatever the pointer is already on, and a focus chord pressed
  next acts on the window the user had. **A drag never focuses the window it moves or resizes** (the grab
  paths in `handle_button` call no `seat.focus`; `op_start_pointer` raises
  a Floating one instead); a tap on the band or body — press+release
  without motion — is a click and focuses in `op_end`.
  `adjust_held` is refreshed from the keyboard's modifier mask on every
  modifiers event (`refresh_adjust_held`), which also re-runs the pointer
  passthrough so the ring lands under a still pointer on key-down and the
  app gets its hover back on key-up. `ccectl key-down 125` holds it in a
  shadow (injection bypasses the device mask, so it keeps its own flag).
  A background press with Super held is an ordinary desktop press — only
  overview exits on it.
- Handles are shown on the **adjust target only** — `Window::is_adjust_target`:
  the window under the pointer (`Cursor::adjust_hover`), in overview and
  with Super held alike; a pointer on the background shows none — for as
  long as the mode is on (`step_border_fade`'s `all_on` branch, `draw_borders`'
  `handles_live`, and `get_border_zone` all ask it). Separately, **focus
  follows the pointer in overview** (the ring does not key on it): the
  motion path focuses the hovered toplevel — guarded on an actual change,
  because `seat.focus` raises a Floating window *before* its same-focus
  short-circuit, so an unguarded call would raise and relayout on every
  motion event — and with `suppress_focus_pan` set, so hovering never moves
  the camera; only clicks and the keyboard may. The ring's fade is
  timer-driven, so both `WindowManager::set_mode` and `seat.focus` arm it —
  assigning `self.mode` or `self.focused` directly would leave the ring
  waiting for an unrelated redraw. The hit test and the invisible catcher
  rects are focused-gated too; hover-to-focus is what keeps that workable,
  since reaching a window's edge focuses it on the way.
- A client drawing an in-surface popover (a cce-ui menu — one buffer with
  the window since cce-ui's Phase 6x) hints its rect via
  `zcce_toplevel_v1.set_popover_region` (manager v7); the ring is clipped
  away beneath it (shader `exclusion`) and its band does not grab there, so
  the menu reads as in front of the chrome. The protocol XML lives in BOTH
  repos — cce-ui's copy strips the `enum="river_output_v1..."` attribute its
  scanner cannot resolve; never sync the file over it wholesale.
- The per-side foam clipping the outside band carried is gone: it split a gap
  SHARED with a neighbouring window, and an inside ring shares nothing.
- **Right-click opens the window context menu** — `scripts/cce-app-menu`, a
  `cce-cloud --json` popup like `cce-desktop-menu` and cce-grid's item menu.
  It opens from a right-click on a handle disc in either adjust mode, and in
  **overview from a right-click anywhere on the window**, since the client
  never sees buttons there (`should_block_button`) and the press is the
  compositor's to spend; Overlay (chrome) and Utility (no handles, no mode)
  bodies are excluded. The menu's "Window Mode" page — a second JSON page
  reached through a `target_page` button, which switches pages without
  closing the popup — sets the mode with `ccectl set-mode <mode> <id>`, one
  window by id. Its marks and arrows are cce-cloud's glyphs, not text: the
  mode rows lead with `"● "` / `"○ "` (drawn as the circle glyphs), and the
  page and Back rows carry only their words, cce-cloud adding the chevrons
  from `target_page` (see cce-cloud's CLAUDE.md, the `Json` mode). Not `ccectl mode`, which appends a persistent app_id rule.
  `cce-desktop-menu` carries the same page for the FOCUSED window: the
  background right-click passes it as `-i <id> -a <app_id>` (settable modes
  only) because it drops focus right after the spawn, so the script could
  not ask for it.

None of this is policy — `cce-window-manager` was untouched. The mode is
already in `ActionCtx`, but what a *pointer* may grab is mechanism.

### Overview drag-selection

A left press on the bare desktop in overview, dragged, stretches a rubber
band from the press point, and every window the band touches is selected,
live (`src/server/selection.rs`; state on `WindowManager::selection`). It is
cce-designer's network-cursor region brought to windows: a new drag replaces
the selection, a press on a window outside it drops it, no modifiers. The
hit rule differs on purpose — the designer asks whether a node's cell is
inside the region, but a window spans many cells and the background between
two of them is a strip, so touching is enough.

- **The background press no longer exits overview on the spot.** It starts a
  `PointerOpType::Select` seat op — the one op whose `window_ptr` is null —
  and the RELEASE decides: past `selection::DRAG_THRESHOLD` (5 px) it was a
  drag; short of it, a click, which drops the selection if there is one and
  leaves overview if there is none. `Cursor::left_click_on_bg_in_overview`
  went with the press-time exit.
- **Pressing the body of a selected window moves the whole selection.** The
  grab fills `Seat::group_move` with the other selected windows and their
  virtual positions; `op_update`'s Move arm carries them by the offset the
  grabbed window actually took, snap included, so a group of Tiled windows
  steps in whole cells and re-tiles on release (`Seat::settle_tiling`, the
  geometric detection `op_end` always ran, now per window). Carried windows
  count as `is_window_being_moved` and are skipped by the overview
  displacement. A tap moves nothing and un-tiles nothing.
- **The band is anchored in virtual coordinates**, so the edge auto-pan
  scrolls the desk under a held drag and the band grows with it. The Select
  arm queues the op frame like any other op for that reason: the edge-pan
  tick moves the camera and leaves the relayout to `op_update`.
- **Drawn outside `interactive_tree`**, in a tree on the scene root placed
  just above it: `Scene::at` stops at the first node it meets, and a node
  with no `SceneNodeData` reads as background, so a highlight inside the
  interactive tree turns a press on a selected window into a press on the
  desktop. Each box is a fill rect plus a `wlr_scene_bevel` in its
  glint-only focus branch, in `bevel_focus_color`;
  `WindowManager::draw_selection` places them per frame from
  `Output::render_and_commit`.
- The selection lives only in overview (`set_mode` clears it) and a
  destroyed window is dropped from it and from `group_move`
  (`Window::destroy`).
- **The desktop images select too** (since 2026-09-30). They are
  `cce-grid`'s pinned items, which the compositor otherwise knows only as
  the grid surface's input region, so the grid reports them over the
  control socket — `grid-items <id>:<x>:<y>:<w>:<h> ...`, virtual units,
  the whole list on every change, ids per grid process — and
  `Selection::desktop_items` keeps the list (dropped with the grid window,
  `selection_forget`). The band picks them up by the same touch rule,
  `draw_selection` washes them like windows (square-cornered: they are
  quads), and a group move carries them: `Seat::group_items` is filled
  beside `group_move` at the grab, `carry_group_items` moves the
  compositor's rects by the group's offset (the grabbed window's, snap
  included) and pushes `move <id>:<x>:<y> ...` on the status socket's
  `selection` topic, and `op_end` pushes `drop`, on which the grid saves
  its sidecar and reports afresh. A press on a SELECTED image is the
  compositor's, not the grid's: `PointerOpType::GroupMove`, the one op
  besides Select with no window — the pointer's own travel moves windows
  and images alike, snapped to whole cells when a carried window was Tiled
  (measured on that window, kept in `start_win_virtual_*`). A press on an
  unselected image drops the selection and goes to the grid, which drags
  it as before. A background click clears images too (`has_selection`), so
  a click with only images selected drops them rather than leaving
  overview. See `../cce-grid/CLAUDE.md` for the grid's half.
- `ccectl selection` prints `selected=<ids|-> items=<ids|->
  band=<virtual rect|-> desk=<id@x,y,wxh;...|->` — `items` are the selected
  images, `desk` every image the grid has reported, which is how a shadow
  sees the report land and where a group move left them; `ccectl camera`
  prints `mode=` too. In a shadow: `pointer-move-to`, `pointer-press left`,
  `pointer-move-to`, `selection`, `pointer-release left`. Keep the band off
  the screen edges or the edge pan scrolls the desk mid-assertion.
  `./verify/image-selection-test` drives the whole of the above — the
  report, the band, both grab sides, the click rules, the exit — against a
  cce-grid it runs itself (`CCE_GRID`, else the workspace's release build).

### Launching from overview

An app launched while in overview **does not leave it** (since 2026-10-05).
When a desk window (anything but Popup/Overlay, a status segment, the
wallpaper or the grid) maps during overview, `Window::map` calls
`WindowManager::pan_overview_to_window`, which keeps the zoom and pans
just far enough to show the whole window (`pan_to_virtual_rect`, the same
`pan_into_view` rule focus follows). A window that already fits moves
nothing. The new window still takes focus. The pan runs whatever
`center_on_spawn` says, since the focus loop's pan skips a first focus
without it, and not while a camera ramp (an overview enter still flying)
owns the camera. Before this, `exit_overview_to_window` flew the camera
to zoom 1 on the new window and switched to Normal. In a shadow:
`ctl overview`, `spawn foot`, then `ctl camera` still reads
`mode=Overview`.

### Config

Loaded on startup from **`$XDG_CONFIG_HOME/cce/config.kdl`** (falls back to
`~/.config/cce/config.kdl`). An adjacent `input.kdl` is merged in for key bindings and
input settings. **The format is KDL** (via the `kdl` crate; `parse_kdl_config`).
`config.rs` maps parsed values onto `WindowManager` state (layout gaps,
border/blur/desktop styling, keybindings → `Action`s, startup programs, output/display
settings). Live reconfiguration comes in over IPC (`ccectl reload`, `bind`, `layout …`,
`config-done`, etc.).

Per-output settings live under `output { <name> … }` as properties or child nodes:
`scale`, `brightness_interval` / `brightness_up` / `brightness_down`, and
**`size_mm="344x215"`** — the panel's real size, written into the `wlr_output`'s
physical size (via the `river_wlr_output_set_phys_size` shim) *before* its
`wl_output` global exists, so every client's geometry event carries it in place of
the EDID figure. That is the number cce-ui's `units::Metric` divides the logical
size by to resolve a `(mm)` config length (see `../cce-ui/CLAUDE.md`, Units). Set it
when EDID lies (TVs, projectors) or is absent (headless, the shadow: `HEADLESS-1`
reports 0×0 and clients fall back to an assumed 96 ppi). `ccectl outputs [--json]`
prints, per output, mode / scale / logical size / mm / logical px per mm and where
the mm came from (`configured`, `measured`, `none`); the creation log line says the
same.

**Swipe binds peek before they fire.** A three-finger swipe bound to a
directional focus or pan (`focus_left (gesture)"swipe3_left"` in input.kdl)
fires once the accumulated travel passes `window_manager { swipe_threshold }`
(libinput units, default 70; `WindowManager::swipe_threshold`; it was 50
until 2026-09-24, when a replay of logged swipes showed every deliberate
first step travelling 75 or more, so 70 drops only hesitant ones). Short of
that the camera *leans* toward the bind the
swipe is heading for, 1:1 with the fingers and in their direction
(`cursor::swipe_lean`, since 2026-09-24; before that it leaned along the
dominant axis only): its size comes from the dominant axis, and the
minor axis leans in proportion to the swipe's slope, so a diagonal swipe
leans diagonally. A swipe within 15° of an axis
(`SWIPE_LEAN_STRAIGHT_SLOPE`) still leans straight, since a hand's
sideways drift leaning the camera was a wobble, and the turn off the axis
is smooth past that band. The size is proportional to the travel —
`window_manager { swipe_peek }` screen px at the threshold (default 60, 0
disables; `WindowManager::swipe_peek_px` — not under `input`, whose
config.kdl block input.kdl's replaces wholesale), clamped there — and eases
back to where it started if the fingers lift first (`handle_swipe_end`), so
a hesitant swipe shows where it would go without going. With animations
off (`cce_ui::motion`) nothing leans: the camera holds still until the
bind fires and the step's pan lands at once. **A fire does
not end the swipe** (since 2026-09-24): the accumulated travel restarts
from zero at the fire, and a further `window_manager {
swipe_repeat_threshold }` of travel (libinput units, default four times
`swipe_threshold`; `WindowManager::swipe_repeat_threshold`) without
lifting fires again — for a focus or pan bind (`action_navigates`) only:
any other swipe bind, the four-finger overview toggle included, fires
once per gesture and the rest of it is ignored (`Cursor::swipe_spent`).
Three windows over is one long swipe, with more
resistance after the first step so it does not run on through the next
window — and a reversal after a step goes straight back. The lean
toward a further step is smaller too: it reaches `window_manager {
swipe_repeat_peek }` (screen px, default half of `swipe_peek`;
`WindowManager::swipe_repeat_peek_px`) at the repeat threshold, so it
moves far more slowly per unit of travel than the first step's lean. The factor was
two at first and read as too eager: replaying a session's logged swipes
(every `handle_swipe_update` is logged at info with its delta, in
`$XDG_RUNTIME_DIR/cce/cce.log`) showed ordinary single swipes travelling
150-250 units, so many stepped twice and then reversed to correct
(left-left-right-right); four times removes nearly all of those while a
long deliberate swipe still steps again. libinput's swipe deltas are
accelerated, so a fast flick covers far more travel than a slow push of
the same length. The lean scales
with whichever threshold is in force. The lean after a step rides on the focus ease
the step started (its pan target moves with the fingers) instead of
freezing it, and a lift short of the next threshold eases only that lean
back out; the steps stay. Clients are sent one cancelled `swipe_end` at
the first fire and hear nothing more of the gesture. **A focus swipe aims
where the fingers went** (since 2026-09-24): when the bind that fires is
a `focus_*`, the step's travel becomes a direction
(`cursor::swipe_focus_vector`, each axis's sense read from the bind table,
so mirrored binds mirror it and an axis without focus binds does not aim),
and `WindowManager::focus_toward` hands it to the policy crate's
`vector_focus`: the nearest window center within `window_manager {
swipe_focus_cone }` degrees (default 45; `swipe_focus_cone_deg`) of a ray
from the focused window's center takes focus, and a swipe toward nothing
in the cone changes nothing (logged `focus_toward …: no window within`).
With no focused window there is no ray, and the four-way action runs for
its entry rule. The keyboard's focus chords stay four-way. Only binds whose
action `cursor::action_navigates` (focus/pan left/right/up/down) peek, and
only toward a direction that has one; a four-finger overview toggle leaves
the desktop still. When the bind fires (`handle_swipe_update`) the action
runs against the camera where the lean left it: a window that needs a pan
gets its ease from there, and a window already in view sets no target, so
the camera simply stops where the lean left it rather than springing back.
**A pan back against the lean is kept.** It arises only when the lean
pushed the new window's near edge off screen, or leaned away from the side
it sits on, and it is only as large as bringing the window in needs. Until
2026-09-24 such a target was dropped so the camera never reversed, which
left the newly focused window clipped whenever the lean overshot; the
diagonal lean and aiming by finger direction made that common. It does not predict the
destination (tried on 2026-09-22 — a lean along the policy's predicted pan,
nothing at all when the target was in view — and retired the same day: the
lean is meant to answer the finger, not the layout), and it does not spring
back to the origin (the first version did, and a switch between two
windows both in view leaned out and back on every swipe). A shadow drives it staged — `ccectl pointer-swipe begin 3`,
`update <dx> <dy>`, `end` — and reads the lean and its return back with
`ccectl camera` (pan, zoom, pan target).

**Idle timeouts** — `idle { display_off <s>; sleep <s>; sleep_command "…" }`,
both 0 (off) by default — are `src/server/idle.rs`, a `Server` subcomponent
rather than window-manager state: two `wl_event_loop` timers re-armed from
`Seat::handle_activity`, held disarmed while `IdleInhibitManager::check_active`
reports an inhibitor. "Display off" reuses the wlr-output-power-management
path (`OutputStateValue::DisabledSoft` + `dirty_windowing`): the output stays
in the layout, nothing re-arranges, and no frame events fire while it is dark.
Only outputs the timeout darkened (`Output::idle_off`) are woken by the next
input, so one a client turned off with `wlopm` stays as the client left it.
The sleep command is `sh -c` under a fork, reaped by the server's SIGCHLD
handler; `systemctl suspend` returns as soon as the job is queued, so resume
is detected from the wlroots session's `active` signal instead (the
`river_wlr_session_get_active_signal` shim — `wlr_session` is opaque to
bindgen), treated as activity so a lid-open lights the screen without a key.
The Power plan's per-mode overrides (`/run/cce/idle_display_off`,
`idle_sleep`; `CCE_IDLE_PLAN_DIR` moves them) are followed by an inotify watch
on that directory, an fd source on the event loop, so nothing ticks at rest;
the 1 s stat poll that was the only mechanism until 2026-10-05 is now the
fallback while the directory is missing, and it switches back to the watch
once the directory appears.
Note that until 2026-09-16 the hardware pointer handlers (`handle_motion`,
`handle_motion_absolute`, `handle_button`, `handle_axis`) and `handle_group_key`
never called `handle_activity` at all — only tablet, touch and gestures did —
so `ext-idle-notify` clients were never told about mouse or keyboard use;
injected `ccectl pointer-*`/`keypress` events count as activity too, which is
what lets a shadow session exercise the timeouts (`ccectl idle timeouts 2 0`,
then `ccectl outputs` reads `enabled=false`, then any injected input reads
`true`). `ccectl idle` prints the state; `idle wake|sleep|display on|off` act
now. Untested in a shadow, which has no session: the resume wake.

**Every sleep locks first** (since 2026-10-01; before, nothing ever started
the lock screen and a laptop woke to its desktop). `LockManager::lock_now` locks
from the compositor's side with no client yet — the normal tree off, each output
rendering the blank locked tree — and then starts `cce-lock` through the
respawn timer, which binds via `handle_new_lock`'s already-locked branch; a
locker that never comes leaves the session locked, the crash path's guarantee.
The idle sleep and `ccectl idle sleep` go through `IdleManager::lock_then_sleep`
(sleep on `on_locked`, or after `LOCK_BEFORE_SLEEP_MS` regardless; activity in
between calls the sleep off and keeps the lock). logind's own sleeps — lid,
power key, `systemctl suspend` — are `sleep_lock.rs`: a thread holding a
`delay` sleep inhibitor that answers `PrepareForSleep(true)` with `ccectl
lock` (whose reply waits for `send_locked`) and then lets logind go. Real seats
only; a shadow has no session and starts no thread, so test `ccectl lock` and
`idle sleep` there (give the shadow a harmless `idle { sleep_command }` first —
its seeded config's default is `systemctl suspend`, the REAL machine) and the
logind half with `cargo test --lib sleep_lock -- --ignored`. While locked,
`handle_group_key` runs no binding but VT switches and config keybinds on
volume/brightness/media keys (`allowed_while_locked`), and no input-method
grab; every other key goes to the lock surface.

Persistent window state is saved to **`~/.local/state/cce/state.json`**
(`XDG_STATE_HOME/cce/state.json`) on shutdown and restored on start
(`save_state` / `load_state` / `spawn_restored_windows`). A window's
`cmdline` comes from `/proc/<pid>/cmdline`, which is what the process
*exec'd into*, not what launched it: an `exec` wrapper in `~/.local/bin`
(Inkscape's `GDK_SCALE=1` wrapper) reads as `/usr/bin/inkscape`, and a
restore that replays that path skips the wrapper. So `save_state` records
the **bare name** whenever the name's first `PATH` hit is a different file
from the one running (`path_shadowed_name`), and the restore's `sh -c`
resolves it the way the launcher did. The absolute path is kept when PATH
agrees with it.
**The restore relaunches the saved `argv`, quoted, never `cmdline`**
(`restore_command`, since 2026-10-02). `cmdline` is argv joined with spaces
and stays what the matchers compare (`saved_by_program`, `same_app`), but run
through `sh -c` it executed an argument's own shell characters: a viewer left
open on `x$(cmd).pdf` ran `cmd` at the next login, and a URL with `&` split in
two. Each argument is now `shell_quote`d. An entry saved before `argv` existed
relaunches only when its cmdline is plain words (`plain_cmdline`), else it is
skipped with a warning and its geometry still applies when started by hand.
foot's `--working-directory=` is a plain argv entry for the same reason.
Wine/Proton windows record their Windows-side exe path (`C:\...`) as the
command, which `/bin/sh` cannot run, so the restore never relaunches them
(`relaunchable`) — and draws no login placeholder for them either: until
2026-09-26 Ubisoft Connect's plate stood a minute over the empty desk,
waiting for a window nothing had started. The entry stays queued, so the app
still lands on its saved spot when the user launches it.
Beside it, **`min-sizes.json`** (`min_sizes.rs`) keeps the minimum sizes
X11 apps revealed by refusing a smaller configure mid-drag — Wine sends no
minimum for a resizable window, so Ubisoft Connect fought every shrink past
1214x804. Only a floor counts (two different sizes answered with the same
one, `xwayland_window::learned_min`), since a stored value is permanent;
it is keyed by app_id, program and title, kept in X11 pixels, applied at
map, and lowered when a window maps smaller than it.
A restored **floating** window is recalled into the current view
(`policy::camera::recalled_origin`, applied at the end of `try_restore`)
when its remembered position would show less than a quarter of it: the
camera at restore is wherever the session left it, and a floating window a
screen away from that is lost, not remembered. Tiled windows stay where the
grid has them. **Except on the tiled desk**: a floating window within one
viewport of the tiled windows' bounding box (`tiled_desk_bounds` — the
session's Tiled entries still queued plus the live Tiled windows) keeps its
remembered spot however far the camera is, since the columns beside it are
what the user pans along (cce-data-editor parked left of the first column
came back mid-view every login before 2026-09-14). The recall is for a
window with no tiled neighbour within a screen.

**A settings window opens over its app** with a `mode_rule` that names it
by title and says `over_sibling`:

```kdl
mode_rule mode="floating" app_id="md.obsidian.Obsidian" title="Settings" over_sibling=(bool)true
```

Obsidian (and Electron apps generally) open Settings as a PARENTLESS
toplevel, so nothing marks it a dialog; until 2026-09-29 it borrowed the
main window's saved entry by app_id — Tiled, latched, at the main window's
size — and the overlap rule pushed it to a free cell. Three things in
`try_restore` make the rule work. An untitled window of an app some title
rule names **waits for its title** before restoring at all (Electron sets
the app_id first, and a rule on the title cannot be judged without one).
A matching title rule then **outranks a borrowed entry** — never the
window's own (`rule_skips_restore`), so a plain title rule still honours a
geometry the user gave that window. And with `over_sibling`, while a mapped
window of the same app_id is up (`find_sibling`: the focused one when it
qualifies), the window is a **satellite** (`Window::satellite`): no saved
state applies, it sizes itself, `try_center_on_sibling` centres it over the
sibling (`centered_over`, slid into the view on any axis it fits) with the
same commit-time redo the view-centred modals use, and `save_state` neither
saves it nor lets it keep the app's `last_window_states` slot. Reproduce
with `verify/clients` `float-pair --dialog-honours-configure`: tile the
main window, and the "Authorize" window maps Tiled at 1404x1076 without a
rule, Floating at its own 400x370 over the main window with one.

**A prompt opens centred on the view** with a `mode_rule` that says
`center` (since 2026-09-30):

```kdl
mode_rule mode="tiled" app_id="com.onepassword.OnePassword" title=" — 1Password"
mode_rule mode="floating" app_id="com.onepassword.OnePassword" title="1Password" center=(bool)true
```

It puts the window on the same path as the built-in session modals
(`Window::try_center_on_view`, gated by `wants_view_center`: the hardcoded
`cce-authenticator`/`cce-filesystem-chooser` list OR a matching rule):
forced Floating and `mode_locked`, never minimized, centred on the camera
as it stands at map with the remembered SIZE kept and the position
discarded, re-centred on the commit that brings a self-sizer's real size,
and `hint_placed` so no spawn pan follows it. 1Password's authorization
popup is a parentless Electron toplevel under the vault window's app_id
with the bare title `1Password`; it saved as Tiled and reopened at that
one spot on the desk wherever the camera was. Two rules because title
matching is substring and the first match wins: the vault window's titles
all end in ` — 1Password`, so the first rule takes them and only the bare
title reaches the second. Reproduce with `float-pair --app-id X
--dialog-honours-configure` under `title="Main window"` / `title="Authorize"
center` rules: pan the camera, respawn, and `ctl windows` shows the
dialog at the screen's centre while the main window sits where it was.

**A window with no title matches `title=""`** (since 2026-10-05,
`mode_rule_matches`): an unset title counts as the empty string, which
every `title=` substring rule is tested against, so only `title=""` can
reach it. The Claude app's quick-entry popup is an untitled parentless
toplevel under the main window's app_id; with no rule able to name it, it
borrowed the main window's Tiled entry (the border drawn at the main
window's size around a far smaller popup, and focus panning the camera to
it). The live config names the main window `title="Claude"` first and then
`title="" center`. `float-pair --untitled-dialog` reproduces it.

**A client reconnecting maps unfocused**: a window that vanishes without
the compositor asking it to close (`Window::unmap` → `note_vanished`) lets
the next window of the same app_id AND the same program (`proc_args`
argv[0]) within 5 s map without taking focus (`take_recent_vanish`) — a
cce-ui client rebuilding its surface on a fresh connection must not steal
focus back. The program half is from 2026-09-26: keyed on app_id alone,
every Proton program is `steam_proton`, so a game Ubisoft Connect launched
a second after closing one of its own windows was held unfocused, and the
user's fullscreen key went to the window that kept focus. An unreadable
program (the process already gone) falls back to the app_id.

Restored windows map unfocused, and their FIRST focus pans the camera only
once the session has seen deliberate input (`WindowManager::startup_input_seen`,
gated in `Seat::focus`), so apps settling in at login do not drag the view
around. Deliberate means a button press, a key press, or the start of a
touchpad swipe or pinch (`handle_swipe_begin`/`handle_pinch_begin`); pointer
motion and a hold do not count. Swipes were added on 2026-09-26: a login
navigated only by three-finger swipes left each restored window's first
focus wherever it sat, often half off screen, while a second focus panned.

### Fullscreen steps aside for focus

A fullscreen window lives in `layers.fullscreen`, above every desk window, so
until 2026-10-01 focusing another window (the Super+Tab switcher,
`focus-window`, a focus chord) moved the keyboard to a window nobody could
see. Now `Window::fullscreen_yields` drops it to `layers.bottom` — behind
every window, above the grid — whenever a desk window (Floating, Tiled,
Utility, another Fullscreen; not its own dialogs, a popup or a status
segment) is ahead of it in `focus_history`. It stays fullscreen: the client
is not resized or told anything. Focusing it again puts it back on top. The
predicate reads the MRU history, not live seat focus, so a launcher or the
switcher opening (overlay UI never enters the history) does not pop it back
over the window you switched to. `Seat::focus` dirties windowing whenever a
fullscreen window exists, because the stacking pass that applies this only
runs on a transaction.

**Stepped aside, it stays on the desk** (since 2026-10-03; before, it stayed
pinned to the output, so a focus chord's pan left it fixed behind the new
window like a backdrop). While fullscreen, `virtual_x/y` is the desk spot
the window covers — set on the enter transition in `manage_finish` (after
`saved_virtual_x/y` takes the restore position) and kept in step with the
camera while the window is on top — and `place_fullscreen_windows`
(`arrange_views`) draws a yielded one there at output size, scaled with the
zoom, so the camera pans away from it. Focusing it again pans the camera to
exactly that spot (`Window::fullscreen_anchor_pan`, from `focus_follow_pan`)
and it rides the desk until the ease lands, then pins — no jump at either
end. `save_state` records `saved_virtual_x/y` for a fullscreen window, as it
did before the spot existed. **Overview treats it the same way**: there a
fullscreen window is never on top (`fullscreen_on_top`), so it is a slab on
the desk stacked behind every window even while focused (hover focuses it
in overview, and pinning it then would cover the desk), and through any
camera flight (`camera_ramp_anim`, an eased zoom) it flies with the desk.
An overview exit onto it lands exactly on its spot — `exit_onto_window`
centres its output-sized rect — and only then pins; `set_mode` dirties
windowing while a fullscreen window exists so the restack runs.

### xdg-activation

`handle_request_activate` (`server.rs`) runs for every activation wlroots
accepts — the token was checked against a recent input serial or the
requesting surface's focus. For a MAPPED window it now does what `ccectl
focus-window` does: un-minimize, `seat.focus`, `raise_window`, dirty. Until
2026-09-21 it only fired the "needs attention" D-Bus notification, so an
activation for an already-mapped window changed nothing on screen. A request
that lands before the map (Chromium/Electron activate a new window between
its app_id and its first buffer, so the log reads `Restoring saved state` →
`xdg activation request` → `Seat::focus`) is left to the map path, which
focuses under its own settle rules. Every `Seat::focus` on a Floating window
raises it, and `render_finish` keeps the floating plane above the tiled one
in render-list order, so focus IS visibility for a float — `ccectl windows`
prints `stack=N` (render-list position, higher is nearer) so that order can
be asserted from a shadow without a screenshot.

Two hazards in `try_restore` bite a second toplevel of a running app, which
the app_id-only third pass of `match_last_window_state` hands the main
window's remembered entry (a transient is excluded, a parentless dialog is
not): `minimized` is taken from a session entry only, never from a borrowed
one — a dialog born minimized is focused, listed and invisible — and a
Floating window whose borrowed origin coincides with a mapped sibling's is
cascaded off it (`cascade_off_siblings`, 40 px diagonal steps). Reproduce
either with `verify/clients` `float-pair` (one client, two parentless
toplevels, the second activated before its first buffer) or a two-window
Electron app; Chromium in a shadow needs `vkey hold` running first, since a
headless seat has no keyboard and Chromium crashes in
`xkb_state_update_mask` on a modifiers event that no keymap preceded.

### IPC & status sockets

- **Control socket** `/tmp/cce-{WAYLAND_DISPLAY}.sock` (`ipc_server.rs`): line-oriented
  request/reply over a Unix socket. `ccectl` / `cce_ctl.rs` is the client.
  `read_command` frames a request (since 2026-10-02): the first line, ended
  by its newline, the client closing, or a 50 ms pause after some bytes
  (clients that send neither still work), at most 64 KiB — an overlong one
  is refused, never cut short and run as the old single 4 KiB `read` did —
  and a connection silent for 5 s is dropped. A NUL byte is refused (it
  reached xkbcommon's `CString::new(..).unwrap()` from `shortcut bind`), and
  `handle_ipc_event` runs each command under `catch_unwind`, since a panic
  unwinding out of that `extern "C"` callback aborts the whole session.
  Numbers parse through `parse_finite` (no NaN/inf into pointer or camera
  math) and injected swipe/pinch steps clamp to `MAX_INJECTED_STEPS`. The
  status and stream sockets read their subscription line through
  `read_line_bounded` (total deadline and size cap): a per-read timeout let a
  byte-a-second client hold the thread that serves every other subscriber.
- **Status socket** `/tmp/cce-status-{WAYLAND_DISPLAY}.sock` (`status_server.rs`): runs
  on its own thread; a client sends one subscription line (`layout`, `title`,
  `modifiers`, `dismiss`, …) and receives text lines on every
  change. This feeds the status bar (`cce-status-interface`). The main loop pushes
  updates through a `StatusSender` mpsc handle.

  **`selection`** is a one-shot topic too, for the desktop grid alone: the
  overview drag-selection carrying its images pushes `move <id>:<x>:<y>
  ...` per step and `drop` at the release (see "Overview drag-selection").

  **`clickaway`** is a one-shot topic (like `dismiss` and `shortcuts`): a
  `press` line for each button press that lands on NO X11 surface while some
  override-redirect X window is showing (`handle_button`). Xwayland only sees
  the pointer over its own surfaces, so an X11 popup — a Wine tray app's
  menu above all — never hears a press on a Wayland window and stays open;
  until 2026-09-26 only a click on one of the app's own X windows closed it.
  The tray bridge (`cce-status-interface`'s `cce-xembed-tray`) subscribes and
  closes the popup its forwarded click opened by addressing it a press just
  outside itself. The bridge's hidden icon containers have no scene tree, so
  they never count as showing.

  **What may start a transaction.** `dirty_windowing()` schedules a full
  manage/arrange/render pass, and on an idle desktop the answer to "why is the
  window manager busy" is always some call site that dirties on a routine
  commit. Two were found on 2026-09-10 and gated: a status segment's *every*
  commit (`handle_window_commit`, now only when the surface size changed — the
  clock ticking once a second used to cost an arrange each time) and a title
  change (`notify_title`, now only when a mode rule matches on `title=`; the
  built-in policy is the only manager, `wm.object` is never bound, so nothing
  else in the manage sequence reads a title — the status bar's `title` topic
  and the state file are fed directly instead). `CCE_DIRTY_TRACE=1` logs one
  debug line per dirty call with its `#[track_caller]` site; it is the tool
  for this question, and costs nothing when unset. `CCE_DIRTY_BACKTRACE=1`
  adds a full backtrace per call (expensive). The state file is written by a
  one-shot timer (`schedule_save_state`, at most once a second) rather than
  on every transaction: `save_state` reads `/proc` for every window, and a
  drag is one transaction per pointer event. Each window's argv is cached by
  pid (`proc_args_cache`, pruned to live windows each save) — `proc_args`
  stats every `PATH` entry; only foot's shell cwd is read fresh. The
  unchanged check compares compact JSON, and a write goes to
  `state.json.tmp` and is renamed over, so a crash cannot truncate it.

  **A forced grid rebuild runs with the grid tree disabled** (`draw_grid`,
  since 2026-10-06): every frame of a zoom flight resizes and moves every
  pooled cell rect and rim, and a scene setter on a live node re-walks the
  scene for what it touched, while under a disabled ancestor it returns at
  once — so the tree goes off around the rebuild and on after (6 overview
  flights in a shadow: 440-540 ms of compositor CPU -> 180-200 ms). **A
  refused commit schedules the next frame** (`render_and_commit`): the
  damage stays pending, but nothing else asked for a frame, so the EBUSY
  bursts the panel's commits hit left the screen stale until something
  else moved; the error is logged once per 10 s with a count.

  **Per-frame work is gated too.** The `/tmp/cce-ovdbg` scene dump needs `CCE_OVDBG=1` in the environment
  before the file is even looked for. The window-stream tick runs only while
  the stream hub has subscribers (the accept thread's eventfd arms it), and a
  failed tearing test is not repeated every frame of the same fullscreen
  episode. `Window::role()` and its `is_status_bar`/`is_grid`/`is_wallpaper`
  wrappers borrow the app id rather than allocating; keep it that way, they
  run several times per pointer-motion event.

  **Blur re-renders only where damage reaches** (scenefx `apply_blur_region`,
  fixed 2026-09-11). `pixman_region32_intersect` returns allocation success,
  not "non-empty"; the vendored code tested that return, so every blur node
  counted as touched by every frame's damage and re-blurred — nine status
  segments cost ~1.3 ms of CPU per frame whenever anything on screen moved
  (measured: 1670 µs → 495 µs per frame with an animating client far from
  the bar). A node whose box lies within the blur sample size (2^(passes+1) ×
  radius = 80 px at the default 3/5) of the damage still re-blurs, as it must.
  `CCE_BLUR_DEBUG=1` logs each blur node render (`blur entry …`) and each
  compensation decision (`blur_region …`) — the tool for "why is this blur
  re-rendering". Known, not fixed: the *optimized* (cached) blur behind a
  translucent window is not re-baked when content beneath it changes, only on
  explicit camera/grid dirtying, so a video under a blurred window shows a
  frozen ghost; buffer commits never pass through `scene_node_update` with
  damage in this scenefx, which is the path the cache's dirtying hangs off.

  **A bake near the output edge is a guess** (fixed 2026-10-01). Through a
  frozen pan (a swipe) each optimized node keeps its bake and only bakes the
  strips it newly shows (`optimized_blur_render`, anchored at
  `baked_x/baked_y` with `baked_region`). But a pixel baked within the blur's
  reach (sample size / output scale, ~40 layout px at scale 2) of an output
  edge sampled that edge's clamped pixels, not the backdrop beyond it — so a
  window that hung off the screen and was swiped on kept a seam of smeared
  grid along where the edge had been, up to the reach wide. Such pixels go
  into `edge_region` as well as `baked_region`, and are re-baked once a pan
  carries them clear of every edge (`optimized re-bake edge guess` under
  `CCE_BLUR_DEBUG=1`). A window sitting at the edge re-bakes nothing. To
  reproduce, the content just past the edge must differ from the content at
  it: a black cell on both sides blurs the same either way, which is why the
  first shadow attempts showed nothing. Put a grid gap just off screen.

  **Status text contrast is backdrop compression** (`module { backdrop_compress }`
  in the bar's config, the minimum WCAG ratio its text must hold). A Wayland
  client cannot see what its translucent module boxes are composited over, so
  the compositor fixes the backdrop instead: `backdrop_compress_params`
  (`config.rs`) turns the ratio and the bar's `module { text_color }` into a
  luminance ceiling, `Window::sync_backdrop_compress` sets it on the segment's
  blur node and droplet lens (`wlr_scene_blur_set_compress` /
  `wlr_scene_droplet_set_compress`), and scenefx's `tex.frag` / `droplet.frag`
  (`compress_backdrop`) pull every backdrop pixel brighter than half the
  ceiling smoothly under it — or, for dark text, lift the shadows. It replaced
  (2026-10-01) the per-segment `backdrop` status topic, whose CPU geometry
  measurement and window-content readbacks ran every frame to feed a bar-side
  scrim.

### Touchscreens

`src/server/touch.rs`. Until 2026-10-04 the seat never offered the touch
capability, so no client ever bound `wl_touch` and a touchscreen did
nothing. The seat now offers it while a touch device is attached
(`Seat::touch_devices`, counted in `attach_device` / `detach_device`), and
**each finger is routed once, at touch-down** (`touch::TouchRoute`, decided
by `Cursor::touch_route_at`):

- **`Client`** — the surface under it belongs to a client that bound
  `wl_touch` (`wlr_surface_accepts_touch`: GTK, Qt, Chromium, Xwayland,
  foot, and every cce-ui app since cce-ui's `backend/touch.rs`) and the
  compositor is not in window-adjust mode. It gets real touch events,
  focus as a click would give, and the press-time menu dismissals
  (`press_dismissals`, shared with `handle_button`). Motion is mapped through
  the surface frame frozen at down (origin plus the scene buffer's scale,
  the pointer implicit grab's `grab_origin`/`grab_scale`), so a finger that
  slides off the window keeps reporting surface-local positions to it.
- **`Pointer`** — everything else: a left button held where the finger is,
  run through the real `handle_button` and motion path, which is how the
  desktop and all the compositor's own presses — overview (where every
  finger takes this route, even over a touch client), the adjust handles,
  window body drags and the rubber band — work by finger with no touch path
  of their own. One finger at a time, since the pointer is singular. **The
  press waits** (since 2026-10-05): the pointer hovers at the down point,
  and the press lands there only once the finger moves past `TAP_SLOP`
  (10 px; then the drag follows) or lifts (a tap). That is what lets a
  second or third finger turn the touch into a gesture with no half-made
  click to take back.
- **`Ignored`** — a second finger while one drives the pointer, or any
  finger while a real button or seat op holds it.
- **`Claimed`** — owned by a gesture, below.

**Gestures** (since 2026-10-05, `touch::Claim`). While one is live every
finger is the compositor's; fingers already given to clients get
`wl_touch.cancel` when it begins.

- **Desk pan and zoom.** One finger dragged on the bare desk pans it, in
  normal mode (in overview it stays the selection band). A second finger
  joining a finger that went down on the desk — in either mode — makes it
  a two-finger pan that pinch-zooms about the midpoint
  (`queue_pan`/`queue_pinch`, as the trackpad's). The lift coasts on the
  pan's velocity, like a trackpad pan, unless the fingers had rested. Two
  fingers that start on a window are the app's (a browser's pinch-zoom).
- **Three or four fingers**, anywhere, claim as the third lands (unless a
  pointer finger is mid-drag). `decide` waits for `DECIDE_TRAVEL` of
  centroid travel (a swipe) or a `DECIDE_SCALE` spread change (a pinch),
  counting the most fingers seen, so four fingers landing one at a time are
  a four-finger gesture. A swipe runs through the touchpad's own
  `handle_swipe_*` (`gesture_from_touch` keeps it off clients'
  pointer-gesture streams and past the trackpad's `gestures { swipe }`
  switch), so the `swipe3_*`/`swipe4_*` binds, the lean, repeat steps and
  focus aim all apply, with travel in layout px against the same
  thresholds. **A touchscreen swipe is natural**: the desk follows the
  fingers, so the bind that fires is the way the CAMERA goes — fingers
  dragging left fire `swipe3_right`, four fingers dragging up fire
  `swipe4_down`. A pinch fires the `pinch3_*`/`pinch4_*` binds
  (`pinch_hits`, the trackpad's thresholds) once.
- **Edge swipes** bind like any gesture, in input.kdl:
  `overview (gesture)"edge_bottom"` (`config::parse_edge_gesture`;
  `edge_left|right|top|bottom`, the edge the finger starts from, optional
  modifiers). A first finger landing within `EDGE_ZONE` (24 px) of a
  screen edge — one no other output continues past — is held only when
  such a bind exists; `EDGE_FIRE` (60 px) inward fires it once. A finger
  that goes along the edge or back out is handed to the normal route from
  its down point (`edge_release`), one that lifts where it landed is
  delivered as the tap it was, late, and a second finger ends the edge
  claim the same way. So a bound `edge_top` delays every tap on the status
  bar's top 24 px until the lift.

A cancel on a `Pointer` finger releases the button if it was pressed (a
stuck button is worse than a stray drop), and on a `Client` finger sends
`wl_touch.cancel` (`river_wlr_seat_touch_cancel_point`, which voids that
client's whole sequence, the protocol's unit). Unplugging the last
touchscreen cancels any fingers still down. The cursor image goes away on
touch-down (`Cursor::hidden_by_touch`; `set_xcursor` and
`handle_request_set_cursor` both honour it) and real pointer motion brings
it back (`unhide_after_touch`, which clears pointer focus so the client
under it re-enters and sets its cursor again).

Drive it in a shadow with `ccectl touch down <id> <x> <y>`, `motion <id> <x>
<y>`, `up <id>`, `cancel <id>` and `tap <x> <y>` (layout pixels; several ids
down at once are several fingers). The first use sets
`Seat::touch_injected`, which offers the capability as a touchscreen would,
so `weston-simple-touch` under `WAYLAND_DEBUG=1` shows the `Client` route's
`wl_touch` traffic — and the `cancel` a third finger sends it. A shadow's
input.kdl is its own copy: add `edge_*`/`pinch3_*` binds there to try them.

**The on-screen keyboard follows a touched field** (`osk.rs`, since
2026-10-05). A finger on an app's window (`note_touch` at a `Client` down
or lift, or a `Pointer` tap's lift; never a layer surface, so taps on the
board itself do not count) arms it for `TOUCH_WINDOW` (800 ms), and the
first text-input-v3 enable or commit inside that window spends the touch and
runs `cce-keyboard show`. When the field goes (`disable_text_input`: a
disable, a destroy, or focus moving), `cce-keyboard hide` runs after
`HIDE_DELAY_MS` (250 ms) — cancelled by any enable, so moving field to field
keeps one board — and only if this module showed it, so a board summoned by
Super+O stays. Off with `window_manager { osk_on_touch (bool)false }`. It
needed the relay to **enter text inputs without an input method**:
`InputRelay::focus` used to send `enter` only when one was registered
(river's rule), so no client ever enabled a field. It now enters always, a
refocus of the same surface is no longer a leave (it was an `assert`), a
text input bound after its client took focus is entered at creation, and an
input method arriving or leaving no longer re-runs focus. Shadow check:
`ctl touch tap` on a cce-gallery TextBox brings `cce-keyboard show` up;
`pointer-click` on it does not.

### Portal global shortcuts

A native Wayland app cannot grab a key; it asks xdg-desktop-portal's
`GlobalShortcuts` interface for one (1Password's Quick Access does), and the
portal frontend hands that to a backend. `../cce-shortcuts-portal` is that
backend and **`src/server/global_shortcuts.rs` is this side of it** — a
table of `(session, id, mods, keysym)` on the window manager
(`portal_shortcuts`) with a control-socket command to fill it and a status
topic to report it:

- `shortcut bind <session> <id> <trigger>` parses a shortcuts-spec trigger
  (`CTRL+SHIFT+space`; modifiers `CTRL`/`ALT`/`SHIFT`/`LOGO`, key an xkb
  keysym name) and replies `ok <trigger_description>` (`Ctrl+Shift+Space`)
  or `error: …`. A chord in `keybinds` is refused — the user's config owns
  it — as is one another session already holds. `unbind <session> [<id>]`,
  `clear` and `list` are the rest. Nothing is persisted; the backend sends
  `clear` when it starts.
- The chord is matched in `handle_group_key` after the builtins and the
  config keybinds, through the same two-level keysym lookup
  (`keyboard_group::match_chord`, which `match_cce_keybind` now wraps), as
  `KeyConsumer::PortalShortcut`. Press AND release are pushed as one-shot
  lines on the status socket's `shortcuts` topic —
  `activated|deactivated <session> <id> <time_msec>` — since the portal has
  a `Deactivated` signal; neither edge reaches the client.

The compositor never learns which app asked: the session object path is
the only identity it carries, and it is one whitespace-free token, which is
why ids come percent-encoded (`Quick%20Access`) and stay that way here.
Drive it in a shadow with `ccectl shortcut bind /s/1 x CTRL+SHIFT+space`
and `verify/clients`' `vkey mod:5 57` — not `ccectl keypress`, which goes
straight to the focused client and never meets the chord matcher.

## Conventions

- This is systems FFI code: raw pointers, `unsafe`, and manual wlroots listener wiring
  are the norm. When adding a wlroots event handler, follow the existing pattern —
  embed a `wl_listener`, register it, and recover `self` with `container_of!`.
- Keep river's SPDX/copyright headers on files that carry them.
- `scratch/` and `scratch/*` (and the many `.png`/`.log`/`patch*.py` files in the
  parent dir) are ad-hoc debugging artifacts, not part of the build.
