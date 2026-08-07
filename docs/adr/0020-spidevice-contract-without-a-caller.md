---
id: 0020
title: SpiDevice — an adopted contract with no caller, and a sentence in ADR-0010 that stopped describing anything
status: proposed
date: 2026-08-07
related: [0009, 0010]
---

# ADR-0020: `SpiDevice` keeps its place, and ADR-0010's descriptive sentence is retracted

## Acceptance status

**Proposed** (2026-08-07). Successor to a _description_ inside
[ADR-0010](0010-spi-transaction-and-dbi-panel.md), not to any of its decisions.
Accepted ADRs are immutable, so a sentence that has stopped being true is
corrected here rather than edited there.

## Context

`drivers::spi::SpiDevice` is implemented by `ExclusiveDevice` and **called by
nothing**, in every configuration. The compiler says so under
`--features debug-display`, which is the only build where the module exists at
all: `trait SpiDevice is never used`.

Two claims have to be kept apart, and I did not keep them apart when I first
reported this.

**The requirement holds.** [ADR-0010](0010-spi-transaction-and-dbi-panel.md) §1
says:

> ILI9486 **must not** bit-bang CS. It either uses `SpiDevice` for short ops or
> `with_bus` when it holds an `ExclusiveDevice`.

That is an either/or, and the driver takes the second branch. Nothing about CS
is violated — `with_bus` asserts CS, runs the body against the raw bus, and
deasserts it on every path including error.

**A sentence beside it does not hold.** The line before reads:

> Short register ops keep using `SpiDevice::write` / `transfer` (one CS each).

They do not. `ili9486.rs` reaches `ExclusiveDevice::with_bus` for short register
ops as well as for RAMWR streams. The sentence describes a code shape that never
existed, and it was written in the same ADR whose whole subject is that a DBI
panel needs **one CS held across many writes** — which is the reason the driver
went the other way.

So the trait sits in the tree with an implementation, no caller, and an accepted
ADR describing a use of it that does not occur.

## Decision

**1. `SpiDevice` stays.** [ADR-0009](0009-optional-spi-tft-debug-console.md)
adopts the `SpiBus` / `SpiDevice` split of embedded-hal 1.0 as the _structure_ of
this driver layer, and that decision is not in question here. The trait is what a
second slave on the same bus — the XPT2046 touch controller named in ADR-0009 —
would be written against. Removing it would mean re-deciding ADR-0009 to save a
type that costs nothing at runtime.

**2. It carries its own state in its doc-comment**, which it already does: that
it has an implementation and no caller, that the ILI9486 path goes through
`with_bus` because ADR-0010's subject is exactly the multi-write CS session, and
that the `allow(dead_code)` beside it is measured rather than habitual — with no
attribute, `--features debug-display` reports the trait unused; with
`#[expect(dead_code)]` in the same position it reports the expectation
unfulfilled. Both cannot be true, and an `expect` a build rejects is worse than
an `allow` that states its reason.

**3. ADR-0010's descriptive sentence is retracted**, and this ADR is what a
reader following `related` finds. The requirement it sits beside is untouched.

## Consequences

### Positive

- The gap between an accepted ADR and the code is closed by saying which half
  was wrong, instead of leaving a reader to discover that a requirement and a
  description disagreed.
- The one remaining `allow` in the tree is the only one, and now has an ADR
  behind it rather than a comment.

### Negative / debt

- **A trait with no caller is still a trait with no caller.** This ADR argues it
  is a contract rather than dead code, and that argument is only as good as the
  second slave arriving. If XPT2046 never lands, a future audit should read this
  decision as expired rather than as settled.
- **Nothing checks that the ILI9486 path stays inside a CS session.** The rule
  "must not bit-bang CS" is enforced by the shape of `with_bus`, which takes a
  closure and cannot be left open — but a driver that grew its own CS handling
  would compile. No gate covers it.

### Gates that catch reversal

| Reversal                                                   | Gate                                                                                                            |
| ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `SpiDevice` gains a caller and this ADR goes stale         | `cargo build --features debug-display` — the `allow(dead_code)` becomes unnecessary, though nothing fails on it |
| The trait is deleted while ADR-0009 still adopts the split | `make xrefs` keeps ADR-0009 reachable; the deletion would leave its structure claim describing nothing          |
| CS driven outside a session                                | **Nothing.** Named here rather than implied                                                                     |

## Alternatives rejected

| Alternative                                                            | Why not                                                                                                                                                                               |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Delete `SpiDevice`                                                     | It is dead code by the letter and a contract by ADR-0009. Deleting it re-decides an accepted ADR to remove a type with no runtime cost, and the touch controller would reintroduce it |
| Make ILI9486 use it for short ops, so ADR-0010's sentence becomes true | Rewrites working, silicon-proven code to match a description. The driver's shape is the one ADR-0010's _decision_ prefers; only its prose lagged                                      |
| Leave the divergence unrecorded                                        | It was found by a lint conversion, not by reading. The next reader deserves better than the same discovery                                                                            |
| Edit ADR-0010                                                          | Accepted ADRs are immutable. The lifecycle exists for exactly this                                                                                                                    |
