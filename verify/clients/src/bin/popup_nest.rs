// popup-nest — a toplevel with a menu and a submenu, the way Chrome's
// three-dot menu opens "Bookmarks and lists": a top-level xdg_popup
// anchored in the window, then a nested xdg_popup anchored on the menu's
// right edge, both asking to flip sideways and slide vertically to fit.
// It proves the compositor unconstrains a SUBMENU against the real screen:
// the box wlroots needs is in the root window's coordinates, and measuring
// it from the parent menu instead pushed tall submenus off the top.
//
// Prints one line per milestone, each popup's position RELATIVE TO ITS
// PARENT as the compositor configured it:
//   toplevel mapped WxH
//   menu X Y W H
//   submenu X Y W H
//
// Args: --app-id ID  --size WxH
//       --menu-at X,Y   anchor point in the window   (default 500,250)
//       --menu WxH                                     (default 300x200)
//       --sub-at X,Y    anchor point in the menu      (default 300,180)
//       --sub WxH                                      (default 200x600)

use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use wayland_client::{
    delegate_noop,
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base};

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Toplevel,
    Menu,
    Submenu,
}

struct Surf {
    role: Role,
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    size: (i32, i32),
    placed: Option<(i32, i32, i32, i32)>,
    needs_buffer: bool,
    mapped: bool,
    color: u32,
}

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    surfs: Vec<Surf>,
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
                    state.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, version.min(4), qh, ()))
                }
                "wl_shm" => state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ())),
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, version.min(3), qh, ()))
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(_: &mut Self, wm: &xdg_wm_base::XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(state: &mut Self, xdg: &xdg_surface::XdgSurface, event: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg.ack_configure(serial);
            if let Some(s) = state.surfs.iter_mut().find(|s| &s.xdg == xdg) {
                s.needs_buffer = true;
            }
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

impl Dispatch<xdg_popup::XdgPopup, Role> for State {
    fn event(state: &mut Self, _: &xdg_popup::XdgPopup, event: xdg_popup::Event, role: &Role, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            xdg_popup::Event::Configure { x, y, width, height } => {
                if let Some(s) = state.surfs.iter_mut().find(|s| s.role == *role) {
                    s.placed = Some((x, y, width, height));
                }
            }
            xdg_popup::Event::PopupDone => println!("popup done"),
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore xdg_positioner::XdgPositioner);

fn make_buffer(shm: &wl_shm::WlShm, w: i32, h: i32, color: u32, qh: &QueueHandle<State>) -> wl_buffer::WlBuffer {
    let size = (w * 4 * h) as u64;
    let fd = unsafe { libc::memfd_create(b"popup-nest\0".as_ptr() as *const _, 0) };
    assert!(fd >= 0, "memfd_create failed");
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.set_len(size).unwrap();
    let mapped = unsafe {
        libc::mmap(std::ptr::null_mut(), size as usize, libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0)
    };
    assert!(mapped != libc::MAP_FAILED, "mmap failed");
    unsafe {
        let px = mapped as *mut u32;
        for i in 0..(w * h) as usize {
            *px.add(i) = color;
        }
        libc::munmap(mapped, size as usize);
    }
    let fd: OwnedFd = OwnedFd::from(file);
    let pool = shm.create_pool(fd.as_fd(), w * 4 * h, qh, ());
    pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ())
}

fn pair(s: &str, sep: char) -> (i32, i32) {
    let (a, b) = s.split_once(sep).expect("pair");
    (a.parse().unwrap(), b.parse().unwrap())
}

fn main() {
    let mut app_id = "test.popupnest".to_string();
    let mut size = (800, 500);
    let mut menu_at = (500, 250);
    let mut menu = (300, 200);
    let mut sub_at = (300, 180);
    let mut sub = (200, 600);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--app-id" => app_id = args.next().unwrap(),
            "--size" => size = pair(&args.next().unwrap(), 'x'),
            "--menu-at" => menu_at = pair(&args.next().unwrap(), ','),
            "--menu" => menu = pair(&args.next().unwrap(), 'x'),
            "--sub-at" => sub_at = pair(&args.next().unwrap(), ','),
            "--sub" => sub = pair(&args.next().unwrap(), 'x'),
            other => panic!("unknown arg {other}"),
        }
    }

    let conn = Connection::connect_to_env().expect("connect to wayland display");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();
    let compositor = state.compositor.clone().expect("no wl_compositor");
    let shm = state.shm.clone().expect("no wl_shm");
    let wm = state.wm_base.clone().expect("no xdg_wm_base");

    // The toplevel insists on its size, so the popup arithmetic is fixed.
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_app_id(app_id);
    toplevel.set_title("popup-nest".into());
    toplevel.set_min_size(size.0, size.1);
    toplevel.set_max_size(size.0, size.1);
    surface.commit();
    state.surfs.push(Surf { role: Role::Toplevel, surface, xdg, size, placed: None, needs_buffer: false, mapped: false, color: 0xff2a_6f97 });

    // What Chrome asks for: open to the lower right of the anchor point,
    // flip sideways and slide vertically when that does not fit.
    let adjust = xdg_positioner::ConstraintAdjustment::FlipX | xdg_positioner::ConstraintAdjustment::SlideY;
    let mut menu_open = false;
    let mut sub_open = false;

    while !state.closed {
        conn.flush().unwrap();
        if let Some(guard) = conn.prepare_read() {
            let mut pfd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
            if unsafe { libc::poll(&mut pfd, 1, 50) } > 0 && (pfd.revents & libc::POLLIN) != 0 {
                let _ = guard.read();
            }
        }
        queue.dispatch_pending(&mut state).unwrap();

        for s in state.surfs.iter_mut() {
            if !s.needs_buffer {
                continue;
            }
            s.needs_buffer = false;
            let (w, h) = match (s.role, s.placed) {
                (Role::Toplevel, _) => s.size,
                (_, Some((_, _, w, h))) if w > 0 && h > 0 => (w, h),
                _ => s.size,
            };
            let buf = make_buffer(&shm, w, h, s.color, &qh);
            s.surface.attach(Some(&buf), 0, 0);
            s.surface.damage_buffer(0, 0, w, h);
            s.surface.commit();
            if !s.mapped {
                s.mapped = true;
                match (s.role, s.placed) {
                    (Role::Toplevel, _) => println!("toplevel mapped {}x{}", w, h),
                    (Role::Menu, Some((x, y, w, h))) => println!("menu {x} {y} {w} {h}"),
                    (Role::Submenu, Some((x, y, w, h))) => println!("submenu {x} {y} {w} {h}"),
                    _ => {}
                }
            }
        }

        let mapped = |state: &State, role: Role| state.surfs.iter().any(|s| s.role == role && s.mapped);
        let open = |state: &mut State, role: Role, parent: &xdg_surface::XdgSurface, at: (i32, i32), sz: (i32, i32), color: u32| {
            let pos = wm.create_positioner(&qh, ());
            pos.set_size(sz.0, sz.1);
            pos.set_anchor_rect(at.0, at.1, 1, 1);
            pos.set_anchor(xdg_positioner::Anchor::BottomRight);
            pos.set_gravity(xdg_positioner::Gravity::BottomRight);
            pos.set_constraint_adjustment(adjust);
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm.get_xdg_surface(&surface, &qh, ());
            let _popup = xdg.get_popup(Some(parent), &pos, &qh, role);
            surface.commit();
            state.surfs.push(Surf { role, surface, xdg, size: sz, placed: None, needs_buffer: false, mapped: false, color });
        };
        if !menu_open && mapped(&state, Role::Toplevel) {
            menu_open = true;
            let parent = state.surfs[0].xdg.clone();
            open(&mut state, Role::Menu, &parent, menu_at, menu, 0xffe0_e0e0);
        }
        if !sub_open && mapped(&state, Role::Menu) {
            sub_open = true;
            let parent = state.surfs.iter().find(|s| s.role == Role::Menu).unwrap().xdg.clone();
            open(&mut state, Role::Submenu, &parent, sub_at, sub, 0xffd9_8c2b);
        }
    }
}
