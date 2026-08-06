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

# CPU time this shell's children have consumed, in USER_HZ. Read from
# `/proc/self/stat` (cutime + cstime) rather than through `/usr/bin/time`, so
# the measurement adds no dependency to a script `make check` always runs.
#
# The comm field can contain spaces, so everything up to the last `)` is
# dropped before counting: the remaining fields start at `state`, which puts
# cutime and cstime at offsets 13 and 14.
#
# The result goes into a global rather than being echoed for a caller to
# capture. `$(…)` runs in a subshell, and a subshell's `/proc/self/stat` is its
# own — a fresh process that has reaped no children, so cutime and cstime are
# always zero. The first version of this did exactly that and measured 0.00
# cores on an idle machine, which would have called every real timer failure
# "indeterminate" and quietly retired the assertion.
read_child_cpu_hz() {
	local stat rest
	read -r stat </proc/self/stat
	rest="${stat##*) }"
	# shellcheck disable=SC2086  # deliberate word splitting into positionals
	set -- ${rest}
	CHILD_CPU_HZ=$((${13} + ${14}))
}

read_child_cpu_hz
cpu_before="${CHILD_CPU_HZ}"

# `timeout` kills a healthy run, so its exit status says nothing; the log is
# the oracle. `|| true` keeps `set -e` from ending the script before we look.
timeout "${SECONDS_TO_RUN}" "${QEMU}" \
	-M "${QEMU_MACHINE}" -kernel "${IMG}" \
	-serial mon:stdio -display none </dev/null >"${log}" 2>&1 || true

# How much host CPU the emulator actually got, in hundredths of a core per
# second of wall time. On an unloaded machine TCG saturates roughly three cores
# (measured: 2.88); under a cgroup quota tight enough to make the guest miss
# deadlines it collapses to 0.08. Two orders of magnitude apart, so the
# threshold below is nowhere near either edge.
#
# Guest tick reports were tried first and are the wrong signal: TCG drives the
# guest timer from wall-clock time, so the count tracks how long the run lasted
# rather than how much CPU it received. At a 20% quota the guest still reported
# 13 ticks while running on a fifth of one core.
clk_tck="$(getconf CLK_TCK 2>/dev/null || echo 100)"
read_child_cpu_hz
emulator_cores=$(((CHILD_CPU_HZ - cpu_before) * 100 / (clk_tck * SECONDS_TO_RUN)))
CORES_TO_BE_MEASURABLE=100 # one whole core, averaged over the run

fail() {
	echo "boot-check: FAIL — $1" >&2
	echo "--- serial log ---" >&2
	cat "${log}" >&2
	exit 1
}

# Neither pass nor fail: the run did not establish its claim. Exit code 3 so a
# caller can tell it from both, and non-zero so nothing downstream mistakes an
# unestablished claim for a verified one.
indeterminate() {
	echo "boot-check: INDETERMINATE — $1" >&2
	printf '  the emulator got %s.%02d cores of host CPU over %ss; under 1.00 it\n' \
		$((emulator_cores / 100)) $((emulator_cores % 100)) "${SECONDS_TO_RUN}" >&2
	echo "  cannot be asked to meet a deadline. Re-run on an idle machine." >&2
	echo "  This is not a kernel failure, and it is not a pass." >&2
	exit 3
}

# Each assertion covers a distinct subsystem, so a failure localises itself.
grep -q 'Harbor: hello' "${log}" ||
	fail "no console output: the kernel did not reach bootstrap::run"
grep -q 'MMU on' "${log}" ||
	fail "the kernel map did not activate"
# Why the board came up. QEMU models `PM_RSTS` and reports a power-on; the
# assertion is on the *shape*, because a warm reset or an unmodelled block are
# both legitimate readings and only silence means the read never happened.
#
# `None` is deliberately a distinct outcome from `PowerOn` in the decode, so a
# register that latched nothing cannot be reported as a clean power cycle.
grep -qE 'reset: (PowerOn|Watchdog|Software|Debug|None) partition=[0-9]+ \(PM_RSTS=0x[0-9a-f]{8}\)' "${log}" ||
	fail "reset-cause line missing or malformed: $(grep '^reset:' "${log}" || echo '(no reset line at all)')"
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
# An exit found a stack still parked from an earlier exit. The stack is released
# rather than leaked, so the pool and the heap both stay consistent and no other
# assertion here would move — which is exactly why this one exists. Bootstrap
# spawns ten tasks that exit at different times, so if the drain in
# `task_trampoline` regresses, this boot is where it shows.
if grep -q 'sched: PENDING-OVERWRITE' "${log}"; then
	fail "an exit found a parked task stack — the pending_free drain has a hole"
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
grep -q 'pl011-agent: rx poll empty' "${log}" ||
	fail "pl011 EL0 RX empty-poll path failed"
grep -q 'pl011-agent: rx own begin' "${log}" ||
	fail "pl011 RX ownership window did not start"
grep -q 'pl011-agent: rx own bytes=2' "${log}" ||
	fail "pl011 RX ownership did not receive loopback bytes"
grep -q 'pl011-agent: rx own end' "${log}" ||
	fail "pl011 RX ownership window did not end (drain not restored path)"
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
# The three refusal counters are separate on purpose: this one is authority
# violations only. It used to be a single number covering a full mailbox and a
# dead endpoint too, so a boot that filled a four-deep mailbox would have
# satisfied this assertion without any capability ever being checked.
grep -qE 'ipc: refuse count=[1-9]' "${log}" ||
	fail "ipc forge was not refused (capability hold check)"

# Neither of the other two should move in a healthy boot: nothing here fills a
# mailbox, and a refusal for `state` means an endpoint resolved and then named a
# dead mailbox, which is kernel bookkeeping being wrong rather than a caller's
# mistake.
grep -qE 'ipc: refuse count=[0-9]+ full=0 state=0' "${log}" ||
	fail "ipc refused a send for capacity or hit a dead endpoint"
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
# these are the quietest possible conditions.
#
# It is also the one assertion in this script that measures the *host*. TCG
# emulates the guest timer against wall-clock time, so a machine busy with
# something else — a parallel `cargo build`, a `cargo install` — makes the guest
# miss deadlines it would never miss otherwise. Seen once, at load average 4.
#
# The note this check used to carry said "if it fires while something else is
# compiling, re-run before believing it" — which is an invitation to ignore a
# red, and this project does not have one of those anywhere else. So the check
# now corroborates instead of advising: a missed deadline is a real failure when
# the guest demonstrably had the CPU to meet it, and an unanswerable question
# when it did not.
#
# Load average is not the corroborating signal, and neither is the guest's own
# tick count — both were tried. Load average was 4 on the machine where this was
# written while the boot was clean, because the load sat on other cores; and TCG
# drives the guest timer from wall-clock time, so tick reports measure how long
# the run lasted rather than how much CPU it got. What separates the two cases
# is the host CPU the emulator was actually given, measured above.
if grep -q 'timer: MISSED' "${log}"; then
	if [[ "${emulator_cores}" -lt "${CORES_TO_BE_MEASURABLE}" ]]; then
		indeterminate "the timer missed deadlines on a host that starved the emulator"
	fi
	fail "timer deadlines expired unserviced, and the emulator had the CPU to meet them"
fi
if grep -qi 'PANIC' "${log}"; then
	fail "the kernel panicked"
fi

printf 'boot-check: clean (%s tick reports, emulator had %s.%02d cores)\n' \
	"$(grep -c 'ticks=' "${log}")" $((emulator_cores / 100)) $((emulator_cores % 100))
