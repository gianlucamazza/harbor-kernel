#!/usr/bin/env python3
"""A pure model is consumed by the code that ships, or it says why not (ADR-0110).

## The failure this closes

`crates/kernel-core/src/genet.rs` grew to 3528 lines of host-tested model while
`src/drivers/genet.rs` open-coded its own producer index. `docs/verification.md`
listed those host tests as evidence for ADR-0105/0106 — evidence about code the
silicon does not execute.

Counting `pub` items not named in `src/` says 38, which is wrong and was the
first answer this project gave: most of them are reached transitively through
functions the driver *does* call. Reachability says **9**, and of those the
interesting one was `RingCursor`, whose `advance()` wrapped at
`TOTAL_DESCRIPTORS` (256) while ring 0 carries `V5_Q0_TX_BD_CNT` (128) BDs. A
second copy of an advance `RingState` already owned, and the copy was wrong.
Nothing caught it because nothing ran it. An unconsumed model is not neutral; it
rots, and its host tests keep passing while it does.

## What it asserts

For each file below: every `pub` item unreachable from `src/` carries a
`Design-ahead (<slice>)` line in its doc comment, naming what will consume it.

Three outcomes are allowed and the gate distinguishes them:

  consumed      reachable from `src/` — nothing to say
  design-ahead  not reachable, and the doc names the slice that will consume it
  (neither)     refused

Deleting is always available and is what happened to four items when this gate
was written. `Design-ahead` is not a parking space: it is a claim that a named
slice needs this, and the slice is on the roadmap where anyone can check.

## What it cannot assert

Whether the model is *correct*, or whether the named slice is real. It compares
reachability against an annotation; a design-ahead marker naming a slice nobody
will ever write reads exactly like one naming next week's work. It is also
blind to a model item that is reachable but whose result the driver ignores.

## Why only genet

Because that is where the divergence was found and measured. Widening this to
all of `kernel-core` would refuse most of the crate on day one — much of it is
consumed by `src/` only indirectly or is genuinely pure — and a gate that must
be silenced everywhere on the day it lands teaches people to silence it. The
list below is the honest scope; adding to it is one line.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Files whose pub surface must be consumed or declared. See "Why only genet".
FILES = [Path("crates/kernel-core/src/genet.rs")]

ITEM_START = re.compile(
    r"^(pub(\(crate\))? )?(unsafe )?(fn|struct|enum|const|static|impl|trait|type|mod) "
)
PUB_ITEM = re.compile(r"^pub (fn|struct|enum|const|static|trait|type) ([A-Za-z0-9_]+)")
IDENT = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\b")


def owner(block_name: str) -> str:
    """The type an `impl` block belongs to; otherwise the item's own name."""
    return block_name[5:] if block_name.startswith("impl:") else block_name


def split_blocks(text: str) -> list[tuple[str, str, list[str]]]:
    """(name, body, preceding doc lines) for each top-level item."""
    lines = text.splitlines(keepends=True)
    blocks: list[tuple[str, str, list[str]]] = []
    name, body, doc = "<prelude>", [], []
    pending_doc: list[str] = []
    for line in lines:
        if ITEM_START.match(line):
            blocks.append((name, "".join(body), doc))
            rest = line[ITEM_START.match(line).end() :]
            keyword = ITEM_START.match(line).group(4)
            segs = re.findall(r"[A-Za-z_][A-Za-z0-9_]*", rest.split("{")[0])
            if keyword == "impl":
                # `impl Foo`, `impl<T> Foo`, `impl Display for Foo` — the type
                # the block belongs to is the last path segment before the brace.
                name = "impl:" + (segs[-1] if segs else "?")
            else:
                name = segs[0] if segs else keyword
            body, doc, pending_doc = [line], pending_doc, []
        else:
            body.append(line)
            stripped = line.lstrip()
            if stripped.startswith("///") or stripped.startswith("#["):
                pending_doc.append(line)
            elif stripped:
                pending_doc = []
    blocks.append((name, "".join(body), doc))
    return blocks


def check(path: Path) -> list[str]:
    text = (ROOT / path).read_text()
    marker = text.find("\n#[cfg(test)]")
    prod = text[:marker] if marker != -1 else text

    blocks = split_blocks(prod)
    bodies: dict[str, str] = {}
    docs: dict[str, list[str]] = {}
    for name, body, doc in blocks:
        bodies[name] = bodies.get(name, "") + body
        docs.setdefault(name, []).extend(doc)

    # Every identifier `src/` mentions. Coarse on purpose: a name the driver
    # writes anywhere counts as consumed, so this errs towards saying "fine".
    grep = subprocess.run(
        ["grep", "-rhoE", r"\b[A-Za-z_][A-Za-z0-9_]*\b", "src/"],
        capture_output=True,
        text=True,
        cwd=ROOT,
    )
    src_idents = set(grep.stdout.split())

    reach = {n for n in bodies if owner(n) in src_idents}
    frontier = list(reach)
    while frontier:
        referenced = set(IDENT.findall(bodies[frontier.pop()]))
        for candidate in bodies:
            if owner(candidate) in referenced and candidate not in reach:
                reach.add(candidate)
                frontier.append(candidate)
    reachable_names = {owner(n) for n in reach}

    problems: list[str] = []
    for line in prod.splitlines():
        m = PUB_ITEM.match(line)
        if not m:
            continue
        item = m.group(2)
        if item in src_idents or item in reachable_names:
            continue
        doc = "".join(docs.get(item, []))
        if "Design-ahead" not in doc:
            problems.append(
                f"{path}: `{item}` is pub, nothing in src/ can reach it, and its\n"
                f"  doc comment does not say which slice will consume it.\n"
                f"  Add `/// Design-ahead (<slice>): <why>` naming a roadmap row,\n"
                f"  make the driver consume it, or delete it (ADR-0110)."
            )
    return problems


def main() -> int:
    problems: list[str] = []
    for path in FILES:
        if not (ROOT / path).exists():
            print(f"model-consumed: FAIL — {path} does not exist", file=sys.stderr)
            return 1
        problems.extend(check(path))

    for p in problems:
        print(f"model-consumed: {p}", file=sys.stderr)
    if problems:
        print(
            f"model-consumed: {len(problems)} unconsumed, undeclared pub item(s)",
            file=sys.stderr,
        )
        return 1

    print(
        f"model-consumed: clean ({len(FILES)} file(s); every pub item is "
        f"reachable from src/ or names the slice that will reach it)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
