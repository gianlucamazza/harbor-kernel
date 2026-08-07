#!/usr/bin/env python3
"""Pack a Harbor external agent store (ADR-0027).

Default product composition (P1): beacon (H!) + chirp (?) — two agents, one
console grant each at slot 1 → held[0].
"""
from __future__ import annotations

import argparse
import struct
import subprocess
import sys
from pathlib import Path

MAGIC = b"HARB"
VERSION = 1
SLOT_NONE = 0xFF
NAME_LEN = 16

# encode_console_hi_exit(1) — keep in sync with kernel_core::prog.
BEACON_ASM = """\
movz x0, #1
movz x1, #0
movz x2, #72
svc #3
movz x0, #1
movz x1, #0
movz x2, #33
svc #3
svc #1
b .
"""

# One console byte '?' then exit (encode_console_once_exit(1, b'?')).
CHIRP_ASM = """\
movz x0, #1
movz x1, #0
movz x2, #63
svc #3
svc #1
b .
"""


def assemble(asm: str) -> bytes:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        td_path = Path(td)
        obj = td_path / "a.o"
        bin_path = td_path / "a.bin"
        src = td_path / "a.s"
        src.write_text(asm)
        subprocess.check_call(
            [
                "llvm-mc",
                "--assemble",
                "--triple=aarch64",
                "-filetype=obj",
                "-o",
                str(obj),
                str(src),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        subprocess.check_call(
            ["llvm-objcopy", "-O", "binary", str(obj), str(bin_path)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return bin_path.read_bytes()


def pad_name(name: str) -> bytes:
    b = name.encode("utf-8")[:NAME_LEN]
    return b + b"\x00" * (NAME_LEN - len(b))


def append_agent(
    buf: bytearray,
    name: str,
    text_pages: int,
    stack_pages: int,
    slots: list[int],
    image: bytes,
) -> None:
    assert len(slots) == 4
    buf += pad_name(name)
    buf += struct.pack("<II", text_pages, stack_pages)
    buf += bytes(slots)
    buf += struct.pack("<I", 0)  # reserved
    buf += struct.pack("<I", len(image))
    buf += image
    while len(buf) % 4:
        buf += b"\x00"


def pack(agents: list[tuple[str, int, int, list[int], bytes]]) -> bytes:
    buf = bytearray()
    buf += MAGIC
    buf += struct.pack("<III", VERSION, len(agents), 0)
    for name, tp, sp, slots, image in agents:
        append_agent(buf, name, tp, sp, slots, image)
    return bytes(buf)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "-o",
        "--output",
        type=Path,
        default=Path("target/agents.bin"),
        help="output store path",
    )
    ap.add_argument(
        "--single-beacon",
        action="store_true",
        help="pack only the M8 beacon (legacy single-agent store)",
    )
    args = ap.parse_args()

    try:
        beacon = assemble(BEACON_ASM)
        chirp = assemble(CHIRP_ASM)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"pack-agent-store: FAIL — need llvm-mc and llvm-objcopy: {e}", file=sys.stderr)
        return 1

    # slot 1 → held index 0 (console send); others empty
    slots = [SLOT_NONE, 0, SLOT_NONE, SLOT_NONE]
    if args.single_beacon:
        agents = [("beacon", 1, 3, slots, beacon)]
    else:
        # P1: two product agents share the same held console grant.
        agents = [
            ("beacon", 1, 3, slots, beacon),
            ("chirp", 1, 3, slots, chirp),
        ]
    blob = pack(agents)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(blob)
    names = ",".join(a[0] for a in agents)
    print(
        f"pack-agent-store: wrote {args.output} ({len(blob)} bytes, n={len(agents)} [{names}])"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
