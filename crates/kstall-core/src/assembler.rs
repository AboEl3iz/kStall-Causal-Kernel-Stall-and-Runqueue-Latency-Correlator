// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use crate::model::{CausalSegment, CauseKind, Episode, EventRecord};
use std::collections::{HashMap, VecDeque};

/// Configuration for the causal episode assembler.
#[derive(Debug, Clone)]
pub struct AssemblerConfig {
    /// Window timeout (ns) after which an unfinished episode is materialized.
    pub orphan_timeout_ns: u64,
}

impl Default for AssemblerConfig {
    fn default() -> Self {
        Self {
            orphan_timeout_ns: 5_000_000_000, /* 5 seconds */
        }
    }
}

/// Assembler that reconstructs causal stall episodes per thread from raw BPF events.
pub struct CausalAssembler {
    config: AssemblerConfig,
    next_episode_id: u64,
    active_episodes: HashMap<u32, Episode>,
    // Track recent cgroup throttle intervals: cgroup_id -> VecDeque<(start_ns, end_ns)>
    cgroup_throttles: HashMap<u64, VecDeque<(u64, u64)>>,
}

impl CausalAssembler {
    pub fn new(config: AssemblerConfig) -> Self {
        Self {
            config,
            next_episode_id: 1,
            active_episodes: HashMap::new(),
            cgroup_throttles: HashMap::new(),
        }
    }

    /// Record a cgroup-level throttle interval.
    pub fn record_throttle(&mut self, cgroup_id: u64, start_ns: u64, end_ns: u64) {
        if cgroup_id == 0 || end_ns <= start_ns {
            return;
        }

        let entry = self.cgroup_throttles.entry(cgroup_id).or_default();
        entry.push_back((start_ns, end_ns));

        // Keep last 128 throttle intervals per cgroup
        while entry.len() > 128 {
            entry.pop_front();
        }
    }

    /// Ingest a raw event record and assemble it into an episode.
    pub fn ingest(&mut self, ev: EventRecord) -> Option<Episode> {
        // Handle cgroup throttle events (which have tid == 0)
        if ev.cause == CauseKind::Throttled {
            self.record_throttle(ev.cgroup_id, ev.ts_start_ns, ev.ts_end_ns);
            return None;
        }

        let tid = ev.tid;
        let mut episode = self.active_episodes.remove(&tid).unwrap_or_else(|| {
            let id = self.next_episode_id;
            self.next_episode_id += 1;
            Episode {
                id,
                tid: ev.tid,
                tgid: ev.tgid,
                comm: ev.comm.clone(),
                cgroup_id: ev.cgroup_id,
                cgroup_path: None,
                container_id: None,
                pod_name: None,
                namespace: None,
                start_ns: ev.ts_start_ns,
                end_ns: ev.ts_end_ns,
                total_delay_ns: 0,
                segments: Vec::new(),
                culprits: Vec::new(),
            }
        });

        // Update episode boundaries
        if ev.ts_start_ns < episode.start_ns {
            episode.start_ns = ev.ts_start_ns;
        }
        if ev.ts_end_ns > episode.end_ns {
            episode.end_ns = ev.ts_end_ns;
        }

        let primary_culprit = ev.culprits.first().cloned();

        // Create base causal segment
        let segment = CausalSegment {
            cause: ev.cause,
            start_ns: ev.ts_start_ns,
            end_ns: ev.ts_end_ns,
            duration_ns: ev.duration_ns,
            cpu: ev.cpu_end,
            culprit: primary_culprit,
        };
        episode.segments.push(segment);

        // Merge culprits
        for c in ev.culprits {
            if !episode
                .culprits
                .iter()
                .any(|existing| existing.tid == c.tid && existing.comm == c.comm)
            {
                episode.culprits.push(c);
            }
        }

        // Apply cgroup throttling correlation if the event overlaps with any throttle intervals
        if let Some(throttles) = self.cgroup_throttles.get(&ev.cgroup_id) {
            for &(t_start, t_end) in throttles {
                let overlap_start = t_start.max(ev.ts_start_ns);
                let overlap_end = t_end.min(ev.ts_end_ns);

                if overlap_end > overlap_start {
                    // Inject a throttled segment
                    episode.segments.push(CausalSegment {
                        cause: CauseKind::Throttled,
                        start_ns: overlap_start,
                        end_ns: overlap_end,
                        duration_ns: overlap_end - overlap_start,
                        cpu: ev.cpu_end,
                        culprit: None,
                    });
                }
            }
        }

        // Perform causal overlap resolution and merge segments
        episode.segments = Self::resolve_overlaps(&episode.segments);

        // Recalculate total delay duration across resolved segments
        episode.total_delay_ns = episode.segments.iter().map(|s| s.duration_ns).sum();

        // For runqueue and preemption events, the switch-in closes this episode
        Some(episode)
    }

