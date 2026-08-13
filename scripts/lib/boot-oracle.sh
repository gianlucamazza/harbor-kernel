#!/usr/bin/env bash
# The boot oracle's assertions — the single definition of "this boot is sound",
# shared verbatim by the QEMU gate (`scripts/boot/qemu-boot-check.sh`) and the
# hardware transcript check (`scripts/check/hw-transcript-check.sh`). Two
# runners, one owner: an assertion added for QEMU is automatically demanded of
# silicon, and a hardware-only drift cannot hide in a second copy — the Pi 4B
# run of 2026-08-09 failed three of exactly these lines (stale early-map TLB,
# ADR-0050 amendment) and QEMU could not see it.
#
# Contract: the caller sets `log` (a de-timestamped serial log) and defines
#   fail <msg>          — report and exit non-zero
#   on_timer_missed     — verdict for a `timer: MISSED` line (an emulator can
#                         be starved by its host; silicon cannot)
# before calling `assert_boot_oracle`.

assert_boot_oracle() {
	# The caller's side of the contract, enforced: a missing log or a missing
	# verdict function is a wiring bug, not an empty pass.
	: "${log:?assert_boot_oracle: caller must set log}"
	declare -F fail >/dev/null || {
		echo "assert_boot_oracle: caller must define fail()" >&2
		exit 2
	}
	declare -F on_timer_missed >/dev/null || {
		echo "assert_boot_oracle: caller must define on_timer_missed()" >&2
		exit 2
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
	# Which core the image found itself on (ADR-0065). Unlike the reset line,
	# this one pins the *values*, not just the shape: QEMU's `-cpu cortex-a72`
	# and the Pi 4B's silicon report the same part, 16-bit ASIDs and a 44-bit
	# PA range, so a divergence in either runner — a QEMU machine-model change,
	# a different Pi — is exactly the drift this assertion exists to catch.
	# Stepping (rNpM) stays free: it varies by silicon batch and proves nothing.
	grep -qaE 'cpu: Cortex-A72 r[0-9]+p[0-9]+ asid16 pa44 \(MIDR=0x[0-9a-f]{8}\)' "${log}" ||
		fail "cpu identity line missing or not the expected Cortex-A72: $(grep -a '^cpu:' "${log}" || echo '(no cpu line at all)')"
	# The discovery report (ADR-0072/0073): one line per fact, unconditional.
	# Shapes, not values — HW carries a firmware-patched tree (real revision,
	# real RAM), the CI fixture is the un-patched distributed blob (zero-size
	# memory, no revision), and a DTB-less boot prints the `unknown (...)`
	# forms. Every row accepts its degraded shape; only *silence* fails,
	# because silence means the report never ran (fail-open is not fail-mute).
	grep -qaE 'discover: model ("[^"]*"( rev=0x[0-9a-f]+)? \(fdt\)|unknown \([a-z0-9 _-]+\))' "${log}" ||
		fail "discover model line missing or malformed: $(grep -a '^discover: model' "${log}" || echo '(no line)')"
	grep -qaE 'discover: memory ([0-9]+ MiB \([0-9]+ ranges?\) (matches|beyond compiled map|short) \(identity [0-9]+ MiB\)|unknown \([a-z0-9 _-]+\))' "${log}" ||
		fail "discover memory line missing or malformed: $(grep -a '^discover: memory' "${log}" || echo '(no line)')"
	grep -qaE 'discover: cpus ([0-9]+ \(fdt\) smp-seen=[0-9]+ (matches|differs)|unknown \([a-z0-9 _-]+\))' "${log}" ||
		fail "discover cpus line missing or malformed: $(grep -a '^discover: cpus' "${log}" || echo '(no line)')"
	grep -qaE 'discover: display compiled=(on|off) \(claim, not probed\)' "${log}" ||
		fail "discover display line missing or malformed: $(grep -a '^discover: display' "${log}" || echo '(no line)')"
	# RNG200 is always probed after the MMU. QEMU has no backend: expect soft
	# NotPresent. Silicon logs `ok word=…`. Either shape is a successful probe path;
	# silence would mean the probe panicked or never ran.
	grep -qaE 'rng200: (ok |unavailable \()' "${log}" ||
		fail "RNG200 probe line missing (expected ok or unavailable)"
	grep -qa 'fully reclaimed' "${log}" ||
		fail "the allocator did not return freed memory"
	# Bring-up cost on the board's own clock. Shape only: an emulator starved by
	# its host and a cold Pi produce wildly different numbers, and pinning one
	# would make this an assertion about the *host*. What it does catch is the
	# line disappearing, which is how a phase mark gets lost in a refactor.
	grep -qaE 'boot: mmu=[0-9]+ ms discover=[0-9]+ ms ready=[0-9]+ ms' "${log}" ||
		fail "boot timing line missing or malformed: $(grep -a '^boot: ' "${log}" || echo '(no line)')"
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
	# K7 / ADR-0050: dual AS with distinct ASIDs both enter EL0.
	grep -qaE 'asid: dual a=[1-9][0-9]* b=[1-9][0-9]* ok' "${log}" ||
		fail "ASID dual-AS oracle failed"
	if grep -qa 'asid: LEAK' "${log}"; then
		fail "ASID pool leaked tags on dual destroy"
	fi
	if grep -qa 'asid: dual FAILED' "${log}" || grep -qa 'asid: dual el0 FAILED' "${log}"; then
		fail "ASID dual path reported failure"
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
	#
	# "Machine-wide" means machine-wide *for IPC sends*: refusals raised by the
	# syscall reply mappers (wait-irq, resolve, transfer — including every
	# el0-xfer-peer refusal) land in per-session SessionStats and never in this
	# counter. Budgeting a new refusal against this number is the mistake this
	# sentence exists to prevent.
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
	# ADR-0090 / K10 residual: force-exit a Running (non-Blocked) task.
	grep -qaE 'force-kill: requested events=[1-9]' "${log}" ||
		fail "force-kill supervisor did not request exit (ADR-0090)"
	grep -qa 'force-kill: child forced' "${log}" ||
		fail "force-kill child did not observe force_exit (ADR-0090)"
	grep -qa 'force-kill: slot empty' "${log}" ||
		fail "force-killed task slot was not reclaimed (ADR-0090)"
	# ADR-0034 / K9: second driver-as-agent (RNG200 page); QEMU may fault the load.
	grep -qaE 'rng-agent: map (read|fault) ok' "${log}" ||
		fail "rng-agent did not exercise the Device page map (ADR-0034)"
	grep -qa 'rng-agent: killed ok' "${log}" ||
		fail "rng-agent did not destroy/unmap (ADR-0034)"
	# ADR-0035 / P5: name registry bind/resolve/missing.
	grep -qa 'name: resolved' "${log}" ||
		fail "name registry did not resolve a bound service (ADR-0035)"
	grep -qa 'name: missing' "${log}" ||
		fail "name registry did not report missing (ADR-0035)"
	# ADR-0036 / P2: on-target keyed blob put/get/missing.
	grep -qa 'store: got' "${log}" ||
		fail "blob store did not round-trip put/get (ADR-0036)"
	grep -qa 'store: missing' "${log}" ||
		fail "blob store did not report missing (ADR-0036)"
	# ADR-0037 / K3 residual: transfer SEND between tasks.
	grep -qa 'ipc: transfer ok' "${log}" ||
		fail "cap transfer did not hand SEND to recipient (ADR-0037)"
	# ADR-0038 / K10 residual: creator exit cancels blocked child.
	grep -qa 'cascade: cancelled' "${log}" ||
		fail "creator-exit cascade did not cancel blocked child (ADR-0038)"
	# ADR-0039 / ADR-0052 / P5 residual: resolve grant + EL0 SYS_RESOLVE.
	grep -qa 'resolve-grant: refused' "${log}" ||
		fail "SYS_RESOLVE without grant did not refuse (ADR-0052)"
	grep -qa 'el0-resolve: ok' "${log}" ||
		fail "EL0 resolve did not install a named cap (ADR-0039)"
	grep -qa 'el0-resolve: refused' "${log}" ||
		fail "EL0 resolve did not refuse a missing name (ADR-0039)"
	# ADR-0040 / K2 residual: park timeout cancels without a sender.
	grep -qa 'ipc: timed-out cancelled' "${log}" ||
		fail "park timeout did not cancel the waiter (ADR-0040)"
	# ADR-0041 / K3 residual: EL0 transfer to creator.
	grep -qa 'el0-xfer: ok' "${log}" ||
		fail "EL0 transfer did not return cap to creator (ADR-0041)"
	grep -qa 'el0-xfer: refused' "${log}" ||
		fail "EL0 transfer did not refuse a bad move (ADR-0041)"
	# ADR-0054 / K3 residual: peer transfer via task-cap.
	grep -qa 'el0-xfer-peer: ok' "${log}" ||
		fail "EL0 peer transfer did not deliver cap to peer (ADR-0054)"
	grep -qa 'el0-xfer-peer: refused' "${log}" ||
		fail "EL0 peer transfer did not refuse without task-cap (ADR-0054)"
	# ADR-0061: the refusal is attributable. detail=4 (BadToTask) is the empty
	# key slot; a vanished task-cap check would answer detail=3 (BadFromSlot),
	# which the pre-taxonomy assertion could not distinguish (review F-8).
	grep -qaE 'el0-xfer-peer: refused refusals=[1-9][0-9]* detail=4' "${log}" ||
		fail "peer-transfer refusal detail is not BadToTask (ADR-0061)"
	# The move invariant, not just the delivery: the donor lost the SEND and kept
	# the task-cap. A copy instead of a move would still print "ok" above.
	grep -qa 'el0-xfer-peer: donor emptied' "${log}" ||
		fail "EL0 peer transfer did not move (donor still holds, or lost the task-cap)"
	# ADR-0055: moving the task-cap itself is delegation and refuses by band.
	grep -qa 'xfer-peer: band refused' "${log}" ||
		fail "task-cap moved as an object — the ADR-0055 band filter is gone"
	# ADR-0057: after the target exits, its task-cap is stale and the move refuses.
	# This is the revoke-on-exit invariant made observable end to end; the
	# empty-slot refusal above cannot discriminate it.
	grep -qa 'xfer-peer: stale refused' "${log}" ||
		fail "stale task-cap did not refuse after target exit (ADR-0057)"
	# The invariant beacon prints in both images; here only presence is asserted —
	# the oracle image legitimately takes EL0 faults (the fault demo), so the
	# product check owns the zero assertions.
	grep -qa 'invariants: overwrites=' "${log}" ||
		fail "invariant beacon did not print"
	# Two lines that must never appear: silent mint exhaustion on the boot path,
	# and a moved stale cap. (The former third line, `sched: STALE-TASKCAP`, died
	# with its print: ADR-0062 puts the epoch in the task identity, so the state
	# that cross-check watched for is unrepresentable.)
	if grep -qa 'mint FAILED' "${log}"; then
		fail "task-cap mint exhausted on the boot path (ADR-0057 §2)"
	fi
	if grep -qa 'STALE MOVED' "${log}"; then
		fail "a stale task-cap moved a cap into a recycled slot (ADR-0057 §1)"
	fi
	# ADR-0042 / K2 residual: EL0 recv timeout.
	grep -qa 'el0-timeout: cancelled' "${log}" ||
		fail "EL0 recv timeout did not cancel (ADR-0042)"
	# ADR-0043 / K9 residual: IRQ-cap device agent.
	grep -qa 'irq-device: woke' "${log}" ||
		fail "IRQ device agent did not wait successfully (ADR-0043)"
	# ADR-0044 / K5: thin stack density.
	grep -qaE 'density: thin n=[1-9]' "${log}" ||
		fail "thin-stack density workers did not spawn (ADR-0044)"
	# ADR-0086 / K5-S: mini stack density (2 KiB usable).
	grep -qaE 'density: mini n=[1-9]' "${log}" ||
		fail "mini-stack density workers did not spawn (ADR-0086)"
	# ADR-0045 / P2 durable reload.
	grep -qa 'durable: reloaded' "${log}" ||
		fail "durable store did not round-trip (ADR-0045)"
	# ADR-0066 / P2 media persistence: exactly one of the healthy trio or an
	# honest degraded line. DURABLE_MEDIA_EXPECT pins the mode when the
	# caller controls the media: fresh | previous | absent (empty = any
	# healthy-or-degraded outcome, the migration-friendly default).
	healthy='durable-media: boot=[0-9]+ from=(Fresh|Previous) part=0x7f slot=(-|A|B) seq=[0-9]+'
	degraded='durable-media: (absent|no-card|unsupported|no-partition|error) ?\(.*\)'
	if grep -qaE "${healthy}" "${log}"; then
		grep -qaE 'durable-media: flushed slot=(A|B) seq=[0-9]+' "${log}" ||
			fail "durable media loaded but never flushed (ADR-0066)"
		grep -qa 'durable-media: verified' "${log}" ||
			fail "durable media flush did not verify on read-back (ADR-0066)"
	else
		grep -qaE "${degraded}" "${log}" ||
			fail "no durable-media line at all — the ADR-0066 path never ran"
	fi
	case "${DURABLE_MEDIA_EXPECT:-}" in
	fresh)
		grep -qaE 'durable-media: boot=1 from=Fresh' "${log}" ||
			fail "expected a fresh-media boot (ADR-0066): $(grep -a 'durable-media:' "${log}" | head -1)"
		;;
	previous)
		grep -qaE 'durable-media: boot=([2-9]|[0-9]{2,}) from=Previous' "${log}" ||
			fail "expected media evidence of a previous boot (ADR-0066): $(grep -a 'durable-media:' "${log}" | head -1)"
		;;
	absent)
		grep -qaE 'durable-media: (absent|no-card)' "${log}" ||
			fail "expected the honest no-media line (ADR-0066): $(grep -a 'durable-media:' "${log}" | head -1)"
		;;
	esac
	# ADR-0068 / K4 same-EL preemption: an EL1 task that never yields loses
	# the CPU on the IRQ epilogue. Supersedes the ADR-0046 cooperative
	# `budget: rotated` oracle — the epilogue wins that race by construction,
	# and this claim is strictly stronger (rotation without cooperation).
	# ADR-0070 / K8 first slice: core 1 left WFE and signalled alive.
	grep -qa 'smp: core1 alive' "${log}" ||
		fail "core 1 did not unpark (ADR-0070): $(grep -a 'smp:' "${log}" | head -1)"
	grep -qa 'smp: core1 timeout' "${log}" &&
		fail "core 1 unpark timed out (ADR-0070)"
	# ADR-0074 / K8 second slice: core 1 took SGI 0 from the primary.
	grep -qa 'smp: core1 ipi' "${log}" ||
		fail "core 1 did not handle the wake SGI (ADR-0074): $(grep -a 'smp:' "${log}" | head -3)"
	grep -qa 'smp: core1 ipi timeout' "${log}" &&
		fail "core 1 wake SGI timed out (ADR-0074)"
	grep -qa 'smp: core1 ipi skipped' "${log}" &&
		fail "core 1 IPI probe skipped because IRQs were unbound (ADR-0074)"
	# ADR-0076 / K8 third slice: pinned worker ran on CPU 1 (primary prints).
	grep -qa 'smp: core1 ran' "${log}" ||
		fail "core 1 did not run a pinned task (ADR-0076): $(grep -a 'smp:' "${log}" | head -5)"
	grep -qa 'smp: core1 ran timeout' "${log}" &&
		fail "core 1 pinned task timed out (ADR-0076)"
	grep -qa 'preempt-el1: rotated' "${log}" ||
		fail "EL1 preemption did not rotate the non-yielding spinner (ADR-0068)"
	grep -qa 'preempt-el1: spinner exited' "${log}" ||
		fail "the preempted EL1 spinner did not observe the stop word and exit (ADR-0068)"
	# ADR-0079 / K8: EL1 quantum preemption on CPU 1 (local CNTP + epilogue).
	grep -qa 'preempt-el1-cpu1: rotated' "${log}" ||
		fail "EL1 preemption on CPU 1 did not rotate the non-yielding spinner (ADR-0079)"
	grep -qa 'preempt-el1-cpu1: spinner exited' "${log}" ||
		fail "the CPU1 preempted EL1 spinner did not exit after the stop word (ADR-0079)"
	grep -qa 'preempt-el1-cpu1: peer gave up' "${log}" &&
		fail "CPU1 EL1 preemption peer gave up (ADR-0079)"
	grep -qa 'preempt-el1-cpu1: watch timeout' "${log}" &&
		fail "CPU1 EL1 preemption watch timed out (ADR-0079)"
	# ADR-0081 / K8: EL0 session + quantum preemption on CPU 1.
	grep -qa 'preempt-el0-cpu1: rotated' "${log}" ||
		fail "EL0 preemption on CPU 1 did not rotate the non-yielding spinner (ADR-0081)"
	grep -qa 'preempt-el0-cpu1: spinner exited' "${log}" ||
		fail "the CPU1 preempted EL0 spinner did not exit after the stop word (ADR-0081)"
	grep -qa 'preempt-el0-cpu1: peer gave up' "${log}" &&
		fail "CPU1 EL0 preemption peer gave up (ADR-0081)"
	grep -qa 'preempt-el0-cpu1: watch timeout' "${log}" &&
		fail "CPU1 EL0 preemption watch timed out (ADR-0081)"
	# ADR-0083 / K8: work steal — victim admitted on CPU0 only, ran on affinity 1.
	grep -qa 'smp: steal ok' "${log}" ||
		fail "work steal did not run a CPU0-only worker on affinity 1 (ADR-0083)"
	grep -qa 'smp: steal timeout' "${log}" &&
		fail "work steal oracle timed out (ADR-0083)"
	# ADR-0064 / K4: IRQ preemption — a non-syscalling EL0 spinner loses the CPU.
	grep -qa 'preempt: rotated' "${log}" ||
		fail "IRQ preemption did not rotate the EL0 spinner (ADR-0064)"
	grep -qa 'preempt: spinner exited' "${log}" ||
		fail "the preempted spinner did not observe the stop word and exit (ADR-0064)"

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

	# ADR-0100: the same claim for the *device* vocabulary. `nowindow` is
	# `beacon`'s bytes with a device grant naming window 0, and this product
	# declares no window — so the refusal is `index >= 0`, arithmetic, and it is
	# seen on every good boot rather than argued for in a document.
	#
	# The negative that matters is below it: a refused entry must not have been
	# spawned, because an agent composed to drive a page it cannot have is not an
	# agent anyone asked to run without one.
	grep -qa 'authority: windows 1 declared' "${log}" ||
		fail "the window vocabulary was not declared (ADR-0100/0101)"
	grep -qa 'loader: nowindow refused — names window 3 of 1' "${log}" ||
		fail "an entry naming a window past the vocabulary was not refused"
	if grep -qa 'loader: nowindow loaded' "${log}"; then
		fail "an entry refused a device window was spawned anyway"
	fi
	if grep -qa 'loader: nowindow ran' "${log}"; then
		fail "an entry refused a device window reached EL0"
	fi

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
	# The image declares its feature set, and the declaration has to match what
	# the image actually does. A banner nobody checks is a comment that survived
	# into the log: it drifts the first time a feature is renamed, and it drifts
	# in the direction of claiming more than is there.
	build_line="$(grep -a '^build: ' "${log}" || true)"
	[[ -n "${build_line}" ]] ||
		fail "the image did not declare its feature set (no 'build:' line)"
	# …and which tree it was built from. `nogit` is a legitimate answer (a
	# source tarball, a build outside the repo); an absent field is not, because
	# then a transcript cited by an ADR cannot be tied to a commit at all.
	[[ "${build_line}" == *" src="* ]] ||
		fail "the image did not declare its source id: ${build_line}"
	# The panel is gone (ADR-0094), so the pair of claims this used to check
	# against each other is down to one half: no image may bring a panel up.
	# Kept rather than deleted, because the failure it guards against is a
	# driver coming back without a composition, which is exactly what ADR-0094
	# says must not happen quietly.
	if grep -qa '^display: ' "${log}"; then
		fail "a panel came up, and no image should have one since ADR-0094: ${build_line}"
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
	# timer + UART RX + wake SGI (ADR-0074)
	grep -qa 'irq: sealed with 3 handlers registered' "${log}" ||
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
		# What "missed" means depends on who ran the boot: an emulator can be
		# starved by its host, silicon cannot. The caller supplies the verdict.
		on_timer_missed
	fi
	if grep -qi 'PANIC' "${log}"; then
		fail "the kernel panicked"
	fi
}
