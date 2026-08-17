#!/usr/bin/env bash
# Refuse a tree where a `kernel-core` module has no recorded mutation decision.
#
# ## The failure this exists for
#
# `mutation-freshness` counts the mutants `cargo mutants --list` reports *for
# the files the run asks for*. That makes it blind by construction to a module
# the run never asks for: `genet.rs` grew to 3142 lines over 51 commits — the
# entire ADR-0105/0106 model — and the gate stayed green at 660 the whole time.
#
# ADR-0058 §2 said a module joins the list the commit it is born. ADR-0049
# recorded that the list is hand-written and deferred the fix with an explicit
# trigger: "Marker-derived list, or the next membership miss". The miss
# happened. This is the mechanism.
#
# ## What it does and does not claim
#
# It does **not** claim every module is mutated. It claims every module has a
# recorded decision — `in_scope`, `queued`, or `exempt` — in
# `docs/mutation-scope.toml`, and that the set of decisions is exactly the set
# of modules. Forgetting becomes impossible; choosing badly stays possible, and
# stays visible in a diff.
#
# The scope file is also the single source of the run's `--file` arguments, so
# there is no second copy of the list to go stale (the failure `xrefs` exists
# for, one document over).
#
# Seen red: with `genet` and `genet_fdt` present in `lib.rs` and absent from
# the scope file, this reports both as unclassified and exits 1.
set -euo pipefail

cd "$(dirname "$0")/../.."

readonly SCOPE="docs/mutation-scope.toml"
readonly LIB="crates/kernel-core/src/lib.rs"

if [[ ! -f "${SCOPE}" ]]; then
	echo "mutation-scope: FAIL — ${SCOPE} is missing" >&2
	exit 1
fi

python3 - "${SCOPE}" "${LIB}" <<'PY'
import sys, tomllib, re

scope_path, lib_path = sys.argv[1], sys.argv[2]

with open(scope_path, "rb") as fh:
    scope = tomllib.load(fh)

in_scope = list(scope.get("in_scope", []))
queued = list(scope.get("queued", {}))
exempt = list(scope.get("exempt", {}))

with open(lib_path, encoding="utf-8") as fh:
    modules = re.findall(r"^pub mod ([a-z0-9_]+);$", fh.read(), re.M)

problems = []

declared = in_scope + queued + exempt
seen = set()
for name in declared:
    if name in seen:
        problems.append(f"'{name}' is classified more than once")
    seen.add(name)

for name in sorted(set(modules) - seen):
    problems.append(
        f"'{name}' is a pub mod of kernel-core with no mutation decision — "
        f"add it to in_scope, queued, or exempt in {scope_path}"
    )

for name in sorted(seen - set(modules)):
    problems.append(f"'{name}' is classified in {scope_path} but is not a pub mod of kernel-core")

for name in queued + exempt:
    table = "queued" if name in queued else "exempt"
    reason = scope[table][name]
    if not isinstance(reason, str) or len(reason.strip()) < 10:
        problems.append(f"'{name}' is {table} with no usable reason — a blank reason is a forgotten decision")

if problems:
    for line in problems:
        print(f"mutation-scope: {line}", file=sys.stderr)
    print(f"mutation-scope: {len(problems)} unrecorded or contradictory decision(s)", file=sys.stderr)
    raise SystemExit(1)

print(
    f"mutation-scope: clean ({len(modules)} kernel-core modules: "
    f"{len(in_scope)} in scope, {len(queued)} queued, {len(exempt)} exempt)"
)
PY
