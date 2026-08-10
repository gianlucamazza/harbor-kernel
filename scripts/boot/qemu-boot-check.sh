#!/usr/bin/env bash
# Boot the kernel under QEMU and assert the run reached a healthy steady state.
#
# One of two runners of the boot oracle: the single definition of "the boot
# is sound" lives in scripts/lib/boot-oracle.sh, shared verbatim with the
# hardware transcript check. This script owns what is QEMU's alone — the
# emulator invocation, the CPU-starvation measurement, and the indeterminate
# verdict a starved host earns instead of a red.
set -euo pipefail

IMG="${1:?usage: $0 <kernel8.img> [ceiling-seconds]}"
# A ceiling on each boot, not a duration: a run ends when the guest has printed
# what the oracle needs of it (ADR-0087).
SECONDS_TO_RUN="${2:-45}"
last_run_seconds="${SECONDS_TO_RUN}"
watched_comm=unknown
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
# ladder on one idle machine, same image, same 15s window (2026-08-10, after
# the ADR-0087 cross-core waits were put on the guest clock):
#
#   2.14 cores  unthrottled     clean
#   0.23        25% quota       marginal — a console line goes missing mid-run
#   0.13        15% quota       the run no longer gets far enough
#   0.08        10% quota       below the credible floor as well
#
# The bar is 0.40 rather than 0.25: what breaks first around a quarter of a
# core is not an assertion timing out but the serial console dropping a line
# (`task-a 1`, with `task-a 2` onwards present — not truncation), and a gate
# should sit clear of the edge it was calibrated against, not on it.
#
# It is worth recording what the same ladder said *before* the oracles stopped
# counting cross-core waits in yields: broken already at 0.23 with the failure
# blamed on rotation. Half of what looked like the emulator needing CPU was the
# oracles needing this host'"'"'s scheduler to be fair.
#
# Guest tick reports were tried first as the starvation signal and are the
# wrong one: TCG drives the guest timer from wall-clock time, so the count
# tracks how long the run lasted rather than how much CPU it received. At a 20%
# quota the guest still reported 12 ticks while running on a fifth of one core.
clk_tck="$(getconf CLK_TCK 2>/dev/null || echo 100)"
CORES_TO_BE_MEASURABLE=40 # 0.40 cores, clear of the 0.23 rung that frayed

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

# Guest time a boot must spend in steady state before the run has done its job,
# in tick reports (one per ten ticks, so one per second of guest time).
#
# The old fixed 15s window bought about fourteen of these, and they are not
# incidental: the assertions about what must *not* appear — an unhandled
# interrupt, a panic, an invariant drifting off zero — are only as strong as
# the time the kernel was left running. Stopping at the first tick would have
# turned a fourteen-second soak into a one-second one while every assertion
# still passed, which is the quietest way to lose a gate.
#
# Ten is that soak, minus the margin the old window needed for slow hosts —
# and because TCG drives the guest timer from wall-clock time, ten reports cost
# about ten seconds anywhere.
STEADY_STATE_REPORTS=10

# The boot has printed everything the oracle needs from it: the last stage it
# waits for, plus the soak above.
oracle_boot_done() {
	grep -qa 'smp: steal ok' "${log}" &&
		(($(grep -ac 'ticks=' "${log}") >= STEADY_STATE_REPORTS))
}

# The no-card phase asserts one line, printed during bring-up.
nocard_boot_done() {
	grep -qa 'durable-media: no-card' "${log}"
}

# Run one boot until `done_when` holds or `seconds` elapse, and leave its serial
# output in `${log}`, the CPU it consumed in `RUN_CPU_HZ`, and its share of a
# core in `emulator_cores`.
#
# `seconds` is a **ceiling**, not a duration (ADR-0087). A fixed window is a
# host-sensitive way to ask a fixed question: the same image needs a different
# amount of wall time to print the same lines on a workstation, on a laptop in
# a throttled slice, and inside CI's container, and the run that fell just
# short of the tail looked exactly like a kernel that never printed it. Stopping
# on the boot's own output asks the guest, and costs the fast hosts nothing.
#
# The emulator is backgrounded and timed here rather than wrapped in
# `timeout(1)`, because the CPU reading has to be taken while the process still
# exists: `/proc/<pid>/stat` is gone the instant it exits, and the reap-side
# number this used to rely on is not trustworthy on every host (see
# `read_cpu_hz_of`). Same SIGTERM, same "the log is the oracle, not the exit
# status" contract as before.
run_boot() {
	local seconds="$1" done_when="$2"
	shift 2
	: >"${log}"
	local deadline=$((SECONDS + seconds)) started=${SECONDS} pid busy_before
	read_host_busy_hz
	busy_before="${HOST_BUSY_HZ}"
	# raspi4b requires min 4 CPUs; ADR-0070 needs secondaries present to unpark.
	"${QEMU}" \
		-M "${QEMU_MACHINE}" -smp 4 -kernel "${IMG}" \
		-serial mon:stdio -display none "$@" </dev/null >"${log}" 2>&1 &
	pid=$!
	RUN_CPU_HZ=0
	watched_comm="$(cat "/proc/${pid}/comm" 2>/dev/null || echo unknown)"
	while ((SECONDS < deadline)) && kill -0 "${pid}" 2>/dev/null; do
		read_cpu_hz_of "${pid}"
		# The last reading before exit, not the first: a boot that ends early
		# still reports what it burned. Zero only if it was never observable.
		((CPU_HZ > RUN_CPU_HZ)) && RUN_CPU_HZ="${CPU_HZ}"
		"${done_when}" && break
		sleep 0.2
	done
	seconds=$((SECONDS - started))
	((seconds > 0)) || seconds=1
	last_run_seconds="${seconds}"
	kill -TERM "${pid}" 2>/dev/null || true
	wait "${pid}" 2>/dev/null || true
	read_host_busy_hz

	emulator_cores=$((RUN_CPU_HZ * 100 / (clk_tck * seconds)))
	share_is_host_wide=0
	if [[ "${watched_comm}" != qemu* ]]; then
		# What we started is not the emulator. Ask the process itself rather
		# than inferring it from a small number: CI's wrapper `exec docker
		# run`s the emulator in a container, and the client left behind reads
		# `comm=docker` while still burning 0.06s of its own relaying serial —
		# enough that "is the reading zero?" answered no and the fallback
		# never fired. The name answers exactly, and answers on the first
		# sample.
		#
		# Never the other way round: a process we *can* see is always the
		# better answer, and the fallback says so wherever it is printed.
		emulator_cores=$(((HOST_BUSY_HZ - busy_before) * 100 / (clk_tck * seconds)))
		share_is_host_wide=1
	fi
	# `*` marks a host-wide reading, expanded in the closing line.
	local mark=""
	((share_is_host_wide == 1)) && mark="*"
	cores_seen+=("$(printf '%s.%02d%s' $((emulator_cores / 100)) $((emulator_cores % 100)) "${mark}")")
}

