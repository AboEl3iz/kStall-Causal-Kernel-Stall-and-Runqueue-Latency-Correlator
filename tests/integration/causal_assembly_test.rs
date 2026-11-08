// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers
//! Integration Test: Causal Assembly & Stall Correlation (Phase 2)
//!
//! Validates:
//! 1. Runqueue + CFS Throttling overlap resolution
//! 2. Direct Reclaim and Compaction attribution
//! 3. Interference Matrix stolen CPU time calculation

use kstall_core::assembler::{AssemblerConfig, CausalAssembler};
use kstall_core::model::{CauseKind, Culprit, EventRecord};
use kstall_core::stats::EpisodeWindow;

#[test]
fn test_complex_causal_episode_assembly() {
    let mut assembler = CausalAssembler::new(AssemblerConfig::default());

    // 1. Cgroup 100 enters throttled state from t=20ms to t=35ms
    assembler.record_throttle(100, 20_000_000, 35_000_000);

    // 2. Thread 1001 in cgroup 100 enters runqueue at t=10ms and runs at t=40ms (30ms total wait)
    let runq_ev = EventRecord {
        abi_version: 1,
        cause: CauseKind::RunqueueWait,
        ts_start_ns: 10_000_000,
        ts_end_ns: 40_000_000,
        duration_ns: 30_000_000,
        tid: 1001,
        tgid: 1000,
        cpu_start: 2,
        cpu_end: 2,
        cgroup_id: 100,
        comm: "app-worker".to_string(),
        culprits: vec![Culprit {
            tid: 9000,
            tgid: 9000,
            comm: "cpu-hog".to_string(),
            cgroup_id: 200,
            stolen_ns: 30_000_000,
            flags: 0,
        }],
        flags: 0,
        kstack_id: -1,
    };

    let ep = assembler.ingest(runq_ev).expect("Should assemble episode");
    assert_eq!(ep.tid, 1001);
    assert_eq!(ep.total_delay_ns, 30_000_000);

    // Verify segments:
    // [10ms, 20ms]: RunqueueWait (10ms)
    // [20ms, 35ms]: Throttled (15ms)
    // [35ms, 40ms]: RunqueueWait (5ms)
    assert_eq!(ep.segments.len(), 3);
    assert_eq!(ep.segments[0].cause, CauseKind::RunqueueWait);
    assert_eq!(ep.segments[0].duration_ns, 10_000_000);

    assert_eq!(ep.segments[1].cause, CauseKind::Throttled);
    assert_eq!(ep.segments[1].duration_ns, 15_000_000);

    assert_eq!(ep.segments[2].cause, CauseKind::RunqueueWait);
    assert_eq!(ep.segments[2].duration_ns, 5_000_000);

    let breakdown = ep.breakdown();
    assert_eq!(breakdown.throttle_ns, 15_000_000);
    assert_eq!(breakdown.runq_ns, 15_000_000);
}

#[test]
fn test_memory_reclaim_and_interference_matrix() {
    let mut win = EpisodeWindow::new(60_000_000_000); // 1 minute window

    let mut assembler = CausalAssembler::new(AssemblerConfig::default());

    // Reclaim event: 8ms direct reclaim stall
    let reclaim_ev = EventRecord {
        abi_version: 1,
        cause: CauseKind::DirectReclaim,
        ts_start_ns: 50_000_000,
        ts_end_ns: 58_000_000,
        duration_ns: 8_000_000,
        tid: 1002,
        tgid: 1000,
        cpu_start: 3,
        cpu_end: 3,
        cgroup_id: 100,
        comm: "allocator".to_string(),
        culprits: vec![Culprit {
            tid: 50,
            tgid: 50,
            comm: "kswapd0".to_string(),
            cgroup_id: 0,
            stolen_ns: 8_000_000,
            flags: 1 << 4, // KS_FLAG_KERNEL_THREAD
        }],
        flags: 0,
        kstack_id: -1,
    };

    if let Some(ep) = assembler.ingest(reclaim_ev) {
        win.push(ep);
    }

    assert_eq!(win.episodes().len(), 1);
    let matrix = win.interference_matrix();
    assert_eq!(matrix.len(), 1);
    assert_eq!(matrix[0].victim_comm, "allocator");
    assert_eq!(matrix[0].culprit_comm, "kswapd0");
    assert_eq!(matrix[0].total_stolen_ns, 8_000_000);
}
