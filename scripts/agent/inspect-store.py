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
WINDOW_NONE = 0xFF

# The record layouts this tool can read. Version is checked rather than assumed:
# a v1 reader pointed at a v2 blob walks off by 12 bytes per record and prints
# an image length taken from the device word — numbers that look like data and
# are not. An audit aid that invents fields is worse than none (ADR-0100).
SUPPORTED = (2,)


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
    if version not in SUPPORTED:
        print(
            f"inspect-agent-store: version {version} is not one this reader knows "
            f"({', '.join(str(v) for v in SUPPORTED)}) — refusing to guess at the layout",
            file=sys.stderr,
        )
        return 1
    off = 16
    for i in range(count):
        if off + NAME_LEN + 28 > len(raw):
            print(f"  agent[{i}]: truncated header", file=sys.stderr)
            return 1
        name = raw[off : off + NAME_LEN].split(b"\x00", 1)[0].decode("utf-8", "replace")
        off += NAME_LEN
        text_pages, stack_pages = struct.unpack_from("<II", raw, off)
        off += 8
        slots = list(raw[off : off + 4])
        off += 4
        (reserved,) = struct.unpack_from("<I", raw, off)
        off += 4
        home_cpu = reserved & 0xFF
        # ADR-0100: device word (window index in bits 7:0) then the VA it lands
        # at. No physical address is on the wire — what a reader can audit here
        # is which position an agent named, not which page it gets.
        (device_word,) = struct.unpack_from("<I", raw, off)
        off += 4
        window = device_word & 0xFF
        (device_va,) = struct.unpack_from("<Q", raw, off)
        off += 8
        (image_len,) = struct.unpack_from("<I", raw, off)
        off += 4
        image = raw[off : off + image_len]
        off += image_len
        while off % 4:
            off += 1
        slot_s = ",".join("_" if s == SLOT_NONE else str(s) for s in slots)
        device_s = "none" if window == WINDOW_NONE else f"window {window} @ {device_va:#x}"
        print(
            f"  [{i}] name={name!r} text_pages={text_pages} stack_pages={stack_pages} "
            f"home_cpu={home_cpu} slots=[{slot_s}] device={device_s} image_len={len(image)}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
