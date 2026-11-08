#!/usr/bin/env bash
# SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
# kStall - Automated Release Packaging Script
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

# Defaults
OUT_DIR="${ROOT_DIR}/dist"
TARGET=""
SKIP_BUILD=false
VERSION=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)
            TARGET="$2"
            shift 2
            ;;
        --version)
            VERSION="$2"
            shift 2
            ;;
        --out-dir)
            OUT_DIR="$2"
            shift 2
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [options]"
            echo "Options:"
            echo "  --target <TRIPLE>   Target architecture triple (e.g., x86_64-unknown-linux-gnu)"
            echo "  --version <VERSION> Version string (defaults to Cargo.toml package version)"
            echo "  --out-dir <DIR>     Output directory for release archives (default: ./dist)"
            echo "  --skip-build        Skip cargo build step (use existing target/release binary)"
            echo "  -h, --help          Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

# Detect Version if not set
if [[ -z "${VERSION}" ]]; then
    VERSION=$(grep -m 1 '^version = ' Cargo.toml | cut -d '"' -f 2 || true)
    if [[ -z "${VERSION}" ]]; then
        VERSION="0.1.0"
    fi
fi

# Detect Target if not set
if [[ -z "${TARGET}" ]]; then
    ARCH=$(uname -m)
    case "${ARCH}" in
        x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
        aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
        arm64)   TARGET="aarch64-unknown-linux-gnu" ;;
        *)       TARGET="${ARCH}-unknown-linux-gnu" ;;
    esac
fi

echo "======================================================="
echo "               kStall Release Packager                 "
echo "======================================================="
echo "  Version:     v${VERSION}"
echo "  Target:      ${TARGET}"
echo "  Output Dir:  ${OUT_DIR}"
echo "======================================================="

# Build release binaries
if [[ "${SKIP_BUILD}" == false ]]; then
    echo "==> 1. Compiling BPF data plane..."
    make -C bpf all

    echo "==> 2. Compiling Rust workspace in release mode..."
    if [[ -n "${TARGET}" && "${TARGET}" != "$(uname -m)-unknown-linux-gnu" ]]; then
        cargo build --release --workspace --target "${TARGET}"
        BIN_SRC="${ROOT_DIR}/target/${TARGET}/release/kstall"
    else
        cargo build --release --workspace
        BIN_SRC="${ROOT_DIR}/target/release/kstall"
    fi
else
    echo "==> Skipping build as requested."
    BIN_SRC="${ROOT_DIR}/target/release/kstall"
    if [[ -n "${TARGET}" && -f "${ROOT_DIR}/target/${TARGET}/release/kstall" ]]; then
        BIN_SRC="${ROOT_DIR}/target/${TARGET}/release/kstall"
    fi
fi

if [[ ! -f "${BIN_SRC}" ]]; then
    echo "Error: Binary not found at ${BIN_SRC}" >&2
    exit 1
fi

# Create staging directory
RELEASE_NAME="kstall-v${VERSION}-${TARGET}"
STAGE_DIR="${OUT_DIR}/${RELEASE_NAME}"
mkdir -p "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}/docs"
mkdir -p "${STAGE_DIR}/deploy"

echo "==> 3. Copying binaries and release assets to staging area..."
# Strip binary copy
cp "${BIN_SRC}" "${STAGE_DIR}/kstall"
if command -v strip &>/dev/null; then
    strip -s "${STAGE_DIR}/kstall" || true
fi
chmod +x "${STAGE_DIR}/kstall"

# Documentation & Licenses
cp README.md "${STAGE_DIR}/"
if [[ -f LICENSE ]]; then cp LICENSE "${STAGE_DIR}/"; fi
if [[ -f LICENSE-APACHE ]]; then cp LICENSE-APACHE "${STAGE_DIR}/"; fi
if [[ -f LICENSE-GPL ]]; then cp LICENSE-GPL "${STAGE_DIR}/"; fi
if [[ -f install.sh ]]; then cp install.sh "${STAGE_DIR}/"; chmod +x "${STAGE_DIR}/install.sh"; fi

if [[ -d docs ]]; then
    cp -r docs/* "${STAGE_DIR}/docs/" || true
fi

# Deployment manifests
if [[ -d deploy ]]; then
    cp deploy/Dockerfile "${STAGE_DIR}/deploy/" 2>/dev/null || true
    cp deploy/daemonset.yaml "${STAGE_DIR}/deploy/" 2>/dev/null || true
    if [[ -d deploy/helm ]]; then
        cp -r deploy/helm "${STAGE_DIR}/deploy/" || true
    fi
fi

echo "==> 4. Creating release archives (.tar.gz and .tar.xz)..."
cd "${OUT_DIR}"
ARCHIVE_GZ="${RELEASE_NAME}.tar.gz"
ARCHIVE_XZ="${RELEASE_NAME}.tar.xz"

# Create .tar.gz
tar -czf "${ARCHIVE_GZ}" "${RELEASE_NAME}"
echo "    Created: ${ARCHIVE_GZ}"

# Create .tar.xz if xz is available
if command -v xz &>/dev/null; then
    tar -c "${RELEASE_NAME}" | xz -9 -c > "${ARCHIVE_XZ}"
    echo "    Created: ${ARCHIVE_XZ}"
fi

# Also create standalone binary tarball for minimal footprint
BIN_ARCHIVE="${RELEASE_NAME}-standalone.tar.gz"
tar -czf "${BIN_ARCHIVE}" -C "${STAGE_DIR}" kstall
echo "    Created: ${BIN_ARCHIVE}"

echo "==> 5. Computing cryptographic SHA-256 checksums..."
for archive in "${ARCHIVE_GZ}" "${ARCHIVE_XZ}" "${BIN_ARCHIVE}"; do
    if [[ -f "${archive}" ]]; then
        sha256sum "${archive}" > "${archive}.sha256"
        echo "    Checksum: $(cat "${archive}.sha256")"
    fi
done

# Generate release manifest JSON
cat <<EOF > "${OUT_DIR}/manifest-${TARGET}.json"
{
  "project": "kstall",
  "version": "${VERSION}",
  "target": "${TARGET}",
  "timestamp": "$(date -u +'%Y-%m-%dT%H:%M:%SZ')",
  "archives": [
    {
      "file": "${ARCHIVE_GZ}",
      "sha256": "$(cut -d ' ' -f 1 "${ARCHIVE_GZ}.sha256")",
      "size_bytes": $(stat -c%s "${ARCHIVE_GZ}")
    },
    {
      "file": "${BIN_ARCHIVE}",
      "sha256": "$(cut -d ' ' -f 1 "${BIN_ARCHIVE}.sha256")",
      "size_bytes": $(stat -c%s "${BIN_ARCHIVE}")
    }
  ]
}
EOF

# Clean up raw staging directory if archive succeeded
rm -rf "${STAGE_DIR}"

echo "======================================================="
echo "   kStall release v${VERSION} packaged successfully!  "
echo "   Artifacts available in: ${OUT_DIR}/"
echo "======================================================="
