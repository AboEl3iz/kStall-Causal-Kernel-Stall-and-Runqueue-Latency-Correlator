// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

#include "vmlinux.h"

#if defined(__TARGET_ARCH_arm64)
struct user_pt_regs {
    __u64 regs[31];
    __u64 sp;
    __u64 pc;
    __u64 pstate;
};
#endif

#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>
#include <bpf/bpf_core_read.h>
#include "kstall.h"

char LICENSE[] SEC("license") = "Dual BSD/GPL";

#define PF_KTHREAD_FLAG 0x00200000U

/* Internal tracking structure for queued / runnable tasks */
struct enqueue_val {
    __u64 ts;
    __u32 target_cpu;
    __u32 culprit_tid;
    __u32 culprit_tgid;
    __u64 culprit_cgid;
    char  culprit_comm[TASK_COMM_LEN];
    __u32 flags;
    __u32 pad;
};

/* Per-CPU current occupant tracking */
struct cpu_occupant {
    __u32 tid;
    __u32 tgid;
    __u64 cgid;
    __u64 since_ts;
    char  comm[TASK_COMM_LEN];
};

/* Maps */
struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 65536);
    __type(key, __u32); /* TID */
    __type(value, struct enqueue_val);
} enqueue_ts SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct cpu_occupant);
} cpu_curr SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_RINGBUF);
    __uint(max_entries, 16 * 1024 * 1024); /* 16 MB ring buffer */
} events SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct kstall_bpf_config);
} config_map SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct kstall_bpf_stats);
} stats_map SEC(".maps");

/* Direct reclaim tracking */
struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 16384);
    __type(key, __u32);
    __type(value, __u64);
} reclaim_ts SEC(".maps");

/* Compaction tracking */
struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 16384);
    __type(key, __u32);
    __type(value, __u64);
} compact_ts SEC(".maps");

/* CFS throttle state tracking per cgroup */
struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 4096);
    __type(key, __u64); /* cgroup_id */
    __type(value, __u64); /* throttle start timestamp */
} throttle_state SEC(".maps");

/* Helper: get runtime configuration */
static __always_inline struct kstall_bpf_config *get_config(void) {
    __u32 key = 0;
    return bpf_map_lookup_elem(&config_map, &key);
}

/* Helper: update statistics */
static __always_inline void inc_stat_seen(void) {
    __u32 key = 0;
    struct kstall_bpf_stats *st = bpf_map_lookup_elem(&stats_map, &key);
    if (st)
        st->seen_events++;
}

static __always_inline void inc_stat_filtered(void) {
    __u32 key = 0;
    struct kstall_bpf_stats *st = bpf_map_lookup_elem(&stats_map, &key);
    if (st)
        st->filtered_events++;
}

static __always_inline void inc_stat_emitted(void) {
    __u32 key = 0;
    struct kstall_bpf_stats *st = bpf_map_lookup_elem(&stats_map, &key);
    if (st)
        st->emitted_events++;
}

static __always_inline void inc_stat_dropped(void) {
    __u32 key = 0;
    struct kstall_bpf_stats *st = bpf_map_lookup_elem(&stats_map, &key);
    if (st)
        st->dropped_events++;
}

/* Helper: check if task should be traced */
static __always_inline bool should_trace_task(struct task_struct *p, struct kstall_bpf_config *cfg) {
    __u32 pid = BPF_CORE_READ(p, pid);
    __u32 tgid = BPF_CORE_READ(p, tgid);

    /* Never trace the kernel idle task (PID 0) */
    if (pid == 0)
        return false;

    if (!cfg)
        return true;

    /* Self-exclude kStall userspace agent */
    if (cfg->agent_pid && tgid == cfg->agent_pid)
        return false;

    /* PID/TGID filters */
    if (cfg->target_pid && pid != cfg->target_pid)
        return false;
    if (cfg->target_tgid && tgid != cfg->target_tgid)
        return false;

    return true;
}

