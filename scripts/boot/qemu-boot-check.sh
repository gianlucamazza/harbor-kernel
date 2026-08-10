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

# CPU time a live process has consumed, in USER_HZ: utime + stime out of
# `/proc/<pid>/stat`. No `/usr/bin/time`, so the measurement adds no dependency
# to a script `make check` always runs.
#
# The comm field can contain spaces, so everything up to the last `)` is
# dropped before counting: the remaining fields start at `state`, so positional
# `n` is `proc(5)` field `n + 2`, which puts utime and stime at `${12}` and
# `${13}`.
#
# Read from the emulator's own entry while it runs, not from this shell's
# cutime after reaping it. Two versions of the reap-side reading were wrong in
# two different ways — one summed the shell's stime with the children's utime,
# and even corrected it reported 0.10 s for a full 15-second boot inside CI's
# container. A number that is only right on the machine it was written on is
# not a measurement, and this gate spends it on a verdict. The process's own
# accounting is the source, and it is the same on every host.
#
# The result goes into a global rather than being echoed: `$(…)` is a subshell,
# and one more layer between the reading and the reader is one more place for
# it to be about the wrong process.
read_cpu_hz_of() {
	local stat rest
	CPU_HZ=0
	[[ -r "/proc/$1/stat" ]] || return 0
	read -r stat <"/proc/$1/stat" || return 0
	rest="${stat##*) }"
	# shellcheck disable=SC2086  # deliberate word splitting into positionals
	set -- ${rest}
	CPU_HZ=$((${12} + ${13}))
}

# Busy CPU across the whole host, in USER_HZ: everything in `/proc/stat`'s
# summary line except idle and iowait.
#
# The fallback for the case the reading above cannot cover: CI does not run
# QEMU as a child at all. It installs a wrapper that `exec docker run`s an Arch
# container, so the process this script starts is a docker *client* — the
# emulator is a child of the daemon, in another namespace, and no amount of
# sampling our own pid will ever see it. That is why both earlier attempts read
# essentially zero there.
#
# Containers share the host kernel, so the emulator's cycles do land in
# `/proc/stat` even when they land nowhere this script can attribute. The
# number is therefore about the *host*, not the emulator: it answers "was there
# CPU being spent while this boot ran", which is the only question the
# indeterminate verdict actually asks. It is labelled as such wherever it is
# printed, because a share that includes a noisy neighbour would read as a
# generous one.
read_host_busy_hz() {
	local cpu user nice system irq softirq steal rest
	# `idle` and `iowait` are read into `rest` on purpose: busy is everything
	# else, and naming them would only invite someone to add them to the sum.
	HOST_BUSY_HZ=0
	read -r cpu user nice system rest </proc/stat || return 0
	[[ "${cpu}" == "cpu" ]] || return 0
	# `rest` is idle iowait irq softirq steal guest guest_nice.
	# shellcheck disable=SC2086  # deliberate word splitting into positionals
	set -- ${rest}
	irq="$3"
	softirq="$4"
	steal="$5"
	HOST_BUSY_HZ=$((user + nice + system + irq + softirq + steal))
}

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

# How much host CPU the emulator actually got, in hundredths of a core per
# second of wall time.
#
# The threshold is measured, not guessed. Walking this gate down a `CPUQuota`
# ladder on one idle machine, same image, same 15s window (2026-08-10):
#
#   2.03 cores  unthrottled          clean
#   0.92        1-core slice cap     clean
#   0.54        60% quota            clean
#   0.37        40% quota            clean
#   0.23        25% quota            FAIL — task output not interleaved
#   0.13        15% quota            FAIL — agent bytes arrived after its report
#
# So the oracle set holds on roughly a third of a core and breaks below a
# quarter. One whole core — the first version of this bar — would have called
# every deadline failure on a laptop whose build shim caps the slice at 1.00
# "indeterminate", which is the same as retiring the assertions on the machine
# they run on most. 0.35 sits just under the last clean rung.
#
# Guest tick reports were tried first and are the wrong signal: TCG drives the
# guest timer from wall-clock time, so the count tracks how long the run lasted
# rather than how much CPU it received. At a 20% quota the guest still reported
# 12 ticks while running on a fifth of one core.
clk_tck="$(getconf CLK_TCK 2>/dev/null || echo 100)"
CORES_TO_BE_MEASURABLE=35 # 0.35 cores, one rung under the last clean run

