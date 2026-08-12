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

# Kernel side: pairs of `pub const HELD_<X>: u8 = <n>;` and `NAME_<X>: &str = "<name>"`.
# The name constant is what the two files actually share; the HELD_ constant is
# what a store entry carries. Both must be present for a position to count.
kernel_pairs="$(
	awk '
		match($0, /^pub const HELD_([A-Z0-9_]+): u8 = ([0-9]+);/, m) { idx[m[1]] = m[2] }
		match($0, /^pub const NAME_([A-Z0-9_]+): &str = "([^"]+)";/, m) { nm[m[1]] = m[2] }
		END {
			for (k in idx) {
				if (!(k in nm)) { printf "MISSING_NAME %s\n", k; continue }
				printf "%s %s\n", nm[k], idx[k]
			}
			for (k in nm) if (!(k in idx)) printf "MISSING_INDEX %s\n", k
		}
	' "${KERNEL}" | sort
)"

if grep -q '^MISSING_' <<<"${kernel_pairs}"; then
	fail "${KERNEL} declares a HELD_ index with no NAME_, or the reverse: $(grep '^MISSING_' <<<"${kernel_pairs}" | tr '\n' ' ')"
fi

# Packer side: the `HELD = { "name": n, ... }` table.
packer_pairs="$(
	awk '
		/^HELD = \{/ { inside = 1; next }
		inside && /^\}/ { inside = 0 }
		inside && match($0, /"([^"]+)":[[:space:]]*([0-9]+)/, m) { printf "%s %s\n", m[1], m[2] }
	' "${PACKER}" | sort
)"

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

n="$(wc -l <<<"${kernel_pairs}")"
echo "vocabulary-sync: clean (${n} declared position(s), kernel and packer agree)"
while IFS= read -r position; do
	echo "  ${position}"
done <<<"${kernel_pairs}"
