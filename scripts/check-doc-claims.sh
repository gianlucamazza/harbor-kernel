#!/usr/bin/env bash
# Assert that the two checkable claims in README.md are still true.
#
# Both have drifted before. The gate list drifted twice — F27 in the review,
# then again on the very commit that added `bringup-builds`, because a gate is
# added to the Makefile and the README is somewhere else. The test count drifted
# by 23. Neither is a documentation problem: a README that lists the wrong gates
# is how someone concludes a check exists when it does not.
#
# Only claims a machine can settle belong here. Prose stays prose.
set -euo pipefail

cd "$(dirname "$0")/.."

fail() {
	echo "doc-claims: FAIL — $1" >&2
	exit 1
}

# 1. The `make check` line must name exactly the prerequisites of the `check`
#    target, in order. Extracted rather than compared by eye.
makefile_gates="$(sed -n 's/^check:[[:space:]]*//p' Makefile)"
[[ -n "${makefile_gates}" ]] || fail "no 'check:' target found in Makefile"

readme_gates="$(sed -n 's/^make check *# *\(.*\), then clippy$/\1/p' README.md)"
[[ -n "${readme_gates}" ]] || fail "README has no 'make check' line in the expected form"

if [[ "${makefile_gates}" != "${readme_gates}" ]]; then
	echo "doc-claims: the README lists different gates than the Makefile runs" >&2
	echo "  Makefile: ${makefile_gates}" >&2
	echo "  README:   ${readme_gates}" >&2
	exit 1
fi

# 2. The host test count. Counted from the source rather than by running the
#    suite: `make check` has already run `make test` by the time it gets here,
#    and re-running it doubled the cheapest gate while `2>/dev/null` swallowed
#    any build error into a generic "could not read a test count".
#
#    `#[test]` attributes are counted directly. That is a different claim from
#    "tests that passed" — but `make check` runs `test` before `doc-claims`, so
#    a red suite has already stopped the gate before this line is reached.
actual="$(grep -rhc '^\s*#\[test\]' crates/kernel-core/src/*.rs |
	awk '{ n += $1 } END { print n + 0 }')"
[[ "${actual}" -gt 0 ]] || fail "found no #[test] attributes under crates/kernel-core/src"

claimed="$(sed -n 's/^| Verification | \([0-9]\+\) host unit tests.*/\1/p' README.md)"
[[ -n "${claimed}" ]] || fail "README has no 'N host unit tests' claim to check"

if [[ "${claimed}" != "${actual}" ]]; then
	fail "README claims ${claimed} host unit tests, there are ${actual}"
fi

# 3. The arch facade's re-export list, which ADR-0015 duplicates as a table in
#    `arch-contract.md`. The contract is what a port is checked against, so a
#    module that leaves the facade and stays in the table is a port built to a
#    surface that no longer exists. `docs/mmu.md` records what happens to a fact
#    kept in two files with nothing comparing them: both copies go stale
#    together.
facade="$(sed -n 's/^pub use aarch64::{\(.*\)};$/\1/p' src/arch/mod.rs |
	tr ',' '\n' | tr -d ' ' | grep . | sort -u)"
[[ -n "${facade}" ]] || fail "no 'pub use aarch64::{…}' re-export list found in src/arch/mod.rs"

# One direction only: every facade module must appear in the contract. The
# reverse would fire on the BSP table in the same file, which names modules the
# arch facade has no business re-exporting.
# shellcheck disable=SC2016  # the backticks are markdown table syntax, not a
# command substitution: the pattern matches "| `mmu` |" in arch-contract.md.
contract="$(sed -n 's/^| `\([a-z0-9_]\+\)` |.*/\1/p' docs/arch-contract.md | sort -u)"
missing="$(comm -23 <(echo "${facade}") <(echo "${contract}"))"
if [[ -n "${missing}" ]]; then
	echo "doc-claims: the arch facade re-exports modules arch-contract.md does not list" >&2
	echo "  missing from the contract: ${missing//$'\n'/, }" >&2
	exit 1
fi

echo "doc-claims: clean (${actual} tests, ${#makefile_gates} chars of gate list agree, \
$(wc -l <<<"${facade}") facade modules in the contract)"
