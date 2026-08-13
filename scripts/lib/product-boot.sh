#!/usr/bin/env bash
# Boot the product image under QEMU and capture its serial output.
#
# Sourced by two gates that need the same run for different questions:
#
#   * `qemu-product-boot-check.sh` — is the shipped composition healthy?
#   * `oracle-census.sh`           — how many slots did it actually occupy?
#
# It lives here rather than in either of them because the second gate was
# written against a number someone typed instead of a run (ADR-0098), and the
# cure for that is not a second copy of the QEMU invocation that can drift from
# the first.
#
# Callers must define `fail`, or accept the default below.

readonly PRODUCT_TARGET=aarch64-unknown-none-softfloat
readonly PRODUCT_OUT="target/${PRODUCT_TARGET}/release"
readonly PRODUCT_IMG="${PRODUCT_OUT}/kernel8-product.img"
readonly PRODUCT_QEMU="${QEMU:-qemu-system-aarch64}"
# Ceiling, not a fixed duration: product reaches composition + first tick
# report well under this on a quiet host (ADR-0087 shape for short boots).
readonly PRODUCT_SECONDS_LIMIT="${PRODUCT_BOOT_SECONDS:-8}"
product_clk_tck="$(getconf CLK_TCK 2>/dev/null || echo 100)"
readonly PRODUCT_CLK_TCK="${product_clk_tck}"
readonly PRODUCT_CORES_TO_BE_MEASURABLE=40

product_read_cpu_hz() {
	local stat rest
	PRODUCT_CPU_HZ=0
	[[ -r "/proc/$1/stat" ]] || return 0
	read -r stat <"/proc/$1/stat" || return 0
	rest="${stat##*) }"
	# shellcheck disable=SC2086  # deliberate word splitting into positionals
	set -- ${rest}
	PRODUCT_CPU_HZ=$((${12} + ${13}))
}

product_read_host_busy_hz() {
	local cpu user nice system rest irq softirq steal
	PRODUCT_HOST_BUSY_HZ=0
	read -r cpu user nice system rest </proc/stat || return 0
	[[ "${cpu}" == "cpu" ]] || return 0
	# shellcheck disable=SC2086  # deliberate word splitting into positionals
	set -- ${rest}
	irq="$3"
	softirq="$4"
	steal="$5"
	PRODUCT_HOST_BUSY_HZ=$((user + nice + system + irq + softirq + steal))
}

# Boot the product image, writing serial output to $1.
#
# Exit status: 0 booted, 2 skipped (no emulator on a workstation). A missing
# emulator in CI is a failure, not a skip — a green gate that never ran is
# what ADR-0096 removed.
product_boot_capture() {
	local log="$1"
	local who="${2:-product-boot}"

	if [[ ! -f "${PRODUCT_IMG}" ]]; then
		echo "${who}: building product image" >&2
		./scripts/boot/product-image.sh
	fi

	if ! command -v "${PRODUCT_QEMU}" >/dev/null; then
		if [[ "${CI:-}" == "true" ]]; then
			echo "${who}: FAIL — ${PRODUCT_QEMU} missing, and a skip is refused in CI" >&2
			echo "  ALLOW_BOOT_SKIP is for a workstation without the emulator." >&2
			echo "  In CI it would report a green gate that never ran (ADR-0096)." >&2
			exit 1
		fi
		if [[ "${ALLOW_BOOT_SKIP:-}" == "1" ]]; then
			echo "${who}: SKIPPED — ${PRODUCT_QEMU} missing, ALLOW_BOOT_SKIP set" >&2
			return 2
		fi
		echo "error: ${PRODUCT_QEMU} not found — ${who} cannot run" >&2
		exit 1
	fi

	# Store is already in the image (ADR-0029 inject). No -device loader.
	# Keep the emulator observable while it runs: an exit code from `timeout`
	# cannot distinguish a healthy ceiling from a starved guest, and treating
	# both alike would make the product gate accept an unmeasured assertion.
	local deadline started pid busy_before seconds watched_comm
	product_read_host_busy_hz
	busy_before="${PRODUCT_HOST_BUSY_HZ}"
	started=${SECONDS}
	deadline=$((started + PRODUCT_SECONDS_LIMIT))
	"${PRODUCT_QEMU}" \
		-machine raspi4b \
		-nographic \
		-serial mon:stdio \
		-d guest_errors \
		-kernel "${PRODUCT_IMG}" \
		>"${log}" 2>&1 &
	pid=$!
	watched_comm="$(cat "/proc/${pid}/comm" 2>/dev/null || echo unknown)"
	PRODUCT_RUN_CPU_HZ=0
	while ((SECONDS < deadline)) && kill -0 "${pid}" 2>/dev/null; do
		product_read_cpu_hz "${pid}"
		((PRODUCT_CPU_HZ > PRODUCT_RUN_CPU_HZ)) && PRODUCT_RUN_CPU_HZ="${PRODUCT_CPU_HZ}"
		sleep 0.2
	done
	seconds=$((SECONDS - started))
	((seconds > 0)) || seconds=1
	kill -TERM "${pid}" 2>/dev/null || true
	wait "${pid}" 2>/dev/null || true
	product_read_host_busy_hz

	PRODUCT_EMULATOR_CORES=$((PRODUCT_RUN_CPU_HZ * 100 / (PRODUCT_CLK_TCK * seconds)))
	PRODUCT_SHARE_IS_HOST_WIDE=0
	if [[ "${watched_comm}" != qemu* ]]; then
		PRODUCT_EMULATOR_CORES=$(((PRODUCT_HOST_BUSY_HZ - busy_before) * 100 / (PRODUCT_CLK_TCK * seconds)))
		PRODUCT_SHARE_IS_HOST_WIDE=1
	fi
	printf '%s: CPU budget %s.%02d cores over %ss%s\n' \
		"${who}" $((PRODUCT_EMULATOR_CORES / 100)) $((PRODUCT_EMULATOR_CORES % 100)) \
		"${seconds}" "$(if ((PRODUCT_SHARE_IS_HOST_WIDE == 1)); then echo ' (host-wide fallback)'; fi)" >&2
	if ((PRODUCT_EMULATOR_CORES < PRODUCT_CORES_TO_BE_MEASURABLE)); then
		echo "${who}: INDETERMINATE — QEMU received only ${PRODUCT_EMULATOR_CORES} hundredths of a core" >&2
		echo "  The product assertions were not judged; rerun on a host with sufficient CPU." >&2
		return 3
	fi
}
