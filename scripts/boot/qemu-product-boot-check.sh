#!/usr/bin/env bash
# Boot the product image (no oracle) and assert the shipped path is healthy.
#
# Complements the static half of `product-image.sh` (marker + symbol check)
# with a QEMU run of the image that actually goes on the SD card.
#
# ## What this gate is
#
# The **composition minimum** for the product configuration (excellence
# F-R5-2 / multi-role 2026-08-10 R5): not a second copy of the oracle
# (`scripts/lib/boot-oracle.sh` ~100 demo lines), but every product-path
# claim that the shipped image already prints and that a silent regression
# would hide. Oracle demos stay behind `feature = "oracle"`; this gate
# refuses their strings rather than re-running them.
#
# Layers (each failure localises):
#   1. Boot identity — hello, MMU, reset, CPU, discovery, RNG probe
#   2. Memory self-checks — frames, heap reclaim, unmap/split smoke
#   3. IRQ / timer / dual-current SMP (product path starts core 1)
#   4. Composition — console server + multi-agent store + wire bytes
#   5. Invariant beacon + anomaly negatives
#   6. Oracle-free surface — demo strings must not appear
set -euo pipefail

cd "$(dirname "$0")/../.."

readonly TARGET=aarch64-unknown-none-softfloat
readonly OUT="target/${TARGET}/release"
readonly IMG="${OUT}/kernel8-product.img"
readonly QEMU="${QEMU:-qemu-system-aarch64}"
# Ceiling, not a fixed duration: product reaches composition + first tick
# report well under this on a quiet host (ADR-0087 shape for short boots).
readonly SECONDS_LIMIT="${PRODUCT_BOOT_SECONDS:-8}"

if [[ ! -f "${IMG}" ]]; then
	echo "product-boot-check: building product image" >&2
	./scripts/boot/product-image.sh
fi

if ! command -v "${QEMU}" >/dev/null; then
	if [[ "${CI:-}" == "true" ]]; then
		echo "product-boot-check: FAIL — ${QEMU} missing, and a skip is refused in CI" >&2
		echo "  ALLOW_BOOT_SKIP is for a workstation without the emulator." >&2
		echo "  In CI it would report a green gate that never ran (ADR-0096)." >&2
		exit 1
	fi
	if [[ "${ALLOW_BOOT_SKIP:-}" == "1" ]]; then
		echo "product-boot-check: SKIPPED — ${QEMU} missing, ALLOW_BOOT_SKIP set" >&2
		exit 0
	fi
	echo "error: ${QEMU} not found — product boot check cannot run" >&2
	exit 1
fi

log="$(mktemp)"
trap 'rm -f "${log}"' EXIT

# Store is already in the image (ADR-0029 inject). No -device loader.
timeout "${SECONDS_LIMIT}" "${QEMU}" \
	-machine raspi4b \
	-nographic \
	-serial mon:stdio \
	-d guest_errors \
	-kernel "${IMG}" \
	>"${log}" 2>&1 || true

fail() {
	echo "product-boot-check: FAIL — $1" >&2
	echo "--- serial log ---" >&2
	cat "${log}" >&2 || true
	exit 1
}

# shellcheck source=scripts/lib/product-oracle.sh
source "$(dirname "$0")/../lib/product-oracle.sh"

assert_product_boot

n_assert=35
echo "product-boot-check: clean (${n_assert}+ layered composition-minimum assertions)"
