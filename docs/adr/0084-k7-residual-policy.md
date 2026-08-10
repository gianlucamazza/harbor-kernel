---
id: 0084
title: K7 residual policy — switch-cost evidence, TTBR1 triggers, ASID honesty
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0014, 0047, 0050, 0044, 0069]
---

# ADR-0084: K7 residual policy after the ASID first slice

## Acceptance status

**Accepted as design / policy** (2026-08-10). The K7 **mechanism** first
slice is already **done (HW)**
([ADR-0047](0047-k7-asid-isolation-design.md)/[0050](0050-k7-asid-first-slice.md)).
What remained was an undifferentiated residual label (“TTBR1 / switch-cost”)
that mixed a small lab question with a large layout rewrite. This ADR **splits
and governs** those residuals so the architecture stays honest after K8 depth
closed on silicon.

This document does **not** implement TTBR1 and does **not** claim switch-cost
measured. It is the standing policy until a follow-on code ADR or a TTBR1
design ADR fires under the triggers below.

## Context

| Piece | Status |
| --- | --- |
| ASID pool + CONTEXTIDR + nG user leaves | **done (HW)** — dual-AS oracle on Pi |
| Early-map TLB retirement at activate | **done (HW)** — A72-specific gap closed |
| Per-switch path | TTBR0+ASID write; **no** `tlbi vmalle1is` (correctness paid) |
| Product VA regime | [ADR-0014](0014-ttbr-split-m5.md) **option C**: TTBR0-only, kernel maps cloned into each user root, `TCR.EPD1` set |
| “Switch-cost measurement” | Named in ADR-0050 §5; **not** done |
| “TTBR1 high-half” | Named successor since ADR-0014; **not** started |
| ASID width / rollover | 8-bit pool (255 user); no generation rollover policy under churn |

Roadmap and stack surfaces had collapsed the last two into one bucket, which
encourages either over-building TTBR1 without a product trigger or silently
forgetting switch-cost evidence.

## Decision

### 1. Split the residual (mandatory project language)

After this ADR, status prose **must not** say only “K7 residual: TTBR1 /
switch-cost” as a single item. Use:

| Residual ID | Kind | Status vocabulary |
| --- | --- | --- |
| **K7-M** Switch-cost evidence | Lab / verification | `open` until a named protocol produces Pi numbers; may stay `deferred` if product does not need the decision |
| **K7-T** TTBR1 high-half layout | Architecture rewrite | `deferred` until a **trigger** in §3 fires; then design ADR → code |
| **K7-R** ASID width / rollover | Scale policy | `deferred` until pool pressure / `OutOfAsid` is a product problem |

K7 first slice (ASID) remains **done (HW)**. Completing K7-M or K7-T is
**not** required to claim that first slice closed.

### 2. Product architecture remains option C

Until a K7-T design ADR is accepted and coded:

- `TTBR0_EL1` carries user root + ASID; kernel coverage is **in** that root
  (EL0-denied).
- `TTBR1_EL1` stays unused; `EPD1` stays set so high VAs fault cleanly.
- Option C is the **named current architecture**, not a temporary shame
  residual. Isolation evidence is ASID + nG + dual-AS HW stamp, not TTBR1.

Claiming “TTBR1 isolation” while still on option C is a documentation defect.

### 3. TTBR1 (K7-T) — deferred with explicit triggers

Implement TTBR1 high-half **only if at least one** trigger holds:

| Trigger | Meaning |
| --- | --- |
| **Density** | Kernel-map clone frames per agent dominate the frame budget (measured or hard budget miss under target agent count) |
| **Isolation depth** | Product requires kernel table pages to be **absent** from every user walk (beyond AP EL0-denied on cloned leaves) |
| **Host-class / H3** | North-star layout alignment ([ADR-0069](0069-harbor-host-class-north-star.md)) explicitly chooses Linux-class split |
| **Cost evidence** | K7-M numbers show walk/clone cost of option C is the dominant switch or create cost *and* TTBR1 is the chosen fix |

**Not a trigger:** closing a roadmap cell for aesthetics; “other kernels do it.”

When a trigger fires, work is a **new design ADR** (not silent code under
0084): relink/high map, enable TTBR1, exception/stack placement, teardown,
full QEMU + HW re-stamp. Half-enabling TTBR1 without a complete high-half
kernel map is **forbidden** (ADR-0014 table).

### 4. Switch-cost evidence (K7-M) — protocol, not product path

| Rule | Choice |
| --- | --- |
| Purpose | Inform K7-T trigger “cost evidence” and document ASID win vs global TLBI era |
| Host | **Pi 4B** (or equal Cortex-A72 silicon). QEMU is **not** a cost oracle |
| Method sketch | Sample `CNTPCT_EL0` around `switch_ttbr0` and optionally around EL0 enter/resume; report median/min over N≥100 under quiet load |
| Surface | Optional line under `oracle` or `bringup` only, e.g. `k7: switch_ns=… n=…` — exact string in a **code** ADR if implemented |
| Boot gate | **Must not** fail `boot-check` on absolute thresholds (host jitter, thermal) |
| Global TLBI A/B | Do **not** reintroduce per-switch `vmalle1is` in product to “compare”; structural claim already paid (no TLBI on switch) |

Shipping K7-M code is **optional**. Accepting this policy ADR is enough for
project perfection; numbers wait for a lab session when useful.

### 5. ASID rollover (K7-R)

- Keep **8-bit** user pool (`ASID_BITS = 8`, ASID 0 kernel) until product
  hits sustained `OutOfAsid` or intentional >255 concurrent AS.
- Then: separate ADR for 16-bit (`TCR.AS`) and/or generation rollover + TLBI
  policy. Not coupled to TTBR1.

### 6. Orthogonality to K5 and K8

| Track | Problem | Not solved by |
| --- | --- | --- |
| **K5** driver-half | Task/stack shape cost | TTBR1 |
| **K7-T** | Map/layout cost and kernel-in-user-root | Thin stacks alone |
| **K8** agent steal + TLB IPI | Cross-core TLB for migrated AS | TTBR1 (local layout) |

Do not merge these into one “isolation residual” ticket.

### 7. Evidence / status rules

| Claim | Allowed when |
| --- | --- |
| K7 first slice done (HW) | Already true (0050 stamp) — unchanged |
| K7-M done | Named Pi protocol run + verification row (if coded) or explicit “deferred no product need” |
| K7-T done | Design ADR + code + QEMU + HW under option A regime |
| “K7 complete” | **Avoid** until M+T+R are each closed or explicitly deferred with this ADR cited |

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Implement TTBR1 now | No trigger; large risk; option C paid on silicon |
| Mark residual done without policy | Hides unfinished decisions |
| QEMU-only switch timing | Misrepresents A72 TLB/cost |
| Bundle measure code + TTBR1 in one ADR | Wrong coupling of lab and rewrite |

## Consequences

### Positive

- Clear architecture: option C is current; TTBR1 is upgrade-with-trigger  
- Measure cannot accidentally gate product boots  
- Roadmap language can list K7-M / K7-T / K7-R separately  

### Residual after this policy ADR

- Optional **code** for K7-M (measure harness)  
- Future **design+code** for K7-T if a trigger fires  
- Future **design** for K7-R under pool pressure  
- K5 driver-half; K8 agent+TLB steal (unchanged)  

## Related

- Regime: [0014](0014-ttbr-split-m5.md)  
- ASID design/code: [0047](0047-k7-asid-isolation-design.md), [0050](0050-k7-asid-first-slice.md)  
- Density: [0044](0044-k5-agent-density.md)  
- Host-class: [0069](0069-harbor-host-class-north-star.md)  
