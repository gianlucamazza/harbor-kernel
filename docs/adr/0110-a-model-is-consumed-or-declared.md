---
id: 0110
title: A pure model is consumed by the code that ships, or it names what will consume it
status: proposed
date: 2026-08-17
related: [0049, 0096, 0104, 0105, 0106]
---

# ADR-0110: A model is consumed, or declared

## Status

**Proposed** (2026-08-17). Answers F-3 of the
[2026-08-17 excellence review](../reviews/2026-08-17-excellence.md) and issue
#83.

## Context

`crates/kernel-core/src/genet.rs` is 3528 lines of pure, host-tested model.
`src/drivers/genet.rs` is 1071 lines of driver. The review claimed they had
diverged: `RingState`, `RingCursor`, `InterruptWork` and `ResetState` had no
consumer in `src/`, while `submit_one_tx` did its own producer-index
arithmetic. `docs/verification.md` cited the host tests over that model as
evidence for ADR-0105/0106 — evidence about code the silicon does not run.

### The first measurement was wrong

Counting `pub` items whose name never appears under `src/` gives **38 of 100**,
and that number is what the review reported. It is misleading. Most of those
items are reached _transitively_: `DescriptorWords` is not named by the driver,
but `write_descriptor` is, and it uses them. A reachability walk from every
identifier `src/` mentions, expanded through the model's own call graph, gives
the real figure:

**9 of 100 pub items are unreachable from the driver.** Not 38.

Correcting this mattered, because the plan built on 38 was to spend a
12–25 hour mutation run on code the silicon does not execute. On 9 items — one
of them a constant — that argument mostly evaporates.

### What the nine were, and what one of them was hiding

`RingCursor` modelled a bounded ring cursor, and its `advance()` wrapped at
`TOTAL_DESCRIPTORS` — **256**. Ring 0's TX ring carries `V5_Q0_TX_BD_CNT`
BDs — **128** — because Linux gives rings 1–4 thirty-two each out of the same
256, and `crates/kernel-core/src/genet.rs` says so eight lines from the
constant. `RingState::next` wraps correctly, at `layout.count`.

So there were two implementations of one rule, the second was wrong, and it had
a passing test asserting the wrong wrap. Nothing caught it because nothing ran
it. Had the driver adopted `RingCursor` — the review's own recommendation — it
would have walked off the end of ring 0.

That is the argument this ADR turns on. **An unconsumed model is not neutral.
It rots, and its green tests rot with it**, because a test written against the
model tests the model's belief rather than the device's behaviour.

### Why the driver cannot simply consume the rest

`src/drivers/genet.rs` is a **bring-up** driver, and deliberately so: it posts
one descriptor at index 0, never advances a producer, polls a bounded window
instead of taking an interrupt, and resets without ever having handed out a
frame token. It has no ring, no interrupt and no outstanding ownership. There
is nothing for `RingState`, `InterruptWork` or `ResetState` to do in it.

They are not divergence. They are the model of the **network service** — the P3
publication step ADR-0105 always named as later. Deleting them would burn
host-tested design work that the next slice needs; pretending the driver
consumes them would be false.

## Decision

### 1. Three outcomes, and a gate that tells them apart

Every `pub` item in a model file is one of:

| Outcome          | Meaning                                                                 |
| ---------------- | ----------------------------------------------------------------------- |
| **consumed**     | reachable from `src/`. Nothing to declare                               |
| **design-ahead** | not reachable, and its doc comment names the slice that will consume it |
| refused          | neither                                                                 |

`scripts/check/model-consumed.py`, in `make check`, computes the reachability
and refuses the third. This is the shape `docs/mutation-scope.toml` already
established a week ago — _in_scope / queued / exempt, each with a reason_ — with
one difference: the declaration lives in the item's own doc comment rather than
in a second file, so it cannot drift from what it describes.

`Design-ahead` is not a parking space. It is a claim that a **named roadmap
row** needs this item, and the row is where anyone can check whether that is
still true.

### 2. What was deleted

Four items, on the grounds that no slice needs them:

- **`RingCursor`** — a second, wrong copy of `RingState::next`. The advance has
  one implementation again.
- **`TX_FLUSH_BEFORE_DOORBELL: bool = false`** — a refuted hypothesis from the
  twenty-five negative slices, frozen as a constant whose only consumer was
  `const { assert!(!TX_FLUSH_BEFORE_DOORBELL) }`: an assertion that a constant
  holds the value it is declared with.
- **`DescRingReport`** — boot reports for the ring-16 descriptor path, which
  ring 0 superseded. Its lines were last printed on 2026-08-15 and the product
  no longer emits them.
- **`GENET_V5_MAJOR: u8 = 5`** — read by nothing, and wrong besides: the
  encoded revision on a Pi 4B is 6, which Linux remaps to logical v5.

### 3. What was declared

Five items carry `Design-ahead (P3 publication)` with the reason: `RingState`,
`RingLayout`, `RingError`, `InterruptWork`, `ResetState`. They are the ring,
interrupt and reset-generation model the network service needs and the bring-up
driver does not.

### 4. Scope

The gate covers `crates/kernel-core/src/genet.rs` and nothing else today.
Widening it to all of `kernel-core` would refuse most of the crate on the day it
landed — much of that crate is pure and reached only through the facade — and a
gate that must be silenced everywhere on arrival teaches people to silence it.
The file list is one line in the script; the trigger to extend it is the next
model that outgrows its consumer.

## Consequences

### Positive

- The host tests `verification.md` cites as ADR-0105/0106 evidence are now
  either over code the silicon runs, or over code declared as not-yet-run.
- One wrong ring advance is gone, before anything adopted it.
- The mutation surface shrinks by four items of dead model before the run that
  #81 schedules.

### Negative / costs

- The gate compares reachability against an annotation. A `Design-ahead` marker
  naming a slice nobody will ever write reads exactly like one naming next
  week's work, and only a reader can tell them apart.
- Reachability is computed by a text walk over top-level items, not by the
  compiler. It errs toward _consumed_ — any identifier `src/` mentions counts —
  so it under-reports rather than crying wolf. A stricter answer would need
  `rustc`'s own resolution, which is not available to a shell gate.
- It is blind to an item that is reachable and whose result the driver throws
  away.

## Alternatives rejected

| Alternative                                                  | Why not                                                                                                             |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------- |
| Make the driver consume the whole model                      | It would have adopted `RingCursor`'s wrong wrap, and the bring-up driver has no ring, interrupt or token to model   |
| Delete everything unreachable                                | Burns host-tested design the P3 publication needs, and turns a declared gap into an undeclared one                  |
| A `docs/genet-model-scope.toml` beside `mutation-scope.toml` | A second file to keep in step with the first, for nine items. The doc comment cannot drift from the item it sits on |
| Leave it to review                                           | This is the residual F-3 recorded and nobody revisited, which is the failure ADR-0049 exists to stop                |

## The gate that catches its own reversal

`make model-consumed`. Seen red on purpose while it was written: removing
`InterruptWork`'s declaration produced

```
model-consumed: `InterruptWork` is pub, nothing in src/ can reach it, and its
  doc comment does not say which slice will consume it.
```

## References

- [ADR-0105](0105-pi4-nic-backend-boundary.md) — the backend, and publication as
  a later step
- [ADR-0096](0096-gates-that-do-not-depend-on-remembering.md)
- [`../verification.md`](../verification.md) — the layers table row
- Issue #83 / [2026-08-17 review](../reviews/2026-08-17-excellence.md) F-3
