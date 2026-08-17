#!/usr/bin/env python3
"""Where the agent store lives inside a Harbor kernel image (ADR-0029).

`inject-store.py` writes that window and `inspect-store.py` reads it back, so
the arithmetic that finds it — resolve `__agent_store_start`/`__agent_store_end`
in the ELF, map VMA to raw offset by subtracting the load base — belongs in one
place. It used to live only in the injector, which meant the audit reader could
only ever look at the blob *about to be* shipped, never at the artifact that
was. Two copies of that arithmetic would be the drift `vocabulary-sync` and
`xrefs` exist to catch, one directory over.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

IMAGE_BASE = 0x80000
# Must match `loader::AGENT_STORE_CAPACITY` when end symbols are missing.
DEFAULT_CAPACITY = 16 * 1024
MAGIC = b"HARB"


def symbol_vmas(elf: Path) -> dict[str, int]:
    out = subprocess.check_output(["llvm-nm", "--defined-only", str(elf)], text=True)
    found: dict[str, int] = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) < 3:
            continue
        addr_s, _kind, name = parts[0], parts[1], parts[2]
        try:
            addr = int(addr_s, 16)
        except ValueError:
            continue
        if (
            name in ("__agent_store_start", "__agent_store_end")
            or name.endswith("AGENT_STORE")
            or "AGENT_STORE" in name
            and name.startswith("_ZN")
        ):
            found[name] = addr
    return found


def resolve_window(elf: Path) -> tuple[int, int]:
    """(VMA, capacity) of the store window, from the ELF's own symbols."""
    syms = symbol_vmas(elf)
    start = syms.get("__agent_store_start")
    end = syms.get("__agent_store_end")
    if start is not None and end is not None and end > start:
        return start, end - start
    # Fall back to the Rust static (mangled name contains AGENT_STORE).
    for name, addr in syms.items():
        if "AGENT_STORE" in name and "AgentStoreBuf" not in name:
            return addr, DEFAULT_CAPACITY
    raise SystemExit(
        f"store-window: no agent store symbols in {elf} (have {sorted(syms)})"
    )


def image_offset(vma: int) -> int:
    return vma - IMAGE_BASE


def read_store(elf: Path, image: Path) -> bytes:
    """The store bytes as they sit in `image`, per `elf`'s symbols.

    Refuses when the two disagree. An ELF from a different build resolves to a
    plausible offset in the wrong place, and the reader downstream would then
    report a composition that was never shipped — an audit aid inventing its
    subject is worse than no audit at all.
    """
    vma, capacity = resolve_window(elf)
    off = image_offset(vma)
    raw = image.read_bytes()
    if off < 0 or off + capacity > len(raw):
        raise SystemExit(
            f"store-window: window {vma:#x}+{capacity} falls outside {image} "
            f"({len(raw)} bytes) — the ELF and the image are not from one build"
        )
    window = raw[off : off + capacity]
    if window[:4] != MAGIC:
        raise SystemExit(
            f"store-window: no HARB magic at {off:#x} in {image} — the ELF and "
            f"the image are not from one build, or no store was injected"
        )
    return window
