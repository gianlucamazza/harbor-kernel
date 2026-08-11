#!/usr/bin/env bash
# Mutation-test the modules that carry authority and scheduling.
#
# Not a `make check` prerequisite: a full run is around seven minutes, and the
# value is in reading *which* mutants survived rather than in a number. The
# cadence is ADR-0058's: a fresh run before any commit that moves a boundary
# (new syscall/argument, new cap band, new authority module), and the file
# list below must gain every module that decides authority — `taskcap.rs`
# landed without joining it, which is how a new authority object went
# unmutated for a day (excellence review 2026-08-08, F-7).
#
# `cargo-mutants` exits 3 whenever anything survived, and nineteen things do:
# unreachable defensive branches, boundary guards, model-check-guarded arms,
# and a handful of *equivalent* mutants (`1 << 0` → `1 >> 0`; `|` → `^` on
# disjoint bits). `docs/verification.md` argues each one. A target that is red every time is a
# target nobody runs, so this compares against that documented baseline instead
# of against zero, and fails only when the number moves the wrong way.
set -uo pipefail

cd "$(dirname "$0")/../.." || exit 1

# Survivors justified in docs/verification.md § mutation testing (fourth run,
# 2026-08-08 — cargo-mutants now emits TWO operators per `+=`, so one arm is
# two mutants; the *sites* below are the same class as before, plus the new
# modules' equivalents):
#   6 × the `!mbox.live` arms in ipc send/try_recv/park (3 sites × -=/*=) —
#       unreachable by exhaustion within the model-check bound
#   2 × revoke_channel's `mb >= mailboxes.len()` state arm (-=/*=) — same
#       defensive-unreachable class: a live endpoint always names a valid mb
#   2 × release_holds boundaries: `send_holders > 0` at 0 (underflow guard)
#       and `n < N` at N (a call cannot release more waiters than caps)
#   3 × `Ok(None) if current != IDLE` in tasks — model-check-unreachable guard
#   2 × `INDEX_BASE | i` → `^` in taskcap/irqcap mint — equivalent (bands and
#       locals are disjoint bits; the const assert in taskcap.rs pins that)
#   1 × irqcap mint's generation-0 skip — reachable only at u16 wrap, and
#       irqcap has no revoke, so the generation never advances past first mint
#   1 × `CapRights::SEND = 1 << 0` → `1 >> 0` in cap — equivalent, not untested
# Sixth run (2026-08-09, ADR-0062 scope gains runqueue.rs and irqwait.rs):
#   1 × `epoch << 16 | slot` → `^` in runqueue's to_raw — equivalent (the two
#       halves are disjoint bits; same class as the band mints above)
#   1 × irqwait signal's `task.slot() < MAX_TASK_IDS` arm — defensive-
#       unreachable: `arm` refuses those slots, so no armed entry carries one
# Eighth run (2026-08-11, ADR-0091/0092 — and the first run since ADR-0062):
#   2 × equivalent in `tasks.rs`, both instances of the idle invariant already
#       argued above — `wake`'s idle guard (idle is never Blocked, so the state
#       check behind it refuses the same ids) and `switch_on`'s steal guard
#       (`try_steal_into` opens with the same emptiness check; the other
#       direction cannot happen). The other thirteen K8/steal survivors this
#       run surfaced were killed by tests, not absorbed here.
readonly BASELINE_MISSED=21

# `partition`'s loop counter mutated to a no-op never terminates. That is a
# detected mutant, not a surviving one — the suite would hang rather than pass —
# and cargo-mutants files it separately.
readonly BASELINE_TIMEOUT=1

# Where a clean run records what it covered (see the end of this script).
readonly STAMP="docs/mutation-stamp.toml"
run_date="$(date -u +%Y-%m-%d)"
run_commit="$(git describe --always 2>/dev/null || echo unknown)"

HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"

if ! command -v cargo-mutants >/dev/null; then
	echo "error: cargo-mutants not installed (cargo install cargo-mutants)" >&2
	exit 1
fi

