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
#
#    All three kinds count, because `cargo test` runs all three and the README
#    says "host tests" without qualifying: unit tests beside the code,
#    integration tests that use the crate from outside, and doc-tests, which are
#    the examples in the public API and fail if the API changes under them.
unit="$(grep -rhc '^\s*#\[test\]' crates/kernel-core/src/*.rs |
	awk '{ n += $1 } END { print n + 0 }')"
integration="$(cat crates/kernel-core/tests/*.rs 2>/dev/null |
	grep -c '^\s*#\[test\]' || true)"
doc="$(grep -rhc '^\s*/// ```$' crates/kernel-core/src/*.rs |
	awk '{ n += $1 } END { print int((n + 0) / 2) }')"
actual=$((unit + integration + doc))
[[ "${unit}" -gt 0 ]] || fail "found no #[test] attributes under crates/kernel-core/src"

# Whitespace-tolerant on purpose: the markdown formatter re-aligns this table
# whenever a neighbouring row changes width, and a gate that a formatter can
# turn red is a gate people learn to work around.
claimed="$(sed -n 's/^|[[:space:]]*Verification[[:space:]]*|[[:space:]]*\([0-9]\+\) host tests.*/\1/p' README.md)"
[[ -n "${claimed}" ]] || fail "README has no 'N host tests' claim to check"

if [[ "${claimed}" != "${actual}" ]]; then
	fail "README claims ${claimed} host tests, there are ${actual} (${unit} unit, ${integration} integration, ${doc} doc)"
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

# 4. Every accepted ADR says when it was accepted. The field is not derivable
#    from `date:` — 0008 was proposed on the 4th and accepted on the 5th — and
#    five of sixteen had drifted without it, which nothing noticed because
#    nothing read the field. Reading it here is what makes it a claim.
missing_accept=""
for adr in docs/adr/0*.md; do
	grep -q '^status: accepted' "${adr}" || continue
	grep -q '^accepted: [0-9]' "${adr}" || missing_accept="${missing_accept} $(basename "${adr}")"
done
if [[ -n "${missing_accept}" ]]; then
	echo "doc-claims: accepted ADRs with no acceptance date:${missing_accept}" >&2
	exit 1
fi

# 5. The README's `## Layout` block names every module that exists.
#
#    It is the map a reader opens first, and it drifts the way the gate list did
#    (F27, twice): a module is added in one place and described in another. When
#    this check was written the block listed six of `kernel-core`'s twenty
#    modules and still described `irq/` as owning the dispatch table that had
#    moved out of it.
#
#    The block is read in two regions, because a bare substring search over the
#    whole thing would let `spi` under `drivers/` satisfy the claim about
#    `kernel_core::spi`. The `crates/kernel-core/` region runs to the `src/`
#    line; the `src/` region runs to the end of the block.
layout="$(awk '/^## Layout/ { inside = 1; next }
	inside && /^```/ { seen++; if (seen == 2) exit; next }
	inside && seen == 1 { print }' README.md)"
[[ -n "${layout}" ]] || fail "README has no '## Layout' code block"

core_region="$(awk '/^crates\/kernel-core\// { inside = 1 } /^src\// { inside = 0 } inside' <<<"${layout}")"
src_region="$(awk '/^src\// { inside = 1 } inside' <<<"${layout}")"
[[ -n "${core_region}" && -n "${src_region}" ]] ||
	fail "the Layout block has no 'crates/kernel-core/' or 'src/' section to read"

missing_modules=""
while IFS= read -r module; do
	grep -qwF -- "${module}" <<<"${core_region}" ||
		missing_modules="${missing_modules} kernel_core::${module}"
done < <(sed -n 's/^pub mod \([a-z0-9_]*\);$/\1/p' crates/kernel-core/src/lib.rs)

# Directories are the kernel's own modules; the loose `.rs` files at the top of
# `src/` are modules too and the block already names them one per line.
while IFS= read -r module; do
	grep -qwF -- "${module}" <<<"${src_region}" ||
		missing_modules="${missing_modules} src/${module}"
done < <({
	find src -mindepth 1 -maxdepth 1 -type d -printf '%f\n'
	find src -mindepth 1 -maxdepth 1 -name '*.rs' -printf '%f\n'
} | sort)

if [[ -n "${missing_modules}" ]]; then
	echo "doc-claims: modules that exist and the README Layout block does not name:" >&2
	echo "  ${missing_modules# }" >&2
	exit 1
fi

core_modules="$(sed -n 's/^pub mod \([a-z0-9_]*\);$/\1/p' crates/kernel-core/src/lib.rs | wc -l)"

echo "doc-claims: clean (${actual} tests = ${unit}+${integration}+${doc}, ${#makefile_gates} chars of gate list agree, \
$(wc -l <<<"${facade}") facade modules in the contract, \
${core_modules} kernel-core modules in the README, \
$(grep -lc '^status: accepted' docs/adr/0*.md | wc -l) ADRs dated)"
