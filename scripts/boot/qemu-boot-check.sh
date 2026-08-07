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

# Every `grep` below carries `-a`. The kernel's own output is text, but an EL0
# agent can `SYS_PUTC` any byte it likes, and one NUL is enough for `grep` to
# decide the log is binary and stop matching. That failure is silent in the
# worst way: the assertions do not report what they found, they report nothing,
# and the first one to notice blames the wrong thing. Seen exactly once, when a
# mutated agent printed a status code of zero.
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
# Oracle path: no external store — builtin beacon+mute (ADR-0027 fallback).
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
grep -qa 'loader: builtin' "${log}" ||
	fail "oracle boot did not use the builtin manifest path"
grep -qa 'Harbor: hello' "${log}" ||
	fail "no console output: the kernel did not reach bootstrap::run"
grep -qa 'MMU on' "${log}" ||
	fail "the kernel map did not activate"
# Why the board came up. QEMU models `PM_RSTS` and reports a power-on; the
# assertion is on the *shape*, because a warm reset or an unmodelled block are
# both legitimate readings and only silence means the read never happened.
#
# `None` is deliberately a distinct outcome from `PowerOn` in the decode, so a
# register that latched nothing cannot be reported as a clean power cycle.
grep -qaE 'reset: (PowerOn|Watchdog|Software|Debug|None) partition=[0-9]+ \(PM_RSTS=0x[0-9a-f]{8}\)' "${log}" ||
	fail "reset-cause line missing or malformed: $(grep -a '^reset:' "${log}" || echo '(no reset line at all)')"
# RNG200 is always probed after the MMU. QEMU has no backend: expect soft
# NotPresent. Silicon logs `ok word=…`. Either shape is a successful probe path;
# silence would mean the probe panicked or never ran.
grep -qaE 'rng200: (ok |unavailable \()' "${log}" ||
	fail "RNG200 probe line missing (expected ok or unavailable)"
grep -qa 'fully reclaimed' "${log}" ||
	fail "the allocator did not return freed memory"
# `mmu::unmap` (and the L2→L3 split when the heap is a block) then remap.
# Failure prints `unmap: FAILED` / `remap FAILED`; silence would mean a hang
# on the first post-unmap access instead.
grep -qa 'unmap: remapped and freed' "${log}" ||
	fail "unmap smoke did not complete (split/TLBI/remap)"
if grep -qa 'unmap: FAILED' "${log}"; then
	fail "mmu::unmap refused a mapped heap page"
fi
# The break-before-make block split, exercised deliberately rather than left to
# an alignment accident. `__heap_start` is below the first 2 MiB boundary, so
# every task stack lands on pages that were never blocks; without this smoke the
# split path would first run in production, the day the heap fills past 2 MiB.
# Asserting `split 1` — not merely that the line appeared — is what proves a
# block was actually rebuilt as a table.
grep -qaE 'split: page at 0x[0-9a-f]+ split 1, remapped' "${log}" ||
	fail "block split path did not run: $(grep -a '^split:' "${log}" || echo '(no split line at all)')"
if grep -qaE 'split: (unmap|remap) FAILED|split: SKIPPED' "${log}"; then
	fail "block split smoke did not complete"
fi
# A task stack leaked because its guard could not be remapped. The heap stays
# consistent — that is why it leaks — so nothing else here would notice.
if grep -qa 'sched: ABANDONED' "${log}"; then
	fail "a task stack was abandoned (guard remap refused)"
fi
# An exit found a stack still parked from an earlier exit. The stack is released
# rather than leaked, so the pool and the heap both stay consistent and no other
# assertion here would move — which is exactly why this one exists. Bootstrap
# spawns ten tasks that exit at different times, so if the drain in
# `task_trampoline` regresses, this boot is where it shows.
if grep -qa 'sched: PENDING-OVERWRITE' "${log}"; then
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
observed="$(grep -aoE '^task-[ab] [0-9]+' "${log}" | tr '\n' ' ' || true)"
expected="task-a 0 task-b 0 task-a 1 task-b 1 task-a 2 task-b 2 task-a 3 task-b 3 "
[[ "${observed}" == "${expected}" ]] ||
	fail "task output not interleaved: ${observed}"
