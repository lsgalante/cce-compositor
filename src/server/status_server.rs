// Status socket server for monolithic cce server
//
// Runs in a dedicated thread. cce-status connects to
// /tmp/cce-status-{WAYLAND_DISPLAY}.sock, sends a subscription line
// ("layout", "title", "modifiers", "adjust", or "dismiss") and receives lines whenever the status changes.
//
// The main loop sends updates through an mpsc channel. The server thread
// owns the socket and handles all I/O independently of the Wayland event loop.

use std::io::{BufRead, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::sync::Arc;

use crate::ipc_server::{drain_wake_fd, new_wake_fd, wake_fd};

/// A status update sent from the main loop to the server thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusUpdate {
    /// Plain text for layout module subscribers
    pub layout_text: String,
    /// Plain text for title module subscribers
    pub title_text: String,
    /// Plain text for modifiers subscriber
    pub modifiers_text: String,
    /// "on" while window-adjust mode is active (overview, or Super held —
    /// `WindowManager::window_adjust_active`), else "off". The desktop grid
    /// subscribes to show its own resize handles on the pinned images in
    /// step with the windows' handles; it never holds keyboard focus, so
    /// it cannot read the modifier state for itself.
    pub adjust_text: String,
    /// What each status segment is composited OVER, by app_id — see
    /// [`crate::backdrop`]. Unlike the other topics this one is
    /// per-subscriber: a segment gets only its own entry, since the whole
    /// point is that the far ends of a bar sit over different things.
    /// Quantized to whole percent, which is what keeps this struct `Eq` and
    /// therefore keeps `update_status`'s resend gate working while the
    /// camera pans.
    pub backdrops: Vec<(String, u8, u8)>,
}

/// A message from the main loop to the server thread: either a new state
/// snapshot for the state topics, or a one-shot menu-dismiss event.
#[derive(Debug, Clone)]
pub enum StatusMsg {
    State(StatusUpdate),
    /// Click-away-close for in-surface status menus: every `dismiss`
    /// subscriber EXCEPT the segment whose app_id is carried here should
    /// close its open menu (the exempt segment saw the press itself).
    MenuDismiss { except_app_id: String },
}

/// Subscription types that the status bar script can request.
///
/// Not `Copy`: `Backdrop` names the segment doing the asking, because it is
/// the one topic whose value differs per subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Subscription {
    Layout,
    Title,
    Modifiers,
    /// `adjust` — "on"/"off" as window-adjust mode comes and goes.
    Adjust,
    /// One-shot menu-dismiss events only — never receives state pushes.
    Dismiss,
    /// `backdrop <app_id>` — what THIS segment is composited over, so it can
    /// adapt its own text contrast. Lines are `<luma> <spread>`, both 0-100.
    Backdrop(String),
    Unknown,
}

impl Subscription {
    fn from_str(s: &str) -> Self {
        let s = s.trim();
        // The one topic that takes an argument. A bare `backdrop` is
        // accepted and simply never matches a segment, which reads as a
        // permanently unknown backdrop rather than as an error.
        if let Some(app_id) = s.strip_prefix("backdrop") {
            return Subscription::Backdrop(app_id.trim().to_string());
        }
        match s {
            "layout" => Subscription::Layout,
            "title" => Subscription::Title,
            "modifiers" => Subscription::Modifiers,
            "adjust" => Subscription::Adjust,
            "dismiss" => Subscription::Dismiss,
            _ => Subscription::Unknown,
        }
    }
}

/// A connected client with a known subscription.
struct Client {
    subscription: Subscription,
    stream: UnixStream,
    /// The last line actually written to this client. A state push only
    /// re-sends a topic whose formatted line CHANGED — a StatusUpdate is
    /// one struct, so e.g. a camera animation (viewport text embeds pan/
    /// zoom) used to re-broadcast identical layout/title lines at frame
    /// rate, and every subscriber rebuilt its segment per frame.
    last_line: Option<String>,
}

/// Handle to the status server for sending updates from the main loop.
///
/// Every send bumps `wake`, the eventfd the server thread `poll()`s on
/// alongside its sockets. The thread used to spin on `try_recv` with a 20 ms
/// sleep — 50 wakeups/s forever, subscribers or not; now it blocks until a
/// socket or the main loop has something for it.
#[derive(Debug, Clone)]
pub struct StatusSender {
    tx: mpsc::Sender<StatusMsg>,
    wake: Arc<OwnedFd>,
}

