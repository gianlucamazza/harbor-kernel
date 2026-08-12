#!/usr/bin/env bash
# The composition's vocabulary, compared between the two files that state it.
#
# ADR-0099: a store entry grants authority by writing an **index** into its
# slots, `src/bootstrap/authority.rs` declares what those integers mean, and
# `scripts/agent/pack-store.py` writes them. That is one fact in two places, and
# this project has already got that wrong twice — the oracle-marker list
# (`product-image.sh` now derives it) and the `MAX_TASKS` census (`oracle-census`
# now compares three copies).
#
# What a disagreement costs here is worse than a stale number: an agent composed
# to hold the console would silently hold whatever else the kernel declared at
# that index. The kernel would print `loaded`, the loader `refusals=0`, and the
# wrong authority would have been granted by arithmetic that was correct on its
# own terms.
#
# What this settles: the names and indices agree, in both directions. What it
# does not: whether an index is *minted* at boot — that is a vacancy, it is a
# runtime fact, and `authority: N <name> VACANT` on the wire is where it shows.
set -euo pipefail

cd "$(dirname "$0")/../.."

readonly KERNEL=src/bootstrap/authority.rs
readonly PACKER=scripts/agent/pack-store.py

fail() {
	echo "vocabulary-sync: FAIL — $1" >&2
	exit 1
}

[[ -f "${KERNEL}" ]] || fail "${KERNEL} is missing"
[[ -f "${PACKER}" ]] || fail "${PACKER} is missing"

# Kernel side: pairs of `pub const <P>_<X>: u8 = <n>;` and
# `NAME_<X>` / `WINDOW_NAME_<X>: &str = "<name>"`. The name constant is what the
# two files actually share; the index constant is what a store entry carries.
# Both must be present for a position to count.
kernel_side() {
	local idx_prefix="$1" name_prefix="$2"
	awk -v ip="${idx_prefix}" -v np="${name_prefix}" '
		match($0, "^pub const " ip "([A-Z0-9_]+): u8 = ([0-9]+);", m) { idx[m[1]] = m[2] }
		match($0, "^pub const " np "([A-Z0-9_]+): &str = \"([^\"]+)\";", m) { nm[m[1]] = m[2] }
		END {
			for (k in idx) {
				if (!(k in nm)) { printf "MISSING_NAME %s\n", k; continue }
				printf "%s %s\n", nm[k], idx[k]
			}
			for (k in nm) if (!(k in idx)) printf "MISSING_INDEX %s\n", k
		}
	' "${KERNEL}" | sort
}

# Packer side: a `<NAME> = { "name": n, ... }` table, possibly with a type
# annotation and possibly closed on its own line.
#
# The name is anchored with a word boundary rather than a following space: the
# first version of this required `^NAME ` and so silently parsed *nothing* out
# of `WINDOWS: dict[str, int] = {}`, which made the window half of this gate
# compare an empty table against an empty table and report clean either way. A
# gate that cannot fail is not a gate — seen, and fixed here.
packer_side() {
	awk -v table="$1" '
		$0 ~ ("^" table "([[:space:]:=]|$)") {
			inside = 1
			# A table closed on its declaration line holds nothing.
			if ($0 ~ /\{[[:space:]]*\}/) inside = 0
			next
		}
		inside && /^\}/ { inside = 0 }
		inside && match($0, /"([^"]+)":[[:space:]]*([0-9]+)/, m) { printf "%s %s\n", m[1], m[2] }
	' "${PACKER}" | sort
}

kernel_pairs="$(kernel_side "HELD_" "NAME_")"

if grep -q '^MISSING_' <<<"${kernel_pairs}"; then
	fail "${KERNEL} declares a HELD_ index with no NAME_, or the reverse: $(grep '^MISSING_' <<<"${kernel_pairs}" | tr '\n' ' ')"
fi

packer_pairs="$(packer_side "HELD")"

[[ -n "${kernel_pairs}" ]] || fail "no HELD_/NAME_ pair parsed from ${KERNEL} — refusing to report clean"
[[ -n "${packer_pairs}" ]] || fail "no HELD table parsed from ${PACKER} — refusing to report clean"

if [[ "${kernel_pairs}" != "${packer_pairs}" ]]; then
	echo "vocabulary-sync: FAIL — the two copies of the vocabulary disagree" >&2
	echo "--- ${KERNEL}" >&2
	echo "${kernel_pairs}" >&2
	echo "--- ${PACKER}" >&2
	echo "${packer_pairs}" >&2
	echo "  A store slot naming an index the kernel declared for something else" >&2
	echo "  is granted by arithmetic, silently (ADR-0099)." >&2
	exit 1
fi

# The device-window vocabulary (ADR-0100), compared the same way. It is empty
# in this product, and an empty pair of tables is a *match* rather than a
# refusal — declaring no window is the shipped state, not a parse failure.
kernel_windows="$(kernel_side "WINDOW_" "WINDOW_NAME_")"
packer_windows="$(packer_side "WINDOWS")"

if grep -q '^MISSING_' <<<"${kernel_windows}"; then
	fail "${KERNEL} declares a WINDOW_ index with no WINDOW_NAME_, or the reverse: $(grep '^MISSING_' <<<"${kernel_windows}" | tr '\n' ' ')"
fi

if [[ "${kernel_windows}" != "${packer_windows}" ]]; then
	echo "vocabulary-sync: FAIL — the two copies of the window vocabulary disagree" >&2
	echo "--- ${KERNEL}" >&2
	echo "${kernel_windows}" >&2
	echo "--- ${PACKER}" >&2
	echo "${packer_windows}" >&2
	echo "  A store entry naming an index the kernel declared for another device" >&2
	echo "  is mapped by arithmetic, silently (ADR-0100)." >&2
	exit 1
fi

n="$(wc -l <<<"${kernel_pairs}")"
w=0
[[ -n "${kernel_windows}" ]] && w="$(wc -l <<<"${kernel_windows}")"
echo "vocabulary-sync: clean (${n} capability position(s), ${w} device window(s), kernel and packer agree)"
while IFS= read -r position; do
	echo "  ${position}"
done <<<"${kernel_pairs}"
if [[ -n "${kernel_windows}" ]]; then
	while IFS= read -r position; do
		echo "  window ${position}"
	done <<<"${kernel_windows}"
fi
