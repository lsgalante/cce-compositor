// float-pair — one client, two parentless floating toplevels, the second
// opened later the way a Chromium/Electron app opens an "Authorize" dialog:
// same app_id, no xdg parent, a fixed size it insists on (min == max, and it
// commits its own size whatever the configure said), a server-side
// decoration request, and an xdg-activation request BEFORE its first buffer,
// with a token issued against the first window's surface.
//
// Prints one line per milestone so a driver can wait on them:
//   main mapped | dialog created | token <t> | activated | dialog mapped
//
// Args: --app-id ID  --delay SECS  --main WxH  --dialog WxH
//       --dialog-honours-configure  --no-activate  --no-decoration
//       --bare-token (no seat/serial/surface on the token)
//       --reactivate SECS  that long after the dialog, request activation of
//                          the (by then unfocused) MAIN window — an activation
//                          for an already-mapped window
// Extra milestones: reactivate requested | reactivated

use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};
use wayland_client::{
    delegate_noop,
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

struct Win {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
    want: (i32, i32),
    honour: bool,
    pending: Option<(i32, i32)>,
    needs_buffer: bool,
    mapped: bool,
    color: u32,
    name: &'static str,
}

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    activation: Option<xdg_activation_v1::XdgActivationV1>,
    seat: Option<wl_seat::WlSeat>,
    decoration: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    wins: Vec<Win>,
    token: Option<String>,
    closed: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, version.min(4), qh, ()),
                    );
                }
                "wl_shm" => state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ())),
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()))
                }
                "xdg_activation_v1" => {
                    state.activation =
                        Some(registry.bind::<xdg_activation_v1::XdgActivationV1, _, _>(name, 1, qh, ()))
                }
                "wl_seat" => state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, 1, qh, ())),
                "zxdg_decoration_manager_v1" => {
                    state.decoration = Some(
                        registry
                            .bind::<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, _, _>(name, 1, qh, ()),
                    )
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            if let Some(win) = state.wins.iter_mut().find(|w| &w.xdg == xdg_surface) {
                win.needs_buffer = true;
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        toplevel: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                if width > 0 && height > 0 {
                    if let Some(win) = state.wins.iter_mut().find(|w| &w.toplevel == toplevel) {
                        win.pending = Some((width, height));
                    }
                }
            }
            xdg_toplevel::Event::Close => state.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &xdg_activation_token_v1::XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            state.token = Some(token);
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore xdg_activation_v1::XdgActivationV1);
delegate_noop!(State: ignore zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);
delegate_noop!(State: ignore zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1);

fn make_buffer(
    shm: &wl_shm::WlShm,
    w: i32,
    h: i32,
    color: u32,
    qh: &QueueHandle<State>,
) -> wl_buffer::WlBuffer {
    let size = (w * 4 * h) as u64;
    let fd = unsafe { libc::memfd_create(b"float-pair\0".as_ptr() as *const _, 0) };
    assert!(fd >= 0, "memfd_create failed");
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.set_len(size).unwrap();
    let mmapped = unsafe {
        libc::mmap(std::ptr::null_mut(), size as usize, libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0)
    };
    assert!(mmapped != libc::MAP_FAILED, "mmap failed");
    unsafe {
        let px = mmapped as *mut u32;
        for i in 0..(w * h) as usize {
            *px.add(i) = color;
        }
        libc::munmap(mmapped, size as usize);
    }
    let fd: OwnedFd = OwnedFd::from(file);
    let pool = shm.create_pool(fd.as_fd(), w * 4 * h, qh, ());
    // The pool object is leaked on purpose: the buffer outlives it anyway,
    // and this is a test client.
    pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ())
}

fn parse_size(s: &str) -> (i32, i32) {
    let (w, h) = s.split_once('x').expect("size is WxH");
    (w.parse().unwrap(), h.parse().unwrap())
}

