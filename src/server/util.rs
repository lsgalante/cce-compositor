// SPDX-FileCopyrightText: © 2022 The River Developers
// SPDX-License-Identifier: GPL-3.0-only

/// CLOCK_MONOTONIC in nanoseconds — the presentation clock's time base
/// (`wlr_output_event_present.when` is on the same clock).
pub fn timestamp_ns() -> u64 {
    let ts = timestamp();
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

pub fn timestamp() -> libc::timespec {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe {
        if libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) < 0 {
            panic!("CLOCK_MONOTONIC not supported");
        }
    }
    ts
}

pub fn msec_timestamp() -> u32 {
    let now = timestamp();
    // 2^32-1 milliseconds is ~50 days.
    // Wrap it using wrapping arithmetic.
    let secs_ms = (now.tv_sec as u64).wrapping_mul(1000);
    let nsecs_ms = (now.tv_nsec as u64) / 1_000_000;
    secs_ms.wrapping_add(nsecs_ms) as u32
}
