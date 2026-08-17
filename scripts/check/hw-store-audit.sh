#!/usr/bin/env bash
# Audit the composition a board actually booted, against the artifact it booted.
#
# ## The gap this closes
#
# P6 is "host tools for store composition and audit". The compose half runs on
# every deploy: `pack-store.py` writes the store and `inject-store.py` puts it
# in the image. The audit half only ever ran on the blob *about to be* shipped
# — `product-image.sh` round-trips `agents.bin` through the reader before
# injection, which proves the packer and the reader agree with each other and
# nothing about what left the building.
#
# This reads the store back out of a **shipped image** — the `kernel8.img` on
# the card, resolved through the same window arithmetic the injector used — and
# compares it line by line with what the board said it loaded. Two independent
# accounts of one composition: a host tool reading bytes, and a kernel
# reporting what it did with them.
#
# ## What it asserts
#
# - the reader can find a store in that image at all (`store_window.py` refuses
#   an ELF and an image from different builds, rather than reporting a
#   composition that was never shipped — which is why the default ELF is
#   `kernel8-product.elf`, saved by `product-image.sh` at the moment it made
#   the image, and not the shared `harbor-kernel` path every build overwrites);
# - the boot's own `build: … src=` matches the tree the image was built from,
#   because two artifacts that never met can agree by coincidence;
# - the count the reader sees equals the `loader: store n=N` the board printed;
# - every agent the store names appears in the transcript as loaded, **with the
#   `home_cpu` the store gave it**. That last field is the one worth comparing:
#   it is a composition decision taken on the host, and the only place it can be
#   observed is the board.
#
# ## What it does not assert
#
# That the agent did anything useful. `loader: … ran sends=…` is the product
# oracle's business (`scripts/lib/product-oracle.sh`), and `make hw-check` is
# what judges a transcript as a whole. This is narrower on purpose: it is about
# the tooling agreeing with the board, not about the board being correct.
set -euo pipefail

cd "$(dirname "$0")/../.."

TRANSCRIPT="${1:-}"
IMAGE="${2:-target/aarch64-unknown-none-softfloat/release/kernel8-product.img}"
ELF="${3:-target/aarch64-unknown-none-softfloat/release/kernel8-product.elf}"

if [[ -z "${TRANSCRIPT}" ]]; then
	echo "usage: $0 <transcript.log> [kernel8.img] [harbor-kernel.elf]" >&2
	echo "  The image should be the one on the card, and the ELF the build it" >&2
	echo "  came from — a mismatch is refused, not guessed at." >&2
	exit 2
fi

for f in "${TRANSCRIPT}" "${IMAGE}" "${ELF}"; do
	[[ -f "${f}" ]] || {
		echo "hw-store-audit: FAIL — missing ${f}" >&2
		exit 1
	}
done

violations=0
note() {
	echo "hw-store-audit: $1" >&2
	violations=$((violations + 1))
}

# The last complete boot, by the same marker `hw-transcript-check.sh` uses: a
# capture holds every power cycle of a session, and only the last one is about
# the image on the card now.
last_hello="$(grep -an 'Harbor: hello' "${TRANSCRIPT}" | tail -1 | cut -d: -f1)"
if [[ -z "${last_hello}" ]]; then
	echo "hw-store-audit: FAIL — no 'Harbor: hello' in ${TRANSCRIPT}" >&2
	echo "  A boot nobody can isolate is a reading, not evidence. Capture again." >&2
	exit 1
fi
boot="$(mktemp)"
trap 'rm -f "${boot}"' EXIT
tail -n "+${last_hello}" "${TRANSCRIPT}" >"${boot}"

# The image on disk is whatever was built last; the boot is whatever ran then.
# Comparing them without checking they are the same build is how an audit
# reports agreement between two things that never met. The product prints its
# own provenance, so ask it.
boot_src="$(sed -nE 's/.*build: .*src=([0-9a-f]+).*/\1/p' "${boot}" | head -1)"
tree_src="$(git describe --always --abbrev=8 2>/dev/null || echo unknown)"
if [[ -z "${boot_src}" ]]; then
	echo "hw-store-audit: FAIL — the boot printed no 'build: … src=' line" >&2
	echo "  Without provenance the image cannot be tied to the boot." >&2
	exit 1
fi
if [[ "${boot_src}" != "${tree_src}"* && "${tree_src}" != "${boot_src}"* ]]; then
	echo "hw-store-audit: FAIL — the boot is from src=${boot_src}, the tree is ${tree_src}" >&2
	echo "  The image beside this tree is not the image that boot ran. Rebuild at" >&2
	echo "  that commit, or take the audit from a boot of what is built now." >&2
	exit 1
fi

reader_out="$(python3 scripts/agent/inspect-store.py --elf "${ELF}" --image "${IMAGE}")" || {
	echo "hw-store-audit: FAIL — the audit reader could not read ${IMAGE}" >&2
	exit 1
}

store_count="$(sed -nE 's/^magic=HARB version=[0-9]+ count=([0-9]+).*/\1/p' <<<"${reader_out}")"
[[ -n "${store_count}" ]] || {
	echo "hw-store-audit: FAIL — the reader printed no count" >&2
	exit 1
}

board_count="$(sed -nE 's/.*loader: store n=([0-9]+).*/\1/p' "${boot}" | head -1)"
if [[ -z "${board_count}" ]]; then
	note "the boot printed no 'loader: store n=' line"
elif [[ "${board_count}" != "${store_count}" ]]; then
	note "the image holds ${store_count} agents, the board loaded ${board_count}"
fi

# One line per agent, compared on the field the host chose and the board obeyed.
while IFS= read -r line; do
	name="$(sed -nE "s/.*name='([^']*)'.*/\1/p" <<<"${line}")"
	home="$(sed -nE 's/.*home_cpu=([0-9]+).*/\1/p' <<<"${line}")"
	[[ -n "${name}" ]] || continue
	loaded="$(grep -a "loader: ${name} loaded " "${boot}" | head -1 || true)"
	if [[ -z "${loaded}" ]]; then
		# A refusal is a legitimate outcome and names its reason; silence is not.
		refused="$(grep -a "loader: ${name} refused" "${boot}" | head -1 || true)"
		if [[ -n "${refused}" ]]; then
			echo "  ${name}: refused on the board — ${refused##* — }"
			continue
		fi
		note "'${name}' is in the shipped store and the board never mentions it"
		continue
	fi
	board_home="$(sed -nE 's/.*home=([0-9]+).*/\1/p' <<<"${loaded}")"
	if [[ "${board_home}" != "${home}" ]]; then
		note "'${name}': the store says home_cpu=${home}, the board says home=${board_home}"
	else
		echo "  ${name}: home_cpu=${home} on both sides"
	fi
done <<<"$(grep -E '^\s+\[[0-9]+\] name=' <<<"${reader_out}")"

if [[ "${violations}" -ne 0 ]]; then
	echo "hw-store-audit: ${violations} disagreement(s) between the shipped store and the board" >&2
	exit 1
fi

echo "hw-store-audit: clean (${store_count} agents; the image on the card and the boot agree)"
