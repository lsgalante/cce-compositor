// vptr — drive the pointer through zwlr_virtual_pointer_v1, for exercising
// the compositor's cursor listeners in cce-shadow sessions.
//
// The compositor attaches the virtual device to the seat's wlr_cursor, so its
// events arrive on the cursor's motion / motion_absolute / button / axis /
// frame signals — the path a hardware pointer takes (`handle_motion` and kin
// in cursor.rs). `ccectl pointer-*` does not: it calls the handlers' `on_*`
// functions directly, so it never reaches a cursor listener.
//
// Usage: vptr <step> ...
//   move DX DY       relative motion
//   abs X Y          absolute motion, in pixels of a 1280x720 layout (the
//                    headless output); scaled through x/y_extent
//   press B          button down; B is left, right, middle or an evdev code
//   release B        button up
//   click B          press then release
//   scroll DY        vertical wheel motion (15 is one notch), as a wheel
//   sleep MS
// Each step but sleep ends with a frame. Prints one line per step sent, then
// "done" after a roundtrip, then destroys the device (which detaches it).

use std::io::Write;
use wayland_client::{
    protocol::{wl_pointer, wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
};

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
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
        if let wl_registry::Event::Global {
            name, interface, version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, ()));
                }
                "zwlr_virtual_pointer_manager_v1" => {
                    state.manager = Some(
                        registry
                            .bind::<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, _, _>(
                                name, 1, qh, (),
                            ),
                    );
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, ()> for State {
    fn event(
        _: &mut Self, _: &zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
        _: zwlr_virtual_pointer_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1, ()> for State {
    fn event(
        _: &mut Self, _: &zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
        _: zwlr_virtual_pointer_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
    }
}

fn button_code(name: &str) -> u32 {
    match name {
        "left" => 0x110,
        "right" => 0x111,
        "middle" => 0x112,
        n => n.parse().expect("button must be left, right, middle or an evdev code"),
    }
}

fn num<T: std::str::FromStr>(args: &[String], i: usize, what: &str) -> T {
    args.get(i)
        .and_then(|a| a.parse().ok())
        .unwrap_or_else(|| panic!("{what}: expected a number at argument {i}"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(!args.is_empty(), "usage: vptr <move DX DY|abs X Y|press B|release B|click B|scroll DY|sleep MS> ...");

    let conn = Connection::connect_to_env().expect("connect to wayland display");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());

    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();

    let manager = state
        .manager
        .clone()
        .expect("no zwlr_virtual_pointer_manager_v1 advertised");
    let ptr = manager.create_virtual_pointer(state.seat.as_ref(), &qh, ());
    queue.roundtrip(&mut state).unwrap();

    let start = std::time::Instant::now();
    let now = || start.elapsed().as_millis() as u32;
    let mut out = std::io::stdout();
    let mut i = 0;
    while i < args.len() {
        let step = args[i].as_str();
        match step {
            "move" => {
                let (dx, dy): (f64, f64) = (num(&args, i + 1, "move"), num(&args, i + 2, "move"));
                ptr.motion(now(), dx, dy);
                ptr.frame();
                writeln!(out, "move {dx} {dy}").ok();
                i += 3;
            }
            "abs" => {
                let (x, y): (u32, u32) = (num(&args, i + 1, "abs"), num(&args, i + 2, "abs"));
                ptr.motion_absolute(now(), x, y, 1280, 720);
                ptr.frame();
                writeln!(out, "abs {x} {y}").ok();
                i += 3;
            }
            "press" | "release" | "click" => {
                let name = args.get(i + 1).expect("button name");
                let code = button_code(name);
                if step != "release" {
                    ptr.button(now(), code, wl_pointer::ButtonState::Pressed);
                    ptr.frame();
                }
                if step != "press" {
                    ptr.button(now(), code, wl_pointer::ButtonState::Released);
                    ptr.frame();
                }
                writeln!(out, "{step} {name}").ok();
                i += 2;
            }
            "scroll" => {
                let dy: f64 = num(&args, i + 1, "scroll");
                ptr.axis_source(wl_pointer::AxisSource::Wheel);
                ptr.axis_discrete(now(), wl_pointer::Axis::VerticalScroll, dy, (dy / 15.0).round() as i32);
                ptr.frame();
                writeln!(out, "scroll {dy}").ok();
                i += 2;
            }
            "sleep" => {
                let ms: u64 = num(&args, i + 1, "sleep");
                conn.flush().ok();
                std::thread::sleep(std::time::Duration::from_millis(ms));
                i += 2;
            }
            other => panic!("unknown step {other:?}"),
        }
        conn.flush().ok();
    }
    queue.roundtrip(&mut state).unwrap();
    writeln!(out, "done").ok();

    ptr.destroy();
    queue.roundtrip(&mut state).unwrap();
}
