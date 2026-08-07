#!/usr/bin/env python3
"""List agents in a Harbor external store (ADR-0027) — host compose/audit aid (P6 light)."""
from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

MAGIC = b"HARB"
NAME_LEN = 16
SLOT_NONE = 0xFF


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("path", type=Path, nargs="?", default=Path("target/agents.bin"))
    args = ap.parse_args()
    if not args.path.is_file():
        print(f"inspect-agent-store: missing {args.path}", file=sys.stderr)
        return 1
    raw = args.path.read_bytes()
    if len(raw) < 16 or raw[:4] != MAGIC:
        print("inspect-agent-store: bad magic or too short", file=sys.stderr)
        return 1
    version, count, _res = struct.unpack_from("<III", raw, 4)
    print(f"magic=HARB version={version} count={count} bytes={len(raw)}")
    off = 16
    for i in range(count):
        if off + NAME_LEN + 16 > len(raw):
            print(f"  agent[{i}]: truncated header", file=sys.stderr)
            return 1
        name = raw[off : off + NAME_LEN].split(b"\x00", 1)[0].decode("utf-8", "replace")
        off += NAME_LEN
        text_pages, stack_pages = struct.unpack_from("<II", raw, off)
        off += 8
        slots = list(raw[off : off + 4])
        off += 4
        off += 4  # reserved
        (image_len,) = struct.unpack_from("<I", raw, off)
        off += 4
        image = raw[off : off + image_len]
        off += image_len
        while off % 4:
            off += 1
        slot_s = ",".join("_" if s == SLOT_NONE else str(s) for s in slots)
        print(
            f"  [{i}] name={name!r} text_pages={text_pages} stack_pages={stack_pages} "
            f"slots=[{slot_s}] image_len={len(image)}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
