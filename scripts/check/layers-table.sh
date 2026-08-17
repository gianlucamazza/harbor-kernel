#!/usr/bin/env bash
# Refuse a gate that runs in `make check` and is absent from the table that
# claims to list every gate — or one whose "Blind to" cell is empty.
#
# ## The failure this exists for
#
# `docs/verification.md` § The layers is the project's map of what each gate
# covers **and what it cannot see**. `CONTRIBUTING.md` said the quiet part out
# loud: that table "claims to list every layer and is checked by nobody". A map
# of blind spots that is itself unmaintained is worse than none, because it is
# read as complete.
#
# When this script was written the table was already missing two gates that
# `make check` runs — `panic-check` and `vocabulary-sync` — and neither
# omission had been noticed. That is the whole argument: the table went stale
# by attention, and attention does not survive a new gate.
#
# ## What it claims
#
# One direction, deliberately. Every prerequisite of `check:` has a row; a row
# may exist for something outside `check` (`x86-boot-check`, hardware) because
# the table is about layers of evidence, not about one target's contents. And
# every row in the section carries a non-empty fourth column, because a layer
# with no stated blind spot is a claim of omniscience.
set -euo pipefail

cd "$(dirname "$0")/../.."

readonly DOC="docs/verification.md"

violations=0
note() {
	echo "layers-table: $1" >&2
	violations=$((violations + 1))
}

# `check:`'s prerequisites, from the Makefile itself rather than from a copy.
mapfile -t gates < <(
	sed -n 's/^check: //p' Makefile | tr ' ' '\n' | grep -v '^$'
)

if [[ "${#gates[@]}" -lt 10 ]]; then
	note "parsed only ${#gates[@]} prerequisites from the Makefile's check: line"
	echo "layers-table: ${violations} problem(s)" >&2
	exit 1
fi

# The section is bounded: from `## The layers` to the next level-2 heading.
section="$(awk '/^## The layers$/ { on = 1; next } on && /^## / { exit } on' "${DOC}")"

# The section also holds smaller tables (the boot oracle's outcomes, the
# skip situations). Only the one whose header is `| Layer |` is the map.
in_table=0
rows=0
while IFS= read -r line; do
	# "| Layer " and not "| Layering", which also starts with "| Layer".
	if [[ "${line}" == "| Layer "*"| Runs"* ]]; then
		in_table=1
		continue
	fi
	[[ "${in_table}" -eq 1 ]] || continue
	if [[ "${line}" != \|* ]]; then
		in_table=0
		continue
	fi
	[[ "${line}" == *---* ]] && continue
	rows=$((rows + 1))
	# Fourth column of a `| a | b | c | d |` row.
	blind="$(awk -F'|' '{ print $5 }' <<<"${line}" | sed 's/^ *//; s/ *$//')"
	layer="$(awk -F'|' '{ print $2 }' <<<"${line}" | sed 's/^ *//; s/ *$//')"
	[[ -n "${blind}" ]] ||
		note "row '${layer}' has an empty 'Blind to' cell — a layer with no blind spot is a claim of omniscience"
done <<<"${section}"

if [[ "${rows}" -lt 10 ]]; then
	note "found only ${rows} rows under '## The layers'; the section moved or changed shape"
fi

# Matched inside the Layer column only, and without assuming the gate is alone
# in its parentheses: one row legitimately names two runners
# (`make panic-check`, `make hw-check`), and an exact-parenthesis match called
# that row missing. A false positive here is not harmless — it invites a
# duplicate row, which is the drift this gate exists to stop.
for gate in "${gates[@]}"; do
	awk -F'|' -v needle="\`make ${gate}\`" '
		/^\| Layer +\| Runs/ { on = 1; next }
		on && $0 !~ /^\|/ { on = 0 }
		on && index($2, needle) { found = 1 }
		END { exit(found ? 0 : 1) }
	' <<<"${section}" ||
		note "\`make ${gate}\` runs in 'make check' and has no row under '## The layers' in ${DOC}"
done

if [[ "${violations}" -ne 0 ]]; then
	echo "layers-table: ${violations} gate(s) missing from the map of blind spots" >&2
	exit 1
fi

echo "layers-table: clean (${#gates[@]} check gates, ${rows} layer rows, every row states what it cannot see)"
