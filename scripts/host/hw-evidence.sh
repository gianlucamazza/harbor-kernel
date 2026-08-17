#!/usr/bin/env bash
# Derive a tracked evidence file from an untracked serial capture (ADR-0109).
#
# ## Why this exists
#
# `.serial-log/` is ignored, and every `done (HW)` row in the roadmap cites a
# file in it. That made the project's strongest claims depend on one laptop —
# `make hw-check TRANSCRIPT=…` could not run on a clone, and the excerpt in
# `verification.md` was a hand-made copy of a fact with nothing comparing it to
# the original.
#
# Tracking the captures whole is not the fix: 36 cited artifacts are 20 MB, of
# which 99.95% is the idle heartbeat. On `20260815-092435.log`, 71 964 of about
# 72 000 lines are `ticks=` and `invariants:`. The capture is a tape of a
# metronome with the record buried in it.
#
# ## What it keeps
#
# Every non-heartbeat line verbatim. Each contiguous run of heartbeat lines
# collapses to **first + count + last** — not deleted, because the roadmap
# cites `slots=4/9` off that line as the measured slot peak (ADR-0098) and
# `frames_free` is how a leak would show. First and last keep the delta across
# the boot, which is the claim; the count keeps the reader honest about how
# long the board idled.
#
# Measured over the 34 cited captures: 20 MB in, 349 KiB out.
#
# ## What it declares
#
# The result is lossy on purpose, and the header says which capture it came
# from and that capture's sha256. Whoever holds the capture can re-derive and
# compare byte for byte — that is what `scripts/check/hw-evidence.sh` does when
# it can. Whoever does not at least reads a stated provenance instead of an
# anonymous excerpt.
#
# Usage:
#   hw-evidence.sh <capture.log> [outdir]   # writes docs/evidence/<basename>
#   hw-evidence.sh --stdout <capture.log>   # emits it instead (used by the gate)
set -euo pipefail

cd "$(dirname "$0")/../.."

readonly VERSION=1
readonly EVIDENCE_DIR="docs/evidence"

to_stdout=0
if [[ "${1:-}" == "--stdout" ]]; then
	to_stdout=1
	shift
fi

CAPTURE="${1:-}"
OUTDIR="${2:-${EVIDENCE_DIR}}"

if [[ -z "${CAPTURE}" ]]; then
	echo "usage: $0 [--stdout] <capture.log> [outdir]" >&2
	exit 2
fi
if [[ ! -f "${CAPTURE}" ]]; then
	echo "hw-evidence: FAIL — no such capture: ${CAPTURE}" >&2
	exit 1
fi

base="$(basename "${CAPTURE}")"

# A capture with a live writer is not a record yet, and deriving from one
# produces a file whose stated sha256 is stale the moment the board's next
# heartbeat lands. Seen the first time this gate ran: `20260817-132244.log` was
# still being appended by a `serial-capture.sh` from 13:22, and the check
# reported "two different files share one name" — true, and the wrong diagnosis
# to leave a reader with.
#
# `--stdout` is exempt: the gate calls it to re-derive for comparison, and
# refusing there would turn a live capture into a gate failure with no action
# attached. The comparison itself will disagree, which is the honest outcome.
if [[ "${to_stdout}" -eq 0 ]] && command -v fuser >/dev/null 2>&1; then
	if fuser "${CAPTURE}" >/dev/null 2>&1; then
		echo "hw-evidence: FAIL — something is still writing to ${CAPTURE}" >&2
		fuser -v "${CAPTURE}" 2>&1 | sed 's/^/  /' >&2
		echo "  A capture that is still growing cannot be a record: the sha256 this" >&2
		echo "  would record is stale at the board's next heartbeat. Stop the capture" >&2
		echo "  (Ctrl-C the 'make serial-capture') and run this again." >&2
		exit 1
	fi
fi

sha="$(sha256sum "${CAPTURE}" | cut -d' ' -f1)"

# The header is deterministic — no extraction date, because git already records
# when the file landed and a timestamp inside the file would make every
# re-derivation differ from the tracked copy for no reason. That determinism is
# what lets the gate compare whole files instead of diffing around a preamble.
emit() {
	cat <<-HDR
		# Harbor hardware evidence — derived by scripts/host/hw-evidence.sh v${VERSION}.
		# Do not edit by hand: the gate re-derives this and compares.
		#
		# capture: ${base}
		# sha256:  ${sha}
		#
		# Heartbeat runs (\`ticks=\` / \`invariants:\`) are collapsed to first + count +
		# last (ADR-0109). Everything else is verbatim. Re-derive with:
		#   scripts/host/hw-evidence.sh .serial-log/${base}
	HDR

	# The runs are buffered rather than counted-and-reprinted so that a run of
	# one or two lines survives untouched: eliding "0 lines" between a first and
	# a last would be a marker that says nothing and costs more than the line it
	# replaced.
	awk '
		function flush(   i) {
			if (n == 0) return
			if (n <= 2) {
				for (i = 1; i <= n; i++) print buf[i]
			} else {
				print buf[1]
				printf "        [... %d heartbeat lines elided ...]\n", n - 2
				print buf[n]
			}
			n = 0
		}
		/ticks=|invariants: / { buf[++n] = $0; next }
		{ flush(); print }
		END { flush() }
	' "${CAPTURE}"
}

if [[ "${to_stdout}" -eq 1 ]]; then
	emit
	exit 0
fi

mkdir -p "${OUTDIR}"
out="${OUTDIR}/${base}"
emit >"${out}"

in_b="$(stat -c%s "${CAPTURE}")"
out_b="$(stat -c%s "${out}")"
printf 'hw-evidence: %s → %s (%d KiB → %d KiB)\n' \
	"${CAPTURE}" "${out}" "$((in_b / 1024))" "$((out_b / 1024))"