if grep -qa 'spawn task-a FAILED' "${log}" || grep -qa 'spawn task-b FAILED' "${log}"; then
	fail "cooperative task spawn failed"
fi
# ADR-0012 S1: named frame pool initialised (capacity matches BSP constant).
grep -qaE 'frames: [0-9]+ free / [0-9]+' "${log}" ||
	fail "frame pool boot line missing"
# M5 S2/S3: prepare AS + EL0 probes; destroy must not leak frames.
grep -qa 'aspace: prepare ok' "${log}" ||
	fail "address-space prepare for EL0 failed"
grep -qa 'el0: SVC ok' "${log}" ||
	fail "EL0 SVC probe failed"
grep -qa 'el0: FAULT ok' "${log}" ||
	fail "EL0 kernel-store fault probe failed"
grep -qa 'aspace: create/destroy ok' "${log}" ||
	fail "address-space create/destroy smoke failed"
if grep -qa 'aspace: LEAK' "${log}"; then
	fail "address-space leaked frames on destroy"
fi
grep -qa 'aspace: dual create/destroy ok' "${log}" ||
	fail "dual address-space create/destroy smoke failed"
if grep -qa 'aspace: dual LEAK' "${log}"; then
	fail "dual address-space leaked frames"
fi
# M5-P1/P2: scheduled EL0 task + SVC dispatch.
grep -qa 'el0-task: svc ping' "${log}" ||
	fail "scheduled el0-task did not complete svc ping"
grep -qa 'el0-task: svc refuse imm=0x99' "${log}" ||
	fail "scheduled el0-task did not refuse unknown svc imm"
grep -qa 'el0-task: resume pings=2' "${log}" ||
	fail "EL0 SVC resume session did not complete two pings + exit"
grep -qa 'el0-task: console sends=2' "${log}" ||
	fail "EL0 console SYS_SEND session did not emit two bytes"
grep -qaE 'el0-task: irq resume irqs=[1-9]' "${log}" ||
	fail "EL0 IRQ save/resume path did not handle at least one IRQ"
grep -qa 'el0-task: ok' "${log}" ||
	fail "scheduled el0-task leaked frames or failed teardown"
# M6 v1: PL011 page-only agent + kill (ADR-0013).
grep -qa 'pl011-agent: FR read + svc ok' "${log}" ||
	fail "pl011 EL0 agent did not read FR and svc"
grep -qa 'pl011-agent: rx poll empty' "${log}" ||
	fail "pl011 EL0 RX empty-poll path failed"
grep -qa 'pl011-agent: rx own begin' "${log}" ||
	fail "pl011 RX ownership window did not start"
grep -qa 'pl011-agent: rx own bytes=2' "${log}" ||
	fail "pl011 RX ownership did not receive loopback bytes"
grep -qa 'pl011-agent: rx own end' "${log}" ||
	fail "pl011 RX ownership window did not end (drain not restored path)"
grep -qa 'pl011-agent: killed ok' "${log}" ||
	fail "pl011 agent AS destroy / kill path failed"
# Multi-agent shell: two TCBs with AS live together, each EL0 once.
grep -qa 'agents: concurrent ok' "${log}" ||
	fail "concurrent multi-agent shell smoke failed"
if grep -qa 'agents: concurrent LEAK' "${log}"; then
	fail "concurrent multi-agent shell leaked frames"
fi
# M4 IPC (ADR-0008 shape + mailbox): message delivered; forge refuse counted.
grep -qa 'ipc: sent tag=1 a=42' "${log}" ||
	fail "ipc sender did not deliver"
grep -qa 'ipc: got tag=1 a=42' "${log}" ||
	fail "ipc receiver did not get the message"
