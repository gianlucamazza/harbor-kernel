---
id: 0109
title: Hardware evidence is tracked; the capture that carried it is not
status: proposed
date: 2026-08-17
related: [0026, 0058, 0087, 0096]
---

# ADR-0109: Hardware evidence is tracked; the capture is not

## Status

**Proposed** (2026-08-17). Answers the question filed as F-12 in the
[2026-08-17 excellence review](../reviews/2026-08-17-excellence.md) and issue
#84: _are hardware transcripts a tracked record?_

## Context

Every `done (HW)` claim in this project ends in a filename. `docs/roadmap.md`
and `docs/verification.md` cite 36 artefacts between them — 34 serial captures
and 2 pcaps — by name, as the thing that makes the claim evidence rather than
prose.

`.gitignore` line 25 is `/.serial-log/`. **Zero of those 36 files are tracked.**
They exist on one laptop. So:

- `make hw-check TRANSCRIPT=.serial-log/20260817-105728.log` cannot be run by
  anyone who clones this repository, including by its author on another machine;
- a `done (HW)` row survives only as an excerpt someone pasted into
  `verification.md` by hand, and a hand-pasted excerpt is exactly the copy of a
  fact that [ADR-0058](0058-adr-amendments-and-mutation-freshness.md) and
  `make xrefs` exist to refuse one directory over;
- the strongest evidence this project has — the `0x88b5` frame that ended
  twenty-five negative GENET slices — is one `rm` from being an anecdote.

This is the project's own standard applied to the project's own record.
[ADR-0096](0096-gates-that-do-not-depend-on-remembering.md) says a gate must
not depend on remembering; a gate whose input lives on one machine depends on
one machine remembering.

### Why nobody tracked them

Because the obvious move is wrong, and its wrongness is measurable. The 36
cited artefacts are **20 MB**. `.git` is 5.6 MB. Committing them would
quadruple the repository, permanently, and grow it again at every hardware
session.

But 20 MB is not 20 MB of evidence. Profiling the largest capture,
`20260815-092435.log` (4.8 MB), by line shape:

```
  35982  N:N:N.N ticks=N
  35982  N:N:N.N invariants: overwrites=N abandoned=N faults=N blocked=N frames_free=N preempts=N slots=N/N
      2  N:N:N.N smp: coreN alive
      2  N:N:N.N unmap: remapped and freed
      ...
```

**71 964 of ~72 000 lines are the idle heartbeat.** The capture is a tape of a
metronome with the record buried in it. What the claims cite — `genet: rx
complete len=160`, `loader: store n=5 image`, `smp: steal ok`, `reset: PowerOn`
— is the remaining 0.05%.

Tracking the tape to keep the record is what made this look expensive. It is
not expensive; it was measured wrong.

## Decision

### 1. The capture is ephemeral. The evidence is tracked.

`.serial-log/` stays ignored. It is a working directory: raw captures, partial
sessions, the boot that was interrupted because the cable was in backwards.
Nothing there is a record.

A new tracked directory, **`docs/evidence/`**, holds one file per cited
capture. It is what `verification.md` and `roadmap.md` cite, and what
`make hw-check` runs against on a clone.

### 2. The heartbeat is collapsed, not deleted

`scripts/host/hw-evidence.sh` derives an evidence file from a capture: every
non-heartbeat line verbatim, and each contiguous run of heartbeat lines reduced
to **its first line, a count, and its last line**.

Collapsed rather than deleted because the heartbeat line is not only noise. The
roadmap cites `slots=4/9` from it as the measured slot peak
([ADR-0098](0098-slot-meter-measured.md)), and `frames_free` is how a leak
would show. Keeping the first and the last keeps the delta across the boot —
which is the claim — and keeping the count keeps the reader honest about how
long the board idled.

Measured on the 34 cited captures: **20 MB → 349 KiB**, a 60× reduction, in
plain diffable greppable text. Small enough that compressing it would cost more
in tooling than it saves in bytes, which is why this is not gzip and not
git-lfs.

### 3. Provenance travels with the evidence

An extracted file could be edited by hand afterwards, and a record that can be
quietly improved is worse than no record. Each evidence file carries a header
naming the source capture, **its sha256**, the extractor's version, and the
extraction date.

That gives two different readers what each can use: whoever holds the capture
can re-derive and compare byte for byte; whoever does not at least reads a
stated provenance instead of an anonymous excerpt.

### 4. The pcaps are tracked as they are

The two pcaps are 460 KiB and are the only evidence in the project that a frame
reached a wire rather than a register. They are binary, immutable, and never
diffed. They are committed unmodified.

## Consequences

### Positive

- `make hw-check` and `make hw-store-audit` run on a clone.
- A `done (HW)` row can be re-checked against the oracle as it grows, which is
  the thing `hw-transcript-check.sh` already warns about and could not offer.
- The excerpt in `verification.md` stops being the record and goes back to
  being what it should be — a reader's summary of a record that exists.

### Negative / costs

- `docs/evidence/` grows at every hardware session. At ~10 KiB per capture that
  is affordable for years, but it is not free and it is not revocable: git
  history keeps what lands in it.
- The extraction is lossy by construction. A failure whose signature is _inside_
  the elided heartbeat — a `frames_free` sagging in the middle of a run rather
  than at its end — would not survive into the evidence file. This is a real
  blind spot and it is declared rather than mitigated: the capture is where a
  suspicion goes, and the capture still exists on the machine that took it for
  as long as it matters.

### Refused alternatives

| Alternative                                           | Why not                                                                                                     |
| ----------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Track the captures whole                              | 20 MB, permanent, 99.95% metronome                                                                          |
| gzip or git-lfs the captures                          | Keeps the metronome, adds tooling, loses `grep` and `diff`                                                  |
| Declare the `verification.md` excerpt _is_ the record | It is a hand-made copy of a fact with nothing comparing it to the original — the failure `xrefs` exists for |
| Keep only the last boot of each capture               | Still megabytes (measured: 10 MB), because a single boot is mostly heartbeat too. Solves the wrong axis     |

## The gate that catches its own reversal

`scripts/check/hw-evidence.sh`, in `make check`:

- every `YYYYMMDD-HHMMSS*.log`/`.pcap` cited in `docs/` has a file in
  `docs/evidence/` — a citation without a record is refused;
- every evidence file's header parses, and names a capture and a sha256;
- **where the capture is present** on the machine running the gate, the
  evidence is re-derived and compared. A hand-edited evidence file is red on the
  machine that can tell, and carries a stated hash everywhere else.

The asymmetry is deliberate and is the honest limit of this design: CI can
check that a record exists and is well-formed, but only the machine that holds
the capture can check that the record is _true_.

## References

- [ADR-0087](0087-oracle-waits-and-the-hosts-verdict.md) — what a transcript is judged by
- [ADR-0096](0096-gates-that-do-not-depend-on-remembering.md) — a gate must not
  depend on one machine remembering
- [`../verification.md`](../verification.md) — where the claims cite
- Issue #84 / [2026-08-17 review](../reviews/2026-08-17-excellence.md) F-12
