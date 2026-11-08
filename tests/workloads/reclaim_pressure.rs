// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers
//! FI-4: Direct Memory Reclaim Pressure Workload (PRD §16.5)
//!
//! Rapidly allocates and dirties memory chunks to trigger direct page-cache reclaim
//! and compaction in memory-constrained environments or cgroups.

use std::hint::black_box;
use std::time::{Duration, Instant};

fn main() {
    let pid = std::process::id();
    println!(
        "[reclaim_pressure] Starting memory pressure workload (PID: {})...",
        pid
    );

    let chunk_size = 32 * 1024 * 1024; // 32 MB per chunk
    let iterations = 20;

    for i in 1..=iterations {
        let start = Instant::now();
        // Allocate and dirty memory
        let mut buffer: Vec<u8> = vec![0u8; chunk_size];
        for (idx, byte) in buffer.iter_mut().enumerate() {
            *byte = (idx % 255) as u8;
        }

        black_box(&buffer);
        let elapsed = start.elapsed();

        if elapsed > Duration::from_millis(5) {
            println!(
                "[reclaim_pressure] iter {:<3} allocated 32MB in {:>6.2}ms (potential reclaim stall)",
                i,
                elapsed.as_secs_f64() * 1000.0
            );
        }

        std::thread::sleep(Duration::from_millis(50));
    }

    println!("[reclaim_pressure] Finished allocations.");
}
