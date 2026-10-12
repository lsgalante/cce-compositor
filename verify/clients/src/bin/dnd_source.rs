// dnd-source — a toplevel that starts a drag, with a 32x32 drag icon, when a
// pointer button is pressed on it; the drag then follows the pointer until
// the button is released.
//
// Prints one line per event: mapped | drag started | target <mime|none>
// | dropped | cancelled | finished. Args: --app-id ID  --size WxH

use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source, wl_pointer,
        wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    ddm: Option<wl_data_device_manager::WlDataDeviceManager>,
    pointer: Option<wl_pointer::WlPointer>,
    needs_buffer: bool,
    press_serial: Option<u32>,
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
                "xdg_wm_base" => state.wm_base = Some(registry.bind(name, 1, qh, ())),
                "wl_seat" => state.seat = Some(registry.bind(name, version.min(5), qh, ())),
                "wl_data_device_manager" => state.ddm = Some(registry.bind(name, version.min(3), qh, ())),
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

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(state: &mut Self, xdg_surface: &xdg_surface::XdgSurface, event: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.needs_buffer = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(state: &mut Self, _: &xdg_toplevel::XdgToplevel, event: xdg_toplevel::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_toplevel::Event::Close = event {
            state.closed = true;
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(state: &mut Self, _: &wl_pointer::WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_pointer::Event::Button { serial, state: WEnum::Value(wl_pointer::ButtonState::Pressed), .. } = event {
            state.press_serial = Some(serial);
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

impl Dispatch<wl_data_source::WlDataSource, ()> for State {
    fn event(_: &mut Self, _: &wl_data_source::WlDataSource, event: wl_data_source::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_data_source::Event::Target { mime_type } => say(&format!("target {}", mime_type.as_deref().unwrap_or("none"))),
            wl_data_source::Event::DndDropPerformed => say("dropped"),
            wl_data_source::Event::Cancelled => say("cancelled"),
            wl_data_source::Event::DndFinished => say("finished"),
            _ => {}
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn event(_: &mut Self, _: &wl_data_device::WlDataDevice, _: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}

    wayland_client::event_created_child!(State, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(State: ignore wl_data_offer::WlDataOffer);

fn buffer(shm: &wl_shm::WlShm, w: i32, h: i32, argb: u32, qh: &QueueHandle<State>) -> wl_buffer::WlBuffer {
    let size = (w * 4 * h) as usize;
    let fd = unsafe { libc::memfd_create(b"dnd-source\0".as_ptr() as *const _, 0) };
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
    let mut app_id = "test.dndsource".to_string();
    let mut size = (400, 300);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--app-id" => app_id = args.next().unwrap(),
            "--size" => {
                let s = args.next().unwrap();
                let (w, h) = s.split_once('x').expect("size is WxH");
                size = (w.parse().unwrap(), h.parse().unwrap());
            }
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
    let ddm = state.ddm.clone().expect("wl_data_device_manager");
    let seat = state.seat.clone().expect("wl_seat");
    let data_device = ddm.get_data_device(&seat, &qh, ());

    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_app_id(app_id);
    toplevel.set_title("dnd-source".into());
    surface.commit();

    let mut mapped = false;
    let mut started = false;
    while !state.closed {
        if queue.blocking_dispatch(&mut state).is_err() {
            break;
        }
        if state.needs_buffer {
            state.needs_buffer = false;
            surface.attach(Some(&buffer(&shm, size.0, size.1, 0xff304860, &qh)), 0, 0);
            surface.damage_buffer(0, 0, size.0, size.1);
            surface.commit();
            if !mapped {
                mapped = true;
                say("mapped");
            }
        }
        if let (Some(serial), false) = (state.press_serial, started) {
            started = true;
            let source = ddm.create_data_source(&qh, ());
            source.offer("text/plain".into());
            source.set_actions(wl_data_device_manager::DndAction::Copy);
            let icon = compositor.create_surface(&qh, ());
            icon.attach(Some(&buffer(&shm, 32, 32, 0xffe0a020, &qh)), 0, 0);
            icon.damage_buffer(0, 0, 32, 32);
            data_device.start_drag(Some(&source), &surface, Some(&icon), serial);
            icon.commit();
            say("drag started");
        }
    }
}
