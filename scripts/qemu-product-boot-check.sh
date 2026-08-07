#!/usr/bin/env bash
# Boot the product image (no oracle) and assert the M8 product path runs.
#
# Complements `check-product-image.sh` (static) with a QEMU smoke: server up,
# beacon loaded and ran, denied-by-default is N/A here (mute is oracle-only).
set -euo pipefail

cd "$(dirname "$0")/.."

readonly TARGET=aarch64-unknown-none-softfloat
readonly OUT="target/${TARGET}/release"
readonly IMG="${OUT}/kernel8-product.img"
readonly QEMU="${QEMU:-qemu-system-aarch64}"
readonly SECONDS_LIMIT="${PRODUCT_BOOT_SECONDS:-8}"

if [[ ! -f "${IMG}" ]]; then
	echo "product-boot-check: building product image" >&2
	cargo build --target "${TARGET}" --release --no-default-features --features board-rpi4 >/dev/null
	llvm-objcopy -O binary "${OUT}/harbor-kernel" "${IMG}"
fi

readonly AGENTS="${AGENTS_BIN:-target/agents.bin}"
if [[ ! -f "${AGENTS}" ]]; then
	echo "product-boot-check: packing agent store" >&2
	python3 scripts/pack-agent-store.py -o "${AGENTS}"
fi

if ! command -v "${QEMU}" >/dev/null; then
	if [[ "${ALLOW_BOOT_SKIP:-}" == "1" ]]; then
		echo "product-boot-check: SKIPPED — ${QEMU} missing, ALLOW_BOOT_SKIP set" >&2
		exit 0
	fi
	echo "error: ${QEMU} not found — product boot check cannot run" >&2
	exit 1
fi

log="$(mktemp)"
trap 'rm -f "${log}"' EXIT

# ADR-0027: product composition from external store at 0x10000000.
timeout "${SECONDS_LIMIT}" "${QEMU}" \
	-machine raspi4b \
	-nographic \
	-serial mon:stdio \
	-d guest_errors \
	-kernel "${IMG}" \
	-device loader,file="${AGENTS}",addr=0x10000000 \
	>"${log}" 2>&1 || true

fail() {
	echo "product-boot-check: FAIL — $1" >&2
	echo "--- serial log ---" >&2
	cat "${log}" >&2 || true
	exit 1
}

grep -qa 'Harbor: hello' "${log}" || fail "product image did not boot"
grep -qa 'console-server: up' "${log}" || fail "console server did not spawn"
grep -qa 'loader: store n=1' "${log}" || fail "product did not load the external agent store"
grep -qa 'loader: beacon loaded' "${log}" || fail "beacon was not loaded"
grep -qa 'loader: beacon ran sends=2 refusals=0' "${log}" || fail "beacon did not run successfully"
grep -qa 'H!loader: beacon ran' "${log}" || fail "beacon bytes did not reach the wire before the report"
# Product must not carry oracle demos.
if grep -qa 'sched: spawned task-a' "${log}"; then
	fail "product image ran oracle demos"
fi

echo "product-boot-check: clean"
