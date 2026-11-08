// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use crate::model::Episode;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Quantiles computed for stall latencies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyQuantiles {
    pub count: usize,
    pub min_ns: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}

/// An entry in the victim x culprit interference matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterferenceEntry {
    pub victim_comm: String,
    pub victim_tgid: u32,
    pub culprit_comm: String,
    pub culprit_tgid: u32,
    pub total_stolen_ns: u64,
    pub incident_count: u64,
}

/// Rolling statistical window for episodes and interference tracking.
pub struct EpisodeWindow {
    window_duration_ns: u64,
    episodes: VecDeque<Episode>,
}

impl EpisodeWindow {
    pub fn new(window_duration_ns: u64) -> Self {
        Self {
            window_duration_ns,
            episodes: VecDeque::new(),
        }
    }

    /// Add an episode and prune episodes outside the rolling window.
    pub fn push(&mut self, episode: Episode) {
        let latest_ts = episode.end_ns;
        self.episodes.push_back(episode);

        let cutoff = latest_ts.saturating_sub(self.window_duration_ns);
        while let Some(front) = self.episodes.front() {
            if front.end_ns < cutoff {
                self.episodes.pop_front();
            } else {
                break;
            }
        }
    }

    /// Get reference to all episodes in the current window.
    pub fn episodes(&self) -> &VecDeque<Episode> {
        &self.episodes
    }

    /// Compute latency percentiles across all episodes in the window.
    pub fn quantiles(&self) -> LatencyQuantiles {
        if self.episodes.is_empty() {
            return LatencyQuantiles::default();
        }

        let mut latencies: Vec<u64> = self.episodes.iter().map(|e| e.total_delay_ns).collect();
        latencies.sort_unstable();

        let count = latencies.len();
        let p50_idx = (count as f64 * 0.50).floor() as usize;
        let p95_idx = (count as f64 * 0.95).floor() as usize;
        let p99_idx = (count as f64 * 0.99).floor() as usize;

        LatencyQuantiles {
            count,
            min_ns: latencies[0],
            p50_ns: latencies[p50_idx.min(count - 1)],
            p95_ns: latencies[p95_idx.min(count - 1)],
            p99_ns: latencies[p99_idx.min(count - 1)],
            max_ns: latencies[count - 1],
        }
    }

    /// Build the interference matrix (victim x culprit) aggregated over the window.
    pub fn interference_matrix(&self) -> Vec<InterferenceEntry> {
        let mut map: HashMap<(String, u32, String, u32), (u64, u64)> = HashMap::new();

        for ep in &self.episodes {
            for culprit in &ep.culprits {
                let key = (ep.comm.clone(), ep.tgid, culprit.comm.clone(), culprit.tgid);
                let entry = map.entry(key).or_insert((0, 0));
                entry.0 += ep.total_delay_ns;
                entry.1 += 1;
            }
        }

        let mut result: Vec<InterferenceEntry> = map
            .into_iter()
            .map(
                |((v_comm, v_tgid, c_comm, c_tgid), (stolen, count))| InterferenceEntry {
                    victim_comm: v_comm,
                    victim_tgid: v_tgid,
                    culprit_comm: c_comm,
                    culprit_tgid: c_tgid,
                    total_stolen_ns: stolen,
                    incident_count: count,
                },
            )
            .collect();

        result.sort_by_key(|a| std::cmp::Reverse(a.total_stolen_ns));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Culprit;

    #[test]
    fn test_quantiles_and_window() {
        let mut win = EpisodeWindow::new(10_000_000);
        for delay in [1000, 2000, 3000, 4000, 5000] {
            win.push(Episode {
                id: delay,
                tid: 100,
                tgid: 100,
                comm: "victim".to_string(),
                cgroup_id: 1,
                cgroup_path: None,
                container_id: None,
                pod_name: None,
                namespace: None,
                start_ns: 0,
                end_ns: delay,
                total_delay_ns: delay,
                segments: Vec::new(),
                culprits: vec![Culprit {
                    tid: 200,
                    tgid: 200,
                    comm: "culprit".to_string(),
                    cgroup_id: 2,
                    stolen_ns: delay,
                    flags: 0,
                }],
            });
        }

        let q = win.quantiles();
        assert_eq!(q.count, 5);
        assert_eq!(q.min_ns, 1000);
        assert_eq!(q.max_ns, 5000);

        let matrix = win.interference_matrix();
        assert_eq!(matrix.len(), 1);
        assert_eq!(matrix[0].total_stolen_ns, 15000);
        assert_eq!(matrix[0].incident_count, 5);
    }
}
