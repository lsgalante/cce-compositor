// SPDX-License-Identifier: GPL-3.0-only

//! The process half of `ccectl restart-app`: what to start again, and starting
//! it. The window half (closing the app's windows, waiting for it to exit) is
//! `window_manager/restart_app.rs`.
//!
//! An app is restarted by closing its windows rather than by a signal: cce-ui
//! clients install no SIGTERM handler, so a killed one never runs its
//! `on_exit` (cce-notes' autosave, every app's saved window state), and a
//! close is also what gives an app with unsaved work its chance to ask. What
//! to start is read from `/proc/<pid>` *before* the close — argv, environment,
//! working directory and the binary's path — so the new process is the old
//! one's exact twin, on whatever binary now sits at that path. That last part
//! is the point after a `ccebuild install`: the running exe reads
//! "<path> (deleted)", and the same path now holds the new build.
//!
//! An app that a systemd user unit runs (`cce-grid.service`, the status bar)
//! is not started by hand — that would put a second copy outside the unit.
//! Once it has exited, the unit is started again with `systemctl --user
//! start`, which is a no-op if its own `Restart=` already did it.
//!
//! Before this existed, restarting a compositor-launched client was a
//! hand-run recipe: copy its environ (the compositor's own has no
//! WAYLAND_DISPLAY), close it, relaunch under bash because zsh does not
//! word-split the copied list. And `ccebuild restart` only reaches units.

