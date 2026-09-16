# cce-fx

`cce-fx` is a standalone Wayland compositor and tiling window manager written in Rust,
built on **wlroots 0.20** (via FFI) with a vendored **scenefx** for blur and
rounded-corner scene effects. It began as a Rust rewrite of the
[river](https://isaacfreund.com/software/river) compositor and remains GPL-3.0 licensed.

This crate is the compositor. It lives inside the larger `cce` Cargo workspace (root at
the parent directory), alongside client apps such as `cce-status-interface`,
`cce-system-interface`, and other `cce-*` siblings that connect to it over its sockets.

## Binaries

- **`cce-fx`** (installed and symlinked as `cce`) — the compositor server. Running it
  starts the monolithic compositor and window manager.
- **`ccectl`** — the control client. It talks to a running server over the control
  socket. Run `ccectl` with no arguments to list the available commands (layout, view,
  mode, viewport, pointer/key injection, bind, spawn, notify, reload, exit, …).

## Build & install

```sh
make build      # cargo build --release
make run        # cargo run --bin cce-fx
make install    # build, then install to ~/.local/bin
make clean      # cargo clean
```

`make install` places `cce-fx` (symlinked as `cce`), `ccectl`, the `scripts/*` menu
helpers, and `gpu-watcher` (plus its systemd user service) into `~/.local/bin`.

### Build requirements

The build (`build.rs`) compiles the vendored `scenefx/` with **meson**/**ninja** on
first run, generates Wayland protocol code with **wayland-scanner**, and binds wlroots
via **bindgen**. It requires these system packages (found through `pkg-config`):
`wlroots-0.20`, `wayland-server`, `xkbcommon`, `pixman-1`, `libinput`, `libevdev`, and
the GL/DRM stack (`GLESv2`, `EGL`, `drm`, `gbm`, `lcms2`), plus the system
`wayland-protocols` XML definitions.

## Configuration

On startup the server loads **`$XDG_CONFIG_HOME/cce/config.kdl`** (default
`~/.config/cce/config.kdl`), merging an adjacent `input.kdl` for key bindings and input
settings. The config format is [KDL](https://kdl.dev). A startup script at
`~/.config/cce/init` is run via `sh -c`. Window layout state is persisted to
`~/.local/state/cce/state.json` on exit and restored on the next start.

Configuration can also be changed live over the control socket with `ccectl`.

Idle timeouts go in an `idle { }` block: `display_off` and `sleep` are seconds of
no input (0, the default, disables one), and `sleep_command` overrides the
default `systemctl suspend`:

```kdl
idle {
    display_off 600
    sleep 1800
}
```

Any pointer or key input wakes the darkened outputs and restarts both countdowns; an
idle-inhibitor held by a mapped surface (a playing video) pauses them. `ccectl idle`
reports the state, and `ccectl idle timeouts <display_off> <sleep>` sets them live.

## Development

See [CLAUDE.md](CLAUDE.md) for a detailed tour of the architecture, the FFI
conventions, the `build.rs` pipeline, and the IPC/status socket protocols.
