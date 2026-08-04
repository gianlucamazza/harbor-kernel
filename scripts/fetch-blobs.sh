#!/usr/bin/env bash
# Download pinned Raspberry Pi platform firmware blobs required to boot.
# These are closed-source VideoCore stages; see docs/blobs.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${ROOT}/third_party/blobs"
# Pin to a known firmware release (update deliberately, with docs/blobs.md).
FIRMWARE_TAG="${FIRMWARE_TAG:-1.20250430}"
BASE_URL="https://github.com/raspberrypi/firmware/raw/${FIRMWARE_TAG}/boot"

FILES=(
  start4.elf
  fixup4.dat
)

mkdir -p "${OUT}"
echo "Fetching firmware tag ${FIRMWARE_TAG} → ${OUT}"

for f in "${FILES[@]}"; do
  dest="${OUT}/${f}"
  echo "  ${f}"
  curl -fsSL "${BASE_URL}/${f}" -o "${dest}.tmp"
  mv "${dest}.tmp" "${dest}"
done

{
  echo "firmware_tag=${FIRMWARE_TAG}"
  echo "fetched_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  for f in "${FILES[@]}"; do
    # sha256sum is portable on Linux; record for provenance.
    sum="$(sha256sum "${OUT}/${f}" | awk '{print $1}')"
    echo "${f}=${sum}"
  done
} > "${OUT}/MANIFEST.txt"

echo "Wrote ${OUT}/MANIFEST.txt"
cat "${OUT}/MANIFEST.txt"
