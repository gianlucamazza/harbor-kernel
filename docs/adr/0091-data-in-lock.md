---
id: 0091
title: Data in the lock — Mutex<T> replaces the cell-beside-a-lock pair
status: accepted
date: 2026-08-11
accepted: 2026-08-11
related: [0008, 0019, 0022, 0063, 0077]
---

# ADR-0091: Data in the lock

## Acceptance status

**Accepted** (2026-08-11), on delegated authority (structural improvement plan
approved by the owner on 2026-08-11; owner delegated acceptance for the slices
that plan names).

## Problem

[ADR-0077](0077-smp-shared-state-discipline.md) fixed the _shape_ of a critical
section — mask, spin, mutate, release, restore — and `sync::IrqSpinLock`
implements it correctly. What it did not fix is **which datum** a given lock
protects. The pairing is by convention:

```rust
static NAMES: SyncCell<Table> = SyncCell::new(Table::new());
static NAMES_LOCK: IrqSpinLock = IrqSpinLock::new();

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    NAMES_LOCK.with(|| {
        // SAFETY: exclusivity from NAMES_LOCK (IRQ mask + spin).
        let table = unsafe { &mut *NAMES.get() };
        f(table)
    })
}
```

That block, with the name changed, appears **28 times** across 16 files. Three
things follow, and none of them is hypothetical:

1. **The SAFETY comment stops being read.** `undocumented_unsafe_blocks =
"deny"` proves each one was _written_; nothing proves any was re-derived.
   Twenty-eight near-identical obligations are the shape a reviewer skims.
2. **Nothing binds cell to lock.** Taking `A_LOCK` and dereferencing `B` is a
   compiling program. The correctness of every one of the 28 sites rests on a
   name matching a name.
3. **The pair invites hand-rolled locking where `with` does not fit.**
   `sched::switch_with` must release the lock before `context_switch` while
   IRQs stay masked, so it calls `sched_lock()` / `sched_unlock()` directly —
   three times, across a `match` with early returns (and `collect_exited`
   duplicates its unlock on two paths). That is legitimate work, but today it
   is indistinguishable from the ordinary pattern: it is one more caller of the
   same primitives, not a named exception.

The mechanism is right. The **ownership** is missing.

## Decision

### 1. `Mutex<T>` owns the value it protects

`sync::Mutex<T>` holds the datum inside the lock and hands out `&mut T` scoped
to a closure:

```rust
pub struct Mutex<T> { locked: AtomicBool, value: UnsafeCell<T> }

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self;
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R;
    pub fn lock_masked(&self) -> MaskedGuard<'_, T>;   // §3
}
```

`with` is the ADR-0077 five-step section unchanged — it is still
`cpu::without_irqs(|| { spin; f(&mut value); release })`, so the `irq-scope`
walker keeps opening it on the same lexical token. What changes is that the
`unsafe` deref happens **once, in the type**, instead of 28 times at the call
sites. Two `unsafe` constructs survive tree-wide for this pattern
(`unsafe impl Sync`, and the deref inside `with` / `MaskedGuard`) where there
were ~28.

`Mutex` is not a sleeping mutex. It is an IRQ-masking spinlock that happens to
own its data; there is no blocking primitive in this kernel, and holding one
across a switch is what ADR-0022 forbids. The type doc says so first.

### 2. A lock beside a cell is refused

New shared kernel state is a `Mutex<T>`. Declaring a `SyncCell` next to an
`IrqSpinLock` is not a style preference to be argued per site — `IrqSpinLock`
is removed, so the pair cannot be written.

Where the guarded thing is not a Rust value, the answer is a type that owns it,
not `Mutex<()>` (which reinstates exactly the pair being retired). The durable
region becomes a zero-sized `DurableWindow` whose `as_mut_slice(&mut self)`
carries the one `unsafe`; the borrow checker then enforces "no aliasable
long-lived `&mut`" (excellence review F-26) instead of a comment asking for it.

### 3. `lock_masked` is the single exception, and it is gated

The scheduler switch path cannot use `with`: it must release the spin bit
_before_ `context_switch` while keeping IRQs masked across the stack swap.
`lock_masked` serves exactly that: it does **not** touch `DAIF` (the caller
already masked), it returns a `MaskedGuard<'_, T>` that derefs to the value and
releases on `Drop`. `switch_with` releases with an explicit `drop(guard)` at the
point the comment already names; `collect_exited` lets `Drop` cover both of its
exits, removing a duplicated unlock.

