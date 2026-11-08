// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use anyhow::Result;
use kstall_core::model::Episode;
use std::collections::HashMap;
use std::io::Write;

/// CSV Exporter for kStall episodes and summary metrics.
pub struct SummaryCsv;

impl SummaryCsv {
    /// Export each episode as a CSV row with latency breakdown and primary culprit.
    pub fn write_episodes<W: Write>(writer: &mut W, episodes: &[Episode]) -> Result<()> {
        writeln!(
            writer,
            "timestamp_s,comm,tgid,tid,total_delay_ms,runq_ms,preempt_ms,reclaim_ms,compact_ms,throttle_ms,primary_cause,primary_culprit,culprit_tgid,cgroup_path"
        )?;

        for ep in episodes {
            let bd = ep.breakdown();
            let primary_cause = ep
                .segments
                .first()
                .map(|s| s.cause.as_str())
                .unwrap_or("unknown");
            let (culprit_comm, culprit_tgid) = ep
                .culprits
                .first()
                .map(|c| (c.comm.as_str(), c.tgid.to_string()))
                .unwrap_or(("-", "-".to_string()));
            let cgroup = ep.cgroup_path.as_deref().unwrap_or("-");

            writeln!(
                writer,
                "{:.6},\"{}\",{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},\"{}\",\"{}\",{},\"{}\"",
                ep.start_ns as f64 / 1_000_000_000.0,
                Self::escape(&ep.comm),
                ep.tgid,
                ep.tid,
                ep.total_delay_ns as f64 / 1_000_000.0,
                bd.runq_ns as f64 / 1_000_000.0,
                bd.preempt_ns as f64 / 1_000_000.0,
                bd.reclaim_ns as f64 / 1_000_000.0,
                bd.compact_ns as f64 / 1_000_000.0,
                bd.throttle_ns as f64 / 1_000_000.0,
                primary_cause,
                Self::escape(culprit_comm),
                culprit_tgid,
                Self::escape(cgroup),
            )?;
        }

        writer.flush()?;
        Ok(())
    }

    /// Export aggregated per-process breakdown summary as CSV.
    pub fn write_aggregated_summary<W: Write>(writer: &mut W, episodes: &[Episode]) -> Result<()> {
        #[derive(Default)]
        struct Agg {
            count: u64,
            total_delay_ns: u64,
            runq_ns: u64,
            preempt_ns: u64,
            reclaim_ns: u64,
            compact_ns: u64,
            throttle_ns: u64,
            top_culprit: HashMap<String, u64>,
        }

        let mut map: HashMap<(String, u32), Agg> = HashMap::new();

        for ep in episodes {
            let entry = map.entry((ep.comm.clone(), ep.tgid)).or_default();
            entry.count += 1;
            entry.total_delay_ns += ep.total_delay_ns;

            let bd = ep.breakdown();
            entry.runq_ns += bd.runq_ns;
            entry.preempt_ns += bd.preempt_ns;
            entry.reclaim_ns += bd.reclaim_ns;
            entry.compact_ns += bd.compact_ns;
            entry.throttle_ns += bd.throttle_ns;

            if let Some(c) = ep.culprits.first() {
                *entry.top_culprit.entry(c.comm.clone()).or_insert(0) += ep.total_delay_ns;
            }
        }

        writeln!(
            writer,
            "comm,tgid,episodes,total_delay_ms,runq_ms,preempt_ms,reclaim_ms,compact_ms,throttle_ms,top_culprit"
        )?;

        let mut rows: Vec<_> = map.into_iter().collect();
        rows.sort_by_key(|a| std::cmp::Reverse(a.1.total_delay_ns));

        for ((comm, tgid), agg) in rows {
            let top_c = agg
                .top_culprit
                .into_iter()
                .max_by_key(|(_, stolen)| *stolen)
                .map(|(c, _)| c)
                .unwrap_or_else(|| "-".to_string());

            writeln!(
                writer,
                "\"{}\",{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},\"{}\"",
                Self::escape(&comm),
                tgid,
                agg.count,
                agg.total_delay_ns as f64 / 1_000_000.0,
                agg.runq_ns as f64 / 1_000_000.0,
                agg.preempt_ns as f64 / 1_000_000.0,
                agg.reclaim_ns as f64 / 1_000_000.0,
                agg.compact_ns as f64 / 1_000_000.0,
                agg.throttle_ns as f64 / 1_000_000.0,
                Self::escape(&top_c),
            )?;
        }

        writer.flush()?;
        Ok(())
    }

    fn escape(s: &str) -> String {
        s.replace('"', "\"\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kstall_core::model::{CausalSegment, CauseKind, Culprit};

    #[test]
    fn test_csv_export() {
        let ep = Episode {
            id: 1,
            tid: 42,
            tgid: 40,
            comm: "worker_thread".to_string(),
            cgroup_id: 100,
            cgroup_path: Some("/kubepods/pod_abc".to_string()),
            container_id: Some("container_123".to_string()),
            pod_name: Some("pod_abc".to_string()),
            namespace: Some("default".to_string()),
            start_ns: 1_000_000_000,
            end_ns: 1_015_000_000,
            total_delay_ns: 15_000_000,
            segments: vec![CausalSegment {
                cause: CauseKind::RunqueueWait,
                start_ns: 1_000_000_000,
                end_ns: 1_015_000_000,
                duration_ns: 15_000_000,
                cpu: 1,
                culprit: Some(Culprit {
                    tid: 99,
                    tgid: 90,
                    comm: "hog".to_string(),
                    cgroup_id: 200,
                    stolen_ns: 15_000_000,
                    flags: 0,
                }),
            }],
            culprits: vec![Culprit {
                tid: 99,
                tgid: 90,
                comm: "hog".to_string(),
                cgroup_id: 200,
                stolen_ns: 15_000_000,
                flags: 0,
            }],
        };

        let mut buf = Vec::new();
        SummaryCsv::write_episodes(&mut buf, std::slice::from_ref(&ep)).expect("write episodes");
        let csv_str = String::from_utf8(buf).expect("utf8");
        assert!(csv_str.contains("worker_thread"));
        assert!(csv_str.contains("hog"));
        assert!(csv_str.contains("15.000"));

        let mut sum_buf = Vec::new();
        SummaryCsv::write_aggregated_summary(&mut sum_buf, &[ep]).expect("write summary");
        let sum_str = String::from_utf8(sum_buf).expect("utf8");
        assert!(sum_str.contains("worker_thread"));
        assert!(sum_str.contains("15.000"));
        assert!(sum_str.contains("hog"));
    }
}
