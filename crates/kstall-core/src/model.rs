// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use serde::{Deserialize, Serialize};

/// Type of kernel stall cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CauseKind {
    RunqueueWait,
    PreemptedWait,
    DirectReclaim,
    Compaction,
    #[serde(rename = "cfs_throttled", alias = "throttled")]
    Throttled,
    Unknown,
}

impl CauseKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RunqueueWait => "runqueue_wait",
            Self::PreemptedWait => "preempted_wait",
            Self::DirectReclaim => "direct_reclaim",
            Self::Compaction => "compaction",
            Self::Throttled => "cfs_throttled",
            Self::Unknown => "unknown",
        }
    }
}

/// Information about an interfering culprit task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Culprit {
    pub tid: u32,
    pub tgid: u32,
    pub comm: String,
    pub cgroup_id: u64,
    pub stolen_ns: u64,
    pub flags: u32,
}

/// An individual kernel event record captured from eBPF.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    pub abi_version: u32,
    pub cause: CauseKind,
    pub ts_start_ns: u64,
    pub ts_end_ns: u64,
    pub duration_ns: u64,
    pub tid: u32,
    pub tgid: u32,
    pub cpu_start: u32,
    pub cpu_end: u32,
    pub cgroup_id: u64,
    pub comm: String,
    pub culprits: Vec<Culprit>,
    pub flags: u32,
    pub kstack_id: i32,
}

/// A contiguous slice in a thread's causal timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalSegment {
    pub cause: CauseKind,
    pub start_ns: u64,
    pub end_ns: u64,
    pub duration_ns: u64,
    pub cpu: u32,
    pub culprit: Option<Culprit>,
}

/// An assembled per-thread stall incident.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Episode {
    pub id: u64,
    pub tid: u32,
    pub tgid: u32,
    pub comm: String,
    pub cgroup_id: u64,
    pub cgroup_path: Option<String>,
    pub container_id: Option<String>,
    pub pod_name: Option<String>,
    pub namespace: Option<String>,
    pub start_ns: u64,
    pub end_ns: u64,
    pub total_delay_ns: u64,
    pub segments: Vec<CausalSegment>,
    pub culprits: Vec<Culprit>,
}

impl Episode {
    /// Compute latency breakdown across causes for this episode.
    pub fn breakdown(&self) -> LatencyBreakdown {
        let mut runq_ns = 0;
        let mut preempt_ns = 0;
        let mut reclaim_ns = 0;
        let mut compact_ns = 0;
        let mut throttle_ns = 0;

        for seg in &self.segments {
            match seg.cause {
                CauseKind::RunqueueWait => runq_ns += seg.duration_ns,
                CauseKind::PreemptedWait => preempt_ns += seg.duration_ns,
                CauseKind::DirectReclaim => reclaim_ns += seg.duration_ns,
                CauseKind::Compaction => compact_ns += seg.duration_ns,
                CauseKind::Throttled => throttle_ns += seg.duration_ns,
                CauseKind::Unknown => {}
            }
        }

        LatencyBreakdown {
            total_ns: self.total_delay_ns,
            runq_ns,
            preempt_ns,
            reclaim_ns,
            compact_ns,
            throttle_ns,
        }
    }
}

/// Breakdown of stall time by subsystem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyBreakdown {
    pub total_ns: u64,
    pub runq_ns: u64,
    pub preempt_ns: u64,
    pub reclaim_ns: u64,
    pub compact_ns: u64,
    pub throttle_ns: u64,
}
