---
id: 0095
title: Boot phases have names, and the console handover is the seam
status: accepted
date: 2026-08-11
accepted: 2026-08-11
related: [0003, 0005, 0022, 0026, 0093]
---

# ADR-0095: Boot phases have names

## Acceptance status

**Accepted** (2026-08-11), on delegated authority (structural improvement plan
approved by the owner on 2026-08-11; owner delegated acceptance for the slices
that plan names).

## Problem

`bootstrap::run` was 1006 lines. Moving the oracle block to `demos::run_all`
took it to 489, which is the point at which the remaining problem is visible
rather than buried: **none of the ~17 phases has a name.** Console claim, DTB
survey, heap and frame bounds, map activation, reset cause, CPU identity, core-1
unpark, discovery, table reserve, pool init, RNG, IRQ bind, dispatch seal, CPU-1
bring-up, RX arm — each is a comment and a run of statements, and the only way
to refer to one is by line number.

Every phase's prose is already written, and good; it sits above an anonymous
block instead of on a function that owns it.

## Decision

### 1. `console::install_tx(uart)` is the seam

One fact organises the whole function: `install_tx` **moves** the handle. Above
that line phases print with `println!(uart, …)` and take `&mut Pl011`; below it
they print with `kprintln!` and take nothing.

That is the rule for phase signatures, and both endpoints —
`console::acquire()` and `install_tx` — **stay inline in `run()`**. Hiding
either end would make the second half of the function unreadable: the reader
would see the signatures change with nothing to explain why.

### 2. One `MemPlan`, not a boot context

`establish_kernel_map` returns `MemPlan { heap_end, frame_base, frame_end,
mmu_at }`, written whole, read by `init_memory_pools` and `report_boot`.
Everything else travels as an explicit parameter or return value: `core1: bool`,
`interrupts_bound: bool`, `discover_at: u64`, `console_cap: Option<CapId>`.

**A `BootCtx` with mutable fields is refused.** The module doc promises that
every line depends on the ones above it, and today that ordering is enforced by
`let` bindings — a phase cannot read what an earlier phase never produced,
because it would not compile. A mutable context converts that compile error
into a zero at boot time, which is the class of failure this kernel spends
gates on catching.

### 3. `establish_kernel_map` is one function, not three

Bounds, `kernel_regions`, and `mmu::activate` look like three phases and cannot
be split: `regions` borrows the local `region_buffer`, so whoever calls
`activate` must own the buffer. It is the largest single cut and it stays whole.

### 4. What does not get extracted

`exception::init()`, `console_loop::heap_check`, `console_loop::run()`, the two
`#[cfg(bringup)]` blocks and `refuse_to_boot` — one to four lines each that
already delegate. Wrapping them adds a name and removes ordering information.

### 5. The pre-MMU constraint does not exist here — a retraction

It has been assumed more than once that `bootstrap::run` is constrained by the
pre-MMU gate. It is not: `scripts/check/pre-mmu-path.sh` audits exactly two
symbols, `_start` and `early_mmu_enable`, and `boot.s` enables the early
identity map before any Rust runs. `run()` executes with the coarse map already
active — its own comment says so.

Recorded here because a misconception that freezes a function is worth a
citable retraction; the real constraints are the `Pl011` handle's ownership and
the `region_buffer` borrow, both named above.

## Gates

| Check                                            | Evidence                                                                                                                                                                                                                                                         |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| No behavioural change                            | `make boot-check` (~100 assertions), `product-boot-check`, `panic-check` green after **each** extracted phase, one commit per phase                                                                                                                              |
| Rule 9 not weakened                              | the oracle block moved _into_ `demos.rs`, the file `product-builds` derives its forbidden-symbol list from                                                                                                                                                       |
| The `unsafe` argument was re-derived, not copied | `enable_interrupts` holds the one `unsafe` this work touches — `console::enable_rx_irq`, whose SAFETY cited "the handle **this function** acquired". After extraction that sentence is false as written and is rewritten to the borrow the caller actually lends |

Evidence is **QEMU**: a refactor with no behavioural change, no hardware claim.
