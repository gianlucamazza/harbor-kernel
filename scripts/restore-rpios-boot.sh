#!/usr/bin/env bash
# Restore Raspberry Pi OS kernel + config on a mounted boot partition
# (diagnostic: verify serial wiring with a known-good OS).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKUP="${ROOT}/.sd-backup"
MOUNT="${1:-}"

if [[ -z "${MOUNT}" ]]; then
  echo "usage: $0 <boot-partition-mount>" >&2
  exit 2
fi
if [[ ! -d "${MOUNT}" ]]; then
  echo "error: mount not found: ${MOUNT}" >&2
  exit 1
fi
if [[ ! -f "${BACKUP}/kernel8.img.rpios" || ! -f "${BACKUP}/config.txt.rpios" ]]; then
  echo "error: missing backup in ${BACKUP}" >&2
  exit 1
fi

# Preserve our bare-metal bits under a timestamped name for easy re-deploy.
ts="$(date +%Y%m%d-%H%M%S)"
if [[ -f "${MOUNT}/kernel8.img" ]]; then
  cp -a "${MOUNT}/kernel8.img" "${BACKUP}/kernel8.img.agentic.${ts}" || true
fi
if [[ -f "${MOUNT}/config.txt" ]]; then
  cp -a "${MOUNT}/config.txt" "${BACKUP}/config.txt.agentic.${ts}" || true
fi

install -m 0644 "${BACKUP}/kernel8.img.rpios" "${MOUNT}/kernel8.img"
install -m 0644 "${BACKUP}/config.txt.rpios" "${MOUNT}/config.txt"

# Ensure UART console is enabled for wiring test (idempotent).
if ! grep -qE '^enable_uart=1' "${MOUNT}/config.txt"; then
  printf '\n# --- added for serial wiring diagnostic ---\nenable_uart=1\n' >> "${MOUNT}/config.txt"
fi
# Prefer PL011 on GPIO 14/15 when DT is present (RPi OS has DTB).
if ! grep -qE '^dtoverlay=disable-bt' "${MOUNT}/config.txt"; then
  printf 'dtoverlay=disable-bt\n' >> "${MOUNT}/config.txt"
fi

sync
echo "Restored Raspberry Pi OS boot files to ${MOUNT}"
ls -la "${MOUNT}/kernel8.img" "${MOUNT}/config.txt"
echo "--- config tail ---"
tail -20 "${MOUNT}/config.txt"