/* Common enqueue handler for sched_wakeup and sched_wakeup_new */
static __always_inline int trace_enqueue(struct task_struct *p) {
    inc_stat_seen();

    struct kstall_bpf_config *cfg = get_config();
    if (!should_trace_task(p, cfg))
        return 0;

    __u32 tid = BPF_CORE_READ(p, pid);
    __u64 now = bpf_ktime_get_ns();
    __u32 cpu = bpf_get_smp_processor_id();

    struct enqueue_val val = {};
    val.ts = now;
    val.target_cpu = cpu;
    val.flags = 0;

    /* Snapshot current occupant of this CPU as initial waker/culprit context */
    __u32 zero = 0;
    struct cpu_occupant *curr = bpf_map_lookup_elem(&cpu_curr, &zero);
    if (curr) {
        val.culprit_tid = curr->tid;
        val.culprit_tgid = curr->tgid;
        val.culprit_cgid = curr->cgid;
        __builtin_memcpy(val.culprit_comm, curr->comm, TASK_COMM_LEN);
    }

    bpf_map_update_elem(&enqueue_ts, &tid, &val, BPF_ANY);
    return 0;
}

SEC("tp_btf/sched_wakeup")
int BPF_PROG(sched_wakeup, struct task_struct *p) {
    return trace_enqueue(p);
}

SEC("tp_btf/sched_wakeup_new")
int BPF_PROG(sched_wakeup_new, struct task_struct *p) {
    return trace_enqueue(p);
}

SEC("tp_btf/sched_migrate_task")
int BPF_PROG(sched_migrate_task, struct task_struct *p, int new_cpu) {
    __u32 tid = BPF_CORE_READ(p, pid);
    struct enqueue_val *val = bpf_map_lookup_elem(&enqueue_ts, &tid);
    if (val) {
        val->target_cpu = (__u32)new_cpu;
        val->flags |= KS_FLAG_MIGRATED;
    }
    return 0;
}

SEC("tp_btf/sched_switch")
int BPF_PROG(sched_switch, bool preempt, struct task_struct *prev, struct task_struct *next, unsigned int prev_state) {
    __u64 now = bpf_ktime_get_ns();
    __u32 cpu = bpf_get_smp_processor_id();
    struct kstall_bpf_config *cfg = get_config();
    __u64 runq_threshold = cfg ? cfg->runq_threshold_ns : 2000000ULL; /* default 2ms */

    __u32 prev_pid = BPF_CORE_READ(prev, pid);
    __u32 next_pid = BPF_CORE_READ(next, pid);

    /* 1. Track preempted task: if prev was still runnable, it was re-queued without wakeup */
    if (prev_state == 0 && prev_pid != 0) {
        if (should_trace_task(prev, cfg)) {
            struct enqueue_val pval = {};
            pval.ts = now;
            pval.target_cpu = cpu;
            pval.flags = KS_FLAG_PREEMPTED;
            pval.culprit_tid = next_pid;
            pval.culprit_tgid = BPF_CORE_READ(next, tgid);
            pval.culprit_cgid = 0;
            bpf_probe_read_kernel_str(pval.culprit_comm, TASK_COMM_LEN, next->comm);
            bpf_map_update_elem(&enqueue_ts, &prev_pid, &pval, BPF_ANY);
        }
    }

    /* 2. Check if next task was waiting on runqueue */
    if (next_pid != 0) {
        struct enqueue_val *val = bpf_map_lookup_elem(&enqueue_ts, &next_pid);
        if (val) {
            __u64 delta = now - val->ts;

            if (delta >= runq_threshold) {
                __u64 victim_cgid = bpf_get_current_cgroup_id();

                /* Outlier detected! Emit event */
                struct kstall_event *ev = bpf_ringbuf_reserve(&events, sizeof(*ev), 0);
                if (ev) {
                    ev->abi_version = KSTALL_ABI_VERSION;
                    ev->type = (val->flags & KS_FLAG_PREEMPTED) ? KS_EV_PREEMPT_RUNQ : KS_EV_RUNQ;
                    ev->ts_start_ns = val->ts;
                    ev->ts_end_ns = now;
                    ev->duration_ns = delta;
                    ev->tid = next_pid;
                    ev->tgid = BPF_CORE_READ(next, tgid);
                    ev->cpu_start = val->target_cpu;
                    ev->cpu_end = cpu;
                    ev->cgroup_id = victim_cgid;
                    bpf_probe_read_kernel_str(ev->comm, TASK_COMM_LEN, next->comm);

                    ev->flags = val->flags;

                    /* Check if victim cgroup is currently throttled */
                    if (bpf_map_lookup_elem(&throttle_state, &victim_cgid)) {
                        ev->flags |= KS_FLAG_THROTTLED_OVERLAP;
                    }

                    ev->kstack_id = -1;
                    ev->_pad = 0;

                    /* Culprit 0: the task that just ran and surrendered CPU to victim */
                    ev->num_culprits = 1;
                    ev->culprits[0].tid = prev_pid;
                    ev->culprits[0].tgid = BPF_CORE_READ(prev, tgid);
                    ev->culprits[0].cgroup_id = 0;
                    ev->culprits[0].stolen_ns = delta;
                    bpf_probe_read_kernel_str(ev->culprits[0].comm, TASK_COMM_LEN, prev->comm);

                    /* Flag kernel threads (e.g. kswapd, kworker, or PF_KTHREAD) */
                    unsigned int prev_flags = BPF_CORE_READ(prev, flags);
                    ev->culprits[0].flags = (prev_flags & PF_KTHREAD_FLAG) ? KS_FLAG_KERNEL_THREAD : 0;
                    ev->culprits[0]._pad = 0;

                    /* Culprit 1: occupant when wakeup occurred (if known and different) */
                    if (val->culprit_tid != 0 && val->culprit_tid != prev_pid) {
                        ev->num_culprits = 2;
                        ev->culprits[1].tid = val->culprit_tid;
                        ev->culprits[1].tgid = val->culprit_tgid;
                        ev->culprits[1].cgroup_id = val->culprit_cgid;
                        ev->culprits[1].stolen_ns = 0;
                        __builtin_memcpy(ev->culprits[1].comm, val->culprit_comm, TASK_COMM_LEN);
                        ev->culprits[1].flags = 0;
                        ev->culprits[1]._pad = 0;
                    }

                    bpf_ringbuf_submit(ev, 0);
                    inc_stat_emitted();
                } else {
                    inc_stat_dropped();
                }
            } else {
                inc_stat_filtered();
            }

            bpf_map_delete_elem(&enqueue_ts, &next_pid);
        }
    }

    /* 3. Update current occupant for this CPU */
    __u32 zero = 0;
    struct cpu_occupant *curr = bpf_map_lookup_elem(&cpu_curr, &zero);
    if (curr) {
        curr->tid = next_pid;
        curr->tgid = BPF_CORE_READ(next, tgid);
        curr->since_ts = now;
        curr->cgid = bpf_get_current_cgroup_id();
        bpf_probe_read_kernel_str(curr->comm, TASK_COMM_LEN, next->comm);
    }

    return 0;
}