# The three refusal counters are separate on purpose: this one is authority
# violations only. It used to be a single number covering a full mailbox and a
# dead endpoint too, so a boot that filled a four-deep mailbox would have
# satisfied this assertion without any capability ever being checked.
#
# Exactly six, not "at least one". The count is machine-wide and every
# producer is deliberate:
#
#   1. the M4 forger, with a capability it does not hold
#   2. the EL0 agent naming a slot it was not granted
#   3. the EL0 agent denied the console
#   4-5. the manifest's `mute`, twice — it runs the same image as `beacon` and was
#        granted no console slot, so both of its console SYS_SEND calls are refused
#   6. ADR-0032 creator_try_send of a CapId after channel revoke (stale handle)
#
# A range would let any one of them satisfy the assertion for the others; it
# already did, while a bug had the counter reset by any successful send in
# between. It was three until the loader landed, five with the loader, six with
# product-path revoke.
grep -qaE 'ipc: refuse count=6 ' "${log}" ||
	fail "authority refusals are not exactly the six the boot performs"
# ADR-0024: parks leave a non-zero event count (console server parks repeatedly).
# Instantaneous blocked= can be zero if sampled while the server is draining.
grep -qaE 'sched: blocked=[0-9]+ block_events=[1-9][0-9]*' "${log}" ||
	fail "parked-task counters missing or block_events still zero after the boot oracle"
# K1 / ADR-0028: timer cookie has a real waiter and a real producer (EL1).
grep -qa 'irq-wait: arm cookie=1' "${log}" ||
	fail "irq-wait task did not arm on the timer cookie"
grep -qaE 'irq-wait: woke drops=0 idle_signals=[0-9]+' "${log}" ||
	fail "irq-wait task was not woken cleanly by the timer IRQ"
# K1 remainder / ADR-0030: EL0 waits via a granted IRQ notification (slot), not a raw cookie.
grep -qa 'el0-irq: arm slot=0' "${log}" ||
	fail "EL0 IRQ wait agent did not arm"
grep -qaE 'el0-irq: woke wait_irqs=[1-9]' "${log}" ||
	fail "EL0 SYS_WAIT_IRQ was not woken by the timer"
grep -qaE 'el0-irq: refused refusals=[1-9]' "${log}" ||
	fail "EL0 SYS_WAIT_IRQ empty-slot refuse was not seen"
# ADR-0025: supervisor cancel of an orphaned park.
grep -qa 'ipc: reaped cancelled' "${log}" ||
	fail "orphan receiver was not cancelled (ADR-0025)"
grep -qaE 'ipc: cancel issued cancel_events=[1-9]' "${log}" ||
	fail "supervisor cancel was not issued"
# ADR-0031 / K2: last SEND-hold drop on an ephemeral channel auto-cancels.
grep -qa 'ipc: auto-reaped cancelled' "${log}" ||
	fail "ephemeral last-hold auto-reap did not cancel the waiter (ADR-0031)"
# ADR-0032 / K3: product-path revoke makes a stale CapId refuse send.
grep -qa 'ipc: release stale refused' "${log}" ||
	fail "channel revoke did not refuse a stale CapId (ADR-0032)"
# ADR-0033 / K10: supervisor reaps a blocked child and restarts by re-spawn.
grep -qaE 'supervisor: reaped id=[0-9]+ reap_events=[1-9]' "${log}" ||
	fail "supervisor did not reap a blocked child (ADR-0033)"
grep -qaE 'supervisor: restarted id=[0-9]+' "${log}" ||
	fail "supervisor did not restart by re-spawn (ADR-0033)"
grep -qa 'supervised: cancelled' "${log}" ||
	fail "supervised child did not observe Cancelled"

# ADR-0021: agents are data, and authority is one entry in a table.
#
# `beacon` and `mute` run the **same bytes** — one `const` image in `.rodata`,
# built by the same encoder the assembler oracle checks. The only difference
# between them is whether the manifest gave slot 1 the loader's console
# send capability. So `beacon` printing `H!` and `mute` being refused twice is
# the claim in its smallest form: nothing in the program and nothing in the
# code that spawns it decides the authority.
#
# `mute` also declares two text pages against `beacon`'s one, so a boot exercises
# a window the BSP no longer fixes — and a multi-page text is the reason
# `poke_user` walks pages instead of assuming one contiguous run.
grep -qa 'console-server: up' "${log}" ||
	fail "the EL1 console server did not spawn"
