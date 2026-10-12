// or-focus — an override-redirect window that takes the keyboard and is
// destroyed, then a second one that maps.
//
// Two override-redirect windows with no _NET_WM_WINDOW_TYPE and no WM_HINTS
// (the shape of a Wine menu or tooltip, which wlroots says wants focus).
// With nothing focused, A takes the seat's keyboard when it maps; the client
// then destroys A and maps B. Until 2026-10-12 the compositor kept A as the
// seat's focus after freeing it, and B's map read the freed record (a crash,
// or B silently never getting the keyboard when the allocator reused A's
// address for B).
//
// Prints one line per step, and whether B saw FocusIn — informational only:
// Xwayland sends none for an override-redirect window the compositor gives
// the keyboard without activating it, so read the compositor log
// ("Seat::focus set to OverrideRedirect", once per window) for the grant.
// Args: --hold SECS (default 2).

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

fn main() {
    let mut hold = 2.0f64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--hold" => hold = args.next().unwrap().parse().unwrap(),
            other => panic!("unknown arg {other}"),
        }
    }
    let (conn, screen_num) = x11rb::connect(None).expect("X connection");
    let screen = conn.setup().roots[screen_num].clone();
    let make = |name: &[u8], x: i16| {
        let win = conn.generate_id().unwrap();
        conn.create_window(
            screen.root_depth,
            win,
            screen.root,
            x,
            60,
            140,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                .background_pixel(screen.white_pixel)
                .override_redirect(1)
                .event_mask(EventMask::FOCUS_CHANGE | EventMask::STRUCTURE_NOTIFY),
        )
        .unwrap();
        conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, name).unwrap();
        win
    };
    let a = make(b"or-focus-a", 60);
    let b = make(b"or-focus-b", 260);
    conn.flush().unwrap();
    let pause = |s: f64| std::thread::sleep(Duration::from_secs_f64(s));

    conn.map_window(a).unwrap();
    conn.flush().unwrap();
    println!("mapped a");
    pause(0.6);
    conn.destroy_window(a).unwrap();
    conn.flush().unwrap();
    println!("destroyed a");
    pause(0.4);
    conn.map_window(b).unwrap();
    conn.flush().unwrap();
    println!("mapped b");

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut b_focused = false;
    while Instant::now() < deadline {
        match conn.poll_for_event() {
            Ok(Some(Event::FocusIn(e))) if e.event == b => {
                b_focused = true;
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => pause(0.02),
            Err(_) => break,
        }
    }
    println!("b focus-in: {}", if b_focused { "yes" } else { "no" });
    pause(hold);
    println!("done");
}
