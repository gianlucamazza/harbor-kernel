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
# dropped before counting: the remaining fields start at `state`, so positional
# `n` is `proc(5)` field `n + 2`. cutime is field 16 and cstime is field 17,
# which is `${14}` and `${15}` — not `${13}` and `${14}`, which are the
# *shell's own* stime and the children's utime. Reading those two summed a
# number that is always ~0 with one that happened to be most of TCG's cost, so
# the total looked plausible on an idle laptop and told the truth nowhere else.
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
	CHILD_CPU_HZ=$((${14} + ${15}))
}

read_child_cpu_hz
cpu_before="${CHILD_CPU_HZ}"

# ADR-0066: a scratch SD card image, partitioned exactly as the host tool
# partitions a real card (one MBR entry of type 0x7f). Sparse, so the 8 GiB
# the QEMU SD model wants (power-of-two) costs kilobytes of real disk. Two
# boots share it: the second is the emulated power cycle — the QEMU process
# is gone in between, so DRAM state is genuinely lost and only the media
# can carry the boot counter across.
sd_img="$(mktemp)"
trap 'rm -f "${log}" "${sd_img}"' EXIT
truncate -s 8G "${sd_img}"
printf 'label: dos\n,2048,7f\n' | sfdisk -q "${sd_img}" >/dev/null

# `timeout` kills a healthy run, so its exit status says nothing; the log is
# the oracle. `|| true` keeps `set -e` from ending the script before we look.
# Oracle path: no external store — builtin beacon+mute (ADR-0027 fallback).
# ADR-0073: QEMU raspi4b does not synthesise an FDT for -kernel; pin the
# same fixture the host fdt tests use so the first boot exercises parse.
DTB_FIXTURE="${DTB_FIXTURE:-crates/kernel-core/tests/fixtures/bcm2711-rpi-4-b.dtb}"
if [[ ! -f "${DTB_FIXTURE}" ]]; then
	echo "error: DTB fixture not found: ${DTB_FIXTURE}" >&2
	exit 1
fi

run_boot() {
	local seconds="$1"
	shift
	: >"${log}"
	# raspi4b requires min 4 CPUs; ADR-0070 needs secondaries present to unpark.
	timeout "${seconds}" "${QEMU}" \
		-M "${QEMU_MACHINE}" -smp 4 -kernel "${IMG}" \
		-serial mon:stdio -display none "$@" </dev/null >"${log}" 2>&1 || true
}

# Boot 1: with -dtb fixture (FDT parse path).
run_boot "${SECONDS_TO_RUN}" -dtb "${DTB_FIXTURE}" -drive "if=sd,format=raw,file=${sd_img}"

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

# Below this the reading is not a small share, it is a failed measurement. A
# guest that boots through to steady state costs the host seconds of CPU; an
# accounting that reports a hundredth of a core for it is not describing the
# emulator. Seen on GitHub's runners, where the number read 0.00 for a run the
# serial log shows going all the way to `ticks=140` — and 0.00 silently
# satisfies every "starved?" test, which would turn the indeterminate verdict
# into a rubber stamp on the one host where it fires.
CORES_TO_BE_CREDIBLE=10 # 0.10 core

# The environment reading belongs on every verdict, not only the ones that
# blame the environment: a FAIL whose host share is unknown cannot be told from
# a FAIL on a host that had the CPU, and the difference is the whole question.
print_cpu_share() {
	printf '  emulator share: %s.%02d cores of host CPU over %ss (%s)\n' \
		$((emulator_cores / 100)) $((emulator_cores % 100)) "${SECONDS_TO_RUN}" \
		"$(if [[ "${emulator_cores}" -lt "${CORES_TO_BE_CREDIBLE}" ]]; then
			echo "not credible — treat as unmeasured"
		elif [[ "${emulator_cores}" -lt "${CORES_TO_BE_MEASURABLE}" ]]; then
			echo "starved"
		else echo "enough to meet deadlines"; fi)" >&2
}

