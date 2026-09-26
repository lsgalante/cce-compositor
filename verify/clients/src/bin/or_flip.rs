// or-flip — an X11 window that stops being override-redirect between maps.
//
// Maps a window with override_redirect set, unmaps it, clears the flag and
// maps it again, then keeps it up for a while. The second MapNotify carries
// the changed flag, so wlroots emits `set_override_redirect` and the
// compositor turns its override-redirect record into a managed window. An
// XEmbed tray handing an icon back to the root window makes Wine do exactly
// this, which is how the compositor crashed on 2026-09-26: the record was
// freed but left in `wm.override_redirects`, and the next frame read it.
//
// Prints one line per step. Args: --hold SECS (default 3), --cycles N
// (default 1; each cycle flips OR on and off again).

use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;

fn main() {
    let mut hold = 3.0f64;
    let mut cycles = 1u32;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--hold" => hold = args.next().unwrap().parse().unwrap(),
            "--cycles" => cycles = args.next().unwrap().parse().unwrap(),
            other => panic!("unknown arg {other}"),
        }
    }
    let (conn, screen_num) = x11rb::connect(None).expect("X connection");
    let screen = conn.setup().roots[screen_num].clone();
    let win = conn.generate_id().unwrap();
    conn.create_window(
        screen.root_depth,
        win,
        screen.root,
        50,
        50,
        120,
        90,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new().background_pixel(screen.white_pixel).override_redirect(1),
    )
    .unwrap();
    conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, b"or-flip").unwrap();
    let pause = |s: f64| std::thread::sleep(Duration::from_secs_f64(s));
    for cycle in 0..cycles {
        conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().override_redirect(1)).unwrap();
        conn.map_window(win).unwrap();
        conn.flush().unwrap();
        println!("cycle {cycle}: mapped override-redirect");
        pause(0.5);
        conn.unmap_window(win).unwrap();
        conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().override_redirect(0)).unwrap();
        conn.map_window(win).unwrap();
        conn.flush().unwrap();
        println!("cycle {cycle}: remapped managed");
        pause(0.5);
        conn.unmap_window(win).unwrap();
        conn.flush().unwrap();
        pause(0.2);
    }
    conn.map_window(win).unwrap();
    conn.flush().unwrap();
    pause(hold);
    println!("done");
}