`scripts/check/irq-scope.sh` — already the owner of "who may hand-roll a masked
region" — gains a third clause, `allowed_masked_lock='src/sched/mod.rs'`. A
`lock_masked(` anywhere else fails the gate. The exception is therefore _one
file, named in two places_, rather than a capability every caller has.

### 4. `SyncCell` survives as a closed, enumerated residual

Three statics cannot become a `Mutex`, and the reason is different for each:

| Static          | Where                     | Why not a `Mutex`                                                                                                                                      |
| --------------- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `STATE`         | `src/irq/mod.rs`          | Read from the IRQ dispatch path after `seal()`. A handler must never take a lock (ADR-0008); mutation happens only during bring-up inside a bare mask. |
| `NAME_POOL`     | `src/bootstrap/loader.rs` | `try_store_manifest` mints `&'static str` out of it. A `Mutex` cannot hand out `'static` borrows of its interior.                                      |
| `STORE_ENTRIES` | `src/bootstrap/loader.rs` | Same: `&'static [AgentEntry]` outlives any guard. Boot-window only, argued in place (`sched::STARTED == 0`).                                           |

`SyncCell`'s doc is rewritten from "the default" to "the exception", listing
these three. **A fourth user requires an ADR.** That closed enumeration is the
deliverable — deleting the type is not the goal, bounding it is.

### 5. Lock order, corrected while it moves

ADR-0077's lock-order paragraph moves onto `Mutex` and gains a clause it was
missing: **LINE → TX nests.** `console::suspend_rx` / `resume_rx` call `apply()`
inside `with_line(...)`, and `apply` calls `with_tx` — on the live RX-handover
path. TX is a leaf and the order is one-way, so the nesting is sound; the
document simply did not say it, and a faithful copy would have propagated a
false statement.

## Alternatives rejected

- **`lock() -> Guard` that masks on acquire and restores on `Drop`.** The
  ergonomic shape, and the one that blinds `scripts/check/irq-scope.sh`: its
  walker opens a region at each lexical `without_irqs(` token. A region opened
  by `lock()` and closed by an implicit drop has no opener to key on, so a
  switch inside a masked region would stop being visible to the gate. `with`
  keeps its opener where the walker already looks; `lock_masked` adds no `DAIF`
  region at all.
- **`Mutex<()>` for the durable window.** A lock beside a datum with the datum
  spelled `()`. §2.
- **A `global_table!` macro (or generic `Facade<T>`) for `naming` / `storage` /
  `taskcap`.** After §1 those three modules are a static plus three one-line
  delegations; what remains alike is the _shape of a façade_, not mechanism.
  A macro would save three lines each and hide the static from
  `scripts/check/doc-symbols.sh`, which resolves module paths against real
  declarations.

## Gates

| Check                            | Evidence                                                                                                          |
| -------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| The pair cannot be written again | `IrqSpinLock` no longer exists; `SyncCell` has three named users and a doc that refuses a fourth                  |
| The exception stays one file     | `irq-scope.sh` `allowed_masked_lock` clause, **seen red**: a `lock_masked` in `loader::remember` reported at `src/bootstrap/loader.rs:127`, exit 1 |
| The scope walker is not blinded  | `make irq-scope` counts **20 -> 18** masked regions. It falls rather than rises, and by exactly the two ad-hoc `without_irqs` regions in `bsp/rpi4/display.rs` that the mutex absorbed. No region became invisible; the walker's blind spot (indirect calls) is unchanged |
| Behaviour unmoved end to end     | `make boot-check` green (~100 assertions), `make check` green |

Evidence is **QEMU** for this slice: it is a refactor with no behavioural
change, and no hardware claim is made. The MMU arena plumbing (§2, `mmu.rs`) is
the one part that touches silicon-visible code; a Pi stamp is prudent
follow-up, not a precondition, and is recorded here as optional rather than
claimed.

**On the transcript diff.** The plan for this slice called for a byte-identical
boot transcript against a pre-migration baseline. That oracle does not exist on
this host: two runs of the **same** binary differ in the heap `Box` address,
the reported fragment count, the interleaving of the IRQ-wait and EL0-IRQ
lines, and instantaneous counters (`frames_free`, `idle_signals`). The
differences seen after this migration fall in that same class. The gate
assertions are the oracle; a whole-log diff is not one, and recording that here
is cheaper than the next person re-deriving it.
