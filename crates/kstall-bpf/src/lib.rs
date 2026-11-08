// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

pub mod event;
pub use event::{RawCulprit, RawEvent, KSTALL_ABI_VERSION, KSTALL_MAX_CULPRITS, TASK_COMM_LEN};

#[allow(clippy::all)]
#[allow(non_upper_case_globals)]
#[allow(non_snake_case)]
#[allow(non_camel_case_types)]
#[allow(dead_code)]
mod bpf_skel {
    include!(concat!(env!("OUT_DIR"), "/kstall.skel.rs"));
}

use anyhow::{Context, Result};
pub use bpf_skel::*;
use kstall_core::model::EventRecord;
use libbpf_rs::skel::{OpenSkel, Skel, SkelBuilder};
use libbpf_rs::{MapCore, RingBufferBuilder};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

/// Configuration options for kStall eBPF subsystem.
#[derive(Debug, Clone)]
pub struct BpfConfig {
    pub runq_threshold_ns: u64,
    pub reclaim_threshold_ns: u64,
    pub throttle_threshold_ns: u64,
    pub target_pid: u32,
    pub target_tgid: u32,
    pub target_cgroup_id: u64,
}

impl Default for BpfConfig {
    fn default() -> Self {
        Self {
            runq_threshold_ns: 2_000_000,     // 2 ms default
            reclaim_threshold_ns: 1_000_000,  // 1 ms default
            throttle_threshold_ns: 1_000_000, // 1 ms default
            target_pid: 0,
            target_tgid: 0,
            target_cgroup_id: 0,
        }
    }
}

/// BPF Manager that loads programs and drains the ring buffer.
pub struct BpfManager {
    _skel: KstallSkel<'static>,
}

impl BpfManager {
    /// Load and attach kStall BPF programs.
    pub fn start(
        config: BpfConfig,
        event_tx: mpsc::Sender<EventRecord>,
        stop_flag: Arc<AtomicBool>,
    ) -> Result<Self> {
        info!("Opening kStall BPF skeleton...");
        let skel_builder = KstallSkelBuilder::default();
        let open_obj: &'static mut std::mem::MaybeUninit<libbpf_rs::OpenObject> =
            Box::leak(Box::new(std::mem::MaybeUninit::uninit()));
        let open_skel = skel_builder
            .open(open_obj)
            .context("Failed to open BPF skeleton")?;

        info!("Loading BPF programs into kernel...");
        let mut skel = open_skel.load().context("Failed to load BPF skeleton")?;

        // Populate config map
        let bpf_cfg = types::kstall_bpf_config {
            runq_threshold_ns: config.runq_threshold_ns,
            reclaim_threshold_ns: config.reclaim_threshold_ns,
            throttle_threshold_ns: config.throttle_threshold_ns,
            target_pid: config.target_pid,
            target_tgid: config.target_tgid,
            target_cgroup_id: config.target_cgroup_id,
            agent_pid: std::process::id(),
            flags: 0,
        };

        let zero_key: u32 = 0;
        let cfg_bytes = unsafe {
            std::slice::from_raw_parts(
                &bpf_cfg as *const _ as *const u8,
                std::mem::size_of::<types::kstall_bpf_config>(),
            )
        };
        let key_bytes = zero_key.to_ne_bytes();
        skel.maps
            .config_map
            .update(&key_bytes, cfg_bytes, libbpf_rs::MapFlags::ANY)
            .context("Failed to update config_map")?;

        info!("Attaching tracepoints...");
        skel.attach().context("Failed to attach BPF programs")?;

        // Leak skeleton to obtain 'static lifetime for ring buffer polling thread
        let skel_box = Box::new(skel);
        let skel_static: &'static mut KstallSkel<'static> = Box::leak(skel_box);

        // Set up ring buffer poller thread
        let mut rb_builder = RingBufferBuilder::new();
        let tx = event_tx;

        rb_builder
            .add(&skel_static.maps.events, move |data: &[u8]| {
                if data.len() < std::mem::size_of::<RawEvent>() {
                    error!("Truncated ring buffer event: {} bytes", data.len());
                    return 0;
                }

                let raw_ptr = data.as_ptr() as *const RawEvent;
                let raw_ev = unsafe { *raw_ptr };
                let record = raw_ev.to_record();

                if let Err(e) = tx.blocking_send(record) {
                    debug!("Event receiver dropped: {:?}", e);
                    return 1;
                }
                0
            })
            .context("Failed to add ring buffer handler")?;

        let ringbuf = rb_builder.build().context("Failed to build ring buffer")?;

        std::thread::spawn(move || {
            debug!("Ring buffer poller started");
            while !stop_flag.load(Ordering::Relaxed) {
                if let Err(e) = ringbuf.poll(Duration::from_millis(50)) {
                    error!("Ring buffer poll error: {:?}", e);
                    break;
                }
            }
            debug!("Ring buffer poller stopped");
        });

        Ok(Self {
            _skel: unsafe { std::ptr::read(skel_static) },
        })
    }
}
