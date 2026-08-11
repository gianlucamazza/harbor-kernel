#!/usr/bin/env bash
# The deliberate-panic image's assertions, in one place (ADR-0093).
#
# Sourced by `scripts/boot/qemu-panic-boot-check.sh` (emulated) and by
# `scripts/check/hw-transcript-check.sh` (silicon). Same argument as
# `boot-oracle.sh` and `product-oracle.sh`: a hardware gate with its own copy
# of the assertions is a second oracle to keep in step.
#
# On silicon this earns something QEMU cannot give. TLB fills are speculative
# on Cortex-A72 and not in TCG, so "the guard page faults" is a claim only the
# board can settle — and the assertion that the printed `FAR` equals the
# address the probe announced is what makes it a claim about *this* fault.
#
# Contract for a caller: set `log` to a file holding one boot's output, define
# `fail()`, then call `assert_panic_boot`.

# `log` and `fail` come from the caller — see the contract above.
# shellcheck disable=SC2154
assert_panic_boot() {
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
}
