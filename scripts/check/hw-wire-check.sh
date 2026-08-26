#!/usr/bin/env bash
# Full silicon networking gate: boot transcript, physical witness, image
# provenance and store composition must describe the same product artifact.
set -euo pipefail

TRANSCRIPT="${1:-}"
PCAP="${2:-}"
IMAGE="${3:-}"
ELF="${4:-}"
EXPECTED_HASH="${5:-}"

if [[ -z "${TRANSCRIPT}" || -z "${PCAP}" || -z "${IMAGE}" || -z "${ELF}" || -z "${EXPECTED_HASH}" ]]; then
	printf 'usage: %s <transcript.log> <capture.pcap> <kernel8.img> <kernel8.elf> <image-sha256>\n' "$0" >&2
	exit 2
fi
for path in "${TRANSCRIPT}" "${PCAP}" "${IMAGE}" "${ELF}"; do
	[[ -f "${path}" ]] || { echo "hw-wire-check: FAIL — missing ${path}" >&2; exit 1; }
done

actual_hash="$(sha256sum "${IMAGE}" | awk '{print $1}')"
[[ "${actual_hash}" == "${EXPECTED_HASH}" ]] || {
	echo "hw-wire-check: FAIL — image hash ${actual_hash} != expected ${EXPECTED_HASH}" >&2
	exit 1
}
echo "hw-wire-check: image sha256=${actual_hash}"

./scripts/check/hw-transcript-check.sh "${TRANSCRIPT}"
./scripts/check/hw-store-audit.sh "${TRANSCRIPT}" "${IMAGE}" "${ELF}"
python3 ./scripts/check/hw-wire-check.py "${PCAP}"

# RX is intentionally content-observed rather than content-filtered. The
# transport must carry arbitrary valid Ethernet frames; the gate only requires
# that the running image reported its header and first payload bytes.
grep -Eq 'genet: rx frame len=[0-9]+ src=0x[0-9a-f]+.*ether=0x[0-9a-f]+.*magic=0x[0-9a-f]+' "${TRANSCRIPT}" || {
	echo "hw-wire-check: FAIL — RX content diagnostic missing from transcript" >&2
	exit 1
}
echo "hw-wire-check: serial RX content diagnostic present"
