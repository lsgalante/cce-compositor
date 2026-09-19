// Monolithic IPC Server socket listener for CCE
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

pub struct IpcRequest {
    pub command: String,
    pub reply_tx: mpsc::Sender<String>,
    /// PID of the process on the other end of the socket, from SO_PEERCRED.
    /// A command that acts on "whoever is asking" (`fade-out`) resolves its
    /// target with this instead of trusting a name the caller supplies: the
    /// kernel vouches for it, and a client always knows its own pid even
    /// when it does not know its app_id. 0 when the credentials were
    /// unreadable, which every such command treats as no target.
    pub peer_pid: i32,
}

/// The server-thread end of the request channel. Every `send` is followed by
/// a write to the wake eventfd, which the compositor has registered with its
/// wl_event_loop — that is what gets a request dispatched. The drain used to
/// be a 10 ms timer polling `try_recv` forever, ~100 wakeups/s on an idle
/// desktop; now the main thread sleeps until a command actually arrives.
#[derive(Clone)]
struct IpcSender {
    tx: mpsc::Sender<IpcRequest>,
    wake: Arc<OwnedFd>,
}

impl IpcSender {
    fn send(&self, req: IpcRequest) -> bool {
        if self.tx.send(req).is_err() {
            return false;
        }
        wake_fd(&self.wake);
        true
    }
}

/// Bump an eventfd. Errors are ignored on purpose: EAGAIN means the counter
/// is already saturated (the reader is about to run anyway), and EBADF only
/// happens at shutdown.
pub fn wake_fd(fd: &OwnedFd) {
    let one: u64 = 1;
    unsafe {
        libc::write(fd.as_raw_fd(), &one as *const u64 as *const libc::c_void, 8);
    }
}

/// Clear an eventfd after its readable event fired.
pub fn drain_wake_fd(fd: std::os::raw::c_int) {
    let mut v: u64 = 0;
    unsafe {
        libc::read(fd, &mut v as *mut u64 as *mut libc::c_void, 8);
    }
}

/// A non-blocking, close-on-exec eventfd for cross-thread wakeups into the
/// wl_event_loop.
pub fn new_wake_fd() -> std::io::Result<Arc<OwnedFd>> {
    let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Arc::new(unsafe { OwnedFd::from_raw_fd(raw) }))
}

fn get_ipc_socket_path(display_socket: Option<&str>) -> String {
    if let Some(display) = display_socket {
        format!("/tmp/cce-{}.sock", display)
    } else {
        "/tmp/cce.sock".to_string()
    }
}

/// Spawn the IPC listener thread. Returns the request receiver and the
/// eventfd that is bumped after every request is queued; the caller adds the
/// fd to its event loop and drains the receiver when it fires.
pub fn spawn_ipc_server(display_socket: Option<String>) -> (mpsc::Receiver<IpcRequest>, Arc<OwnedFd>) {
    let (tx, rx) = mpsc::channel::<IpcRequest>();
    let wake = new_wake_fd().expect("Failed to create IPC wake eventfd");
    let sender = IpcSender { tx, wake: wake.clone() };

    thread::Builder::new()
        .name("cce-ipc-server".to_string())
        .spawn(move || {
            ipc_server_main(sender, display_socket);
        })
        .expect("Failed to spawn CCE IPC server thread");

    (rx, wake)
}

fn ipc_server_main(tx: IpcSender, display_socket: Option<String>) {
    let socket_path = get_ipc_socket_path(display_socket.as_deref());
    let _ = std::fs::remove_file(&socket_path);

    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("[ipc] failed to bind IPC socket {}: {}", socket_path, e);
            return;
        }
    };

    log::info!("[ipc] Listening on UNIX socket: {}", socket_path);

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let tx_clone = tx.clone();
                thread::spawn(move || {
                    handle_client(s, tx_clone);
                });
            }
            Err(e) => {
                // EMFILE and friends leave the socket readable, so a bare
                // continue spins this thread at 100% and floods the log
                // (167GB observed under fd exhaustion). Back off instead —
                // the session is degraded but stays diagnosable.
                log::error!("[ipc] accept error: {}", e);
                thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

/// PID of the process on the other end of a Unix socket, via SO_PEERCRED.
/// 0 when the credentials cannot be read — the kernel supplies them for every
/// AF_UNIX peer, so that only happens on a socket already going away.
/// (`UnixStream::peer_cred` is still nightly-only, hence the raw getsockopt.)
fn socket_peer_pid(stream: &UnixStream) -> i32 {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if rc == 0 {
        cred.pid
    } else {
        0
    }
}

fn handle_client(mut stream: UnixStream, tx: IpcSender) {
    let peer_pid = socket_peer_pid(&stream);
    let mut buf = [0u8; 4096];
    match stream.read(&mut buf) {
        Ok(0) => {}
        Ok(n) => {
            let s = String::from_utf8_lossy(&buf[..n]);
            let cmd = s.trim().to_string();
            if !cmd.is_empty() {
                // Commands answer from the IPC drain and so are quick; a
                // second is a generous leash that still surfaces a wedged
                // compositor. `screenshot` is the exception: its reply now
                // waits for the capture, which happens on the next composited
                // frame, and a cold readback (first capture after an idle
                // spell — NVIDIA recompiles shaders on the way) has been
                // measured over a second. Timing that out would report
                // failure for a capture that lands.
                let timeout = if cmd.starts_with("screenshot") {
                    std::time::Duration::from_secs(5)
                } else {
                    std::time::Duration::from_millis(1000)
                };
                let (reply_tx, reply_rx) = mpsc::channel();
                if tx.send(IpcRequest { command: cmd, reply_tx, peer_pid }) {
                    if let Ok(reply) = reply_rx.recv_timeout(timeout) {
                        let _ = stream.write_all(reply.as_bytes());
                    } else {
                        let _ = stream.write_all(b"error: timeout processing command\n");
                    }
                }
            }
        }
        Err(e) => {
            log::error!("[ipc] stream read error: {}", e);
        }
    }
}