# Boot 1: with -dtb fixture (FDT parse path).
run_boot "${SECONDS_TO_RUN}" oracle_boot_done -dtb "${DTB_FIXTURE}" -drive "if=sd,format=raw,file=${sd_img}"

# The environment reading belongs on every verdict, not only the ones that
# blame the environment: a FAIL whose host share is unknown cannot be told from
# a FAIL on a host that had the CPU, and the difference is the whole question.
print_cpu_share() {
	printf '  %s: %s.%02d cores over %ss (%s)\n' \
		"$(if ((share_is_host_wide == 1)); then
			echo "host-wide CPU, the emulator is not this shell's child"
		else echo "emulator share"; fi)" \
		$((emulator_cores / 100)) $((emulator_cores % 100)) "${last_run_seconds}" \
		"$(if [[ "${emulator_cores}" -lt "${CORES_TO_BE_CREDIBLE}" ]]; then
			echo "not credible — treat as unmeasured"
		elif [[ "${emulator_cores}" -lt "${CORES_TO_BE_MEASURABLE}" ]]; then
			echo "starved"
		else echo "enough to meet deadlines"; fi)" >&2
}

# The runner's verdict on a failed assertion (ADR-0087).
#
# One rule, applied to every assertion rather than to a hand-picked list of the
# timing-shaped ones: on a host that did not give the emulator enough CPU, *no*
# assertion is attributable to the kernel. A starved boot does not fail the
# rotation claim and pass the rest — it simply does not get far enough, and
# which line goes missing first is a property of the host. Classifying
# assertions as "structural" or "deadline" was tried and is whack-a-mole: the
# quota ladder found a new arrival-shaped one on every rung (`task-a` interleave,
# then agent bytes before the report, then pl011 RX loopback, then the received
# payload). They were never a class; they were the assertions that happened to
# come first.
#
# Above the bar the verdict is a plain red, exactly as before. Below it there is
# no verdict to give, and saying so is the only honest answer.
#
# Silicon has no such excuse, which is why `hw-transcript-check.sh` shares the
# assertions and keeps its own `fail`.
fail() {
	if [[ "${emulator_cores}" -lt "${CORES_TO_BE_MEASURABLE}" ]]; then
		indeterminate "$1"
	fi
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
	echo "  The boot did not get the CPU to be judged on, so nothing it did or" >&2
	echo "  did not print is the kernel's. Re-run on an idle machine — and note" >&2
	echo "  that a build shim or thermal governor that puts the run in a" >&2
	echo "  throttled slice starves it just as effectively as a busy one." >&2
	echo "  This is not a kernel failure, and it is not a pass." >&2
	exit 3
}

# The assertions themselves live in one place, shared with the hardware
# transcript check — see the header of scripts/lib/boot-oracle.sh.
# shellcheck source=scripts/lib/boot-oracle.sh
source "$(dirname "$0")/../lib/boot-oracle.sh"

on_timer_missed() {
	# `fail` already asks whether this host could be judged at all.
	fail "timer deadlines expired unserviced"
}

# Boot 1 on fresh media: the durable window exists and has never been
# written, so the ADR-0066 line must read from=Fresh boot=1.
DURABLE_MEDIA_EXPECT=fresh assert_boot_oracle
ticks_first="$(grep -ac 'ticks=' "${log}")"

# Boot 2: DTB-less power cycle (degraded discover: unknown (no dtb) + durable).
# DRAM is gone; only the card can carry the counter across.
run_boot "${SECONDS_TO_RUN}" oracle_boot_done -drive "if=sd,format=raw,file=${sd_img}"
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
run_boot 8 nocard_boot_done
grep -qa 'durable-media: no-card (no SDHC/SDXC answered)' "${log}" ||
	fail "without a card the boot did not report the honest no-card line (ADR-0066)"

printf 'boot-check: clean (%s + %s tick reports over two media boots, %s cores per boot%s)\n' \
	"${ticks_first}" "${ticks_second}" \
	"$(
		IFS='/'
		echo "${cores_seen[*]}"
	)" \
	"$(if ((share_is_host_wide == 1)); then echo " — * is host-wide, not the emulator's own"; fi)"
