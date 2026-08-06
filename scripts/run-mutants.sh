#!/usr/bin/env bash
# Mutation-test the modules that carry authority and scheduling.
#
# Not a `make check` prerequisite: a full run is around seven minutes, and the
# value is in reading *which* mutants survived rather than in a number. The
# cadence is the one ADR-0001 sets for the multi-role review — before a
# milestone that moves a boundary.
#
# `cargo-mutants` exits 3 whenever anything survived, and nine things do: they
# are on two provably unreachable defensive branches, and `docs/verification.md`
# argues each one. A target that is red every time is a target nobody runs, so
# this compares against that documented baseline instead of against zero, and
# fails only when the number moves the wrong way.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

# Survivors justified in docs/verification.md § mutation testing:
#   6 × `!mbox.live` in ipc — no endpoint is ever released, so it cannot hold
#   3 × `Ok(None) if current != IDLE` in tasks — idle is always current or queued
readonly BASELINE_MISSED=9

# `partition`'s loop counter mutated to a no-op never terminates. That is a
# detected mutant, not a surviving one — the suite would hang rather than pass —
# and cargo-mutants files it separately.
readonly BASELINE_TIMEOUT=1

HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"

if ! command -v cargo-mutants >/dev/null; then
	echo "error: cargo-mutants not installed (cargo install cargo-mutants)" >&2
	exit 1
fi

# `CARGO_BUILD_TARGET` is not optional. `.cargo/config.toml` pins the bare-metal
# target, and cargo-mutants builds in a scratch directory where `-- --target`
# reaches `cargo test` but not the build before it — without this the run ends
# in several hundred compile errors that look like mutants and are not.
CARGO_BUILD_TARGET="${HOST_TARGET}" cargo mutants -p kernel-core \
	--file '**/ipc.rs' --file '**/tasks.rs' --file '**/layout.rs' \
	--file '**/irqtable.rs' --file '**/rxline.rs' --file '**/reset.rs'
status=$?

# 0 = nothing survived, 3 = something did. Anything else is the tool failing.
if [[ "${status}" -ne 0 && "${status}" -ne 3 ]]; then
	echo "mutants: cargo-mutants failed (exit ${status})" >&2
	exit "${status}"
fi

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

echo "mutants: clean (${missed} survivors, all justified in docs/verification.md)"
