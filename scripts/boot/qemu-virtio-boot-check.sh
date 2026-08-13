#!/usr/bin/env bash
# Verify the AArch64 QEMU virt modern virtio-mmio transport and packet path.
#
# This gate is narrower than the product oracle: it proves the BSP map, DTB
# reservation, slot discovery, modern negotiation, split-ring descriptor
# submission/completion, deterministic peer RX, payload delivery, directional
# EL0 capabilities, and reset refusal lifecycle.
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
peer_port="$(shuf -i 20000-45000 -n 1)"
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

set +e
timeout "${SECONDS_TO_RUN}s" "${QEMU}" \
    -machine virt,gic-version=2 -cpu cortex-a72 -m 128M -smp 1 \
    -kernel "${IMG}" -global virtio-mmio.force-legacy=false \
    -serial mon:stdio -display none -no-reboot \
    -netdev socket,id=n0,listen=127.0.0.1:"${peer_port}" \
    -device virtio-net-device,netdev=n0 \
    </dev/null >"${modern_log}" 2>&1 &
qemu_pid=$!
python3 "${BASH_SOURCE[0]%/*}/qemu-virtio-peer.py" \
    --port "${peer_port}" --deadline "${SECONDS_TO_RUN}" \
    --delay 5
peer_result=$?
wait "${qemu_pid}"
qemu_result=$?
set -e
if [[ ${peer_result} -ne 0 ]]; then
    echo "qemu-virtio-check: deterministic peer could not inject RX" >&2
    cat "${modern_log}" >&2
    exit 1
fi
if [[ ${qemu_result} -ne 0 && ${qemu_result} -ne 124 ]]; then
    echo "qemu-virtio-check: QEMU failed with status ${qemu_result}" >&2
    cat "${modern_log}" >&2
    exit 1
fi

grep -aq 'DTB mapped:' "${modern_log}" || {
    echo "qemu-virtio-check: DTB was not mapped" >&2
    exit 1
}
if grep -aq 'DTB map FAILED\|BOOT REFUSED' "${modern_log}"; then
    echo "qemu-virtio-check: memory-map refusal or DTB collision" >&2
    exit 1
fi
grep -aqE 'virtio-net: modern probe ok base=0x[0-9a-f]+ vendor=0x[0-9a-f]+ features=0x100000000 queues=2 size=8 ready tx-descriptor=submitted' "${modern_log}" || {
    echo "qemu-virtio-check: modern virtio-net probe did not complete" >&2
    grep -aE 'virtio-net:|discover:|boot:' "${modern_log}" >&2 || true
    exit 1
}
grep -aqE 'virtio-net: tx descriptor complete used_len=[0-9]+' "${modern_log}" || {
    echo "qemu-virtio-check: deterministic TX descriptor was not completed" >&2
    grep -aE 'virtio-net:|discover:|boot:' "${modern_log}" >&2 || true
    exit 1
}
grep -aqE 'net: tx accepted slot=1 len=16' "${modern_log}" || {
    echo "qemu-virtio-check: EL1 network service did not accept the agent TX token" >&2
    grep -aE 'net:|edge-gateway|virtio-net:' "${modern_log}" >&2 || true
    exit 1
}
grep -aqE 'net: tx complete slot=1 len=16' "${modern_log}" || {
    echo "qemu-virtio-check: EL1 network service did not return TX completion" >&2
    grep -aE 'net:|edge-gateway|virtio-net:' "${modern_log}" >&2 || true
    exit 1
}
grep -aqE 'virtio-net: rx available len=[0-9]+' "${modern_log}" || {
    echo "qemu-virtio-check: deterministic peer RX was not consumed" >&2
    grep -aE 'net:|edge-gateway|virtio-net:' "${modern_log}" >&2 || true
    exit 1
}
grep -aqE 'net: rx available slot=[0-9]+ len=[0-9]+' "${modern_log}" || {
    echo "qemu-virtio-check: RX token did not cross the service endpoint" >&2
    grep -aE 'net:|edge-gateway|virtio-net:' "${modern_log}" >&2 || true
    exit 1
}
grep -aqE 'net: rx returned slot=[0-9]+ len=[0-9]+' "${modern_log}" || {
    echo "qemu-virtio-check: EL0 did not return the RX token" >&2
    grep -aE 'net:|edge-gateway|virtio-net:' "${modern_log}" >&2 || true
    exit 1
}
grep -aq 'loader: edge-gateway ran sends=2 refusals=0' "${modern_log}" || {
    echo "qemu-virtio-check: edge-gateway did not complete through directional caps" >&2
    grep -aE 'net:|edge-gateway|virtio-net:' "${modern_log}" >&2 || true
    exit 1
}
grep -aq 'virtio-net: recovery complete' "${modern_log}" || {
    echo "qemu-virtio-check: network reset/recovery was not exercised" >&2
    grep -aE 'net:|edge-gateway|virtio-net:' "${modern_log}" >&2 || true
    exit 1
}
grep -aqE 'boot: mmu=[0-9]+ ms discover=[0-9]+ ms ready=[0-9]+ ms' "${modern_log}" || {
    echo "qemu-virtio-check: kernel did not reach steady state" >&2
    exit 1
}
grep -aq 'IRQs enabled (timer + UART RX + virtio-mmio slots)' "${modern_log}" || {
    echo "qemu-virtio-check: virtio-mmio IRQ bindings were not enabled" >&2
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
