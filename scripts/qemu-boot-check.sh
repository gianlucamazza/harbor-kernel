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
# RNG200 is always probed after the MMU. QEMU has no backend: expect soft
# NotPresent. Silicon logs `ok word=…`. Either shape is a successful probe path;
# silence would mean the probe panicked or never ran.
grep -qE 'rng200: (ok |unavailable \()' "${log}" ||
	fail "RNG200 probe line missing (expected ok or unavailable)"
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
# The break-before-make block split, exercised deliberately rather than left to
# an alignment accident. `__heap_start` is below the first 2 MiB boundary, so
# every task stack lands on pages that were never blocks; without this smoke the
# split path would first run in production, the day the heap fills past 2 MiB.
# Asserting `split 1` — not merely that the line appeared — is what proves a
# block was actually rebuilt as a table.
grep -qE 'split: page at 0x[0-9a-f]+ split 1, remapped' "${log}" ||
	fail "block split path did not run: $(grep '^split:' "${log}" || echo '(no split line at all)')"
if grep -qE 'split: (unmap|remap) FAILED|split: SKIPPED' "${log}"; then
	fail "block split smoke did not complete"
fi
# A task stack leaked because its guard could not be remapped. The heap stays
# consistent — that is why it leaks — so nothing else here would notice.
if grep -q 'sched: ABANDONED' "${log}"; then
	fail "a task stack was abandoned (guard remap refused)"
fi
# M3 cooperative demo (ADR-0006): the console must show the two tasks *alternating*.
#
# The order is deterministic, not a race: both tasks are on the runqueue before
# either runs, and each yields exactly once per line, so round-robin fixes the
# sequence. Asserting the whole sequence rather than four independent greps is
# the difference between proving a switch happened and proving both tasks ran —
# `task-a 0..3` followed by `task-b 0..3` satisfies the second and means the
# scheduler never switched until the first task exited.
# `|| true`: no matching lines is the most interesting failure here (the tasks
# never ran at all), and under `set -e` a failing grep inside a command
# substitution kills the script before `fail` can report anything.
observed="$(grep -oE '^task-[ab] [0-9]+' "${log}" | tr '\n' ' ' || true)"
expected="task-a 0 task-b 0 task-a 1 task-b 1 task-a 2 task-b 2 task-a 3 task-b 3 "
[[ "${observed}" == "${expected}" ]] ||
	fail "task output not interleaved: ${observed}"
if grep -q 'spawn task-a FAILED' "${log}" || grep -q 'spawn task-b FAILED' "${log}"; then
	fail "cooperative task spawn failed"
fi
# ADR-0012 S1: named frame pool initialised (capacity matches BSP constant).
grep -qE 'frames: [0-9]+ free / [0-9]+' "${log}" ||
	fail "frame pool boot line missing"
# M5 S2/S3: prepare AS + EL0 probes; destroy must not leak frames.
grep -q 'aspace: prepare ok' "${log}" ||
	fail "address-space prepare for EL0 failed"
grep -q 'el0: SVC ok' "${log}" ||
	fail "EL0 SVC probe failed"
grep -q 'el0: FAULT ok' "${log}" ||
	fail "EL0 kernel-store fault probe failed"
grep -q 'aspace: create/destroy ok' "${log}" ||
	fail "address-space create/destroy smoke failed"
if grep -q 'aspace: LEAK' "${log}"; then
	fail "address-space leaked frames on destroy"
fi
grep -q 'aspace: dual create/destroy ok' "${log}" ||
	fail "dual address-space create/destroy smoke failed"
if grep -q 'aspace: dual LEAK' "${log}"; then
	fail "dual address-space leaked frames"
fi
# M5-P1/P2: scheduled EL0 task + SVC dispatch.
grep -q 'el0-task: svc ping' "${log}" ||
	fail "scheduled el0-task did not complete svc ping"
grep -q 'el0-task: svc refuse imm=0x99' "${log}" ||
	fail "scheduled el0-task did not refuse unknown svc imm"
grep -q 'el0-task: resume pings=2' "${log}" ||
	fail "EL0 SVC resume session did not complete two pings + exit"
grep -q 'el0-task: putc bytes=2' "${log}" ||
	fail "EL0 SYS_PUTC session did not emit two bytes"
grep -qE 'el0-task: irq resume irqs=[1-9]' "${log}" ||
	fail "EL0 IRQ save/resume path did not handle at least one IRQ"
grep -q 'el0-task: ok' "${log}" ||
	fail "scheduled el0-task leaked frames or failed teardown"
# M6 v1: PL011 page-only agent + kill (ADR-0013).
grep -q 'pl011-agent: FR read + svc ok' "${log}" ||
	fail "pl011 EL0 agent did not read FR and svc"
# Empty is the honest QEMU path (no typed input). `rx poll data` is HW/input.
grep -qE 'pl011-agent: rx poll (empty|data)' "${log}" ||
	fail "pl011 EL0 RX poll session failed"
grep -q 'pl011-agent: killed ok' "${log}" ||
	fail "pl011 agent AS destroy / kill path failed"
# Multi-agent shell: two TCBs with AS live together, each EL0 once.
grep -q 'agents: concurrent ok' "${log}" ||
	fail "concurrent multi-agent shell smoke failed"
if grep -q 'agents: concurrent LEAK' "${log}"; then
	fail "concurrent multi-agent shell leaked frames"
fi
# M4 IPC (ADR-0008 shape + mailbox): message delivered; forge refuse counted.
grep -q 'ipc: sent tag=1 a=42' "${log}" ||
	fail "ipc sender did not deliver"
grep -q 'ipc: got tag=1 a=42' "${log}" ||
	fail "ipc receiver did not get the message"
grep -qE 'ipc: refuse count=[1-9]' "${log}" ||
	fail "ipc forge was not refused (capability hold check)"
if grep -q 'ipc: FORGE OK' "${log}"; then
	fail "forged capability send succeeded"
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