impl StatusSender {
    pub fn send(&self, update: StatusUpdate) {
        // If the channel is full or the receiver is gone, just drop it.
        if self.tx.send(StatusMsg::State(update)).is_ok() {
            wake_fd(&self.wake);
        }
    }

    /// Fire a one-shot menu-dismiss at every `dismiss` subscriber except the
    /// segment with this app_id (pass "-" to exempt nobody).
    pub fn send_menu_dismiss(&self, except_app_id: &str) {
        if self.tx.send(StatusMsg::MenuDismiss { except_app_id: except_app_id.to_string() }).is_ok() {
            wake_fd(&self.wake);
        }
    }
}

impl Drop for StatusSender {
    /// Dropping the last handle disconnects the channel; the thread only
    /// notices when it next wakes, so give it one.
    fn drop(&mut self) {
        wake_fd(&self.wake);
    }
}

pub fn get_status_socket_path(display_socket: Option<&str>) -> String {
    if let Some(display) = display_socket {
        format!("/tmp/cce-status-interface-{}.sock", display)
    } else {
        "/tmp/cce-status-interface.sock".to_string()
    }
}

/// Spawn the status server thread. Returns a StatusSender for the main loop.
pub fn spawn_status_server(display_socket: Option<String>) -> StatusSender {
    let (tx, rx) = mpsc::channel::<StatusMsg>();
    let wake = new_wake_fd().expect("failed to create status wake eventfd");
    let thread_wake = wake.clone();

    std::thread::Builder::new()
        .name("cce-status-server".into())
        .spawn(move || {
            status_server_main(rx, thread_wake, display_socket);
        })
        .expect("failed to spawn status server thread");

    StatusSender { tx, wake }
}

/// Block until the wake eventfd, the listener, or any subscriber socket is
/// readable. Returns `(wake, accept, per-client readiness)`; a client is
/// "ready" on data, hangup or error alike, since all three are handled by
/// reading it.
fn wait_for_activity(wake: &OwnedFd, listener: &UnixListener, clients: &[Client]) -> Option<(bool, bool, Vec<bool>)> {
    let mut fds: Vec<libc::pollfd> = Vec::with_capacity(2 + clients.len());
    for fd in [wake.as_raw_fd(), listener.as_raw_fd()] {
        fds.push(libc::pollfd { fd, events: libc::POLLIN, revents: 0 });
    }
    for client in clients {
        fds.push(libc::pollfd { fd: client.stream.as_raw_fd(), events: libc::POLLIN, revents: 0 });
    }
    let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
    if n < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::Interrupted {
            return Some((false, false, vec![false; clients.len()]));
        }
        log::error!("[status] poll failed: {}", err);
        return None;
    }
    let ready = |f: &libc::pollfd| f.revents != 0;
    Some((ready(&fds[0]), ready(&fds[1]), fds[2..].iter().map(ready).collect()))
}

