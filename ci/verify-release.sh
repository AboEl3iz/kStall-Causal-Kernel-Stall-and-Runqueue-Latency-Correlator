#!/usr/bin/env bash
# SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
# kStall - Release Artifact Verification & Smoke Test
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

OUT_DIR="${ROOT_DIR}/dist"
ARCHIVE="${1:-}"

if [[ -z "${ARCHIVE}" ]]; then
    ARCHIVE=$(find "${OUT_DIR}" -maxdepth 1 -name "kstall-*.tar.gz" ! -name "*-standalone.tar.gz" | head -n 1 || true)
fi

if [[ -z "${ARCHIVE}" || ! -f "${ARCHIVE}" ]]; then
    echo "Error: Release archive not found. Run ci/release.sh first or pass archive path as argument." >&2
    exit 1
fi

echo "======================================================="
echo "        kStall Release Archive Verification            "
echo "======================================================="
echo "  Archive: ${ARCHIVE}"

# 1. Checksum validation
CHECKSUM_FILE="${ARCHIVE}.sha256"
if [[ -f "${CHECKSUM_FILE}" ]]; then
    echo "==> 1. Validating SHA-256 Checksum..."
    cd "$(dirname "${ARCHIVE}")"
    sha256sum -c "$(basename "${CHECKSUM_FILE}")"
    cd "${ROOT_DIR}"
    echo "    Checksum OK."
fi

# 2. Extract into temporary sandbox
echo "==> 2. Extracting archive to sandbox..."
SANDBOX=$(mktemp -d)
trap 'rm -rf "${SANDBOX}"' EXIT

tar -xzf "${ARCHIVE}" -C "${SANDBOX}"
EXTRACTED_DIR=$(find "${SANDBOX}" -mindepth 1 -maxdepth 1 -type d | head -n 1)

if [[ -z "${EXTRACTED_DIR}" ]]; then
    echo "Error: Archive did not extract into a root directory." >&2
    exit 1
fi

echo "==> 3. Verifying package structure..."
test -f "${EXTRACTED_DIR}/kstall" || { echo "Missing kstall binary!"; exit 1; }
test -f "${EXTRACTED_DIR}/README.md" || { echo "Missing README.md!"; exit 1; }
test -x "${EXTRACTED_DIR}/kstall" || { echo "kstall binary is not executable!"; exit 1; }

echo "    Structure OK."

# 4. Binary smoke test
echo "==> 4. Running binary sanity checks..."
EXT_BIN="${EXTRACTED_DIR}/kstall"

echo "  -> Checking version output:"
"${EXT_BIN}" --version

echo "  -> Checking help output:"
"${EXT_BIN}" --help > /dev/null

echo "  -> Checking doctor diagnostic:"
"${EXT_BIN}" doctor || true

echo "  -> Checking export subcommand help:"
"${EXT_BIN}" export --help > /dev/null

echo "  -> Checking replay subcommand help:"
"${EXT_BIN}" replay --help > /dev/null

echo "==> 5. Inspecting ELF attributes & dependencies..."
if command -v file &>/dev/null; then
    file "${EXT_BIN}"
fi

if command -v ldd &>/dev/null; then
    echo "  -> Dynamic dependencies:"
    ldd "${EXT_BIN}" || true
fi

echo "======================================================="
echo "   Release archive passed all verification checks!     "
echo "======================================================="
