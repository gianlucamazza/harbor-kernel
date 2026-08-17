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

# The window arithmetic is shared with `inspect-store.py`, which reads back
# what this writes (`store_window.py`). One copy, two users.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from store_window import IMAGE_BASE, resolve_window  # noqa: E402


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
