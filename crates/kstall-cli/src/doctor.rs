// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use anyhow::Result;
use std::fs;
use std::path::Path;

struct CheckItem {
    name: &'static str,
    passed: bool,
    details: String,
    critical: bool,
}

pub fn run_doctor() -> Result<i32> {
    println!("kStall Doctor - Kernel & Environment Health Diagnostic");
    println!("=======================================================\n");

    let mut checks = Vec::new();

    // 1. Kernel Release
    let release = match fs::read_to_string("/proc/sys/kernel/osrelease") {
        Ok(s) => s.trim().to_string(),
        Err(_) => "unknown".to_string(),
    };
    checks.push(CheckItem {
        name: "Kernel Version",
        passed: true,
        details: format!("Detected: {}", release),
        critical: false,
    });

    // 2. BTF Support
    let btf_path = Path::new("/sys/kernel/btf/vmlinux");
    let btf_ok = btf_path.exists();
    checks.push(CheckItem {
        name: "Kernel BTF (/sys/kernel/btf/vmlinux)",
        passed: btf_ok,
        details: if btf_ok {
            "Present (CO-RE supported)".to_string()
        } else {
            "Missing - kernel must be built with CONFIG_DEBUG_INFO_BTF=y".to_string()
        },
        critical: true,
    });

    // 3. Tracefs mount
    let tracefs_ok = Path::new("/sys/kernel/tracing").exists()
        || Path::new("/sys/kernel/debug/tracing").exists();
    checks.push(CheckItem {
        name: "Tracefs / Tracing Mount",
        passed: tracefs_ok,
        details: if tracefs_ok {
            "Mounted".to_string()
        } else {
            "Missing - mount tracefs on /sys/kernel/tracing".to_string()
        },
        critical: true,
    });

    // 4. Cgroup v2
    let cgroup2_ok = Path::new("/sys/fs/cgroup/cgroup.controllers").exists();
    checks.push(CheckItem {
        name: "Cgroup v2 Unified Hierarchy",
        passed: cgroup2_ok,
        details: if cgroup2_ok {
            "Active (unified cgroup v2)".to_string()
        } else {
            "Degraded (legacy cgroup v1 or missing controllers)".to_string()
        },
        critical: false,
    });

    // 5. User permissions (root / CAP_BPF)
    let is_root = unsafe { libc::geteuid() == 0 };
    checks.push(CheckItem {
        name: "Process Privileges",
        passed: is_root,
        details: if is_root {
            "Running as root (CAP_BPF & CAP_PERFMON available)".to_string()
        } else {
            "Not running as root - requires CAP_BPF / CAP_PERFMON".to_string()
        },
        critical: true,
    });

    // 6. Sysctl unprivileged_bpf_disabled
    if let Ok(val) = fs::read_to_string("/proc/sys/kernel/unprivileged_bpf_disabled") {
        let val = val.trim();
        checks.push(CheckItem {
            name: "sysctl unprivileged_bpf_disabled",
            passed: true,
            details: format!(
                "Value: {} ({})",
                val,
                match val {
                    "0" => "unprivileged bpf enabled",
                    "1" => "unprivileged bpf disabled",
                    "2" => "unprivileged bpf permanently disabled",
                    _ => "custom",
                }
            ),
            critical: false,
        });
    }

    let mut has_critical_failure = false;

    for item in checks {
        let status_mark = if item.passed {
            "\x1b[32m[PASS]\x1b[0m"
        } else if item.critical {
            has_critical_failure = true;
            "\x1b[31m[FAIL]\x1b[0m"
        } else {
            "\x1b[33m[WARN]\x1b[0m"
        };

        println!("{:<38} {} - {}", item.name, status_mark, item.details);
    }

    println!("\n-------------------------------------------------------");
    if !is_root {
        eprintln!("\x1b[31mStatus: System is NOT fully ready to run kStall.\x1b[0m");
        eprintln!("Resolve critical [FAIL] items before starting BPF tracing (root / CAP_BPF / CAP_PERFMON required).");
        Ok(3)
    } else if has_critical_failure {
        eprintln!("\x1b[31mStatus: System is NOT fully ready to run kStall.\x1b[0m");
        eprintln!("Resolve critical [FAIL] items before starting BPF tracing.");
        Ok(2)
    } else {
        println!("\x1b[32mStatus: All critical environment checks passed! System ready.\x1b[0m");
        Ok(0)
    }
}
