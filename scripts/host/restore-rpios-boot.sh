#!/usr/bin/env bash
# Restore Raspberry Pi OS kernel + config on a mounted boot partition
# (diagnostic: verify serial wiring with a known-good OS).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BACKUP="${ROOT}/.sd-backup"
MOUNT="${1:-}"

if [[ -z "${MOUNT}" ]]; then
  echo "usage: $0 <boot-partition-mount>" >&2
  exit 2
fi
# The same guard `deploy-sd.sh` uses. This script had two checks against its
# nine, and it is the one reached for when something has already gone wrong —
# the worst moment to install a bootloader into the local filesystem.
# shellcheck source=lib/sd-target.sh
source "${ROOT}/scripts/lib/sd-target.sh"

assert_boot_partition "${MOUNT}" || exit 1
if [[ ! -f "${BACKUP}/kernel8.img.rpios" || ! -f "${BACKUP}/config.txt.rpios" ]]; then
  echo "error: missing backup in ${BACKUP}" >&2
  exit 1
fi

# Preserve our bare-metal bits under a timestamped name for easy re-deploy.
#
# The backup is a precondition, not a courtesy. These two `cp` calls used to end
# in `|| true` and the overwrite below ran regardless: a full backup directory,
# a permission error or a read failure destroyed the Harbor image with no copy
# anywhere. This is the script reached for when something has already gone
# wrong, which is the worst possible place to be best-effort about the only
# remaining copy.
ts="$(date +%Y%m%d-%H%M%S)"
keep() {
  local src="$1" dest="$2"
  [[ -f "${src}" ]] || return 0
  if ! cp -a "${src}" "${dest}"; then
    echo "error: could not back up ${src} to ${dest}" >&2
    echo "  refusing to overwrite it — the Pi OS files are still in ${BACKUP}" >&2
    exit 1
  fi
}
keep "${MOUNT}/kernel8.img" "${BACKUP}/kernel8.img.harbor.${ts}"
keep "${MOUNT}/config.txt" "${BACKUP}/config.txt.harbor.${ts}"

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
