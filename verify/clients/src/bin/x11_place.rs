// x11-place — X11 windows that place themselves before they map.
//
// `--mode transient`: a managed parent window, then a child with
// WM_TRANSIENT_FOR on it whose WM_NORMAL_HINTS carry USPosition and which
// is configured to --at before it maps — Qt's `move()` before `show()`.
// The compositor must map it where it asked (`place_transient_where_it_asked`).
//
// `--mode shy`: a top-level that declines input (WM_HINTS input = False)
// and skips the taskbar, created at --at — Wine's WS_EX_NOACTIVATE |
// WS_EX_TOOLWINDOW helper. The compositor must map it where it is
// (`place_shy_where_it_is`) and never focus it.
//
// `--mode or`: an override-redirect window at --at, painted solid. With
// `--class cce-xembed-tray` it is one of the tray bridge's containers, which
// the compositor must never show (`is_xembed_tray_container`).
//
// --at is X11 root pixels (physical under xwayland_hidpi); --class sets
// WM_CLASS (both halves) of the window under test, for the
// `xwayland_hidpi_except` match. Prints one line per step.
// `--mode plain`: an ordinary top-level, for comparison.
//
// Args: --mode transient|shy|or|plain  --at X,Y  --size WxH  --class C  --hold SECS

use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::properties::{WmHints, WmSizeHints, WmSizeHintsSpecification};
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;

fn say(line: &str) {
    println!("{line}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

fn main() {
    let mut mode = String::from("transient");
    let mut at = (300i16, 200i16);
    let mut size = (240u16, 160u16);
    let mut hold = 6.0f64;
    let mut class: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--mode" => mode = args.next().unwrap(),
            "--at" => {
                let s = args.next().unwrap();
                let (x, y) = s.split_once(',').expect("at is X,Y");
                at = (x.parse().unwrap(), y.parse().unwrap());
            }
            "--size" => {
                let s = args.next().unwrap();
                let (w, h) = s.split_once('x').expect("size is WxH");
                size = (w.parse().unwrap(), h.parse().unwrap());
            }
            "--hold" => hold = args.next().unwrap().parse().unwrap(),
            "--class" => class = Some(args.next().unwrap()),
            other => panic!("unknown arg {other}"),
        }
    }
    let (conn, screen_num) = x11rb::connect(None).expect("X connection");
    let screen = conn.setup().roots[screen_num].clone();
    let create_with = |x: i16, y: i16, w: u16, h: u16, pixel: u32, override_redirect: bool| -> Window {
        let win = conn.generate_id().unwrap();
        conn.create_window(
            screen.root_depth,
            win,
            screen.root,
            x,
            y,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .background_pixel(pixel)
                .override_redirect(override_redirect as u32)
                .event_mask(EventMask::STRUCTURE_NOTIFY),
        )
        .unwrap();
        win
    };
    let create = |x: i16, y: i16, w: u16, h: u16, pixel: u32| create_with(x, y, w, h, pixel, false);
    // Give the XWM time to see the window's CreateNotify and select its
    // property changes: a property set before that is one wlroots never
    // hears of (the window then mapped as an ordinary one, ~1 run in 5).
    let settle = || {
        conn.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    };
    let name = |win: Window, title: &str, class: &str| {
        conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, title.as_bytes()).unwrap();
        let wm_class = format!("{class}\0{class}\0");
        conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_CLASS, AtomEnum::STRING, wm_class.as_bytes()).unwrap();
    };
    let wait_map = |win: Window| {
        loop {
            match conn.wait_for_event().unwrap() {
                x11rb::protocol::Event::MapNotify(e) if e.window == win => return,
                _ => {}
            }
        }
    };

    match mode.as_str() {
        "transient" => {
            let parent = create(0, 0, 600, 400, 0x00305070);
            name(parent, "x11-place parent", "x11-place-parent");
            conn.map_window(parent).unwrap();
            conn.flush().unwrap();
            wait_map(parent);
            say("parent mapped");
            std::thread::sleep(Duration::from_millis(1500));

            let child = create(0, 0, size.0, size.1, 0x00a06030);
            settle();
            name(child, "x11-place transient", class.as_deref().unwrap_or("x11-place-transient"));
            conn.change_property32(PropMode::REPLACE, child, AtomEnum::WM_TRANSIENT_FOR, AtomEnum::WINDOW, &[parent])
                .unwrap();
            let hints = WmSizeHints {
                position: Some((WmSizeHintsSpecification::UserSpecified, at.0 as i32, at.1 as i32)),
                ..WmSizeHints::default()
            };
            hints.set_normal_hints(&conn, child).unwrap().check().unwrap();
            // Moved before it is shown: the request the compositor grants
            // while the window is still unmapped.
            conn.configure_window(child, &ConfigureWindowAux::new().x(at.0 as i32).y(at.1 as i32)).unwrap();
            conn.flush().unwrap();
            std::thread::sleep(Duration::from_millis(300));
            conn.map_window(child).unwrap();
            conn.flush().unwrap();
            wait_map(child);
            say("transient mapped");
        }
        "shy" => {
            let win = create(at.0, at.1, size.0, size.1, 0x00e0e0e0);
            settle();
            name(win, "", class.as_deref().unwrap_or("x11-place-shy"));
            let mut wm_hints = WmHints::new();
            wm_hints.input = Some(false);
            wm_hints.set(&conn, win).unwrap().check().unwrap();
            let state = conn.intern_atom(false, b"_NET_WM_STATE").unwrap().reply().unwrap().atom;
            let skip = conn.intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR").unwrap().reply().unwrap().atom;
            conn.change_property32(PropMode::REPLACE, win, state, AtomEnum::ATOM, &[skip]).unwrap();
            conn.map_window(win).unwrap();
            conn.flush().unwrap();
            wait_map(win);
            say("shy mapped");
        }
        "plain" => {
            let win = create(at.0, at.1, size.0, size.1, 0x00808080);
            settle();
            name(win, "x11-place plain", class.as_deref().unwrap_or("x11-place-plain"));
            conn.map_window(win).unwrap();
            conn.flush().unwrap();
            wait_map(win);
            say("plain mapped");
        }
        "or" => {
            let win = create_with(at.0, at.1, size.0, size.1, 0x0020c040, true);
            settle();
            name(win, "x11-place or", class.as_deref().unwrap_or("x11-place-or"));
            conn.map_window(win).unwrap();
            conn.flush().unwrap();
            wait_map(win);
            say("or mapped");
        }
        other => panic!("unknown mode {other}"),
    }
    std::thread::sleep(Duration::from_secs_f64(hold));
    say("exit");
}
