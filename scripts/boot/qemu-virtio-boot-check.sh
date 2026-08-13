#!/usr/bin/env bash
# Verify the AArch64 QEMU virt modern virtio-mmio transport probe.
#
# This gate is narrower than the product oracle: it proves the BSP map, DTB
# reservation, slot discovery, modern negotiation, and reset lifecycle. It
# does not claim queues, packet I/O, or EL0 capabilities.
set -euo pipefail

IMG="${1:?usage: $0 <qemu-virt-kernel.img> [ceiling-seconds]}"
SECONDS_TO_RUN="${2:-15}"
QEMU="${QEMU:-qemu-system-aarch64}"

[[ -f "${IMG}" ]] || { echo "qemu-virtio-check: image not found: ${IMG}" >&2; exit 1; }
command -v "${QEMU}" >/dev/null || {
    echo "qemu-virtio-check: ${QEMU} is required; refusing to skip" >&2
    exit 1
}

modern_log="$(mktemp)"
absent_log="$(mktemp)"
trap 'rm -f "${modern_log}" "${absent_log}"' EXIT

run_boot() {
    local output="$1"
    shift
    set +e
    timeout "${SECONDS_TO_RUN}s" "${QEMU}" \
        -machine virt,gic-version=2 -cpu cortex-a72 -m 128M -smp 1 \
        -kernel "${IMG}" -global virtio-mmio.force-legacy=false \
        -serial mon:stdio -display none -no-reboot "$@" \
        </dev/null >"${output}" 2>&1
    local result=$?
    set -e
    # timeout is expected; the assertions below are the verdict.
    if [[ ${result} -ne 0 && ${result} -ne 124 ]]; then
        echo "qemu-virtio-check: QEMU failed with status ${result}" >&2
        cat "${output}" >&2
        exit 1
    fi
}

run_boot "${modern_log}" \
    -netdev user,id=n0 \
    -device virtio-net-device,netdev=n0

grep -aq 'DTB mapped:' "${modern_log}" || {
    echo "qemu-virtio-check: DTB was not mapped" >&2
    exit 1
}
if grep -aq 'DTB map FAILED\|BOOT REFUSED' "${modern_log}"; then
    echo "qemu-virtio-check: memory-map refusal or DTB collision" >&2
    exit 1
fi
grep -aqE 'virtio-net: modern probe ok base=0x[0-9a-f]+ vendor=0x[0-9a-f]+ features=0x100000000 reset' "${modern_log}" || {
    echo "qemu-virtio-check: modern virtio-net probe did not complete" >&2
    grep -aE 'virtio-net:|discover:|boot:' "${modern_log}" >&2 || true
    exit 1
}
grep -aqE 'boot: mmu=[0-9]+ ms discover=[0-9]+ ms ready=[0-9]+ ms' "${modern_log}" || {
    echo "qemu-virtio-check: kernel did not reach steady state" >&2
    exit 1
}

run_boot "${absent_log}"
grep -aq 'virtio-net: unavailable (' "${absent_log}" || {
    echo "qemu-virtio-check: absent-device refusal was not observed" >&2
    exit 1
}
if grep -aq 'virtio-net: modern probe ok' "${absent_log}"; then
    echo "qemu-virtio-check: device reported ready while absent" >&2
    exit 1
fi

echo "qemu-virtio-check: modern transport and absent-device refusal clean"
