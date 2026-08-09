#!/usr/bin/env bash
# Assert a Pi 4B serial transcript satisfies the boot oracle.
#
# The assertions are the QEMU gate's, sourced from one place
# (`scripts/lib/boot-oracle.sh`) — not a hardware-flavoured copy. What this
# adds is the half QEMU cannot give: TLB fills are speculative on silicon and
# not in TCG, so a class of staleness (the ADR-0050 amendment's early-map
# residue) is only ever red here. A `done (HW)` claim cites the transcript
# this was run against, per docs/verification.md.
#
# Usage: hw-transcript-check.sh <serial-transcript.log>
# The transcript is a `scripts/host/serial-capture.sh` log: host timestamps
# are stripped before the assertions run.
set -euo pipefail

TRANSCRIPT="${1:?usage: $0 <serial-transcript.log>}"

if [[ ! -f "${TRANSCRIPT}" ]]; then
	echo "error: transcript not found: ${TRANSCRIPT}" >&2
	exit 1
fi

log="$(mktemp)"
trap 'rm -f "${log}"' EXIT

# serial-capture prepends `HH:MM:SS.<frac> ` per line; the oracle's anchored
# patterns (`^display: `, `^build: `) need the bare board output.
sed -E 's/^[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]+ //' "${TRANSCRIPT}" >"${log}"

fail() {
	echo "hw-transcript-check: FAIL — $1" >&2
	echo "--- transcript: ${TRANSCRIPT} ---" >&2
	exit 1
}

on_timer_missed() {
	# Silicon has no starved-emulator excuse: a missed deadline is a failure.
	fail "timer deadlines expired unserviced on hardware"
}

# shellcheck source=scripts/lib/boot-oracle.sh
source "$(dirname "$0")/../lib/boot-oracle.sh"

assert_boot_oracle

echo "hw-transcript-check: clean (${TRANSCRIPT})"
