// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers
//! FI-1: CPU Starvation Hog Workload (PRD §16.5)
//!
//! Saturates CPU cores to induce runqueue queuing delays on co-scheduled threads.

use std::env;
use std::hint::black_box;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = env::args().collect();
    let duration_secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let num_threads: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);

    println!(
        "[starvation_hog] Starting {} hog threads for {}s (PID: {})...",
        num_threads,
        duration_secs,
        std::process::id()
    );

    let mut handles = Vec::new();
    for i in 0..num_threads {
        let handle = thread::spawn(move || {
            let start = Instant::now();
            let limit = Duration::from_secs(duration_secs);
            let mut acc: u64 = 0;

            while start.elapsed() < limit {
                acc = black_box(acc.wrapping_add(1));
            }
            println!("[starvation_hog] Thread {} finished.", i);
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().unwrap();
    }

    println!("[starvation_hog] Complete.");
}