# Below this the reading is not a small share, it is a failed measurement. A
# guest that boots through to steady state costs the host seconds of CPU; an
# accounting that reports a hundredth of a core for it is not describing the
# emulator, and 0.00 silently satisfies every "starved?" test — which would
# turn the indeterminate verdict into a rubber stamp on exactly the host where
# it fires. Kept as a floor even though the reading now comes from the
# emulator's own `/proc` entry: the day it reads zero again, the gate has to
# say "I could not measure this" rather than "the host was busy".
CORES_TO_BE_CREDIBLE=10 # 0.10 core

# Every boot's share, in order, for the closing line. A gate that reports the
# environment it ran in only when it fails leaves nothing to compare against
# the day it starts failing.
cores_seen=()

# Run one boot for `seconds` and leave its serial output in `${log}`, the CPU it
# consumed in `RUN_CPU_HZ`, and its share of a core in `emulator_cores`.
#
# The emulator is backgrounded and timed here rather than wrapped in
# `timeout(1)`, because the CPU reading has to be taken while the process still
# exists: `/proc/<pid>/stat` is gone the instant it exits, and the reap-side
# number this used to rely on is not trustworthy on every host (see
# `read_cpu_hz_of`). Same deadline, same SIGTERM, same "the log is the oracle,
# not the exit status" contract as before.
run_boot() {
	local seconds="$1"
	shift
	: >"${log}"
	local deadline=$((SECONDS + seconds)) pid busy_before
	read_host_busy_hz
	busy_before="${HOST_BUSY_HZ}"
	# raspi4b requires min 4 CPUs; ADR-0070 needs secondaries present to unpark.
	"${QEMU}" \
		-M "${QEMU_MACHINE}" -smp 4 -kernel "${IMG}" \
		-serial mon:stdio -display none "$@" </dev/null >"${log}" 2>&1 &
	pid=$!
	RUN_CPU_HZ=0
	while ((SECONDS < deadline)) && kill -0 "${pid}" 2>/dev/null; do
		read_cpu_hz_of "${pid}"
		# The last reading before exit, not the first: a boot that ends early
		# still reports what it burned. Zero only if it was never observable.
		((CPU_HZ > RUN_CPU_HZ)) && RUN_CPU_HZ="${CPU_HZ}"
		sleep 0.2
	done
	kill -TERM "${pid}" 2>/dev/null || true
	wait "${pid}" 2>/dev/null || true
	read_host_busy_hz

	emulator_cores=$((RUN_CPU_HZ * 100 / (clk_tck * seconds)))
	share_is_host_wide=0
	if ((emulator_cores < CORES_TO_BE_CREDIBLE)); then
		# The emulator was not ours to watch. Fall back to what the host as a
		# whole burned, and say so — never silently, and never the other way
		# round: a process we *can* see is always the better answer.
		emulator_cores=$(((HOST_BUSY_HZ - busy_before) * 100 / (clk_tck * seconds)))
		share_is_host_wide=1
	fi
	# `*` marks a host-wide reading, expanded in the closing line.
	local mark=""
	((share_is_host_wide == 1)) && mark="*"
	cores_seen+=("$(printf '%s.%02d%s' $((emulator_cores / 100)) $((emulator_cores % 100)) "${mark}")")
}

# Boot 1: with -dtb fixture (FDT parse path).
run_boot "${SECONDS_TO_RUN}" -dtb "${DTB_FIXTURE}" -drive "if=sd,format=raw,file=${sd_img}"

# The environment reading belongs on every verdict, not only the ones that
# blame the environment: a FAIL whose host share is unknown cannot be told from
# a FAIL on a host that had the CPU, and the difference is the whole question.
print_cpu_share() {
	printf '  %s: %s.%02d cores over %ss (%s)\n' \
		"$(if ((share_is_host_wide == 1)); then
			echo "host-wide CPU, the emulator is not this shell's child"
		else echo "emulator share"; fi)" \
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
	echo "  machine — and note that a build shim or thermal governor that puts" >&2
	echo "  the run in a throttled slice starves it just as effectively as a" >&2
	echo "  busy one." >&2
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
# This boot's own share, not a running average of both: the verdict below is
# about the run it is judging.
emulator_cores=$((RUN_CPU_HZ * 100 / (clk_tck * SECONDS_TO_RUN)))
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

printf 'boot-check: clean (%s + %s tick reports over two media boots, %s cores per boot%s)\n' \
	"${ticks_first}" "${ticks_second}" \
	"$(
		IFS='/'
		echo "${cores_seen[*]}"
	)" \
	"$(if ((share_is_host_wide == 1)); then echo " — * is host-wide, not the emulator's own"; fi)"
