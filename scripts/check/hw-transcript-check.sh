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

# A capture holds as many boots as the board was power-cycled during it, and
# that is not an accident to be tidied up: ADR-0066's evidence *is* a power
# cycle, so the honest capture of it has two `Harbor: hello` lines. The
# assertions are about one boot, though — `task-a 0 … task-b 3` is a sequence,
# and two boots concatenated satisfy neither the sequence nor the counters.
#
# So the last complete boot is what gets asserted, and the count is printed
# rather than assumed: a reader who is told "clean" deserves to know which of
# the three boots in the file that was about. The last is the right one for
# `DURABLE_MEDIA_EXPECT=previous` too — it is the boot that had a previous.
#
# Seen red: `20260810-160227.log`, cited in `docs/verification.md` as the
# ADR-0077 F-R1-P1 stamp, failed `task output not interleaved` with each line
# appearing exactly twice.
boots="$(grep -ac 'Harbor: hello' "${log}" || true)"
if [[ "${boots}" -gt 1 ]]; then
	last="$(grep -an 'Harbor: hello' "${log}" | tail -1 | cut -d: -f1)"
	tail -n "+${last}" "${log}" >"${log}.last"
	mv "${log}.last" "${log}"
fi

# Which tree the captured image was built from (`build: … src=<id>`), against
# the tree whose oracle is about to judge it.
#
# The assertions come from `boot-oracle.sh` as it is *now*, and it grows: a
# capture from before ADR-0090 has no force-kill line, so today's oracle fails
# it — correctly, and for a reason that has nothing to do with the board. The
# gate says so up front rather than leaving a stale capture to read as a
# hardware regression. It is a warning, not a refusal: re-checking an archived
# transcript against a newer oracle is a legitimate thing to do deliberately.
transcript_src="$(sed -nE 's/.*[[:space:]]src=([0-9a-zA-Z._-]+).*/\1/p' "${log}" | head -1)"
tree_src="$(git describe --always --dirty 2>/dev/null || echo unknown)"
if [[ -n "${transcript_src}" && "${transcript_src}" != "${tree_src}" ]]; then
	echo "hw-transcript-check: NOTE — capture is from src=${transcript_src}, tree is ${tree_src}" >&2
	echo "  The assertions below are this tree's. A capture older than an oracle" >&2
	echo "  line fails on the line, not on the board — capture again to claim HW." >&2
fi

fail() {
	echo "hw-transcript-check: FAIL — $1" >&2
	echo "--- transcript: ${TRANSCRIPT} (src=${transcript_src:-unknown}, tree ${tree_src}) ---" >&2
	exit 1
}

on_timer_missed() {
	# Silicon has no starved-emulator excuse: a missed deadline is a failure.
	fail "timer deadlines expired unserviced on hardware"
}

# shellcheck source=scripts/lib/boot-oracle.sh
source "$(dirname "$0")/../lib/boot-oracle.sh"

# ADR-0066: the canonical HW transcript is recorded on a second-or-later
# powered boot, so one log carries cross-power-cycle evidence
# (from=Previous, boot>=2). Override for bring-up captures only.
DURABLE_MEDIA_EXPECT="${DURABLE_MEDIA_EXPECT:-previous}" assert_boot_oracle

printf 'hw-transcript-check: clean (%s, boot %s of %s in the capture)\n' \
	"${TRANSCRIPT}" "${boots}" "${boots}"
