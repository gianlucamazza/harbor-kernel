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


def _vocabularies() -> tuple[dict[int, str], dict[int, str]]:
    """The kernel's index -> name tables, read from the packer beside this file.

    So the audit reader can print what an index *means* rather than only what it
    is (ADR-0101): `window 0 (rng)` audits a composition, `window 0` asks the
    reader to go and look. Loaded from `pack-store.py` rather than restated,
    because that table is the one `make vocabulary-sync` compares against
    `src/bootstrap/authority.rs` — a third copy here would be the drift that
    gate exists to catch.

    A packer that cannot be loaded is not fatal: the indices still print.
    """
    import importlib.util

    packer = Path(__file__).resolve().parent / "pack-store.py"
    try:
        spec = importlib.util.spec_from_file_location("harbor_pack_store", packer)
        if spec is None or spec.loader is None:
            return {}, {}
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return (
            {v: k for k, v in mod.HELD.items()},
            {v: k for k, v in mod.WINDOWS.items()},
        )
    except Exception:
        return {}, {}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("path", type=Path, nargs="?", default=Path("target/agents.bin"))
    args = ap.parse_args()
    if not args.path.is_file():
        print(f"inspect-agent-store: missing {args.path}", file=sys.stderr)
        return 1
    held_names, window_names = _vocabularies()
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
        if reserved & ~0x1FF:
            print(f"  agent[{i}]: reserved grant bits are non-zero", file=sys.stderr)
            return 1
        home_cpu = reserved & 0xFF
        may_resolve = bool(reserved & 0x100)
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
        def named(index: int, table: dict[int, str]) -> str:
            name = table.get(index)
            return f"{index}({name})" if name else str(index)

        slot_s = ",".join(
            "_" if s == SLOT_NONE else named(s, held_names) for s in slots
        )
        device_s = (
            "none"
            if window == WINDOW_NONE
            else f"window {named(window, window_names)} @ {device_va:#x}"
        )
        print(
            f"  [{i}] name={name!r} text_pages={text_pages} stack_pages={stack_pages} "
            f"home_cpu={home_cpu} may_resolve={may_resolve} slots=[{slot_s}] "
            f"device={device_s} image_len={len(image)}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