fn main() {
    let mut app_id = "test.floatpair".to_string();
    let mut delay = 3.0f64;
    let mut main_size = (1024, 800);
    let mut dialog_size = (400, 370);
    let mut dialog_honours = false;
    let mut activate = true;
    let mut decoration = true;
    let mut bare_token = false;
    let mut reactivate: Option<f64> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--app-id" => app_id = args.next().unwrap(),
            "--delay" => delay = args.next().unwrap().parse().unwrap(),
            "--main" => main_size = parse_size(&args.next().unwrap()),
            "--dialog" => dialog_size = parse_size(&args.next().unwrap()),
            "--dialog-honours-configure" => dialog_honours = true,
            "--no-activate" => activate = false,
            "--no-decoration" => decoration = false,
            "--bare-token" => bare_token = true,
            "--reactivate" => reactivate = Some(args.next().unwrap().parse().unwrap()),
            other => panic!("unknown arg {other}"),
        }
    }

    let conn = Connection::connect_to_env().expect("connect to wayland display");
    let display = conn.display();
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = display.get_registry(&qh, ());

    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();

    let compositor = state.compositor.clone().expect("no wl_compositor");
    let shm = state.shm.clone().expect("no wl_shm");
    let wm_base = state.wm_base.clone().expect("no xdg_wm_base");

    let make_win = |state: &mut State, title: &str, want: (i32, i32), honour: bool, color: u32, fixed: bool, name: &'static str| {
        let surface = compositor.create_surface(&qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg.get_toplevel(&qh, ());
        toplevel.set_app_id(app_id.clone());
        toplevel.set_title(title.into());
        if fixed {
            toplevel.set_min_size(want.0, want.1);
            toplevel.set_max_size(want.0, want.1);
        }
        if decoration {
            if let Some(dm) = &state.decoration {
                let deco = dm.get_toplevel_decoration(&toplevel, &qh, ());
                deco.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide);
            }
        }
        surface.commit();
        state.wins.push(Win {
            surface,
            xdg,
            toplevel,
            want,
            honour,
            pending: None,
            needs_buffer: false,
            mapped: false,
            color,
            name,
        });
    };

    make_win(&mut state, "Main window", main_size, true, 0xff2a_6f97, false, "main");

    let start = Instant::now();
    let mut dialog_created = false;
    let mut token_requested = false;
    let mut activated = false;
    let mut react_requested = false;
    let mut reactivated = false;

    while !state.closed {
        conn.flush().unwrap();
        if let Some(guard) = conn.prepare_read() {
            let mut pfd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let n = unsafe { libc::poll(&mut pfd, 1, 50) };
            if n > 0 && (pfd.revents & libc::POLLIN) != 0 {
                let _ = guard.read();
            } else {
                drop(guard);
            }
        }
        queue.dispatch_pending(&mut state).unwrap();

        for win in state.wins.iter_mut() {
            if win.needs_buffer {
                win.needs_buffer = false;
                let size = match (win.honour, win.pending) {
                    (true, Some(p)) => p,
                    _ => win.want,
                };
                let buf = make_buffer(&shm, size.0, size.1, win.color, &qh);
                win.surface.attach(Some(&buf), 0, 0);
                win.surface.damage_buffer(0, 0, size.0, size.1);
                win.surface.commit();
                if !win.mapped {
                    win.mapped = true;
                    println!("{} mapped {}x{}", win.name, size.0, size.1);
                }
            }
        }

        // The token is fetched a second before the dialog so it is in hand
        // when the dialog is created, and the activation goes out right
        // after the dialog's initial commit — before its first buffer, the
        // way Chromium does it (the live log shows the request landing
        // between the app_id and the map).
        if activate && !token_requested && start.elapsed() + Duration::from_secs(1) >= Duration::from_secs_f64(delay) {
            token_requested = true;
            if let (Some(act), Some(seat)) = (state.activation.clone(), state.seat.clone()) {
                let tok = act.get_activation_token(&qh, ());
                tok.set_app_id(app_id.clone());
                if !bare_token {
                    tok.set_surface(&state.wins[0].surface);
                    tok.set_serial(0, &seat);
                }
                tok.commit();
            } else {
                println!("no xdg_activation_v1 or wl_seat; not activating");
            }
        }
        if !dialog_created && start.elapsed() >= Duration::from_secs_f64(delay) {
            dialog_created = true;
            make_win(&mut state, "Authorize", dialog_size, dialog_honours, 0xffd9_8c2b, true, "dialog");
            println!("dialog created");
            if let Some(t) = state.token.clone() {
                activated = true;
                println!("token {}", t);
                state.activation.as_ref().unwrap().activate(t, &state.wins[1].surface);
                println!("activated");
            }
        }
        if let Some(secs) = reactivate {
            if dialog_created && !react_requested && start.elapsed() >= Duration::from_secs_f64(delay + secs) {
                react_requested = true;
                state.token = None;
                let act = state.activation.clone().expect("no xdg_activation_v1");
                let tok = act.get_activation_token(&qh, ());
                tok.set_app_id(app_id.clone());
                tok.commit();
                println!("reactivate requested");
            }
            if react_requested && !reactivated {
                if let Some(t) = state.token.clone() {
                    reactivated = true;
                    state.activation.as_ref().unwrap().activate(t, &state.wins[0].surface);
                    println!("reactivated");
                }
            }
        }
        if dialog_created && token_requested && !activated {
            if let Some(t) = state.token.clone() {
                activated = true;
                println!("token (late) {}", t);
                state.activation.as_ref().unwrap().activate(t, &state.wins[1].surface);
                println!("activated");
            }
        }
    }
}
