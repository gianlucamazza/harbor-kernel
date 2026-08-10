#!/usr/bin/env bash
# Enforce the layering rules in docs/architecture.md against the actual imports.
#
# Those rules *are* the architecture — drivers never know the board, arch never
# names board peripherals, `exception` reaches only `irq`. They used to be
# enforced by review alone, which this project twice watched fail: the rule
# against pre-MMU atomics was withdrawn by the person who wrote it and cost a
# silent board, and the README's gate list went stale on the very commit that
# added a gate. This script closes the import-edge half of that gap (F24).
#
# The check walks every import edge, not every module: a rule holds only for the
# edges that propagate it, so an edge nobody looks at is where it stops holding.
set -euo pipefail

cd "$(dirname "$0")/../.."

# Modules every layer may use: the macro surface and the shared-state primitive.
# `println`/`print` are macro paths, not a layer.
# `println` / `print` / `kprintln` are macros, not layers.
ubiquitous='println print kprintln sync'

# Allowed targets per source layer, deny by default. Anything not listed is a
# violation, so a new module has to be placed here deliberately.
allowed_for() {
	case "$1" in
	# The bottom. Naming a driver or the board here inverts the whole stack.
	arch::aarch64::exception*) echo "irq" ;; # rule 4: only irq
	arch*) echo "arch" ;;                    # rule 3: CPU/ISA only
	drivers*) echo "arch irq" ;;             # rule 1: never the board
	irq*) echo "arch irq" ;;
	# ADR-0028: timer/UART signal waiters via irq::wait (no sched import).
	time*) echo "arch irq" ;;
	console*) echo "arch bsp drivers irq" ;;
	# Product panic: console + fault address naming via the live map (policy).
	# Trap publishes syndrome (`arch::exception::last_fault`); panic asks
	# `mm::layout` to name FAR — arch may not import layout (feat boot diagnostics).
	# Lab panic lives under lab::*.
	panic*) echo "arch console mm status" ;;
	# Lab maturity path: arch + board bind only (project-topology).
	lab*) echo "arch bsp" ;;
	mm*) echo "arch bsp" ;;
	# Cooperative scheduler: TCBs, stacks, switch, wake queue - not drivers/board.
	# ADR-0028: drains irq::wait and exposes wait_for_irq.
	# ADR-0031: spawn/exit register SEND holds via ipc (K2 last-hold auto-reap).
	# ADR-0040: park timeout polls time::ticks on the voluntary path.
	# ADR-0054: revoke task-caps on exit; peer transfer resolves task-caps.
	sched*) echo "arch mm irq ipc time taskcap" ;;
	# M4 IPC: mailboxes + caps; parks/wakes via sched only.
	# ADR-0040: recv_with_timeout reads time::ticks for the deadline.
	ipc*) echo "arch sched time" ;;
	# ADR-0035 / P5: name → CapId registry (trusted EL1).
	naming*) echo "arch" ;;
	# ADR-0054 / K3: task capabilities for peer transfer (trusted EL1).
	taskcap*) echo "arch" ;;
	# ADR-0036 / P2: keyed blob store (trusted EL1).
	storage*) echo "arch" ;;
	# ADR-0045 / P2 durable region (trusted EL1).
	durable*) echo "arch" ;;
	# The board binds protocols together; that is its job (rule 2).
	bsp*) echo "arch bsp console drivers irq time" ;;
	# Policy sits on top of everything and is allowed to.
	# `lab` is maturity dispatch only from main on lab targets.
	bootstrap* | main) echo "agent arch bsp console drivers durable ipc irq lab mm naming sched status storage taskcap time" ;;
	# Agent shell: AS + EL0 sessions; SYS_SEND/RECV via ipc; Irq → irq::handle_cpu_irq.
	# No drivers/board (device PA/VA come from bootstrap demos / BSP constants via mm).
	#
	# `ipc` was added when `SYS_SEND`/`SYS_RECV` arrived (M7 slice 2, ADR-0017 §2):
	# an agent naming a capability by slot has to reach the mailbox table, and the
	# translation deliberately lives in `ipc` rather than here, because the
	# authority counter is `ipc`'s to maintain. The edge is the cost of keeping
	# the definition of "authority violation" in one module.
	# ADR-0039: SYS_RESOLVE reaches the name registry (no CapId to EL0).
	agent*) echo "arch console ipc irq mm naming sched" ;;
	# TFT status surface: policy only; paints via BSP display handle.
	status*) echo "arch bsp drivers mm time" ;;
	*) echo "" ;;
	esac
}

