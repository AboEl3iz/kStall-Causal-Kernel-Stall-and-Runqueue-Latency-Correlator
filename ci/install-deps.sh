#!/usr/bin/env bash
# SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
# kStall - CI & Developer Dependency Installer
set -euo pipefail

echo "==> Detecting package manager and installing dependencies..."

if command -v apt-get &>/dev/null; then
    echo "==> Debian/Ubuntu detected. Installing dependencies via apt..."
    sudo apt-get update -y
    sudo apt-get install -y --no-install-recommends \
        build-essential \
        clang \
        llvm \
        libelf-dev \
        libbpf-dev \
        linux-libc-dev \
        pkg-config \
        zlib1g-dev \
        bpftool \
        curl \
        git \
        ca-certificates
elif command -v dnf &>/dev/null; then
    echo "==> Fedora/RHEL detected. Installing dependencies via dnf..."
    sudo dnf install -y \
        clang \
        llvm \
        elfutils-libelf-devel \
        libbpf-devel \
        bpftool \
        pkgconfig \
        zlib-devel \
        make \
        gcc \
        git
elif command -v pacman &>/dev/null; then
    echo "==> Arch Linux detected. Installing dependencies via pacman..."
    sudo pacman -Sy --noconfirm --needed \
        base-devel \
        clang \
        llvm \
        libelf \
        libbpf \
        bpf \
        pkgconf \
        zlib \
        git
else
    echo "==> Warning: Unrecognized package manager. Please ensure clang, llvm, libbpf-dev, bpftool, and pkg-config are installed."
fi

# Ensure Rust components are present
if command -v rustup &>/dev/null; then
    echo "==> Ensuring Rust components (rustfmt, clippy) are installed..."
    rustup component add rustfmt clippy || true
fi

echo "==> Dependencies successfully installed."
