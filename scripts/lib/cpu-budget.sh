#!/usr/bin/env bash
# How much CPU an emulator actually received, so a gate can refuse to judge.
#
# ## Why a gate needs this
#
# `timeout`'s exit code cannot tell a healthy ceiling from a starved guest.
# A QEMU that got 6 hundredths of a core did not fail the assertions — it never
# reached them, and reporting that as red is reporting a fact about the host as
# if it were a fact about the code. ADR-0087 makes the outcome three-valued:
# clean(0), FAIL(1), INDETERMINATE(3).
#
# ## Why it is here and not in `product-boot.sh`
#
# It was in `product-boot.sh`, which meant only the two product gates had it.
# `qemu-virtio-check` — the gate for the whole accepted P3 virtio path — had no
# guard at all, and could go red on a busy laptop for a reason that has nothing
# to do with virtio. Copying the loop into it would have been a second
# implementation of a measurement, which is the drift `vocabulary-sync` and
# `xrefs` exist to catch in their own domains.
#
# ## Use
#
#   source scripts/lib/cpu-budget.sh
#   cpu_budget_start                       # before launching
#   ...launch in background, $! is the pid...
#   cpu_budget_watch "${pid}" "${deadline_seconds}"
#   cpu_budget_verdict "gate-name" || exit $?      # 3 = INDETERMINATE
#
# `cpu_budget_watch` returns when the process exits or the deadline passes,
# leaving `CPU_BUDGET_SECONDS` and `CPU_BUDGET_CORES` (hundredths of a core).

cpu_budget_clk_tck="$(getconf CLK_TCK 2>/dev/null || echo 100)"
readonly CPU_BUDGET_CLK_TCK="${cpu_budget_clk_tck}"
# Below this, the run is not evidence. 40 hundredths is well under what a quiet
# host gives a single-vCPU guest and well over what a saturated one does.
readonly CPU_BUDGET_MEASURABLE=40

cpu_budget_read_pid_hz() {
	local stat rest
	CPU_BUDGET_PID_HZ=0
	[[ -r "/proc/$1/stat" ]] || return 0
	read -r stat <"/proc/$1/stat" || return 0
	rest="${stat##*) }"
	# shellcheck disable=SC2086  # deliberate word splitting into positionals
	set -- ${rest}
	CPU_BUDGET_PID_HZ=$((${12} + ${13}))
}

cpu_budget_read_host_busy_hz() {
	local cpu user nice system rest irq softirq steal
	CPU_BUDGET_HOST_BUSY_HZ=0
	read -r cpu user nice system rest </proc/stat || return 0
	[[ "${cpu}" == "cpu" ]] || return 0
	# shellcheck disable=SC2086  # deliberate word splitting into positionals
	set -- ${rest}
	irq="$3"
	softirq="$4"
	steal="$5"
	CPU_BUDGET_HOST_BUSY_HZ=$((user + nice + system + irq + softirq + steal))
}

cpu_budget_start() {
	cpu_budget_read_host_busy_hz
	CPU_BUDGET_BUSY_BEFORE="${CPU_BUDGET_HOST_BUSY_HZ}"
	CPU_BUDGET_STARTED=${SECONDS}
	CPU_BUDGET_PEAK_HZ=0
}

# Watch $1 until it exits or $2 seconds elapse, sampling its CPU time.
#
# Falls back to a host-wide measurement when the watched pid is not the
# emulator itself — a wrapper (`timeout`, a shell) accrues almost no CPU of its
# own, so sampling it would report a starved guest on a perfectly idle host.
#
# **Pass the emulator's own pid.** The fallback is a floor against a false
# INDETERMINATE, not a measurement: host-wide busy time *rises* with load, so a
# saturated machine reads as well fed and the guard reports the opposite of what
# it is for. Seen the first time this was wired into `qemu-virtio-check` behind
# a `timeout`: 7.11 cores on a host where QEMU was getting 0.81, and the number
# was mostly `ffmpeg`. The `(host-wide fallback)` suffix on the printed line is
# there so a reader can tell the two apart.
#
# **In CI the fallback is always what runs**, and no change here can fix it:
# `.github/workflows/ci.yml` wraps `qemu-system-aarch64` in `docker run`, so the
# emulator is a child of the container runtime and never appears in the process
# tree this samples. Every CI budget line therefore carries the suffix, and the
# guard there is a floor against a wholly idle runner rather than a measurement
# of what the guest received. Issue #28 is the standing item for giving the boot
# oracle guaranteed CPU; until it is paid, CI's numbers are weaker than a
# workstation's and the printed suffix is what says so.
cpu_budget_watch() {
	local pid="$1" limit="$2" deadline watched_comm
	deadline=$((CPU_BUDGET_STARTED + limit))
	watched_comm="$(cat "/proc/${pid}/comm" 2>/dev/null || echo unknown)"
	while ((SECONDS < deadline)) && kill -0 "${pid}" 2>/dev/null; do
		cpu_budget_read_pid_hz "${pid}"
		((CPU_BUDGET_PID_HZ > CPU_BUDGET_PEAK_HZ)) && CPU_BUDGET_PEAK_HZ="${CPU_BUDGET_PID_HZ}"
		sleep 0.2
	done
	CPU_BUDGET_SECONDS=$((SECONDS - CPU_BUDGET_STARTED))
	((CPU_BUDGET_SECONDS > 0)) || CPU_BUDGET_SECONDS=1
	cpu_budget_read_host_busy_hz

	CPU_BUDGET_CORES=$((CPU_BUDGET_PEAK_HZ * 100 / (CPU_BUDGET_CLK_TCK * CPU_BUDGET_SECONDS)))
	CPU_BUDGET_HOST_WIDE=0
	if [[ "${watched_comm}" != qemu* ]]; then
		CPU_BUDGET_CORES=$(((CPU_BUDGET_HOST_BUSY_HZ - CPU_BUDGET_BUSY_BEFORE) * 100 / (\
			CPU_BUDGET_CLK_TCK * CPU_BUDGET_SECONDS)))
		CPU_BUDGET_HOST_WIDE=1
	fi
}

# Print the budget and return 3 when the run was not measurable (ADR-0087).
cpu_budget_verdict() {
	local who="$1"
	printf '%s: CPU budget %s.%02d cores over %ss%s\n' \
		"${who}" $((CPU_BUDGET_CORES / 100)) $((CPU_BUDGET_CORES % 100)) \
		"${CPU_BUDGET_SECONDS}" \
		"$(if ((CPU_BUDGET_HOST_WIDE == 1)); then echo ' (host-wide fallback)'; fi)" >&2
	if ((CPU_BUDGET_CORES < CPU_BUDGET_MEASURABLE)); then
		echo "${who}: INDETERMINATE — QEMU received only ${CPU_BUDGET_CORES} hundredths of a core" >&2
		echo "  The assertions were not judged; rerun on a host with sufficient CPU." >&2
		return 3
	fi
	return 0
}
