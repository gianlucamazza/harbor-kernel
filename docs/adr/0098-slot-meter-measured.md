---
id: 0098
title: The slot meter is measured, not remembered
status: accepted
date: 2026-08-11
accepted: 2026-08-11
related: [0001, 0006, 0026, 0058, 0076, 0083, 0085, 0087, 0090, 0096]
---

# ADR-0098: The slot meter is measured, not remembered

## Acceptance status

**Accepted** (2026-08-11), on delegated authority: the owner asked whether the
kernel architecture was complete, and then asked for the work that answer named
to be planned and completed ([ADR-0096](0096-gates-that-do-not-depend-on-remembering.md)
was accepted the same way, for the same reason).

## Problem

[ADR-0085](0085-k5-density-residual-design.md) §2 names three density meters and
puts **slots** first: concurrent agents ≤ `MAX_TASKS` − idle − infrastructure.
It then forbids raising `MAX_TASKS` as a density win, and `oracle-census.sh` was
written to enforce that. The gate compares three things a machine can settle —
source constant, architecture table, documented last-raise reason — and then
guards the headroom with a fourth number that no machine settles:

```sh
# Product peak concurrent slots after composition steadies (not a measurement):
#   idle0 + idle1 + console-server + beacon + chirp = 5
readonly PRODUCT_PEAK_SLOTS=5
```

Its own comment says _not a measurement_. Three consequences, all live today:

1. **The number drifts silently.** The comment two lines down still reads
   "Today 5/52" while the ceiling it compares against is **54**; the file
   carries two stacked `Last justified raise:` lines (ADR-0083, then ADR-0090)
   where only the last one is true. Nothing went red, because nothing reads
   them.
2. **The guard cannot fire for the reason it exists.** A composition that
   admits a sixth or a fifteenth agent moves the real peak and leaves
   `PRODUCT_PEAK_SLOTS` at 5. The ratio check keeps passing on a number that
   describes the composition of 2026-08-10.
3. **The K5-H trigger is undecidable.** ADR-0085 §3 releases K5-H (and, through
   it, K5-B) on a _slot wall_: "product composition needs concurrent agents
   that K5-S + honest `MAX_TASKS` and a paid K5-H still cannot host". The
   roadmap's next row is `K5-H design (if slot wall)`. Nothing in the tree
   measures how close the product is to that wall, so the trigger is settled by
   opinion.

This is [ADR-0096](0096-gates-that-do-not-depend-on-remembering.md)'s shape in
the one gate ADR-0096 did not look at: a rule whose enforcement is a habit. It
is also [ADR-0087](0087-oracle-waits-and-the-hosts-verdict.md)'s shape — a
verdict that depends on what someone knew at the time rather than on what ran.

## Decision

### 1. The kernel counts its own occupancy (pure)

`kernel_core::tasks::Tasks<N>` gains two readings next to the counters it
already keeps (`overwrites`, `blocked_count`, `block_events`):

| Reading        | Meaning                                                   |
| -------------- | --------------------------------------------------------- |
| `live_count()` | Slots not `Empty` **right now**, idle identities included |
| `peak_slots()` | High-water mark of `live_count()` since boot; never reset |

Both are pure and host-tested. `live_count` scans, exactly like
`blocked_count` — the table is 54 entries and the caller is a rate-limited
print, so a cached counter would buy nothing and add a second source of truth
to keep honest.

