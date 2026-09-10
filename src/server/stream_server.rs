// Window-stream socket server.
//
// The compositor-native answer to remote window viewing (cce-remote):
// a client connects to /tmp/cce-stream-{WAYLAND_DISPLAY}.sock, sends one
// subscription line — `window <query>` where query is an id, app_id, or
// the literal `focused` — and then receives raw frames of that window:
//
//     frame <width> <height> <len>\n
//     <len bytes of tightly packed RGBA>
//
// Frames are DAMAGE-DRIVEN at the source: `Window::stream_dirty` is set by
// the window's commit listener, and the window manager's stream timer (a
// wlroots event-loop timer, ~30 fps while subscribers exist, idle cadence
// otherwise) captures only dirty windows via the screenshot readback path —
// so an idle window costs nothing, occluded/off-viewport windows stream
// fine, and `focused` re-resolves every tick so the stream follows focus.
//
// Threading mirrors status_server: an accept thread owns the listener; each
// subscriber gets a writer thread fed through a BOUNDED channel — the main
// thread only try_send()s, so a stalled client skips frames (backpressure =
// frame dropping) and can never block the compositor.

use std::io::{BufRead, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// One captured frame, shared across all subscribers of the same window.
pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub rgba: Vec<u8>,
}

/// A subscriber as the MAIN THREAD sees it: the query to resolve each tick
/// and the bounded sender feeding its writer thread.
pub struct Sub {
    pub query: String,
    pub tx: mpsc::SyncSender<Arc<Frame>>,
    /// Force a frame regardless of damage (first frame after subscribing,
    /// and the periodic keepalive that lets writers detect dead clients).
    pub needs_frame: bool,
    pub last_sent: Instant,
}

/// Shared subscriber registry: the accept thread pushes, the window
/// manager's stream timer drains dead entries and feeds frames.
#[derive(Clone)]
pub struct StreamHub {
    pub subs: Arc<Mutex<Vec<Sub>>>,
    /// Bumped by the accept thread after pushing a subscriber; the window
    /// manager has it as an event source and arms its frame tick from it.
    pub wake: Arc<std::os::fd::OwnedFd>,
}

pub fn get_stream_socket_path(display_socket: Option<&str>) -> String {
    match display_socket {
        Some(display) => format!("/tmp/cce-stream-{}.sock", display),
        None => "/tmp/cce-stream.sock".to_string(),
    }
}

/// Spawn the accept thread; returns the hub for the main loop's timer.
pub fn spawn_stream_server(display_socket: Option<String>) -> StreamHub {
    let hub = StreamHub {
        subs: Arc::new(Mutex::new(Vec::new())),
        wake: crate::ipc_server::new_wake_fd().expect("failed to create stream wake eventfd"),
    };
    let accept_hub = hub.clone();
    std::thread::Builder::new()
        .name("cce-stream-server".into())
        .spawn(move || accept_loop(accept_hub, display_socket))
        .expect("failed to spawn stream server thread");
    hub
}

fn accept_loop(hub: StreamHub, display_socket: Option<String>) {
    let socket_path = get_stream_socket_path(display_socket.as_deref());
    let _ = std::fs::remove_file(&socket_path);
    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("[stream] failed to bind {}: {}", socket_path, e);
            return;
        }
    };
    log::info!("[stream] listening on {}", socket_path);

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        // Subscription line, with a timeout so a silent connect can't park.
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let mut line = String::new();
        {
            let mut reader = std::io::BufReader::new(&stream);
            if reader.read_line(&mut line).is_err() {
                continue;
            }
        }
        let Some(query) = line.trim().strip_prefix("window ").map(str::trim) else {
            continue;
        };
        if query.is_empty() || query.len() > 128 {
            continue;
        }
        let _ = stream.set_read_timeout(None);

        // Bounded at 2: the main thread never blocks; a slow client just
        // gets the freshest frame that fits.
        let (tx, rx) = mpsc::sync_channel::<Arc<Frame>>(2);
        let query = query.to_string();
        log::info!("[stream] subscriber for window '{}'", query);
        std::thread::Builder::new()
            .name("cce-stream-writer".into())
            .spawn(move || writer_loop(stream, rx))
            .ok();
        if let Ok(mut subs) = hub.subs.lock() {
            subs.push(Sub { query, tx, needs_frame: true, last_sent: Instant::now() });
        }
        crate::ipc_server::wake_fd(&hub.wake);
    }
}

/// Blocking writes on a dedicated thread per subscriber. Exits on write
/// error; the dropped receiver surfaces as Disconnected on the main
/// thread's next try_send, which prunes the Sub.
fn writer_loop(mut stream: UnixStream, rx: mpsc::Receiver<Arc<Frame>>) {
    while let Ok(frame) = rx.recv() {
        let header = format!("frame {} {} {}\n", frame.width, frame.height, frame.rgba.len());
        if stream.write_all(header.as_bytes()).is_err()
            || stream.write_all(&frame.rgba).is_err()
        {
            return;
        }
    }
}