fn status_server_main(rx: mpsc::Receiver<StatusMsg>, wake: Arc<OwnedFd>, display_socket: Option<String>) {
    let socket_path = get_status_socket_path(display_socket.as_deref());
    // Remove stale socket
    let _ = std::fs::remove_file(&socket_path);

    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("[status] failed to bind {}: {}", socket_path, e);
            return;
        }
    };

    // Set non-blocking so accept() doesn't hang the thread
    if let Err(e) = listener.set_nonblocking(true) {
        log::error!("[status] failed to set non-blocking: {}", e);
        return;
    }

    log::info!("[status] listening on {}", socket_path);

    let mut clients: Vec<Client> = Vec::new();
    let mut latest: Option<StatusUpdate> = None;

    loop {
        let Some((wake_ready, accept_ready, client_ready)) = wait_for_activity(&wake, &listener, &clients) else {
            // poll() itself failing is not something a retry fixes fast;
            // back off so the error line cannot flood the log.
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        };
        if wake_ready {
            drain_wake_fd(wake.as_raw_fd());
        }

        let mut has_new_update = false;

        // Accept new connections (the listener is non-blocking)
        for _ in 0..5 {
            if !accept_ready {
                break;
            }
            match listener.accept() {
                Ok((stream, _addr)) => {
                    // Read the subscription line while the socket is still
                    // blocking (bounded by a read timeout): poll() hands us
                    // the connection the instant it lands, which can be
                    // before the client's first line is in the buffer.
                    let sub = read_subscription(&stream);
                    if let Err(e) = stream.set_nonblocking(true) {
                        log::error!("[status] failed to set non-blocking on client: {}", e);
                        continue;
                    }
                    if sub != Subscription::Unknown {
                        log::info!("[status] new subscriber for {:?}", sub);
                        let client = Client {
                            subscription: sub,
                            stream,
                            last_line: None,
                        };
                        clients.push(client);
                        has_new_update = true; // push the latest status to the new client
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(e) => {
                    // EMFILE and friends: the socket stays readable, so
                    // without a pause this loop (and its log line) spins the
                    // thread at 100% — observed as a 167GB log once dead
                    // subscribers had exhausted the fd table.
                    log::error!("[status] accept error: {}", e);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    break;
                }
            }
        }

        // Reap dead subscribers by reading: a subscriber never sends after
        // its subscription line, so a successful zero-byte read is EOF (the
        // client vanished). Waiting for a WRITE to fail leaked them instead
        // — last_line dedup means a quiet topic may never write again, and
        // every bar restart stranded its whole subscriber set. Enough
        // restarts exhausted the fd table and took the session down.
        {
            let mut buf = [0u8; 64];
            let mut dead_clients = Vec::new();
            for (i, client) in clients.iter_mut().enumerate() {
                // Only sockets poll() flagged; the rest are quiet, not dead.
                if !client_ready.get(i).copied().unwrap_or(false) {
                    continue;
                }
                loop {
                    use std::io::Read;
                    match client.stream.read(&mut buf) {
                        Ok(0) => {
                            log::info!(
                                "[status] client {:?} disconnected (eof)",
                                client.subscription
                            );
                            dead_clients.push(i);
                            break;
                        }
                        // Unexpected chatter: drain and keep the client.
                        Ok(_) => continue,
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => {
                            dead_clients.push(i);
                            break;
                        }
                    }
                }
            }
            for i in dead_clients.into_iter().rev() {
                clients.remove(i);
            }
        }

        // Process incoming updates from the main loop
        let mut dismiss_events: Vec<String> = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(StatusMsg::State(update)) => {
                    latest = Some(update);
                    has_new_update = true;
                }
                Ok(StatusMsg::MenuDismiss { except_app_id }) => {
                    dismiss_events.push(except_app_id);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    log::info!("[status] channel disconnected, exiting");
                    let _ = std::fs::remove_file(&socket_path);
                    return;
                }
            }
        }

        // One-shot dismiss lines go only to `dismiss` subscribers; the line
        // payload is the exempt app_id.
        if !dismiss_events.is_empty() {
            let mut dead_clients = Vec::new();
            for (i, client) in clients.iter_mut().enumerate() {
                if client.subscription != Subscription::Dismiss {
                    continue;
                }
                for except in &dismiss_events {
                    match client
                        .stream
                        .write_all(except.as_bytes())
                        .and_then(|_| client.stream.write_all(b"\n"))
                    {
                        Ok(_) => {}
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(_) => {
                            dead_clients.push(i);
                            break;
                        }
                    }
                }
            }
            dead_clients.dedup();
            for i in dead_clients.into_iter().rev() {
                clients.remove(i);
            }
        }

        // If we got a new update, push it to all clients
        if has_new_update {
            if let Some(ref update) = latest {
                let mut dead_clients = Vec::new();

                for (i, client) in clients.iter_mut().enumerate() {
                    // Dismiss subscribers get one-shot events only, never
                    // state pushes.
                    if client.subscription == Subscription::Dismiss {
                        continue;
                    }
                    let msg = format_for_subscription(&client.subscription, update);
                    // Only lines that changed for THIS topic go out (see
                    // Client::last_line); a fresh client always gets one.
                    if client.last_line.as_deref() == Some(msg.as_str()) {
                        continue;
                    }
                    match client
                        .stream
                        .write_all(msg.as_bytes())
                        .and_then(|_| client.stream.write_all(b"\n"))
                    {
                        Ok(_) => {
                            client.last_line = Some(msg);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // Client not ready to receive — skip for now
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                            log::info!(
                                "[status] client {:?} disconnected (broken pipe)",
                                client.subscription
                            );
                            dead_clients.push(i);
                        }
                        Err(e) => {
                            log::error!(
                                "[status] write error to client {:?}: {}",
                                client.subscription, e
                            );
                            dead_clients.push(i);
                        }
                    }
                }

                // Remove dead clients (iterate in reverse to preserve indices)
                for i in dead_clients.into_iter().rev() {
                    clients.remove(i);
                }
            }
        }
    }
}

