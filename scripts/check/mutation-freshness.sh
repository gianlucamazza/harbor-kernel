#!/usr/bin/env bash
# Refuse a tree whose mutable surface has moved since the last mutation run.
#
# ## The failure this exists for
#
# ADR-0058 §2 says a module joins `run-mutants.sh`'s FILES list the commit it
# is born, and that rule was followed — `runqueue` and `irqwait` joined for
# ADR-0062. What nothing said is that the list must be **re-run** when modules
# already on it grow. K8 then landed queues, per-core timers, EL0-on-CPU1 and
# work stealing across twenty-odd commits, all of it `done (HW)`, and the next
# run (2026-08-11) found fourteen survivors in code nobody had written that
# week. Every one was killable by a test the day it landed. What was missing
# was a run.
#
# A rule that depends on someone remembering is the shape this project refuses
# everywhere else, so this is the gate for it (ADR-0096).
#
# ## Why the count, and not a hash or a date
#
# A hash of the scope files goes red on a comment. A date goes red on a
# calendar rather than on a change. `cargo mutants --list` reports the mutants
# the engine would generate — it parses, it does not run — so the number moves
# when and only when there is new mutable surface: a new function, a new
# branch, a new operator. Renaming a variable or rewriting a doc comment
# leaves it alone. It costs about two seconds.
#
# Consequence worth stating: this catches surface that *appeared*, not tests
# that stopped killing what was already there. A mutation run is still the only
# thing that answers the second question, and this gate cannot replace it.
#
# Seen red: with `docs/mutation-stamp.toml` recording 607 mutants, adding one
# `if` to `kernel_core::lifecycle` reported `608 mutants, stamp says 607`,
# exit 1.
set -euo pipefail

cd "$(dirname "$0")/../.."

readonly STAMP="docs/mutation-stamp.toml"

if [[ ! -f "${STAMP}" ]]; then
	echo "mutation-freshness: FAIL — ${STAMP} is missing" >&2
	echo "  Run 'make mutants'; a clean run writes it." >&2
	exit 1
fi

stamped_count="$(sed -nE 's/^mutants = ([0-9]+)$/\1/p' "${STAMP}")"
stamped_date="$(sed -nE 's/^date = (.+)$/\1/p' "${STAMP}")"
stamped_commit="$(sed -nE 's/^commit = (.+)$/\1/p' "${STAMP}")"

if [[ -z "${stamped_count}" ]]; then
	echo "mutation-freshness: FAIL — ${STAMP} has no 'mutants = N' line" >&2
	echo "  It is written by scripts/host/run-mutants.sh; re-run 'make mutants'." >&2
	exit 1
fi

if ! command -v cargo-mutants >/dev/null; then
	echo "mutation-freshness: FAIL — cargo-mutants is not installed" >&2
	echo "  cargo install cargo-mutants" >&2
	echo "  The stamp cannot be checked against a surface nothing can measure," >&2
	echo "  and a gate that passes when it cannot look is not a gate." >&2
	exit 1
fi

# The same file list the run uses, read from the run's own script so the two
# cannot drift: a scope that differs between them would make this gate compare
# a surface to a stamp of a different surface.
mapfile -t files < <(
	sed -n '/^readonly FILES=(/,/^)/p' scripts/host/run-mutants.sh |
		sed -e '1d' -e '$d' |
		tr -s ' \t' '\n' |
		grep -v '^$'
)

if [[ "${#files[@]}" -lt 10 ]]; then
	echo "mutation-freshness: FAIL — parsed only ${#files[@]} scope files from run-mutants.sh" >&2
	echo "  The FILES block moved or changed shape; this gate would otherwise" >&2
	echo "  compare a truncated surface and pass." >&2
	exit 1
fi

file_args=()
for f in "${files[@]}"; do
	file_args+=(--file "**/${f}.rs")
done

current="$(cargo mutants --list -p kernel-core "${file_args[@]}" 2>/dev/null | wc -l)"

if [[ "${current}" -ne "${stamped_count}" ]]; then
	echo "mutation-freshness: FAIL — ${current} mutants, stamp says ${stamped_count}" >&2
	echo "  The mutable surface moved since the run of ${stamped_date} (${stamped_commit})." >&2
	if [[ "${current}" -gt "${stamped_count}" ]]; then
		echo "  $((current - stamped_count)) new mutant(s): decisions no run has ever tried to break." >&2
	else
		echo "  $((stamped_count - current)) fewer: surface was removed, and the stamp still claims it." >&2
	fi
	echo "  Run 'make mutants'. It rewrites the stamp when it comes back clean." >&2
	exit 1
fi

echo "mutation-freshness: clean (${current} mutants, run ${stamped_date} at ${stamped_commit})"