# `CARGO_BUILD_TARGET` is not optional. `.cargo/config.toml` pins the bare-metal
# target, and cargo-mutants builds in a scratch directory where `-- --target`
# reaches `cargo test` but not the build before it — without this the run ends
# in several hundred compile errors that look like mutants and are not.
# One name per line; every module that decides authority belongs here.
readonly FILES=(
	ipc tasks layout irqtable rxline reset cap syscall prog manifest
	taskcap irqcap reply runqueue irqwait capslots lifecycle loaderplan
)
file_args=()
for f in "${FILES[@]}"; do
	file_args+=(--file "**/${f}.rs")
done

CARGO_BUILD_TARGET="${HOST_TARGET}" cargo mutants -p kernel-core "${file_args[@]}"
status=$?

# 0 = nothing survived, 3 = something did. Anything else is the tool failing.
if [[ "${status}" -ne 0 && "${status}" -ne 3 ]]; then
	echo "mutants: cargo-mutants failed (exit ${status})" >&2
	exit "${status}"
fi

# The artifact must cover the files this script asked for. Without this, a
# scoped or interrupted run leaves a short missed.txt that passes the baselines
# having verified nothing — the on-disk state this check was seen red against
# (a manifest-only re-run posing as the full result). ADR-0058 §2.
python3 - "${FILES[@]}" <<'PY'
import json, sys

want = {f"crates/kernel-core/src/{name}.rs" for name in sys.argv[1:]}
got = {m["file"] for m in json.load(open("mutants.out/mutants.json"))}
missing = sorted(want - got)
if missing:
    print("mutants: FAIL — the run did not cover:", ", ".join(missing), file=sys.stderr)
    print("  A partial artifact must not grade itself. Re-run without --file", file=sys.stderr)
    print("  narrowing, or fix the FILES list in this script.", file=sys.stderr)
    sys.exit(1)
extra = sorted(got - want)
if extra:
    print("mutants: run covered files the list does not name:", ", ".join(extra), file=sys.stderr)
    print("  Add them to FILES so the scope stays a decision.", file=sys.stderr)
    sys.exit(1)
PY

missed="$(wc -l <mutants.out/missed.txt)"
timeout="$(wc -l <mutants.out/timeout.txt)"

if [[ "${missed}" -gt "${BASELINE_MISSED}" || "${timeout}" -gt "${BASELINE_TIMEOUT}" ]]; then
	echo "mutants: FAIL — ${missed} survived (baseline ${BASELINE_MISSED}), ${timeout} timed out (baseline ${BASELINE_TIMEOUT})" >&2
	echo "  New survivors are the useful part of the result. Read them:" >&2
	sed 's/^/    /' mutants.out/missed.txt >&2
	echo "  Then either write the test that kills them, or justify them in" >&2
	echo "  docs/verification.md and raise the baseline in this script." >&2
	exit 1
fi

if [[ "${missed}" -lt "${BASELINE_MISSED}" ]]; then
	echo "mutants: ${missed} survived, fewer than the baseline of ${BASELINE_MISSED}."
	echo "  Lower the baseline in this script — a stale one hides the next regression."
fi

# Record what this run covered, so `make check` can tell when it has gone
# stale. The stamp is tracked in git precisely because the question it answers
# — "has the mutable surface moved since anyone last ran this?" — is about the
# repository's history, not about this working copy (ADR-0096).
mutant_count="$(cargo mutants --list -p kernel-core "${file_args[@]}" 2>/dev/null | wc -l)"
cat >"${STAMP}" <<STAMP
# Written by scripts/host/run-mutants.sh. Do not edit by hand.
#
# \`make mutation-freshness\` compares \`mutants\` against the surface
# \`cargo mutants --list\` reports today. A different number means the scope
# gained or lost something the last run never saw (ADR-0096).
date = ${run_date}
commit = ${run_commit}
mutants = ${mutant_count}
survivors = ${missed}
baseline = ${BASELINE_MISSED}
STAMP

echo "mutants: clean (${missed} survivors, all justified in docs/verification.md)"
echo "  stamped ${STAMP}: ${mutant_count} mutants at ${run_commit}"
