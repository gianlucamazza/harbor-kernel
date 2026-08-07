#!/usr/bin/env bash
# `src/arch/` must not name a physical address. That is board knowledge, and
# rule 3 of `docs/architecture.md` reserves this tree for CPU and ISA.
#
# `make layering` already enforces the rule at the *import* level: nothing under
# `arch` may `use crate::bsp`. It cannot see the other way of knowing the board,
# which is to write its addresses out by hand — and that is exactly what
# happened. The early identity map sat in `arch/aarch64/mmu.rs` encoding "three
# gigabytes of RAM, then peripherals at 0xC000_0000" for a BCM2711, and it was
# **F23**: the last of thirty review findings to stay open, for two days, with a
# gate one directory away that could not see it.
#
# The signal is alignment, not magnitude. A physical range base is aligned to at
# least 256 MiB; a register encoding almost never is. `0x30d00800` (the
# `SCTLR_EL1` RES1 pattern), `0x0000FFFFFFFFF000` (the descriptor address mask)
# and `0xd00dfeed` (the DTB magic, a spec constant and not an address) all pass,
# and `0xC000_0000` does not.
#
# The one shape this refuses that it arguably should not is a bit-31 mask
# written as `0x80000000`. Write it `1 << 31`, which is what it means.
#
# Comments are stripped first: `bootinfo.rs` names `0x2eff1f00` in prose, as the
# address the firmware happened to choose on the test board, and prose about a
# board is not the same as depending on one.
set -euo pipefail

cd "$(dirname "$0")/../.."

# 256 MiB. Anything at least this large and aligned to it is a range base.
readonly ALIGNMENT=$((256 * 1024 * 1024))

violations=0
while IFS= read -r file; do
	# Strip `//` comments and the bodies of `/* */` is unnecessary — this tree
	# has none — but line comments carry board addresses on purpose.
	stripped="$(sed 's://.*::' "${file}")"
	while IFS= read -r literal; do
		[[ -z "${literal}" ]] && continue
		value="$(printf '%d' "${literal//_/}")"
		((value >= ALIGNMENT)) || continue
		((value % ALIGNMENT == 0)) || continue
		echo "arch-board-free: ${file} names ${literal}, a physical range base" >&2
		violations=$((violations + 1))
	done < <(grep -oE '0x[0-9A-Fa-f_]+' <<<"${stripped}" || true)
done < <(find src/arch -type f \( -name '*.rs' -o -name '*.s' \))

if [[ "${violations}" -ne 0 ]]; then
	echo "arch-board-free: ${violations} board address(es) inside the ISA tree" >&2
	echo "  Board knowledge belongs in src/bsp/<board>/memmap.rs; if the ISA tree" >&2
	echo "  needs it, something has to hand it over — see src/mm/early.rs." >&2
	exit 1
fi

echo "arch-board-free: clean ($(find src/arch -type f \( -name '*.rs' -o -name '*.s' \) | wc -l) files, no physical range bases)"
