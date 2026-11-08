#!/usr/bin/env bash
# SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
# kStall - Container Build and Smoke Test Script
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

IMAGE_NAME="${IMAGE_NAME:-kstall}"
REGISTRY="${REGISTRY:-ghcr.io/kstall}"
VERSION=$(grep -m 1 '^version = ' Cargo.toml | cut -d '"' -f 2 || echo "0.1.0")
TAG="${VERSION}"
PUSH=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tag)
            TAG="$2"
            shift 2
            ;;
        --push)
            PUSH=true
            shift
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

FULL_IMAGE="${REGISTRY}/${IMAGE_NAME}:${TAG}"
LATEST_IMAGE="${REGISTRY}/${IMAGE_NAME}:latest"

echo "======================================================="
echo "               kStall Container Builder                "
echo "======================================================="
echo "  Target Image: ${FULL_IMAGE}"
echo "  Latest Image: ${LATEST_IMAGE}"
echo "======================================================="

if ! command -v docker &>/dev/null; then
    echo "Error: docker command not found." >&2
    exit 1
fi

echo "==> 1. Building Docker image..."
docker build -f deploy/Dockerfile -t "${FULL_IMAGE}" -t "${LATEST_IMAGE}" .

echo "==> 2. Running Container Smoke Tests..."
echo "  -> Checking kstall version inside container:"
docker run --rm "${FULL_IMAGE}" --version

echo "  -> Checking kstall doctor inside container:"
docker run --rm "${FULL_IMAGE}" doctor || true

if [[ "${PUSH}" == true ]]; then
    echo "==> 3. Pushing images to registry..."
    docker push "${FULL_IMAGE}"
    docker push "${LATEST_IMAGE}"
    echo "    Pushed successfully."
fi

echo "======================================================="
echo "   Container build & smoke test completed successfully!"
echo "======================================================="
