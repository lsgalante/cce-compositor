// Status socket server for monolithic cce server
//
// Runs in a dedicated thread. cce-status connects to
// /tmp/cce-status-{WAYLAND_DISPLAY}.sock, sends a subscription line
// ("viewport", "layout", or "title"), and receives JSON lines whenever the status changes.
//
// The main loop sends updates through an mpsc channel. The server thread
// owns the socket and handles all I/O independently of the Wayland event loop.

use std::io::{BufRead, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;

/// A status update sent from the main loop to the server thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusUpdate {
    /// JSON string for viewport module subscribers
    pub viewport_json: String,
    /// Plain text for layout module subscribers
    pub layout_text: String,
    /// Plain text for title module subscribers
    pub title_text: String,
    /// Plain text for modifiers subscriber
    pub modifiers_text: String,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subscription {
    Viewport,
    Layout,
    Title,
    Modifiers,
    /// One-shot menu-dismiss events only — never receives state pushes.
    Dismiss,
    Unknown,
}

impl Subscription {
    fn from_str(s: &str) -> Self {
        match s.trim() {
            "viewport" => Subscription::Viewport,
            "layout" => Subscription::Layout,
            "title" => Subscription::Title,
            "modifiers" => Subscription::Modifiers,
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
#[derive(Debug, Clone)]
pub struct StatusSender {
    tx: mpsc::Sender<StatusMsg>,
}

impl StatusSender {
    pub fn send(&self, update: StatusUpdate) {
        // If the channel is full or the receiver is gone, just drop it.
        let _ = self.tx.send(StatusMsg::State(update));
    }

    /// Fire a one-shot menu-dismiss at every `dismiss` subscriber except the
    /// segment with this app_id (pass "-" to exempt nobody).
    pub fn send_menu_dismiss(&self, except_app_id: &str) {
        let _ = self.tx.send(StatusMsg::MenuDismiss { except_app_id: except_app_id.to_string() });
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

    std::thread::Builder::new()
        .name("cce-status-server".into())
        .spawn(move || {
            status_server_main(rx, display_socket);
        })
        .expect("failed to spawn status server thread");

    StatusSender { tx }
}

fn status_server_main(rx: mpsc::Receiver<StatusMsg>, display_socket: Option<String>) {
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
        let mut activity = false;
        let mut has_new_update = false;

        // Accept new connections (non-blocking)
        for _ in 0..5 {
            match listener.accept() {
                Ok((stream, _addr)) => {
                    if let Err(e) = stream.set_nonblocking(true) {
                        log::error!("[status] failed to set non-blocking on client: {}", e);
                        continue;
                    }
                    // Read the subscription line
                    let sub = read_subscription(&stream);
                    if sub != Subscription::Unknown {
                        log::info!("[status] new subscriber for {:?}", sub);
                        let client = Client {
                            subscription: sub,
                            stream,
                            last_line: None,
                        };
                        clients.push(client);
                        activity = true;
                        has_new_update = true; // push the latest status to the new client
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(e) => {
                    log::error!("[status] accept error: {}", e);
                    break;
                }
            }
        }

        // Process incoming updates from the main loop
        let mut dismiss_events: Vec<String> = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(StatusMsg::State(update)) => {
                    latest = Some(update);
                    activity = true;
                    has_new_update = true;
                }
                Ok(StatusMsg::MenuDismiss { except_app_id }) => {
                    dismiss_events.push(except_app_id);
                    activity = true;
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
                    let msg = format_for_subscription(client.subscription, update);
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

        if !activity {
            // Small sleep to avoid busy-looping when nothing is happening
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

fn read_subscription(stream: &UnixStream) -> Subscription {
    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();
    // Try to read with a small timeout
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(100)))
        .ok();
    match reader.read_line(&mut line) {
        Ok(_) => Subscription::from_str(&line),
        Err(e) => {
            log::error!("[status] failed to read subscription: {}", e);
            Subscription::Unknown
        }
    }
}

fn format_for_subscription(sub: Subscription, update: &StatusUpdate) -> String {
    match sub {
        Subscription::Viewport => update.viewport_json.clone(),
        Subscription::Layout => update.layout_text.clone(),
        Subscription::Title => update.title_text.clone(),
        Subscription::Modifiers => update.modifiers_text.clone(),
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

    // The viewport payload carries only the active viewport number (nearest
    // View1-4 anchor): the bar reads it at menu-open time for the layout
    // menu's viewport-layout target. Nothing renders this payload — the
    // viewport tabs are gone, and the old camera debug text (live pan/zoom
    // floats) caused per-frame bar rebuilds during camera animations.
    let anchors = [(0.0f64, 0.0f64), (2000.0, 0.0), (0.0, 2000.0), (2000.0, 2000.0)];
    let active = anchors
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = (wm.desk_pan_x - a.0).powi(2) + (wm.desk_pan_y - a.1).powi(2);
            let db = (wm.desk_pan_x - b.0).powi(2) + (wm.desk_pan_y - b.1).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i + 1)
        .unwrap_or(1);
    let viewport_json = format!("{{\"active\": {}}}", active);

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
        viewport_json,
        layout_text,
        title_text,
        modifiers_text,
    }
}
