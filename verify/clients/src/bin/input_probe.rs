// input-probe — one toplevel that prints every pointer and keyboard event it
// receives, for checking what the compositor synthesizes on a client's
// behalf (touchpad view drags, the held Shift of an hscroll-shift app).
//
// Prints one line per event, values only (no serials or times):
//   mapped | enter | leave | motion <sx> <sy> | button <code> <pressed|released>
//   axis <h|v> <value> | axis-stop <h|v> | axis-source <n> | frame
//   kbd-enter | kbd-leave | key <code> <pressed|released>
//   mods <depressed> <latched> <locked> <group>
//   touch-down <id> <sx> <sy> | touch-motion <id> <sx> <sy> | touch-up <id>
//   | touch-cancel
// Args: --app-id ID  --size WxH  --exit-unmapped SECS (ack the first
// configure but never attach a buffer, then quit after SECS: a toplevel the
// compositor manages but that never maps).

use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use wayland_client::{
    delegate_noop,
    protocol::{wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    touch: Option<wl_touch::WlTouch>,
    needs_buffer: bool,
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
                "wl_seat" => {
                    let _: wl_seat::WlSeat = registry.bind(name, version.min(7), qh, ());
                }
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

fn axis_name(a: WEnum<wl_pointer::Axis>) -> &'static str {
    match a {
        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => "h",
        _ => "v",
    }
}

fn pressed(s: bool) -> &'static str {
    if s { "pressed" } else { "released" }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(_: &mut Self, _: &wl_pointer::WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_pointer::Event::Enter { .. } => say("enter"),
            wl_pointer::Event::Leave { .. } => say("leave"),
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => say(&format!("motion {surface_x:.0} {surface_y:.0}")),
            wl_pointer::Event::Button { button, state, .. } => {
                say(&format!("button {button} {}", pressed(matches!(state, WEnum::Value(wl_pointer::ButtonState::Pressed)))))
            }
            wl_pointer::Event::Axis { axis, value, .. } => say(&format!("axis {} {value:.2}", axis_name(axis))),
            wl_pointer::Event::AxisStop { axis, .. } => say(&format!("axis-stop {}", axis_name(axis))),
            wl_pointer::Event::AxisSource { axis_source } => say(&format!("axis-source {}", u32::from(axis_source))),
            wl_pointer::Event::Frame => say("frame"),
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(_: &mut Self, _: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_keyboard::Event::Enter { .. } => say("kbd-enter"),
            wl_keyboard::Event::Leave { .. } => say("kbd-leave"),
            wl_keyboard::Event::Key { key, state, .. } => {
                say(&format!("key {key} {}", pressed(matches!(state, WEnum::Value(wl_keyboard::KeyState::Pressed)))))
            }
            wl_keyboard::Event::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. } => {
                say(&format!("mods {mods_depressed} {mods_latched} {mods_locked} {group}"))
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for State {
    fn event(_: &mut Self, _: &wl_touch::WlTouch, event: wl_touch::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_touch::Event::Down { id, x, y, .. } => say(&format!("touch-down {id} {x:.0} {y:.0}")),
            wl_touch::Event::Motion { id, x, y, .. } => say(&format!("touch-motion {id} {x:.0} {y:.0}")),
            wl_touch::Event::Up { id, .. } => say(&format!("touch-up {id}")),
            wl_touch::Event::Cancel => say("touch-cancel"),
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
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Touch) && state.touch.is_none() {
                state.touch = Some(seat.get_touch(qh, ()));
            }
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_surface::WlSurface);

fn make_buffer(shm: &wl_shm::WlShm, w: i32, h: i32, qh: &QueueHandle<State>) -> wl_buffer::WlBuffer {
    let size = (w * 4 * h) as usize;
    let fd = unsafe { libc::memfd_create(b"input-probe\0".as_ptr() as *const _, 0) };
    assert!(fd >= 0);
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.set_len(size as u64).unwrap();
    let map = unsafe { libc::mmap(std::ptr::null_mut(), size, libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0) };
    assert!(map != libc::MAP_FAILED);
    unsafe {
        let px = map as *mut u32;
        for i in 0..(w * h) as usize {
            *px.add(i) = 0xff406040;
        }
        libc::munmap(map, size);
    }
    let fd: OwnedFd = OwnedFd::from(file);
    let pool = shm.create_pool(fd.as_fd(), size as i32, qh, ());
    pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ())
}

fn main() {
    let mut app_id = "test.inputprobe".to_string();
    let mut size = (600, 400);
    let mut exit_unmapped: Option<f64> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--app-id" => app_id = args.next().unwrap(),
            "--exit-unmapped" => exit_unmapped = Some(args.next().unwrap().parse().unwrap()),
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
    let wm_base = state.wm_base.clone().expect("xdg_wm_base");
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_app_id(app_id);
    toplevel.set_title("input-probe".into());
    surface.commit();

    if let Some(secs) = exit_unmapped {
        while !state.needs_buffer {
            queue.blocking_dispatch(&mut state).unwrap();
        }
        say("configured");
        let end = std::time::Instant::now() + std::time::Duration::from_secs_f64(secs);
        while std::time::Instant::now() < end {
            let _ = queue.dispatch_pending(&mut state);
            let _ = conn.flush();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        toplevel.destroy();
        xdg.destroy();
        surface.destroy();
        let _ = conn.flush();
        say("exit");
        return;
    }

    let mut mapped = false;
    while !state.closed {
        if queue.blocking_dispatch(&mut state).is_err() {
            break;
        }
        if state.needs_buffer {
            state.needs_buffer = false;
            let buffer = make_buffer(state.shm.as_ref().expect("wl_shm"), size.0, size.1, &qh);
            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, size.0, size.1);
            surface.commit();
            if !mapped {
                mapped = true;
                say("mapped");
            }
        }
    }
}
