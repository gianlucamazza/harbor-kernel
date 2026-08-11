---
id: 0097
title: The loader's plan is data, and the loader executes it
status: accepted
date: 2026-08-11
accepted: 2026-08-11
related: [0021, 0029, 0049, 0058, 0063, 0088, 0092]
---

# ADR-0097: The loader's plan is data

## Acceptance status

**Accepted** (2026-08-11), on delegated authority (the owner asked for the
remaining gaps to be closed; owner delegated acceptance for the work that
answer named).

## Problem

`bootstrap::loader::load_all` is the last of the four **R1 extractions**
[ADR-0049](0049-deferred-residuals.md) has been carrying — the other three
landed with ADR-0063, ADR-0060 and ADR-0092.

What lives in the kernel binary today, testable by nothing:

- **which manifest is in force** — an image store if one parses, the built-in
  table otherwise, with a printed line per case;
- **the refusal of an empty table**, which is a different outcome from a table
  whose entries all refuse;
- **per entry, the order `validate` → `bind` → act**, and the four classes of
  message that come out of it. The order is the thing: `validate` refuses a
  geometry before `bind` is asked whether the loader may grant it, so an entry
  that is both malformed _and_ over-reaching is reported as malformed.

`validate` and `bind` are already pure and host-tested
(`kernel_core::manifest`). The **composition** of them is not, and composition
is where a loader gets authority wrong: this is the code that decides what an
agent is allowed to be given, one call before it is given.

## Decision

`kernel_core::loaderplan` turns the whole of it into **data**:

```rust
pub fn plan(
    table: &[AgentEntry],
    held: &[CapId],
    frame_size: usize,
    out: &mut [EntryPlan],
) -> Result<usize, PlanError>;
```

Each entry becomes an `EntryPlan::Spawn { index, home_cpu, slots }` or an
`EntryPlan::Refuse(reason)` carrying the refusal exactly as the ABI reports it.
An empty table is `Err(PlanError::Empty)` — a distinct answer, not zero plans.
`Source::{Store, Builtin}` says which manifest the plan was made from.

The kernel keeps what a pure function cannot do: parsing the image store
(`unsafe`, linker sections, `'static` pools), spawning, remembering the
task→entry mapping, and printing. `load_all` becomes: build the plan, walk it,
execute or report.

**Nothing about the refusals or their order changes.** The oracle lines stay
byte-identical, which is what the boot gates assert; the point is that the
order is now asserted by host tests and reachable by mutants, rather than by
reading the loop.

## Gates

| Check                          | Evidence                                                                                                                                                                                                                                                                                                                         |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Every plan outcome host-tested | `kernel_core::loaderplan` tests: store vs builtin, empty table, invalid geometry, a slot naming a capability the loader does not hold (with the three fields the ABI reports), `home_cpu` carried through from the entry (ADR-0088), and `validate` refusing before `bind` for an entry that is both malformed and over-reaching |
| The decision is mutated        | `loaderplan` in `run-mutants.sh` FILES the commit it is born (ADR-0058 §2), and `make mutation-freshness` (ADR-0096) now fails the moment that surface moves without a run                                                                                                                                                       |
| Behaviour unmoved              | `loader:` oracle lines unchanged in `boot-check` and `product-boot-check`                                                                                                                                                                                                                                                        |

Closes the last row of ADR-0049's **kernel-core extractions**. What remains
outside the host-test and mutation nets in `src/` after this is mechanism —
MMIO, assembly, lock discipline — not decisions.