fn read_subscription(stream: &UnixStream) -> Subscription {
    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();
    // Bounded blocking read: a subscriber writes its one line right after
    // connecting, so this returns at once in practice; the timeout is for a
    // client that connects and says nothing.
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(200)))
        .ok();
    match reader.read_line(&mut line) {
        Ok(_) => Subscription::from_str(&line),
        Err(e) => {
            log::error!("[status] failed to read subscription: {}", e);
            Subscription::Unknown
        }
    }
}

fn format_for_subscription(sub: &Subscription, update: &StatusUpdate) -> String {
    match sub {
        Subscription::Layout => update.layout_text.clone(),
        Subscription::Title => update.title_text.clone(),
        Subscription::Modifiers => update.modifiers_text.clone(),
        Subscription::Adjust => update.adjust_text.clone(),
        Subscription::Backdrop(app_id) => {
            // A segment the compositor has no sample for (not mapped yet, or
            // its app_id does not match a window) is told so explicitly
            // rather than left to time out: "unknown" is a state the bar
            // renders for, not an absence.
            match update.backdrops.iter().find(|(id, _, _)| id == app_id) {
                Some((_, luma, spread)) => format!("{} {}", luma, spread),
                None => "unknown".to_string(),
            }
        }
        Subscription::Dismiss | Subscription::Unknown => String::new(),
    }
}

pub unsafe fn build_status_update(wm: &crate::window_manager::WindowManager) -> StatusUpdate {
    // `focused_window()` falls back to the most recent real window so the
    // bar doesn't flash while overlay UI (the launcher) briefly holds
    // focus. But an explicit Focus::None (desktop click) is a real,
    // user-visible state — keystrokes go nowhere — and the status feed
    // must report it honestly instead of showing the last window as if it
    // still had focus.
    let seat_focus_is_none = wm
        .first_seat()
        .map(|s| matches!((*s).focused, crate::seat::Focus::None))
        .unwrap_or(false);
    let focused_window = if seat_focus_is_none {
        std::ptr::null_mut()
    } else {
        wm.focused_window()
    };

    let layout_text = if !focused_window.is_null() {
        (*focused_window).tiling_mode.as_str().to_string()
    } else {
        let focused_layer = wm.focused_layer_surface();
        let mut is_cce_cloud = false;
        if !focused_layer.is_null() {
            let wlr_layer_surface = crate::ffi::wlr_layer_surface_v1_try_from_wlr_surface(focused_layer);
            if !wlr_layer_surface.is_null() && !(*wlr_layer_surface).namespace.is_null() {
                let ns = std::ffi::CStr::from_ptr((*wlr_layer_surface).namespace).to_string_lossy();
                if ns.starts_with("cce-cloud") {
                    is_cce_cloud = true;
                }
            }
        }
        if is_cce_cloud {
            "Overlay".to_string()
        } else {
            "---".to_string()
        }
    };

    let title_text = if !focused_window.is_null() {
        let title_ptr = (*focused_window).get_title();
        if !title_ptr.is_null() {
            std::ffi::CStr::from_ptr(title_ptr).to_string_lossy().into_owned()
        } else {
            "(none)".to_string()
        }
    } else {
        let focused_layer = wm.focused_layer_surface();
        if !focused_layer.is_null() {
            let wlr_layer_surface = crate::ffi::wlr_layer_surface_v1_try_from_wlr_surface(focused_layer);
            if !wlr_layer_surface.is_null() && !(*wlr_layer_surface).namespace.is_null() {
                std::ffi::CStr::from_ptr((*wlr_layer_surface).namespace).to_string_lossy().into_owned()
            } else {
                "(none)".to_string()
            }
        } else {
            "(none)".to_string()
        }
    };

    let seat_ptr = wm.first_seat().unwrap_or(std::ptr::null_mut());
    let mut super_pressed = false;
    if !seat_ptr.is_null() {
        let wlr_keyboard = crate::ffi::river_wlr_seat_get_keyboard((*seat_ptr).wlr_seat);
        if !wlr_keyboard.is_null() {
            let modifiers = crate::ffi::wlr_keyboard_get_modifiers(wlr_keyboard);
            super_pressed = modifiers & 0x40 != 0;
        }
    }
    let modifiers_text = if super_pressed { "super" } else { "none" }.to_string();

    StatusUpdate {
        layout_text,
        title_text,
        modifiers_text,
        adjust_text: if wm.window_adjust_active() { "on" } else { "off" }.to_string(),
        // Measured in the render pass (see `Output::measure_status_backdrops`)
        // because that is where the frame's grid geometry already lives;
        // here it is only carried.
        backdrops: wm.status_backdrops.borrow().clone(),
    }
}