    /// Resolve overlapping segments according to causal priority:
    /// Throttled (4) > Reclaim/Compaction (3) > PreemptedWait (2) > RunqueueWait (1)
    fn resolve_overlaps(segments: &[CausalSegment]) -> Vec<CausalSegment> {
        if segments.is_empty() {
            return Vec::new();
        }

        if segments.len() == 1 {
            return segments.to_vec();
        }

        // Collect all discrete boundary timestamps
        let mut boundaries = Vec::new();
        for s in segments {
            boundaries.push(s.start_ns);
            boundaries.push(s.end_ns);
        }
        boundaries.sort_unstable();
        boundaries.dedup();

        let mut resolved = Vec::new();

        for i in 0..boundaries.len().saturating_sub(1) {
            let t0 = boundaries[i];
            let t1 = boundaries[i + 1];

            if t1 <= t0 {
                continue;
            }

            // Find candidates covering [t0, t1]
            let candidates: Vec<&CausalSegment> = segments
                .iter()
                .filter(|s| s.start_ns <= t0 && s.end_ns >= t1)
                .collect();

            if let Some(winner) = candidates.iter().max_by_key(|s| match s.cause {
                CauseKind::Throttled => 4,
                CauseKind::DirectReclaim | CauseKind::Compaction => 3,
                CauseKind::PreemptedWait => 2,
                CauseKind::RunqueueWait => 1,
                CauseKind::Unknown => 0,
            }) {
                // If adjacent to previous segment with same cause and culprit, extend it
                if let Some(last) = resolved.last_mut() {
                    let last: &mut CausalSegment = last;
                    if last.cause == winner.cause
                        && last.culprit == winner.culprit
                        && last.end_ns == t0
                    {
                        last.end_ns = t1;
                        last.duration_ns = last.end_ns - last.start_ns;
                        continue;
                    }
                }

                resolved.push(CausalSegment {
                    cause: winner.cause,
                    start_ns: t0,
                    end_ns: t1,
                    duration_ns: t1 - t0,
                    cpu: winner.cpu,
                    culprit: winner.culprit.clone(),
                });
            }
        }

        resolved
    }

    /// Evict episodes that have not been updated for longer than `orphan_timeout_ns`.
    pub fn evict_orphans(&mut self, current_time_ns: u64) -> Vec<Episode> {
        let mut evicted = Vec::new();
        let timeout = self.config.orphan_timeout_ns;

        self.active_episodes.retain(|_, ep| {
            if current_time_ns.saturating_sub(ep.end_ns) > timeout {
                evicted.push(ep.clone());
                false
            } else {
                true
            }
        });

        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CauseKind, Culprit};

    #[test]
    fn test_ingest_runq_event() {
        let mut assembler = CausalAssembler::new(AssemblerConfig::default());
        let ev = EventRecord {
            abi_version: 1,
            cause: CauseKind::RunqueueWait,
            ts_start_ns: 100_000,
            ts_end_ns: 105_000,
            duration_ns: 5_000,
            tid: 1001,
            tgid: 1000,
            cpu_start: 0,
            cpu_end: 1,
            cgroup_id: 42,
            comm: "worker-pool".to_string(),
            culprits: vec![Culprit {
                tid: 9999,
                tgid: 9999,
                comm: "stress-ng".to_string(),
                cgroup_id: 1,
                stolen_ns: 5_000,
                flags: 0,
            }],
            flags: 0,
            kstack_id: -1,
        };

        let ep = assembler.ingest(ev).expect("should produce episode");
        assert_eq!(ep.tid, 1001);
        assert_eq!(ep.total_delay_ns, 5_000);
        assert_eq!(ep.segments.len(), 1);
        assert_eq!(ep.segments[0].cause, CauseKind::RunqueueWait);
        assert_eq!(ep.culprits.len(), 1);
        assert_eq!(ep.culprits[0].comm, "stress-ng");
    }

    #[test]
    fn test_cfs_throttle_overlap_resolution() {
        let mut assembler = CausalAssembler::new(AssemblerConfig::default());

        // Pre-record a cgroup throttle interval on cgroup 42: from t=110 to t=125
        assembler.record_throttle(42, 110_000, 125_000);

        // Ingest a runqueue wait event: from t=100 to t=130
        let ev = EventRecord {
            abi_version: 1,
            cause: CauseKind::RunqueueWait,
            ts_start_ns: 100_000,
            ts_end_ns: 130_000,
            duration_ns: 30_000,
            tid: 2002,
            tgid: 2000,
            cpu_start: 1,
            cpu_end: 1,
            cgroup_id: 42,
            comm: "tokio-runtime".to_string(),
            culprits: vec![Culprit {
                tid: 5000,
                tgid: 5000,
                comm: "hog".to_string(),
                cgroup_id: 2,
                stolen_ns: 15_000,
                flags: 0,
            }],
            flags: 0,
            kstack_id: -1,
        };

        let ep = assembler.ingest(ev).expect("should produce episode");
        assert_eq!(ep.tid, 2002);
        assert_eq!(ep.total_delay_ns, 30_000);

        // Verify the 3 resolved segments:
        // 1. [100k - 110k] RunqueueWait (10k)
        // 2. [110k - 125k] Throttled (15k)
        // 3. [125k - 130k] RunqueueWait (5k)
        assert_eq!(ep.segments.len(), 3);
        assert_eq!(ep.segments[0].cause, CauseKind::RunqueueWait);
        assert_eq!(ep.segments[0].duration_ns, 10_000);

        assert_eq!(ep.segments[1].cause, CauseKind::Throttled);
        assert_eq!(ep.segments[1].duration_ns, 15_000);

        assert_eq!(ep.segments[2].cause, CauseKind::RunqueueWait);
        assert_eq!(ep.segments[2].duration_ns, 5_000);

        // Verify breakdown
        let breakdown = ep.breakdown();
        assert_eq!(breakdown.throttle_ns, 15_000);
        assert_eq!(breakdown.runq_ns, 15_000);
        assert_eq!(breakdown.total_ns, 30_000);
    }
}
