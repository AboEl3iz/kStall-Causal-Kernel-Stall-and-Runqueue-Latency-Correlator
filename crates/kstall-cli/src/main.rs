// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

mod doctor;

use anyhow::Result;
use clap::{Parser, Subcommand};
use kstall_bpf::{BpfConfig, BpfManager};
use kstall_core::assembler::{AssemblerConfig, CausalAssembler};
use kstall_core::model::Episode;
use kstall_export::{write_episode_json_line, ChromeTrace, SummaryCsv};
use kstall_meta::{parse_cgroup_metadata, CgroupResolver};
use std::fs::File;
use std::io::{stdout, BufWriter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(
    name = "kstall",
    version = "0.1.0",
    about = "Causal Kernel-Stall & Runqueue Latency Correlator",
    long_about = "kStall captures kernel scheduler and subsystem latency outliers and attributes them to causes and culprits."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Live interactive TUI waterfall and interference matrix
    Top {
        #[arg(short = 'p', long = "pid", help = "Filter by target PID/TGID")]
        pid: Option<u32>,

        #[arg(
            short = 'C',
            long = "containers-only",
            help = "Show stalls from containerized tasks only (Docker/CRI)"
        )]
        containers_only: bool,

        #[arg(long = "container", help = "Filter by container ID or short prefix")]
        container: Option<String>,

        #[arg(
            short = 't',
            long = "threshold",
            default_value = "2ms",
            help = "Runqueue outlier threshold (e.g. 2ms, 500us)"
        )]
        threshold: String,

        #[arg(
            short = 'w',
            long = "window",
            default_value = "5m",
            help = "Rolling time window (e.g. 5m, 1m)"
        )]
        window: String,
    },

    /// Stream stall outlier events directly to console (Text or JSON)
    Watch {
        #[arg(short = 'p', long = "pid", help = "Filter by target PID/TGID")]
        pid: Option<u32>,

        #[arg(
            short = 'C',
            long = "containers-only",
            help = "Capture stalls from containerized tasks only (Docker/CRI)"
        )]
        containers_only: bool,

        #[arg(long = "container", help = "Filter by container ID or short prefix")]
        container: Option<String>,

        #[arg(
            short = 't',
            long = "threshold",
            default_value = "2ms",
            help = "Runqueue outlier threshold"
        )]
        threshold: String,

        #[arg(
            short = 'f',
            long = "format",
            default_value = "text",
            help = "Output format: text | json"
        )]
        format: String,
    },

    /// Record stall episodes to a file (JSON Lines format)
    Record {
        #[arg(short = 'o', long = "output", help = "Output file path")]
        output: PathBuf,

        #[arg(short = 'p', long = "pid", help = "Filter by target PID/TGID")]
        pid: Option<u32>,

        #[arg(
            short = 'C',
            long = "containers-only",
            help = "Record stalls from containerized tasks only (Docker/CRI)"
        )]
        containers_only: bool,

        #[arg(long = "container", help = "Filter by container ID or short prefix")]
        container: Option<String>,

        #[arg(short = 't', long = "threshold", default_value = "2ms")]
        threshold: String,

        #[arg(
            short = 'd',
            long = "duration",
            default_value = "30s",
            help = "Duration to record (e.g. 30s, 5m)"
        )]
        duration: String,
    },

    /// Replay recorded episodes in the interactive TUI
    Replay {
        #[arg(
            short = 'i',
            long = "input",
            help = "Input recording file (JSON lines)"
        )]
        input: PathBuf,

        #[arg(
            short = 'C',
            long = "containers-only",
            help = "Show stalls from containerized tasks only (Docker/CRI)"
        )]
        containers_only: bool,

        #[arg(long = "container", help = "Filter by container ID or short prefix")]
        container: Option<String>,

        #[arg(
            short = 'w',
            long = "window",
            default_value = "1h",
            help = "Rolling time window (e.g. 1h, 30m)"
        )]
        window: String,
    },

    /// Export recorded episodes to Chrome Trace JSON or Summary CSV
    Export {
        #[arg(
            short = 'i',
            long = "input",
            help = "Input recording file (JSON lines)"
        )]
        input: PathBuf,

        #[arg(short = 'o', long = "output", help = "Output file path")]
        output: PathBuf,

        #[arg(
            short = 'f',
            long = "format",
            default_value = "chrome",
            help = "Export format: chrome | csv | csv-summary"
        )]
        format: String,
    },

    /// Diagnose kernel environment, tracepoints, and privileges
    Doctor,

    /// Display version information
    Version,
}

