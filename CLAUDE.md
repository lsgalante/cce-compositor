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

Six modules carry unit tests — `backdrop.rs` (the most of any, covering the
measurement and the desktop/window blend), `config.rs`, `window_manager.rs`,
`screenshot.rs`, `migrate_input.rs`, `text.rs`. They cluster where the logic is
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
  keys take, so keybindings and builtins fire for injected keys.
- **`status-stub`** — maps an xdg toplevel with a `cce-status*` app_id 400px
  tall, which `any_expanded_status_segment` reads as an open in-surface menu
  (expanded is geometric: thicker than `layout.bar_height`). It subscribes to
  the status socket's `dismiss` topic, prints one line per push, and shrinks
  to a bar strip on the first one — reacting the way the real bar does.

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
- **`window_manager.rs`** (~5.7k lines) — the heart of the mechanism side. Holds the
  WM state, the camera fields, window lists, the IPC command dispatcher
  `process_ipc_command()`, the `Policy::action` snapshot builder
  (`build_action_ctx`) and the `Compositor` command applier. IPC requests arrive on
  an mpsc channel drained by a wlroots event-loop timer (`handle_ipc_timer`) so all
  mutation happens on the main thread. Decision logic (camera math, action
  dispatch, snapping, refocus, grid geometry) lives in `cce-window-manager`.
- **`window.rs`** (~4.9k lines) — per-window model and rendering (borders, blur,
  viewport transforms).
- **`crate::tiling`** (from `cce-window-manager`) — `TilingMode` enum: `Floating`,
  `Tiled` (grid-aligned; the window reports xdg maximized), `Fullscreen`,
  `Popup`, `Overlay`, `Status`, `Utility`. Tiled-ness is geometric: the seat
  op's end (`seat.rs::op_end`) promotes/demotes via
  `policy::snap::is_cell_aligned`. `Utility` is the one mode a client asks for
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
  `drag_icon.rs`, `wm_node.rs`.
- Output: `output.rs`, `output_manager.rs`. Session: `lock_manager.rs`,
  `idle_inhibit_manager.rs`. Rendering: `scene.rs`, `scene_node_data.rs`.

### Window move/resize handles

Pointer move and resize exist **only in overview mode**, and their band is
**inside** the content rect — an inset ring hugging the window's own edges,
where the old band sat outside them.

- `cursor::get_border_zone` is the hit test: it returns `BorderZone::None`
  outright unless `wm.mode == Overview`, so in normal mode a window cannot be
  dragged or resized at all. Only the pointer is gated — `move_window_*`,
  `ccectl move-window`, and a client repositioning itself all still work in
  normal mode.
- Inside the ring, **all four edges resize**, the top included. Dragging the
  window's body is what moves it in overview, so the top edge no longer has
  to be spent on moving the way the outside band's did.
- The ring is drawn by **one scenefx node**, `wlr_scene_frame`
  (`scenefx/render/fx_renderer/shaders/frame.frag`), not by rects. Its
  inner edge is a WAVE of eight hills and eight valleys: a hill in the
  middle of each side, a hill on each corner, a valley between every two.
  The valleys sit `R` in from every corner — a quarter of the window's
  SHORTER side, on screen — and are `band_min` thick. Between a side's two
  valleys the band swells to `band` at the midpoint along a raised cosine
  (shaped by `swell_curve`). Between the two valleys flanking a corner the
  inner edge is a SUPERELLIPSE arc — a squircle corner of radius
  R − band_min, tangent to both valleys — whose exponent the shader solves
  so its deepest point, on the corner's 45° diagonal, is exactly `band` in
  from the silhouette: the corner hill peaks on the diagonal at the same
  height as the side hills, and meets the valleys with no crease. The
  earlier profile (edge bars and corner arms pinching to seams at
  `corner_length`, with gap notches and a round `bulge` pad on each corner)
  is gone; those three keys still parse and are passed to the node, and
  the shader ignores them.
  The shader's zone logic works in TOP-DOWN box-local coordinates
  (`gl_FragCoord` minus the box position, unflipped): the `corner_dist` SDF
  flips its own copy, and mirroring the zone coordinate the same way once
  swapped every zone label vertically — the top edge lit the bottom. And a
  hover swap must repaint even when no reveal value moves: in overview the
  ring is already fully revealed, so `step_border_fade` compares the hovered
  zone against the one last drawn (`border_hover_drawn`), or the shader
  keeps showing the previous zone until an unrelated commit repaints.
  The valleys are the zone seams: in the shader, a fragment belongs to the
  side it is nearest (so the split runs along the diagonals) and is a
  corner zone when it lies within R of that side's end; in
  `get_border_zone` the corner zones are the R×R squares at the content
  corners. Both derive R the same way and must stay in step. The profile
  is continuous along a side, which a rect cannot express: its only shaping
  tool is a clipped region whose corner radius is a single scalar, capped by
  the thickness change (tens of px) while a side is hundreds long, so it
  reads as a bump near the centre rather than a swell. `border.segments`'
  12 rects are what it replaced; they stay allocated but disabled.
