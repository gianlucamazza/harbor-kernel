#!/usr/bin/env bash
# Read the durable-store partition (ADR-0066) and report both slots.
#
# The independent half of the power-cycle evidence: after the board claims
# `durable-media: flushed slot=… seq=…`, this reads the same bytes off the
# physical card with nothing of the kernel in the loop — dd + a CRC check —
# so the claim and the media can disagree loudly.
set -euo pipefail

DEV="${1:?usage: $0 /dev/sdX (the whole card)}"

part=""
while read -r name ptype; do
	if [[ "${ptype}" == "0x7f" ]]; then
		part="/dev/${name}"
		break
	fi
done < <(lsblk -nro NAME,PARTTYPE "${DEV}")

if [[ -z "${part}" ]]; then
	echo "error: no durable-store partition (type 0x7f) on ${DEV}" >&2
	echo "hint: scripts/host/durable-partition.sh ${DEV}" >&2
	exit 1
fi

echo "durable store: ${part}"

# Slot geometry mirrors kernel_core::durable_media: header at sector 0/16,
# 8 payload sectors after each header.
window="$(mktemp)"
trap 'rm -f "${window}"' EXIT
# dd writes to stdout and the user's shell owns the redirection: root
# writing into a user-owned file in sticky /tmp is exactly what
# fs.protected_regular refuses. SC2024 warns the redirect is unprivileged —
# here that is the point, only the *read* needs root.
# shellcheck disable=SC2024
sudo dd if="${part}" bs=512 count=32 status=none >"${window}"
python3 - "${window}" <<'PY'
import sys, struct, zlib

with open(sys.argv[1], "rb") as f:
    data = f.read(32 * 512)
if len(data) < 32 * 512:
    print(f"error: short read ({len(data)} bytes)", file=sys.stderr)
    sys.exit(1)

MAGIC = int.from_bytes(b"DMH1", "little")

def slot(name, header_sector, payload_sector):
    hdr = data[header_sector * 512:(header_sector + 1) * 512]
    payload = data[payload_sector * 512:(payload_sector + 8) * 512]
    magic, version = struct.unpack_from("<II", hdr, 0)
    seq, crc = struct.unpack_from("<QI", hdr, 8)
    if magic != MAGIC:
        print(f"slot={name} empty (no DMH1 magic)")
        return
    ok = "ok" if zlib.crc32(payload) == crc else "BAD"
    durb = "durb=ok" if payload[:4] == b"DURB" else "durb=BAD"
    print(f"slot={name} seq={seq} version={version} crc={ok} {durb}")

slot("A", 0, 1)
slot("B", 16, 17)
PY
