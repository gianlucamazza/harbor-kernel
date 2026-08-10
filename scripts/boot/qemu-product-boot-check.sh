#!/usr/bin/env bash
# Boot the product image (no oracle) and assert the shipped path is healthy.
#
# Complements the static half of `product-image.sh` (marker + symbol check)
# with a QEMU run of the image that actually goes on the SD card.
#
# ## What this gate is
#
# The **composition minimum** for the product configuration (excellence
# F-R5-2 / multi-role 2026-08-10 R5): not a second copy of the oracle
# (`scripts/lib/boot-oracle.sh` ~100 demo lines), but every product-path
# claim that the shipped image already prints and that a silent regression
# would hide. Oracle demos stay behind `feature = "oracle"`; this gate
# refuses their strings rather than re-running them.
#
# Layers (each failure localises):
#   1. Boot identity — hello, MMU, reset, CPU, discovery, RNG probe
#   2. Memory self-checks — frames, heap reclaim, unmap/split smoke
#   3. IRQ / timer / dual-current SMP (product path starts core 1)
#   4. Composition — console server + multi-agent store + wire bytes
#   5. Invariant beacon + anomaly negatives
#   6. Oracle-free surface — demo strings must not appear
set -euo pipefail

cd "$(dirname "$0")/../.."

readonly TARGET=aarch64-unknown-none-softfloat
readonly OUT="target/${TARGET}/release"
readonly IMG="${OUT}/kernel8-product.img"
readonly QEMU="${QEMU:-qemu-system-aarch64}"
# Ceiling, not a fixed duration: product reaches composition + first tick
# report well under this on a quiet host (ADR-0087 shape for short boots).
readonly SECONDS_LIMIT="${PRODUCT_BOOT_SECONDS:-8}"

if [[ ! -f "${IMG}" ]]; then
	echo "product-boot-check: building product image" >&2
	./scripts/boot/product-image.sh
fi

if ! command -v "${QEMU}" >/dev/null; then
	if [[ "${ALLOW_BOOT_SKIP:-}" == "1" ]]; then
		echo "product-boot-check: SKIPPED — ${QEMU} missing, ALLOW_BOOT_SKIP set" >&2
		exit 0
	fi
	echo "error: ${QEMU} not found — product boot check cannot run" >&2
	exit 1
fi

log="$(mktemp)"
trap 'rm -f "${log}"' EXIT

# Store is already in the image (ADR-0029 inject). No -device loader.
timeout "${SECONDS_LIMIT}" "${QEMU}" \
	-machine raspi4b \
	-nographic \
	-serial mon:stdio \
	-d guest_errors \
	-kernel "${IMG}" \
	>"${log}" 2>&1 || true

fail() {
	echo "product-boot-check: FAIL — $1" >&2
	echo "--- serial log ---" >&2
	cat "${log}" >&2 || true
	exit 1
}

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
# Concurrent product agents share the console endpoint: bytes may interleave.
grep -qa 'H!' "${log}" || fail "beacon bytes did not reach the wire"
grep -qaF '?' "${log}" || fail "chirp byte did not reach the wire"

# ---------------------------------------------------------------------------
# 5. Invariant beacon + anomaly negatives
# ---------------------------------------------------------------------------
# Must-stay-zero set (excellence C-6). `blocked`/`frames_free`/`preempts` are
# load-dependent and deliberately not pinned.
grep -qaE 'invariants: overwrites=0 abandoned=0 faults=0 ' "${log}" ||
	fail "invariant beacon missing or non-zero (overwrites/abandoned/faults)"
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

n_assert=35
echo "product-boot-check: clean (${n_assert}+ layered composition-minimum assertions)"
