/* SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause) */
/* Copyright (c) 2026 kStall Developers */
#ifndef __KSTALL_H
#define __KSTALL_H

#ifndef __VMLINUX_H__
#include <linux/types.h>
#endif

#define KSTALL_ABI_VERSION 1
#define TASK_COMM_LEN 16
#define KSTALL_MAX_CULPRITS 3

/* Event types */
enum kstall_event_type {
    KS_EV_UNKNOWN       = 0,
    KS_EV_RUNQ          = 1,  /* wakeup -> run */
    KS_EV_PREEMPT_RUNQ  = 2,  /* preempted -> run */
    KS_EV_RECLAIM       = 3,  /* direct reclaim stall */
    KS_EV_COMPACT       = 4,  /* memory compaction stall */
    KS_EV_THROTTLE      = 5,  /* cgroup CFS throttled interval */
};

/* Event flags */
#define KS_FLAG_PREEMPTED         (1U << 0)
#define KS_FLAG_MIGRATED          (1U << 1)
#define KS_FLAG_THROTTLED_OVERLAP (1U << 2)
#define KS_FLAG_TRUNCATED         (1U << 3)
#define KS_FLAG_KERNEL_THREAD     (1U << 4)

/* Culprit description */
struct kstall_culprit {
    __u32 tid;
    __u32 tgid;
    __u64 cgroup_id;
    __u64 stolen_ns;
    char  comm[TASK_COMM_LEN];
    __u32 flags;
    __u32 _pad;
};

/* Unified event record emitted via BPF ring buffer */
struct kstall_event {
    __u32 abi_version;
    __u32 type;              /* enum kstall_event_type */
    __u64 ts_start_ns;       /* Monotonic start timestamp (ns) */
    __u64 ts_end_ns;         /* Monotonic end timestamp (ns) */
    __u64 duration_ns;       /* Total stall duration (ns) */
    __u32 tid;               /* Victim thread ID */
    __u32 tgid;              /* Victim thread group ID (PID) */
    __u32 cpu_start;         /* CPU where wait began */
    __u32 cpu_end;           /* CPU where wait ended / task ran */
    __u64 cgroup_id;         /* Victim cgroup v2 ID */
    char  comm[TASK_COMM_LEN]; /* Victim comm */

    /* Culprit attribution: top-K tasks occupying CPU or causing interference */
    struct kstall_culprit culprits[KSTALL_MAX_CULPRITS];
    __u32 num_culprits;

    __u32 flags;             /* KS_FLAG_* */
    __s32 kstack_id;         /* Kernel stack trace ID (-1 if disabled) */
    __u32 _pad;
};

/* Configuration structure passed from userspace agent */
struct kstall_bpf_config {
    __u64 runq_threshold_ns;     /* e.g., 2,000,000 ns (2ms) */
    __u64 reclaim_threshold_ns;  /* e.g., 1,000,000 ns (1ms) */
    __u64 throttle_threshold_ns; /* e.g., 1,000,000 ns (1ms) */
    __u32 target_pid;            /* Filter by target PID (0 = disable) */
    __u32 target_tgid;           /* Filter by target TGID (0 = disable) */
    __u64 target_cgroup_id;      /* Filter by target cgroup ID (0 = disable) */
    __u32 agent_pid;             /* kStall agent PID to self-exclude */
    __u32 flags;
};

/* Statistics counters */
struct kstall_bpf_stats {
    __u64 seen_events;
    __u64 filtered_events;
    __u64 emitted_events;
    __u64 dropped_events;
};

#endif /* __KSTALL_H */
