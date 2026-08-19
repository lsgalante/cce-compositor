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
