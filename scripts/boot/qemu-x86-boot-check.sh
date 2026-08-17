#!/usr/bin/env bash
# H3 L0 lab oracle (ADR-0071): boot the x86_64 freestanding ELF under QEMU q35
# and assert banner + cpu + alive. Separate from the AArch64 product oracle
# (scripts/lib/boot-oracle.sh) — status vocabulary is done (QEMU-x86).
set -euo pipefail

ELF="${1:?usage: $0 <harbor-x86.elf> [seconds]}"
SECONDS_TO_RUN="${2:-5}"
QEMU="${QEMU_X86:-qemu-system-x86_64}"
QEMU_MACHINE="${QEMU_X86_MACHINE:-q35}"
QEMU_CPU="${QEMU_X86_CPU:-qemu64}"

if [[ ! -f "${ELF}" ]]; then
	echo "error: x86 lab image not found: ${ELF}" >&2
	exit 1
fi

if ! command -v "${QEMU}" >/dev/null; then
	if [[ "${CI:-}" == "true" ]]; then
		echo "x86-boot-check: FAIL — ${QEMU} missing, and a skip is refused in CI" >&2
		echo "  ALLOW_BOOT_SKIP is for a workstation without the emulator." >&2
		echo "  In CI it would report a green gate that never ran (ADR-0096)." >&2
		exit 1
	fi
	if [[ -n "${ALLOW_BOOT_SKIP:-}" ]]; then
		echo "x86-boot-check: SKIPPED — ${QEMU} missing, ALLOW_BOOT_SKIP set" >&2
		exit 0
	fi
	echo "error: ${QEMU} not found — the x86 lab boot check cannot run" >&2
	echo "  install it (pacman -S qemu-system-x86), or set ALLOW_BOOT_SKIP=1" >&2
	exit 1
fi

# ADR-0087: `timeout`'s exit code cannot tell a healthy hlt-loop from a starved
# guest, and this gate had no guard at all — the same defect `qemu-virtio-check`
# carried until 2026-08-17, in the same shape, found by the same audit.
# shellcheck source=scripts/lib/cpu-budget.sh
source "${BASH_SOURCE[0]%/*}/../lib/cpu-budget.sh"

log="$(mktemp)"
trap 'rm -f "${log}"' EXIT

# Capture COM1 via stdio → log file. QEMU's `file:` chardev can drop the last
# writes on SIGTERM under load; stdio is flushed with the process exit path.
# -no-reboot: triple-fault stops the VM instead of spinning.
# -accel tcg: do not fight host KVM contention when the box is busy.
# timeout returns 124 on wall-clock expiry (expected: guest hlt-loops).
set +e
cpu_budget_start
# QEMU directly rather than behind `timeout`, so `cpu_budget_watch` samples the
# emulator instead of a wrapper that accrues no CPU of its own. Its deadline is
# the ceiling; SIGTERM ends the hlt loop, exactly as `timeout` did.
"${QEMU}" \
	-accel tcg \
	-machine "${QEMU_MACHINE}" \
	-cpu "${QEMU_CPU}" \
	-m 128M \
	-kernel "${ELF}" \
	-display none \
	-serial stdio \
	-monitor none \
	-no-reboot \
	</dev/null >"${log}" 2>&1 &
qemu_pid=$!
cpu_budget_watch "${qemu_pid}" "${SECONDS_TO_RUN}"
kill -TERM "${qemu_pid}" 2>/dev/null || true
wait "${qemu_pid}" 2>/dev/null
set -e

# No exit-status check on the emulator: this gate ends it with SIGTERM at the
# ceiling, so its status says only that we stopped it. The assertions below are
# the verdict.

# ADR-0087's rule, and its shape matters here: the measured share consults a
# **failure**, it does not invalidate a pass. This guest boots in milliseconds
# and then `hlt`-loops by design, so its average over a 5-second window is
# legitimately a few hundredths of a core on a perfectly idle host — a
# pre-assertion check reported INDETERMINATE on a run whose five lines were all
# present and correct. `boot-check` had this right; the first draft here did
# not.
fail() {
	# `ENDED_EARLY` first: an emulator that exited on its own was not starved,
	# it stopped. Without that distinction a corrupt image — which QEMU rejects
	# in milliseconds, burning no CPU — would report INDETERMINATE forever
	# rather than the red it is.
	if ((CPU_BUDGET_ENDED_EARLY == 1)) ||
		cpu_budget_verdict "x86-boot-check" >/dev/null 2>&1; then
		echo "x86-boot-check: FAIL — $1" >&2
	else
		echo "x86-boot-check: INDETERMINATE — $1" >&2
		echo "  The emulator received ${CPU_BUDGET_CORES} hundredths of a core;" >&2
		echo "  this failure is not established. Rerun on a quieter host." >&2
		echo "--- serial log ---" >&2
		cat -A "${log}" >&2 || true
		exit 3
	fi
	echo "--- serial log ---" >&2
	cat -A "${log}" >&2 || true
	exit 1
}

# -a: guest may emit NULs; do not let grep treat the log as binary.
grep -qa 'Harbor: hello (x86 lab)' "${log}" ||
	fail "banner missing: $(grep -a 'Harbor:' "${log}" || echo '(no Harbor line)')"
grep -qaE '^cpu: .+ family=[0-9]+ model=[0-9]+ \(eax1=0x[0-9a-f]+\)' "${log}" ||
	fail "cpu identity missing or malformed: $(grep -a '^cpu:' "${log}" || echo '(no cpu line)')"
grep -qa 'x86-lab: alive' "${log}" ||
	fail "alive marker missing"

echo "x86-boot-check: clean ($(wc -l <"${log}") lines, ${SECONDS_TO_RUN}s budget)"
