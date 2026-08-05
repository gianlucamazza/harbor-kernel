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

cd "$(dirname "$0")/.."

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
	time*) echo "arch" ;;
	console*) echo "arch bsp drivers" ;;
	# panic may paint the TFT status banner when debug-display is on.
	panic*) echo "arch console status" ;;
	mm*) echo "arch bsp" ;;
	# Cooperative scheduler: TCBs, stacks, switch, wake queue — not drivers/board.
	sched*) echo "arch mm" ;;
	# M4 IPC: mailboxes + caps; parks/wakes via sched only.
	ipc*) echo "arch sched" ;;
	# The board binds protocols together; that is its job (rule 2).
	bsp*) echo "arch bsp console drivers irq time" ;;
	# Policy sits on top of everything and is allowed to.
	bootstrap* | main) echo "agent arch bsp console drivers ipc irq mm sched status time" ;;
	# Agent shell: AS + EL0 sessions; SYS_PUTC via console TX; Irq → irq::handle_cpu_irq.
	# No drivers/board (device PA/VA come from bootstrap demos / BSP constants via mm).
	agent*) echo "arch console irq mm sched" ;;
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
edge='(^|[^A-Za-z0-9_])%s::(%s)([^A-Za-z0-9_]|$)'
isa_re="$(printf "${edge}" arch "${isa_alt}")"
board_re="$(printf "${edge}" bsp "${board_alt}")"

report_facade() {
	# $1 file, $2 regex, $3 message
	local hits
	hits="$(grep -nE "$2" <<<"${body}" || true)"
	[[ -z "${hits}" ]] && return 0
	echo "layering: $1 $3" >&2
	sed 's/^/  /' <<<"${hits}" >&2
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
