#!/usr/bin/env bash
# SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
# kStall - CI Linting and Static Analysis Script
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

echo "==> 1. Checking Rust formatting (cargo fmt --check)..."
cargo fmt --all -- --check

echo "==> 2. Running Cargo Check across workspace..."
cargo check --workspace --all-targets

echo "==> 3. Running Clippy linter with strict warnings..."
cargo clippy --workspace --all-targets -- -D warnings

echo "==> 4. Validating Shell scripts syntax..."
if command -v shellcheck &>/dev/null; then
    find ci deploy -name "*.sh" -exec shellcheck {} +
    echo "    ShellCheck passed."
else
    # Basic bash syntax check if shellcheck is not installed
    for f in $(find ci deploy -name "*.sh"); do
        bash -n "$f"
    done
    echo "    Bash syntax validation passed (shellcheck not found, checked with bash -n)."
fi

echo "==> All linting and static analysis checks passed successfully!"
