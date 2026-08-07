#!/usr/bin/env bash
# The boot oracle must not be in an image that does not ask for it.
#
# Rule 9 of `docs/architecture.md` says diagnostic scaffolding lives behind a
# feature rather than in the production surface. `src/bootstrap/demos.rs` was 544
# lines of demo agents with no `cfg` at all, compiled into every image — the same
# shape as the rule-7 exception ADR-0019 closed, and unnoticed for the same
# reason: `make layering` checks import edges, and nothing checked *what lands in
# the image*.
#
# Compiling without the feature is not enough on its own. A demo can come back
# through another path — a `pub use`, a call left outside its `cfg` — and the
# build would still succeed. So the assertion is on the **ELF**: no symbol from
# the demo module may be there.
#
# ## What this gate also reports, and why it is not a failure
#
# Without the oracle a large part of the kernel becomes unreachable: `Agent`,
# `AddressSpace`, the EL0 session machinery, the PL011 RX handover. That is not
# rot. **Nothing in the product creates an agent**, because an agent's text has
# to be compiled into the kernel — there is no loader. The count below is that
# gap expressed as a number instead of an opinion, and it is the argument for the
# loader milestone.
#
# So this gate does not run clippy with `-D warnings` on the product
# configuration. Refusing an unreachable `Agent` would be refusing the kernel for
# a missing feature it has never claimed to have.
set -euo pipefail

cd "$(dirname "$0")/.."

readonly TARGET=aarch64-unknown-none-softfloat
readonly OUT="target/${TARGET}/release"

command -v llvm-nm >/dev/null || {
	echo "product-builds: FAIL — llvm-nm is not on the PATH" >&2
	echo "  Refusing to report clean without reading the symbol table." >&2
	exit 1
}

# The oracle image first: the marker set is validated against it.
cargo build --target "${TARGET}" --release >/dev/null
llvm-objcopy -O binary "${OUT}/harbor-kernel" "${OUT}/kernel8.img"
oracle_size="$(stat -c %s "${OUT}/kernel8.img")"

echo "product-builds: building without the oracle"
cargo build --target "${TARGET}" --release --no-default-features --features board-rpi4 \
	>/dev/null || {
	echo "product-builds: FAIL — the image does not build without the oracle" >&2
	exit 1
}

product_elf="${OUT}/harbor-kernel"
# The image, not the ELF: the ELF carries debug info that `objcopy -O binary`
# drops, so comparing ELFs would report a difference nobody flashes.
llvm-objcopy -O binary "${product_elf}" "${OUT}/kernel8-product.img"
product_size="$(stat -c %s "${OUT}/kernel8-product.img")"

# Every console string the demos can print, taken **from the source** rather
# than listed here. A hand-written list is a fact in two places, and it went
# wrong twice while this gate was being written:
#
#   1. The first version grepped `llvm-nm` for `bootstrap::demos` and reported
#      *clean* with four kilobytes of demo code in the image — release LTO
#      renames and inlines, so the module path is not in the symbol table.
#   2. The second listed six markers by hand and still passed the same leak,
#      because the leaked function was `m5_aspace_and_el0_smoke`, whose output
#      none of the six covered.
#
# String literals live in `.rodata` and survive LTO — which is why the boot
# check can assert on them at all. So: pull every literal of at least twelve
# characters out of `demos.rs`, confirm each one is really in the image that
# *has* the oracle (a marker absent from both proves nothing), and require all
# of them to be absent from the product image.
mapfile -t markers < <(
	grep -oE '"[^"]{12,}"' src/bootstrap/demos.rs |
		tr -d '"' |
		grep -v "[{\\\\]" |
		sort -u
)

[[ "${#markers[@]}" -ge 10 ]] || {
	echo "product-builds: FAIL — only ${#markers[@]} usable strings found in demos.rs" >&2
	echo "  The marker set is derived from the source; too few means the pattern" >&2
	echo "  stopped matching and this gate would pass on an empty check." >&2
	exit 1
}

leaked=0
checked=0
for marker in "${markers[@]}"; do
	# A marker that is not in the oracle image either is not evidence of
	# anything — skip it rather than count it.
	grep -qaF -- "${marker}" "${OUT}/kernel8.img" || continue
	checked=$((checked + 1))
	if grep -qaF -- "${marker}" "${OUT}/kernel8-product.img"; then
		echo "product-builds: FAIL — '${marker}' is in an image built without the oracle" >&2
		leaked=$((leaked + 1))
	fi
done

if [[ "${leaked}" -ne 0 ]]; then
	echo "  ${leaked} of ${checked} oracle strings reached the product image: a call" >&2
	echo "  outside its \`cfg(feature = \"oracle\")\`, or a module pulled in another way." >&2
	echo "  Rule 9: diagnostic scaffolding stays out of the production surface." >&2
	exit 1
fi

# Second net, kept because it costs nothing and catches a demo that leaks
# without printing: a symbol path the linker did keep.
if llvm-nm "${product_elf}" 2>/dev/null | grep -q 'bootstrap::demos'; then
	echo "product-builds: FAIL — a demo symbol survived into the product ELF" >&2
	llvm-nm "${product_elf}" | grep 'bootstrap::demos' | sed 's/^/    /' >&2
	exit 1
fi

unreachable="$(cargo build --target "${TARGET}" --release --no-default-features \
	--features board-rpi4 2>&1 | grep -c '^warning: .*never' || true)"

# Leave the default image in place for the targets that follow in `make check`.
cargo build --target "${TARGET}" --release >/dev/null
llvm-objcopy -O binary "${OUT}/harbor-kernel" "${OUT}/kernel8.img"

# M8 product gate: the product image must create the beacon agent and the
# console server. Format strings are split (`loader: {} loaded…` + name
# `"beacon"`), so assert the pieces that actually land in .rodata.
for marker in "console-server: up" "beacon" "loader: "; do
	grep -qaF -- "${marker}" "${OUT}/kernel8-product.img" || {
		echo "product-builds: FAIL — product image lacks '${marker}'" >&2
		echo "  M8 requires a non-empty product path (console server + beacon)." >&2
		exit 1
	}
done

# Product must be larger than the empty-manifest era (~54 KiB): beacon + server.
readonly PRODUCT_MIN_BYTES=56000
if [[ "${product_size}" -lt "${PRODUCT_MIN_BYTES}" ]]; then
	echo "product-builds: FAIL — product image is ${product_size} B (< ${PRODUCT_MIN_BYTES})" >&2
	echo "  Expected growth from the empty-manifest size after M8 beacon." >&2
	exit 1
fi

printf 'product-builds: clean (no demo symbols; image %s B without the oracle, %s B with, +%s B)\n' \
	"${product_size}" "${oracle_size}" "$((oracle_size - product_size))"
printf '  %s items unreachable without the oracle.\n' "${unreachable}"
printf '  Product carries console-server + beacon (M8); mute stays oracle-only.\n'
