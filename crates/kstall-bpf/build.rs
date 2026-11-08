// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use libbpf_cargo::SkeletonBuilder;
use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));
    let skel = out_dir.join("kstall.skel.rs");

    let bpf_src = Path::new("../../bpf/kstall.bpf.c");
    let bpf_header = Path::new("../../bpf/kstall.h");

    println!("cargo:rerun-if-changed={}", bpf_src.display());
    println!("cargo:rerun-if-changed={}", bpf_header.display());

    let arch = match env::consts::ARCH {
        "x86_64" => "x86",
        "aarch64" => "arm64",
        other => other,
    };

    let target_def = format!("-D__TARGET_ARCH_{}", arch);

    if let Err(err) = SkeletonBuilder::new()
        .source(bpf_src)
        .clang_args([
            "-I../../bpf",
            &target_def,
            "-O2",
            "-g",
            "-Wno-missing-declarations",
            "-Wno-unknown-attributes",
        ])
        .build_and_generate(&skel)
    {
        panic!(
            "Failed to generate BPF skeleton from {}: {}",
            bpf_src.display(),
            err
        );
    }
}
