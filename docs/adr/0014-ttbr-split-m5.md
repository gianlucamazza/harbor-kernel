---
id: 0014
title: TTBR regime for M5 — single TTBR0 with shared kernel maps (v1)
status: accepted
date: 2026-08-05
accepted: 2026-08-05
---

# ADR-0014: TTBR regime for M5 (closes M5-D1)

## Acceptance status

**Accepted** (2026-08-05). Closes finding **M5-D1** from
[m5-prep](../reviews/2026-08-05-m5-prep.md). Binding for **S3/S4** (EL0 entry
and done-when probes).

**Implementation (2026-08-05):** S3/S4 landed — deep-clone of kernel coverage
into the user root, private user window, `arch::el0::run` / `enter` /
`resume` / `end_session`, sole `mmu::switch_ttbr0` for entry and lower-EL
restore (kernel `TTBR0` preferred when the session ends). Probes **closed on
QEMU and Pi 4B**: [verification.md §M5](../verification.md#m5-el0--address-spaces).

**Later (same ADR regime):** multi-SVC and IRQ resume re-use the published
kernel root and saved user context; they do **not** change the TTBR0-only
clone model. High-half / `TTBR1` remains a successor ADR.

## Context

### Today (M0–M4)

| Register / policy | Value |
| ----------------- | ----- |
| `TTBR0_EL1` | Kernel identity W^X root (`mmu::activate`) |
| `TTBR1_EL1` | Unused; `TCR_EL1.EPD1` set |
| VA window | 39-bit via `T0SZ = 25` (walk starts at L1) |
| Map style | Identity: VA == PA for kernel RAM/device windows |
| Leaf AP | EL1 RW/RO, **no EL0** (`AP` + `UXN` in `kernel_core::paging::leaf`) |

`AddressSpace` (S2) owns a **separate** root frame from the ADR-0012 pool and
does **not** install it in any TTBR yet.

### M5 done-when (architecture)

1. A task runs at **EL0** in its **own** `TTBR0`.
2. An EL0 write to a **kernel** address takes a **permission** fault (ESR recorded).
3. `SVC` returns to EL1 and back.

### Temptations

| Option | Cost | Risk |
| ------ | ---- | ---- |
| **A. TTBR1 = kernel high, TTBR0 = user low** (Linux-class) | Relink or dual-map kernel at high VA; enable TTBR1; rewrite early map | Correct long-term; **too large** as the first EL0 PR |
| **B. TTBR0-only user table without kernel maps** | Switch TTBR0 to user-only root | Exception vectors / stack / handlers fault immediately |
| **C. TTBR0-only; each user AS includes shared kernel maps + private user region** | Copy or share L1 (and below) kernel coverage into each AS | Modest; matches teaching OS / some microkernels pre-high-half |
| **D. “Interim identity forever”** | No private user root | Fails done-when “own TTBR0” |

## Decision

### M5 v1 (required for S3/S4) — option C

1. **Remain TTBR0-only** with `EPD1` set. Do **not** enable TTBR1 in M5 v1.
2. **Kernel map** continues to live in the ADR-0005 arena root pointed to by
   the **kernel** `TTBR0` while only EL1 runs (unchanged bootstrap path).
3. Each **user `AddressSpace`** has its own root (S2). Before EL0 entry that AS
   must contain:
   - **Kernel coverage** sufficient for EL1 exception entry and voluntary
     kernel code while that TTBR0 is live: same VAs as today’s identity kernel
     map (image, stacks, heap, frame pool, devices used by handlers), with
     descriptors that **deny EL0** (existing `leaf` AP/UXN policy).
   - A **private user region** (VA range **disjoint** from kernel image/heap/
     frame pool), backed by ADR-0012 frames, for user stack (and later code).
4. **Context switch to EL0:** `TTBR0_EL1` ← user AS root; ASID optional later
   (v1 may use ASID 0 + full TLB invalidate on switch).
5. **Return to pure EL1 kernel thread / idle:** `TTBR0_EL1` ← kernel root again
   (or leave user TTBR0 if kernel mappings are complete — v1 **must** document
   one policy; preferred: **restore kernel TTBR0** when scheduling idle/EL1-only
   tasks so kernel map mutations stay on one root).
6. **EL0 fault on kernel VA:** satisfied by AP (no EL0 write), not by absence
   of the mapping. ESR path is the existing data-abort machinery.

### Explicit non-goals for M5 v1

- High-half kernel / TTBR1 enablement.
- ASIDs / ASIDs with no TLB shootdown (SMP).
- Multiple concurrent user AS beyond what the scheduler needs for one demo task
  (M5-D5: one live user AS is enough for done-when).

### Successor (post-M5, separate ADR)

**TTBR1 kernel high half** is governed by
[ADR-0084](0084-k7-residual-policy.md) (**K7-T**): deferred until a named
trigger fires (density, isolation depth, host-class layout, or cost evidence).
Sketch needs (still valid as intuition):

- stronger isolation (kernel tables never in user walk),
- or denser user VA / fewer clone frames,
- or alignment with Linux-style / H3 layout.

Until then, option C is not a “hack”: it is the **deliberate product regime**,
with a named upgrade path and explicit non-goals (no half-enabled TTBR1).

## How kernel maps enter a user AS (implementation shape for S3)

Not fully specified here, but constrained:

| Approach | Allowed? |
| -------- | -------- |
| Share higher-level table pages (refcounted) between kernel root and user roots for identity kernel range | Yes, if free/teardown is correct |
| Deep-copy kernel leaf coverage into user AS frames at create | Yes; simpler, more frames |
| Map kernel range as EL0-accessible “for convenience” | **No** |
| User VA overlapping kernel heap/image/frame pool | **No** |

Preferred v1: **deep-copy or walk-and-clone** of kernel identity coverage into
the user root at `AddressSpace` “prepare for EL0” time, counting frames against
the ADR-0012 pool. Share-with-refcount is a later optimisation.

## User VA window (v1 suggestion)

Pick a fixed **low** user window that does not collide with identity kernel use
below ~128 MiB (heap 64 MiB + frame pool 2 MiB + image):

| Region | Suggested v1 |
| ------ | ------------- |
| User stack (grows down) | e.g. `0x0000_0000_4000_0000`–… (1 GiB slot) or a smaller reserved window documented in `memmap` |
| Kernel identity | remains `0 .. IDENTITY_RAM_END` (+ device windows) |

Exact constants live in BSP/`mm` in the S3 PR; this ADR only requires
**disjointness** and **named** windows.

## Consequences

### Positive

- S3 can proceed without a kernel high-half rewrite.
- Done-when (2) is achievable with existing leaf permission encoding.
- Exception handlers keep working if user TTBR0 includes kernel maps (or TTBR0
  is switched back to kernel before deep C — v1 prefers kernel maps in user AS
  so vectors need not switch TTBR in the first instructions).

### Costs / residual risk

- Every user AS pays frame cost for kernel coverage if cloned.
- A bug that marks a kernel page EL0-writable breaks isolation — gate with
  permission probe (done-when) and review of map helpers.
- Not the final Linux-like layout; must not be sold as “complete virtual memory”.

### Gate that would catch a reversal

| Reversal | Signal |
| -------- | ------ |
| Enable TTBR1 without high-half kernel map | Walk into garbage; silent hang or early fault storm |
| User TTBR0 without kernel maps, no TTBR switch in vector | Fault on exception entry |
| EL0-writable kernel pages | Done-when permission probe fails |
| Claim “TTBR1 isolation” while still on option C | doc-claims / review |

## Alternatives considered

| Alternative | Why not for M5 v1 |
| ----------- | ----------------- |
| A — TTBR1 high kernel first | Blocks EL0 for a full relink/map project |
| B — User-only TTBR0 | Breaks EL1 while user TTBR is live |
| D — No private user root | Fails architecture done-when |

## Implementation order (after this ADR)

| Slice | Content |
| ----- | ------- |
| S3a | `AddressSpace::prepare_kernel_coverage()` (clone or share) + named user VA window |
| S3b | EL0 trampoline: set TTBR0 → user root, `ERET` to EL0; return path restores kernel TTBR0 |
| S4 | Probes: EL0 store to kernel VA → ESR; `SVC` round-trip |

## Related

- [ADR-0003](0003-early-mmu.md) — early map then TTBR0 switch
- [ADR-0005](0005-static-page-table-arena.md) — kernel table arena
- [ADR-0012](0012-frame-allocator-for-address-spaces.md) — user frames
- [mmu.md](../mmu.md) — current TTBR0-only regime
- [architecture.md](../architecture.md) — M5 done-when
- [m5-prep](../reviews/2026-08-05-m5-prep.md) — M5-D1
