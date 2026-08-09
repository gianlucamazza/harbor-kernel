---
id: 0050
title: K7 first slice — ASID pool, CONTEXTIDR, nG user leaves
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0014, 0026, 0047]
amended: 2026-08-09
---

# ADR-0050: ASID first code slice (K7 entry)

## Acceptance status

**Accepted** (2026-08-08). Implements the first code slice of
[ADR-0047](0047-k7-asid-isolation-design.md): pure ASID allocator, ASID in
`TTBR0` + `CONTEXTIDR` on switch, non-global user leaves, dual-AS oracle.

## Decision

### 1. Pure `kernel_core::asid`

- 8-bit ASID field (`TCR_EL1.AS=0` default).
- ASID 0 reserved for the kernel root.
- `AsidPool::{alloc, free}`, `pack_ttbr0` / `unpack_*` — host-tested.

### 2. Page tables

- User-accessible leaves set **nG** (`DESC_NG`) so TLB entries are ASID-tagged.
- Kernel identity leaves remain Global so restoring ASID 0 does not require a
  full wipe.

### 3. Kernel wiring

- `mm::asid` owns the pool (IRQ-masked, like frames).
- `AddressSpace::create` allocates an ASID; `destroy` runs `tlbi aside1is` then
  frees the tag.
- `switch_ttbr0(ttbr)` writes full TTBR0 (root + ASID) and `CONTEXTIDR_EL1`;
  **no** `tlbi vmalle1is` on switch (ADR-0047 selective path). Mapping changes
  keep their existing TLBI plans.

### 4. Oracle

Bootstrap dual-AS path prints `asid: dual a=… b=… ok` after both enter EL0
with distinct ASIDs.

### Amendment (2026-08-09 — reconciliation per ADR-0058)

The first Pi 4B run of this slice (transcript
`.serial-log/20260809-093312.log`) showed a mechanism gap: removing the
per-switch `tlbi vmalle1is` also removed the only thing that retired the
**early map's** Global 1 GiB L1 blocks from the TLB. On Cortex-A72 (which
fills the TLB speculatively, unlike QEMU) a stale early block served the
first EL0 fetches at the user window — instruction abort, permission fault
level 1 — and until evicted also shadowed the fine map's W^X for EL1.
`mmu::activate` now runs one `tlbi vmalle1is` immediately after switching to
the fine root (`retire_early_map`): the early map's lifetime ends there, and
that boundary owns dropping its residue. §3 is unchanged — the per-switch
path stays TLBI-free. Reconciled by the commit introducing
`retire_early_map`; QEMU cannot discriminate this property, so the gate is
the hardware transcript check.

### 5. Residuals

- TTBR1 high-half split (optional later).
- 16-bit ASIDs / rollover policy when pool exhausts under heavy churn.
- HW TLB stamp (measure switch cost on Pi).
- SMP shootdown remains **K8**.

## Gates

| Check | Evidence |
| --- | --- |
| Host allocator + nG leaves | unit tests |
| QEMU dual-AS | `asid: dual a=… b=… ok` |
