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
grep -q 'rpi_minimal_agentic: hello' "${log}" ||
	fail "no console output: the kernel did not reach bootstrap::run"
grep -q 'MMU on' "${log}" ||
	fail "the kernel map did not activate"
grep -q 'fully reclaimed' "${log}" ||
	fail "the allocator did not return freed memory"
# Two tick reports mean the timer IRQ fired repeatedly *and* the WFI idle loop
# kept waking: a stalled idle loop prints the first and then goes quiet.
grep -q 'ticks=20' "${log}" ||
	fail "timer IRQ or WFI idle loop stalled"
if grep -q 'irq: unhandled' "${log}"; then
	fail "unhandled interrupts were dispatched"
fi
if grep -qi 'PANIC' "${log}"; then
	fail "the kernel panicked"
fi

echo "boot-check: clean ($(grep -c 'ticks=' "${log}") tick reports)"
