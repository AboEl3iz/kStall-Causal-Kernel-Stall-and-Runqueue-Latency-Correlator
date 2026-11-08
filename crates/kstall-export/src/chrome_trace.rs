// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use anyhow::Result;
use kstall_core::model::Episode;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;

/// Chrome Trace event structure.
#[derive(Debug, Serialize)]
struct TraceEvent {
    name: String,
    cat: String,
    ph: &'static str,
    ts: f64, // microseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    dur: Option<f64>, // microseconds
    pid: u32,
    tid: u32,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    args: HashMap<String, serde_json::Value>,
}

/// Chrome Trace container format.
#[derive(Debug, Serialize)]
pub struct ChromeTrace {
    trace_events: Vec<TraceEvent>,
}

impl ChromeTrace {
    pub fn from_episodes(episodes: &[Episode]) -> Self {
        let mut events = Vec::new();

        // Track seen PIDs and TIDs for metadata naming
        let mut named_pids = std::collections::HashSet::new();
        let mut named_tids = std::collections::HashSet::new();

        for ep in episodes {
            // Process name metadata event
            if named_pids.insert(ep.tgid) {
                let mut args = HashMap::new();
                args.insert(
                    "name".to_string(),
                    serde_json::Value::String(ep.comm.clone()),
                );
                events.push(TraceEvent {
                    name: "process_name".to_string(),
                    cat: "__metadata".to_string(),
                    ph: "M",
                    ts: 0.0,
                    dur: None,
                    pid: ep.tgid,
                    tid: 0,
                    args,
                });
            }

            // Thread name metadata event
            if named_tids.insert((ep.tgid, ep.tid)) {
                let mut args = HashMap::new();
                args.insert(
                    "name".to_string(),
                    serde_json::Value::String(format!("{} (TID {})", ep.comm, ep.tid)),
                );
                events.push(TraceEvent {
                    name: "thread_name".to_string(),
                    cat: "__metadata".to_string(),
                    ph: "M",
                    ts: 0.0,
                    dur: None,
                    pid: ep.tgid,
                    tid: ep.tid,
                    args,
                });
            }

            // Emit segments
            for seg in &ep.segments {
                let mut args = HashMap::new();
                args.insert("cpu".to_string(), serde_json::Value::from(seg.cpu));
                if let Some(culprit) = &seg.culprit {
                    args.insert(
                        "culprit_comm".to_string(),
                        serde_json::Value::String(culprit.comm.clone()),
                    );
                    args.insert(
                        "culprit_tid".to_string(),
                        serde_json::Value::from(culprit.tid),
                    );
                    args.insert(
                        "culprit_tgid".to_string(),
                        serde_json::Value::from(culprit.tgid),
                    );
                }

                events.push(TraceEvent {
                    name: seg.cause.as_str().to_string(),
                    cat: "stall".to_string(),
                    ph: "X",
                    ts: seg.start_ns as f64 / 1000.0,
                    dur: Some(seg.duration_ns as f64 / 1000.0),
                    pid: ep.tgid,
                    tid: ep.tid,
                    args,
                });
            }
        }

        Self {
            trace_events: events,
        }
    }

    /// Write Chrome Trace JSON to a writer.
    pub fn write_to<W: Write>(&self, mut writer: W) -> Result<()> {
        serde_json::to_writer_pretty(&mut writer, &self)?;
        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kstall_core::model::{CausalSegment, CauseKind, Culprit};

    #[test]
    fn test_chrome_trace_generation() {
        let ep = Episode {
            id: 1,
            tid: 101,
            tgid: 100,
            comm: "redis-server".to_string(),
            cgroup_id: 50,
            cgroup_path: Some("/docker/redis".to_string()),
            container_id: Some("cid123".to_string()),
            pod_name: None,
            namespace: None,
            start_ns: 2_000_000_000,
            end_ns: 2_010_000_000,
            total_delay_ns: 10_000_000,
            segments: vec![CausalSegment {
                cause: CauseKind::RunqueueWait,
                start_ns: 2_000_000_000,
                end_ns: 2_010_000_000,
                duration_ns: 10_000_000,
                cpu: 2,
                culprit: Some(Culprit {
                    tid: 202,
                    tgid: 200,
                    comm: "kswapd0".to_string(),
                    cgroup_id: 0,
                    stolen_ns: 10_000_000,
                    flags: 0,
                }),
            }],
            culprits: vec![Culprit {
                tid: 202,
                tgid: 200,
                comm: "kswapd0".to_string(),
                cgroup_id: 0,
                stolen_ns: 10_000_000,
                flags: 0,
            }],
        };

        let trace = ChromeTrace::from_episodes(&[ep]);
        let mut buf = Vec::new();
        trace.write_to(&mut buf).expect("write trace");

        let json_str = String::from_utf8(buf).expect("utf8");
        let val: serde_json::Value = serde_json::from_str(&json_str).expect("parse json");
        let events = val["trace_events"].as_array().expect("trace_events array");

        // Should have process metadata, thread metadata, and 1 segment event
        assert!(events.iter().any(|e| e["name"] == "process_name"));
        assert!(events.iter().any(|e| e["name"] == "thread_name"));
        let seg_event = events
            .iter()
            .find(|e| e["name"] == "runqueue_wait")
            .expect("runqueue_wait event");
        assert_eq!(seg_event["ph"], "X");
        assert_eq!(seg_event["ts"], 2_000_000.0); // us
        assert_eq!(seg_event["dur"], 10_000.0); // us
        assert_eq!(seg_event["args"]["culprit_comm"], "kswapd0");
    }
}
