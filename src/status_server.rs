// Status socket server for waybar integration
//
// Runs in a dedicated thread. Waybar custom module scripts connect to
// /tmp/clearwm-status.sock, send a subscription line ("tags", "layout",
// or "title"), and receive JSON lines whenever the status changes.
//
// The main loop sends updates through an mpsc channel — no blocking,
// no fork, no pkill. The server thread owns the socket and handles
// all I/O independently of the Wayland event loop.

use std::io::{BufRead, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;

/// The socket path for the status server.
pub const STATUS_SOCKET_PATH: &str = "/tmp/clearwm-status.sock";

/// A status update sent from the main loop to the server thread.
#[derive(Debug, Clone)]
pub struct StatusUpdate {
    /// JSON string for tags module subscribers
    pub tags_json: String,
    /// Plain text for layout module subscribers
    pub layout_text: String,
    /// Plain text for title module subscribers
    pub title_text: String,
}

/// Subscription types that waybar scripts can request.
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
#[derive(Debug)]
pub struct StatusSender {
    tx: mpsc::Sender<StatusUpdate>,
}

impl StatusSender {
    pub fn send(&self, update: StatusUpdate) {
        // If the channel is full or the receiver is gone, just drop it.
        // Status updates are frequent; missing one is fine.
        let _ = self.tx.send(update);
    }
}

/// Spawn the status server thread. Returns a StatusSender for the main loop.
pub fn spawn_status_server() -> StatusSender {
    let (tx, rx) = mpsc::channel::<StatusUpdate>();

    std::thread::Builder::new()
        .name("clearwm-status".into())
        .spawn(move || {
            status_server_main(rx);
        })
        .expect("failed to spawn status server thread");

    StatusSender { tx }
}

fn status_server_main(rx: mpsc::Receiver<StatusUpdate>) {
    // Remove stale socket
    let _ = std::fs::remove_file(STATUS_SOCKET_PATH);

    let listener = match UnixListener::bind(STATUS_SOCKET_PATH) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[status] failed to bind {}: {}", STATUS_SOCKET_PATH, e);
            return;
        }
    };

    // Set non-blocking so accept() doesn't hang the thread
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("[status] failed to set non-blocking: {}", e);
        return;
    }

    eprintln!("[status] listening on {}", STATUS_SOCKET_PATH);

    let mut clients: Vec<Client> = Vec::new();
    let mut latest: Option<StatusUpdate> = None;

    loop {
        // Accept new connections (non-blocking)
        for _ in 0..5 {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    if let Err(e) = stream.set_nonblocking(true) {
                        eprintln!("[status] failed to set non-blocking on client: {}", e);
                        continue;
                    }
                    // Read the subscription line
                    let subscription = read_subscription(&stream);
                    if subscription == Subscription::Unknown {
                        eprintln!("[status] client sent unknown subscription, dropping");
                        continue;
                    }
                    eprintln!("[status] new client subscribed: {:?}", subscription);

                    // Send current state immediately so waybar shows data on startup
                    if let Some(ref update) = latest {
                        let msg = format_for_subscription(subscription, update);
                        let _ = stream.write_all(msg.as_bytes());
                        let _ = stream.write_all(b"\n");
                    }

                    clients.push(Client {
                        subscription,
                        stream,
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break; // No more pending connections
                }
                Err(e) => {
                    eprintln!("[status] accept error: {}", e);
                    break;
                }
            }
        }

        // Receive status updates from the main loop
        // Use try_recv in a loop to drain all pending updates (only the latest matters)
        loop {
            match rx.try_recv() {
                Ok(update) => {
                    latest = Some(update);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    eprintln!("[status] channel disconnected, exiting");
                    let _ = std::fs::remove_file(STATUS_SOCKET_PATH);
                    return;
                }
            }
        }

        // If we got an update, push it to all clients
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
                        eprintln!(
                            "[status] client {:?} disconnected (broken pipe)",
                            client.subscription
                        );
                        dead_clients.push(i);
                    }
                    Err(e) => {
                        eprintln!(
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

        // Small sleep to avoid busy-looping when nothing is happening
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Read the subscription line from a newly connected client.
/// The client sends one line: "tags", "layout", or "title".
fn read_subscription(stream: &UnixStream) -> Subscription {
    use std::io::BufReader;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    // Try to read with a small timeout
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(100)))
        .ok();
    match reader.read_line(&mut line) {
        Ok(_) => Subscription::from_str(&line),
        Err(e) => {
            eprintln!("[status] failed to read subscription: {}", e);
            Subscription::Unknown
        }
    }
}

