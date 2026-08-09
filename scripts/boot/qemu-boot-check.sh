#!/usr/bin/env bash
# Boot the kernel under QEMU and assert the run reached a healthy steady state.
#
# One of two runners of the boot oracle: the single definition of "the boot
# is sound" lives in scripts/lib/boot-oracle.sh, shared verbatim with the
# hardware transcript check. This script owns what is QEMU's alone — the
# emulator invocation, the CPU-starvation measurement, and the indeterminate
# verdict a starved host earns instead of a red.
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
# agent can send any byte over a console endpoint (`SYS_SEND`, tag 0), and one NUL is enough for `grep` to
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

# The assertions themselves live in one place, shared with the hardware
# transcript check — see the header of scripts/lib/boot-oracle.sh.
# shellcheck source=scripts/lib/boot-oracle.sh
source "$(dirname "$0")/../lib/boot-oracle.sh"

on_timer_missed() {
	if [[ "${emulator_cores}" -lt "${CORES_TO_BE_MEASURABLE}" ]]; then
		indeterminate "the timer missed deadlines on a host that starved the emulator"
	fi
	fail "timer deadlines expired unserviced, and the emulator had the CPU to meet them"
}

assert_boot_oracle

printf 'boot-check: clean (%s tick reports, emulator had %s.%02d cores)\n' \
	"$(grep -ac 'ticks=' "${log}")" $((emulator_cores / 100)) $((emulator_cores % 100))
