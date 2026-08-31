# The cce workspace

Guidance for working anywhere in the `cce` Wayland desktop workspace — the layout,
the multi-repo rule, the build/install entry point, config, and IPC.

**This file lives here, not at the workspace root, because the root is not a git
repository** (see the multi-repo section below) — anything written there is
unversioned and lost on a fresh clone. The root `CLAUDE.md` is a pointer to this
file and repeats only the two rules that must not be acted against before reading
it. Per-crate notes stay in each crate's own `CLAUDE.md`; compositor-specific
detail is in the adjacent `CLAUDE.md`.

**Paths below are relative to the workspace root** — the parent of this crate — so
`cce-ui/src/` means `../cce-ui/src/` when read from here.

## What this is

This is the **`cce` Cargo workspace** (`resolver = "2"`). It is a complete Wayland
desktop environment written in Rust, split into two halves:

- **`cce-compositor/`** — the compositor + tiling window manager (`cce-fx`, symlinked
  as `cce`), built on wlroots 0.20 via FFI with vendored scenefx. This crate is its own
  world: it has a `build.rs` native-build pipeline, a `Makefile`, and its own detailed
  **`cce-compositor/CLAUDE.md`** — read that before working inside `cce-compositor/`.
- **~18 `cce-*` client apps** (`cce-status-interface`, `cce-system-interface`,
  `cce-designer`, `cce-files`, `cce-color-editor`, `cce-mail`, `cce-graph`, `cce-notifier`,
  `cce-authenticator`, `cce-display-manager`, `cce-text-editor`,
  `cce-data-editor`, `cce-fonts`, `cce-cloud`, `cce-layout-interface`,
  `cce-screenaver`, `cce-test-interface`, `cce-terminal`, …) — Wayland client GUIs that connect to the
  compositor and to each other over Unix sockets. (The desktop background is drawn
  natively by the compositor — the former `cce-wallpaper` client was retired.)

The one thing tying every crate together is **`cce-ui`**, the shared GUI toolkit. Every
client depends on it (`cce-ui = { path = "../cce-ui" }`); the compositor depends on it
too. There is one other shared crate: **`cce-window-manager`** — the compositor's
pure-Rust window-management policy layer (arrange pass, `TilingMode`, saved state,
slotmap; no FFI), extracted from `cce-compositor/` and consumed only by it. The compositor
re-exports it as `crate::policy` / `crate::tiling` / `crate::slotmap`.

### Version control: this is a MULTI-repo, not a monorepo

The workspace root itself (this directory — holding `Cargo.toml`, `Cargo.lock`,
`target/`) is **not** under version control. Instead, **each member crate is its own
independent git repository** with its own committed `Cargo.lock`. The crates sit
side-by-side under this directory to form the build workspace, but are versioned and
published separately.

**The local repos are the source of truth — there are no push remotes.** Publishing
goes through **gitsite** (`~/Dropbox/src/gitsite`): repos listed in its `repos.conf`
are mirrored, rendered, and deployed as a static read-only site at
**https://git.lucas.co** (browsable, and clonable over dumb HTTP for `clone`-mode
entries). A systemd user timer (`gitsite.timer`) republishes automatically when any
listed repo's HEAD changes, so committing locally IS publishing. Crate repos carry a
fetch-only `origin = https://git.lucas.co/<crate>.git` — `git push` does not work
against it by design (static host; no receive-pack, no SSH). New crates get a line in
`repos.conf`. (The pre-2026-08-11 per-crate codeberg.org remotes are retired; those
repos still exist server-side for old history.)

Consequences to respect:
- **Do not `git init` at the root** — it would swallow every crate as an embedded repo.
  Commit inside the relevant crate's own repo.
- **Each crate must build standalone.** Do not introduce `[workspace.dependencies]` /
  `<dep>.workspace = true`: a standalone clone of a single crate's repo has no
  `[workspace]` parent, so inherited deps fail to resolve. Dependency versions are
  intentionally declared per-crate (minor drift between independent crates is fine).
