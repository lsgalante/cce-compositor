// seat-cycle — create and destroy seats mid-session through
// river_input_manager_v1, for exercising a seat's teardown (`Seat::destroy`
// from the manage pass) in cce-shadow sessions; nothing else reaches it
// before the compositor exits.
//
// Usage: seat-cycle <step> ...
//   create NAME      river_input_manager_v1.create_seat
//   destroy NAME     river_input_manager_v1.destroy_seat
//   bind NAME        bind the wl_seat global named NAME (waits for it)
//   kbd NAME         a zwp_virtual_keyboard_v1 on the bound seat NAME, with a
//                    us keymap; the compositor attaches it to that seat
//   key NAME CODE    press and release evdev CODE on NAME's keyboard
//   drop NAME        destroy NAME's virtual keyboard
//   sleep MS
// Each step ends with a roundtrip and prints one line; "seats: ..." lists
// the wl_seat globals by name at the end, then "done".

use std::collections::BTreeMap;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use wayland_client::{
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};

#[allow(non_upper_case_globals, non_camel_case_types, dead_code, clippy::all)]
mod river {
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::backend as wayland_backend;
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("../../protocol/river-input-management-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("../../protocol/river-input-management-v1.xml");
}
use river::{river_input_device_v1, river_input_manager_v1};

#[derive(Default)]
struct State {
    manager: Option<river_input_manager_v1::RiverInputManagerV1>,
    vkbd_manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
    // global name -> (seat, its name once announced)
    seats: BTreeMap<u32, (wl_seat::WlSeat, Option<String>)>,
    removed: Vec<u32>,
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
        match event {
            wl_registry::Event::Global { name, interface, version } => match interface.as_str() {
                "wl_seat" => {
                    let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, name);
                    state.seats.insert(name, (seat, None));
                }
                "river_input_manager_v1" => {
                    state.manager = Some(registry.bind(name, version.min(2), qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.vkbd_manager = Some(registry.bind(name, 1, qh, ()));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some((seat, _)) = state.seats.remove(&name) {
                    if seat.version() >= 5 {
                        seat.release();
                    }
                    state.removed.push(name);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, u32> for State {
    fn event(
        state: &mut Self, _: &wl_seat::WlSeat, event: wl_seat::Event, global: &u32, _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Name { name } = event {
            if let Some(entry) = state.seats.get_mut(global) {
                entry.1 = Some(name);
            }
        }
    }
}

impl Dispatch<river_input_manager_v1::RiverInputManagerV1, ()> for State {
    fn event(
        _: &mut Self, _: &river_input_manager_v1::RiverInputManagerV1,
        _: river_input_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(State, river_input_manager_v1::RiverInputManagerV1, [
        river_input_manager_v1::EVT_INPUT_DEVICE_OPCODE => (river_input_device_v1::RiverInputDeviceV1, ()),
    ]);
}

impl Dispatch<river_input_device_v1::RiverInputDeviceV1, ()> for State {
    fn event(
        _: &mut Self, _: &river_input_device_v1::RiverInputDeviceV1,
        _: river_input_device_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
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

    let fd = unsafe { libc::memfd_create(b"seat-cycle-keymap\0".as_ptr() as *const _, 0) };
    assert!(fd >= 0, "memfd_create failed");
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(string.as_bytes()).unwrap();
    file.write_all(b"\0").unwrap();
    let size = string.len() as u32 + 1;
    (OwnedFd::from(file), size)
}

fn seat_named<'a>(state: &'a State, name: &str) -> Option<&'a wl_seat::WlSeat> {
    state.seats.values().find(|(_, n)| n.as_deref() == Some(name)).map(|(s, _)| s)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(!args.is_empty(), "usage: seat-cycle <create N|destroy N|bind N|kbd N|key N CODE|drop N|sleep MS> ...");

    let conn = Connection::connect_to_env().expect("connect to wayland display");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());

    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();
    queue.roundtrip(&mut state).unwrap();
    let manager = state.manager.clone().expect("no river_input_manager_v1 advertised");

    let mut kbds: BTreeMap<String, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1> = BTreeMap::new();
    let start = std::time::Instant::now();
    let now = || start.elapsed().as_millis() as u32;
    let mut out = std::io::stdout();
    let mut i = 0;
    while i < args.len() {
        let step = args[i].as_str();
        let arg = |k: usize| args.get(i + k).cloned().unwrap_or_else(|| panic!("{step}: missing argument {k}"));
        match step {
            "create" => {
                manager.create_seat(arg(1));
                i += 2;
            }
            "destroy" => {
                manager.destroy_seat(arg(1));
                i += 2;
            }
            "bind" => {
                let name = arg(1);
                let mut found = false;
                for _ in 0..50 {
                    queue.roundtrip(&mut state).unwrap();
                    if seat_named(&state, &name).is_some() {
                        found = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                writeln!(out, "bind {name}: {}", if found { "ok" } else { "no such seat" }).ok();
                i += 2;
                continue;
            }
            "kbd" => {
                let name = arg(1);
                let seat = seat_named(&state, &name).unwrap_or_else(|| panic!("kbd: no seat {name}")).clone();
                let vm = state.vkbd_manager.clone().expect("no zwp_virtual_keyboard_manager_v1 advertised");
                let kbd = vm.create_virtual_keyboard(&seat, &qh, ());
                let (fd, size) = keymap_fd();
                kbd.keymap(1, fd.as_fd(), size);
                kbds.insert(name, kbd);
                i += 2;
            }
            "key" => {
                let name = arg(1);
                let code: u32 = arg(2).parse().expect("key: evdev code");
                let kbd = kbds.get(&name).unwrap_or_else(|| panic!("key: no keyboard on {name}"));
                kbd.key(now(), code, 1);
                kbd.key(now(), code, 0);
                i += 3;
            }
            "drop" => {
                let name = arg(1);
                if let Some(kbd) = kbds.remove(&name) {
                    kbd.destroy();
                }
                i += 2;
            }
            "sleep" => {
                let ms: u64 = arg(1).parse().expect("sleep: milliseconds");
                conn.flush().ok();
                std::thread::sleep(std::time::Duration::from_millis(ms));
                i += 2;
                continue;
            }
            other => panic!("unknown step {other:?}"),
        }
        let shown: Vec<&str> = args[i - if step == "key" { 3 } else { 2 }..i].iter().map(String::as_str).collect();
        match queue.roundtrip(&mut state) {
            Ok(_) => writeln!(out, "{}", shown.join(" ")).ok(),
            Err(e) => {
                writeln!(out, "{}: connection lost ({e})", shown.join(" ")).ok();
                return;
            }
        };
    }
    queue.roundtrip(&mut state).unwrap();
    let names: Vec<String> = state.seats.values().map(|(_, n)| n.clone().unwrap_or_else(|| "?".into())).collect();
    writeln!(out, "seats: {} (globals removed: {})", names.join(" "), state.removed.len()).ok();
    for (_, kbd) in kbds {
        kbd.destroy();
    }
    queue.roundtrip(&mut state).ok();
    writeln!(out, "done").ok();
}