/* Phase 2: Direct Reclaim */
SEC("tp_btf/mm_vmscan_direct_reclaim_begin")
int BPF_PROG(mm_vmscan_direct_reclaim_begin, int order, gfp_t gfp_flags) {
    __u32 tid = (__u32)bpf_get_current_pid_tgid();
    __u64 now = bpf_ktime_get_ns();
    bpf_map_update_elem(&reclaim_ts, &tid, &now, BPF_ANY);
    return 0;
}

SEC("tp_btf/mm_vmscan_direct_reclaim_end")
int BPF_PROG(mm_vmscan_direct_reclaim_end, unsigned long nr_reclaimed) {
    __u32 tid = (__u32)bpf_get_current_pid_tgid();
    __u64 *start_ts = bpf_map_lookup_elem(&reclaim_ts, &tid);
    if (!start_ts)
        return 0;

    __u64 now = bpf_ktime_get_ns();
    __u64 delta = now - *start_ts;
    bpf_map_delete_elem(&reclaim_ts, &tid);

    struct kstall_bpf_config *cfg = get_config();
    __u64 threshold = cfg ? cfg->reclaim_threshold_ns : 1000000ULL; /* default 1ms */

    if (delta >= threshold) {
        struct kstall_event *ev = bpf_ringbuf_reserve(&events, sizeof(*ev), 0);
        if (ev) {
            ev->abi_version = KSTALL_ABI_VERSION;
            ev->type = KS_EV_RECLAIM;
            ev->ts_start_ns = *start_ts;
            ev->ts_end_ns = now;
            ev->duration_ns = delta;
            ev->tid = tid;
            ev->tgid = (__u32)(bpf_get_current_pid_tgid() >> 32);
            ev->cpu_start = bpf_get_smp_processor_id();
            ev->cpu_end = ev->cpu_start;
            ev->cgroup_id = bpf_get_current_cgroup_id();
            bpf_get_current_comm(ev->comm, TASK_COMM_LEN);
            ev->num_culprits = 0;
            ev->flags = 0;
            ev->kstack_id = -1;
            ev->_pad = 0;
            bpf_ringbuf_submit(ev, 0);
            inc_stat_emitted();
        } else {
            inc_stat_dropped();
        }
    }
    return 0;
}

