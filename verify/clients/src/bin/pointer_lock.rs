// pointer-lock — one toplevel that asks the compositor to lock (or confine)
// the pointer on its surface, persistently, and listens to relative motion:
// the shape of a game capturing the mouse.
//
// Prints one line per event so a driver can wait on them:
//   mapped | locked | unlocked | confined | unconfined
//   enter <sx> <sy> | motion <sx> <sy> | leave | relative <dx> <dy> (running sums)
//
// Args: --mode lock|confine  --app-id ID  --size WxH

use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use wayland_client::{
    delegate_noop,
    protocol::{wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::wp::pointer_constraints::zv1::client::{
    zwp_confined_pointer_v1, zwp_locked_pointer_v1, zwp_pointer_constraints_v1,
};
use wayland_protocols::wp::relative_pointer::zv1::client::{
    zwp_relative_pointer_manager_v1, zwp_relative_pointer_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    constraints: Option<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1>,
    relative: Option<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1>,
    pointer: Option<wl_pointer::WlPointer>,
    needs_buffer: bool,
    closed: bool,
    rel: (f64, f64),
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
                "wl_compositor" => {
                    state.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, version.min(4), qh, ()))
                }
                "wl_shm" => state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ())),
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()))
                }
                "wl_seat" => state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(5), qh, ())),
                "zwp_pointer_constraints_v1" => {
                    state.constraints = Some(
                        registry.bind::<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1, _, _>(name, 1, qh, ()),
                    )
                }
                "zwp_relative_pointer_manager_v1" => {
                    state.relative = Some(
                        registry
                            .bind::<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1, _, _>(name, 1, qh, ()),
                    )
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

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(_: &mut Self, _: &wl_pointer::WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_pointer::Event::Enter { surface_x, surface_y, .. } => say(&format!("enter {surface_x:.0} {surface_y:.0}")),
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => say(&format!("motion {surface_x:.0} {surface_y:.0}")),
            wl_pointer::Event::Leave { .. } => say("leave"),
            _ => {}
        }
    }
}

impl Dispatch<zwp_relative_pointer_v1::ZwpRelativePointerV1, ()> for State {
    fn event(state: &mut Self, _: &zwp_relative_pointer_v1::ZwpRelativePointerV1, event: zwp_relative_pointer_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zwp_relative_pointer_v1::Event::RelativeMotion { dx, dy, .. } = event {
            state.rel.0 += dx;
            state.rel.1 += dy;
            say(&format!("relative {:.0} {:.0}", state.rel.0, state.rel.1));
        }
    }
}

impl Dispatch<zwp_locked_pointer_v1::ZwpLockedPointerV1, ()> for State {
    fn event(_: &mut Self, _: &zwp_locked_pointer_v1::ZwpLockedPointerV1, event: zwp_locked_pointer_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwp_locked_pointer_v1::Event::Locked => say("locked"),
            zwp_locked_pointer_v1::Event::Unlocked => say("unlocked"),
            _ => {}
        }
    }
}

impl Dispatch<zwp_confined_pointer_v1::ZwpConfinedPointerV1, ()> for State {
    fn event(_: &mut Self, _: &zwp_confined_pointer_v1::ZwpConfinedPointerV1, event: zwp_confined_pointer_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwp_confined_pointer_v1::Event::Confined => say("confined"),
            zwp_confined_pointer_v1::Event::Unconfined => say("unconfined"),
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
delegate_noop!(State: ignore zwp_pointer_constraints_v1::ZwpPointerConstraintsV1);
delegate_noop!(State: ignore zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1);

fn make_buffer(shm: &wl_shm::WlShm, w: i32, h: i32, qh: &QueueHandle<State>) -> wl_buffer::WlBuffer {
    let size = (w * 4 * h) as u64;
    let fd = unsafe { libc::memfd_create(b"pointer-lock\0".as_ptr() as *const _, 0) };
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
            *px.add(i) = 0xff3a6ea5;
        }
        libc::munmap(mmapped, size as usize);
    }
    let fd: OwnedFd = OwnedFd::from(file);
    let pool = shm.create_pool(fd.as_fd(), w * 4 * h, qh, ());
    pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ())
}

fn main() {
    let mut mode = "lock".to_string();
    let mut app_id = "test.pointerlock".to_string();
    let mut size = (600, 400);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--mode" => mode = args.next().unwrap(),
            "--app-id" => app_id = args.next().unwrap(),
            "--size" => {
                let s = args.next().unwrap();
                let (w, h) = s.split_once('x').expect("size is WxH");
                size = (w.parse().unwrap(), h.parse().unwrap());
            }
            other => panic!("unknown arg {other}"),
        }
    }

    let conn = Connection::connect_to_env().expect("connect to wayland display");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();
    queue.roundtrip(&mut state).unwrap();

    let compositor = state.compositor.clone().expect("no wl_compositor");
    let wm_base = state.wm_base.clone().expect("no xdg_wm_base");
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_app_id(app_id);
    toplevel.set_title("pointer-lock".into());
    surface.commit();

    let mut mapped = false;
    let mut constrained = false;
    while !state.closed {
        if queue.blocking_dispatch(&mut state).is_err() {
            break; // the compositor went away
        }
        if state.needs_buffer {
            state.needs_buffer = false;
            let buffer = make_buffer(state.shm.as_ref().expect("no wl_shm"), size.0, size.1, &qh);
            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, size.0, size.1);
            surface.commit();
            if !mapped {
                mapped = true;
                say("mapped");
            }
        }
        if mapped && !constrained {
            if let (Some(pointer), Some(constraints)) = (state.pointer.clone(), state.constraints.clone()) {
                constrained = true;
                let lifetime = zwp_pointer_constraints_v1::Lifetime::Persistent;
                if mode == "confine" {
                    constraints.confine_pointer(&surface, &pointer, None, lifetime, &qh, ());
                } else {
                    constraints.lock_pointer(&surface, &pointer, None, lifetime, &qh, ());
                }
                if let Some(relative) = state.relative.clone() {
                    relative.get_relative_pointer(&pointer, &qh, ());
                }
                surface.commit();
                say(&format!("requested {mode}"));
            }
        }
    }
}
