// ime — an input method (zwp_input_method_v2), for exercising the
// compositor's input-method relay in cce-shadow sessions: the relay's
// listeners, its keyboard grab and its popups (`input_relay.rs`,
// `input_popup.rs`). A text-input-v3 client with keyboard focus — a GTK
// entry such as `zenity --entry` — is what activates it.
//
// Prints one line per event:
//   bound | activate | deactivate | done | unavailable
//   | content-type <hint> <purpose> | grab-keymap | grab-key <code> <pressed|released>
//   | grab-released | popup-rect <x> <y> <w> <h> | committed <text> | exit
// Args:
//   --commit TEXT      on the first activation, commit TEXT to the field
//   --popup            on the first activation, map a 120x40 input popup
//   --grab SECS        grab the keyboard at once and release it after SECS
//   --exit-after SECS  destroy the input method and quit (default: run until
//                      the compositor says it is unavailable)

use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use wayland_client::{
    delegate_noop,
    protocol::{wl_buffer, wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2, zwp_input_method_manager_v2, zwp_input_method_v2,
    zwp_input_popup_surface_v2,
};

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    manager: Option<zwp_input_method_manager_v2::ZwpInputMethodManagerV2>,
    /// `done` events received: what `commit` must quote.
    serial: u32,
    /// An `activate` not yet followed by its `done`.
    activating: bool,
    /// The first activation completed, not yet acted on.
    activated: bool,
    unavailable: bool,
}

fn say(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
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
                "wl_compositor" => state.compositor = Some(registry.bind(name, version.min(4), qh, ())),
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_seat" if state.seat.is_none() => state.seat = Some(registry.bind(name, version.min(7), qh, ())),
                "zwp_input_method_manager_v2" => state.manager = Some(registry.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<zwp_input_method_v2::ZwpInputMethodV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &zwp_input_method_v2::ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_input_method_v2::Event;
        match event {
            Event::Activate => {
                state.activating = true;
                say("activate");
            }
            Event::Deactivate => {
                state.activating = false;
                say("deactivate");
            }
            Event::ContentType { hint, purpose } => {
                let (h, p) = (u32::from(hint), u32::from(purpose));
                say(&format!("content-type {h} {p}"));
            }
            Event::Done => {
                state.serial += 1;
                if std::mem::take(&mut state.activating) {
                    state.activated = true;
                }
                say("done");
            }
            Event::Unavailable => {
                state.unavailable = true;
                say("unavailable");
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2,
        event: zwp_input_popup_surface_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_input_popup_surface_v2::Event::TextInputRectangle { x, y, width, height } = event {
            say(&format!("popup-rect {x} {y} {width} {height}"));
        }
    }
}

impl Dispatch<zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
        event: zwp_input_method_keyboard_grab_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_input_method_keyboard_grab_v2::Event;
        match event {
            Event::Keymap { .. } => say("grab-keymap"),
            Event::Key { key, state, .. } => {
                let pressed = matches!(state, WEnum::Value(wl_keyboard::KeyState::Pressed));
                say(&format!("grab-key {key} {}", if pressed { "pressed" } else { "released" }));
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore zwp_input_method_manager_v2::ZwpInputMethodManagerV2);

fn make_buffer(shm: &wl_shm::WlShm, w: i32, h: i32, qh: &QueueHandle<State>) -> wl_buffer::WlBuffer {
    let size = (w * 4 * h) as usize;
    let fd = unsafe { libc::memfd_create(b"ime\0".as_ptr() as *const _, 0) };
    assert!(fd >= 0);
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.set_len(size as u64).unwrap();
    let map = unsafe { libc::mmap(std::ptr::null_mut(), size, libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0) };
    assert!(map != libc::MAP_FAILED);
    unsafe {
        let px = map as *mut u32;
        for i in 0..(w * h) as usize {
            *px.add(i) = 0xff204080;
        }
        libc::munmap(map, size);
    }
    let fd: OwnedFd = OwnedFd::from(file);
    let pool = shm.create_pool(fd.as_fd(), size as i32, qh, ());
    pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ())
}

fn main() {
    let mut commit: Option<String> = None;
    let mut popup = false;
    let mut grab_for: Option<f64> = None;
    let mut exit_after: Option<f64> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--commit" => commit = Some(args.next().expect("--commit TEXT")),
            "--popup" => popup = true,
            "--grab" => grab_for = Some(args.next().unwrap().parse().unwrap()),
            "--exit-after" => exit_after = Some(args.next().unwrap().parse().unwrap()),
            other => panic!("unknown arg {other}"),
        }
    }

    let conn = Connection::connect_to_env().expect("connect");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();

    let compositor = state.compositor.clone().expect("wl_compositor");
    let shm = state.shm.clone().expect("wl_shm");
    let seat = state.seat.clone().expect("wl_seat");
    let manager = state.manager.clone().expect("no zwp_input_method_manager_v2 advertised");
    let im = manager.get_input_method(&seat, &qh, ());
    queue.roundtrip(&mut state).unwrap();
    say("bound");

    let start = std::time::Instant::now();
    let mut grab = grab_for.map(|_| im.grab_keyboard(&qh, ()));
    let mut popup_parts: Option<(wl_surface::WlSurface, zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2)> = None;

    while !state.unavailable {
        let _ = queue.flush();
        if let Some(guard) = queue.prepare_read() {
            let mut fds = [libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 }];
            unsafe { libc::poll(fds.as_mut_ptr(), 1, 50) };
            if fds[0].revents & libc::POLLIN != 0 && guard.read().is_err() {
                break;
            }
        }
        if queue.dispatch_pending(&mut state).is_err() {
            break;
        }
        if std::mem::take(&mut state.activated) {
            if let Some(text) = commit.take() {
                im.commit_string(text.clone());
                im.commit(state.serial);
                say(&format!("committed {text}"));
            }
            if popup && popup_parts.is_none() {
                let surface = compositor.create_surface(&qh, ());
                let p = im.get_input_popup_surface(&surface, &qh, ());
                surface.attach(Some(&make_buffer(&shm, 120, 40, &qh)), 0, 0);
                surface.damage_buffer(0, 0, 120, 40);
                surface.commit();
                popup_parts = Some((surface, p));
            }
        }
        if let (Some(secs), Some(_)) = (grab_for, grab.as_ref()) {
            if start.elapsed().as_secs_f64() >= secs {
                grab.take().unwrap().release();
                say("grab-released");
            }
        }
        if exit_after.is_some_and(|secs| start.elapsed().as_secs_f64() >= secs) {
            break;
        }
    }

    if let Some((surface, p)) = popup_parts {
        p.destroy();
        surface.destroy();
    }
    if let Some(g) = grab {
        g.release();
    }
    im.destroy();
    let _ = queue.roundtrip(&mut state);
    say("exit");
}
