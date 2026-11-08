#!/usr/bin/env bash
# SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
# kStall Universal Installer Script
set -euo pipefail

REPO="kstall/kstall"
BIN_NAME="kstall"
INSTALL_DIR="${BIN_DIR:-/usr/local/bin}"

echo "======================================================="
echo "             kStall Official Installer                 "
echo "======================================================="

# Ensure OS is Linux
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
if [[ "${OS}" != "linux" ]]; then
    echo "Error: kStall requires Linux (with eBPF CO-RE support). Detected OS: ${OS}" >&2
    exit 1
fi

# Detect architecture
ARCH=$(uname -m)
case "${ARCH}" in
    x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
    aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
    arm64)   TARGET="aarch64-unknown-linux-gnu" ;;
    *)
        echo "Error: Unsupported architecture: ${ARCH}. Supported architectures: x86_64, aarch64." >&2
        exit 1
        ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd || echo "")"
LOCAL_BIN=""

# Check if running from an extracted release package or git repo
if [[ -n "${SCRIPT_DIR}" && -f "${SCRIPT_DIR}/kstall" ]]; then
    LOCAL_BIN="${SCRIPT_DIR}/kstall"
elif [[ -n "${SCRIPT_DIR}" && -f "${SCRIPT_DIR}/target/release/kstall" ]]; then
    LOCAL_BIN="${SCRIPT_DIR}/target/release/kstall"
fi

if [[ -n "${LOCAL_BIN}" ]]; then
    echo "==> Installing from local binary: ${LOCAL_BIN}"
    TMP_BIN="${LOCAL_BIN}"
else
    echo "==> Resolving latest release for ${TARGET}..."
    VERSION="${KSTALL_VERSION:-}"
    if [[ -z "${VERSION}" ]]; then
        VERSION=$(curl -sSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)
        if [[ -z "${VERSION}" ]]; then
            VERSION="v0.1.0"
        fi
    fi
    VERSION_CLEAN="${VERSION#v}"

    ARCHIVE_URL="https://github.com/${REPO}/releases/download/${VERSION}/kstall-v${VERSION_CLEAN}-${TARGET}-standalone.tar.gz"
    echo "==> Downloading ${ARCHIVE_URL}..."
    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "${TMP_DIR}"' EXIT

    curl -fsSL "${ARCHIVE_URL}" -o "${TMP_DIR}/kstall.tar.gz" || {
        echo "Failed to download pre-built release binary." >&2
        echo "Please build from source: 'git clone https://github.com/${REPO} && cd kstall && make all'" >&2
        exit 1
    }

    tar -xzf "${TMP_DIR}/kstall.tar.gz" -C "${TMP_DIR}"
    TMP_BIN="${TMP_DIR}/kstall"
fi

# Check permissions for install directory
mkdir -p "${INSTALL_DIR}" 2>/dev/null || true
USE_SUDO=false
if [[ ! -w "${INSTALL_DIR}" ]]; then
    if [[ $EUID -ne 0 ]]; then
        if command -v sudo &>/dev/null; then
            echo "==> Destination '${INSTALL_DIR}' is not user-writable; invoking sudo..."
            USE_SUDO=true
        else
            INSTALL_DIR="${HOME}/.local/bin"
            mkdir -p "${INSTALL_DIR}"
            echo "==> Falling back to user directory: ${INSTALL_DIR}"
        fi
    fi
fi

TARGET_PATH="${INSTALL_DIR}/${BIN_NAME}"
echo "==> Installing to ${TARGET_PATH}..."
if [[ "${USE_SUDO}" == true ]]; then
    sudo install -m 755 "${TMP_BIN}" "${TARGET_PATH}"
else
    install -m 755 "${TMP_BIN}" "${TARGET_PATH}"
fi

echo "==> Installation complete!"
"${TARGET_PATH}" --version

echo ""
echo "==> Verifying kernel prerequisites (doctor)..."
"${TARGET_PATH}" doctor || true

echo ""
echo "======================================================="
echo "  kStall is ready! Try running:"
echo "    sudo ${TARGET_PATH} watch --threshold 2ms"
echo "    sudo ${TARGET_PATH} top"
echo "======================================================="
