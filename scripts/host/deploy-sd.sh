#!/usr/bin/env bash
# Copy kernel image, config.txt, and platform blobs onto a mounted boot partition.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MOUNT="${1:-}"
IMG="${2:-}"
# Optional third argument: a boot description to place instead of the tracked
# one. The Pi 4 firmware reads the `.dtb` from the boot partition, so this is
# the only file on the card that changes what the kernel is told the hardware
# is — which makes it both the lever ADR-0105's absent-device evidence needs
# (scripts/host/absent-nic-dtb.sh) and the file a deploy must never leave to
# chance. Omit it and the tracked fixture is written.
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

# A deploy always leaves a boot description, exactly as it always leaves
# `start4.elf` and `config.txt`. It is not optional and it is not inherited.
#
# ## The bug this is the fix for
#
# The first version of this block removed every `*.dtb` and wrote none. It was
# written so that one capture's evidence-only description could not survive
# into later boots — a real hazard, since a board silently reporting no NIC
# looks exactly like a regression. What it missed is that the card already
# carried Raspberry Pi OS's `bcm2711-rpi-4-b.dtb`, and **that** was the
# description feeding every Harbor boot: `DTB mapped: 61440 bytes` in the
# transcripts is 60 KiB of real board description, not something the firmware
# synthesises. Removing it and writing nothing left the board with no device
# tree, and the Pi 4 then never carries the kernel as far as the UART: zero
# bytes on the wire, no durable write, an ACT LED that blinks once and stops.
# It cost most of an afternoon and looked, in turn, like a dead adapter, a
# corrupt card, a brown-out and a failed board.
#
# Inheriting a file the deploy does not write is the shape of the mistake.
# Absence *and* presence both have to be produced, not assumed — so this writes
# one every time, from the tracked fixture unless the caller names another.
readonly BOARD_DTB="bcm2711-rpi-4-b.dtb"
readonly DEFAULT_DTB="${ROOT}/crates/kernel-core/tests/fixtures/${BOARD_DTB}"

dtb_source="${DTB:-${DEFAULT_DTB}}"
if [[ ! -f "${dtb_source}" ]]; then
	echo "error: boot description not found: ${dtb_source}" >&2
	echo "  A deploy without one leaves a card the firmware will not boot." >&2
	exit 1
fi
install -m 0644 "${dtb_source}" "${MOUNT}/${BOARD_DTB}"

install -m 0644 "${IMG}" "${MOUNT}/kernel8.img"
install -m 0644 "${CONFIG}" "${MOUNT}/config.txt"
install -m 0644 "${BLOBS}/start4.elf" "${MOUNT}/start4.elf"
install -m 0644 "${BLOBS}/fixup4.dat" "${MOUNT}/fixup4.dat"

sync

# Assert the card is bootable before saying so. Every file below is required by
# the Pi 4 boot chain, and the failure that motivates this check was a deploy
# that reported success while leaving the card without a device tree — the
# board then produced nothing at all, which reads as a hardware fault rather
# than as a missing file. A deploy that cannot say "bootable" should not say
# "deployed".
missing=0
for required in kernel8.img config.txt start4.elf fixup4.dat "${BOARD_DTB}"; do
	if [[ ! -s "${MOUNT}/${required}" ]]; then
		echo "deploy-sd: FAIL — ${required} is missing or empty on ${MOUNT}" >&2
		missing=$((missing + 1))
	fi
done
if [[ "${missing}" -ne 0 ]]; then
	echo "deploy-sd: the card is not bootable; ${missing} required file(s) absent" >&2
	exit 1
fi

echo "Deployed to ${MOUNT}:"
ls -la "${MOUNT}/kernel8.img" "${MOUNT}/config.txt" "${MOUNT}/start4.elf" "${MOUNT}/fixup4.dat"
if [[ -n "${DTB}" ]]; then
	echo "boot description: $(basename "${DTB}") -> ${BOARD_DTB}  (evidence-only)"
	echo "  This card now tells the kernel something other than the tracked one."
	echo "  Run a plain 'make deploy' to put the tracked description back."
else
	echo "boot description: tracked fixture -> ${BOARD_DTB}"
fi
echo "note: product compositions are injected into kernel8.img (.agent_store, ADR-0029)"
