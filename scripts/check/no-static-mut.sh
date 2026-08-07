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

if [[ "${violations}" -ne 0 ]]; then
	echo "no-static-mut: ${violations} declaration(s) under src/ crates/" >&2
	echo "  Rule 7 forbids static mut; shared state is Atomic* or SyncCell." >&2
	echo "  See docs/adr/0019-no-static-mut.md." >&2
	exit 1
fi

nfiles="$(find src crates -type f -name '*.rs' | wc -l)"
echo "no-static-mut: clean (${nfiles} files, no static mut declarations)"
