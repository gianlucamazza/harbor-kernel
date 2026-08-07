#!/usr/bin/env bash
# Assert that nothing running before the MMU is enabled uses an atomic
# read-modify-write.
#
# Why this matters: with translation off every access is Device-nGnRnE, and the
# LDXR/STXR pair behind `swap`/`fetch_add`/`compare_exchange` makes no forward
# progress there on Cortex-A72. The retry loop spins forever — no output, no
# fault — and QEMU does not reproduce it, because TCG's exclusive monitor
# ignores memory attributes. It cost an afternoon once.
#
# The property is about the whole path, not one function. Checking only
# `early_mmu_enable` would be correct today by accident: `_start` happens to
# call nothing before it. So this derives the path from the image and fails if
# the path grows, instead of assuming it never will.
set -euo pipefail

ELF="${1:?usage: $0 <kernel elf>}"
OBJDUMP="${OBJDUMP:-llvm-objdump}"

# The linker folds `.text.boot` into `.text`, so the entry code is reachable
# only by symbol — which is why `boot.s` gives `_start` a type and a size.
ENTRY='_start'
GATE='early_mmu_enable'

exclusive_re='\b(ld|st)a?xr[bh]?\b|\bcas[ab]*l?[bh]?\b|\bswp[ab]*l?[bh]?\b'

# An indirect branch — `blr x0`, `br x1` — is a call edge whose target cannot be
# derived from the image. The guarantee here is not a property of one function;
# it propagates along call edges, so an edge this script cannot follow is a hole
# in it, not a detail. Refusing them is the only honest option: "follow `blr`
# too" is undecidable, and inspecting the reachable set minus the indirect ones
# would report coverage it does not have.
indirect_re='\b(blr|br)\s+x[0-9]+\b'

fail() {
	echo "error: $1" >&2
	exit 1
}

disasm_symbol() {
	$OBJDUMP -d --disassemble-symbols="$1" --no-show-raw-insn "${ELF}" 2>/dev/null
}

# 1. The entry code itself.
entry_disasm="$(disasm_symbol "${ENTRY}")"

# A vanished or renamed symbol must fail, not silently inspect nothing.
grep -q "<${ENTRY}>:" <<<"${entry_disasm}" ||
	fail "symbol '${ENTRY}' not found in ${ELF}: this check inspected nothing"

# `grep -q ... && fail` would end the script under `set -e` on the *success*
# path, where grep finds nothing and returns 1. Use `if`, not `&&`.
if grep -qE "${exclusive_re}" <<<"${entry_disasm}"; then
	fail "atomic read-modify-write in ${ENTRY}, which runs before the MMU"
fi

if grep -qE "${indirect_re}" <<<"${entry_disasm}"; then
	fail "indirect branch in ${ENTRY}: its target is not derivable, so the pre-MMU path can no longer be audited. Make the call direct, or move it after ${GATE}."
fi

# 2. What the entry code calls. A new `bl` here silently extends the pre-MMU
#    window — precisely the regression a per-function check cannot see.
# `|| true`: with `set -o pipefail` a grep that matches nothing — a leaf
# function, which is the healthy case — would fail the assignment and end the
# script. A benign non-match must not read as an error.
#
# `b` as well as `bl`. A direct tail branch leaves `_start` for code this script
# never disassembles, and it used to be neither audited nor refused — the check
# printed "clean" having walked past the edge. The thesis one screen up ("an
# edge this script cannot follow is a hole in it") applies to an edge it simply
# did not look for just as much as to an indirect one.
#
# Local labels (`.L_park`, `1f`) disassemble as `<_start+0x…>`, which the
# `+0x…` strip below turns back into `_start` — a branch within the entry code
# is not an edge out of it, and the self-name is dropped with the audited set.
callees="$(grep -oE '\b(bl|b)\b[^<]*<[^>]+>' <<<"${entry_disasm}" |
	grep -oE '<[^>]+>$' | tr -d '<>' | sed 's/+0x.*//' | sort -u || true)"

for callee in ${callees}; do
	case "${callee}" in
	"${GATE}" | kernel_main | "${ENTRY}") ;;
	*)
		fail "${ENTRY} calls '${callee}': the pre-MMU window now includes code this check does not inspect. Move the call after ${GATE}, or add it to the audited set in $0."
		;;
	esac
done

grep -q "${GATE}" <<<"${callees}" ||
	fail "${ENTRY} no longer calls ${GATE}: the MMU may not be enabled before Rust runs"

# 3. The gate and anything it calls. It is a leaf today; catch a new call
#    rather than assume it stays one.
gate_disasm="$(disasm_symbol "${GATE}")"
gate_callees="$(grep -oE '\bbl\b[^<]*<[^>]+>' <<<"${gate_disasm}" |
	grep -oE '<[^>]+>$' | tr -d '<>' | sed 's/+0x.*//' | sort -u || true)"

for symbol in "${GATE}" ${gate_callees}; do
	symbol_disasm="$(disasm_symbol "${symbol}")"
	if grep -qE "${exclusive_re}" <<<"${symbol_disasm}"; then
		fail "atomic read-modify-write in '${symbol}', reached before the MMU is on"
	fi
	if grep -qE "${indirect_re}" <<<"${symbol_disasm}"; then
		fail "indirect branch in '${symbol}', reached before the MMU is on: its target is not derivable, so what runs there cannot be audited"
	fi
done

echo "pre-mmu-path: clean (${ENTRY} → ${GATE}${gate_callees:+ → }${gate_callees})"
