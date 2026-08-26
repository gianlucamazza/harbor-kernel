#!/usr/bin/env python3
"""Check a libpcap capture for the GENET Ethernet witness frame."""

from __future__ import annotations

import struct
import sys
from pathlib import Path

SOURCE = bytes.fromhex("02 00 00 00 00 01")
ETHER_TYPE = b"\x88\xb5"
WITNESS = b"harbor-p3-genet-tx"


def packets(path: Path):
    raw = path.read_bytes()
    if len(raw) < 24:
        raise ValueError("pcap header is truncated")
    magic = raw[:4]
    formats = {
        b"\xd4\xc3\xb2\xa1": ("<", False),
        b"\xa1\xb2\xc3\xd4": (">", False),
        b"\x4d\x3c\xb2\xa1": ("<", True),
        b"\xa1\xb2\x3c\x4d": (">", True),
    }
    try:
        endian, nano = formats[magic]
    except KeyError as exc:
        raise ValueError("unsupported pcap byte order") from exc
    _, _, _, _, _, snaplen, linktype = struct.unpack_from(endian + "IHHIIII", raw, 0)
    if linktype != 1:
        raise ValueError(f"unsupported link type {linktype}, expected Ethernet")
    offset = 24
    while offset < len(raw):
        if offset + 16 > len(raw):
            raise ValueError("pcap record header is truncated")
        _, _, captured, original = struct.unpack_from(endian + "IIII", raw, offset)
        offset += 16
        if captured > snaplen or offset + captured > len(raw):
            raise ValueError("pcap record length is invalid")
        frame = raw[offset : offset + captured]
        offset += captured
        yield frame, original


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <capture.pcap>", file=sys.stderr)
        return 2
    path = Path(sys.argv[1])
    count = 0
    for frame, original_len in packets(path):
        if len(frame) < 14:
            continue
        if frame[6:12] == SOURCE and frame[12:14] == ETHER_TYPE:
            count += 1
            if original_len < 60 or len(frame) < 60:
                print("wire witness is shorter than the Ethernet minimum", file=sys.stderr)
                return 1
            if WITNESS not in frame[14:]:
                print("wire witness has the wrong payload", file=sys.stderr)
                return 1
    if count == 0:
        print(
            "wire witness missing: no source=02:00:00:00:00:01 ether=0x88b5 frame",
            file=sys.stderr,
        )
        return 1
    print(f"hw-wire-check: pcap witness frames={count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
