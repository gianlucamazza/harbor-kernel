#!/usr/bin/env bash
# Boot the product image under QEMU and capture its serial output.
#
# Sourced by two gates that need the same run for different questions:
#
#   * `qemu-product-boot-check.sh` — is the shipped composition healthy?
#   * `oracle-census.sh`           — how many slots did it actually occupy?
#
# It lives here rather than in either of them because the second gate was
# written against a number someone typed instead of a run (ADR-0098), and the
# cure for that is not a second copy of the QEMU invocation that can drift from
# the first.
#
# Callers must define `fail`, or accept the default below.

readonly PRODUCT_TARGET=aarch64-unknown-none-softfloat
readonly PRODUCT_OUT="target/${PRODUCT_TARGET}/release"
readonly PRODUCT_IMG="${PRODUCT_OUT}/kernel8-product.img"
readonly PRODUCT_QEMU="${QEMU:-qemu-system-aarch64}"
# Ceiling, not a fixed duration: product reaches composition + first tick
# report well under this on a quiet host (ADR-0087 shape for short boots).
readonly PRODUCT_SECONDS_LIMIT="${PRODUCT_BOOT_SECONDS:-8}"

# Boot the product image, writing serial output to $1.
#
# Exit status: 0 booted, 2 skipped (no emulator on a workstation). A missing
# emulator in CI is a failure, not a skip — a green gate that never ran is
# what ADR-0096 removed.
product_boot_capture() {
	local log="$1"
	local who="${2:-product-boot}"

	if [[ ! -f "${PRODUCT_IMG}" ]]; then
		echo "${who}: building product image" >&2
		./scripts/boot/product-image.sh
	fi

	if ! command -v "${PRODUCT_QEMU}" >/dev/null; then
		if [[ "${CI:-}" == "true" ]]; then
			echo "${who}: FAIL — ${PRODUCT_QEMU} missing, and a skip is refused in CI" >&2
			echo "  ALLOW_BOOT_SKIP is for a workstation without the emulator." >&2
			echo "  In CI it would report a green gate that never ran (ADR-0096)." >&2
			exit 1
		fi
		if [[ "${ALLOW_BOOT_SKIP:-}" == "1" ]]; then
			echo "${who}: SKIPPED — ${PRODUCT_QEMU} missing, ALLOW_BOOT_SKIP set" >&2
			return 2
		fi
		echo "error: ${PRODUCT_QEMU} not found — ${who} cannot run" >&2
		exit 1
	fi

	# Store is already in the image (ADR-0029 inject). No -device loader.
	timeout "${PRODUCT_SECONDS_LIMIT}" "${PRODUCT_QEMU}" \
		-machine raspi4b \
		-nographic \
		-serial mon:stdio \
		-d guest_errors \
		-kernel "${PRODUCT_IMG}" \
		>"${log}" 2>&1 || true
}