/// Format the relevant part of a StatusUpdate for a given subscription.
fn format_for_subscription(sub: Subscription, update: &StatusUpdate) -> String {
    match sub {
        Subscription::Tags => update.tags_json.clone(),
        Subscription::Layout => update.layout_text.clone(),
        Subscription::Title => update.title_text.clone(),
        Subscription::Unknown => String::new(),
    }
}

/// Build a StatusUpdate from the current WindowManager state.
/// This is the same logic that write_status_files() uses, but produces
/// the data for the socket instead of writing to files.
pub fn build_status_update(wm: &crate::types::WindowManager) -> StatusUpdate {
    // Tags: generate the same pango-marked JSON that clearwm-tags.sh produces
    let tags_json = render_tags_json(
        wm.active_tags,
        wm.focused_tags,
        crate::types::NUM_TAGS as u32,
    );

    // Layout: focused window's tiling mode
    let layout_text = wm
        .focused_window()
        .map(|w| w.tiling_mode.as_str().to_string())
        .unwrap_or_else(|| "none".to_string());

    // Title: focused window's title
    let title_text = wm
        .focused_window()
        .and_then(|w| w.title.clone())
        .unwrap_or_else(|| "(none)".to_string());

    StatusUpdate {
        tags_json,
        layout_text,
        title_text,
    }
}

/// Render tag state as a JSON string with pango markup, matching the format
/// produced by the old clearwm-tags.sh script.
///
/// Colors:
/// - Active + Focused: bright (#a8c0d8)
/// - Focused only: dim (#666666)
/// - Active only: medium (#888888)
/// - Neither: dark (#444444)
fn render_tags_json(active: u32, focused: u32, num_tags: u32) -> String {
    let mut text = String::new();
    for i in 0..num_tags {
        let bit = 1u32 << i;
        let label = i + 1;

        let is_active = (active & bit) != 0;
        let is_focused = (focused & bit) != 0;

        let color = if is_focused && is_active {
            "#a8c0d8"
        } else if is_focused {
            "#666666"
        } else if is_active {
            "#888888"
        } else {
            "#444444"
        };

        text.push_str(&format!("<span color='{}'>{}</span>", color, label));
    }

    // waybar expects JSON: {"text": "...", "tooltip": "Tags"}
    // Need to escape the pango markup for JSON
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{{\"text\": \"{}\", \"tooltip\": \"Tags\"}}", escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_tags_json_single_tag() {
        let json = render_tags_json(1, 1, 4);
        // Tag 1 should be active+focused (#a8c0d8), tags 2-4 should be dark (#444444)
        assert!(json.contains("#a8c0d8"), "tag 1 should be bright: {}", json);
        assert!(
            json.contains("#444444"),
            "inactive tags should be dark: {}",
            json
        );
        assert!(json.starts_with("{\"text\":"));
    }

    #[test]
    fn test_render_tags_json_no_focus() {
        let json = render_tags_json(1, 0, 4);
        // Tag 1 is active but not focused → #888888
        assert!(
            json.contains("#888888"),
            "active unfocused should be medium: {}",
            json
        );
    }

    #[test]
    fn test_subscription_from_str() {
        assert_eq!(Subscription::from_str("tags"), Subscription::Tags);
        assert_eq!(Subscription::from_str("layout"), Subscription::Layout);
        assert_eq!(Subscription::from_str("title"), Subscription::Title);
        assert_eq!(Subscription::from_str("foo"), Subscription::Unknown);
        assert_eq!(Subscription::from_str("tags\n"), Subscription::Tags);
    }
}
