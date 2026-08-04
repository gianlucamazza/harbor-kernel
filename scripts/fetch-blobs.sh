#!/usr/bin/env bash
# Download the pinned Raspberry Pi platform firmware blobs required to boot,
# and verify them against hashes committed to this repository.
# These are closed-source VideoCore stages; see docs/blobs.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${ROOT}/third_party/blobs"
EXPECTED="${OUT}/EXPECTED.sha256"

# Pin to a known firmware release (update deliberately, with docs/blobs.md).
FIRMWARE_TAG="${FIRMWARE_TAG:-1.20250430}"
BASE_URL="https://github.com/raspberrypi/firmware/raw/${FIRMWARE_TAG}/boot"

# Set to fetch a tag whose hashes are not in EXPECTED.sha256 yet — the only
# legitimate use is bumping the pin, where you then review and commit the sums.
ALLOW_UNVERIFIED="${ALLOW_UNVERIFIED:-}"

FILES=(
	start4.elf
	fixup4.dat
)

if [[ ! -f "${EXPECTED}" ]]; then
	echo "error: missing ${EXPECTED}" >&2
	exit 1
fi

pinned_tag="$(sed -n 's/^firmware_tag=//p' "${EXPECTED}")"
if [[ "${pinned_tag}" != "${FIRMWARE_TAG}" && -z "${ALLOW_UNVERIFIED}" ]]; then
	echo "error: requested tag ${FIRMWARE_TAG} but ${EXPECTED} pins ${pinned_tag}" >&2
	echo "hint: bumping the pin is deliberate — see the header of EXPECTED.sha256" >&2
	exit 1
fi

mkdir -p "${OUT}"
echo "Fetching firmware tag ${FIRMWARE_TAG} → ${OUT}"

# Download to a staging directory: nothing reaches the blob directory, and so
# nothing can reach an SD card, before its hash has been checked.
staging="$(mktemp -d)"
trap 'rm -rf "${staging}"' EXIT

for f in "${FILES[@]}"; do
	echo "  ${f}"
	curl -fsSL "${BASE_URL}/${f}" -o "${staging}/${f}"
done

if [[ -n "${ALLOW_UNVERIFIED}" ]]; then
	echo
	echo "ALLOW_UNVERIFIED set — not checking hashes. Sums of what arrived:"
	(cd "${staging}" && sha256sum "${FILES[@]}")
	echo
	echo "Review these, then put them in ${EXPECTED} with firmware_tag=${FIRMWARE_TAG}."
else
	# A git tag is mutable, and a mirror or a machine-in-the-middle can serve
	# anything. This is the only step that makes the download trustworthy.
	echo "Verifying against ${EXPECTED}"
	if ! (cd "${staging}" && grep -v '^\(#\|firmware_tag=\|$\)' "${EXPECTED}" | sha256sum --check --status); then
		echo "error: firmware hash mismatch — refusing to install" >&2
		echo "expected:" >&2
		grep -v '^\(#\|firmware_tag=\|$\)' "${EXPECTED}" >&2
		echo "got:" >&2
		(cd "${staging}" && sha256sum "${FILES[@]}") >&2
		exit 1
	fi
	echo "OK: hashes match the committed values"
fi

for f in "${FILES[@]}"; do
	install -m 0644 "${staging}/${f}" "${OUT}/${f}"
done

# The manifest records provenance — what was fetched, when. It is not the
# integrity check; EXPECTED.sha256 is.
{
	echo "firmware_tag=${FIRMWARE_TAG}"
	echo "fetched_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
	echo "verified=$([[ -n "${ALLOW_UNVERIFIED}" ]] && echo no || echo yes)"
	for f in "${FILES[@]}"; do
		echo "${f}=$(sha256sum "${OUT}/${f}" | awk '{print $1}')"
	done
} >"${OUT}/MANIFEST.txt"

echo "Wrote ${OUT}/MANIFEST.txt"
cat "${OUT}/MANIFEST.txt"
