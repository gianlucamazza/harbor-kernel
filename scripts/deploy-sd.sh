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

# Refuse to write anywhere that is not actually a mounted boot partition.
#
# Without this the script installs into any existing directory: an unmounted
# card, or the difference between /run/media/$USER/boot and .../bootfs, means
# quietly writing a bootloader into the local filesystem and wondering later
# why the Pi did not pick up the new kernel.
if ! mountpoint -q "${MOUNT}"; then
	echo "error: ${MOUNT} is not a mount point — is the card inserted?" >&2
	echo "hint: SD_MOUNT=/run/media/\$USER/bootfs make deploy" >&2
	exit 1
fi

fstype="$(findmnt -no FSTYPE "${MOUNT}" || true)"
case "${fstype}" in
vfat | msdos | exfat) ;;
*)
	echo "error: ${MOUNT} is ${fstype:-unknown}, not a FAT boot partition" >&2
	exit 1
	;;
esac

# A Pi boot partition carries the firmware the EEPROM loads. If none of these
# is present, this is some other FAT volume — a camera card, say.
looks_like_boot=""
for probe in bootcode.bin start4.elf start.elf config.txt kernel8.img; do
	if [[ -e "${MOUNT}/${probe}" ]]; then
		looks_like_boot=1
		break
	fi
done

if [[ -z "${looks_like_boot}" && -z "${FORCE_EMPTY:-}" ]]; then
	echo "error: ${MOUNT} has no Raspberry Pi boot files — refusing to write" >&2
	echo "hint: FORCE_EMPTY=1 to initialise a blank boot partition" >&2
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
