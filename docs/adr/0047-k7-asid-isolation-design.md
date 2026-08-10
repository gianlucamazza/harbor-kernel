---
id: 0047
title: K7 design — ASID / production isolation (design only)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
amended: 2026-08-08
related: [0014, 0026]
---

# ADR-0047: ASID isolation design (K7 — design accepted)

## Acceptance status

**Accepted as design** (2026-08-08). First code slice landed in
[ADR-0050](0050-k7-asid-first-slice.md). Residuals after first code are split
and governed by [ADR-0084](0084-k7-residual-policy.md) (switch-cost evidence,
TTBR1 triggers, ASID rollover) — not a single undifferentiated “TTBR1 /
switch-cost” bucket.

## Context

Today user ASes clone kernel maps ([ADR-0014](0014-ttbr-split-m5.md)). Production
isolation wants per-AS **ASID** tags so TLB entries are not globally invalidated
on every switch, and optionally TTBR1 split for kernel.

## Decision (design)

| Item | Intent |
| --- | --- |
| ASID allocation | Fixed pool; assign at AS create; free on destroy |
| Switch | Write TTBR0 + CONTEXTIDR ASID; selective TLBI |
| Residual TTBR1 | Deferred with triggers — [ADR-0084](0084-k7-residual-policy.md) |
| Evidence | Host model of ASID reuse; QEMU multi-AS smoke; dual-AS HW stamp; optional later switch-cost lab (0084 K7-M) |

### First implementation slice

Landed in [ADR-0050](0050-k7-asid-first-slice.md):

1. Pure ASID allocator (host-tested).  
2. Wire CONTEXTIDR + ASID in TTBR0 on switch; nG user leaves.  
3. Oracle: two ASes enter EL0 with distinct ASIDs (`asid: dual … ok`).

### Non-goals of this document

SMP TLB shootdown (K8); full production isolation measurement.

## Gates (when coded)

| Check | Evidence |
| --- | --- |
| Host allocator | unit tests |
| QEMU dual-AS | named oracle |

## Deferral

Code deferred: needs careful exception/TTBR interaction; not required for H1 entry.

**Amended 2026-08-08** (ADR-0058 convention): slice items and the oracle
definition reconciled with the landed first slice —
[ADR-0050](0050-k7-asid-first-slice.md), commit `560ba4e`.