grep -qa 'loader: beacon loaded text=1 stack=3' "${log}" ||
	fail "the loader did not create the granted manifest agent"
grep -qa 'loader: mute loaded text=2 stack=3' "${log}" ||
	fail "the loader did not create an agent with a multi-page text window"
grep -qa 'loader: beacon ran sends=2 refusals=0' "${log}" ||
	fail "the granted manifest agent did not use the console it was given"
grep -qa 'loader: mute ran sends=0 refusals=2' "${log}" ||
	fail "the ungranted manifest agent was not refused the console"
grep -qa 'H!loader: beacon ran' "${log}" ||
	fail "the manifest agent's bytes did not reach the console before its report"

# Neither of the other two should move in a healthy boot: nothing here fills a
# mailbox, and a refusal for `state` means an endpoint resolved and then named a
# dead mailbox, which is kernel bookkeeping being wrong rather than a caller's
# mistake.
grep -qaE 'ipc: refuse count=[0-9]+ full=0 state=0' "${log}" ||
	fail "ipc refused a send for capacity or hit a dead endpoint"
if grep -qa 'ipc: FORGE OK' "${log}"; then
	fail "forged capability send succeeded"
fi
# M7 slice 2 (ADR-0017 §2): authority named by slot index, between EL0 agents.
grep -qa 'el0-ipc: sent slot=0 tag=7 a=42' "${log}" ||
	fail "EL0 agent did not send through the slot it holds"
# The refusal on the good path. An agent naming slot 1 of a table holding one
# capability is reaching past its own authority, and a boot where this line is
# absent is a boot where nothing was ever refused — which this project treats
# as a protection unverified rather than a protection unneeded.
grep -qaE 'el0-ipc: refused slot=1 authority=[1-9]' "${log}" ||
	fail "EL0 agent was not refused a slot it does not hold"
# The image declares its feature set, and the declaration has to match what the
# image actually does. A banner nobody checks is a comment that survived into
# the log: it drifts the first time a feature is renamed, and it drifts in the
# direction of claiming more than is there.
#
# Both directions, because either alone is satisfiable by a lie: an image that
# says `debug-display` must bring the panel up, and one that says `headless`
# must not touch it. The trap this closes is a plain `make deploy` replacing a
# glass build with a headless one, where the only symptom was a dark panel that
# looks exactly like broken hardware.
build_line="$(grep -a '^build: ' "${log}" || true)"
[[ -n "${build_line}" ]] ||
	fail "the image did not declare its feature set (no 'build:' line)"
if [[ "${build_line}" == *"debug-display"* ]]; then
	grep -qa '^display: ' "${log}" ||
		fail "image says debug-display, but the panel never came up: ${build_line}"
elif [[ "${build_line}" == *"headless"* ]]; then
	if grep -qa '^display: ' "${log}"; then
		fail "image says headless, but a panel came up: ${build_line}"
	fi
fi

# ADR-0017 §3: the console is a capability, and one agent is deliberately
# without it. This line is the protection being *seen* to fire on the good path
# — and its second half matters as much: the byte that agent tried to print
# must not appear on the console.
grep -qa 'el0-ipc: console denied, printed nothing' "${log}" ||
	fail "the agent without a console capability was not refused"
if grep -qa 'Xel0-ipc: console denied' "${log}"; then
	fail "the denied agent printed its byte anyway"
fi

# ADR-0018: the kernel ends the session, the creator decides the task. Three
# separate claims, and the third is the one a single "it faulted" line would
# hide — the creator has to still be running afterwards, and so does the peer.
grep -qaE 'el0-ipc: agent faulted esr=0x[0-9a-f]+ far=0x[0-9a-f]+ faults=[1-9]' "${log}" ||
	fail "the deliberate EL0 fault was not reported with its syndrome and count"