/// Parse human-friendly duration strings like "2ms", "500us", "10s", "5m" into nanoseconds.
fn parse_duration_ns(s: &str) -> u64 {
    let s = s.trim();
    if let Some(val) = s.strip_suffix("ns") {
        val.parse().unwrap_or(2_000_000)
    } else if let Some(val) = s.strip_suffix("us") {
        val.parse::<f64>()
            .map(|v| (v * 1_000.0) as u64)
            .unwrap_or(2_000_000)
    } else if let Some(val) = s.strip_suffix("ms") {
        val.parse::<f64>()
            .map(|v| (v * 1_000_000.0) as u64)
            .unwrap_or(2_000_000)
    } else if let Some(val) = s.strip_suffix('s') {
        val.parse::<f64>()
            .map(|v| (v * 1_000_000_000.0) as u64)
            .unwrap_or(2_000_000)
    } else if let Some(val) = s.strip_suffix('m') {
        val.parse::<f64>()
            .map(|v| (v * 60_000_000_000.0) as u64)
            .unwrap_or(300_000_000_000)
    } else if let Some(val) = s.strip_suffix('h') {
        val.parse::<f64>()
            .map(|v| (v * 3_600_000_000_000.0) as u64)
            .unwrap_or(3_600_000_000_000)
    } else {
        s.parse::<u64>().unwrap_or(2_000_000)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let cli = Cli::parse();

    match cli.command {
        Commands::Doctor => {
            let code = doctor::run_doctor()?;
            std::process::exit(code);
        }

        Commands::Version => {
            println!("kStall v0.1.0 (ABI v1, kernel 5.15+ BTF CO-RE)");
            Ok(())
        }

        Commands::Watch {
            pid,
            threshold,
            format,
            containers_only,
            container,
        } => {
            let threshold_ns = parse_duration_ns(&threshold);
            let bpf_cfg = BpfConfig {
                runq_threshold_ns: threshold_ns,
                reclaim_threshold_ns: 1_000_000,
                throttle_threshold_ns: 1_000_000,
                target_pid: pid.unwrap_or(0),
                target_tgid: pid.unwrap_or(0),
                target_cgroup_id: 0,
            };

            let stop_flag = Arc::new(AtomicBool::new(false));
            let s_flag = stop_flag.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                s_flag.store(true, Ordering::Relaxed);
            });

            let (event_tx, mut event_rx) = mpsc::channel(1024);
            let _mgr = BpfManager::start(bpf_cfg, event_tx, stop_flag.clone())?;

            let mut assembler = CausalAssembler::new(AssemblerConfig::default());
            let mut cgroup_res = CgroupResolver::new();

            let filter_info = if containers_only {
                " [Containers Only]"
            } else if let Some(cid) = &container {
                &format!(" [Container: {}]", cid)
            } else {
                ""
            };

            info!(
                "kStall watching for stalls >= {}ns{} (Ctrl+C to stop)...",
                threshold_ns, filter_info
            );

            let stdout = stdout();
            let mut handle = stdout.lock();

            while !stop_flag.load(Ordering::Relaxed) {
                tokio::select! {
                    Some(ev) = event_rx.recv() => {
                        if let Some(mut ep) = assembler.ingest(ev) {
                            if let Some(path) = cgroup_res.resolve_cgroup_id(ep.cgroup_id) {
                                let meta = parse_cgroup_metadata(&path);
                                ep.cgroup_path = Some(path);
                                ep.container_id = meta.container_id;
                                ep.pod_name = meta.pod_name;
                            }

                            if containers_only && ep.container_id.is_none() {
                                continue;
                            }
                            if let Some(target_cid) = &container {
                                let matches = ep
                                    .container_id
                                    .as_ref()
                                    .map(|cid| cid.starts_with(target_cid) || cid.contains(target_cid))
                                    .unwrap_or(false);
                                if !matches {
                                    continue;
                                }
                            }

                            if format == "json" {
                                write_episode_json_line(&mut handle, &ep)?;
                            } else {
                                let culprit_str = ep.culprits.first().map(|c| format!("culprit={}:{}", c.comm, c.tgid)).unwrap_or_else(|| "-".to_string());
                                let container_str = ep.container_id.as_deref().map(|cid| format!("container={}", &cid[..12.min(cid.len())])).unwrap_or_else(|| "container=-".to_string());
                                println!(
                                    "[{:.3}s] TID {:<6} {:<16} delayed {:>7.2}ms | cause: {:<14} | {:<22} | {}",
                                    ep.start_ns as f64 / 1_000_000_000.0,
                                    ep.tid,
                                    ep.comm,
                                    ep.total_delay_ns as f64 / 1_000_000.0,
                                    ep.segments.first().map(|s| s.cause.as_str()).unwrap_or("unknown"),
                                    container_str,
                                    culprit_str
                                );
                            }
                        }
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                        if stop_flag.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                }
            }

            info!("Exiting kStall watch mode.");
            Ok(())
        }

        Commands::Top {
            pid,
            threshold,
            window,
            containers_only,
            container,
        } => {
            let threshold_ns = parse_duration_ns(&threshold);
            let window_ns = parse_duration_ns(&window);

            let bpf_cfg = BpfConfig {
                runq_threshold_ns: threshold_ns,
                reclaim_threshold_ns: 1_000_000,
                throttle_threshold_ns: 1_000_000,
                target_pid: pid.unwrap_or(0),
                target_tgid: pid.unwrap_or(0),
                target_cgroup_id: 0,
            };

            let stop_flag = Arc::new(AtomicBool::new(false));
            let (event_tx, mut event_rx) = mpsc::channel(1024);
            let (episode_tx, episode_rx) = mpsc::channel(1024);

            let _mgr = BpfManager::start(bpf_cfg, event_tx, stop_flag.clone())?;

            // Background assembler task
            let s_flag = stop_flag.clone();
            tokio::spawn(async move {
                let mut assembler = CausalAssembler::new(AssemblerConfig::default());
                let mut cgroup_res = CgroupResolver::new();

                while !s_flag.load(Ordering::Relaxed) {
                    if let Some(ev) = event_rx.recv().await {
                        if let Some(mut ep) = assembler.ingest(ev) {
                            if let Some(path) = cgroup_res.resolve_cgroup_id(ep.cgroup_id) {
                                let meta = parse_cgroup_metadata(&path);
                                ep.cgroup_path = Some(path);
                                ep.container_id = meta.container_id;
                                ep.pod_name = meta.pod_name;
                            }
                            let _ = episode_tx.send(ep).await;
                        }
                    }
                }
            });

            // Run TUI
            kstall_tui::run_tui_filtered(
                episode_rx,
                stop_flag,
                window_ns,
                containers_only,
                container,
            )
            .await?;
            Ok(())
        }

        Commands::Record {
            output,
            pid,
            threshold,
            duration,
            containers_only,
            container,
        } => {
            let threshold_ns = parse_duration_ns(&threshold);
            let duration_ns = parse_duration_ns(&duration);

            let bpf_cfg = BpfConfig {
                runq_threshold_ns: threshold_ns,
                reclaim_threshold_ns: 1_000_000,
                throttle_threshold_ns: 1_000_000,
                target_pid: pid.unwrap_or(0),
                target_tgid: pid.unwrap_or(0),
                target_cgroup_id: 0,
            };

            let stop_flag = Arc::new(AtomicBool::new(false));
            let s_flag = stop_flag.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                s_flag.store(true, Ordering::Relaxed);
            });

            let (event_tx, mut event_rx) = mpsc::channel(1024);
            let _mgr = BpfManager::start(bpf_cfg, event_tx, stop_flag.clone())?;

            let file = File::create(&output)?;
            let mut writer = BufWriter::new(file);

            let mut assembler = CausalAssembler::new(AssemblerConfig::default());
            let mut cgroup_res = CgroupResolver::new();
            let start = tokio::time::Instant::now();
            let limit = tokio::time::Duration::from_nanos(duration_ns);

            info!(
                "Recording stalls to {} for {:?}...",
                output.display(),
                limit
            );

            while !stop_flag.load(Ordering::Relaxed) && start.elapsed() < limit {
                tokio::select! {
                    Some(ev) = event_rx.recv() => {
                        if let Some(mut ep) = assembler.ingest(ev) {
                            if let Some(path) = cgroup_res.resolve_cgroup_id(ep.cgroup_id) {
                                let meta = parse_cgroup_metadata(&path);
                                ep.cgroup_path = Some(path);
                                ep.container_id = meta.container_id;
                                ep.pod_name = meta.pod_name;
                            }

                            if containers_only && ep.container_id.is_none() {
                                continue;
                            }
                            if let Some(target_cid) = &container {
                                let matches = ep
                                    .container_id
                                    .as_ref()
                                    .map(|cid| cid.starts_with(target_cid) || cid.contains(target_cid))
                                    .unwrap_or(false);
                                if !matches {
                                    continue;
                                }
                            }

                            write_episode_json_line(&mut writer, &ep)?;
                        }
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {}
                }
            }

            info!("Recording complete. Saved to {}", output.display());
            Ok(())
        }

        Commands::Replay {
            input,
            window,
            containers_only,
            container,
        } => {
            let window_ns = parse_duration_ns(&window);
            let file = File::open(&input)?;
            let reader = std::io::BufReader::new(file);

            let (episode_tx, episode_rx) = mpsc::channel(4096);
            let stop_flag = Arc::new(AtomicBool::new(false));

            let mut count = 0;
            for line in std::io::BufRead::lines(reader) {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let ep: Episode = serde_json::from_str(&line)?;
                let _ = episode_tx.send(ep).await;
                count += 1;
            }

            info!(
                "Loaded {} recorded episodes from {}. Launching replay TUI...",
                count,
                input.display()
            );
            kstall_tui::run_tui_filtered(
                episode_rx,
                stop_flag,
                window_ns,
                containers_only,
                container,
            )
            .await?;
            Ok(())
        }

        Commands::Export {
            input,
            output,
            format,
        } => {
            let file = File::open(&input)?;
            let reader = std::io::BufReader::new(file);
            let mut episodes = Vec::new();

            for line in std::io::BufRead::lines(reader) {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let ep: Episode = serde_json::from_str(&line)?;
                episodes.push(ep);
            }

            info!("Read {} episodes from {}", episodes.len(), input.display());
            let out_file = File::create(&output)?;
            let mut writer = BufWriter::new(out_file);

            match format.as_str() {
                "csv" => {
                    SummaryCsv::write_episodes(&mut writer, &episodes)?;
                    info!("Exported episodes CSV to {}", output.display());
                }
                "csv-summary" => {
                    SummaryCsv::write_aggregated_summary(&mut writer, &episodes)?;
                    info!("Exported aggregated summary CSV to {}", output.display());
                }
                _ => {
                    let trace = ChromeTrace::from_episodes(&episodes);
                    trace.write_to(&mut writer)?;
                    info!("Exported Chrome Trace JSON to {}", output.display());
                }
            }
            Ok(())
        }
    }
}
