// layer-popup — an on-demand overlay layer surface with an xdg popup on it,
// the shape of a launcher or a desktop menu.
//
// The layer sits at the top-left of the output (300x200) and the popup at
// (40, 40) inside it (150x100), so its centre is at (115, 90) in layout
// coordinates. A pointer press on the popup destroys the popup and its
// wl_surface — the layer stays — the way a menu closes on a pick. Until
// 2026-10-12 a click there gave the seat the POPUP's surface as its focus,
// which the layer's unmap never cleared, so the next key press read the
// freed surface (`Seat::focus_is_chrome`).
//
// Prints one line per event: mapped | popup mapped | press | popup destroyed.
// Args: --hold SECS after the popup goes (default 3).

use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};
use wayland_client::{
    delegate_noop,
    protocol::{wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    layer_configured: Option<u32>,
    popup_configured: Option<u32>,
    pressed: bool,
    over_popup: bool,
    popup_surface_id: Option<wayland_client::backend::ObjectId>,
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
                "xdg_wm_base" => state.wm_base = Some(registry.bind(name, 1, qh, ())),
                "zwlr_layer_shell_v1" => state.layer_shell = Some(registry.bind(name, version.min(4), qh, ())),
                "wl_seat" => state.seat = Some(registry.bind(name, version.min(5), qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(_: &mut Self, wm_base: &xdg_wm_base::XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(state: &mut Self, _: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, event: zwlr_layer_surface_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            state.layer_configured = Some(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(state: &mut Self, _: &xdg_surface::XdgSurface, event: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event {
            state.popup_configured = Some(serial);
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(state: &mut Self, _: &wl_pointer::WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_pointer::Event::Enter { surface, .. } => {
                state.over_popup = Some(surface.id()) == state.popup_surface_id;
            }
            wl_pointer::Event::Leave { .. } => state.over_popup = false,
            wl_pointer::Event::Button { state: WEnum::Value(wl_pointer::ButtonState::Pressed), .. } => {
                if state.over_popup {
                    state.pressed = true;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &wl_seat::WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event {
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
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
delegate_noop!(State: ignore xdg_positioner::XdgPositioner);
delegate_noop!(State: ignore xdg_popup::XdgPopup);

fn buffer(shm: &wl_shm::WlShm, w: i32, h: i32, argb: u32, qh: &QueueHandle<State>) -> wl_buffer::WlBuffer {
    let size = (w * 4 * h) as usize;
    let fd = unsafe { libc::memfd_create(b"layer-popup\0".as_ptr() as *const _, 0) };
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

fn main() {
    let mut hold = 3.0f64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--hold" => hold = args.next().unwrap().parse().unwrap(),
            other => panic!("unknown arg {other}"),
        }
    }
    let conn = Connection::connect_to_env().expect("connect");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();
    queue.roundtrip(&mut state).unwrap();

    let compositor = state.compositor.clone().expect("wl_compositor");
    let shm = state.shm.clone().expect("wl_shm");
    let wm_base = state.wm_base.clone().expect("xdg_wm_base");
    let layer_shell = state.layer_shell.clone().expect("zwlr_layer_shell_v1");

    let surface = compositor.create_surface(&qh, ());
    let layer = layer_shell.get_layer_surface(&surface, None, zwlr_layer_shell_v1::Layer::Overlay, "layer-popup-test".into(), &qh, ());
    layer.set_size(300, 200);
    layer.set_anchor(zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Left);
    layer.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand);
    surface.commit();
    while state.layer_configured.is_none() {
        queue.blocking_dispatch(&mut state).unwrap();
    }
    layer.ack_configure(state.layer_configured.unwrap());
    surface.attach(Some(&buffer(&shm, 300, 200, 0xff304060, &qh)), 0, 0);
    surface.damage_buffer(0, 0, 300, 200);
    surface.commit();
    say("mapped");

    let positioner = wm_base.create_positioner(&qh, ());
    positioner.set_size(150, 100);
    positioner.set_anchor_rect(40, 40, 1, 1);
    positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
    positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
    let popup_surface = compositor.create_surface(&qh, ());
    state.popup_surface_id = Some(popup_surface.id());
    let popup_xdg = wm_base.get_xdg_surface(&popup_surface, &qh, ());
    let popup = popup_xdg.get_popup(None, &positioner, &qh, ());
    layer.get_popup(&popup);
    popup_surface.commit();
    while state.popup_configured.is_none() {
        queue.blocking_dispatch(&mut state).unwrap();
    }
    popup_xdg.ack_configure(state.popup_configured.unwrap());
    popup_surface.attach(Some(&buffer(&shm, 150, 100, 0xffc08030, &qh)), 0, 0);
    popup_surface.damage_buffer(0, 0, 150, 100);
    popup_surface.commit();
    say("popup mapped");

    while !state.pressed {
        if queue.blocking_dispatch(&mut state).is_err() {
            return;
        }
    }
    say("press");
    popup.destroy();
    popup_xdg.destroy();
    popup_surface.destroy();
    conn.flush().unwrap();
    say("popup destroyed");

    let end = Instant::now() + Duration::from_secs_f64(hold);
    while Instant::now() < end {
        let _ = queue.dispatch_pending(&mut state);
        let _ = conn.flush();
        std::thread::sleep(Duration::from_millis(50));
    }
    say("done");
}
