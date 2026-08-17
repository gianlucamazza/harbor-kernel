#!/usr/bin/env bash
# Copy kernel image, config.txt, and platform blobs onto a mounted boot partition.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MOUNT="${1:-}"
IMG="${2:-}"
# Optional third argument: a boot description to place beside the image. The
# Pi firmware prefers a `.dtb` on the boot partition over its own, so this is
# the only thing on the card that can change what the kernel is told the
# hardware is — which makes it the one file that must never be left behind by
# accident (ADR-0105's absent-device evidence, scripts/host/absent-nic-dtb.sh).
DTB="${3:-}"

if [[ -z "${MOUNT}" || -z "${IMG}" ]]; then
	echo "usage: $0 <sd-boot-mount> <kernel8.img> [boot-description.dtb]" >&2
	exit 2
fi

# Shared with restore-rpios-boot.sh: one definition of "this is a Pi boot
# partition", so the two writers cannot end up with different amounts of care.
# shellcheck source=lib/sd-target.sh
source "${ROOT}/scripts/lib/sd-target.sh"

assert_boot_partition "${MOUNT}" || exit 1
# ADR-0066: informational — a missing store partition degrades honestly on
# the board, so this warns with the fix rather than blocking the deploy.
warn_durable_partition "${MOUNT}"

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

# Remove any boot description first, always. A deploy that only *adds* files
# would let one capture's evidence-only DTB survive into every later boot, and
# the symptom — a board that reports no NIC — looks exactly like a regression.
# Absence is the default and has to be restored, not assumed.
shopt -s nullglob
for stale in "${MOUNT}"/*.dtb; do
	echo "removing stale boot description: $(basename "${stale}")"
	rm -f "${stale}"
done
shopt -u nullglob

if [[ -n "${DTB}" ]]; then
	if [[ ! -f "${DTB}" ]]; then
		echo "error: boot description not found: ${DTB}" >&2
		exit 1
	fi
	install -m 0644 "${DTB}" "${MOUNT}/bcm2711-rpi-4-b.dtb"
fi

install -m 0644 "${IMG}" "${MOUNT}/kernel8.img"
install -m 0644 "${CONFIG}" "${MOUNT}/config.txt"
install -m 0644 "${BLOBS}/start4.elf" "${MOUNT}/start4.elf"
install -m 0644 "${BLOBS}/fixup4.dat" "${MOUNT}/fixup4.dat"

sync
echo "Deployed to ${MOUNT}:"
ls -la "${MOUNT}/kernel8.img" "${MOUNT}/config.txt" "${MOUNT}/start4.elf" "${MOUNT}/fixup4.dat"
if [[ -n "${DTB}" ]]; then
	echo "boot description: $(basename "${DTB}") -> bcm2711-rpi-4-b.dtb"
	echo "  This card now tells the kernel something other than the firmware would."
	echo "  Run a plain 'make deploy' to take it off again."
fi
echo "note: product compositions are injected into kernel8.img (.agent_store, ADR-0029)"
