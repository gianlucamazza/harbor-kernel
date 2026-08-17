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

# ADR-0087: `timeout`'s exit code cannot tell a healthy ceiling from a starved
# guest, and this gate had no guard at all — it could go red on a busy laptop
# for a reason with nothing to do with virtio. Shared with the product gates so
# there is one measurement, not two.
# shellcheck source=scripts/lib/cpu-budget.sh
source "${BASH_SOURCE[0]%/*}/../lib/cpu-budget.sh"

modern_log="$(mktemp)"
absent_log="$(mktemp)"
peer_port="$(shuf -i 20000-45000 -n 1)"
trap 'rm -f "${modern_log}" "${absent_log}"' EXIT

run_boot() {
    local output="$1"
    shift
    set +e
    # `-nic none` because this runner is used for the *absent-device* boot, and
    # without it QEMU helpfully adds its own default NIC — so the gate was
    # asserting "no device" while a device was present and merely of a kind the
    # mmio probe cannot see. It also drags in `efi-virtio.rom`, which the pinned
    # Debian QEMU container does not ship: CI's first run of this gate died on
    # `failed to find romfile "efi-virtio.rom"`. Absence has to be asked for.
    timeout "${SECONDS_TO_RUN}s" "${QEMU}" \
        -machine virt,gic-version=2 -cpu cortex-a72 -m 128M -smp 1 \
        -kernel "${IMG}" -global virtio-mmio.force-legacy=false \
        -nic none \
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
cpu_budget_start
# QEMU directly, not behind `timeout`: `cpu_budget_watch` samples the pid it is
# given, and given a `timeout` wrapper it samples a process that accrues no CPU
# and silently falls back to a host-wide number. That number *rises* on a busy
# host, so the guard would have read a saturated machine as well fed — a guard
# reporting the opposite of what it is for. `cpu_budget_watch`'s own deadline
# is the ceiling instead, exactly as the product gates do it.
"${QEMU}" \
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
cpu_budget_watch "${qemu_pid}" "${SECONDS_TO_RUN}"
kill -TERM "${qemu_pid}" 2>/dev/null || true
wait "${qemu_pid}" 2>/dev/null
set -e
# Before the assertions, not after: a starved run reaches none of them, and
# reporting that as a virtio failure is reporting the host as if it were code.
cpu_budget_verdict "qemu-virtio-check" || exit $?
# 3 means the peer never reached QEMU's socket: the experiment did not run, so
# there is nothing to judge (ADR-0087's distinction, applied to the harness
# rather than to the CPU). 1 means it connected and the write failed, which is
# a real red. Seen the day this gate entered CI: the guest booted fine —
# modern probe ok, service up, TX accepted and completed — and only the peer
# could not connect. Calling that a virtio failure would report the runner as
# if it were the driver.
if [[ ${peer_result} -eq 3 ]]; then
    echo "qemu-virtio-check: INDETERMINATE — the deterministic peer never reached QEMU" >&2
    echo "  The RX half was not exercised, so it was not judged. The guest's own" >&2
    echo "  output is below; the transport assertions it does cover are not run." >&2
    tail -40 "${modern_log}" >&2
    exit 3
fi
if [[ ${peer_result} -ne 0 ]]; then
    echo "qemu-virtio-check: deterministic peer connected and could not inject RX" >&2
    cat "${modern_log}" >&2
    exit 1
fi
# No status check on the modern boot: this gate now ends it with SIGTERM at the
# ceiling, so its exit status says only that we stopped it. The assertions
# below are the verdict, and the CPU budget above is what says they were
# reachable at all.

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
