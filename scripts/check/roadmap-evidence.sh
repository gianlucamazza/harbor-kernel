#!/usr/bin/env bash
# A status flip is a claim about evidence, and the evidence index is
# `docs/verification.md`. Three consecutive `done (QEMU)` flips — ASID
# (ADR-0050), resolve-grant (ADR-0052), peer transfer (ADR-0054) — landed with
# no row there, while line 589 of the index still called two of them residual.
# Nothing read the index: `doc-symbols` skips it, `xrefs` checks its links,
# `doc-claims` never opens it (excellence review 2026-08-08, F-6).
#
# This closes the mechanical half: every ADR cited on a roadmap line that says
# `done (QEMU)` or `done (HW)` must at least appear in the evidence index.
# Whether the entry is *good* evidence stays review's job — sets, not
# semantics, like every gate in this directory.
#
# Seen red on the day it was written: 22 of 27 cited ADRs had no mention.
set -euo pipefail

cd "$(dirname "$0")/../.."

cited="$(grep -E 'done \((QEMU|HW)\)' docs/roadmap.md |
	grep -oE 'ADR-[0-9]{4}' | sort -u)"
[[ -n "${cited}" ]] || {
	echo "roadmap-evidence: no done-row ADR citations found — refusing to report clean" >&2
	exit 1
}

missing=""
while IFS= read -r id; do
	grep -q "${id}" docs/verification.md || missing="${missing} ${id}"
done <<<"${cited}"

if [[ -n "${missing}" ]]; then
	echo "roadmap-evidence: roadmap marks work done citing ADRs the evidence index never mentions:${missing}" >&2
	echo "  Add the oracle/transcript row to docs/verification.md, or do not flip the status." >&2
	exit 1
fi

echo "roadmap-evidence: clean ($(wc -l <<<"${cited}") ADRs cited by done rows, all present in docs/verification.md)"
