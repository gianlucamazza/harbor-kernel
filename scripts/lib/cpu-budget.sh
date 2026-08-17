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

# Every pid whose `comm` names a QEMU system emulator, whoever its parent is.
#
# The point is the container case: `docker run` puts the emulator under the
# container runtime, so it is nowhere in the caller's process tree — but it is
# still an ordinary process in the host's pid namespace, visible in /proc like
# any other. Matching on `comm` finds it there without knowing anything about
# cgroup layout, which varies by driver and by distribution.
#
# `comm` is truncated to 15 characters, so `qemu-system-aarch64` reads as
# `qemu-system-aar`. The prefix is what is matched.
cpu_budget_emulator_pids() {
	local stat comm
	for stat in /proc/[0-9]*/stat; do
		[[ -r "${stat}" ]] || continue
		read -r _ comm _ <"${stat}" 2>/dev/null || continue
		[[ "${comm}" == "(qemu-system"* ]] && printf '%s\n' "${stat%/stat}"
	done
}

cpu_budget_start() {
	cpu_budget_read_host_busy_hz
	CPU_BUDGET_BUSY_BEFORE="${CPU_BUDGET_HOST_BUSY_HZ}"
	CPU_BUDGET_STARTED=${SECONDS}
	CPU_BUDGET_PEAK_HZ=0
	# Emulators already running before this gate started are somebody else's,
	# and counting them would let a neighbouring QEMU vouch for this one.
	CPU_BUDGET_PRE_EXISTING=" $(cpu_budget_emulator_pids | sed 's|/proc/||' | tr '\n' ' ')"
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
# The fallback used to be *all* CI ever ran, because
# `.github/workflows/ci.yml` wraps `qemu-system-aarch64` in `docker run` and the
# emulator is therefore a child of the container runtime, nowhere in the
# caller's process tree. Every CI budget line carried the suffix, which meant
# issue #28's own detector could not distinguish "the runner gave QEMU 2.3
# cores" from "the runner was busy with something else".
#
# So the emulator is now found by identity rather than by parentage — see
# `cpu_budget_emulator_pids`. A container does not hide a process from the
# host's /proc, only from the caller's descendants. The fallback stays for the
# case where no emulator can be found at all, which is a real possibility
# (a pid namespace of its own, a differently-named binary) and is exactly when
# a floor rather than a measurement is the honest thing to have.
#
# The identity scan has its own limit, and it is the mirror of the old one: a
# second, unrelated QEMU started during the window is counted in. Pre-existing
# ones are excluded at `cpu_budget_start`, so this needs a neighbour that starts
# *inside* the gate. In this repository the gates are sequential, so it does not
# arise; on a shared machine it would inflate the number rather than deflate it,
# which fails toward a false green and is worth knowing.
cpu_budget_watch() {
	local pid="$1" limit="$2" deadline dir emulator sum found
	deadline=$((CPU_BUDGET_STARTED + limit))
	CPU_BUDGET_SAW_EMULATOR=0
	while ((SECONDS < deadline)) && kill -0 "${pid}" 2>/dev/null; do
		# The launched pid first: on a workstation it *is* the emulator, and
		# reading one file beats scanning /proc.
		cpu_budget_read_pid_hz "${pid}"
		sum="${CPU_BUDGET_PID_HZ}"
		found=0
		if [[ "$(cat "/proc/${pid}/comm" 2>/dev/null)" == qemu-system* ]]; then
			found=1
		else
			# Otherwise look for the emulator by identity. This is the container
			# case, and it is why the host-wide fallback below is now a last
			# resort rather than the normal CI path.
			sum=0
			for dir in $(cpu_budget_emulator_pids); do
				emulator="${dir#/proc/}"
				[[ "${CPU_BUDGET_PRE_EXISTING}" == *" ${emulator} "* ]] && continue
				cpu_budget_read_pid_hz "${emulator}"
				sum=$((sum + CPU_BUDGET_PID_HZ))
				found=1
			done
		fi
		((found == 1)) && CPU_BUDGET_SAW_EMULATOR=1
		((sum > CPU_BUDGET_PEAK_HZ)) && CPU_BUDGET_PEAK_HZ="${sum}"
		sleep 0.2
	done
	CPU_BUDGET_SECONDS=$((SECONDS - CPU_BUDGET_STARTED))
	((CPU_BUDGET_SECONDS > 0)) || CPU_BUDGET_SECONDS=1
	cpu_budget_read_host_busy_hz

	CPU_BUDGET_CORES=$((CPU_BUDGET_PEAK_HZ * 100 / (CPU_BUDGET_CLK_TCK * CPU_BUDGET_SECONDS)))
	CPU_BUDGET_HOST_WIDE=0
	if ((CPU_BUDGET_SAW_EMULATOR == 0)); then
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
