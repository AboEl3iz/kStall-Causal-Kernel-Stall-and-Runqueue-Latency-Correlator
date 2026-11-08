// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers
//! Ground-Truth Latency Probe (PRD §16.4 & §16.7)
//!
//! A micro-workload that executes periodic high-precision sleep loops
//! and computes its own measured wakeup lateness. When subjected to CPU starvation
//! (e.g. stress-ng or nice/chrt pinning), this serves as the oracle ground truth
//! against which kStall's reported stall duration is cross-validated.

use std::thread::sleep;
use std::time::{Duration, Instant};

fn main() {
    let target_interval = Duration::from_millis(50);
    let pid = std::process::id();
    println!(
        "[latency_probe] Started (PID: {}). Target interval: {:?}",
        pid, target_interval
    );

    let mut iteration: u64 = 0;
    loop {
        let expected_wake = Instant::now() + target_interval;
        sleep(target_interval);
        let actual_wake = Instant::now();

        let lateness = actual_wake.saturating_duration_since(expected_wake);
        iteration += 1;

        if lateness > Duration::from_millis(2) {
            println!(
                "[latency_probe] [OUTLIER] iter={:<6} PID={} lateness={:>6.2}ms (expected {:?}, actual wait {:?})",
                iteration,
                pid,
                lateness.as_secs_f64() * 1000.0,
                target_interval,
                target_interval + lateness
            );
        }
    }
}
