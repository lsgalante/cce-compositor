use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;

use crate::paths;

pub struct IpcRequest {
    pub command: String,
    pub reply_tx: mpsc::Sender<String>,
}

pub struct IpcReceiver {
    pub rx: mpsc::Receiver<IpcRequest>,
    pub tx: mpsc::Sender<IpcRequest>,
}

pub fn spawn_ipc_server(pipe_write: libc::c_int) -> IpcReceiver {
    let (tx, rx) = mpsc::channel::<IpcRequest>();
    let tx_clone = tx.clone();

    std::thread::Builder::new()
        .name("ccec-ipc".into())
        .spawn(move || {
            ipc_server_main(tx, pipe_write);
        })
        .expect("failed to spawn IPC server thread");

    IpcReceiver { rx, tx: tx_clone }
}

fn ipc_server_main(tx: mpsc::Sender<IpcRequest>, pipe_write: libc::c_int) {
    let socket_path = paths::get_socket_path();
    let _ = std::fs::remove_file(&socket_path);

    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[ipc] failed to bind {}: {}", socket_path, e);
            return;
        }
    };

    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("[ipc] failed to set non-blocking: {}", e);
        return;
    }

    eprintln!("[ipc] listening on {}", socket_path);

    let mut streams: Vec<UnixStream> = Vec::new();

    loop {
        let mut activity = false;
        for _ in 0..5 {
            match listener.accept() {
                Ok((stream, _addr)) => {
                    if let Err(e) = stream.set_nonblocking(true) {
                        eprintln!("[ipc] failed to set non-blocking on client: {}", e);
                        continue;
                    }
                    eprintln!("[ipc] new connection");
                    streams.push(stream);
                    activity = true;
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
                    let mut sent = false;
                    let (reply_tx, reply_rx) = mpsc::channel();
                    let mut cmd_count = 0;
                    for line in s.lines() {
                        let cmd = line.trim().to_string();
                        if !cmd.is_empty() {
                            let _ = tx.send(IpcRequest {
                                command: cmd,
                                reply_tx: reply_tx.clone(),
                            });
                            sent = true;
                            cmd_count += 1;
                        }
                    }
                    if sent {
                        // Wake up main thread
                        unsafe {
                            libc::write(pipe_write, &1u8 as *const u8 as *const libc::c_void, 1);
                        }
                        
                        let mut response = String::new();
                        for _ in 0..cmd_count {
                            if let Ok(res) = reply_rx.recv_timeout(std::time::Duration::from_millis(1000)) {
                                response.push_str(&res);
                            } else {
                                response.push_str("error: timeout or no response\n");
                            }
                        }
                        let _ = stream.write_all(response.as_bytes());
                        dead.push(i);
                    }
                    activity = true;
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

        if !activity {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
