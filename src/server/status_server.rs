// Status socket server for monolithic cce server
//
// Runs in a dedicated thread. cce-status-interface connects to
// /tmp/cce-status-{WAYLAND_DISPLAY}.sock, sends a subscription line
// ("tags", "layout", or "title"), and receives JSON lines whenever the status changes.
//
// The main loop sends updates through an mpsc channel. The server thread
// owns the socket and handles all I/O independently of the Wayland event loop.

use std::io::{BufRead, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;

/// A status update sent from the main loop to the server thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusUpdate {
    /// JSON string for tags module subscribers
    pub tags_json: String,
    /// Plain text for layout module subscribers
    pub layout_text: String,
    /// Plain text for title module subscribers
    pub title_text: String,
}

/// Subscription types that the status bar script can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subscription {
    Tags,
    Layout,
    Title,
    Unknown,
}

impl Subscription {
    fn from_str(s: &str) -> Self {
        match s.trim() {
            "tags" => Subscription::Tags,
            "layout" => Subscription::Layout,
            "title" => Subscription::Title,
            _ => Subscription::Unknown,
        }
    }
}

/// A connected client with a known subscription.
struct Client {
    subscription: Subscription,
    stream: UnixStream,
}

/// Handle to the status server for sending updates from the main loop.
#[derive(Debug, Clone)]
pub struct StatusSender {
    tx: mpsc::Sender<StatusUpdate>,
}

impl StatusSender {
    pub fn send(&self, update: StatusUpdate) {
        // If the channel is full or the receiver is gone, just drop it.
        let _ = self.tx.send(update);
    }
}

pub fn get_status_socket_path(display_socket: Option<&str>) -> String {
    if let Some(display) = display_socket {
        format!("/tmp/cce-status-{}.sock", display)
    } else {
        "/tmp/cce-status.sock".to_string()
    }
}

/// Spawn the status server thread. Returns a StatusSender for the main loop.
pub fn spawn_status_server(display_socket: Option<String>) -> StatusSender {
    let (tx, rx) = mpsc::channel::<StatusUpdate>();

    std::thread::Builder::new()
        .name("cce-status-server".into())
        .spawn(move || {
            status_server_main(rx, display_socket);
        })
        .expect("failed to spawn status server thread");

    StatusSender { tx }
}

fn status_server_main(rx: mpsc::Receiver<StatusUpdate>, display_socket: Option<String>) {
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
                Ok((mut stream, _addr)) => {
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
        loop {
            match rx.try_recv() {
                Ok(update) => {
                    latest = Some(update);
                    activity = true;
                    has_new_update = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    log::info!("[status] channel disconnected, exiting");
                    let _ = std::fs::remove_file(&socket_path);
                    return;
                }
            }
        }

        // If we got a new update, push it to all clients
        if has_new_update {
            if let Some(ref update) = latest {
                let mut dead_clients = Vec::new();

                for (i, client) in clients.iter_mut().enumerate() {
                    let msg = format_for_subscription(client.subscription, update);
                    match client
                        .stream
                        .write_all(msg.as_bytes())
                        .and_then(|_| client.stream.write_all(b"\n"))
                    {
                        Ok(_) => {}
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
        Subscription::Tags => update.tags_json.clone(),
        Subscription::Layout => update.layout_text.clone(),
        Subscription::Title => update.title_text.clone(),
        Subscription::Unknown => String::new(),
    }
}

pub unsafe fn build_status_update(wm: &crate::window_manager::WindowManager) -> StatusUpdate {
    let focused_window = wm.focused_window();
    let focused_tags = if !focused_window.is_null() {
        (*focused_window).tags
    } else {
        0
    };

    let tags_json = render_tags_json(
        wm.active_tags,
        focused_tags,
        4,
        &wm.layout.status_normal_color,
    );

    let layout_text = if wm.expose_active {
        "Expose".to_string()
    } else if !focused_window.is_null() {
        (*focused_window).tiling_mode.as_str().to_string()
    } else {
        wm.global_layout.as_str().to_string()
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

    StatusUpdate {
        tags_json,
        layout_text,
        title_text,
    }
}

fn render_tags_json(active: u32, focused: u32, num_tags: u32, normal_color: &str) -> String {
    let mut text = String::new();
    for i in 0..num_tags {
        let bit = 1u32 << i;
        let label = i + 1;

        let is_active = (active & bit) != 0;
        let is_focused = (focused & bit) != 0;

        let color = if is_focused && is_active {
            normal_color
        } else if is_focused {
            "#666666"
        } else if is_active {
            "#888888"
        } else {
            "#444444"
        };

        text.push_str(&format!("<span color='{}'>{}</span>", color, label));
    }

    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{{\"text\": \"{}\", \"tooltip\": \"Tags\"}}", escaped)
}
