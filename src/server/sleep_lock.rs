//! Lock the session before the machine sleeps, whoever puts it to sleep.
//!
//! The idle timeout locks before its own sleep (`IdleManager::lock_then_sleep`),
//! but most sleeps are not ours: closing the lid, the power key, `systemctl
//! suspend` from a terminal are all logind's. Until 2026-10-01 none of them
//! locked anything, so a laptop opened after any of them showed the desktop.
//!
//! logind's answer is a DELAY inhibitor: hold one, and before sleeping logind
//! emits `PrepareForSleep(true)` and waits — up to `InhibitDelayMaxSec`, 5s by
//! default — for the holder to close it. This thread holds one, and on the
//! signal asks the compositor to lock over its own control socket (`lock`,
//! which replies once every output shows the locked scene), then lets go. On
//! `PrepareForSleep(false)` — the resume — it takes a fresh inhibitor for the
//! next sleep.
//!
//! The control socket is the way in because it already is the bridge from a
//! thread to the main loop, and a `lock` reply that waits for the lock is
//! exactly the "done" this needs. Failure is not silent and not fatal: no
//! system bus, or logind refusing, is logged and retried, and the idle path
//! keeps locking on its own.

use std::io::{Read, Write};
use std::time::Duration;

/// Started only for a real seat (a DRM session): a headless shadow has no
/// lid, and taking a delay inhibitor on the real system bus from every
/// shadow would make each suspend wait on them.
pub fn spawn(display_socket: Option<String>) {
    let socket = crate::ipc_server::get_ipc_socket_path(display_socket.as_deref());
    let spawned = std::thread::Builder::new()
        .name("cce-sleep-lock".to_string())
        .spawn(move || loop {
            if let Err(e) = run(&socket) {
                log::warn!("lock-before-sleep: {e}; retrying in 30s");
            }
            std::thread::sleep(Duration::from_secs(30));
        });
    if let Err(e) = spawned {
        log::error!("lock-before-sleep: could not start its thread: {e}");
    }
}

fn run(socket: &str) -> Result<(), String> {
    let conn = zbus::blocking::Connection::system().map_err(|e| format!("no system bus: {e}"))?;
    let logind = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .map_err(|e| format!("no logind: {e}"))?;
    // Subscribed before the first inhibitor is taken, so no PrepareForSleep
    // can fall between holding one and listening for it.
    let mut signals = logind
        .receive_signal("PrepareForSleep")
        .map_err(|e| format!("cannot watch PrepareForSleep: {e}"))?;

    loop {
        let inhibitor: zbus::zvariant::OwnedFd = logind
            .call("Inhibit", &("sleep", "cce", "Lock the screen before sleep", "delay"))
            .map_err(|e| format!("logind refused a sleep inhibitor: {e}"))?;
        log::info!("lock-before-sleep: holding a sleep delay");

        wait_for(&mut signals, true)?;
        log::info!("lock-before-sleep: the system is going to sleep; locking");
        match request(socket, "lock") {
            Ok(reply) => log::info!("lock-before-sleep: {}", reply.trim()),
            // The sleep goes ahead either way: logind stops waiting at its
            // own deadline, and holding on would only spend that.
            Err(e) => log::error!("lock-before-sleep: the lock request failed: {e}"),
        }
        drop(inhibitor);

        wait_for(&mut signals, false)?;
        log::info!("lock-before-sleep: resumed");
    }
}

/// Block until `PrepareForSleep(going)`.
fn wait_for(signals: &mut zbus::blocking::proxy::SignalIterator<'_>, going: bool) -> Result<(), String> {
    for msg in signals.by_ref() {
        match msg.body().deserialize::<bool>() {
            Ok(v) if v == going => return Ok(()),
            Ok(_) => {}
            Err(e) => log::warn!("lock-before-sleep: unreadable PrepareForSleep: {e}"),
        }
    }
    Err("the system bus closed the PrepareForSleep stream".to_string())
}

/// One control-socket command, answered: the same exchange `ccectl` makes.
fn request(socket: &str, command: &str) -> std::io::Result<String> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(command.as_bytes())?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    /// Talks to the real logind, so it is opt-in: `cargo test -p cce-fx --lib
    /// sleep_lock -- --ignored`. It takes a delay inhibitor and drops it at
    /// once (nothing sleeps), and checks the signal subscription the thread
    /// waits on can be made — the two calls a shadow, with no seat, never
    /// exercises.
    #[test]
    #[ignore]
    fn logind_grants_a_sleep_delay_and_the_signal_can_be_watched() {
        let conn = zbus::blocking::Connection::system().expect("system bus");
        let logind = zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .unwrap();
        let _signals = logind.receive_signal("PrepareForSleep").expect("PrepareForSleep subscription");
        let fd: zbus::zvariant::OwnedFd = logind
            .call("Inhibit", &("sleep", "cce-test", "test: dropped at once", "delay"))
            .expect("a sleep delay inhibitor");
        use std::os::fd::AsRawFd;
        assert!(std::os::fd::AsFd::as_fd(&fd).as_raw_fd() >= 0);
    }
}
