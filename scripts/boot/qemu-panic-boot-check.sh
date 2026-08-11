#!/usr/bin/env bash
# Boot the deliberate-panic image and assert the panic path works (ADR-0093).
#
# Every other gate asserts that no boot printed `PANIC` — negative evidence,
# and the only evidence `src/panic.rs` has ever had (excellence F-24,
# ADR-0049). This one boots an image that faults on purpose and checks that the
# diagnostic which runs when everything else has failed does what it claims.
#
# ## Why this is not `qemu-boot-check.sh` with a flag
#
# `scripts/lib/boot-oracle.sh` fails any log containing `PANIC`, correctly, and
# relaxing that to accommodate this image would blunt the gate that matters on
# every ordinary boot. So this is its own runner with its own assertions, in
# the shape of `qemu-product-boot-check.sh`: layered, ceiling-not-duration
# (ADR-0087), done when the guest says so.
#
# ## The fault
#
# A write to a task stack's guard page. It reaches the branch with the most
# policy in it ("task-stack guard page, i.e. stack overflow") and is the first
# positive evidence that the ADR-0005 guard faults at all — every other gate
# only observes that nothing fell into it.
#
# The probe announces the address **before** it writes. Without that line,
# "the kernel did not panic" and "the probe never ran" are the same log.
set -euo pipefail

cd "$(dirname "$0")/../.."

readonly TARGET=aarch64-unknown-none-softfloat
readonly OUT="target/${TARGET}/release"
readonly IMG="${1:-${OUT}/kernel8-panic.img}"
readonly QEMU="${QEMU:-qemu-system-aarch64}"
# Ceiling, not a duration: this boot dies early on purpose, so it reaches
# `*** halt ***` well inside this on any host that can run the emulator.
readonly SECONDS_LIMIT="${PANIC_BOOT_SECONDS:-20}"

if [[ ! -f "${IMG}" ]]; then
	echo "error: ${IMG} not found — run 'make panic-check'" >&2
	exit 1
fi

if ! command -v "${QEMU}" >/dev/null; then
	if [[ "${CI:-}" == "true" ]]; then
		echo "panic-check: FAIL — ${QEMU} missing, and a skip is refused in CI" >&2
		echo "  ALLOW_BOOT_SKIP is for a workstation without the emulator." >&2
		echo "  In CI it would report a green gate that never ran (ADR-0096)." >&2
		exit 1
	fi
	if [[ "${ALLOW_BOOT_SKIP:-}" == "1" ]]; then
		echo "panic-check: SKIPPED — ${QEMU} missing, ALLOW_BOOT_SKIP set" >&2
		exit 0
	fi
	echo "error: ${QEMU} not found — panic boot check cannot run" >&2
	exit 1
fi

log="$(mktemp)"
trap 'rm -f "${log}"' EXIT

# Stop on the guest's own last word rather than after a fixed sleep: the image
# parks a core after `*** halt ***`, so waiting longer only wastes the ceiling.
"${QEMU}" -machine raspi4b -smp 4 -nographic -serial mon:stdio \
	-kernel "${IMG}" </dev/null >"${log}" 2>&1 &
pid=$!
deadline=$((SECONDS + SECONDS_LIMIT))
while ((SECONDS < deadline)) && kill -0 "${pid}" 2>/dev/null; do
	grep -qa '\*\*\* halt \*\*\*' "${log}" && break
	sleep 0.2
done
kill -TERM "${pid}" 2>/dev/null || true
wait "${pid}" 2>/dev/null || true

fail() {
	echo "panic-check: FAIL — $1" >&2
	echo "--- serial log ---" >&2
	cat "${log}" >&2 || true
	exit 1
}

# ---------------------------------------------------------------------------
# 1. Still a real boot — the map has to exist for `describe_address` to answer
# ---------------------------------------------------------------------------
grep -qa 'Harbor: hello' "${log}" || fail "the panic image did not boot"
grep -qa 'MMU on' "${log}" || fail "the kernel map did not activate"

# ---------------------------------------------------------------------------
# 2. The probe ran, and said where it was about to write
# ---------------------------------------------------------------------------
announce="$(grep -aoE '^panic-probe: stack guard at 0x[0-9a-f]{4,16}, writing' "${log}" || true)"
[[ -n "${announce}" ]] ||
	fail "no panic-probe announce line — the probe did not run, and a missing panic would look identical"

# ---------------------------------------------------------------------------
# 3. The handler was reached — this is the positive evidence
# ---------------------------------------------------------------------------
grep -qa '\*\*\* KERNEL PANIC \*\*\*' "${log}" ||
	fail "the probe wrote to the guard page and nothing panicked"
grep -qa 'sync exception EL1:' "${log}" ||
	fail "the panic did not come from a trap (no EL1 sync exception line)"
grep -qaE 'ESR=0x[0-9a-f]{16}' "${log}" || fail "no ESR in the panic body"
grep -qaE 'FAR=0x[0-9a-f]{16}' "${log}" || fail "no FAR in the panic body"

# ---------------------------------------------------------------------------
# 4. The syndrome belongs to *this* fault
#
# The one assertion that separates this gate from `grep PANIC`: the address the
# panic path printed must be the address the probe wrote. A stale `last_fault`
# — a real failure mode, since the syndrome is published by the trap and read
# by policy several frames later — would pass every check above.
# ---------------------------------------------------------------------------
announced_va="$(sed -E 's/^.* at (0x[0-9a-f]+), writing$/\1/' <<<"${announce}")"
printed_far="$(grep -aoE 'FAR=0x[0-9a-f]{16}' "${log}" | head -1 | cut -d= -f2)"
# Compare as numbers: the announce prints the VA unpadded, FAR is 16 digits.
if ((announced_va != printed_far)); then
	fail "FAR ${printed_far} is not the address the probe wrote (${announced_va}) — last_fault is reporting a different fault"
fi

# ---------------------------------------------------------------------------
# 5. The address was named, by the branch with the policy in it
# ---------------------------------------------------------------------------
# No `$` anchor: the PL011 ends lines with CRLF, so the carriage return sits
# between the last word and the end of line.
grep -qaE '^fault: 0x[0-9a-f]+ unmapped inside "heap" — task-stack guard page, i\.e\. stack overflow' "${log}" ||
	fail "the guard page was not named as a stack overflow: $(grep -a '^fault:' "${log}" || echo '(no fault line)')"

# ---------------------------------------------------------------------------
# 6. The diagnostic completed, the guard held, and the core stopped
# ---------------------------------------------------------------------------
grep -qa '\*\*\* halt \*\*\*' "${log}" || fail "the panic body was truncated before '*** halt ***'"

panics="$(grep -ca 'KERNEL PANIC' "${log}" || true)"
((panics == 1)) ||
	fail "'KERNEL PANIC' appears ${panics} times — the PANICKING re-entry guard did not hold"

hellos="$(grep -ca 'Harbor: hello' "${log}" || true)"
((hellos == 1)) ||
	fail "'Harbor: hello' appears ${hellos} times — the board reset instead of parking; cpu::halt did not stop the core"

after="$(sed -n '/\*\*\* halt \*\*\*/,$p' "${log}" | tail -n +2 | grep -av '^\s*$' | grep -av 'qemu-system' || true)"
[[ -z "${after}" ]] ||
	fail "output continued after '*** halt ***': ${after}"

echo "panic-check: clean (guard-page fault at ${announced_va}, named, halted once)"
