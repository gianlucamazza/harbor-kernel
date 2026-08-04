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

# 2. The host test count. `make test` prints one result line per target; only
#    the unit-test binary has tests, so the counts are summed rather than
#    assuming which line carries them.
actual="$(make test 2>/dev/null |
	sed -n 's/^test result: ok\. \([0-9]\+\) passed.*/\1/p' |
	awk '{ n += $1 } END { print n + 0 }')"
[[ "${actual}" -gt 0 ]] || fail "could not read a test count from 'make test'"

claimed="$(sed -n 's/^| Verification | \([0-9]\+\) host unit tests.*/\1/p' README.md)"
[[ -n "${claimed}" ]] || fail "README has no 'N host unit tests' claim to check"

if [[ "${claimed}" != "${actual}" ]]; then
	fail "README claims ${claimed} host unit tests, there are ${actual}"
fi

echo "doc-claims: clean (${actual} tests, ${#makefile_gates} chars of gate list agree)"