- A crate's `[profile.*]` is honored when it's built standalone (it is then its own
  workspace root) and ignored (with a warning) in the full-tree build — that warning is
  expected, not a bug to "fix" by deleting the profile.

## Build, test, run

The workspace `target/` dir is shared at the repo root (`./target/`). There is no root
Makefile, but **`ccebuild` is the DE-wide entry point** — do not hand-roll a loop over
the crates. It ships in `cce-compositor/scripts/ccebuild` and installs to
`~/.local/bin`:

```sh
ccebuild install            # build the workspace, install every binary + unit
ccebuild install cce-mail  # just one package (what each crate's `make install` runs)
ccebuild restart            # restart user services left on a replaced binary
ccebuild status             # built-vs-installed drift, AND running-vs-installed
ccebuild prune              # target/ artifacts of crates cargo no longer knows
ccebuild install-system     # the root-owned binaries (needs sudo)
```

The full deploy loop is `ccebuild install && ccebuild restart`. `ccebuild` derives
every binary from `cargo metadata`, which is the point: the per-crate Makefiles used
to name their binaries by hand, so crates with extra `[[bin]]` targets shipped
incomplete for weeks (`cce-ui` without `cce-relief`, `cce-display-manager` without its
three `cce-keyring-unlock*` helpers). Each crate's `make install` is now a thin
wrapper around `ccebuild install --no-build <pkg>`; `make build/run/clean` are
unchanged. **Never add a binary name to a Makefile** — cargo already knows it.

### Desktop entries

A crate that should appear in the launcher — or be selectable as an XDG default —
ships **`<crate>/<name>.desktop` at its own root**, next to `Cargo.toml` and beside
any `*.service` it ships. `ccebuild install` copies those into
`$XDG_DATA_HOME/applications` and runs `update-desktop-database`, filtered by
package the same way units are.

Two rules, both learned the hard way when these files lived only in
`~/.local/share/applications` and were hand-edited there:

- **`Exec=` is a bare binary name**, never an absolute path. `~/.local/bin` is the
  first entry on the session PATH, and the launcher (`cce-cloud`) spawns through
  `sh -c`, so the name resolves. Nine of the ten imported entries had baked in
  `/home/lsgalante/.local/bin/…`.
- **An app is only reachable as a default handler if it declares `MimeType=`.** The
  settings app's Default Apps page builds each dropdown by scanning installed
  entries for the ones claiming that category's MIME types, so an app with no
  `MimeType=` line simply never appears as a candidate — which is why `cce-files`
  could not be chosen as the file manager despite having an entry. Declaring a type
  also means honoring it: the app has to accept the path or URL argv the field code
  (`%f`/`%u`) passes it.

### App icons

An entry's `Icon=` should be the app's own name (`Icon=cce-files`), backed by
`cce-icons/hicolor/scalable/apps/cce-files.svg`. `ccebuild install` mirrors any
crate's `hicolor/` tree into `$XDG_DATA_HOME/icons/hicolor/` and refreshes the GTK
icon cache; `cce-icons/hicolor/README.md` documents the naming and the symlink
convention that keeps `svg/` the sole source of the artwork.

Before 2026-08-16 the entries borrowed generic freedesktop names
(`preferences-system`, `system-file-manager`), which resolved only if some other
installed theme happened to provide them, and `cce-preview` "worked" only because
five PNGs had been hand-copied into `~/.local/share/icons` — unversioned, and gone
on a fresh clone. The same failure as the `.desktop` files themselves.

Note that **nothing displayed an `Icon=` key at all** until the launcher was taught
to: `cce-cloud`'s `AppInfo` had no icon field. `cce_ui::icon` is the shared
resolver (theme name or absolute path → file); it is distinct from
`cce_ui::upload_icon`, which loads a *bundled* cce-icons glyph for in-widget use.

