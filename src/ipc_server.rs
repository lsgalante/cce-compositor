use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;

pub const IPC_SOCKET_PATH: &str = "/tmp/clearwm.sock";

pub struct IpcReceiver {
    pub rx: mpsc::Receiver<String>,
}

pub fn spawn_ipc_server() -> IpcReceiver {
    let (tx, rx) = mpsc::channel::<String>();

    std::thread::Builder::new()
        .name("clearwm-ipc".into())
        .spawn(move || {
            ipc_server_main(tx);
        })
        .expect("failed to spawn IPC server thread");

    IpcReceiver { rx }
}

fn ipc_server_main(tx: mpsc::Sender<String>) {
    let _ = std::fs::remove_file(IPC_SOCKET_PATH);

    let listener = match UnixListener::bind(IPC_SOCKET_PATH) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[ipc] failed to bind {}: {}", IPC_SOCKET_PATH, e);
            return;
        }
    };

    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("[ipc] failed to set non-blocking: {}", e);
        return;
    }

    eprintln!("[ipc] listening on {}", IPC_SOCKET_PATH);

    let mut streams: Vec<UnixStream> = Vec::new();

    loop {
        for _ in 0..5 {
            match listener.accept() {
                Ok((stream, _addr)) => {
                    if let Err(e) = stream.set_nonblocking(true) {
                        eprintln!("[ipc] failed to set non-blocking on client: {}", e);
                        continue;
                    }
                    eprintln!("[ipc] new connection");
                    streams.push(stream);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    eprintln!("[ipc] accept error: {}", e);
                    break;
                }
            }
        }

        let mut dead = Vec::new();
        for (i, stream) in streams.iter_mut().enumerate() {
            let mut buf = [0u8; 4096];
            match stream.read(&mut buf) {
                Ok(0) => {
                    dead.push(i);
                }
                Ok(n) => {
                    let s = String::from_utf8_lossy(&buf[..n]);
                    for line in s.lines() {
                        let cmd = line.trim().to_string();
                        if !cmd.is_empty() {
                            let _ = tx.send(cmd);
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    eprintln!("[ipc] read error: {}", e);
                    dead.push(i);
                }
            }
        }

        for i in dead.into_iter().rev() {
            streams.remove(i);
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
