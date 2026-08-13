#!/usr/bin/env python3
"""Pack a Harbor external agent store (ADR-0027).

Default product composition: beacon (H!) + chirp (?) + lookup (N) + entropy + blob (S).
"""
from __future__ import annotations

import argparse
import struct
import subprocess
import sys
from pathlib import Path

MAGIC = b"HARB"
VERSION = 2
SLOT_NONE = 0xFF

# The composition's vocabulary (ADR-0099). These integers are an ABI with
# `src/bootstrap/authority.rs`, which is where they are declared and minted:
# a store entry's slot holds one of them, and the kernel binds it by indexing.
# `make vocabulary-sync` compares this table against that file — a fact in two
# places is how the oracle-marker list and the MAX_TASKS census both went wrong.
HELD = {
    "console": 0,
    "blob": 1,
    "blob-reply": 2,
    "net-tx": 3,
    "net-tx-complete": 4,
    "net-rx": 5,
    "net-rx-return": 6,
}

# The device-window vocabulary (ADR-0100). Same ABI relationship as HELD above,
# against the WINDOW_* constants of `src/bootstrap/authority.rs`, and compared
# by `make vocabulary-sync`. A store entry carries one of these **indices** and
# never a physical address: the board decides what the page is, the composition
# only decides where in its own window it lands.
#
WINDOWS = {
    "rng": 0,
}

# "No device window" — the value every agent in this product carries today.
WINDOW_NONE = 0xFF
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

# encode_resolve_send_exit(0, b'N') — ADR-0102. The agent starts with no
# console slot, resolves the product binding by name, then sends one byte.
LOOKUP_ASM = """\
movz x0, #0
movz x1, #7
movz x2, #28515
movk x2, #29550, lsl #16
movk x2, #27759, lsl #32
movk x2, #101, lsl #48
svc #7
movz x0, #0
movz x1, #0
movz x2, #78
svc #3
svc #1
b .
"""


# encode_read_device_bit_console_exit(0x5100, RNG_CTRL=0, CTRL_RBGEN=0, 1, 'R', 'r')
# — keep in sync with kernel_core::prog.
#
# ADR-0101: the first agent that arrives in a store *and* drives a device. It
# reads RNG_CTRL from the window the loader mapped for it and sends 'R' if the
# block is enabled, 'r' if not — a byte only a real load can produce.
ENTROPY_ASM = """\
movz x0, #0x5100, lsl #16
ldr w1, [x0, #0]
tbnz w1, #0, #12
movz x2, #114
b #8
movz x2, #82
movz x0, #1
movz x1, #0
svc #3
svc #1
b .
"""

# encode_blob_round_trip_exit(2, 3, 1) — P2 durable endpoint.
BLOB_ASM = """\
movz x0, #2
movz x1, #0x1001
movz x2, #0x6663
movk x2, #0x0067, lsl #16
movk x2, #0x0300, lsl #48
movz x3, #0x6570
movk x3, #0x7372, lsl #16
movk x3, #0x6973, lsl #32
movk x3, #0x0774, lsl #48
svc #3
movz x0, #2
movz x1, #0x1002
movz x2, #0x6663
movk x2, #0x0067, lsl #16
movk x2, #0x0300, lsl #48
movz x3, #0
svc #3
movz x0, #3
svc #4
movz x0, #1
movz x1, #0
movz x2, #83
svc #3
svc #1
b .
"""

# Where the RNG window lands in the agent's own address space. The composition
# chooses this; the board chooses which page appears there (ADR-0100).
ENTROPY_VA = 0x5100_0000


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
    home_cpu: int = 0,
    may_resolve: bool = False,
    packet_pool: bool = False,
    window: int = WINDOW_NONE,
    device_va: int = 0,
) -> None:
    assert len(slots) == 4
    assert 0 <= home_cpu < 256
    assert 0 <= window < 256
    # ADR-0100: an entry with no window has no use for an address, and the
    # parser refuses one — so the format cannot carry an address nothing reads.
    assert window != WINDOW_NONE or device_va == 0
    assert window == WINDOW_NONE or device_va % 4096 == 0
    buf += pad_name(name)
    buf += struct.pack("<II", text_pages, stack_pages)
    buf += bytes(slots)
    # ADR-0088/0102/0104: low byte = home_cpu, bit 8 = resolve, bit 9 = packet pool
    buf += struct.pack(
        "<I",
        (home_cpu & 0xFF) | (int(may_resolve) << 8) | (int(packet_pool) << 9),
    )
    # ADR-0100: device u32 low byte = window index, then the VA it lands at.
    buf += struct.pack("<I", window & 0xFF)
    buf += struct.pack("<Q", device_va)
    buf += struct.pack("<I", len(image))
    buf += image
    while len(buf) % 4:
        buf += b"\x00"


# (name, text_pages, stack_pages, slots, image, home_cpu, may_resolve, window, device_va)
PackAgent = tuple[str, int, int, list[int], bytes, int, bool, int, int]


def pack(agents: list[PackAgent]) -> bytes:
    buf = bytearray()
    buf += MAGIC
    buf += struct.pack("<III", VERSION, len(agents), 0)
    for name, tp, sp, slots, image, home, may_resolve, window, device_va in agents:
        append_agent(
            buf, name, tp, sp, slots, image,
            home_cpu=home, may_resolve=may_resolve,
            window=window, device_va=device_va,
        )
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
        lookup = assemble(LOOKUP_ASM)
        entropy = assemble(ENTROPY_ASM)
        blob = assemble(BLOB_ASM)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"pack-agent-store: FAIL — need llvm-mc and llvm-objcopy: {e}", file=sys.stderr)
        return 1

    # Slot 1 names the console position of the kernel's vocabulary; the other
    # three stay empty. The convention that slot 0 is deliberately unused is
    # `manifest.rs`'s: a program that miscounts finds nothing rather than
    # something adjacent.
    console_slots = [SLOT_NONE, HELD["console"], SLOT_NONE, SLOT_NONE]
    blob_slots = [SLOT_NONE, HELD["console"], HELD["blob"], HELD["blob-reply"]]
    if args.single_beacon:
        agents = [("beacon", 1, 3, console_slots, beacon, 0, False, WINDOW_NONE, 0)]
    else:
        # P1 + ADR-0088: beacon homes on product CPU 0; chirp pins to CPU 1
        # so the shipped composition exercises dual-current without oracle demos.
        # ADR-0101: `entropy` names the `rng` window by name; the index that
        # reaches the wire is this table's, and `make vocabulary-sync` is what
        # keeps it the same integer the kernel declared.
        agents = [
            ("beacon", 1, 3, console_slots, beacon, 0, False, WINDOW_NONE, 0),
            ("chirp", 1, 3, console_slots, chirp, 1, False, WINDOW_NONE, 0),
            ("lookup", 1, 3, [SLOT_NONE] * 4, lookup, 0, True, WINDOW_NONE, 0),
            ("entropy", 1, 3, console_slots, entropy, 0, False, WINDOWS["rng"], ENTROPY_VA),
            ("blob", 1, 3, blob_slots, blob, 0, False, WINDOW_NONE, 0),
        ]
    blob = pack(agents)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(blob)
    names = ",".join(
        f"{a[0]}@cpu{a[5]}" + ("" if a[7] == WINDOW_NONE else f"+w{a[7]}")
        for a in agents
    )
    print(
        f"pack-agent-store: wrote {args.output} ({len(blob)} bytes, n={len(agents)} [{names}])"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
