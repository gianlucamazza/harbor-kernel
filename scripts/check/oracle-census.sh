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
#   3. product peak occupancy — **booted and read**, not documented — stays
#      well below the ceiling
#
# Rule 3 used to be a constant with a comment saying "not a measurement", and
# it drifted exactly as one would expect: the comment still read "Today 5/52"
# after the ceiling reached 54. ADR-0098 gave the kernel a slot meter and this
# gate a boot to read it from. It no longer knows the product's occupancy — it
# asks the product.
#
# Raising the ceiling: update EXPECTED_MAX_TASKS *and* the comment's ADR,
# update docs/architecture.md, and justify why it is not a density claim.
set -euo pipefail

cd "$(dirname "$0")/../.."

fail() {
	echo "oracle-census: FAIL — $1" >&2
	exit 1
}

# Last justified raise: ADR-0090 force-kill oracle pair (supervisor + EL0 child)
# — on top of ADR-0083's steal oracle (watcher + two thin victims).
# Do not bump without a named concurrent-demo reason (ADR-0085).
readonly EXPECTED_MAX_TASKS=54

# Headroom floor: product must not need more than half the ceiling without an
# explicit design ADR. The ratio catches a silent product census explosion or a
# MAX_TASKS collapse, not fine growth. The peak it multiplies comes from the
# boot below, so this is the only number here about the product, and it is a
# policy rather than a fact.
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

# Product occupancy: boot the shipped image and read ADR-0098's slot meter off
# the invariant beacon. The boot itself is the same one `product-boot-check`
# runs, from the same helper, so the two gates cannot end up describing
# different images — and the census re-runs it rather than reading a log some
# earlier target left behind, because a stale artefact is the same failure in a
# new place.
# shellcheck source=scripts/lib/product-boot.sh
source scripts/lib/product-boot.sh

boot_log="$(mktemp)"
trap 'rm -f "${boot_log}"' EXIT

skipped=0
product_boot_capture "${boot_log}" oracle-census || skipped=$?

if ((skipped == 2)); then
	echo "oracle-census: SKIPPED — no emulator, so the product peak cannot be measured" >&2
	echo "  The remembered constant it replaced is deliberately not a fallback (ADR-0098)." >&2
	exit 0
fi

# Largest watermark the product printed. `|| true` so a missing field reaches
# the message below instead of ending the gate with a bare status 1.
product_peak="$(
	grep -oaE 'slots=[0-9]+/[0-9]+' "${boot_log}" |
		cut -d/ -f2 |
		sort -n |
		tail -n1 || true
)"
[[ -n "${product_peak}" ]] ||
	fail "product boot printed no slots=<live>/<peak> field — the meter ADR-0098 added is gone, and this gate does not guess"

if ((product_peak * PRODUCT_CEILING_RATIO > src_max)); then
	fail "measured product peak ${product_peak} × ${PRODUCT_CEILING_RATIO} exceeds MAX_TASKS ${src_max} — the composition is at the slot wall ADR-0085 §3 names, and K5-H needs a design ADR before it moves"
fi

# Report oracle spawn pressure (informational, not a hard budget — demos
# are sequential and exit). Counts call sites, not peak concurrency.
oracle_spawns="$(
	{
		grep -E 'sched::spawn(_with_|_on|_thin|_mini)?' src/bootstrap/mod.rs src/bootstrap/demos.rs 2>/dev/null || true
	} | grep -c 'spawn' || true
)"

# Agents, in the open: the meter counts every occupied slot, and two of them
# are the per-CPU idle identities (ADR-0098 §1). Subtracting here beats a
# constant that nets them out before anyone can see it.
readonly IDLE_SLOTS=2
product_agents=$((product_peak - IDLE_SLOTS))

echo "oracle-census: clean (MAX_TASKS=${src_max}; measured product peak ${product_peak} slots = ${product_agents} + ${IDLE_SLOTS} idle; ~${oracle_spawns} oracle spawn call sites)"
echo "  Ceiling is oracle concurrent-demo tax (ADR-0085) — not a density win."
echo "  Peak is booted and read, not remembered (ADR-0098); K5-H stays deferred while it sits this far below the ceiling."
