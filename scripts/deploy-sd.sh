#!/usr/bin/env bash
# Copy kernel image, config.txt, and platform blobs onto a mounted boot partition.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MOUNT="${1:-}"
IMG="${2:-}"

if [[ -z "${MOUNT}" || -z "${IMG}" ]]; then
  echo "usage: $0 <sd-boot-mount> <kernel8.img>" >&2
  exit 2
fi

if [[ ! -d "${MOUNT}" ]]; then
  echo "error: mount point not found: ${MOUNT}" >&2
  exit 1
fi
if [[ ! -f "${IMG}" ]]; then
  echo "error: kernel image not found: ${IMG}" >&2
  exit 1
fi

BLOBS="${ROOT}/third_party/blobs"
CONFIG="${ROOT}/boot/config.txt"

if [[ ! -f "${BLOBS}/start4.elf" || ! -f "${BLOBS}/fixup4.dat" ]]; then
  echo "error: platform blobs missing; run: make blobs" >&2
  exit 1
fi
if [[ ! -f "${CONFIG}" ]]; then
  echo "error: missing ${CONFIG}" >&2
  exit 1
fi

install -m 0644 "${IMG}" "${MOUNT}/kernel8.img"
install -m 0644 "${CONFIG}" "${MOUNT}/config.txt"
install -m 0644 "${BLOBS}/start4.elf" "${MOUNT}/start4.elf"
install -m 0644 "${BLOBS}/fixup4.dat" "${MOUNT}/fixup4.dat"

sync
echo "Deployed to ${MOUNT}:"
ls -la "${MOUNT}/kernel8.img" "${MOUNT}/config.txt" "${MOUNT}/start4.elf" "${MOUNT}/fixup4.dat"
