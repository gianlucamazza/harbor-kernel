#!/usr/bin/env python3
"""Inject one raw Ethernet frame into QEMU's socket netdev peer."""

import argparse
import socket
import struct
import time


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--deadline", type=float, default=10.0)
    parser.add_argument("--delay", type=float, default=2.0)
    args = parser.parse_args()

    deadline = time.monotonic() + args.deadline
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((args.host, args.port), timeout=1.0) as peer:
                time.sleep(args.delay)
                # QEMU -netdev socket uses a four-byte network-order frame
                # length followed by the raw Ethernet frame.
                frame = bytes.fromhex(
                    "02000000000102000000000288b5"
                    "686172626f722d7135652d7278"
                )
                frame += bytes(60 - len(frame))
                peer.sendall(struct.pack("!I", len(frame)) + frame)
                return 0
        except OSError:
            time.sleep(0.1)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
