#!/usr/bin/env bash
# Shared guard for anything that writes to a Raspberry Pi boot partition.
#
# Two scripts write there — `deploy-sd.sh` and `restore-rpios-boot.sh` — and
# they had different amounts of caution: nine checks against two. The weaker one
# would happily install a bootloader into whatever directory it was handed,
# which is the same class of mistake as an unmounted card, and it is the script
# you reach for when something has already gone wrong.
#
# The checks live here rather than being copied into both. A duplicated rule
# drifts in every copy at once; this project watched a duplicated probe table do
# exactly that the same day this was written.

# Refuse unless MOUNT is a mounted FAT volume that looks like a Pi boot
# partition. Set FORCE_EMPTY=1 to accept a blank one.
assert_boot_partition() {
	local mount="$1"

	if [[ -z "${mount}" ]]; then
		echo "error: no mount point given" >&2
		return 1
	fi
	if [[ ! -d "${mount}" ]]; then
		echo "error: mount point not found: ${mount}" >&2
		return 1
	fi

	# Without this, an unmounted card — or the difference between
	# /run/media/$USER/boot and .../bootfs — means quietly writing a bootloader
	# into the local filesystem and wondering later why the Pi ignored it.
	if ! mountpoint -q "${mount}"; then
		echo "error: ${mount} is not a mount point — is the card inserted?" >&2
		echo "hint: SD_MOUNT=/run/media/\$USER/bootfs make deploy" >&2
		return 1
	fi

	local fstype
	fstype="$(findmnt -no FSTYPE "${mount}" || true)"
	case "${fstype}" in
	vfat | msdos | exfat) ;;
	*)
		echo "error: ${mount} is ${fstype:-unknown}, not a FAT boot partition" >&2
		return 1
		;;
	esac

	# A Pi boot partition carries the firmware the EEPROM loads. If none of
	# these is present, this is some other FAT volume — a camera card, say.
	local probe
	for probe in bootcode.bin start4.elf start.elf config.txt kernel8.img; do
		if [[ -e "${mount}/${probe}" ]]; then
			return 0
		fi
	done

	if [[ -n "${FORCE_EMPTY:-}" ]]; then
		return 0
	fi

	echo "error: ${mount} has no Raspberry Pi boot files — refusing to write" >&2
	echo "hint: FORCE_EMPTY=1 to initialise a blank boot partition" >&2
	return 1
}

# Report whether the card carrying MOUNT has the durable-store partition
# (MBR type 0x7f, ADR-0066). Informational: a card without one boots with an
# honest `durable-media: no-partition` line, so absence warns rather than
# refuses — the kernel's own oracle is the enforcement. Uses lsblk (sysfs),
# so no elevated access is needed for the check.
warn_durable_partition() {
	local mount="$1"
	local src disk name ptype
	src="$(findmnt -no SOURCE "${mount}" || true)"
	disk="$(lsblk -no PKNAME "${src}" 2>/dev/null | head -1 || true)"
	if [[ -z "${disk}" ]]; then
		echo "note: cannot resolve the card device behind ${mount}; durable partition unchecked" >&2
		return 0
	fi
	while read -r name ptype; do
		if [[ "${ptype}" == "0x7f" ]]; then
			echo "durable partition: present (/dev/${name})"
			return 0
		fi
	done < <(lsblk -nro NAME,PARTTYPE "/dev/${disk}")
	echo "note: no durable-store partition (type 0x7f) on /dev/${disk}" >&2
	echo "      the kernel will print 'durable-media: no-partition' and skip media persistence" >&2
	echo "hint: scripts/host/durable-partition.sh /dev/${disk}" >&2
	return 0
}

# Refuse unless the pinned platform blobs match the hashes committed to the
# repo. `fetch-blobs.sh` checks them on arrival, which protects the download and
# nothing after it; this covers what actually reaches the card.
assert_blobs_pinned() {
	local blobs="$1"
	local expected="${blobs}/EXPECTED.sha256"

	if [[ ! -f "${expected}" ]]; then
		echo "error: missing ${expected} — cannot verify what is about to be written" >&2
		return 1
	fi

	local blob want got
	for blob in start4.elf fixup4.dat; do
		want="$(sed -n "s/^\([0-9a-f]\{64\}\)  ${blob}\$/\1/p" "${expected}")"
		if [[ -z "${want}" ]]; then
			echo "error: no expected hash for ${blob} in ${expected}" >&2
			return 1
		fi
		got="$(sha256sum "${blobs}/${blob}" | cut -d' ' -f1)"
		if [[ "${got}" != "${want}" ]]; then
			echo "error: ${blob} does not match the pinned firmware — refusing to write" >&2
			echo "  expected ${want}" >&2
			echo "  found    ${got}" >&2
			echo "hint: make blobs, or see docs/blobs.md if you are bumping firmware_tag" >&2
			return 1
		fi
	done
}
