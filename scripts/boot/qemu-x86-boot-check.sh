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
	if [[ -n "${ALLOW_BOOT_SKIP:-}" ]]; then
		echo "x86-boot-check: SKIPPED — ${QEMU} missing, ALLOW_BOOT_SKIP set" >&2
		exit 0
	fi
	echo "error: ${QEMU} not found — the x86 lab boot check cannot run" >&2
	echo "  install it (pacman -S qemu-system-x86), or set ALLOW_BOOT_SKIP=1" >&2
	exit 1
fi

log="$(mktemp)"
trap 'rm -f "${log}"' EXIT

# Capture COM1 via stdio → log file. QEMU's `file:` chardev can drop the last
# writes on SIGTERM under load; stdio is flushed with the process exit path.
# -no-reboot: triple-fault stops the VM instead of spinning.
# -accel tcg: do not fight host KVM contention when the box is busy.
# timeout returns 124 on wall-clock expiry (expected: guest hlt-loops).
set +e
timeout --signal=TERM --kill-after=2 "${SECONDS_TO_RUN}" \
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
	>"${log}" 2>&1
rc=$?
set -e

# 0 = guest exited (unexpected but log may still be valid)
# 124 = timeout killed a healthy hlt loop
if [[ "${rc}" -ne 0 && "${rc}" -ne 124 ]]; then
	echo "x86-boot-check: FAIL — qemu exited ${rc}" >&2
	echo "--- serial log ---" >&2
	cat -A "${log}" >&2 || true
	exit 1
fi

fail() {
	echo "x86-boot-check: FAIL — $1" >&2
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
