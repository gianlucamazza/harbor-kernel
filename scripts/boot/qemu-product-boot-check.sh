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

# QEMU invocation, image build and the skip policy are shared with
# `oracle-census.sh`, which needs the same boot to read ADR-0098's slot meter.
# shellcheck source=scripts/lib/product-boot.sh
source "$(dirname "$0")/../lib/product-boot.sh"

log="$(mktemp)"
trap 'rm -f "${log}"' EXIT

product_status=0
product_boot_capture "${log}" product-boot-check || product_status=$?
case "${product_status}" in
	0) ;;
	2) exit 0 ;; # Explicit local opt-out: no emulator installed.
	3) exit 3 ;; # Environment did not establish a credible QEMU verdict.
	*) exit "${product_status}" ;;
esac

fail() {
	echo "product-boot-check: FAIL — $1" >&2
	echo "--- serial log ---" >&2
	cat "${log}" >&2 || true
	exit 1
}

on_timer_missed() {
	fail "timer deadlines expired unserviced on the product path"
}

# shellcheck source=scripts/lib/product-oracle.sh
source "$(dirname "$0")/../lib/product-oracle.sh"

assert_product_boot

n_assert=51
echo "product-boot-check: clean (${n_assert}+ layered composition-minimum assertions)"
