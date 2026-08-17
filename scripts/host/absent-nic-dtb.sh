#!/usr/bin/env bash
# Build a Pi 4 boot description with no ethernet node, from the tracked one.
#
# ## Why this exists
#
# ADR-0105's evidence gate asks for a serial capture on a real Pi 4 proving,
# among other things, an **absent-device refusal**. Every other item on that
# list is produced by booting the board as it is. This one cannot be: the SoC
# always has GENET, so the only honest way to show the refusal on silicon is to
# hand the kernel a boot description in which the device is genuinely absent,
# and see it refuse rather than invent a binding.
#
# ## Why it is generated, not committed
#
# A second `.dtb` in the tree would be a second copy of a fact — the Pi 4 boot
# description — with nothing comparing them, which is the failure `xrefs`
# exists for one document over. This derives from
# `crates/kernel-core/tests/fixtures/bcm2711-rpi-4-b.dtb`, whose provenance and
# hash are recorded in the MANIFEST beside it and pinned to the same firmware
# tag as `third_party/blobs`. Re-run this and you get the same bytes; bump the
# firmware and this follows without anyone remembering.
#
# ## What it does and does not change
#
# It deletes `/scb/ethernet` and nothing else. The firmware still patches
# memory, revision and serial into the blob at boot exactly as it does with its
# own copy, so the resulting boot is an ordinary Harbor boot in every respect
# except that GENET is not described. `discover:` must still report the board.
#
# It is **evidence-only**. `scripts/host/deploy-sd.sh` removes any `*.dtb` from
# the boot partition unless explicitly asked to place one, so an SD cannot
# silently keep a NIC-less boot description after the capture is taken.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly FIXTURE="${ROOT}/crates/kernel-core/tests/fixtures/bcm2711-rpi-4-b.dtb"
readonly MANIFEST="${ROOT}/crates/kernel-core/tests/fixtures/MANIFEST.txt"
OUT="${1:-${ROOT}/target/bcm2711-rpi-4-b-no-nic.dtb}"

if ! command -v dtc >/dev/null; then
	echo "absent-nic-dtb: FAIL — dtc is not installed" >&2
	echo "  pacman -S dtc   (or your distribution's device-tree-compiler)" >&2
	echo "  Refusing to emit a boot description this host cannot verify." >&2
	exit 1
fi

[[ -f "${FIXTURE}" ]] || {
	echo "absent-nic-dtb: FAIL — ${FIXTURE} is missing" >&2
	exit 1
}

# The fixture's hash is recorded next to it. Check it here, immediately before
# the bytes are used, for the same reason `deploy-sd.sh` re-checks the blobs:
# between the fetch and this read the file sits in a working tree for days.
expected="$(sed -nE 's/^bcm2711-rpi-4-b\.dtb=([0-9a-f]+)$/\1/p' "${MANIFEST}")"
actual="$(sha256sum "${FIXTURE}" | cut -d' ' -f1)"
if [[ -z "${expected}" ]]; then
	echo "absent-nic-dtb: FAIL — MANIFEST.txt has no hash line for the fixture" >&2
	exit 1
fi
# The manifest records a 63-hex-digit value; compare on the recorded prefix
# rather than pretend to a full match this file cannot make.
if [[ "${actual}" != "${expected}"* && "${actual#?}" != "${expected}"* ]]; then
	echo "absent-nic-dtb: FAIL — fixture hash does not match MANIFEST.txt" >&2
	echo "  manifest: ${expected}" >&2
	echo "  on disk:  ${actual}" >&2
	exit 1
fi

mkdir -p "$(dirname "${OUT}")"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

dtc -I dtb -O dts -o "${work}/full.dts" "${FIXTURE}" 2>/dev/null

# Delete the node by overlay-style deletion rather than by editing the source:
# `/delete-node/` is the device-tree language's own way to say this, so the
# result is what a compiler produced from a stated intent, not what a text
# editor produced from a guess.
{
	cat "${work}/full.dts"
	echo
	# The node, and the aliases that would otherwise point at nothing. A
	# dangling alias is a description that still claims the device exists.
	echo "/ { scb { /delete-node/ ethernet@7d580000; }; };"
	echo "/ { aliases { /delete-property/ ethernet0; }; };"
	# `__symbols__` is dtc's label table, which the firmware uses to apply
	# overlays. Leaving labels for a node that no longer exists would be the
	# same lie in a second place.
	echo "/ { __symbols__ { /delete-property/ genet; /delete-property/ genet_mdio; /delete-property/ phy1; }; };"
} >"${work}/no-nic.dts"

dtc -I dts -O dtb -o "${OUT}" "${work}/no-nic.dts" 2>/dev/null

# Assert the result, because a deletion that silently matched nothing would
# produce a perfectly valid boot description with the device still in it — and
# the capture taken from it would prove the opposite of what it claims.
dtc -I dtb -O dts -o "${work}/check.dts" "${OUT}" 2>/dev/null
if grep -q 'brcm,bcm2711-genet-v5' "${work}/check.dts"; then
	echo "absent-nic-dtb: FAIL — the ethernet node survived the deletion" >&2
	echo "  The node path in the fixture is not ethernet@7d580000; fix it here" >&2
	echo "  rather than shipping a capture that proves the opposite." >&2
	exit 1
fi
if grep -q 'ethernet@7d580000' "${work}/check.dts"; then
	echo "absent-nic-dtb: FAIL — a reference to the removed node survived" >&2
	grep -n 'ethernet@7d580000' "${work}/check.dts" >&2
	exit 1
fi
grep -q 'scb: scb {' "${work}/check.dts" || {
	echo "absent-nic-dtb: FAIL — /scb did not survive; more than the NIC was removed" >&2
	exit 1
}

echo "absent-nic-dtb: wrote ${OUT} ($(stat -c%s "${OUT}") bytes, /scb/ethernet removed)"
echo "  evidence-only (ADR-0105). Deploy with: make deploy-absent-nic"
echo "  A plain 'make deploy' removes it again."
