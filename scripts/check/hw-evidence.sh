#!/usr/bin/env bash
# Every hardware claim cites a record this repository actually holds (ADR-0109).
#
# ## The failure this closes
#
# `.gitignore` line 25 ignores `/.serial-log/`, and 38 files in it were cited by
# name across `docs/` as the evidence behind `done (HW)`. None were tracked. A
# clone could read the claim and could not read what it rests on, and
# `make hw-check TRANSCRIPT=…` had no input outside one laptop. ADR-0096 says a
# gate must not depend on remembering; a gate whose input lives on one machine
# depends on that machine surviving.
#
# ## What it asserts
#
# - **No citation without a record.** Any `YYYYMMDD-HHMMSS*.log|.pcap` named in
#   `docs/` has a file in `docs/evidence/`.
# - **No record without a citation.** The reverse, so the directory cannot grow
#   a quiet archive of files no claim uses — the same argument `oracle-census`
#   makes about demos nobody boots.
# - **Every evidence log states its provenance.** The header names the capture
#   and its sha256, so an excerpt is never anonymous.
# - **Where the capture is present, the record is re-derived and compared.**
#
# ## What it cannot assert
#
# That an evidence file is *true*, on a machine without the capture. CI can see
# that a record exists, is well-formed and claims a hash; only the machine that
# holds the capture can see that the hash is the capture's and the body is what
# the extractor produces from it. That asymmetry is ADR-0109's declared limit,
# and it is why the comparison is conditional rather than skipped quietly: the
# summary says how many were verified and how many were taken on their word.
set -euo pipefail

cd "$(dirname "$0")/../.."

readonly EVIDENCE_DIR="docs/evidence"
readonly CAPTURE_DIR=".serial-log"
# `-boot2-k5s` is why the suffix allows digits: an earlier draft of this pattern
# matched neither of the two captures that carry one, and a citation scanner
# that silently misses a name is worse than none.
readonly NAME_RE='[0-9]{8}-[0-9]{6}[a-z0-9-]*\.(log|pcap)'

violations=0
note() {
	echo "hw-evidence: $1" >&2
	violations=$((violations + 1))
}

[[ -d "${EVIDENCE_DIR}" ]] || {
	echo "hw-evidence: FAIL — ${EVIDENCE_DIR} does not exist (ADR-0109)" >&2
	exit 1
}

cited="$(mktemp)"
held="$(mktemp)"
trap 'rm -f "${cited}" "${held}"' EXIT

# The evidence files themselves carry `capture: <name>` in their headers, so
# scanning them for citations would make every record cite itself and the
# orphan check vacuous.
grep -rhoE "${NAME_RE}" docs/ --include='*.md' |
	sort -u >"${cited}"
find "${EVIDENCE_DIR}" -maxdepth 1 -type f -printf '%f\n' | sort -u >"${held}"

while IFS= read -r name; do
	[[ -n "${name}" ]] || continue
	[[ -f "${EVIDENCE_DIR}/${name}" ]] ||
		note "'${name}' is cited in docs/ and there is no ${EVIDENCE_DIR}/${name}
  A claim whose evidence only exists on one laptop is a claim nobody can check.
  Derive it: scripts/host/hw-evidence.sh ${CAPTURE_DIR}/${name}"
done <"${cited}"

while IFS= read -r name; do
	[[ -n "${name}" ]] || continue
	grep -qxF "${name}" "${cited}" ||
		note "'${name}' sits in ${EVIDENCE_DIR} and no document cites it
  Either a claim lost its citation, or this is an archive. Neither is a record."
done <"${held}"

# Present on disk is not the same as held by the repository, and the gap is not
# hypothetical: a global excludes file with `*.log` in it made `git add -A`
# stage this directory's two pcaps and silently skip all 36 transcripts. Every
# check below would still have passed on the machine that wrote them, and the
# clone they exist for would have had nothing. So the record has to be *tracked*,
# and that is asked of git rather than of the filesystem.
if git rev-parse --git-dir >/dev/null 2>&1; then
	untracked="$(git ls-files --others --exclude-standard --ignored "${EVIDENCE_DIR}")"
	if [[ -n "${untracked}" ]]; then
		while IFS= read -r path; do
			[[ -n "${path}" ]] || continue
			note "${path} is on disk and git is ignoring it
  A record only this machine can read is what ADR-0109 exists to end.
  Check the rule:  git check-ignore -v ${path}"
		done <<<"${untracked}"
	fi
fi

verified=0
on_trust=0

for path in "${EVIDENCE_DIR}"/*.log; do
	[[ -e "${path}" ]] || continue
	name="$(basename "${path}")"

	capture_line="$(sed -nE 's/^# capture:[[:space:]]+(.+)$/\1/p' "${path}" | head -1)"
	sha_line="$(sed -nE 's/^# sha256:[[:space:]]+([0-9a-f]{64})$/\1/p' "${path}" | head -1)"

	if [[ -z "${capture_line}" || -z "${sha_line}" ]]; then
		note "${name}: header does not name a capture and a sha256
  Re-derive it rather than repairing the header by hand:
    scripts/host/hw-evidence.sh ${CAPTURE_DIR}/${name}"
		continue
	fi
	if [[ "${capture_line}" != "${name}" ]]; then
		note "${name}: header says it came from '${capture_line}'
  The filename and the provenance disagree; one of them is wrong."
		continue
	fi

	capture="${CAPTURE_DIR}/${name}"
	if [[ ! -f "${capture}" ]]; then
		on_trust=$((on_trust + 1))
		continue
	fi

	actual_sha="$(sha256sum "${capture}" | cut -d' ' -f1)"
	if [[ "${actual_sha}" != "${sha_line}" ]]; then
		note "${name}: the capture on this machine hashes ${actual_sha:0:12}…,
  the record claims ${sha_line:0:12}…. Two different files share one name."
		continue
	fi

	if ! diff -q <(./scripts/host/hw-evidence.sh --stdout "${capture}") "${path}" >/dev/null; then
		note "${name}: re-deriving it from ${capture} does not reproduce it
  The tracked record has been edited by hand, or the extractor changed without
  the records being regenerated. Diff:
    diff <(scripts/host/hw-evidence.sh --stdout ${capture}) ${path}"
		continue
	fi
	verified=$((verified + 1))
done

if [[ "${violations}" -ne 0 ]]; then
	echo "hw-evidence: ${violations} problem(s)" >&2
	exit 1
fi

echo "hw-evidence: clean ($(wc -l <"${held}") records; ${verified} re-derived from their capture, ${on_trust} on stated provenance)"
