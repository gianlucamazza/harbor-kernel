---
id: 0096
title: Gates that do not depend on remembering
status: accepted
date: 2026-08-11
accepted: 2026-08-11
related: [0001, 0050, 0058, 0087, 0092]
---

# ADR-0096: Gates that do not depend on remembering

## Acceptance status

**Accepted** (2026-08-11), on delegated authority (the owner asked what was
still missing and then asked for it to be fixed; owner delegated acceptance for
the work that answer named).

## Problem

Three rules in this project are right, written down, and enforced by nobody.
Each is a sentence a person has to remember at the moment it matters.

1. **A mutation run before a boundary-moving commit** (ADR-0058, and
   `run-mutants.sh`'s own header). The rule about the _file list_ has a gate —
   the script refuses an artifact that did not cover its own list. The rule
   about **running it** has none. K8 landed queues, per-core timers,
   EL0-on-CPU1 and work stealing across twenty-odd commits, all `done (HW)`,
   and the next run found **fourteen** survivors in code nobody had written
   that week. Every one was killable by a test the day it landed.

2. **`hw-transcript-check.sh` had no Makefile target.** It is the only gate
   that sees what QEMU cannot — TLB fills are speculative on silicon and not in
   TCG, so the ADR-0050 staleness class is only ever red there — and it was
   invoked by typing its path from memory.

3. **A skipped gate exits 0.** `ALLOW_MIRI_SKIP`, `ALLOW_SHELLCHECK_SKIP` and
   `ALLOW_BOOT_SKIP` print `SKIPPED` on stderr and pass. That is right for a
   workstation without QEMU. In CI it is a green gate that never ran, and the
   only thing standing between the two is that nobody has set the variable yet.

The common shape: **a rule whose enforcement is a habit.** This project spends
its effort on the opposite — every ADR names the gate that would catch its own
reversal — so these three are the exception, and they are the exception in the
place where it costs most.

## Decision

### 1. `make mutation-freshness` — the stamp against the surface

A clean `make mutants` writes `docs/mutation-stamp.toml`: date, commit, mutant
count, survivors, baseline. `scripts/check/mutation-freshness.sh` runs
`cargo mutants --list` — which parses and does not execute, about two seconds —
and fails when the count differs from the stamp. It is a `make check`
prerequisite.

**The count, not a hash and not a date.** A hash of the scope files goes red on
a comment; a date goes red on the calendar rather than on a change. The mutant
count moves when, and only when, there is new mutable surface: a function, a
branch, an operator. That is exactly the event that makes a previous run stale.

**What it does not do, stated because a gate that overclaims is worse than
none:** it catches surface that _appeared_, not tests that stopped killing what
was already there. Only a run answers the second question. This gate makes the
run's absence visible; it does not stand in for it.

The scope list is read out of `run-mutants.sh` rather than duplicated, so the
gate cannot end up measuring a different surface than the run covers, and it
refuses to proceed if it parses fewer than ten files — a truncated scope would
otherwise pass quietly.

**One criterion, everywhere.** The gate needs `cargo-mutants`, so CI installs
it, the same way it already installs shellcheck and an Arch-packaged QEMU for
gates that would otherwise report having checked nothing. A fallback for hosts
without the engine was written and then deleted: it compared the scope files
through git instead, which is a *different question* with a different answer,
and a gate that answers differently depending on where it runs is two gates —
with the one running in CI being the one nobody can reproduce locally. Missing
the tool is a failure, not a second mode.

### 2. `make hw-check TRANSCRIPT=…`

The hardware gate gets a target. It also gets something it was missing: the
capture's `src=` is compared against the tree, and a mismatch is announced
before the assertions run.

This matters because the oracle **grows**. A capture from before ADR-0090 has
no force-kill line, so today's oracle fails it — correctly, and for a reason
that has nothing to do with the board. Without the note, a stale capture reads
as a hardware regression. It is a warning and not a refusal: deliberately
re-checking an archived transcript against a newer oracle is a legitimate
thing to do.

### 3. A skip is a workstation affordance, never a CI outcome

Every `ALLOW_*_SKIP` path now refuses when `CI=true`. The skip stays for the
developer without QEMU or nightly; in CI, a missing tool is a failure, because
the alternative is a pipeline that reports a gate it did not run.

## Consequences

- `make check` gains one prerequisite and about two seconds.
- The first `mutation-freshness` failure a contributor sees will be after they
  add a branch to a scope module. The message names the run to make, and
  `make mutants` rewrites the stamp itself — the stamp is never edited by hand.
- `docs/mutation-stamp.toml` is tracked. It answers a question about the
  repository's history ("has anyone run this since the surface moved?"), not
  about a working copy, so it belongs in git rather than beside `mutants.out/`.

## Gates

| Check                                 | Evidence                                                                                                                                       |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Staleness is now a failure            | `make mutation-freshness`, **seen red**: with the stamp at 612, one added `const fn` in a scope module reports `614 mutants, stamp says 612`, exit 1. And seen *not* red for an `if false { … }` in the same file — no decision to break, no new surface, which is the gate tracking what it claims to |
| The hardware gate is reachable        | `make hw-check` with no `TRANSCRIPT` names what it needs; with one, it runs and prints the capture's provenance                                |
| A skip cannot be a CI pass            | **seen red**: `CI=true ALLOW_BOOT_SKIP=1` with a missing emulator reports `a skip is refused in CI`, exit 1                                    |
| The gate measures the run's own scope | it parses `FILES` out of `run-mutants.sh` and refuses fewer than ten entries                                                                   |
