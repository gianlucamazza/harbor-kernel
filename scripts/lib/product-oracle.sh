#!/usr/bin/env bash
# The product image's composition-minimum assertions, in one place.
#
# Sourced by `scripts/boot/qemu-product-boot-check.sh` (emulated) and by
# `scripts/check/hw-transcript-check.sh` (silicon), for the same reason
# `boot-oracle.sh` exists: a hardware gate that carried its own copy of the
# assertions would be a second oracle to keep in step, and the two would
# disagree on the day it mattered.
#
# Contract for a caller: set `log` to a file holding one boot's output, define
# `fail()`, then call `assert_product_boot`. The hardware caller strips its
# capture timestamps first.
#
# What this asserts is the **product** path — the image that goes on the card,
# without `feature = "oracle"`. Layers, each failure localising:
#   1. Boot identity  2. Memory self-checks  3. IRQ / timer / dual-current SMP
#   4. Composition (console server + store + wire bytes)  5. Invariants
#   6. Oracle-free surface

# `log` and `fail` come from the caller — see the contract above.
# shellcheck disable=SC2154
assert_product_boot() {
	# ---------------------------------------------------------------------------
	# 1. Boot identity (same lines product and oracle share — not demo scaffolding)
	# ---------------------------------------------------------------------------
	grep -qa 'Harbor: hello' "${log}" || fail "product image did not boot"
	grep -qa 'MMU on' "${log}" || fail "the kernel map did not activate"
	grep -qaE 'reset: (PowerOn|Watchdog|Software|Debug|None) partition=[0-9]+ \(PM_RSTS=0x[0-9a-f]{8}\)' "${log}" ||
		fail "reset-cause line missing or malformed: $(grep -a '^reset:' "${log}" || echo '(no reset line)')"
	grep -qaE 'cpu: Cortex-A72 r[0-9]+p[0-9]+ asid16 pa44 \(MIDR=0x[0-9a-f]{8}\)' "${log}" ||
		fail "cpu identity missing or not Cortex-A72: $(grep -a '^cpu:' "${log}" || echo '(no cpu line)')"
	# Discovery (ADR-0072/0073): shapes only — QEMU may be DTB-less.
	grep -qaE 'discover: model ("[^"]*"( rev=0x[0-9a-f]+)? \(fdt\)|unknown \([a-z0-9 _-]+\))' "${log}" ||
		fail "discover model line missing"
	grep -qaE 'discover: memory ([0-9]+ MiB \([0-9]+ ranges?\) (matches|beyond compiled map|short) \(identity [0-9]+ MiB\)|unknown \([a-z0-9 _-]+\))' "${log}" ||
		fail "discover memory line missing"
	grep -qaE 'discover: cpus ([0-9]+ \(fdt\) smp-seen=[0-9]+ (matches|differs)|unknown \([a-z0-9 _-]+\))' "${log}" ||
		fail "discover cpus line missing"
	grep -qaE 'discover: display compiled=(on|off) \(claim, not probed\)' "${log}" ||
		fail "discover display line missing"
	grep -qaE 'rng200: (ok |unavailable \()' "${log}" ||
		fail "RNG200 probe line missing (expected ok or unavailable)"
	grep -qaE 'boot: mmu=[0-9]+ ms discover=[0-9]+ ms ready=[0-9]+ ms' "${log}" ||
		fail "boot timing line missing"

	# ---------------------------------------------------------------------------
	# 2. Memory self-checks (product bootstrap — console_loop, not oracle demos)
	# ---------------------------------------------------------------------------
	grep -qaE 'frames: [0-9]+ free / [0-9]+' "${log}" || fail "frame pool boot line missing"
	grep -qa 'fully reclaimed' "${log}" || fail "the allocator did not return freed memory"
	grep -qa 'unmap: remapped and freed' "${log}" || fail "unmap/remap smoke missing"
	grep -qaE 'split: page at 0x[0-9a-f]+ split [0-9]+, remapped' "${log}" ||
		fail "L2→L3 split smoke missing"

	# ---------------------------------------------------------------------------
	# 3. IRQ / timer / dual-current (product path: start_cpu1 + marker)
	# ---------------------------------------------------------------------------
	grep -qaE 'irq: sealed with [1-9][0-9]* handlers registered' "${log}" ||
		fail "IRQ table was not sealed with at least one handler"
	grep -qa 'IRQs enabled (timer + UART RX)' "${log}" || fail "IRQs were not enabled"
	# First periodic report proves the timer IRQ path (TICK_PRINT_EVERY = 10).
	grep -qaE 'ticks=[1-9][0-9]*' "${log}" || fail "no timer tick report (timer path dead?)"
	grep -qa 'smp: core1 alive' "${log}" || fail "core 1 did not unpark"
	grep -qa 'smp: core1 ipi' "${log}" || fail "core 1 IPI probe missing"
	grep -qa 'smp: core1 ran' "${log}" || fail "core 1 marker did not run (multi-current product path)"

	# ---------------------------------------------------------------------------
	# 4. Composition minimum (M8 + P1 store)
	# ---------------------------------------------------------------------------
	grep -qa 'console-server: up' "${log}" || fail "console server did not spawn"
	grep -qa 'console: capability minted' "${log}" || fail "console send capability was not minted"
	grep -qa 'loader: store n=2 image' "${log}" || fail "product did not load the injected multi-agent store"
	# ADR-0088: product composition pins chirp on CPU 1; beacon stays home 0.
	grep -qaE 'loader: beacon loaded text=[0-9]+ stack=[0-9]+ home=0' "${log}" ||
		fail "beacon was not loaded on home=0"
	grep -qaE 'loader: chirp loaded text=[0-9]+ stack=[0-9]+ home=1' "${log}" ||
		fail "chirp was not loaded on home=1 (product multi-core pin)"
	grep -qa 'loader: beacon ran sends=2 refusals=0' "${log}" || fail "beacon did not run successfully"
	grep -qa 'loader: chirp ran sends=1 refusals=0' "${log}" || fail "chirp did not run successfully"
	# Concurrent product agents share the console endpoint: bytes may interleave —
	# and since ADR-0088 pins chirp on CPU 1, they *do*. `H!` was written when the
	# composition was single-core and asserts the opposite of the line above it:
	# it passes only when beacon's two sends land adjacently, which is a claim
	# about the host's vCPU scheduling, not about the kernel. On a host that gives
	# QEMU less than a core it fails about half the time, with `H?!` on the wire —
	# chirp's byte between beacon's two. That is a correct product boot.
	#
	# What the kernel actually promises is that beacon's bytes arrive, in order,
	# through the console server. So: `H`, then `!`, with only other agents'
	# non-alphanumeric bytes allowed in between (ADR-0087's rule that no assertion
	# may depend on the host, applied to an assertion rather than to a wait).
	grep -qaE 'H[^[:alnum:][:space:]]*!' "${log}" ||
		fail "beacon bytes did not reach the wire, or arrived out of order"
	grep -qaF '?' "${log}" || fail "chirp byte did not reach the wire"

	# ---------------------------------------------------------------------------
	# 5. Invariant beacon + anomaly negatives
	# ---------------------------------------------------------------------------
	# Must-stay-zero set (excellence C-6). `blocked`/`frames_free`/`preempts` are
	# load-dependent and deliberately not pinned.
	grep -qaE 'invariants: overwrites=0 abandoned=0 faults=0 ' "${log}" ||
		fail "invariant beacon missing or non-zero (overwrites/abandoned/faults)"
	# ADR-0098: the density meter must be on the line, in the shipped image —
	# `oracle-census.sh` reads its peak instead of carrying a constant, and a
	# missing field there fails the census rather than falling back to one.
	# Shape only here (live ≤ peak, both non-zero); the ceiling is the census's
	# question, not this smoke's.
	# `|| true`: under `set -e` a grep with no match would end the gate with
	# status 1 and no message, which is the failure mode this whole ADR is
	# about — a red that says nothing.
	slots_field="$(grep -oaE 'slots=[0-9]+/[0-9]+' "${log}" | tail -n1 || true)"
	[[ -n "${slots_field}" ]] ||
		fail "invariant beacon carries no slots=<live>/<peak> field (ADR-0098)"
	slots_live="${slots_field#slots=}"
	slots_peak="${slots_live#*/}"
	slots_live="${slots_live%%/*}"
	((slots_live >= 1)) || fail "slots reports ${slots_live} live with the console loop running"
	((slots_peak >= slots_live)) ||
		fail "slots peak ${slots_peak} is below the live count ${slots_live} — the watermark does not track"
	if grep -qa 'sched: ABANDONED' "${log}"; then
		fail "a task stack was abandoned (guard remap refused)"
	fi
	if grep -qa 'sched: PENDING-OVERWRITE' "${log}"; then
		fail "an exit found a parked task stack — pending_free drain hole"
	fi
	# `timer: MISSED` is host-load sensitive under TCG (ADR-0087). The full
	# boot-check measures emulator CPU and can say INDETERMINATE; this short
	# product smoke does not — ignore deadline miss here rather than fail green
	# composition on a busy laptop.

	# ---------------------------------------------------------------------------
	# 6. Oracle-free surface — product must not carry demo scaffolding
	# ---------------------------------------------------------------------------
	# `product-image.sh` greps the ELF; this catches a cfg leak that still boots.
	oracle_leaks=(
		'panic-probe:'
		'sched: spawned task-a'
		'sched: spawned task-b'
		'sched: spawned el0-task'
		'aspace: prepare ok'
		'el0: SVC ok'
		'density: thin'
		'density: mini'
		'ipc: sent tag='
		'agents: concurrent'
		'pl011-agent:'
		'loader: builtin'
	)
	for marker in "${oracle_leaks[@]}"; do
		if grep -qaF -- "${marker}" "${log}"; then
			fail "product image ran oracle scaffolding (saw '${marker}')"
		fi
	done
}
