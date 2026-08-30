// status-stub — a fake status segment for behavioral tests.
//
// Maps an xdg toplevel with app_id "cce-status-teststub", which the compositor
// roles as a StatusBar (any "cce-status*" app_id) and therefore sizes from the
// client's own content. It maps EXPANDED_H tall — thicker than any sane
// bar_height, so `any_expanded_status_segment` reads it as an open in-surface
// menu. It subscribes to the `dismiss` topic on the compositor's status socket
// and reacts the way the real bar does: on the first dismiss push it shrinks
// to SHRUNK_H (back to a bar strip, menu closed), printing one
// "dismiss <payload>" line to stdout per push so a driver script can count
// them. Runs until killed (the cce-shadow stop sweep finds it by HOME).

use std::io::{BufRead, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::mpsc;
use wayland_client::{
    delegate_noop,
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const WIDTH: i32 = 360;
const EXPANDED_H: i32 = 400; // > bar_height: reads as an open menu
const SHRUNK_H: i32 = 8; // <= bar_height (default 24): plain bar strip

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    configured: bool,
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
                "wl_shm" => {
                    state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
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
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Configure sizes are ignored: a Status window sizes itself.
        if let xdg_toplevel::Event::Close = event {
            state.closed = true;
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_surface::WlSurface);

/// One shm pool big enough for the expanded buffer; both buffers share it
/// (contents are a solid fill, overlap does not matter).
fn make_pool_fd() -> OwnedFd {
    let size = (WIDTH * 4 * EXPANDED_H) as u64;
    let fd = unsafe { libc::memfd_create(b"status-stub\0".as_ptr() as *const _, 0) };
    assert!(fd >= 0, "memfd_create failed");
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.set_len(size).unwrap();
    // Solid opaque slate; any visible pixels will do.
    let mmapped = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size as usize,
            libc::PROT_WRITE,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            0,
        )
    };
    assert!(mmapped != libc::MAP_FAILED, "mmap failed");
    unsafe {
        let px = mmapped as *mut u32;
        for i in 0..(WIDTH * EXPANDED_H) as usize {
            *px.add(i) = 0xff30_3a4a;
        }
        libc::munmap(mmapped, size as usize);
    }
    OwnedFd::from(file)
}

fn spawn_dismiss_listener(tx: mpsc::Sender<()>) {
    std::thread::spawn(move || {
        let display = std::env::var("WAYLAND_DISPLAY").expect("WAYLAND_DISPLAY not set");
        let path = format!("/tmp/cce-status-interface-{}.sock", display);
        let mut stream = None;
        for _ in 0..50 {
            match std::os::unix::net::UnixStream::connect(&path) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        }
        let mut stream = stream.expect("could not connect to status socket");
        stream.write_all(b"dismiss\n").unwrap();
        let reader = std::io::BufReader::new(stream);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            println!("dismiss {}", line);
            std::io::stdout().flush().ok();
            let _ = tx.send(());
        }
    });
}

fn main() {
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

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_app_id("cce-status-teststub".into());
    toplevel.set_title("escape-dismiss-test".into());
    surface.commit();

    // First configure before the first buffer, per xdg-shell.
    while !state.configured {
        queue.blocking_dispatch(&mut state).unwrap();
    }

    let pool_fd = make_pool_fd();
    let pool = shm.create_pool(pool_fd.as_fd(), WIDTH * 4 * EXPANDED_H, &qh, ());
    let tall = pool.create_buffer(
        0, WIDTH, EXPANDED_H, WIDTH * 4, wl_shm::Format::Argb8888, &qh, (),
    );
    let short = pool.create_buffer(
        0, WIDTH, SHRUNK_H, WIDTH * 4, wl_shm::Format::Argb8888, &qh, (),
    );

    surface.attach(Some(&tall), 0, 0);
    surface.damage_buffer(0, 0, WIDTH, EXPANDED_H);
    surface.commit();

    let (tx, rx) = mpsc::channel();
    spawn_dismiss_listener(tx);

    let mut shrunk = false;
    while !state.closed {
        conn.flush().unwrap();
        if let Some(guard) = conn.prepare_read() {
            let mut pfd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let n = unsafe { libc::poll(&mut pfd, 1, 100) };
            if n > 0 && (pfd.revents & libc::POLLIN) != 0 {
                let _ = guard.read();
            } else {
                drop(guard);
            }
        }
        queue.dispatch_pending(&mut state).unwrap();

        if !shrunk && rx.try_recv().is_ok() {
            surface.attach(Some(&short), 0, 0);
            surface.damage_buffer(0, 0, WIDTH, SHRUNK_H);
            surface.commit();
            shrunk = true;
        }
    }
}
