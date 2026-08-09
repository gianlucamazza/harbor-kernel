---
id: 0065
title: Platform self-check — CPU identity decoded, printed, and asserted at boot
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0002, 0003, 0011, 0050]
---

# ADR-0065: Platform self-check (CPU identity as a boot assertion)

## Acceptance status

**Accepted** (2026-08-09), on delegated authority: the owner delegated
acceptance and implementation of this ADR ("pianifica e completa tutto tu",
session 2026-08-09) after reviewing the exploration that produced it.

## Context

The kernel is full of knowledge about the core it runs on, and none of it is
observed at runtime. "Cortex-A72" appears only in comments that justify load-
bearing decisions:

- exclusives make no progress on Device memory pre-MMU (`mm/early.rs`,
  architecture rule 7's hard exception);
- I-cache and D-cache are not coherent, so EL0 text needs an explicit
  clean+invalidate (`cache.rs`, `mm/aspace.rs`);
- `SCTLR_EL1` RES1 bits are checked at the ARMv8.0-A level (`selftest.rs`);
- the TLB fills speculatively, which shaped the ADR-0050 amendment
  (`mmu.rs`).

If the image ever boots on a different core — a Pi SKU change, a QEMU
`-cpu` drift, a future board — none of these assumptions announces its own
violation. The failure shows up downstream as stale text, a hang, or a
corrupted exclusive loop, with nothing on the serial line naming the cause.

Meanwhile [ADR-0011](0011-dtb-mapped-board-constants-risk-accept.md) settled
the _board_ half of platform truth: compiled BSP constants rule, DTB mapped
RO for diagnostics only. The _core_ half was never settled — it is not even
compiled-in; it is implicit. This ADR names it.

The structural precedent already exists in tree: `kernel_core::reset`
decodes `PM_RSTS` as a pure function, bootstrap prints one `reset:` line,
and `assert_boot_oracle` asserts its shape in both runners — with `None`
a distinct outcome so a register that latched nothing cannot pass as a
clean power cycle. The CPU identity gets the same treatment.

One assumption is already handled correctly and one is safe by
construction, and the decision below deliberately leaves both alone:
`cache.rs` derives the cache-line size from `CTR_EL0` at runtime, and
`asid::ASID_BITS = 8` is the width _programmed_ into `TCR_EL1.AS`, valid on
any ARMv8 core regardless of what the hardware could support.

## Decision

1. **Pure decode in `kernel_core::cpuid`.** Total functions over integers,
   host-tested, no MMIO, no asm — the crate's contract. Decodes:
   - `MIDR_EL1` → implementer / part number / variant / revision, with the
     known part (Cortex-A72) recognised into a typed model and everything
     else carried as raw numbers, never dropped;
   - `ID_AA64MMFR0_EL1` → supported ASID width, PA range, 4 KiB granule
     support;
   - `ID_AA64PFR0_EL1` → EL0/EL1 implemented in AArch64, FP/AdvSIMD field.
2. **Thin readers in `arch/aarch64`.** One `mrs` per register, no logic —
   the same split as `reset` (BSP reads `PM_RSTS`, kernel-core decodes it).
3. **One boot line, printed unconditionally** (product and oracle images):
   `cpu: Cortex-A72 r0p3 asid16 pa44 (MIDR=0x…)` — or the raw
   implementer/part numbers when the core is not recognised. Silence means
   the read never happened; an unknown core is a _distinct printed
   outcome_, not an absence.
4. **The compiled expectation is asserted where it is load-bearing.** At
   boot, after the decode:
   - 4 KiB granule supported and EL0/EL1 implemented — **refusal** (panic
     with the register value in the message) if violated: the kernel's
     paging and session model are meaningless without them;
   - hardware ASID width ≥ the programmed `ASID_BITS` — refusal, because
     ADR-0050's isolation arithmetic silently collapses otherwise;
   - part number ≠ the expected Cortex-A72 — **not** a refusal: the line
     says so, and the A72-specific assumptions (I/D non-coherence handling,
     pre-MMU exclusives confinement) are conservative on other cores. The
     divergence is visible, the boot continues.
5. **The boot oracle asserts the line's shape and the expected core.**
   `assert_boot_oracle` gains one assertion on the `cpu:` line — shared
   verbatim by the QEMU gate (`-cpu cortex-a72`) and the hardware
   transcript check, so the two runners demand the same identity and a
   drift in either (a QEMU machine-model change, a different Pi) fails the
   gate that owns it.

### What this is not

- Not a DTB parser and not a second source of board truth — ADR-0011
  stands untouched; this ADR covers the core, which ADR-0011 never claimed.
- Not feature-driven self-patching (Linux `alternatives`): one board, one
  core, no dispatch to specialise. The model is _declarative_: observe,
  compare with the compiled expectation, refuse or report.
- Not a defence against a lying hypervisor: at EL1 every ID register can be
  trapped and emulated from EL2. The self-check asserts consistency of the
  observed platform with the compiled one, not its authenticity.

## Consequences

### Positive

- Every A72 comment that today justifies code becomes an assumption the
  boot either verifies or visibly flags — the implicit platform model turns
  into a checked one.
- A wrong-core boot fails loudly at the first serial line instead of as an
  unexplained hang downstream.
- The decode is pure and host-tested; the arch surface grows by three `mrs`
  readers.

### Costs / residual risk

- One more boot line and oracle assertion to keep honest.
- The recognised-part table is one entry deep; a second board port must
  extend it (the porting checklist gains a step).
- ID registers describe the core, not its errata: the pre-MMU exclusives
  behaviour is still a comment-backed discipline (`pre-mmu-path.sh`), not
  something a register read can prove.

### Gate that would catch a reversal

| Reversal                                              | Signal                                               |
| ----------------------------------------------------- | ---------------------------------------------------- |
| `cpu:` line removed or reworded without successor     | `assert_boot_oracle` fails in both runners           |
| Decode moved into arch/bootstrap (impure, untestable) | `kernel-core` public-api test loses the type; review |
| Refusals downgraded to prints without ADR             | Review: this table names them as refusals            |

## Alternatives considered

| Alternative                                                         | Why not                                                                                                         |
| ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Keep the assumptions as comments                                    | The status quo this ADR exists to end: load-bearing, unobserved                                                 |
| Full feature detection + runtime dispatch (cpufeature/alternatives) | No second core to dispatch to; speculative surface with no consumer                                             |
| Refuse on any unrecognised part                                     | Over-strong: the A72-specific handling is conservative elsewhere; visibility suffices until a second SKU exists |
| Derive board truth from DTB while at it                             | Separate decision, already settled by ADR-0011; would be a silent dual source                                   |

## When to revisit

- A second supported core or board SKU (the recognised table stops being
  one entry; refusal policy for unknown parts may harden).
- EL2 entry or virtualisation work: authenticity of the observed platform
  becomes a real question this ADR explicitly does not answer.

## Related

- [ADR-0011](0011-dtb-mapped-board-constants-risk-accept.md) — the board
  half of platform truth
- [ADR-0050](0050-k7-asid-first-slice.md) — the ASID arithmetic the check
  protects
- [ADR-0002](0002-softfloat-kernel.md) / [ADR-0003](0003-early-mmu.md) —
  decisions whose premises the decode makes observable
- `kernel_core::reset` — the decode → line → oracle-assertion precedent
