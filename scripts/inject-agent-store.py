#!/usr/bin/env python3
"""Inject a packed agent store into a Harbor kernel image (ADR-0029).

Resolves `__agent_store_start` / `__agent_store_end` (or the `AGENT_STORE`
static) in the ELF, maps VMA → raw image offset (`VMA − 0x80000`), and
overwrites that window with `agents.bin`.
"""
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

IMAGE_BASE = 0x80000
# Must match `loader::AGENT_STORE_CAPACITY` when end symbols are missing.
DEFAULT_CAPACITY = 16 * 1024


def symbol_vmas(elf: Path) -> dict[str, int]:
    out = subprocess.check_output(
        ["llvm-nm", "--defined-only", str(elf)],
        text=True,
    )
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
        if name in (
            "__agent_store_start",
            "__agent_store_end",
        ) or name.endswith("AGENT_STORE") or "AGENT_STORE" in name and name.startswith("_ZN"):
            found[name] = addr
    return found


def resolve_window(elf: Path) -> tuple[int, int]:
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
        f"inject-agent-store: no agent store symbols in {elf} "
        f"(have {sorted(syms)})"
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--elf", type=Path, required=True, help="linked harbor-kernel ELF")
    ap.add_argument("--image", type=Path, required=True, help="raw kernel8*.img")
    ap.add_argument("--store", type=Path, required=True, help="packed agents.bin")
    args = ap.parse_args()

    for p, label in (
        (args.elf, "ELF"),
        (args.image, "image"),
        (args.store, "store"),
    ):
        if not p.is_file():
            print(f"inject-agent-store: missing {label} {p}", file=sys.stderr)
            return 1

    vma, sec_size = resolve_window(args.elf)
    store = args.store.read_bytes()
    if len(store) > sec_size:
        print(
            f"inject-agent-store: store {len(store)} B > window {sec_size} B",
            file=sys.stderr,
        )
        return 1
    if vma < IMAGE_BASE:
        print(f"inject-agent-store: VMA {vma:#x} below image base", file=sys.stderr)
        return 1
    off = vma - IMAGE_BASE
    img = bytearray(args.image.read_bytes())
    if off + sec_size > len(img):
        print(
            f"inject-agent-store: window ends past image ({off + sec_size} > {len(img)})",
            file=sys.stderr,
        )
        return 1
    img[off : off + sec_size] = store + bytes(sec_size - len(store))
    args.image.write_bytes(img)
    if img[off : off + 4] != b"HARB":
        print("inject-agent-store: post-write magic check failed", file=sys.stderr)
        return 1
    print(
        f"inject-agent-store: wrote {len(store)} B into {args.image} "
        f"at file+{off:#x} (VMA {vma:#x}, window {sec_size} B)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