`ccebuild status` is the tool for "is what's running actually the code I built?".
Because `install` unlinks before writing, a process still on the old inode reports its
exe as `(deleted)`, which is how both `status` and `restart` detect drift. It also
catches apps launched straight out of `target/` rather than `~/.local/bin`.

**But it cannot see a client that is stale against `cce-ui`.** The toolkit is a
static Rust library, so committing and installing `cce-ui` itself changes nothing
about the ~20 crates that link it — each has to be rebuilt and reinstalled before
it carries the change. `status` compares each binary's mtime in `target/release`
against the one in `~/.local/bin`, so a client nobody rebuilt has both old and
equal and reads as up to date. It is stale against a *dependency*, the one kind of
staleness that check has no notion of.

Then a **running** process keeps its old inode until it is relaunched, and
`cce-fx` keeps its own until the next login. So a toolkit fix lands in three
stages — commit, rebuild dependents, relaunch — and it is the middle one that
gets skipped. Learned from cce-ui@2416904, which raised each client's
`RLIMIT_NOFILE`: the fix was committed and cce-ui installed, and every client
still ran at the old limit until its own crate was rebuilt, thirteen of them.

Sweep with one cargo invocation over the dependents (`cargo build --release -p …
-p …` — one shape, since alternating with a bare `--workspace` build re-resolves
features and invalidates crates, as below), then `ccebuild install
--no-build <crate>` for each. Verify by looking *inside* the installed binary for
something the change introduced — `strings ~/.local/bin/<crate> | grep -q
'<new log string>'` — rather than trusting that the build ran. (`cce-browser` belongs in the sweep
too, but for a different reason than previously recorded here: since
2026-08-30 its **default build is WPE WebKit** against the system
`libWPEWebKit` — seconds, ~14 MB, no Servo compiled at all. The old
leave-it-out rule dated from Servo being the default engine — always the
crates.io package, never vendored — which cost more than the rest of the
workspace combined; that backend still exists behind `--no-default-features
--features servo` and is still that expensive, so only build it deliberately.
The default flip is itself a lesson for sweeps: while WPE was opt-in, a
featureless sweep rebuild silently reverted the installed browser to the
wrong engine. Defaults are what sweeps build; an opt-in variant of a binary
does not survive one.)

**A hit proves freshness; a miss proves nothing.** Not every string literal in
the source survives into the binary, and the two cases are not distinguishable
from the outside. Measured against a current `cce-fx` on 2026-08-28: the live
`Action` names `mode_next_shared` and `toggle_overview` appear (twice and once),
while `overlay_right`, `brightness_down` and `focus_up` — same file, same kind
of literal, all reachable in `cce-window-manager/src/api.rs` — report zero.
Probably link-time constant merging; recorded as observed, not explained. So
prefer a long distinctive log string over a short match-arm literal, and never
read a zero as "the build didn't take" — that false negative has already cost a
session an afternoon of chasing a build that had worked.

