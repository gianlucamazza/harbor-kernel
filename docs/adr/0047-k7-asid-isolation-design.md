---
id: 0047
title: K7 design — ASID / production isolation (design only)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0014, 0026]
---

# ADR-0047: ASID isolation design (K7 — design accepted, code deferred)

## Acceptance status

**Accepted as design** (2026-08-08). No isolation code in this ADR. Implementation
requires a follow-on code ADR after TLB/ASID budget is measured on Pi.

## Context

Today user ASes clone kernel maps ([ADR-0014](0014-ttbr-split-m5.md)). Production
isolation wants per-AS **ASID** tags so TLB entries are not globally invalidated
on every switch, and optionally TTBR1 split for kernel.

## Decision (design)

| Item | Intent |
| --- | --- |
| ASID allocation | Fixed pool; assign at AS create; free on destroy |
| Switch | Write TTBR0 + CONTEXTIDR ASID; selective TLBI |
| Residual TTBR1 | Optional later if kernel/user split pays off |
| Evidence | Host model of ASID reuse; QEMU multi-AS smoke; HW TLB stamp |

### First implementation slice (future)

1. Pure ASID allocator (host-tested).  
2. Wire CONTEXTIDR on switch.  
3. Oracle: two ASes, no global TLBI on switch (counter or probe).

### Non-goals of this document

Implementing ASID now; SMP TLB shootdown (K8).

## Gates (when coded)

| Check | Evidence |
| --- | --- |
| Host allocator | unit tests |
| QEMU dual-AS | named oracle |

## Deferral

Code deferred: needs careful exception/TTBR interaction; not required for H1 entry.