- **The ring's thickness is a SCREEN width, not a world one**, floored at
  `HOVER_BAND_MIN`. Handles exist only in overview, which is zoomed *out*, so
  a band that scaled with the window would be at its thinnest exactly where it
  is the only way to resize: 16px renders as 7 at a typical overview zoom and
  the thin corners as 2.5, which is neither visible nor clickable. R, the
  valley position, is a fraction of the on-screen window, so the composition
  holds at any zoom — only the thickness is pinned. `draw_borders` and
  `cursor::get_border_zone` each derive it the same way and must stay in step.
- `swell_curve` shapes the SIDE hills: the raised cosine's height, 0 at the
  valleys and 1 at the midpoint, is raised to this power. Below 1 broadens
  the hill (a flatter top, tighter valleys); above 1 sharpens it. The shader
  floors it at 0.6 — below 0.5 the valleys turn into cusps. The corner
  hills' shape is the superellipse's own and does not take it.
- Three knobs shape it, all under `border` in config.kdl. `handle_width` is
  the hill height — the thickness at the middle of a side and on each
  corner's diagonal — in screen px, **its own key, not derived from
  `width`**, because the ring must be thick enough to see and hit while the
  desktop is zoomed out, while the window's visible border is a much finer
  line; deriving one from the other meant you could not thicken the grip
  without thickening every border. `taper` is the valley thickness as a
  fraction of the hill's — 1.0 is an even ring, clamped to (0, 1] because
  past 1 the valleys would be thicker than the hills, which is the moulding
  inside out. `swell_curve` is above. `corner_length`, `segment_gap` and
  `bulge` belonged to the retired seam-and-pad profile: still parsed, no
  longer drawn.
- The thickness is capped at a fifth of the window's shorter on-screen side,
  so a zoomed-out window is never mostly ring. That cap replaced a hard
  cutoff which disabled the handles below a size threshold: a window you
  cannot resize at all is worse than one with a slimmer grip.
- The shader's zone numbering MUST match `BorderElement::index()`; it is what
  the hovered-zone uniform selects on.
- `window::window_takes_handles` is the single predicate for which windows get
  handles (excluding Popup, Fullscreen, Status, Utility, circular, hidden),
  used by both the hit test and the drawing. Keep those in step: a handle that
  is drawn but not honoured — or honoured but not drawn — is the failure mode
  this arrangement exists to prevent. Note the *grab* zone stays the full
  even band (the four catcher rects) even where the ring is drawn thin: the
  swell is ornament, and a corner you can see but not grab would be worse.
- Handles are shown on the **focused window only**, for as long as overview
  is on (`step_border_fade`'s `all_on` branch, gated on
  `Window::is_seat_focused`). **Focus follows the pointer in overview**: the
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

None of this is policy — `cce-window-manager` was untouched. The mode is
already in `ActionCtx`, but what a *pointer* may grab is mechanism.

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
A restored **floating** window is recalled into the current view
(`policy::camera::recalled_origin`, applied at the end of `try_restore`)
when its remembered position would show less than a quarter of it: the
camera at restore is wherever the session left it, and a floating window a
screen away from that is lost, not remembered. Tiled windows stay where the
grid has them.

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
  readback where it can be — the desktop background is drawn from a declarative
  spec, so `backdrop.rs` computes cell-vs-gap coverage under each segment rect
  on the CPU (`Output::measure_status_backdrops`, per frame, gated by
  `update_status`'s equality check).

  A window covering part of a segment is the case that has to be *read*:
  `Output::read_window_region` composites that window's surfaces (subsurfaces
  included) over just the overlapping strip via
  `screenshot::read_texture_region`, and `backdrop::blend` folds the result
  into the desktop measurement for the rest of the segment. Two gates keep that
  readback off the render thread's back, and the second one matters more than
  the first: a 250ms throttle, and a check that the window's summed surface
  commit sequence changed at all (`river_wlr_surface_current_seq`). A window
  nobody is typing in is read exactly once. Content that still cannot be read —
  no committed buffer, an unsupported read format, an implausibly large strip —
  falls back to `backdrop::UNKNOWN`.

## Conventions

- This is systems FFI code: raw pointers, `unsafe`, and manual wlroots listener wiring
  are the norm. When adding a wlroots event handler, follow the existing pattern —
  embed a `wl_listener`, register it, and recover `self` with `container_of!`.
- Keep river's SPDX/copyright headers on files that carry them.
- `scratch/` and `scratch/*` (and the many `.png`/`.log`/`patch*.py` files in the
  parent dir) are ad-hoc debugging artifacts, not part of the build.
