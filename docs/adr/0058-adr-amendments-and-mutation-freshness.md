---
id: 0058
title: Process — ADR amendments and mutation freshness
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0001, 0049]
---

# ADR-0058: ADR amendments and mutation freshness

## Acceptance status

**Accepted** (2026-08-08), on delegated authority (excellence review 2026-08-08,
findings F-10/F-7; owner delegated acceptance of the review's needs-ADR
remediations).

## Problem 1 — the immutability rule and the practice disagree

CONTRIBUTING and `docs/README.md` say accepted ADRs are immutable, change only
via successor. Practice — four times now (0039, 0045, 0047, 0053) — rewrites
an accepted ADR's body after the implementing slice lands, reconciling stated
mechanism with built mechanism. The edits were honest; the rule made them
invisible.

## Decision 1 — the `amended:` field

Post-acceptance **reconciliation amendments** are permitted, narrowly: aligning
an accepted ADR's stated mechanism/evidence with what a later accepted ADR or
landed slice actually did, without changing the decision's substance. Every
such edit:

- adds/updates an `amended: YYYY-MM-DD` frontmatter field, and
- names the reconciling ADR or commit in the edited section.

Anything that changes the _decision_ still requires a successor. An edit to an
accepted ADR body without an `amended:` bump is a violation; review checks it
(a body diff is not mechanically derivable from tree state, so this half stays
review's job and is written down here rather than implied).

## Problem 2 — mutation evidence has no forcing function

`make mutants` is in neither `check` nor CI; its output is gitignored; the
on-disk artifact was a single-file scoped run indistinguishable from a full
one; the file list is hand-written and did not gain `taskcap.rs` when the new
authority module landed.

## Decision 2 — scope-validated, boundary-triggered mutation runs

1. `run-mutants.sh` **validates scope**: after a run it asserts the set of
   files in `mutants.out/mutants.json` equals its `--file` list, and fails
   otherwise — a scoped or interrupted run can no longer grade itself green.
2. The file list gains every module that decides authority: `taskcap.rs`
   joins it now; adding an authority-deciding module to `kernel-core` without
   adding it to the list is the same defect class as leaving a syscall out of
   `SECURITY.md`.
3. **Cadence:** a fresh full run is required before any commit that moves a
   boundary (new syscall/argument, new cap band, new authority module) — the
   PR template gains a checkbox. Deriving the list mechanically from source
   markers is a registered residual (ADR-0049), not a promise.

## Gates

| Check                  | Evidence                                                                            |
| ---------------------- | ----------------------------------------------------------------------------------- |
| Scoped run cannot pass | `run-mutants.sh` scope assert (seen red against the on-disk manifest-only artifact) |
| Amendment discipline   | review + `amended:` field present on 0039/0045/0046/0047/0053/0054 backfill         |
