// Monolithic IPC Server socket listener for CCE
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::thread;

pub struct IpcRequest {
    pub command: String,
    pub reply_tx: mpsc::Sender<String>,
}

fn get_ipc_socket_path(display_socket: Option<&str>) -> String {
    if let Some(display) = display_socket {
        format!("/tmp/cce-{}.sock", display)
    } else {
        "/tmp/cce.sock".to_string()
    }
}

pub fn spawn_ipc_server(display_socket: Option<String>) -> mpsc::Receiver<IpcRequest> {
    let (tx, rx) = mpsc::channel::<IpcRequest>();
    
    thread::Builder::new()
        .name("cce-ipc-server".to_string())
        .spawn(move || {
            ipc_server_main(tx, display_socket);
        })
        .expect("Failed to spawn CCE IPC server thread");

    rx
}

fn ipc_server_main(tx: mpsc::Sender<IpcRequest>, display_socket: Option<String>) {
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
            Ok(mut s) => {
                let tx_clone = tx.clone();
                thread::spawn(move || {
                    handle_client(s, tx_clone);
                });
            }
            Err(e) => {
                log::error!("[ipc] accept error: {}", e);
            }
        }
    }
}

fn handle_client(mut stream: UnixStream, tx: mpsc::Sender<IpcRequest>) {
    let mut buf = [0u8; 4096];
    match stream.read(&mut buf) {
        Ok(0) => {}
        Ok(n) => {
            let s = String::from_utf8_lossy(&buf[..n]);
            let cmd = s.trim().to_string();
            if !cmd.is_empty() {
                let (reply_tx, reply_rx) = mpsc::channel();
                if tx.send(IpcRequest { command: cmd, reply_tx }).is_ok() {
                    if let Ok(reply) = reply_rx.recv_timeout(std::time::Duration::from_millis(1000)) {
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
