#!/usr/bin/env python3
"""Inject one raw Ethernet frame into QEMU's socket netdev peer.

## Exit status, and why it is three-valued

  0  the frame was written to QEMU's socket
  3  never connected — the harness could not run the experiment
  1  connected, and the write failed

The difference between 3 and 1 is the difference between a fact about the host
and a fact about the code, and it is the same distinction ADR-0087 draws for a
starved emulator. `qemu-virtio-check` went into `make check` and CI on
2026-08-17 and immediately reported *"deterministic peer could not inject RX"*
on the runner while the guest booted perfectly — modern probe ok, service up,
TX accepted and completed. Reporting that as a virtio failure would have been
reporting the runner as if it were the driver.

It also said nothing about *why*, which is why the last error is printed now:
a gate that fails without a reason costs the next reader the whole
investigation.
"""

import argparse
import socket
import struct
import sys
import time


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--deadline", type=float, default=10.0)
    parser.add_argument("--delay", type=float, default=2.0)
    args = parser.parse_args()

    deadline = time.monotonic() + args.deadline
    attempts = 0
    last_error: OSError | None = None
    while time.monotonic() < deadline:
        attempts += 1
        try:
            peer = socket.create_connection((args.host, args.port), timeout=1.0)
        except OSError as error:
            last_error = error
            time.sleep(0.1)
            continue
        # Connected. From here a failure is about the transfer, not the harness.
        with peer:
            try:
                time.sleep(args.delay)
                # QEMU -netdev socket uses a four-byte network-order frame
                # length followed by the raw Ethernet frame.
                frame = bytes.fromhex(
                    "02000000000102000000000288b5686172626f722d7135652d7278"
                )
                frame += bytes(60 - len(frame))
                peer.sendall(struct.pack("!I", len(frame)) + frame)
                return 0
            except OSError as error:
                print(
                    f"qemu-virtio-peer: connected to {args.host}:{args.port} "
                    f"and the write failed: {error}",
                    file=sys.stderr,
                )
                return 1

    print(
        f"qemu-virtio-peer: no connection to {args.host}:{args.port} in "
        f"{args.deadline:g}s over {attempts} attempts; last error: {last_error}",
        file=sys.stderr,
    )
    return 3


if __name__ == "__main__":
    raise SystemExit(main())
