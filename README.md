# kStall: Causal Kernel-Stall and Runqueue Latency Correlator

kStall is a low-overhead, in-kernel latency profiler built with eBPF (C) and Rust. It isolates and explains tail-latency outliers ("ghost latency") by attributing delay intervals to kernel-side root causes (runqueue starvation, CFS bandwidth throttling, direct memory reclaim, and memory compaction) and identifying the interfering tasks or containers responsible for the delay.

---

## Table of Contents

- [Overview](#overview)
- [The Problem: Ghost Latency](#the-problem-ghost-latency)
- [Key Features](#key-features)
- [System Architecture](#system-architecture)
  - [Data Plane (eBPF)](#data-plane-ebpf)
  - [Control Plane and Userspace (Rust)](#control-plane-and-userspace-rust)
  - [Causal Overlap Resolution](#causal-overlap-resolution)
- [Prerequisites and Kernel Requirements](#prerequisites-and-kernel-requirements)
- [Building and Installation](#building-and-installation)
  - [Build Requirements](#build-requirements)
  - [Compilation](#compilation)
  - [Running Diagnostics](#running-diagnostics)
- [Quick Start](#quick-start)
- [Command-Line Reference](#command-line-reference)
  - [Global Options](#global-options)
  - [Duration Syntax](#duration-syntax)
  - [top](#kstall-top)
  - [watch](#kstall-watch)
  - [record](#kstall-record)
  - [replay](#kstall-replay)
  - [export](#kstall-export)
  - [doctor](#kstall-doctor)
  - [version](#kstall-version)
- [Terminal User Interface (TUI)](#terminal-user-interface-tui)
  - [Tabs and Views](#tabs-and-views)
  - [Keybindings](#keybindings)
- [Export Formats and Offline Analysis](#export-formats-and-offline-analysis)
  - [Chrome Trace / Perfetto](#chrome-trace--perfetto)
  - [CSV and Aggregated CSV Summary](#csv-and-aggregated-csv-summary)
  - [JSON Lines Streaming Format](#json-lines-streaming-format)
- [Container and Kubernetes Deployment](#container-and-kubernetes-deployment)
  - [Docker](#docker)
  - [Kubernetes DaemonSet](#kubernetes-daemonset)
  - [Helm Chart](#helm-chart)
  - [Security Capabilities and Privileges](#security-capabilities-and-privileges)
- [Synthetic Workloads and Validation](#synthetic-workloads-and-validation)
- [Repository Structure](#repository-structure)
- [License](#license)

---

## Overview

High-throughput distributed systems (such as Kafka, Cassandra, Redis, PostgreSQL, and microservices written in Go, Java, or Rust) often encounter severe tail-latency spikes (p99/p999) while total host CPU utilization remains modest (20% to 30%).

Standard profiling tools frequently fail in this scenario:
- **On-CPU profilers** (such as `perf record` or flame graphs) capture what runs on CPU, but threads experiencing tail latency are waiting rather than running.
- **Classic Off-CPU profilers** (such as `offcputime`) record where a thread blocked (e.g., in `epoll_wait`, `futex`, or `pthread_cond_wait`), which reveals what the thread was waiting on, but not why it took tens or hundreds of milliseconds to resume executing instructions after becoming runnable.

kStall bridges this gap by capturing **outliers only** at the scheduler and memory management layers, correlating each delay episode with culprit processes or containers, and visualizing the causal timeline in real time.

---

## The Problem: Ghost Latency

When a thread transitions from a blocked state to runnable, the Linux kernel places it onto a CPU runqueue. Under normal conditions, execution begins within microseconds. When tail latency strikes, delay accumulates before user code executes:

1. **Runqueue Starvation**: Other high-priority or CPU-intensive tasks monopolize the core, preventing the runnable thread from being scheduled.
2. **Involuntary Preemption**: A running thread is preempted mid-timeslice and forced back into the runqueue.
3. **CFS Bandwidth Throttling**: The thread's cgroup has exhausted its configured `cpu.max` / `cpu.cfs_quota_us` quota, causing the kernel to deschedule the entire runqueue for milliseconds.
4. **Direct Memory Reclaim**: An allocation triggers synchronous page reclamation (`mm_vmscan_direct_reclaim`) due to memory pressure or cgroup page limits.
5. **Memory Compaction**: Allocation of huge pages or contiguous blocks causes latency stalls inside `mm_compaction_begin/end`.

kStall instruments these exact transition points via eBPF, reconstructs the causal chain, and attributes every millisecond of stall to a specific reason and culprit.

---

## Key Features

- **In-Kernel Outlier Filtering**: Configurable latency thresholds (e.g., `--threshold 2ms`) are evaluated directly in BPF maps. Events below the threshold are discarded in kernel space, keeping steady-state host CPU overhead below 1-2%.
- **Causal Delay Attribution**: Every latency incident is dissected into granular causal segments: Runqueue Wait, Preempted Wait, CFS Throttling, Direct Reclaim, and Compaction.
- **Culprit Identification**: Records the exact task (`comm`, `PID`, `TID`, and `cgroup`) that occupied the CPU while the victim thread was waiting or that caused preemption.
- **Container and Kubernetes Native**: Resolves cgroup v2 inode IDs to filesystem paths and parses Container IDs (CRI-O, containerd, Docker, Podman) and Kubernetes Pod UIDs.
- **Interactive Terminal UI**: Built with `ratatui` and `crossterm`, providing live waterfall timelines, latency breakdowns, victim-versus-culprit interference matrices, and detailed modal drill-downs.
- **Record and Replay**: Record live stall incidents to JSON Lines files on production nodes, then replay and inspect them offline on any workstation without requiring root privileges.
- **Multiple Exporters**: Export recorded incidents to Google Chrome Trace / Perfetto format (`trace_events`), tabular CSV, or aggregated summary CSV.
- **Environment Doctor**: Includes a built-in diagnostic subcommand (`kstall doctor`) that validates kernel release, BTF presence, tracefs mounts, cgroup hierarchy, and capability settings.

---

## System Architecture

```text
+-------------------------------------------------------------------------------+
|                                  KERNEL SPACE                                 |
|                                                                               |
|  Sched Wakeup / Switch       Direct Reclaim / Compact        CFS Throttling   |
|  [tp_btf/sched_wakeup]      [tp_btf/mm_vmscan_direct]     [kprobe/throttle_   |
|  [tp_btf/sched_switch]      [tp_btf/mm_compaction]         cfs_rq]            |
|            |                           |                          |           |
|            v                           v                          v           |
|     +-------------+             +-------------+            +-------------+    |
|     | enqueue_ts  |             | reclaim_ts  |            |  throttle_  |    |
|     |  (LRU Map)  |             | compact_ts  |            |  state Map  |    |
|     +-------------+             +-------------+            +-------------+    |
|            |                           |                          |           |
|            +---------------------------+--------------------------+           |
|                                        |                                      |
|                       In-Kernel Outlier Threshold Check                       |
|                               (delta >= threshold)                            |
|                                        |                                      |
|                                        v                                      |
|                           +--------------------------+                        |
|                           |  BPF Ring Buffer (16 MB) |                        |
|                           +--------------------------+                        |
+----------------------------------------|--------------------------------------+
                                         | Userspace Poller
+----------------------------------------v--------------------------------------+
|                                USERSPACE AGENT                                |
|                                                                               |
|  [crates/kstall-bpf]       --> Consumes ring buffer events                    |
|  [crates/kstall-meta]      --> Resolves cgroup inode -> container/pod ID      |
|  [crates/kstall-core]      --> CausalAssembler merges overlapping intervals   |
|                                Quantile calculation (p50, p95, p99, max)      |
|                                Victim x Culprit Interference Matrix           |
|                                                                               |
|                                        |                                      |
|            +---------------------------+---------------------------+          |
|            v                           v                           v          |
|    Interactive TUI             Live Terminal Stream          Trace Exporter   |
|  (Waterfall, Matrix)           (Text / JSON Lines)          (Perfetto / CSV)  |
|  [crates/kstall-tui]           [crates/kstall-cli]        [crates/kstall-export]
+-------------------------------------------------------------------------------+
```

### Data Plane (eBPF)

The data plane (`bpf/kstall.bpf.c`) is written in C using BPF CO-RE (Compile Once - Run Everywhere):

- `tp_btf/sched_wakeup` and `tp_btf/sched_wakeup_new`: Records task wakeup timestamp and snapshots the currently running occupant on the target CPU as a candidate waker/culprit.
- `tp_btf/sched_migrate_task`: Tracks task movements across CPU runqueues.
- `tp_btf/sched_switch`: Inspects preempted tasks (`TASK_RUNNING`), checks switch-in latency against `runq_threshold_ns`, captures the surrendered CPU occupant as the primary culprit, and emits outlier records to the BPF ring buffer.
- `tp_btf/mm_vmscan_direct_reclaim_begin` / `end`: Measures synchronous direct page reclamation latency.
- `tp_btf/mm_compaction_begin` / `end`: Measures memory compaction stalls.
- `kprobe/throttle_cfs_rq` / `unthrottle_cfs_rq`: Tracks cgroup bandwidth throttling durations.

### Control Plane and Userspace (Rust)

The userspace agent is modularized into several crates:

- `kstall-bpf`: Manages BPF lifecycle via `libbpf-rs`, loads skeletons, populates configuration maps, and streams events through Tokio channels.
- `kstall-core`: Implements the `CausalAssembler` which constructs coherent `Episode` structures, performs overlap reconciliation, and maintains sliding window quantile statistics (`EpisodeWindow`).
- `kstall-meta`: Maps cgroup v2 inode IDs to path strings by traversing `/sys/fs/cgroup` and extracts Kubernetes pod names and container IDs.
- `kstall-export`: Generates Chrome Trace / Perfetto format JSON, per-episode CSV, and aggregated workload summary CSV files.
- `kstall-tui`: Implements the four-tab terminal user interface.
- `kstall-cli`: Exposes the command-line interface and subcommands.

### Causal Overlap Resolution

When a thread encounters multiple concurrent or sequential stalls (for instance, being placed on a runqueue while its cgroup is throttled, or triggering direct reclaim during execution), `kStall` resolves overlaps according to causal priority:

```text
Priority: CFS Throttled > Direct Reclaim / Compaction > Preempted Wait > Runqueue Wait
```

If a thread is queued for 12 ms, but its cgroup was CFS-throttled for 10 ms of that duration, `CausalAssembler` attributes 10 ms to CFS throttling and 2 ms to pure runqueue wait, preventing double-counting.

---

## Prerequisites and Kernel Requirements

kStall relies on modern Linux kernel tracing infrastructure:

- **Linux Kernel**: Version 5.15 or newer (tested on 5.15, 6.1, 6.6, 6.12, and 6.18+).
- **BTF (BPF Type Format)**: Enabled in kernel configuration (`CONFIG_DEBUG_INFO_BTF=y`). Verify existence of `/sys/kernel/btf/vmlinux`.
- **Cgroup v2**: Unified cgroup hierarchy mounted at `/sys/fs/cgroup`.
- **Tracefs**: Tracing virtual filesystem mounted at `/sys/kernel/tracing` or `/sys/kernel/debug/tracing`.
- **Privileges**: Root access or the following Linux capabilities:
  - `CAP_BPF` (BPF program loading and map operations)
  - `CAP_PERFMON` (attach to tracepoints and kprobes)
  - `CAP_SYS_RESOURCE` (setrlimit memory locking for BPF maps)

---

## Building and Installation

### Quick Install (Pre-built Binaries)

Install the latest pre-compiled release binary directly on any Linux x86_64 or aarch64 system:

```bash
curl -fsSL https://raw.githubusercontent.com/kstall/kstall/master/install.sh | bash
```

Alternatively, download official release packages, standalone binaries, and cryptographic SHA-256 checksums from the [GitHub Releases](https://github.com/kstall/kstall/releases) page.

### Build Requirements

Ensure the host toolchain includes:
- **Rust**: Stable toolchain (1.80 or newer, with `cargo`)
- **LLVM and Clang**: Version 14 or newer
- **libbpf**: Development libraries (`libbpf-dev` on Debian/Ubuntu, `libbpf` on Fedora/Arch)
- **bpftool**: Standard kernel BPF utility
- **make**, **pkg-config**, **zlib**, and **libelf**

On Debian / Ubuntu:
```bash
sudo apt-get update
sudo apt-get install -y clang llvm libbpf-dev libelf-dev zlib1g-dev linux-libc-dev bpftool make pkg-config
```

On Fedora / RHEL:
```bash
sudo dnf install -y clang llvm libbpf-devel elfutils-libelf-devel zlib-devel bpftool make pkgconfig
```

On Arch Linux:
```bash
sudo pacman -S clang llvm libbpf libelf zlib bpf make pkgconf
```

### Compilation

Clone the repository and build both BPF objects and userspace binaries:

```bash
git clone https://github.com/kstall/kstall.git
cd kstall

# Build BPF skeleton and all Rust workspace crates
make all

# For an optimized release build:
make release
```

The compiled binary will be placed at `./target/release/kstall` (or `./target/debug/kstall`).

### Running Diagnostics

Before running kStall in production, execute the `doctor` command to verify your kernel and system capabilities:

```bash
./target/release/kstall doctor
```

Example output:
```text
kStall Doctor - Kernel & Environment Health Diagnostic
=======================================================

Kernel Version                         [PASS] - Detected: 6.18.55-2-cachyos-lts
Kernel BTF (/sys/kernel/btf/vmlinux)   [PASS] - Present (CO-RE supported)
Tracefs / Tracing Mount                [PASS] - Mounted
Cgroup v2 Unified Hierarchy            [PASS] - Active (unified cgroup v2)
Process Privileges                     [PASS] - Running as root (CAP_BPF & CAP_PERFMON available)
sysctl unprivileged_bpf_disabled       [PASS] - Value: 2 (unprivileged bpf permanently disabled)

-------------------------------------------------------
Status: All critical environment checks passed! System ready.
```

---

## Quick Start

### 1. Launch Live Interactive TUI
Observe stall outliers live across the entire host with a default 2 ms threshold:
```bash
sudo ./target/release/kstall top --threshold 2ms
```

### 2. Stream Outliers Directly to Console
Stream stall events in human-readable text format:
```bash
sudo ./target/release/kstall watch --threshold 5ms
```

Stream events as machine-readable JSON:
```bash
sudo ./target/release/kstall watch --threshold 2ms --format json
```

### 3. Record a Production Incident
Capture stall incidents to a file for 60 seconds:
```bash
sudo ./target/release/kstall record \
    --output incident_capture.jsonl \
    --threshold 2ms \
    --duration 60s
```

### 4. Replay and Analyze Offline
Replay the captured incident in the interactive TUI on any development machine (does not require root or Linux):
```bash
./target/release/kstall replay --input incident_capture.jsonl
```

### 5. Export to Perfetto / Chrome Trace
Convert recorded stall episodes to Chrome Trace format for timeline inspection:
```bash
./target/release/kstall export \
    --input incident_capture.jsonl \
    --output trace.json \
    --format chrome
```
Open `chrome://tracing` or [ui.perfetto.dev](https://ui.perfetto.dev) in any browser and load `trace.json`.

---

## Command-Line Reference

The main executable provides the following subcommands:

```text
Usage: kstall <COMMAND>

Commands:
  top      Live interactive TUI waterfall and interference matrix
  watch    Stream stall outlier events directly to console (Text or JSON)
  record   Record stall episodes to a file (JSON Lines format)
  replay   Replay recorded episodes in the interactive TUI
  export   Export recorded episodes to Chrome Trace JSON or Summary CSV
  doctor   Diagnose kernel environment, tracepoints, and privileges
  version  Display version information
```

### Global Options

- `-h, --help`: Print command help.
- `-V, --version`: Print binary version.

### Duration Syntax

All duration arguments (`--threshold`, `--duration`, `--window`) accept human-readable units:
- `ns`: Nanoseconds (e.g., `500000ns`)
- `us`: Microseconds (e.g., `250us`, `500us`)
- `ms`: Milliseconds (e.g., `2ms`, `10ms`)
- `s`: Seconds (e.g., `30s`, `60s`)
- `m`: Minutes (e.g., `5m`, `15m`)
- `h`: Hours (e.g., `1h`)

---

### kstall top

Launches the full-screen terminal user interface with live event streaming.

```text
Usage: kstall top [OPTIONS]

Options:
  -p, --pid <PID>              Filter by target process PID or thread TID
  -C, --containers-only        Show stalls from containerized workloads only (Docker/CRI)
      --container <CONTAINER>  Filter by container ID or container ID prefix
  -t, --threshold <THRESHOLD>  Runqueue outlier threshold [default: 2ms]
  -w, --window <WINDOW>        Rolling time window duration [default: 5m]
  -h, --help                   Print help
```

Examples:
```bash
# Monitor all host processes with stalls >= 1ms over a 10-minute rolling window
sudo kstall top -t 1ms -w 10m

# Focus solely on containerized tasks
sudo kstall top -C --threshold 2ms

# Focus on a specific database process
sudo kstall top --pid 14285 -t 500us
```

---

### kstall watch

Streams stall outlier events to stdout as they occur. Useful for log aggregation or quick CLI inspection.

```text
Usage: kstall watch [OPTIONS]

Options:
  -p, --pid <PID>              Filter by target PID/TGID
  -C, --containers-only        Capture stalls from containerized tasks only
      --container <CONTAINER>  Filter by container ID or short prefix
  -t, --threshold <THRESHOLD>  Runqueue outlier threshold [default: 2ms]
  -f, --format <FORMAT>        Output format: text | json [default: text]
  -h, --help                   Print help
```

Example Text Output:
```text
[1731067214.382s] TID 14820  mysqld           delayed   12.45ms | cause: cfs_throttled  | container=6d55787926bd | culprit=-
[1731067214.410s] TID 18934  tokio-runtime-w  delayed    4.18ms | cause: runqueue_wait  | container=e9a8f4c1b2d3 | culprit=kcompactd0:58
[1731067214.502s] TID 19001  redis-server     delayed    8.92ms | cause: direct_reclaim | container=a1b2c3d4e5f6 | culprit=stress-ng:21044
```

Example JSON Output:
```bash
sudo kstall watch -t 2ms -f json | jq .
```
```json
{
  "id": 142,
  "tid": 14820,
  "tgid": 14800,
  "comm": "mysqld",
  "cgroup_id": 4026532891,
  "cgroup_path": "/sys/fs/cgroup/kubepods.slice/kubepods-burstable-pod7d174661.slice/cri-containerd-6d55787926bdf.scope",
  "container_id": "6d55787926bdf",
  "pod_name": "7d174661-37d5-4c19-9d1f-a18b76c8c977",
  "start_ns": 1731067214382000000,
  "end_ns": 1731067214394450000,
  "total_delay_ns": 12450000,
  "segments": [
    {
      "cause": "cfs_throttled",
      "start_ns": 1731067214382000000,
      "end_ns": 1731067214392000000,
      "duration_ns": 10000000,
      "cpu": 3,
      "culprit": null
    },
    {
      "cause": "runqueue_wait",
      "start_ns": 1731067214392000000,
      "end_ns": 1731067214394450000,
      "duration_ns": 2450000,
      "cpu": 3,
      "culprit": {
        "tid": 21044,
        "tgid": 21040,
        "comm": "stress-ng",
        "cgroup_id": 4026531999,
        "stolen_ns": 2450000,
        "flags": 0
      }
    }
  ],
  "culprits": [
    {
      "tid": 21044,
      "tgid": 21040,
      "comm": "stress-ng",
      "cgroup_id": 4026531999,
      "stolen_ns": 2450000,
      "flags": 0
    }
  ]
}
```

---

### kstall record

Records stall episodes to a file using the streaming JSON Lines format.

```text
Usage: kstall record [OPTIONS] --output <OUTPUT>

Options:
  -o, --output <OUTPUT>        Output file path (e.g. stalls.jsonl)
  -p, --pid <PID>              Filter by target PID/TGID
  -C, --containers-only        Record stalls from containerized tasks only
      --container <CONTAINER>  Filter by container ID or short prefix
  -t, --threshold <THRESHOLD>  Runqueue outlier threshold [default: 2ms]
  -d, --duration <DURATION>    Recording duration [default: 30s]
  -h, --help                   Print help
```

Example:
```bash
sudo kstall record -o /var/log/kstall_outliers.jsonl -t 2ms -d 5m
```

---

### kstall replay

Loads a previously recorded JSON Lines file into the interactive TUI. This operation is kernel-independent and can be performed without root or Linux capabilities.

```text
Usage: kstall replay [OPTIONS] --input <INPUT>

Options:
  -i, --input <INPUT>          Input recording file path (JSON Lines)
  -C, --containers-only        Show stalls from containerized tasks only
      --container <CONTAINER>  Filter by container ID or short prefix
  -w, --window <WINDOW>        Rolling time window duration [default: 1h]
  -h, --help                   Print help
```

Example:
```bash
kstall replay -i incident_capture.jsonl
```

---

### kstall export

Converts recorded episodes into standard formats for offline analysis, reporting, or graphing.

```text
Usage: kstall export [OPTIONS] --input <INPUT> --output <OUTPUT>

Options:
  -i, --input <INPUT>    Input recording file (JSON Lines)
  -o, --output <OUTPUT>  Output file path
  -f, --format <FORMAT>  Export format: chrome | csv | csv-summary [default: chrome]
  -h, --help             Print help
```

Supported Formats:
1. `chrome`: Chrome Trace / Perfetto JSON format. Visualizes processes, threads, and stall segments along an interactive timeline.
2. `csv`: Detailed per-episode table with timestamp, comm, PID, TID, total delay, breakdown by cause, primary culprit, and cgroup path.
3. `csv-summary`: Aggregated per-workload summary table with incident count, total delay, and mean duration.

Examples:
```bash
# Export to Chrome Trace JSON for Perfetto
kstall export -i stalls.jsonl -o trace.json -f chrome

# Export to detailed episode CSV
kstall export -i stalls.jsonl -o episodes.csv -f csv

# Export to aggregated summary CSV
kstall export -i stalls.jsonl -o summary.csv -f csv-summary
```

---

### kstall doctor

Runs automated checks on the host environment to verify all required kernel facilities and privileges.

Exit codes:
- `0`: All checks passed. System is fully operational.
- `2`: Critical kernel feature missing (e.g., BTF or tracefs).
- `3`: Insufficient process capabilities (e.g., not root and lacking `CAP_BPF`/`CAP_PERFMON`).

---

### kstall version

Prints version, ABI version, and compilation targets.

```bash
$ kstall version
kStall v0.1.0 (ABI v1, kernel 5.15+ BTF CO-RE)
```

---

## Terminal User Interface (TUI)

The TUI provides real-time observability into stall outliers and their causes.

### Tabs and Views

#### 1. Waterfall (Timeline)
The default view. Displays a ranked list of stall episodes. Each entry features an inline visual bar graph illustrating the proportion of time spent in each cause category:
- Total stall duration in milliseconds
- Victim process name, Thread ID (TID), and Process ID (PID)
- Container ID and Kubernetes Pod UID (if containerized)
- Primary interfering culprit process and PID
- Visual cause distribution bar

#### 2. Latency Breakdown
Displays cumulative metrics for the current window:
- Aggregated time spent across all five cause categories: Runqueue Wait, Preempted Wait, CFS Throttling, Direct Reclaim, and Compaction.
- Percentage contribution of each cause to overall host latency.
- Statistical quantiles: Min, p50, p95, p99, and Max latency durations.

#### 3. Interference Matrix
Aggregates and ranks victim-versus-culprit pairs:
- **Victim Workload**: Name and PID of delayed task.
- **Interfering Culprit**: Name and PID of task occupying the CPU during the delay.
- **Stolen Time**: Total milliseconds of CPU time stolen by the culprit from the victim.
- **Incident Count**: Number of discrete stall episodes attributed to this pairing.

#### 4. Event Log
A chronological stream of raw stall events, updated in real time as outliers cross the kernel threshold.

---

### Keybindings

| Key | Action |
| :--- | :--- |
| `1` / `2` / `3` / `4` | Switch directly to Tab 1 (Waterfall), 2 (Breakdown), 3 (Matrix), or 4 (Log) |
| `Tab` / `BackTab` | Cycle forward / backward through tabs |
| `Up` / `k` | Move selection to previous row |
| `Down` / `j` | Move selection to next row |
| `Enter` | Open detailed modal drill-down for the selected episode |
| `/` | Enter search filter mode (type substring to filter by comm, TID, container, etc.) |
| `c` | Toggle container filter mode (`CONTAINERS ONLY` vs `ALL TASKS`) |
| `s` | Cycle sort order: `Duration Descending` -> `Time Descending` -> `Duration Ascending` -> `Time Ascending` |
| `p` | Pause / resume live ingestion stream |
| `e` | Export current in-memory buffer directly to Chrome Trace JSON (`kstall_trace_<timestamp>.json`) |
| `?` | Toggle help popup |
| `Esc` | Close modal popup, exit filter input mode, or clear search |
| `q` / `Ctrl+C` | Exit kStall |

---

## Export Formats and Offline Analysis

### Chrome Trace / Perfetto

The `chrome` exporter writes JSON formatted according to the Google Chrome Trace Event format. This can be opened directly in [ui.perfetto.dev](https://ui.perfetto.dev) or `chrome://tracing`.

- Process tracks (`M` metadata events) correspond to OS processes (`comm` + `PID`).
- Thread tracks represent individual OS threads (`TID`).
- Slices (`X` complete events) display exact start timestamps and microsecond durations for each stall segment, color-coded by cause:
  - `runqueue_wait`
  - `preempted_wait`
  - `cfs_throttled`
  - `direct_reclaim`
  - `compaction`
- Slice metadata arguments detail CPU IDs, cgroup paths, container IDs, and culprit process names.

### CSV and Aggregated CSV Summary

#### Detailed Episode CSV (`-f csv`)
Columns:
```text
timestamp_s,comm,tgid,tid,total_delay_ms,runq_ms,preempt_ms,reclaim_ms,compact_ms,throttle_ms,primary_cause,primary_culprit,culprit_tgid,cgroup_path
```

#### Workload Summary CSV (`-f csv-summary`)
Aggregates stalls by process name (`comm`):
```text
comm,incident_count,total_delay_ms,avg_delay_ms
```

### JSON Lines Streaming Format

Every line written by `record` or `watch --format json` is an independent, valid JSON object serializing the `Episode` struct. It can be parsed with tools like `jq`, ingested into Elasticsearch/Logstash, or processed with Python/Pandas scripts.

---

## Container and Kubernetes Deployment

### Docker

Build the lightweight container image using the included multi-stage Dockerfile:

```bash
docker build -t kstall:latest -f deploy/Dockerfile .
```

Run with the required BPF and tracing capabilities:

```bash
docker run --rm -it \
    --name kstall \
    --pid host \
    --cap-add BPF \
    --cap-add PERFMON \
    --cap-add SYS_RESOURCE \
    -v /sys/kernel/tracing:/sys/kernel/tracing:ro \
    -v /sys/kernel/debug:/sys/kernel/debug:ro \
    -v /sys/kernel/btf:/sys/kernel/btf:ro \
    -v /sys/fs/cgroup:/sys/fs/cgroup:ro \
    -v /sys/fs/bpf:/sys/fs/bpf:ro \
    kstall:latest watch --threshold 2ms
```

---

### Kubernetes DaemonSet

Deploy kStall across your cluster to stream stall events from containerized tasks. A production DaemonSet manifest is provided at `deploy/daemonset.yaml`.

Key manifest configuration:

```yaml
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: kstall-agent
  namespace: kube-system
  labels:
    app.kubernetes.io/name: kstall
spec:
  selector:
    matchLabels:
      app.kubernetes.io/name: kstall
  template:
    metadata:
      labels:
        app.kubernetes.io/name: kstall
    spec:
      hostPID: true
      hostNetwork: false
      tolerations:
        - operator: Exists
          effect: NoSchedule
      containers:
        - name: kstall
          image: kstall:latest
          args:
            - "watch"
            - "--threshold"
            - "5ms"
            - "--format"
            - "json"
          securityContext:
            privileged: false
            capabilities:
              add:
                - BPF
                - PERFMON
                - SYS_RESOURCE
          resources:
            requests:
              cpu: 50m
              memory: 64Mi
            limits:
              cpu: 200m
              memory: 256Mi
          volumeMounts:
            - name: sys-kernel-tracing
              mountPath: /sys/kernel/tracing
              readOnly: true
            - name: sys-kernel-debug
              mountPath: /sys/kernel/debug
              readOnly: true
            - name: sys-kernel-btf
              mountPath: /sys/kernel/btf
              readOnly: true
            - name: sys-fs-cgroup
              mountPath: /sys/fs/cgroup
              readOnly: true
            - name: sys-fs-bpf
              mountPath: /sys/fs/bpf
              readOnly: true
      volumes:
        - name: sys-kernel-tracing
          hostPath:
            path: /sys/kernel/tracing
        - name: sys-kernel-debug
          hostPath:
            path: /sys/kernel/debug
        - name: sys-kernel-btf
          hostPath:
            path: /sys/kernel/btf
        - name: sys-fs-cgroup
          hostPath:
            path: /sys/fs/cgroup
        - name: sys-fs-bpf
          hostPath:
            path: /sys/fs/bpf
```

Deploy to your cluster:
```bash
kubectl apply -f deploy/daemonset.yaml
```

View cluster-wide latency outliers:
```bash
kubectl logs -n kube-system -l app.kubernetes.io/name=kstall -f --tail=100
```

---

### Helm Chart

A Helm chart is located in `deploy/helm/kstall`:

```bash
# Install with default values
helm install kstall ./deploy/helm/kstall --namespace kube-system

# Install with custom threshold and containers-only filter
helm install kstall ./deploy/helm/kstall \
    --namespace kube-system \
    --set config.threshold="2ms" \
    --set config.containersOnly=true
```

---

### Security Capabilities and Privileges

kStall does **not** require full container privilege (`privileged: true`). It runs safely within hardened environments using fine-grained Linux capabilities:

- `CAP_BPF`: Grants permissions to load BPF programs and create BPF maps.
- `CAP_PERFMON`: Grants permissions to attach BPF probes to kernel tracepoints and kprobes.
- `CAP_SYS_RESOURCE`: Allows the agent to adjust memory limits (`RLIMIT_MEMLOCK`) required for pinned BPF ring buffers.
- `hostPID: true`: Enables resolution of host PIDs and TIDs matching kernel tracepoint arguments.
- **Read-Only Mounts**: All host filesystem mounts (`/sys/fs/cgroup`, `/sys/kernel/btf`, `/sys/kernel/tracing`) are mounted read-only (`readOnly: true`). kStall cannot modify kernel parameters or container cgroups.

---

## Synthetic Workloads and Validation

The `tests/workloads` directory contains test utilities to validate detection accuracy against injected latency:

1. **CFS Quota Throttling (`cfs_throttle_tester.sh`)**:
   Creates a temporary cgroup v2 slice with constrained `cpu.max` (e.g., `20000 100000` = 20% CPU limit), runs an active compute workload, and triggers CFS throttling.
   ```bash
   sudo ./tests/workloads/cfs_throttle_tester.sh
   ```

2. **CPU Starvation Hog (`starvation_hog.rs`)**:
   Spawns CPU-bound worker threads with high scheduling priorities to create runqueue starvation for lower-priority tasks.
   ```bash
   cargo run --release --bin starvation_hog
   ```

3. **Direct Memory Reclaim Pressure (`reclaim_pressure.rs`)**:
   Constrains memory allocation limits within a test cgroup and repeatedly allocates page cache buffers to trigger synchronous `mm_vmscan_direct_reclaim`.
   ```bash
   cargo run --release --bin reclaim_pressure
   ```

4. **Latency Probe Oracle (`latency_probe.rs`)**:
   Performs calibrated sub-millisecond sleeps while measuring actual elapsed wall-clock time to detect scheduler delays and compare them against kStall reports.
   ```bash
   cargo run --release --bin latency_probe
   ```

Run the unit and integration test suite:
```bash
make test
# Or directly via cargo:
cargo test --workspace
```

---

## Repository Structure

```text
kstall/
├── Cargo.toml                 # Workspace manifest
├── Makefile                   # Top-level build targets (all, bpf, test, doctor)
├── kstall-PRD.md              # Product Requirements Document
│
├── bpf/                       # eBPF Data Plane (C)
│   ├── kstall.bpf.c           # Tracepoints, kprobes, and ring buffer logic
│   ├── kstall.h               # Shared ABI definitions (struct kstall_event)
│   ├── vmlinux.h              # CO-RE kernel type definitions
│   └── Makefile               # BPF compilation and skeleton generation
│
├── crates/                    # Userspace Control Plane (Rust)
│   ├── kstall-cli/            # CLI entrypoint, commands, and doctor checks
│   ├── kstall-bpf/            # libbpf-rs bindings, loader, ring buffer poller
│   ├── kstall-core/           # CausalAssembler, Episode model, quantile stats
│   ├── kstall-meta/           # Cgroup v2 inode resolver, Kubernetes metadata
│   ├── kstall-export/         # Chrome Trace JSON, CSV, and summary serializers
│   └── kstall-tui/            # Ratatui interactive waterfall and matrix views
│
├── deploy/                    # Deployment Assets
│   ├── Dockerfile             # Multi-stage container build
│   ├── daemonset.yaml         # Kubernetes DaemonSet specification
│   └── helm/                  # Helm packaging chart
│
├── docs/                      # Technical Documentation
│   ├── ARCHITECTURE.md        # Deep-dive architecture and memory layout
│   └── DEVELOPMENT.md         # Developer setup and contribution instructions
│
└── tests/                     # Tests and Micro-Workloads
    ├── integration/           # Integration tests for assembly and replay
    └── workloads/             # Synthetic latency injection probes
```

---

## License

kStall is licensed under either:
- [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0)
- [GNU General Public License, Version 2.0](https://www.gnu.org/licenses/gpl-2.0.html)

at your option. The eBPF kernel code (`bpf/`) is licensed under Dual BSD/GPL.