grep -qa 'el0-ipc: creator alive after fault' "${log}" ||
	fail "the creator did not survive its agent's fault"

# The payload crossing EL0 → kernel → EL0. The receiving agent SYS_SENDs the
# message field to the console endpoint, so the `*` on the console is the
# byte the *other* agent sent (42), not a status code.
grep -qa 'el0-ipc: got payload via EL0 recvs=1' "${log}" ||
	fail "EL0 agent did not receive the message through its slot"
grep -qa '\*el0-ipc: got payload' "${log}" ||
	fail "the received payload was not printed by the receiving agent"

# ADR-0022: the receiver waited, and the send is what woke it.
#
# Presence is not enough here, and it was all this script checked until the park
# existed. The receiving agent is spawned **first** and opens with no yields, so
# the order of these three lines is the property:
#
#   1. `try-recv empty`  — the mailbox really was empty when the agent arrived,
#      taken by `SYS_TRY_RECV` on the same slot, so the wait that follows is a
#      wait and not a coincidence of scheduling.
#   2. `sent`            — the peer posts.
#   3. `got payload`     — the parked agent resumes with it.
#
# A recv that stopped parking would print the `try-recv unexpected` branch, or
# reach `got payload` before `sent`. Both are red here and neither is red on a
# presence check.
grep -qa 'el0-ipc: try-recv empty without waiting empties=1' "${log}" ||
	fail "SYS_TRY_RECV did not report an empty mailbox without waiting"
line_of() { grep -na "$1" "${log}" | head -1 | cut -d: -f1; }
empty_at="$(line_of 'el0-ipc: try-recv empty')"
sent_at="$(line_of 'el0-ipc: sent slot=0')"
got_at="$(line_of '\*el0-ipc: got payload')"
if [[ -z "${empty_at}" || -z "${sent_at}" || -z "${got_at}" ]]; then
	fail "the EL0 exchange is missing a line the ordering check needs"
fi
if ((empty_at >= sent_at || sent_at >= got_at)); then
	fail "the EL0 receiver did not park: expected empty(${empty_at}) < sent(${sent_at}) < got(${got_at})"
fi

# Two tick reports mean the timer IRQ fired repeatedly *and* the WFI idle loop
# kept waking: a stalled idle loop prints the first and then goes quiet.
grep -qa 'ticks=20' "${log}" ||
	fail "timer IRQ or WFI idle loop stalled"
if grep -qa 'irq: unhandled' "${log}"; then
	fail "unhandled interrupts were dispatched"
fi
# Both handlers registered before the table froze. A boot that registered none
# looks exactly like a healthy one until the first interrupt nobody answers, and
# by then the evidence is a counter rather than the moment it went wrong.
grep -qa 'irq: sealed with 2 handlers registered' "${log}" ||
	fail "dispatch table sealed with the wrong number of handlers: $(grep -a '^irq: sealed' "${log}" || echo '(no seal line at all)')"
# The allocator refuses frees it cannot justify — a double free, or a pointer it
# never handed out. Refusing keeps the heap intact, so nothing else here would
# notice; the count is the only evidence that a caller is wrong about what it owns.
if grep -qa 'heap: REFUSED' "${log}"; then
	fail "the allocator refused an invalid free"
fi
# Received bytes lost for want of ring space. Nothing types during this run, so
# any count here is the RX path losing bytes it was handed.
if grep -qa 'console: DROPPED' "${log}"; then
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
if grep -qa 'timer: MISSED' "${log}"; then
	if [[ "${emulator_cores}" -lt "${CORES_TO_BE_MEASURABLE}" ]]; then
		indeterminate "the timer missed deadlines on a host that starved the emulator"
	fi
	fail "timer deadlines expired unserviced, and the emulator had the CPU to meet them"
fi
if grep -qi 'PANIC' "${log}"; then
	fail "the kernel panicked"
fi

printf 'boot-check: clean (%s tick reports, emulator had %s.%02d cores)\n' \
	"$(grep -ac 'ticks=' "${log}")" $((emulator_cores / 100)) $((emulator_cores % 100))
