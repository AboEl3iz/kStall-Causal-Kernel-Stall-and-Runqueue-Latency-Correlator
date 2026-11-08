# kStall Developer Guide

## Prerequisites

- Linux 5.15+ kernel with BTF enabled (`/sys/kernel/btf/vmlinux`)
- Clang & LLVM (version 14+)
- `bpftool`
- Rust 1.80+ (Stable)
- `libbpf` development package (`libbpf-dev` on Debian/Ubuntu, `libbpf` on Arch/Fedora)

---

## Quick Start

### 1. Build BPF Object and Rust Crates
```bash
make all
```

### 2. Run Capability Diagnostics
```bash
make doctor
# Or run binary directly:
./target/debug/kstall doctor
```

### 3. Run Unit and Integration Tests
```bash
make test
```

### 4. Run Live Trace with Text Output (Requires sudo/CAP_BPF)
```bash
sudo ./target/debug/kstall watch --threshold 2ms
```

### 5. Run Live TUI Mode
```bash
sudo ./target/debug/kstall top --threshold 2ms
```

---

## Repository Structure & Guidelines

- **Shared ABI:** Any modification to `struct kstall_event` must update both `bpf/kstall.h` and `crates/kstall-bpf/src/event.rs`. Keep fields 8-byte aligned and avoid compiler padding discrepancies.
- **Kernel-Free Core:** All causal assembly logic and quantile calculations in `crates/kstall-core` must remain independent of kernel headers or BPF libraries to allow fast, deterministic offline unit testing.
