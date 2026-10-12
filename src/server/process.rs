// SPDX-FileCopyrightText: © 2022 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

use std::sync::Mutex;
use nix::sys::resource::{getrlimit, setrlimit, Resource};

static ORIGINAL_RLIMIT: Mutex<Option<libc::rlimit>> = Mutex::new(None);

pub fn setup() {
    // Ignore SIGPIPE so we don't get killed when writing to a socket that
    // has had its read end closed by another process.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    // Most unix systems have a default limit of 1024 file descriptors.
    // Raise it to avoid issues with many wayland clients.
    if let Ok((cur, max)) = getrlimit(Resource::RLIMIT_NOFILE) {
        let mut orig = ORIGINAL_RLIMIT.lock().unwrap();
        *orig = Some(libc::rlimit {
            rlim_cur: cur,
            rlim_max: max,
        });

        // A compositor's legitimate fd usage scales with clients × buffers
        // (every imported dmabuf holds one), and hitting the ceiling turns
        // accept() into an EMFILE spin that takes the session down — 4096
        // proved reachable under client-reconnect churn. Children get the
        // original limit back via cleanup_child.
        let new_cur = std::cmp::min(65536, max);
        if let Err(e) = setrlimit(Resource::RLIMIT_NOFILE, new_cur, max) {
            log::error!("setrlimit failed: {}, using system default limit of {}", e, cur);
        } else {
            log::info!("raised file descriptor limit of the river process to {}", new_cur);
        }
    } else {
        log::error!("getrlimit failed, using system default file descriptor limit");
    }
}

pub fn cleanup_child() {
    unsafe {
        if libc::setsid() < 0 {
            // setsid failed
        }

        let mut empty_mask: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut empty_mask);
        libc::sigprocmask(libc::SIG_SETMASK, &empty_mask, std::ptr::null_mut());

        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
    }

    let orig = ORIGINAL_RLIMIT.lock().unwrap();
    if let Some(original) = *orig {
        unsafe {
            libc::setrlimit(libc::RLIMIT_NOFILE, &original);
        }
    }
}

/// Start `/bin/sh -c cmd`, forked and detached like every other client the
/// compositor starts, with this process's environment plus `env` (an entry
/// there replaces the inherited one). Returns the child's pid; a command
/// with a NUL byte in it is `EINVAL`.
///
/// Everything the child needs — argv, envp — is built before the fork: this
/// process is multithreaded, and the child runs only `cleanup_child` and
/// `execve` (`_exit` if that fails, so no atexit handler of ours runs twice).
pub fn spawn_sh(cmd: &str, env: &[(&str, &str)]) -> nix::Result<nix::unistd::Pid> {
    use std::ffi::{CString, OsStr};
    use std::os::unix::ffi::OsStrExt;
    let cmd = CString::new(cmd).map_err(|_| nix::errno::Errno::EINVAL)?;
    let entry = |k: &OsStr, v: &OsStr| {
        let mut bytes = k.as_bytes().to_vec();
        bytes.push(b'=');
        bytes.extend_from_slice(v.as_bytes());
        CString::new(bytes).ok()
    };
    let envp_owned: Vec<CString> = std::env::vars_os()
        .filter(|(k, _)| !env.iter().any(|(e, _)| OsStr::new(e) == k.as_os_str()))
        .filter_map(|(k, v)| entry(&k, &v))
        .chain(env.iter().filter_map(|(k, v)| entry(OsStr::new(k), OsStr::new(v))))
        .collect();
    let mut envp: Vec<*const libc::c_char> = envp_owned.iter().map(|s| s.as_ptr()).collect();
    envp.push(std::ptr::null());
    let argv: [*const libc::c_char; 4] = [c"/bin/sh".as_ptr(), c"-c".as_ptr(), cmd.as_ptr(), std::ptr::null()];

    // SAFETY: the child calls only `cleanup_child` (async-signal-safe libc
    // calls, and a lock of ORIGINAL_RLIMIT, which only `setup` takes, at
    // startup) and then `execve`/`_exit` on the arrays built above.
    match unsafe { nix::unistd::fork() }? {
        nix::unistd::ForkResult::Child => {
            cleanup_child();
            unsafe {
                libc::execve(argv[0], argv.as_ptr(), envp.as_ptr());
                libc::_exit(1)
            }
        }
        nix::unistd::ForkResult::Parent { child } => Ok(child),
    }
}

/// Run `cmd` through `/bin/sh -c`, detached (`spawn_sh`): the `spawn`
/// action; the on-screen keyboard's show and hide.
pub fn spawn_shell(cmd: &str) {
    log::info!("executing spawn: {}, WAYLAND_DISPLAY: {:?}", cmd, std::env::var("WAYLAND_DISPLAY"));
    if let Err(e) = spawn_sh(cmd, &[]) {
        log::error!("failed to fork child process: {}", e);
    }
}
