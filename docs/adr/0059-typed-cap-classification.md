---
id: 0059
title: Typed capability classification (CapClass)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0017, 0030, 0053, 0055]
---

# ADR-0059: Typed capability classification

## Acceptance status

**Accepted** (2026-08-08), on delegated authority (architectural improvement
plan, move 4; owner delegated acceptance per the approved plan).

## Problem

Band membership is decided by consumers with raw bitmasks — three sites, three
hand-written mask expressions (`idx & 0x4000 != 0 || idx & 0x8000 != 0` in
`sched::transfer_held`, the mirror checks in `taskcap::lookup` and
`irqcap::lookup`). A mask test is not a decode: any future endpoint index with
bit 14 or 15 set would be silently misclassified, every new band means editing
every filter, and the disjointness `const` assert guards the constants but not
the consumers.

## Decision

One decoder owns the classification, in `kernel_core::cap`:

```rust
pub enum CapClass {
    /// No band bit: an IPC endpoint index.
    Endpoint(u16),
    /// Bit 14 (`taskcap::INDEX_BASE`): a task-cap local index.
    Task(u16),
    /// Bit 15 (`irqcap::INDEX_BASE`): an IRQ-cap local index.
    Irq(u16),
    /// Both band bits: decodable by no table, refused everywhere.
    Invalid,
}

impl CapId {
    pub const fn classify(self) -> CapClass { /* bits 15:14, payload 13:0 */ }
}
```

Bits 15:14 of the index are the class; bits 13:0 are the local payload. This is
what the bands already meant — the ADR makes it the decode instead of a
convention three files re-derive. Consumers:

- `sched::transfer_held`: transferable ⇔ `matches!(cap.classify(), CapClass::Endpoint(_))`
  (ADR-0055 policy, now a total match instead of two masks);
- `taskcap::lookup` / `irqcap::lookup`: accept exactly their own class and
  bound the payload against their table size.

The closed enum **is** the band registry: a fourth object kind is a new
variant, and every `match` that does not handle it fails the build. Opening the
set further than four two-bit classes is a successor's problem.

## Non-goals

- Re-encoding `CapId` (index/generation split unchanged; ABI untouched).
- A dynamic band registry — the set is closed by construction.
- Rights inside the class (rights stay `CapRights` on the endpoint table).

## Gates

| Check                           | Evidence                                                                                                    |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Decode is total                 | host tests over all four quadrants + payload bounds                                                         |
| Consumers agree with the decode | existing band-refusal tests (taskcap/irqcap live-entry probes, `xfer-peer: band refused` oracle) stay green |
| Mutation                        | `cap.rs` is in the mutation file list                                                                       |