fail() {
	echo "boot-check: FAIL — $1" >&2
	print_cpu_share
	echo "--- serial log ---" >&2
	cat "${log}" >&2
	exit 1
}

# Neither pass nor fail: the run did not establish its claim. Exit code 3 so a
# caller can tell it from both, and non-zero so nothing downstream mistakes an
# unestablished claim for a verified one.
indeterminate() {
	echo "boot-check: INDETERMINATE — $1" >&2
	print_cpu_share
	echo "  A deadline cannot be demanded of an emulator that did not get a core," >&2
	echo "  nor of one whose share this host will not report. Re-run on an idle" >&2
	echo "  machine that accounts for its children." >&2
	echo "  This is not a kernel failure, and it is not a pass." >&2
	exit 3
}

# The verdict for an assertion that can only be met by running fast enough:
# timer deadlines, and every rotation or exit an oracle waits a bounded number
# of yields for. `scripts/lib/boot-oracle.sh` decides *what* a sound boot says;
# this decides who is to blame when a host could not host the question.
#
# Silicon has no such excuse, which is why `hw-transcript-check.sh` shares the
# assertions and not this function (ADR-0087).
deadline_verdict() {
	if [[ "${emulator_cores}" -lt "${CORES_TO_BE_CREDIBLE}" ]]; then
		indeterminate "$1, on a host that did not report what the emulator got"
	fi
	if [[ "${emulator_cores}" -lt "${CORES_TO_BE_MEASURABLE}" ]]; then
		indeterminate "$1, on a host that starved the emulator"
	fi
	fail "$1, and the emulator had the CPU to meet it"
}

# The assertions themselves live in one place, shared with the hardware
# transcript check — see the header of scripts/lib/boot-oracle.sh.
# shellcheck source=scripts/lib/boot-oracle.sh
source "$(dirname "$0")/../lib/boot-oracle.sh"

on_timer_missed() {
	deadline_verdict "timer deadlines expired unserviced"
}

# Rotation and exit under preemption: same shape as the timer, same verdict.
on_deadline_missed() {
	deadline_verdict "$1"
}

# Boot 1 on fresh media: the durable window exists and has never been
# written, so the ADR-0066 line must read from=Fresh boot=1.
DURABLE_MEDIA_EXPECT=fresh assert_boot_oracle
ticks_first="$(grep -ac 'ticks=' "${log}")"

# Boot 2: DTB-less power cycle (degraded discover: unknown (no dtb) + durable).
# DRAM is gone; only the card can carry the counter across.
run_boot "${SECONDS_TO_RUN}" -drive "if=sd,format=raw,file=${sd_img}"
read_child_cpu_hz
emulator_cores=$(((CHILD_CPU_HZ - cpu_before) * 100 / (clk_tck * 2 * SECONDS_TO_RUN)))
DURABLE_MEDIA_EXPECT=previous assert_boot_oracle
grep -qaE 'durable-media: boot=2 from=Previous part=0x7f slot=(A|B) seq=1' "${log}" ||
	fail "the second boot did not read the first boot's committed state (ADR-0066)"
ticks_second="$(grep -ac 'ticks=' "${log}")"

# No-card phase: the empty-slot path must stay honest, and only QEMU can
# hold it to that — silicon always boots from a card. Both SDHCI hosts
# exist in the machine model, so the honest line is `no-card` (CMD8
# unanswered), not `absent` (no controller). A short run suffices: the
# line prints during bring-up, well before steady state, so this phase
# asserts it alone rather than the whole oracle.
run_boot 8
grep -qa 'durable-media: no-card (no SDHC/SDXC answered)' "${log}" ||
	fail "without a card the boot did not report the honest no-card line (ADR-0066)"

printf 'boot-check: clean (%s + %s tick reports over two media boots, emulator had %s.%02d cores)\n' \
	"${ticks_first}" "${ticks_second}" \
	$((emulator_cores / 100)) $((emulator_cores % 100))