/* Phase 2: Compaction */
SEC("tp_btf/mm_compaction_begin")
int BPF_PROG(mm_compaction_begin, struct compact_control *cc, unsigned long zone_idx, unsigned long order, bool sync) {
    __u32 tid = (__u32)bpf_get_current_pid_tgid();
    __u64 now = bpf_ktime_get_ns();
    bpf_map_update_elem(&compact_ts, &tid, &now, BPF_ANY);
    return 0;
}

SEC("tp_btf/mm_compaction_end")
int BPF_PROG(mm_compaction_end, struct compact_control *cc, unsigned long zone_idx, unsigned long order, bool sync, int status) {
    __u32 tid = (__u32)bpf_get_current_pid_tgid();
    __u64 *start_ts = bpf_map_lookup_elem(&compact_ts, &tid);
    if (!start_ts)
        return 0;

    __u64 now = bpf_ktime_get_ns();
    __u64 delta = now - *start_ts;
    bpf_map_delete_elem(&compact_ts, &tid);

    struct kstall_bpf_config *cfg = get_config();
    __u64 threshold = cfg ? cfg->reclaim_threshold_ns : 1000000ULL; /* default 1ms */

    if (delta >= threshold) {
        struct kstall_event *ev = bpf_ringbuf_reserve(&events, sizeof(*ev), 0);
        if (ev) {
            ev->abi_version = KSTALL_ABI_VERSION;
            ev->type = KS_EV_COMPACT;
            ev->ts_start_ns = *start_ts;
            ev->ts_end_ns = now;
            ev->duration_ns = delta;
            ev->tid = tid;
            ev->tgid = (__u32)(bpf_get_current_pid_tgid() >> 32);
            ev->cpu_start = bpf_get_smp_processor_id();
            ev->cpu_end = ev->cpu_start;
            ev->cgroup_id = bpf_get_current_cgroup_id();
            bpf_get_current_comm(ev->comm, TASK_COMM_LEN);
            ev->num_culprits = 0;
            ev->flags = 0;
            ev->kstack_id = -1;
            ev->_pad = 0;
            bpf_ringbuf_submit(ev, 0);
            inc_stat_emitted();
        } else {
            inc_stat_dropped();
        }
    }
    return 0;
}

/* Phase 2: CFS Throttling Hooks */
SEC("kprobe/throttle_cfs_rq")
int BPF_KPROBE(trace_throttle_cfs_rq, struct cfs_rq *cfs_rq) {
    struct cgroup *cg = BPF_CORE_READ(cfs_rq, tg, css.cgroup);
    __u64 cgid = BPF_CORE_READ(cg, kn, id);
    if (!cgid)
        return 0;

    __u64 now = bpf_ktime_get_ns();
    bpf_map_update_elem(&throttle_state, &cgid, &now, BPF_ANY);
    return 0;
}

SEC("kprobe/unthrottle_cfs_rq")
int BPF_KPROBE(trace_unthrottle_cfs_rq, struct cfs_rq *cfs_rq) {
    struct cgroup *cg = BPF_CORE_READ(cfs_rq, tg, css.cgroup);
    __u64 cgid = BPF_CORE_READ(cg, kn, id);
    if (!cgid)
        return 0;

    __u64 *start_ts = bpf_map_lookup_elem(&throttle_state, &cgid);
    if (!start_ts)
        return 0;

    __u64 now = bpf_ktime_get_ns();
    __u64 delta = now - *start_ts;
    bpf_map_delete_elem(&throttle_state, &cgid);

    struct kstall_bpf_config *cfg = get_config();
    __u64 threshold = cfg ? cfg->throttle_threshold_ns : 1000000ULL; /* default 1ms */

    if (delta >= threshold) {
        struct kstall_event *ev = bpf_ringbuf_reserve(&events, sizeof(*ev), 0);
        if (ev) {
            ev->abi_version = KSTALL_ABI_VERSION;
            ev->type = KS_EV_THROTTLE;
            ev->ts_start_ns = *start_ts;
            ev->ts_end_ns = now;
            ev->duration_ns = delta;
            ev->tid = 0;
            ev->tgid = 0;
            ev->cpu_start = bpf_get_smp_processor_id();
            ev->cpu_end = ev->cpu_start;
            ev->cgroup_id = cgid;
            ev->comm[0] = '\0';
            ev->num_culprits = 0;
            ev->flags = KS_FLAG_THROTTLED_OVERLAP;
            ev->kstack_id = -1;
            ev->_pad = 0;
            bpf_ringbuf_submit(ev, 0);
            inc_stat_emitted();
        } else {
            inc_stat_dropped();
        }
    }
    return 0;
}
