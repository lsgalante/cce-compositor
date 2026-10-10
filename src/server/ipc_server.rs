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

pub fn get_ipc_socket_path(display_socket: Option<&str>) -> String {
    cce_core::ipc::ctl::control_socket_for(display_socket)
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

/// One subscription line from a socket client, within `limit` bytes and an
/// overall `deadline`. The status and stream sockets read their client's
/// first line on a thread that serves everyone else too, with `read_line`
/// and a per-read timeout only: a client trickling a byte per timeout held
/// that thread (every status-bar update, or every new stream subscriber)
/// indefinitely, and grew the line without bound inside the compositor.
/// None on timeout, overflow, EOF before any byte, or a read error.
pub fn read_line_bounded(stream: &UnixStream, limit: usize, deadline: std::time::Duration) -> Option<String> {
    let until = std::time::Instant::now() + deadline;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 512];
    let mut reader = stream;
    loop {
        let left = until.checked_duration_since(std::time::Instant::now())?;
        if left.is_zero() {
            return None;
        }
        stream.set_read_timeout(Some(left)).ok()?;
        match reader.read(&mut chunk) {
            Ok(0) if buf.is_empty() => return None,
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(end) = buf.iter().position(|&b| b == b'\n') {
                    buf.truncate(end);
                    break;
                }
                if buf.len() > limit {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
    if buf.len() > limit {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Longest command accepted. The old single `read` into 4096 bytes cut a
/// longer command — or one written in pieces — short and RAN the prefix
/// (`spawn` included); now an overlong command is refused whole.
const MAX_COMMAND: usize = 64 * 1024;
/// How long a connection may sit without sending anything. Each one holds a
/// thread, and the old read had no timeout, so idle connections leaked them.
const FIRST_BYTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Once some bytes are in, a pause this long ends the command. Clients send
/// one line and then wait for the reply, but not all of them send the
/// newline or close their side, and those must not stall.
const QUIET_GAP: std::time::Duration = std::time::Duration::from_millis(50);

#[derive(Debug, PartialEq)]
enum Request {
    Command(String),
    /// Nothing arrived (a probe, an idle connection, EOF at once).
    Empty,
    TooLong,
    /// A NUL byte: no command contains one, and it reached a CString
    /// conversion (`shortcut bind`'s keysym lookup) whose panic aborted the
    /// compositor.
    Nul,
}

/// Read one command: up to its newline, the client closing its side, or a
/// pause of `QUIET_GAP` after the first bytes — never more than
/// `MAX_COMMAND`, never waiting more than `FIRST_BYTE_TIMEOUT` for a start.
fn read_command(stream: &mut UnixStream) -> Request {
    let _ = stream.set_read_timeout(Some(FIRST_BYTE_TIMEOUT));
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') {
                    break;
                }
                if buf.len() > MAX_COMMAND {
                    return Request::TooLong;
                }
                let _ = stream.set_read_timeout(Some(QUIET_GAP));
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
            Err(_) => return Request::Empty,
        }
    }
    let line = match buf.iter().position(|&b| b == b'\n') {
        Some(end) => &buf[..end],
        None => &buf[..],
    };
    if line.len() > MAX_COMMAND {
        return Request::TooLong;
    }
    if line.contains(&0) {
        return Request::Nul;
    }
    let cmd = String::from_utf8_lossy(line).trim().to_string();
    if cmd.is_empty() {
        Request::Empty
    } else {
        Request::Command(cmd)
    }
}

fn handle_client(mut stream: UnixStream, tx: IpcSender) {
    let peer_pid = socket_peer_pid(&stream);
    match read_command(&mut stream) {
        Request::Empty => {}
        Request::TooLong => {
            let _ = stream.write_all(format!("error: command longer than {MAX_COMMAND} bytes, not run\n").as_bytes());
        }
        Request::Nul => {
            let _ = stream.write_all(b"error: command contains a NUL byte, not run\n");
        }
        Request::Command(cmd) => {
            {
                // Commands answer from the IPC drain and so are quick; a
                // second is a generous leash that still surfaces a wedged
                // compositor. `screenshot` is the exception: its reply now
                // waits for the capture, which happens on the next composited
                // frame, and a cold readback (first capture after an idle
                // spell — NVIDIA recompiles shaders on the way) has been
                // measured over a second. Timing that out would report
                // failure for a capture that lands.
                // `lock` answers once the session IS locked: a frame on every
                // output plus a locker starting, and the lock-before-sleep
                // thread holds logind's sleep (5s at most) on that reply.
                // `focus-window --wait` answers once the window holds still,
                // `SETTLE_TIMEOUT_MS` (3s) at most.
                let timeout = if cmd.starts_with("screenshot") {
                    std::time::Duration::from_secs(5)
                } else if cmd == "lock" || cmd.starts_with("focus-window --wait") {
                    std::time::Duration::from_secs(4)
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
    }
}

#[cfg(test)]
mod framing_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn send(bytes: &[u8], close: bool) -> Request {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client.write_all(bytes).unwrap();
        if close {
            client.shutdown(std::net::Shutdown::Write).unwrap();
        }
        let got = read_command(&mut server);
        drop(client);
        got
    }

    #[test]
    fn a_command_ends_at_its_newline_its_close_or_a_pause() {
        assert_eq!(send(b"windows --json\n", false), Request::Command("windows --json".into()));
        assert_eq!(send(b"lock", true), Request::Command("lock".into()));
        // No newline and no close (the old sleep_lock request did this): the
        // quiet gap ends it rather than the 5s first-byte timeout.
        let t = Instant::now();
        assert_eq!(send(b"lock", false), Request::Command("lock".into()));
        assert!(t.elapsed() < Duration::from_secs(1));
        // Only the first line is the command.
        assert_eq!(send(b"spawn foo\nspawn bar\n", false), Request::Command("spawn foo".into()));
    }

    #[test]
    fn a_command_written_in_pieces_arrives_whole() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            client.write_all(b"spawn ").unwrap();
            std::thread::sleep(Duration::from_millis(10));
            client.write_all(b"cce-terminal\n").unwrap();
            client
        });
        assert_eq!(read_command(&mut server), Request::Command("spawn cce-terminal".into()));
        drop(writer.join().unwrap());
    }

    #[test]
    fn an_overlong_command_is_refused_not_cut_short() {
        // The old reader ran the first 4096 bytes of this.
        let mut long = b"spawn ".to_vec();
        long.extend(std::iter::repeat(b'x').take(MAX_COMMAND + 10));
        long.push(b'\n');
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            let _ = client.write_all(&long);
            client
        });
        assert_eq!(read_command(&mut server), Request::TooLong);
        drop(server);
        drop(writer.join().unwrap());
        // 5000 bytes, over the old 4096, is now one whole command.
        let mid = format!("spawn {}\n", "y".repeat(5000));
        assert_eq!(send(mid.as_bytes(), false), Request::Command(mid.trim().to_string()));
    }

    #[test]
    fn a_nul_byte_is_refused() {
        assert_eq!(send(b"shortcut bind /s/1 x CTRL+a\0b\n", false), Request::Nul);
    }

    #[test]
    fn a_silent_or_trickling_subscriber_is_cut_off_on_time() {
        let (_client, server) = UnixStream::pair().unwrap();
        let t = Instant::now();
        assert_eq!(read_line_bounded(&server, 256, Duration::from_millis(100)), None);
        assert!(t.elapsed() < Duration::from_millis(500));

        // A byte every 30ms never finishes a line; the TOTAL deadline ends it
        // (the old per-read timeout never fired, each read being on time).
        let (mut client, server) = UnixStream::pair().unwrap();
        let trickle = std::thread::spawn(move || {
            for _ in 0..40 {
                if client.write_all(b"a").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        });
        let t = Instant::now();
        assert_eq!(read_line_bounded(&server, 256, Duration::from_millis(200)), None);
        assert!(t.elapsed() < Duration::from_millis(600), "took {:?}", t.elapsed());
        drop(server);
        trickle.join().unwrap();

        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(b"layout\n").unwrap();
        assert_eq!(read_line_bounded(&server, 256, Duration::from_millis(200)).as_deref(), Some("layout"));
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(&[b'z'; 300]).unwrap();
        assert_eq!(read_line_bounded(&server, 256, Duration::from_millis(200)), None, "over the size cap");
    }
}