**Where the watermark moves.** Occupancy rises in exactly three places —
`start` (CPU 0's idle), `start_cpu1` ([ADR-0076](0076-k8-per-core-queues-first-slice.md)),
and `admit_on` — and falls in exactly one, `switch_on(Switch::Exit)`. The peak
is therefore updated at those three, after the state is written, and nowhere
else. A fourth occupying path added later without touching the watermark is a
bug this ADR's tests are written to catch: `peak_slots` is asserted to equal
the largest `live_count` a test sequence ever passes through, not merely to be
"at least" it.

**Idle counts.** The meter reports what the table holds, not what a reader
would like it to hold: two of the slots are CPU 0's and CPU 1's idle
identities, and pretending otherwise would make the census subtract a constant
it also has to remember. ADR-0085's meter subtracts idle and infrastructure to
get _agents_; the census does that subtraction in the open, from a number the
kernel printed.

### 2. The product image prints it

The `invariants:` line in `console_loop` — the beacon that already exists in
**both** images so the product path is asserted in the image that ships
(excellence C-6) — gains one field:

```
invariants: overwrites=0 abandoned=0 faults=0 blocked=1 frames_free=512 preempts=0 slots=5/6
```

`slots=<live>/<peak>`. One more cheap read on a line that is already
rate-limited by the tick report, in the image the SD card gets. No new oracle
line, and nothing behind `feature = "oracle"`: the number that governs product
density has to come from the product.

### 3. The census reads the measurement

`scripts/check/oracle-census.sh` stops carrying `PRODUCT_PEAK_SLOTS` as a
constant. It runs the product boot check's transcript — the same run
`make product-boot-check` already performs — and takes the **largest `peak`
field** the product printed. From there the existing rules are unchanged in
intent and finally honest in input:

- product peak × `PRODUCT_CEILING_RATIO` ≤ `MAX_TASKS` (headroom floor);
- the three-way agreement between source, architecture table and the
  documented raise reason stays exactly as it was.

**It refuses to guess.** A transcript with no `slots=` field fails the gate; it
does not fall back to a remembered number. That is the whole point — a gate
that silently substitutes an old constant for a missing measurement is the
thing being removed.

**Ordering.** `oracle-census` runs after `product-boot-check` in `make check`
so the transcript exists. The census re-runs the boot itself when invoked
alone, rather than depending on a file someone produced earlier — a gate that
reads a stale artefact is the same failure in a new place.

### 4. The K5-H trigger gets an address

ADR-0085 §3's _slot wall_ is now a comparison between two printed numbers
rather than a judgement: product `peak` against `MAX_TASKS` less the oracle
tax. Until the measured peak approaches the ceiling, **K5-H stays deferred and
this ADR is the evidence for that** — the roadmap row stops saying "if slot
wall" as an open question and starts citing the measurement.

This ADR does **not** open K5-H, does not touch the pair
([ADR-0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md)), and does
not raise `MAX_TASKS`. It is the meter, not the mechanism.

## Alternatives

| Option                                                  | Why not                                                                                                                                                                                            |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fix the stale comment (`5/52` → `5/54`) and move on     | Repairs today's drift and leaves the mechanism that produced it. The next composition change breaks it again, silently, and the gate keeps passing                                                 |
| Count occupancy in `src/sched` instead of `kernel_core` | The scheduler table is the pure half on purpose; a counter maintained above it cannot be host-tested against the transitions that move it                                                          |
| A separate `density: slots …` oracle line               | It would live behind `feature = "oracle"` and describe the demo image. The census governs the **product**, and ADR-0085 §4 already learned this: an oracle-only number is a number about the demos |
| Have the census parse `admit` call sites statically     | Call sites are not concurrency — the script already says so about its informational spawn count. Sequential demos that exit would read as a wall that is not there                                 |

## Consequences

- One field on one line in the shipped image; one scan per tick report.
- `make check` gains a real dependency order (`product-boot-check` before
  `oracle-census`); the census is slower alone because it boots.
- The first run of the new census settles what the constant claimed. If the
  measured peak differs from 5, **the measurement wins** and the comment that
  said 5 is deleted rather than argued with.
- Mutable surface in `kernel-core` moves, so [ADR-0058](0058-adr-amendments-and-mutation-freshness.md)'s
  run and [ADR-0096](0096-gates-that-do-not-depend-on-remembering.md)'s stamp
  are part of this slice, not a follow-up.

## The gate that would catch this ADR's reversal

`make oracle-census`. Delete the measurement and put a constant back, and the
gate reads a `slots=` field that is no longer printed and fails; leave the
field and stop updating the watermark, and the host tests in
`crates/kernel-core/src/tasks.rs` fail on the exact-peak assertion. The
reversal that has no gate — nobody ever composing more agents — is the one the
census exists to notice.

## Evidence

| Level | What                                                                                                           |
| ----- | -------------------------------------------------------------------------------------------------------------- |
| Host  | `live_count` / `peak_slots` unit tests: admit/exit sequences, CPU 1's idle, the parked slot, exact watermark   |
| QEMU  | `make product-boot-check` asserts the `slots=` field in the product image; `make oracle-census` reads its peak |
| Mutation | 621 mutants (`mutation-freshness` seen red at 621 against a stamp of 612 — the gate catching this very slice). Three of `note_occupancy`'s four mutants die on the exact-peak assertions; the fourth (`>` → `>=`) is equivalent and raises the baseline to 22. The full run first filed that one as a **timeout**; re-running it alone took six seconds, so the per-mutant floor — conditional on `MUTANTS_JOBS` until now — became unconditional ([ADR-0087](0087-oracle-waits-and-the-hosts-verdict.md)'s rule, third gate) |
| HW    | None required — the meter is not a boundary. The field rides along on the next Pi stamp's `invariants:` line   |
