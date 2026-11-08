// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use kstall_core::model::{CausalSegment, CauseKind, Culprit, Episode};
use kstall_export::{write_episode_json_line, ChromeTrace, SummaryCsv};
use std::io::Cursor;

fn create_sample_episodes() -> Vec<Episode> {
    vec![
        Episode {
            id: 1,
            tid: 1001,
            tgid: 1000,
            comm: "kafka-broker".to_string(),
            cgroup_id: 10,
            cgroup_path: Some("/kubepods.slice/pod_kafka/kafka".to_string()),
            container_id: Some("container_k1".to_string()),
            pod_name: Some("kafka-0".to_string()),
            namespace: Some("streaming".to_string()),
            start_ns: 10_000_000_000,
            end_ns: 10_025_000_000,
            total_delay_ns: 25_000_000,
            segments: vec![
                CausalSegment {
                    cause: CauseKind::RunqueueWait,
                    start_ns: 10_000_000_000,
                    end_ns: 10_015_000_000,
                    duration_ns: 15_000_000,
                    cpu: 3,
                    culprit: Some(Culprit {
                        tid: 888,
                        tgid: 888,
                        comm: "stress-ng-cpu".to_string(),
                        cgroup_id: 99,
                        stolen_ns: 15_000_000,
                        flags: 0,
                    }),
                },
                CausalSegment {
                    cause: CauseKind::DirectReclaim,
                    start_ns: 10_015_000_000,
                    end_ns: 10_025_000_000,
                    duration_ns: 10_000_000,
                    cpu: 3,
                    culprit: None,
                },
            ],
            culprits: vec![Culprit {
                tid: 888,
                tgid: 888,
                comm: "stress-ng-cpu".to_string(),
                cgroup_id: 99,
                stolen_ns: 15_000_000,
                flags: 0,
            }],
        },
        Episode {
            id: 2,
            tid: 2002,
            tgid: 2000,
            comm: "postgres".to_string(),
            cgroup_id: 20,
            cgroup_path: Some("/system.slice/postgresql.service".to_string()),
            container_id: None,
            pod_name: None,
            namespace: None,
            start_ns: 20_000_000_000,
            end_ns: 20_040_000_000,
            total_delay_ns: 40_000_000,
            segments: vec![CausalSegment {
                cause: CauseKind::Throttled,
                start_ns: 20_000_000_000,
                end_ns: 20_040_000_000,
                duration_ns: 40_000_000,
                cpu: 0,
                culprit: None,
            }],
            culprits: Vec::new(),
        },
    ]
}

#[test]
fn test_json_lines_record_and_replay_roundtrip() {
    let episodes = create_sample_episodes();
    let mut buffer = Vec::new();

    // 1. Record episodes to JSON lines
    for ep in &episodes {
        write_episode_json_line(&mut buffer, ep).expect("write json line");
    }

    // 2. Replay episodes by parsing JSON lines
    let cursor = Cursor::new(buffer);
    let mut replayed = Vec::new();
    for line in std::io::BufRead::lines(cursor) {
        let line = line.expect("read line");
        let ep: Episode = serde_json::from_str(&line).expect("parse episode");
        replayed.push(ep);
    }

    assert_eq!(replayed.len(), 2);
    assert_eq!(replayed[0].comm, "kafka-broker");
    assert_eq!(replayed[0].total_delay_ns, 25_000_000);
    assert_eq!(replayed[0].segments.len(), 2);
    assert_eq!(replayed[1].comm, "postgres");
    assert_eq!(replayed[1].total_delay_ns, 40_000_000);
}

#[test]
fn test_chrome_trace_export_structure() {
    let episodes = create_sample_episodes();
    let trace = ChromeTrace::from_episodes(&episodes);

    let mut buf = Vec::new();
    trace.write_to(&mut buf).expect("write chrome trace");

    let parsed: serde_json::Value = serde_json::from_slice(&buf).expect("parse trace json");
    let events = parsed["trace_events"]
        .as_array()
        .expect("trace_events list");

    // Must have metadata tracks and segment events
    assert!(events
        .iter()
        .any(|e| e["name"] == "process_name" && e["args"]["name"] == "kafka-broker"));
    assert!(events
        .iter()
        .any(|e| e["name"] == "process_name" && e["args"]["name"] == "postgres"));
    assert!(events
        .iter()
        .any(|e| e["name"] == "runqueue_wait" && e["dur"] == 15000.0));
    assert!(events
        .iter()
        .any(|e| e["name"] == "direct_reclaim" && e["dur"] == 10000.0));
    assert!(events
        .iter()
        .any(|e| e["name"] == "cfs_throttled" && e["dur"] == 40000.0));
}

#[test]
fn test_csv_detailed_and_aggregated_export() {
    let episodes = create_sample_episodes();

    // Test detailed CSV
    let mut detailed_buf = Vec::new();
    SummaryCsv::write_episodes(&mut detailed_buf, &episodes).expect("write detailed csv");
    let detailed_str = String::from_utf8(detailed_buf).expect("utf8 detailed");

    assert!(detailed_str.starts_with("timestamp_s,comm,tgid,tid,total_delay_ms"));
    assert!(detailed_str.contains("\"kafka-broker\""));
    assert!(detailed_str.contains("\"postgres\""));
    assert!(detailed_str.contains("stress-ng-cpu"));

    // Test aggregated CSV
    let mut agg_buf = Vec::new();
    SummaryCsv::write_aggregated_summary(&mut agg_buf, &episodes).expect("write agg csv");
    let agg_str = String::from_utf8(agg_buf).expect("utf8 agg");

    assert!(agg_str.starts_with("comm,tgid,episodes,total_delay_ms"));
    assert!(agg_str.contains("\"kafka-broker\""));
    assert!(agg_str.contains("\"postgres\""));
}
