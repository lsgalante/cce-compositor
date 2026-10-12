// layer-bar — one wlr-layer-shell surface, configured from the command line:
// the shape of a bar, a dock or a launcher.
//
// Anchored to --edge (top|bottom|left|right: that edge and both of its
// neighbours, the way a bar spans the screen; or `none`, centred) at --size
// WxH (0 stretches along an anchored pair), in --layer, with --exclusive N
// and --keyboard none|exclusive|on-demand, under --namespace. Painted solid
// --color (ARGB hex). The compositor closes a surface whose exclusive zone
// would leave less than half the output (`LayerShellOutput::send_configures`).
//
// --fade-out-after SECS sends `fade-out` on the control socket that long
// after mapping — the close-fade a client asks for itself, which the
// compositor resolves to this process's surfaces by SO_PEERCRED — and prints
// the reply (the ramp's length in ms; 0 means nothing of ours was found).
//
// Prints one line per event: configure WxH | mapped | closed | enter | leave
// | key | fade-out reply N. Args also: --hold SECS (default 5; exits early
// once closed).

use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};
use wayland_client::{
    delegate_noop,
    protocol::{wl_buffer, wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    configured: Option<(u32, u32, u32)>,
    closed: bool,
}

fn say(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(state: &mut Self, registry: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => state.compositor = Some(registry.bind(name, version.min(4), qh, ())),
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "zwlr_layer_shell_v1" => state.layer_shell = Some(registry.bind(name, version.min(4), qh, ())),
                "wl_seat" => state.seat = Some(registry.bind(name, version.min(5), qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(state: &mut Self, _: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, event: zwlr_layer_surface_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                say(&format!("configure {width}x{height}"));
                state.configured = Some((serial, width, height));
            }
            zwlr_layer_surface_v1::Event::Closed => {
                say("closed");
                state.closed = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(_: &mut Self, _: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_keyboard::Event::Enter { .. } => say("enter"),
            wl_keyboard::Event::Leave { .. } => say("leave"),
            wl_keyboard::Event::Key { state: WEnum::Value(wl_keyboard::KeyState::Pressed), key, .. } => say(&format!("key {key}")),
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &wl_seat::WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event {
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

fn buffer(shm: &wl_shm::WlShm, w: i32, h: i32, argb: u32, qh: &QueueHandle<State>) -> wl_buffer::WlBuffer {
    let size = (w * 4 * h) as usize;
    let fd = unsafe { libc::memfd_create(b"layer-bar\0".as_ptr() as *const _, 0) };
    assert!(fd >= 0);
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.set_len(size as u64).unwrap();
    let map = unsafe { libc::mmap(std::ptr::null_mut(), size, libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0) };
    assert!(map != libc::MAP_FAILED);
    unsafe {
        let px = map as *mut u32;
        for i in 0..(w * h) as usize {
            *px.add(i) = argb;
        }
        libc::munmap(map, size);
    }
    let fd: OwnedFd = OwnedFd::from(file);
    let pool = shm.create_pool(fd.as_fd(), size as i32, qh, ());
    pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ())
}

/// Send `fade-out` on the compositor's control socket and return its reply.
fn fade_out() -> String {
    use std::io::{Read, Write as _};
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    let path = format!("/tmp/cce-{display}.sock");
    let Ok(mut sock) = std::os::unix::net::UnixStream::connect(&path) else {
        return format!("error: no control socket at {path}");
    };
    let _ = sock.write_all(b"fade-out\n");
    let _ = sock.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    let _ = sock.read_to_string(&mut reply);
    reply.trim().to_string()
}

fn main() {
    let mut edge = String::from("top");
    let mut size = (0u32, 30u32);
    let mut layer = zwlr_layer_shell_v1::Layer::Top;
    let mut exclusive = 0i32;
    let mut keyboard = zwlr_layer_surface_v1::KeyboardInteractivity::None;
    let mut namespace = String::from("layer-bar-test");
    let mut color = 0xff406080u32;
    let mut hold = 5.0f64;
    let mut fade_after: Option<f64> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--edge" => edge = args.next().unwrap(),
            "--size" => {
                let s = args.next().unwrap();
                let (w, h) = s.split_once('x').expect("size is WxH");
                size = (w.parse().unwrap(), h.parse().unwrap());
            }
            "--layer" => {
                layer = match args.next().unwrap().as_str() {
                    "background" => zwlr_layer_shell_v1::Layer::Background,
                    "bottom" => zwlr_layer_shell_v1::Layer::Bottom,
                    "top" => zwlr_layer_shell_v1::Layer::Top,
                    "overlay" => zwlr_layer_shell_v1::Layer::Overlay,
                    other => panic!("unknown layer {other}"),
                }
            }
            "--exclusive" => exclusive = args.next().unwrap().parse().unwrap(),
            "--keyboard" => {
                keyboard = match args.next().unwrap().as_str() {
                    "none" => zwlr_layer_surface_v1::KeyboardInteractivity::None,
                    "exclusive" => zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
                    "on-demand" => zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
                    other => panic!("unknown keyboard mode {other}"),
                }
            }
            "--namespace" => namespace = args.next().unwrap(),
            "--color" => color = u32::from_str_radix(args.next().unwrap().trim_start_matches("0x"), 16).unwrap(),
            "--hold" => hold = args.next().unwrap().parse().unwrap(),
            "--fade-out-after" => fade_after = Some(args.next().unwrap().parse().unwrap()),
            other => panic!("unknown arg {other}"),
        }
    }
    use zwlr_layer_surface_v1::Anchor;
    let anchor = match edge.as_str() {
        "top" => Anchor::Top | Anchor::Left | Anchor::Right,
        "bottom" => Anchor::Bottom | Anchor::Left | Anchor::Right,
        "left" => Anchor::Left | Anchor::Top | Anchor::Bottom,
        "right" => Anchor::Right | Anchor::Top | Anchor::Bottom,
        "none" => Anchor::empty(),
        other => panic!("unknown edge {other}"),
    };

    let conn = Connection::connect_to_env().expect("connect");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();
    queue.roundtrip(&mut state).unwrap();

    let compositor = state.compositor.clone().expect("wl_compositor");
    let shm = state.shm.clone().expect("wl_shm");
    let layer_shell = state.layer_shell.clone().expect("zwlr_layer_shell_v1");
    let surface = compositor.create_surface(&qh, ());
    let layer_surface = layer_shell.get_layer_surface(&surface, None, layer, namespace, &qh, ());
    layer_surface.set_size(size.0, size.1);
    layer_surface.set_anchor(anchor);
    layer_surface.set_exclusive_zone(exclusive);
    layer_surface.set_keyboard_interactivity(keyboard);
    surface.commit();

    let deadline = Instant::now() + Duration::from_secs_f64(hold);
    let mut mapped = false;
    let mut mapped_at: Option<Instant> = None;
    while Instant::now() < deadline && !state.closed {
        if let (Some(after), Some(at)) = (fade_after, mapped_at) {
            if at.elapsed() >= Duration::from_secs_f64(after) {
                fade_after = None;
                say(&format!("fade-out reply {}", fade_out()));
            }
        }
        queue.flush().unwrap();
        if let Some(guard) = queue.prepare_read() {
            let mut fds = [libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 }];
            let ms = deadline.saturating_duration_since(Instant::now()).as_millis().min(100) as i32;
            unsafe { libc::poll(fds.as_mut_ptr(), 1, ms) };
            if fds[0].revents & libc::POLLIN != 0 {
                let _ = guard.read();
            }
        }
        if queue.dispatch_pending(&mut state).is_err() {
            break;
        }
        if let Some((serial, w, h)) = state.configured.take() {
            layer_surface.ack_configure(serial);
            let (w, h) = (w.max(1) as i32, h.max(1) as i32);
            surface.attach(Some(&buffer(&shm, w, h, color, &qh)), 0, 0);
            surface.damage_buffer(0, 0, w, h);
            surface.commit();
            if !mapped {
                mapped = true;
                mapped_at = Some(Instant::now());
                say("mapped");
            }
        }
    }
    say("exit");
}
