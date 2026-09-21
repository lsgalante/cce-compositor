// vkey — inject key events through zwp_virtual_keyboard_v1 (wtype-style),
// for exercising the compositor's keyboard stack in cce-shadow sessions.
//
// Usage: vkey <keycode|mod:MASK> ...
//   keycode   evdev code, pressed then released (h=35 e=18 l=38 o=24, 1=Escape)
//   mod:MASK  set held modifiers for the keys that follow (xkb depressed mask
//             under the us keymap: shift=1 ctrl=4 alt=8 super=64); cleared on exit
//   hold      after the events, keep the virtual keyboard alive until killed.
//             A headless shadow seat has no keyboard at all, so a client that
//             gains focus there gets wl_keyboard.modifiers with no keymap
//             before it — Chromium/Electron crash in xkb_state_update_mask on
//             that. Holding one keyboard gives every later client a keymap.

use std::io::Write;
use std::os::fd::{BorrowedFd, FromRawFd, OwnedFd};
use wayland_client::{
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
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
                "wl_seat" => {
                    state.seat =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.manager = Some(
                        registry
                            .bind::<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, _, _>(
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

impl Dispatch<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, ()> for State {
    fn event(
        _: &mut Self, _: &zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
        _: zwp_virtual_keyboard_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1, ()> for State {
    fn event(
        _: &mut Self, _: &zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
        _: zwp_virtual_keyboard_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
    }
}

fn keymap_fd() -> (OwnedFd, u32) {
    let ctx = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
    let keymap = xkbcommon::xkb::Keymap::new_from_names(
        &ctx, "", "", "us", "", None, xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .expect("compile keymap");
    let string = keymap.get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1);

    let fd = unsafe { libc::memfd_create(b"vkey-keymap\0".as_ptr() as *const _, 0) };
    assert!(fd >= 0, "memfd_create failed");
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(string.as_bytes()).unwrap();
    file.write_all(b"\0").unwrap();
    let size = string.len() as u32 + 1;
    (OwnedFd::from(file), size)
}

fn main() {
    // args: keycodes, or "mod:<depressed-mask>" to change held modifiers
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(!args.is_empty(), "usage: vkey <keycode|mod:MASK> ...");

    let conn = Connection::connect_to_env().expect("connect to wayland display");
    let display = conn.display();
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = display.get_registry(&qh, ());

    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();

    let seat = state.seat.clone().expect("no wl_seat advertised");
    let manager = state
        .manager
        .clone()
        .expect("no zwp_virtual_keyboard_manager_v1 advertised");

    let vk = manager.create_virtual_keyboard(&seat, &qh, ());

    let (fd, size) = keymap_fd();
    vk.keymap(1 /* XKB_V1 */, unsafe { BorrowedFd::borrow_raw(std::os::fd::AsRawFd::as_raw_fd(&fd)) }, size);
    queue.roundtrip(&mut state).unwrap();

    let mut t = 0u32;
    let hold = args.iter().any(|a| a == "hold");
    for arg in args.iter().filter(|a| *a != "hold") {
        if let Some(mask) = arg.strip_prefix("mod:") {
            let depressed: u32 = mask.parse().expect("mod mask must be a number");
            vk.modifiers(depressed, 0, 0, 0);
        } else {
            let keycode: u32 = arg.parse().expect("keycode must be a number");
            vk.key(t, keycode, 1); // pressed
            vk.key(t + 5, keycode, 0); // released
            t += 10;
        }
    }
    vk.modifiers(0, 0, 0, 0);
    queue.roundtrip(&mut state).unwrap();

    if hold {
        println!("holding virtual keyboard");
        std::io::stdout().flush().ok();
        loop {
            queue.blocking_dispatch(&mut state).unwrap();
        }
    }

    vk.destroy();
    queue.roundtrip(&mut state).unwrap();
    println!("sent {} event(s)", args.len());
}
