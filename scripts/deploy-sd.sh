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

# Shared with restore-rpios-boot.sh: one definition of "this is a Pi boot
# partition", so the two writers cannot end up with different amounts of care.
# shellcheck source=lib/sd-target.sh
source "${ROOT}/scripts/lib/sd-target.sh"

assert_boot_partition "${MOUNT}" || exit 1

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

# Verify the blobs again, here, immediately before they are written. Between
# the fetch and this write the files sit in a working tree for days: an
# interrupted re-fetch, a botched firmware bump, an editor saving over one of
# them. ADR-0004 pins this firmware because the GIC configuration depends on it,
# so "some start4.elf" is not the same claim as "the one that was validated".
assert_blobs_pinned "${BLOBS}" || exit 1

install -m 0644 "${IMG}" "${MOUNT}/kernel8.img"
install -m 0644 "${CONFIG}" "${MOUNT}/config.txt"
install -m 0644 "${BLOBS}/start4.elf" "${MOUNT}/start4.elf"
install -m 0644 "${BLOBS}/fixup4.dat" "${MOUNT}/fixup4.dat"

sync
echo "Deployed to ${MOUNT}:"
ls -la "${MOUNT}/kernel8.img" "${MOUNT}/config.txt" "${MOUNT}/start4.elf" "${MOUNT}/fixup4.dat"