When the answer actually matters, test the behavior instead of a proxy for it:
put a deliberately bogus value where the real one goes (a made-up action name in
a shadow session's `input.kdl`) and watch for the code path that rejects it —
the compositor's "unknown window-manager action" warning firing for the bogus
name and staying quiet for yours proves the running binary knows yours.

For plain cargo work:

```sh
cargo build --release                       # build every crate
cargo build -p cce-status-interface         # build one client
cargo run  -p cce-system-interface           # run one client
cargo test --workspace                       # all tests (tests are sparse)
cargo test -p cce-fx --lib config::          # tests in one module of one crate
```

Building the workspace compiles the **compositor** too, which triggers its `build.rs`
(meson/ninja to build vendored scenefx, wayland-scanner for protocols, bindgen over
wlroots). That needs native system deps — see `cce-compositor/CLAUDE.md` for the full list. If you
only touch a client, prefer `-p <crate>` to avoid rebuilding the compositor — but note
that alternating `cargo build --release` with `cargo build --release -p <crate>`
resolves different unified feature sets, so each invocation re-invalidates a few
crates (~11s). Pick one shape and stay with it.

Binary names do not reliably match the crate: `cce-fx` lives in `cce-compositor/`,
`cce-system-interface` and `cce-files` declare explicit `[[bin]]` names, and several
crates ship extra bins (`cce-ui` → `cce-ramp`/`cce-relief`, `cce-compositor` →
`ccectl`). Ask cargo rather than guessing:
`cargo metadata --no-deps --format-version 1 | jq -r '.packages[].targets[] | select(.kind|index("bin")) | .name'`.

## The `cce-ui` toolkit (start here for any client work)

`cce-ui` is a **custom retained-mode GUI toolkit**, not a wrapper around an existing
framework. Understanding it is the prerequisite for touching any client.

- **Transport**: raw `wayland-client` 0.31 + `smithay-client-toolkit` 0.19, driven by a
  `calloop` event loop. Clients are real Wayland surfaces, not toolkit windows.
- **Rendering**: raw Vulkan via **ash** (`cce-ui/src/vk/` — `VkRenderer`; the wgpu
  path was retired), with **cosmic-text** for text shaping (depended on directly
  since the wgpu retirement — it used to be reached through glyphon, whose only
  other export was the wgpu renderer nothing here used). Widgets emit
  vertex batches (quads, rounded rects, vectors, arcs, circles) — see the re-export
  list in `cce-ui/src/engine.rs`. There is no HTML/DOM; the UI is drawn as GPU
  primitives.
- **The `Application` trait** (`cce-ui/src/backend/window_runner.rs`) is the contract
  every client implements. Key methods: `new`, `settings`, `update(msg)`, `tick(dt)`,
  `display_list` (the single paint path) plus `overlay_quads` / `custom_vertices`,
  and the input hooks (`handle_pointer_move`, `handle_mouse_input`, …). Apps needing
  direct renderer access (3D scenes, app-shaped text, non-rect window chrome) use the
  extended hooks `renderer_init` / `stage_renderer` / `standard_csd` /
  `take_window_action` — `cce-designer` is the reference consumer. A client's
  `main.rs` is typically a struct implementing `Application` plus a one-line
  `cce_ui::engine::run::<MyApp>();`.
- **Modules**: `widget/` (containers, inputs, editor, `json_layout`), `layout.rs`
  (fonts + sizing, lots of `*_font_parsed()` getters), `color.rs`, `config.rs`,
  `protocol.rs` (talking to the compositor), `context.rs`, `process.rs`,
  `file_dialog.rs`, `scale.rs` (HiDPI), `mcp.rs` (tools-only MCP server over
  Streamable HTTP so apps can expose their state/actions to AI agents —
  `cce-designer` is the reference consumer, see its CLAUDE.md).

When adding a widget or a client, mirror an existing client (e.g.
`cce-status-interface`) rather than inventing a new structure.

## Configuration (shared across the whole DE)

Config is **KDL** (`kdl` crate), loaded from `~/.config/cce/` (honoring
`XDG_CONFIG_HOME`), via `cce-ui/src/config.rs`:

- **`~/.config/cce/config.kdl`** — the shared/global config (`get_config_path()`).
- **`~/.config/cce/<app-name>/config.kdl`** — per-app override
  (`get_app_config_path(app_name)`).
- **`~/.config/cce/input.kdl`** — DE-wide keybindings and pointer input settings,
  domain-scoped (`cce-ui/src/input.rs`): top-level nodes are domains
  (`cce-window-manager` for compositor actions, `cce-ui` for toolkit-wide widget
  defaults, `cce-<app>` for per-app bindings), children are `name "chord"`
  bindings. Resolution for an app is `<app>.<name>` → `cce-ui.<name>`; the
  compositor maps its domain onto `cce-window-manager::api::Action` via the
  policy crate's `bindings` module. Legacy keybind entries in `config.kdl` still
  load; `input.kdl` wins on conflict. `ccectl migrate-input` extracts config.kdl
  keybindings into input.kdl (with backup; config.kdl is never rewritten).
  A top-level `input { }` block holds global pointer hardware defaults with
  per-device-class sub-blocks (`mouse` / `trackpad` / `trackpoint`: accel,
  scroll_factor…), consumed by the compositor; an `input { }` child inside an
  app domain holds that app's scroll overrides, applied client-side by cce-ui
  (pixel deltas scale as trackpad, discrete wheel clicks as mouse). Keybinding
  and input edits are made directly on the file (e.g. via cce-data-editor) —
  there is deliberately no dedicated settings UI.
- Config edits are backed up under `~/.config/cce/backups/config.kdl.<n>.bak`.

The compositor additionally runs `~/.config/cce/init` on startup and persists window
state to `~/.local/state/cce/state.json` — details in `cce-compositor/CLAUDE.md`.

## How the pieces talk (IPC)

Clients and compositor communicate over Unix sockets keyed by `$WAYLAND_DISPLAY`:

- **Control**: `/tmp/cce-{WAYLAND_DISPLAY}.sock` — line-oriented request/reply. The
  `ccectl` binary (in `cce-compositor/`) is the CLI client; run `ccectl` with no args for the
  command list.
- **Status**: `/tmp/cce-status-{WAYLAND_DISPLAY}.sock` — subscribe to `layout` /
  `title` / `modifiers` / `dismiss` / `backdrop <app_id>` and receive push updates.
  This feeds `cce-status-interface` (the status bar). `backdrop` is the odd one
  out: it takes the asking segment's app_id and reports what that segment is
  composited over, which is the one thing a Wayland client can never see for
  itself. (The `viewport` topic went with the viewport-tag feature.)

## Repo hygiene

The repo root and `cce-compositor/scratch/` are littered with **ad-hoc debugging artifacts** — many
`screenshot_*.png`, `*.log` (some enormous, e.g. `debug.txt`, `dropbox_strace.log`),
and one-off `*.py` inspection scripts (`patch*.py`, `scan_*.py`, `inspect_*.py`). These
are **not part of the build**. Don't treat them as source, and don't add more to the
root; use the scratchpad directory for temporary files.

## Concurrent sessions (multiple agents in this workspace)

Several Claude Code sessions may be working in sibling crates **at the same
time**. The workspace shares one `target/`, one `~/.local/bin`, and one live
compositor session between them, so an unscoped command in one session damages
the others. The rules:

- **Scope builds and installs to your crate.** `cargo build --release -p
  <crate>` and `ccebuild install --no-build <crate>` — never a bare
  `ccebuild install`, which deploys *every* crate's most recent build,
  including another session's half-finished work. A concurrent build blocking
  on cargo's build-directory lock ("Blocking waiting for file lock") is
  normal — wait it out; don't kill it or conclude the build is broken.
- **Restart with `ccebuild restart <crate>`, never the bare form.** Bare
  `ccebuild restart` is unscoped: it restarts every user service running a
  replaced binary, including apps another session has installed but is not
  ready to restart. The per-crate form restarts only units shipped by the
  named crate(s). Apps that are not services restart by name
  (`pkill -x <bin>`, relaunch detached).
- **Shared crates are exclusive.** Before editing `cce-ui`,
  `cce-window-manager`, or `cce-icons`, run `git status` there. Foreign dirt
  means another session owns that crate right now — coordinate or stop; don't
  edit around it. Commit your own crate's work promptly so other sessions
  always see clean repos.
- **Verify in your own shadow session.** `cce-shadow start --new` gives each
  agent a private headless compositor. Driving the *live* session (`ccectl`
  pointer injection, screenshots, app restarts) is only safe when you know
  you are the sole session doing so — two agents share one pointer and one
  screen, and each contaminates the other's observations.
- **Coordinate through the harness.** `ListAgents` shows the other local
  Claude sessions; `SendMessage` reaches them. Before touching a shared crate
  that shows foreign dirt, ask the session that owns it instead of guessing.
