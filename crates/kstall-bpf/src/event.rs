// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use kstall_core::model::{CauseKind, Culprit, EventRecord};

pub const KSTALL_ABI_VERSION: u32 = 1;
pub const TASK_COMM_LEN: usize = 16;
pub const KSTALL_MAX_CULPRITS: usize = 3;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RawCulprit {
    pub tid: u32,
    pub tgid: u32,
    pub cgroup_id: u64,
    pub stolen_ns: u64,
    pub comm: [u8; TASK_COMM_LEN],
    pub flags: u32,
    pub _pad: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RawEvent {
    pub abi_version: u32,
    pub event_type: u32,
    pub ts_start_ns: u64,
    pub ts_end_ns: u64,
    pub duration_ns: u64,
    pub tid: u32,
    pub tgid: u32,
    pub cpu_start: u32,
    pub cpu_end: u32,
    pub cgroup_id: u64,
    pub comm: [u8; TASK_COMM_LEN],
    pub culprits: [RawCulprit; KSTALL_MAX_CULPRITS],
    pub num_culprits: u32,
    pub flags: u32,
    pub kstack_id: i32,
    pub _pad: u32,
}

impl RawEvent {
    /// Decode a null-terminated C string buffer into a safe Rust String.
    fn decode_comm(bytes: &[u8]) -> String {
        let len = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..len]).to_string()
    }

    /// Convert raw C ABI event from BPF ring buffer into safe Rust `EventRecord`.
    pub fn to_record(&self) -> EventRecord {
        let cause = match self.event_type {
            1 => CauseKind::RunqueueWait,
            2 => CauseKind::PreemptedWait,
            3 => CauseKind::DirectReclaim,
            4 => CauseKind::Compaction,
            5 => CauseKind::Throttled,
            _ => CauseKind::Unknown,
        };

        let num_culprits = (self.num_culprits as usize).min(KSTALL_MAX_CULPRITS);
        let mut culprits = Vec::with_capacity(num_culprits);

        for i in 0..num_culprits {
            let raw_c = &self.culprits[i];
            culprits.push(Culprit {
                tid: raw_c.tid,
                tgid: raw_c.tgid,
                comm: Self::decode_comm(&raw_c.comm),
                cgroup_id: raw_c.cgroup_id,
                stolen_ns: raw_c.stolen_ns,
                flags: raw_c.flags,
            });
        }

        EventRecord {
            abi_version: self.abi_version,
            cause,
            ts_start_ns: self.ts_start_ns,
            ts_end_ns: self.ts_end_ns,
            duration_ns: self.duration_ns,
            tid: self.tid,
            tgid: self.tgid,
            cpu_start: self.cpu_start,
            cpu_end: self.cpu_end,
            cgroup_id: self.cgroup_id,
            comm: Self::decode_comm(&self.comm),
            culprits,
            flags: self.flags,
            kstack_id: self.kstack_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_event_size_alignment() {
        assert_eq!(std::mem::size_of::<RawCulprit>(), 48);
        assert_eq!(std::mem::size_of::<RawEvent>(), 232);
        assert_eq!(std::mem::align_of::<RawEvent>(), 8);
    }
}
