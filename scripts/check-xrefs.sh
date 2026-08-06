#!/usr/bin/env bash
# Cross-references that are facts in two places with nothing comparing them.
#
# `docs/mmu.md` records what happens to such a fact: "This table used to exist
# in both files, and both copies went stale together — which is exactly what a
# duplicated fact does." Each check below is one of those pairs. All of them
# were true when this script was written, which is the point: they were true by
# attention, and attention does not survive a rename.
set -euo pipefail

cd "$(dirname "$0")/.."

violations=0
note() {
	echo "xrefs: $1" >&2
	violations=$((violations + 1))
}

# 1. Every relative markdown link resolves. A doc that points at a file someone
#    moved is worse than one that says nothing: it reads as a reference.
links=0
while IFS= read -r file; do
	dir="$(dirname "${file}")"
	while IFS= read -r target; do
		[[ -z "${target}" ]] && continue
		links=$((links + 1))
		[[ -e "${dir}/${target}" ]] || note "${file} links to '${target}', which does not exist"
	done < <(grep -oE '\]\([^)#]+\)' "${file}" |
		sed 's/^](//; s/)$//' |
		grep -vE '^(https?|mailto):' || true)
done < <(find . -name '*.md' -not -path './target/*' -not -path './.git/*')

# 2. Every `ADR-NNNN` named anywhere is an ADR that exists. Code comments cite
#    these as the reason for a design; a citation to nothing is a dead end at
#    exactly the moment someone is asking why.
while IFS= read -r id; do
	num="${id#ADR-}"
	compgen -G "docs/adr/${num}-*.md" >/dev/null ||
		note "${id} is cited but no docs/adr/${num}-*.md exists"
done < <(grep -rhoE 'ADR-[0-9]{4}' src crates docs README.md 2>/dev/null | sort -u)

# 3. The ADR index repeats each ADR's status. Two copies, one table.
adrs=0
for adr in docs/adr/0*.md; do
	adrs=$((adrs + 1))
	num="$(basename "${adr}" .md | cut -d- -f1)"
	file_status="$(sed -n 's/^status: *//p' "${adr}" | head -1)"
	index_status="$(awk -F'|' -v n="${num}" '$2 ~ "\\["n"\\]" { gsub(/ /, "", $4); print $4 }' docs/adr/README.md)"
	[[ -n "${index_status}" ]] ||
		note "$(basename "${adr}") is not listed in docs/adr/README.md"
	[[ -z "${index_status}" || "${file_status}" == "${index_status}" ]] ||
		note "$(basename "${adr}"): status is '${file_status}', the index says '${index_status}'"
	# 4. …and its own id, which is the filename in a second place.
	frontmatter_id="$(sed -n 's/^id: *//p' "${adr}" | head -1)"
	[[ "${frontmatter_id}" == "${num}" ]] ||
		note "$(basename "${adr}"): frontmatter id is '${frontmatter_id}'"
done

if [[ "${violations}" -ne 0 ]]; then
	echo "xrefs: ${violations} broken cross-reference(s)" >&2
	exit 1
fi

echo "xrefs: clean (${links} links, ${adrs} ADRs, ids and statuses agree)"
