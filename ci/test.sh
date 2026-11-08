#!/usr/bin/env bash
# SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
# kStall - Automated CI Test Suite
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

echo "======================================================="
echo "               kStall Automated CI Pipeline            "
echo "======================================================="

echo "==> 1. Verifying toolchain environment..."
echo "  - Rust:    $(rustc --version 2>&1 || echo 'Not found')"
echo "  - Cargo:   $(cargo --version 2>&1 || echo 'Not found')"
echo "  - Clang:   $(clang --version 2>&1 | head -n 1 || echo 'Not found')"
echo "  - Bpftool: $(bpftool version 2>&1 | head -n 1 || echo 'Not found')"

echo "==> 2. Building eBPF Data Plane..."
make -C bpf all

echo "==> 3. Running Lint and Static Analysis..."
"${SCRIPT_DIR}/lint.sh"

echo "==> 4. Running Unit and Integration Tests..."
cargo test --workspace -- --nocapture

echo "==> 5. Building workspace release binaries..."
cargo build --workspace --release

echo "==> 6. Testing CLI Subcommands & Diagnostics..."
TARGET_BIN="./target/release/kstall"
if [[ ! -f "${TARGET_BIN}" ]]; then
    TARGET_BIN="./target/debug/kstall"
fi

"${TARGET_BIN}" --version
"${TARGET_BIN}" --help > /dev/null
"${TARGET_BIN}" doctor || true

echo "==> 7. Validating Golden Sample replay and exports..."
GOLDEN_SAMPLE="tests/workloads/golden_sample.jsonl"
if [[ -f "${GOLDEN_SAMPLE}" ]]; then
    TMP_OUT=$(mktemp -d)
    "${TARGET_BIN}" export --input "${GOLDEN_SAMPLE}" --format chrome --output "${TMP_OUT}/trace.json"
    "${TARGET_BIN}" export --input "${GOLDEN_SAMPLE}" --format csv --output "${TMP_OUT}/summary.csv"
    "${TARGET_BIN}" export --input "${GOLDEN_SAMPLE}" --format csv-summary --output "${TMP_OUT}/agg_summary.csv"
    test -s "${TMP_OUT}/trace.json"
    test -s "${TMP_OUT}/summary.csv"
    test -s "${TMP_OUT}/agg_summary.csv"
    rm -rf "${TMP_OUT}"
    echo "    Golden sample trace & CSV export verified."
fi

echo "======================================================="
echo "   All kStall CI checks completed successfully!        "
echo "======================================================="
