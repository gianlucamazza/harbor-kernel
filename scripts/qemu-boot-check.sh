#!/usr/bin/env bash
# Boot the kernel under QEMU and assert the run reached a healthy steady state.
#
# This is the single definition of "the boot is sound", used by `make check`
# and by CI. It lived only in the workflow before, which meant the local gate
# was a subset of the remote one — a local green did not predict CI — and the
# assertions were somewhere nobody runs while working, free to drift.
set -euo pipefail

IMG="${1:?usage: $0 <kernel8.img> [seconds]}"
SECONDS_TO_RUN="${2:-15}"
QEMU="${QEMU:-qemu-system-aarch64}"
QEMU_MACHINE="${QEMU_MACHINE:-raspi4b}"

if [[ ! -f "${IMG}" ]]; then
	echo "error: kernel image not found: ${IMG}" >&2
	exit 1
fi

# Absence of the emulator must be loud, and by default fatal. A check that
# passes when it cannot run reports coverage it does not have — and "skipped"
# scrolls past in a log that ends in a green tick. A developer without QEMU
# opts out explicitly; CI never does, so a missing install fails the job
# instead of quietly removing the boot from the gate.
if ! command -v "${QEMU}" >/dev/null; then
	if [[ -n "${ALLOW_BOOT_SKIP:-}" ]]; then
		echo "boot-check: SKIPPED — ${QEMU} missing, ALLOW_BOOT_SKIP set" >&2
		exit 0
	fi
	echo "error: ${QEMU} not found — the boot check cannot run" >&2
	echo "  install it (pacman -S qemu-system-aarch64), or set ALLOW_BOOT_SKIP=1" >&2
	exit 1
fi

log="$(mktemp)"
trap 'rm -f "${log}"' EXIT

# `timeout` kills a healthy run, so its exit status says nothing; the log is
# the oracle. `|| true` keeps `set -e` from ending the script before we look.
timeout "${SECONDS_TO_RUN}" "${QEMU}" \
	-M "${QEMU_MACHINE}" -kernel "${IMG}" \
	-serial mon:stdio -display none </dev/null >"${log}" 2>&1 || true

fail() {
	echo "boot-check: FAIL — $1" >&2
	echo "--- serial log ---" >&2
	cat "${log}" >&2
	exit 1
}

# Each assertion covers a distinct subsystem, so a failure localises itself.
grep -q 'Harbor: hello' "${log}" ||
	fail "no console output: the kernel did not reach bootstrap::run"
grep -q 'MMU on' "${log}" ||
	fail "the kernel map did not activate"
grep -q 'fully reclaimed' "${log}" ||
	fail "the allocator did not return freed memory"
# `mmu::unmap` (and the L2→L3 split when the heap is a block) then remap.
# Failure prints `unmap: FAILED` / `remap FAILED`; silence would mean a hang
# on the first post-unmap access instead.
grep -q 'unmap: remapped and freed' "${log}" ||
	fail "unmap smoke did not complete (split/TLBI/remap)"
if grep -q 'unmap: FAILED' "${log}"; then
	fail "mmu::unmap refused a mapped heap page"
fi
# M3 cooperative demo: two tasks yield; console shows interleaved lines.
grep -q 'task-a 0' "${log}" || fail "task-a did not run"
grep -q 'task-b 0' "${log}" || fail "task-b did not run"
grep -q 'task-a 3' "${log}" || fail "task-a did not finish its yields"
grep -q 'task-b 3' "${log}" || fail "task-b did not finish its yields"
if grep -q 'spawn task-a FAILED' "${log}" || grep -q 'spawn task-b FAILED' "${log}"; then
	fail "cooperative task spawn failed"
fi
# Two tick reports mean the timer IRQ fired repeatedly *and* the WFI idle loop
# kept waking: a stalled idle loop prints the first and then goes quiet.
grep -q 'ticks=20' "${log}" ||
	fail "timer IRQ or WFI idle loop stalled"
if grep -q 'irq: unhandled' "${log}"; then
	fail "unhandled interrupts were dispatched"
fi
# The allocator refuses frees it cannot justify — a double free, or a pointer it
# never handed out. Refusing keeps the heap intact, so nothing else here would
# notice; the count is the only evidence that a caller is wrong about what it owns.
if grep -q 'heap: REFUSED' "${log}"; then
	fail "the allocator refused an invalid free"
fi
# Received bytes lost for want of ring space. Nothing types during this run, so
# any count here is the RX path losing bytes it was handed.
if grep -q 'console: DROPPED' "${log}"; then
	fail "the RX handler dropped received bytes"
fi
# A missed deadline means the timer handler did not run in time. Harmless at
# 10 Hz with nothing else running, which is exactly why it must be loud here:
# this is the quietest possible conditions.
if grep -q 'timer: MISSED' "${log}"; then
	fail "timer deadlines expired unserviced"
fi
if grep -qi 'PANIC' "${log}"; then
	fail "the kernel panicked"
fi

echo "boot-check: clean ($(grep -c 'ticks=' "${log}") tick reports)"
