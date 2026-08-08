#!/usr/bin/env bash
# Refuse any `static mut` declaration under `src/` and `crates/` (ADR-0019).
#
# Rule 7 of `docs/architecture.md` says shared IRQ/main state uses atomics —
# never `static mut`. That rule read as absolute while the tree still had one
# exception (`CURRENT_EL0`), defended by a false premise about linker-visible
# names. ADR-0019 retracts the premise and lands the last symbol as an
# `AtomicPtr`; this gate is what keeps a second exception from landing the way
# the first one did — as a comment nobody re-checked.
#
# Only *declarations* count:
#   - Line comments are stripped first (prose that names the form is fine).
#   - The lifetime form `&'static mut T` contains the same two words after a
#     quote — match only when `static` is not the body of `'static`.
#
# Seen red against the pre-migration tree (`static mut CURRENT_EL0` in
# `src/arch/aarch64/el0.rs`). Seen green after the atomic landing.
set -euo pipefail

cd "$(dirname "$0")/../.."

violations=0
while IFS= read -r file; do
	# Drop `//` line comments; prose about the form must not trip the gate.
	stripped="$(sed 's://.*::' "${file}")"
	while IFS= read -r hit; do
		[[ -z "${hit}" ]] && continue
		echo "no-static-mut: ${file}:${hit}" >&2
		violations=$((violations + 1))
	done < <(grep -nE "(^|[^'])static[[:space:]]+mut[[:space:]]+[A-Za-z_]" <<<"${stripped}" || true)
done < <(find src crates -type f -name '*.rs')

# The keyword pair is not the only way to get `static mut`'s ergonomics back:
# a `fn … -> &'static mut T` accessor mints the same unbounded aliasable
# borrow from an innocent-looking call (excellence review F-26 — two such
# accessors existed and this gate certified a property the code did not
# have). Refuse the shape outside the one argued exception: `el0::current`,
# whose `AtomicPtr` publish/assert machinery is ADR-0019's own replacement
# for the last `static mut` and is audited in place.
allowed_static_mut_fns='src/arch/aarch64/el0.rs'
while IFS= read -r hit; do
	[[ -z "${hit}" ]] && continue
	file="${hit%%:*}"
	rest="${hit#*:}"
	line="${rest%%:*}"
	content="${rest#*:}"
	content="${content%%//*}"
	[[ "${content}" == *"-> &'static mut"* ]] || continue
	case " ${allowed_static_mut_fns} " in
	*" ${file} "*) ;;
	*)
		echo "no-static-mut: ${file}:${line}: fn returning &'static mut — static mut ergonomics without the keyword" >&2
		echo "  Scope the borrow to a closure (with_region / with_state pattern)," >&2
		echo "  or add the file here with its argument." >&2
		violations=$((violations + 1))
		;;
	esac
done < <(grep -rn --include='*.rs' -e "-> &'static mut" src crates || true)

if [[ "${violations}" -ne 0 ]]; then
	echo "no-static-mut: ${violations} violation(s) under src/ crates/" >&2
	echo "  Rule 7 forbids static mut; shared state is Atomic* or SyncCell." >&2
	echo "  See docs/adr/0019-no-static-mut.md." >&2
	exit 1
fi

nfiles="$(find src crates -type f -name '*.rs' | wc -l)"
echo "no-static-mut: clean (${nfiles} files, no static mut declarations)"