use std::ffi::{CString, OsStr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// How long the app has to exit after its windows were asked to close
/// before the restart gives up (or, with `--force`, signals it).
pub const GRACE_MS: u64 = 10_000;
/// With `--force`: SIGTERM at [`GRACE_MS`], SIGKILL this long after.
pub const KILL_AFTER_TERM_MS: u64 = 3_000;
/// Poll period while a restart waits.
pub const POLL_MS: i32 = 100;

/// Everything needed to start a process again, read before it goes.
#[derive(Debug)]
pub struct Snapshot {
    pub pid: i32,
    /// The binary's basename, for replies and logs.
    pub name: String,
    /// Where the binary is now: the exe link with any " (deleted)" removed.
    pub exe: PathBuf,
    pub argv: Vec<CString>,
    pub envp: Vec<CString>,
    pub cwd: Option<PathBuf>,
    /// The systemd user unit that runs it, if one does.
    pub unit: Option<String>,
}

impl Snapshot {
    /// Read `/proc/<pid>`. Refuses a process that is not the user's, the
    /// compositor itself, or one whose binary is no longer on disk.
    pub fn take(pid: i32) -> Result<Snapshot, String> {
        if pid <= 1 {
            return Err("that window has no process id".into());
        }
        if pid == std::process::id() as i32 {
            return Err("that window belongs to the compositor".into());
        }
        let proc_dir = PathBuf::from(format!("/proc/{}", pid));
        let owner = std::os::unix::fs::MetadataExt::uid(
            &std::fs::metadata(&proc_dir).map_err(|e| format!("process {}: {}", pid, e))?,
        );
        // SAFETY: getuid has no preconditions.
        if owner != unsafe { libc::getuid() } {
            return Err(format!("process {} is not this user's", pid));
        }
        let link = std::fs::read_link(proc_dir.join("exe")).map_err(|e| format!("process {}: exe: {}", pid, e))?;
        let exe = live_path(&link);
        if !exe.is_file() {
            return Err(format!("{} is no longer on disk", exe.display()));
        }
        let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name == "Xwayland" {
            return Err("that window's process is Xwayland itself".into());
        }
        let argv = nul_list(&std::fs::read(proc_dir.join("cmdline")).map_err(|e| format!("process {}: cmdline: {}", pid, e))?);
        if argv.is_empty() {
            return Err(format!("process {} has no command line (a kernel thread or a zombie)", pid));
        }
        let envp = nul_list(&std::fs::read(proc_dir.join("environ")).map_err(|e| format!("process {}: environ: {}", pid, e))?);
        let cwd = std::fs::read_link(proc_dir.join("cwd")).ok().map(|p| live_path(&p)).filter(|p| p.is_dir());
        let unit = std::fs::read_to_string(proc_dir.join("cgroup")).ok().and_then(|cg| {
            let own = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
            unit_of(&cg, &own)
        });
        Ok(Snapshot { pid, name, exe, argv, envp, cwd, unit })
    }

    /// Start it again: `systemctl --user start` for a unit, else a detached
    /// twin of the old process (`cleanup_child` like every other client the
    /// compositor starts). Returns the new pid; for a unit, the systemctl's.
    pub fn relaunch(&self) -> Result<i32, String> {
        if let Some(unit) = &self.unit {
            return crate::process::spawn_sh(&format!("systemctl --user start {}", sh_quote(unit)), &[])
                .map(|p| p.as_raw())
                .map_err(|e| format!("systemctl: {}", e));
        }
        let exe = CString::new(self.exe.as_os_str().as_bytes()).map_err(|_| "exe path has a NUL".to_string())?;
        // Fall back to HOME when the old directory is gone, as a fresh
        // launch from the launcher would have.
        let home = self
            .envp
            .iter()
            .find_map(|e| e.to_bytes().strip_prefix(b"HOME=").map(|h| PathBuf::from(OsStr::from_bytes(h))));
        let cwd = self
            .cwd
            .clone()
            .or(home)
            .and_then(|p| CString::new(p.as_os_str().as_bytes()).ok());
        let mut argv: Vec<*const libc::c_char> = self.argv.iter().map(|a| a.as_ptr()).collect();
        argv.push(std::ptr::null());
        let mut envp: Vec<*const libc::c_char> = self.envp.iter().map(|e| e.as_ptr()).collect();
        envp.push(std::ptr::null());

        // SAFETY: as `process::spawn_sh` — everything is built before the
        // fork, and the child runs only `cleanup_child`, `chdir` and
        // `execve`/`_exit` on it.
        match unsafe { nix::unistd::fork() }.map_err(|e| format!("fork: {}", e))? {
            nix::unistd::ForkResult::Child => {
                crate::process::cleanup_child();
                unsafe {
                    if let Some(cwd) = &cwd {
                        libc::chdir(cwd.as_ptr());
                    }
                    libc::execve(exe.as_ptr(), argv.as_ptr(), envp.as_ptr());
                    libc::_exit(127)
                }
            }
            nix::unistd::ForkResult::Parent { child } => Ok(child.as_raw()),
        }
    }
}

/// A restart waiting for its process to exit.
pub struct PendingRestart {
    pub snap: Snapshot,
    /// Readable once the process has exited — zombie or reaped, and immune
    /// to the pid being reused meanwhile.
    pidfd: OwnedFd,
    /// The `ccectl` caller, answered when the restart lands or gives up.
    pub reply: Option<std::sync::mpsc::Sender<String>>,
    pub started_ns: u64,
    pub force: bool,
    /// When `--force` sent SIGTERM.
    pub termed_ns: Option<u64>,
    pub killed: bool,
}

impl PendingRestart {
    pub fn new(snap: Snapshot, reply: Option<std::sync::mpsc::Sender<String>>, force: bool, now_ns: u64) -> Result<Self, String> {
        // SAFETY: pidfd_open takes a pid and flags and returns a new fd or -1.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, snap.pid, 0) } as i32;
        if fd < 0 {
            return Err(format!("pidfd_open({}): {}", snap.pid, std::io::Error::last_os_error()));
        }
        // SAFETY: `fd` is the new descriptor pidfd_open just returned, owned
        // by nothing else.
        let pidfd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(PendingRestart { snap, pidfd, reply, started_ns: now_ns, force, termed_ns: None, killed: false })
    }

    /// Whether the process has exited.
    pub fn exited(&self) -> bool {
        let mut pfd = libc::pollfd { fd: self.pidfd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd, zero timeout.
        unsafe { libc::poll(&mut pfd, 1, 0) > 0 && pfd.revents & libc::POLLIN != 0 }
    }

    /// Send `sig` through the pidfd, so it cannot reach a reused pid.
    pub fn signal(&self, sig: i32) {
        // SAFETY: pidfd_send_signal on our own pidfd with no siginfo.
        unsafe {
            libc::syscall(libc::SYS_pidfd_send_signal, self.pidfd.as_raw_fd(), sig, std::ptr::null::<libc::siginfo_t>(), 0);
        }
    }

    pub fn answer(&mut self, line: String) {
        if let Some(tx) = self.reply.take() {
            let _ = tx.send(line);
        }
    }
}

/// `/proc` links name a replaced or removed file "<path> (deleted)"; the
/// path is what matters.
fn live_path(link: &Path) -> PathBuf {
    let bytes = link.as_os_str().as_bytes();
    PathBuf::from(OsStr::from_bytes(bytes.strip_suffix(b" (deleted)").unwrap_or(bytes)))
}

/// `/proc/<pid>/cmdline` and `environ`: NUL-terminated strings.
fn nul_list(bytes: &[u8]) -> Vec<CString> {
    bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .filter_map(|s| CString::new(s).ok())
        .collect()
}

/// The systemd user unit a process runs in, from its `/proc/<pid>/cgroup`
/// (cgroup v2: "0::/user.slice/…/user@1000.service/app.slice/cce-grid.service").
/// None for a scope (session-N.scope: the compositor's children;
/// app-*.scope: the launcher's), for the user manager itself, and for a
/// process sharing the compositor's own cgroup — which, were the compositor
/// itself in a unit, would otherwise make every client "the compositor's
/// unit", and starting that is no restart of the app.
fn unit_of(cgroup: &str, own_cgroup: &str) -> Option<String> {
    let path = |cg: &str| cg.lines().find_map(|l| l.strip_prefix("0::")).map(str::to_string);
    let p = path(cgroup)?;
    if path(own_cgroup).as_deref() == Some(p.as_str()) {
        return None;
    }
    let leaf = p.rsplit('/').next()?;
    if !p.contains("/user@") || leaf.starts_with("user@") || !leaf.ends_with(".service") {
        return None;
    }
    // systemd escapes '-' in slice and unit names as `\x2d`; that is the
    // unit's real name, which systemctl takes as it is.
    Some(leaf.to_string())
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleted_exe_resolves_to_its_path() {
        assert_eq!(live_path(Path::new("/home/u/.local/bin/cce-notes (deleted)")), PathBuf::from("/home/u/.local/bin/cce-notes"));
        assert_eq!(live_path(Path::new("/usr/bin/foo")), PathBuf::from("/usr/bin/foo"));
    }

    #[test]
    fn cmdline_keeps_arguments_with_spaces() {
        let v = nul_list(b"cce-notes\0/home/u/Vault 1\0\0");
        assert_eq!(v, vec![CString::new("cce-notes").unwrap(), CString::new("/home/u/Vault 1").unwrap()]);
        assert!(nul_list(b"").is_empty());
    }

    #[test]
    fn only_a_user_service_is_a_unit() {
        let own = "0::/user.slice/user-1000.slice/session-3.scope\n";
        let unit = |cg: &str| unit_of(cg, own);
        assert_eq!(
            unit("0::/user.slice/user-1000.slice/user@1000.service/app.slice/cce-grid.service\n").as_deref(),
            Some("cce-grid.service")
        );
        // The compositor's children and the launcher's apps are scopes.
        assert_eq!(unit("0::/user.slice/user-1000.slice/session-3.scope\n"), None);
        assert_eq!(unit("0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-cce-cloud-cce-gallery-1.scope\n"), None);
        assert_eq!(unit("0::/user.slice/user-1000.slice/user@1000.service/init.scope\n"), None);
        // A system service is not the user's to start.
        assert_eq!(unit("0::/system.slice/foo.service\n"), None);
        // Sharing the compositor's cgroup is never a unit of its own.
        assert_eq!(unit_of("0::/user.slice/user-1000.slice/user@1000.service/app.slice/cce.service\n", "0::/user.slice/user-1000.slice/user@1000.service/app.slice/cce.service\n"), None);
    }

    #[test]
    fn quoting_survives_systemd_escapes() {
        assert_eq!(sh_quote(r"dbus-:1.2-org.x\x2dy@0.service"), r"'dbus-:1.2-org.x\x2dy@0.service'");
        assert_eq!(sh_quote("a'b"), r"'a'\''b'");
    }
}
