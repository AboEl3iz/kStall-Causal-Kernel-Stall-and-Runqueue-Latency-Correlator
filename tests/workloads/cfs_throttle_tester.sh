#!/usr/bin/env bash
# SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
# FI-3: CFS Bandwidth Throttling Injector (PRD §16.5)

set -euo pipefail

CGROUP_ROOT="/sys/fs/cgroup"
TEST_CGROUP="${CGROUP_ROOT}/kstall_throttle_test"

if [ ! -w "${CGROUP_ROOT}" ]; then
    echo "[!] Root privileges or write access to ${CGROUP_ROOT} required for cgroup manipulation."
    exit 1
fi

echo "==> Creating temporary cgroup v2: ${TEST_CGROUP}"
mkdir -p "${TEST_CGROUP}"

cleanup() {
    echo "==> Cleaning up cgroup ${TEST_CGROUP}..."
    rmdir "${TEST_CGROUP}" 2>/dev/null || true
}
trap cleanup EXIT

# Set tight quota: 20ms quota per 100ms period (0.2 CPU core)
echo "20000 100000" > "${TEST_CGROUP}/cpu.max"
echo "==> Set cpu.max to 20000 100000 (20ms / 100ms)"

echo "==> Launching CPU burst inside ${TEST_CGROUP} to induce throttling..."
# Put current subshell into cgroup
(
    echo $$ > "${TEST_CGROUP}/cgroup.procs"
    # Busy spin for 500ms
    start_ts=$(date +%s%N)
    while [ $(($(date +%s%N) - start_ts)) -lt 500000000 ]; do
        :
    done
)

echo "==> Check cpu.stat in test cgroup:"
cat "${TEST_CGROUP}/cpu.stat" | grep -E "nr_throttled|throttled_usec"
echo "==> FI-3 CFS Throttling injection complete."