violations=0

for file in $(find src -name '*.rs' | sort); do
	module="$(sed 's|^src/||; s|/mod\.rs$||; s|\.rs$||; s|/|::|g' <<<"${file}")"
	permitted="$(allowed_for "${module}") ${ubiquitous}"

	# Strip comments first: a doc link like [`crate::drivers`] is prose, not an
	# import, and counting it would make the gate cry wolf on documentation.
	imports="$(sed 's|//.*||' "${file}" |
		grep -ohE 'crate::[a-z_]+' | sed 's/crate:://' | sort -u || true)"

	for target in ${imports}; do
		# A module importing from itself is not an edge between layers.
		[[ "${module}" == "${target}"* ]] && continue
		if ! grep -qw -- "${target}" <<<"${permitted}"; then
			echo "layering: ${module} imports crate::${target}, which its layer may not use" >&2
			echo "  allowed: $(allowed_for "${module}")" >&2
			violations=$((violations + 1))
		fi
	done
done

# Facade isolation (ADR-0015): outside the owning tree, never name an ISA or a
# concrete board path. Policy uses `crate::arch::*` and `crate::bsp::board::*`.
# Without this, layering only sees the first path segment (`arch` / `bsp`) and
# a `use crate::arch::aarch64::cpu` would pass silently.
#
# The names come from the tree, never from a list written here. A gate that
# enumerates what exists today goes quiet exactly when the second ISA arrives —
# which is the only moment it was written for. `check-pre-mmu-path.sh` refuses
# to inspect nothing for the same reason.
isa_alt="$(find src/arch -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort | paste -sd'|' -)"
board_alt="$(find src/bsp -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort | paste -sd'|' -)"
if [[ -z "${isa_alt}" || -z "${board_alt}" ]]; then
	echo "layering: no ISA or board directory found under src/ — refusing to report clean" >&2
	exit 1
fi

# Matched without the `crate::` prefix on purpose: `use crate::{arch::aarch64::cpu,
# bsp}` carries the same edge and does not contain the literal `crate::arch`.
# Bounded on both sides so `march::aarch64` or `aarch64_foo` cannot match.
# Built by concatenation rather than `printf "${fmt}"`: a variable used as a
# format string is how a `%` in a module name becomes a formatting directive.
edge_prefix='(^|[^A-Za-z0-9_])'
edge_suffix='([^A-Za-z0-9_]|$)'
isa_re="${edge_prefix}arch::(${isa_alt})${edge_suffix}"
board_re="${edge_prefix}bsp::(${board_alt})${edge_suffix}"

report_facade() {
	# $1 file, $2 regex, $3 message
	local hits
	hits="$(grep -nE "$2" <<<"${body}" || true)"
	[[ -z "${hits}" ]] && return 0
	echo "layering: $1 $3" >&2
	echo "${hits//$'\n'/$'\n'  }" | sed '1s/^/  /' >&2
	violations=$((violations + 1))
}

for file in $(find src -name '*.rs' | sort); do
	# Strip line comments; do not treat doc prose as imports.
	body="$(sed 's|//.*||' "${file}")"
	if [[ "${file}" != src/arch/* ]]; then
		report_facade "${file}" "${isa_re}" "names an ISA directly — use the facade crate::arch::*"
	fi
	if [[ "${file}" != src/bsp/* ]]; then
		report_facade "${file}" "${board_re}" "names a board directly — use crate::bsp::board"
	fi
done

if [[ "${violations}" -ne 0 ]]; then
	echo "layering: ${violations} violation(s) of the rules in docs/architecture.md" >&2
	exit 1
fi

echo "layering: clean ($(find src -name '*.rs' | wc -l) files, every import edge checked)"
