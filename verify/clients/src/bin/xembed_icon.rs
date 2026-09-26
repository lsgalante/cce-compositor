// xembed-icon — a legacy X11 tray icon, the way Wine's systray docks one.
//
// Waits for an owner of `_NET_SYSTEM_TRAY_S0`, creates a window in the
// visual the tray advertises (`_NET_SYSTEM_TRAY_VISUAL`, ARGB when there is
// one), sets `_XEMBED_INFO` mapped, and sends SYSTEM_TRAY_REQUEST_DOCK. It
// paints itself one solid colour on every Expose. One milestone per line:
//
//   tray owner 0xWIN
//   window 0xWIN depth D
//   docked into 0xPARENT          (ReparentNotify away from the root)
//   embedded                      (XEMBED_EMBEDDED_NOTIFY)
//   button N press|release x,y root rx,ry
//   recolored 0xAARRGGBB
//   undocked                      (reparented back to the root)
//
// Args: --color 0xAARRGGBB (premultiplied pixel, default opaque red)
//       --recolor SECS 0xAARRGGBB   repaint in a second colour later
//       --exit-after SECS           destroy the window and exit
//       --size N                    window size, default 32

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

fn parse_hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).expect("hex colour")
}

fn main() {
    let mut color = 0xffd0_2020u32;
    let mut recolor: Option<(f64, u32)> = None;
    let mut exit_after: Option<f64> = None;
    let mut size: u16 = 32;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--color" => color = parse_hex(&args.next().unwrap()),
            "--recolor" => {
                let secs = args.next().unwrap().parse().unwrap();
                recolor = Some((secs, parse_hex(&args.next().unwrap())));
            }
            "--exit-after" => exit_after = Some(args.next().unwrap().parse().unwrap()),
            "--size" => size = args.next().unwrap().parse().unwrap(),
            other => panic!("unknown arg {other}"),
        }
    }

    let (conn, screen_num) = x11rb::connect(None).expect("X connection");
    let screen = conn.setup().roots[screen_num].clone();
    let atom = |name: &str| conn.intern_atom(false, name.as_bytes()).unwrap().reply().unwrap().atom;
    let selection = atom(&format!("_NET_SYSTEM_TRAY_S{screen_num}"));
    let opcode = atom("_NET_SYSTEM_TRAY_OPCODE");
    let tray_visual = atom("_NET_SYSTEM_TRAY_VISUAL");
    let xembed = atom("_XEMBED");
    let xembed_info = atom("_XEMBED_INFO");

    let deadline = Instant::now() + Duration::from_secs(20);
    let owner = loop {
        let owner = conn.get_selection_owner(selection).unwrap().reply().unwrap().owner;
        if owner != NONE {
            break owner;
        }
        assert!(Instant::now() < deadline, "no tray appeared");
        std::thread::sleep(Duration::from_millis(200));
    };
    println!("tray owner {owner:#x}");

    // The tray's visual when it names one (and it is on this screen).
    let advertised = conn
        .get_property(false, owner, tray_visual, AtomEnum::VISUALID, 0, 1)
        .unwrap()
        .reply()
        .ok()
        .and_then(|r| r.value32().and_then(|mut v| v.next()));
    let (depth, visual) = advertised
        .and_then(|vid| {
            screen.allowed_depths.iter().find_map(|d| {
                d.visuals.iter().any(|v| v.visual_id == vid).then_some((d.depth, vid))
            })
        })
        .unwrap_or((screen.root_depth, screen.root_visual));
    let colormap = conn.generate_id().unwrap();
    conn.create_colormap(ColormapAlloc::NONE, colormap, screen.root, visual).unwrap();

    let win = conn.generate_id().unwrap();
    conn.create_window(
        depth,
        win,
        screen.root,
        0,
        0,
        size,
        size,
        0,
        WindowClass::INPUT_OUTPUT,
        visual,
        &CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(colormap)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::STRUCTURE_NOTIFY,
            ),
    )
    .unwrap();
    conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, b"xembed-icon test")
        .unwrap();
    conn.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"xembed-icon\0XembedIcon\0",
    )
    .unwrap();
    conn.change_property32(PropMode::REPLACE, win, xembed_info, xembed_info, &[0, 1]).unwrap();
    let gc = conn.generate_id().unwrap();
    conn.create_gc(gc, win, &CreateGCAux::new().foreground(color)).unwrap();
    println!("window {win:#x} depth {depth}");

    let dock = ClientMessageEvent::new(32, owner, opcode, [CURRENT_TIME, 0, win, 0, 0]);
    conn.send_event(false, owner, EventMask::NO_EVENT, dock).unwrap();
    conn.flush().unwrap();

    let start = Instant::now();
    let paint = |c: u32| {
        conn.change_gc(gc, &ChangeGCAux::new().foreground(c)).unwrap();
        conn.poly_fill_rectangle(win, gc, &[Rectangle { x: 0, y: 0, width: size, height: size }]).unwrap();
        conn.flush().unwrap();
    };
    loop {
        let elapsed = start.elapsed().as_secs_f64();
        if let Some((at, c)) = recolor {
            if elapsed >= at {
                color = c;
                paint(color);
                println!("recolored {color:#010x}");
                recolor = None;
            }
        }
        if exit_after.is_some_and(|at| elapsed >= at) {
            conn.destroy_window(win).unwrap();
            conn.flush().unwrap();
            println!("exiting");
            return;
        }
        while let Some(event) = conn.poll_for_event().unwrap() {
            match event {
                Event::Expose(e) if e.count == 0 => paint(color),
                Event::ReparentNotify(e) if e.window == win => {
                    if e.parent == screen.root {
                        println!("undocked");
                    } else {
                        println!("docked into {:#x}", e.parent);
                    }
                }
                Event::ClientMessage(e) if e.type_ == xembed => {
                    if e.data.as_data32()[1] == 0 {
                        println!("embedded");
                    }
                }
                Event::ButtonPress(e) => {
                    println!("button {} press {},{} root {},{}", e.detail, e.event_x, e.event_y, e.root_x, e.root_y)
                }
                Event::ButtonRelease(e) => {
                    println!("button {} release {},{} root {},{}", e.detail, e.event_x, e.event_y, e.root_x, e.root_y)
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
