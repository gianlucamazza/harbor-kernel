#!/usr/bin/env bash
# Oracle census vs product occupancy — ratchet guard for MAX_TASKS.
#
# ADR-0085 forbids raising `MAX_TASKS` as a density win. Multi-role
# 2026-08-10 F-R7-1: the climb to 52 is **oracle tax** (concurrent demos),
# not product composition. Nothing used to compare the three places that
# name the bound, so a silent raise in `src/sched` left architecture and
# product narrative behind (same class as SECURITY residual drift).
#
# This gate settles only what a machine can settle:
#   1. source `MAX_TASKS` == architecture capacity table
#   2. source `MAX_TASKS` == the documented last-raise reason below
#   3. product peak occupancy (documented) stays well below the ceiling
#
# Raising the ceiling: update EXPECTED_MAX_TASKS *and* the comment's ADR,
# update docs/architecture.md, and justify why it is not a density claim.
set -euo pipefail

cd "$(dirname "$0")/../.."

fail() {
	echo "oracle-census: FAIL — $1" >&2
	exit 1
}

# Last justified raise: ADR-0083 steal oracle (watcher + two thin victims).
# Do not bump without a named concurrent-demo reason (ADR-0085).
# Last justified raise: ADR-0090 force-kill oracle pair (supervisor + EL0 child).
readonly EXPECTED_MAX_TASKS=54

# Product peak concurrent slots after composition steadies (not a measurement):
#   idle0 + idle1 + console-server + beacon + chirp = 5
# Core1 marker exits before the store agents finish. Raising product load
# without a density design does not justify EXPECTED_MAX_TASKS.
readonly PRODUCT_PEAK_SLOTS=5
# Headroom floor: product must not need more than half the ceiling without
# an explicit design ADR. Today 5/52; the ratio catches a silent product
# census explosion or a MAX_TASKS collapse, not fine growth.
readonly PRODUCT_CEILING_RATIO=2 # product_peak * ratio <= max_tasks

src_max="$(
	sed -n 's/^pub const MAX_TASKS: usize = \([0-9][0-9]*\);$/\1/p' src/sched/mod.rs |
		head -n1
)"
[[ -n "${src_max}" ]] || fail "could not parse pub const MAX_TASKS from src/sched/mod.rs"
[[ "${src_max}" == "${EXPECTED_MAX_TASKS}" ]] ||
	fail "MAX_TASKS is ${src_max} in source, census expects ${EXPECTED_MAX_TASKS} (update this script + architecture table with ADR reason)"

arch_max="$(
	# Capacity table row: | `sched::MAX_TASKS` | **N** | ...
	# shellcheck disable=SC2016 # literal backticks in the markdown cell
	grep -E '^\| `sched::MAX_TASKS`' docs/architecture.md |
		grep -oE '\*\*[0-9]+\*\*' |
		tr -d '*' |
		head -n1
)"
[[ -n "${arch_max}" ]] || fail "architecture.md capacity table has no sched::MAX_TASKS row"
[[ "${arch_max}" == "${src_max}" ]] ||
	fail "architecture.md says MAX_TASKS=${arch_max}, source says ${src_max}"

if ((PRODUCT_PEAK_SLOTS * PRODUCT_CEILING_RATIO > src_max)); then
	fail "product peak ${PRODUCT_PEAK_SLOTS} × ${PRODUCT_CEILING_RATIO} exceeds MAX_TASKS ${src_max}"
fi

# Report oracle spawn pressure (informational, not a hard budget — demos
# are sequential and exit). Counts call sites, not peak concurrency.
oracle_spawns="$(
	{
		grep -E 'sched::spawn(_with_|_on|_thin|_mini)?' src/bootstrap/mod.rs src/bootstrap/demos.rs 2>/dev/null || true
	} | grep -c 'spawn' || true
)"

echo "oracle-census: clean (MAX_TASKS=${src_max}; product peak ≤${PRODUCT_PEAK_SLOTS}; ~${oracle_spawns} oracle spawn call sites)"
echo "  Ceiling is oracle concurrent-demo tax (ADR-0085) — not a density win."
