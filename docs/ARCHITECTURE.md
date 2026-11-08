# kStall Architecture & Design Documentation

## 1. System Overview

`kStall` is a low-overhead eBPF-based latency profiler designed to isolate and diagnose "ghost latency" in production systems running on Linux (kernel 5.15+ with BTF).

Unlike traditional statistical profilers that record every scheduler event or only sample on-CPU stack traces, `kStall` operates on an **in-kernel outlier-only** model:
- Outliers are filtered in BPF ring buffers before reaching userspace.
- Delay durations are correlated with culprits (e.g. CPU-hogging processes, interfering containers, or kernel worker threads).
- Reconstructed causal timelines are presented via an interactive terminal UI or exported as Chrome Trace / Perfetto format.

---

## 2. Component Layout

```text
kstall/
├── bpf/                    # C eBPF source plane
│   ├── kstall.h            # Shared ABI definition (versioned, strictly aligned)
│   ├── kstall.bpf.c        # In-kernel tracepoint probes & LRU maps
│   ├── vmlinux.h           # CO-RE BTF kernel type definitions
│   └── Makefile            # Standalone BPF compilation & skeleton generation
├── crates/
│   ├── kstall-bpf/         # libbpf-rs loader, skeleton bindings, ring buffer poller
│   ├── kstall-core/        # Kernel-free causal assembler, episode store, quantile stats
│   ├── kstall-meta/        # Cgroup v2 inode resolver, Kubernetes pod & container extractor
│   ├── kstall-export/      # Chrome Trace JSON and streaming JSON Lines serializer
│   ├── kstall-tui/         # Interactive Ratatui waterfall timeline & interference matrix
│   └── kstall-cli/         # Command-line entrypoint (`top`, `watch`, `record`, `doctor`)
├── tests/
│   ├── workloads/          # Micro-workloads & latency probe oracle
├── deploy/                 # Dockerfile and Kubernetes DaemonSet manifests
└── ci/                     # Automated test scripts
```

---

## 3. Data Flow & Latency Accounting

1. **Wakeup Path (`sched_wakeup`, `sched_wakeup_new`):**
   - Woken task enters runqueue.
   - Timestamp is stored in `enqueue_ts` map, keyed by TID.
   - Occupant currently executing on target CPU is snapshotted as candidate waker/culprit.

2. **Preemption Path (`sched_switch`):**
   - If `prev` task has `prev_state == TASK_RUNNING`, it was preempted while runnable.
   - It is immediately timestamped in `enqueue_ts` with flag `KS_FLAG_PREEMPTED`.

3. **Switch-in Path (`sched_switch`):**
   - When `next` task is switched in, `enqueue_ts` is checked.
   - If `delta = (now - entry.ts) >= threshold`:
     - An outlier event is emitted to the BPF ring buffer.
     - Culprit 0 is attributed to the task that just surrendered CPU (`prev_pid`).
     - Culprit 1 is attributed to initial occupant at wakeup time if different.

4. **Userspace Processing:**
   - Ring buffer drains events into Tokio channels.
   - `CausalAssembler` merges events into per-thread `Episode` structures.
   - `CgroupResolver` maps cgroup inode to cgroup directory path and pod/container metadata.
   - Episode is routed to TUI, JSON Lines output, or trace file.
