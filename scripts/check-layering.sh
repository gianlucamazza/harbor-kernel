#!/usr/bin/env bash
# Enforce the layering rules in docs/architecture.md against the actual imports.
#
# Those rules *are* the architecture — drivers never know the board, arch never
# names board peripherals, `exception` reaches only `irq`. Until now they were
# enforced by review alone, which this project has twice watched fail: the rule
# against pre-MMU atomics was withdrawn by the person who wrote it and cost a
# silent board, and the README's gate list went stale on the very commit that
# added a gate. Layering rules are in the same position and matter more.
#
# The check walks every import edge, not every module: a rule holds only for the
# edges that propagate it, so an edge nobody looks at is where it stops holding.
set -euo pipefail

cd "$(dirname "$0")/.."

# Modules every layer may use: the macro surface and the shared-state primitive.
# `println`/`print` are macro paths, not a layer.
ubiquitous='println print sync'

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
	console*) echo "bsp drivers" ;;
	panic*) echo "arch console" ;;
	mm*) echo "arch bsp" ;;
	# The board binds protocols together; that is its job (rule 2).
	bsp*) echo "arch bsp console drivers irq time" ;;
	# Policy sits on top of everything and is allowed to.
	bootstrap* | main) echo "arch bsp console drivers irq mm time" ;;
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

if [[ "${violations}" -ne 0 ]]; then
	echo "layering: ${violations} violation(s) of the rules in docs/architecture.md" >&2
	exit 1
fi

echo "layering: clean ($(find src -name '*.rs' | wc -l) files, every import edge checked)"
