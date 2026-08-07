#!/usr/bin/env bash
# A module path written in the descriptive docs must name code that is there.
#
# `check-xrefs.sh` follows markdown links, and `check-doc-claims.sh` compares the
# two counted claims in the README. Neither can see the most common way a
# document goes wrong: it names a symbol by its **module path**, the code moves,
# and the sentence keeps reading correctly while pointing at nothing.
#
# That has now happened three times. Twice it was the gate list (F27). The third
# was `docs/mmu.md`, which described the early identity map as
# `arch::mmu::EARLY_L1` for a day after F23 moved it to `src/mm/early.rs` — the
# whole point of that finding was that board topology does not belong in `arch`,
# and the document explaining the map still put it there. `make arch-board-free`
# watches the code; nothing watched the prose explaining the code.
#
# The check is path-aware on purpose. Asking only "does `EARLY_L1` exist
# somewhere" would have passed: it does exist, in a different module. So for
# `a::b::NAME` this finds where `NAME` is declared and requires the declaring
# file to actually be module `b`.
#
# Scope: **descriptive** documents only. ADRs and review reports are dated
# records of what was true when they were written — ADR-0016 names nine
# `static mut` that no longer exist, and that is correct, not stale.
# `verification.md` is the same shape: a log, including code that has since been
# fixed.
set -euo pipefail

cd "$(dirname "$0")/.."

readonly DOCS=(
	README.md
	SECURITY.md
	docs/architecture.md
	docs/mmu.md
	docs/interrupts.md
	docs/hardware.md
	docs/boot-chain.md
	docs/porting.md
	docs/arch-contract.md
	docs/blobs.md
)

python3 - "${DOCS[@]}" <<'PY'
import io, os, re, subprocess, sys

# Roots that are not our modules: crate names and the standard library.
ROOTS = {"kernel_core", "harbor_kernel", "crate", "core", "std", "alloc"}

sources = subprocess.run(
    ["bash", "-c", 'find src crates -name "*.rs"'],
    capture_output=True, text=True, check=True,
).stdout.split()

# Where each name is declared. `const fn` is why the const arm requires the
# trailing `:` of a type annotation: without it, `pub const fn plan(` records
# "fn" as the declared name and the real one never enters the index. That bug
# was in the first draft of this script and reported two live functions as
# missing, which is the failure mode a gate must not have.
DECL = re.compile(
    r"\bfn\s+([A-Za-z_]\w*)"
    r"|\b(?:struct|enum|trait|type|mod|union)\s+([A-Za-z_]\w*)"
    r"|\b(?:const|static)\s+(?:mut\s+)?([A-Za-z_]\w*)\s*:"
)
declared = {}
for path in sources:
    text = io.open(path, encoding="utf-8").read()
    for match in DECL.finditer(text):
        name = next(g for g in match.groups() if g)
        declared.setdefault(name, set()).add(path)

PATH = re.compile(r"`((?:[a-z_]\w*::){1,3}[A-Za-z_]\w*)`")
violations = 0
checked = 0
for doc in sys.argv[1:]:
    if not os.path.exists(doc):
        continue
    for match in PATH.finditer(io.open(doc, encoding="utf-8").read()):
        full = match.group(1)
        parts = full.split("::")
        if parts[0] in ROOTS and len(parts) == 2:
            continue          # `kernel_core::ipc` — a module of a crate root
        if parts[0] in ("core", "std", "alloc"):
            continue          # the standard library is not ours to check
        name, parent = parts[-1], parts[-2]
        if parent in ROOTS:
            continue          # `crate::arch` and friends
        checked += 1
        where = declared.get(name)
        if not where:
            print(f"doc-symbols: {doc}: `{full}` — {name} is declared nowhere", file=sys.stderr)
            violations += 1
            continue
        if not any(os.path.basename(w)[:-3] == parent or f"/{parent}/" in w for w in where):
            listed = ", ".join(sorted(where))
            print(
                f"doc-symbols: {doc}: `{full}` — {name} lives in {listed}, "
                f"which is not a module '{parent}'",
                file=sys.stderr,
            )
            violations += 1

if violations:
    print(f"doc-symbols: {violations} path(s) naming code that is not there", file=sys.stderr)
    print("  Descriptive docs must survive a move. If the code was renamed or", file=sys.stderr)
    print("  relocated, the sentence explaining it moved too — or it is now", file=sys.stderr)
    print("  explaining a file that does not exist.", file=sys.stderr)
    sys.exit(1)

print(f"doc-symbols: clean ({checked} module paths across {len(sys.argv) - 1} descriptive docs)")
PY
