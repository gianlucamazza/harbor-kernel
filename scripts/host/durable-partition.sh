#!/usr/bin/env bash
# Create the durable-store partition (ADR-0066) on a Pi SD card — once.
#
# A 1 MiB MBR partition of type 0x7f (the type designated for experimental
# use) appended after the last existing partition. The kernel discovers it
# by type from sector 0 — the card's own table names the store, so there is
# no magic LBA to keep in sync between kernel and host.
#
# Deliberately refuses to touch anything else: existing entries are never
# moved or resized, and a card that already carries a 0x7f entry is left
# exactly as it is.
set -euo pipefail

DEV="${1:?usage: $0 /dev/sdX (the whole card, not a partition)}"

if [[ ! -b "${DEV}" ]]; then
	echo "error: ${DEV} is not a block device" >&2
	exit 1
fi
case "${DEV}" in
*[0-9])
	echo "error: ${DEV} looks like a partition — pass the whole card (e.g. /dev/sdb)" >&2
	exit 1
	;;
esac

# Never the disk the running system lives on.
root_disk="$(lsblk -no PKNAME "$(findmnt -no SOURCE / || true)" 2>/dev/null | head -1 || true)"
if [[ -n "${root_disk}" && "/dev/${root_disk}" == "${DEV}" ]]; then
	echo "error: ${DEV} carries the running system — refusing" >&2
	exit 1
fi

# Refuse while any partition of the card is mounted: sfdisk would re-read
# the table under a live filesystem.
if lsblk -nro MOUNTPOINT "${DEV}" | grep -q .; then
	echo "error: ${DEV} has mounted partitions — unmount first (udisksctl unmount -b ...)" >&2
	exit 1
fi

# Idempotence: one store partition per card, ever.
if lsblk -nro PARTTYPE "${DEV}" | grep -q '^0x7f$'; then
	echo "durable partition already present on ${DEV} — nothing to do"
	exit 0
fi

# MBR only: the kernel's reader fails closed on GPT (ADR-0066).
label="$(sudo sfdisk --dump "${DEV}" | sed -n 's/^label: //p')"
if [[ "${label}" != "dos" ]]; then
	echo "error: ${DEV} has a '${label:-missing}' partition table; the ADR-0066 reader is MBR-only" >&2
	exit 1
fi

echo "appending a 1 MiB type-0x7f partition to ${DEV}:"
sudo sfdisk --dump "${DEV}" | sed 's/^/  /'

# sfdisk --append places the entry in the first free aligned gap after the
# existing partitions; ',2048,7f' = default start, 2048 sectors, type 0x7f.
echo ',2048,7f' | sudo sfdisk --append "${DEV}"
sync
sudo partprobe "${DEV}" 2>/dev/null || true

echo "created:"
lsblk -o NAME,SIZE,PARTTYPE "${DEV}"
