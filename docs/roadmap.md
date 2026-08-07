# Completeness roadmap

**Single source of truth** for open and done **K** (kernel) and **P** (product)
tracks. Policy: [ADR-0026](adr/0026-kernel-and-product-completeness.md). Model
and layering: [architecture.md](architecture.md). Product narrative and use
cases: [vision.md](vision.md).

Status vocabulary: `open` | `in design` | `done (QEMU)` | `done (HW)`.

Order below is a **working plan** for product coherence — not a ship calendar.
**Design ADR before any boundary move** ([ADR-0001](adr/0001-multi-role-analysis.md)).

Foundation M0–M8 is **closed on Pi 4B**. Historical milestone narrative stays in
[architecture § roadmap](architecture.md#roadmap).

<a id="completeness-roadmap"></a>

---

## Mission (why this roadmap exists)

Harbor finishes an OS where:

| Pillar | Meaning |
| --- | --- |
| **Agents** | Software is isolation units (driver task + EL0 program), not ambient processes |
| **Grants** | Authority is slot-indexed and enumerable — nothing ambient |
| **Evidence** | Boundaries are shown (host / QEMU / HW), not only compiled |
| **Completeness** | Kernel mechanisms **and** product services close under this model ([ADR-0026](adr/0026-kernel-and-product-completeness.md)) |

Harbor is **not** Linux/POSIX, not a perpetual lab, and not an LLM framework.
Product services (storage, net, UI, naming) live **as agents and compositions**
above a small TCB — same shape as the console endpoint.

---

## Horizons → product outcomes

Horizons are product stories ([vision](vision.md)); **status lives only in the
K/P tables** below. A horizon is “paid” when its listed tracks have evidence at
the named level — not when prose wishes it.

| Horizon | Product outcome | Tracks that close it |
| --- | --- | --- |
| **H0 — Foundation** | Boundary lab on Pi 4B: tasks, caps, EL0, PL011 agent, blocking recv, console + beacon, cancel | **Done (HW)** — M0–M8 + ADR-0024/0025 |
| **H1 — Composition / appliance OS** | Multi-agent product you can compose and load without rebuild-only demos; early device/supervisor story | **Paid (QEMU first slices):** K1, K2 (auto-reap), K6, P1, P6. **Still on the critical path:** K3, K9, K10, then P2 / P5 (and P3/P4 as the appliance needs them). K5 helps density; K2 timeout residual open |
| **H2 — Boundary OS** | Full boundary OS: fair execution, denser agents, production isolation, multi-core, remaining platform paths | K4, K5 (remainder), K7, K8, remainder of P2–P5, hardening of first slices to **done (HW)** where claimed |

**H1 product bar (what “composition OS” means here):**

1. **Compose** — pack/inject/inspect agents + grants (P6; first slice done QEMU).
2. **Run several agents** from a product store, not a single beacon (P1; first slice done QEMU).
3. **Load without rebuild-only** — external store path (K6; done QEMU); on-target persist/load still P2.
4. **Wait and drive devices as agents** — IRQ wait EL0/EL1 (K1 done QEMU); second driver-agent (K9 open).
5. **Supervise** — cancel exists (HW); reclaim orphans and restart policy (K2, K10 open).
6. **Move authority** — transfer/revoke/release so compositions are not reboot-scoped (K3 open).
7. **Find services** — naming without hard-wired oracle tables (P5; wants K3).

Use cases that pull H1: modular robot/industrial stacks, least-privilege edge
gateways, sealed composition firmware, on-device third-party sandbox
([vision § H1](vision.md#h1--composition--appliance-os)).

---

## K — microkernel mechanisms

| ID | Track | Status | Done when (sketch) | Needs first |
| --- | --- | --- | --- | --- |
| K1 | Wait-on-IRQ (first-class) | **done (QEMU)** ([ADR-0028](adr/0028-wait-on-irq.md) EL1 + [ADR-0030](adr/0030-el0-irq-capability.md) EL0) | EL1 `wait_for_irq`; EL0 `SYS_WAIT_IRQ` via IRQ notification cap; oracle `irq-wait: woke` + `el0-irq: woke` | ADR-0008 → 0028 → 0030 |
| K2 | Park reclaim (timeout and/or auto-reap on last send drop) | **done (QEMU)** first slice ([ADR-0031](adr/0031-k2-last-send-hold-auto-reap.md) last-SEND-hold auto-reap); timeout still open | Ephemeral channels: last SEND hold drop cancels waiter; console default stays stable | Successor to ADR-0025 → 0031; timeout later |
| K3 | Cap transfer / revoke / endpoint release | **open** | Authority can move and die without reboot; stale generation exercised by real release | ADR-0017 successor |
| K4 | Preemption or CPU budget | **open** | Hostile busy-loop is not permanent DoS residual | Successor to ADR-0006; name agent-pair impact (0023) |
| K5 | Agent density (shrink/collapse driver half) | **open** | Many small agents without 16 KiB kernel stack each by default | Successor to ADR-0023 |
| K6 | External agent load + byte manifest | **done (QEMU)** ([ADR-0027](adr/0027-h1-external-agent-store.md) format, [ADR-0029](adr/0029-agent-store-in-image.md) placement) | Image store inject; product prefers store, oracle empty → builtin | ADR-0021 → 0027 → 0029 |
| K7 | ASID (+ TTBR1 if required) | **open** | Production isolation without cloned-kernel-only story as the end state | Design ADR |
| K8 | SMP | **open** | Multi-core runqueue/IRQ model on silicon | Design ADR |
| K9 | Driver-as-agent beyond PL011 (+ IRQ caps) | **open** | Second peripheral on the M6 pattern; IRQ-cap path | K1 useful; ADR-0013 pattern |
| K10 | Supervisor lifecycle (restart, creator exit) | **open** | Product supervisor can restart/reap without ad-hoc demos | Builds on 0018/0025 |

---

## P — product operating system

Product tracks deliver **services and platform paths** as agents/compositions,
not as a growing special-case syscall surface ([vision](vision.md) shape).

| ID | Track | Status | Done when (sketch) | Typical deps | Horizon |
| --- | --- | --- | --- | --- | --- |
| P1 | Multi-agent product image beyond beacon | **done (QEMU)** first slice (beacon + chirp in store) | Product store n≥2; both run via console endpoint | ADR-0027/0029 | H1 |
| P2 | Storage path (block + load/persist) | **open** | Persist or load agent/data without rebuild-only workflow | Often after K6 | H1 (appliance) → H2 depth |
| P3 | Network agent + caps | **open** | Network I/O only via granted caps; no ambient net | K1/K9 helpful | H1 edge gateway → H2 |
| P4 | Display/input product path | **open** | Product-grade path (may graduate `debug-display` discipline) | Device agents; optional after K9 | H1 lab UI → H2 |
| P5 | Naming / discovery / system services | **open** | Endpoints findable without hard-coded oracle wiring | K3 useful | H1 composition → H2 |
| P6 | Compose/audit tooling | **done (QEMU)** first slice (pack / inject / inspect) | Host tools for store composition and audit | P1 | H1 |

---

## H1 working order (product-critical path)

Priority is **composition OS usefulness**, not ID order. Each step still needs
its design ADR before boundary code.

| Step | Track(s) | Why now (mission fit) |
| --- | --- | --- |
| 1 | **K3** cap transfer / revoke / release | Authority must move with the composition, not only at spawn |
| 2 | **K10** supervisor lifecycle | Creator/supervisor restarts and reaps real product agents |
| 3 | **K9** second driver-as-agent | Proves device agents beyond PL011 using IRQ caps (K1) |
| 4 | **P5** naming / discovery | Compositions find endpoints without oracle hard-wiring (wants K3) |
| 5 | **P2** storage path | Appliance load/persist without rebuild-only workflow |
| 6 | **P3** / **P4** as needed | Edge net / product display when a concrete composition needs them |
| 7 | **K5** density | Scale agent count for multi-agent appliances (can interleave earlier if blocked on slots) |
| — | **K2 timeout** (later slice) | Deadline queue; not required for last-hold auto-reap |

**H2 (after or interleaved when design is ready):** **K4** preemption/budget,
**K7** ASID/TTBR1, **K8** SMP — production fairness and isolation depth, not
the first composition demo.

First slices already paid for H1 entry: **K1**, **K6**, **P1**, **P6**.

```text
Mission: agents · grants · evidence · finish the OS
                │
    H0 foundation ████████ done (HW)
                │
    H1 composition ███░░░░░ K1 K2 K6 P1 P6 done (QEMU)
                │          next: K3 → K10 → K9 → P5 → P2 → (P3|P4) · K5
                │
    H2 boundary    ░░░░░░░░ K4 K7 K8 + HW stamps + remaining P depth
```

---

## Standing watches (not completeness tracks)

| Work | Done when | Issue |
| --- | --- | --- |
| **ADR-0020 expiry watch** | XPT2046 lands and `SpiDevice` gets a caller, or the trait goes and ADR-0020 is superseded | [#14](https://github.com/gianlucamazza/harbor-kernel/issues/14) |

---

## Out of model (permanent non-goals)

These are **not** completeness tracks ([ADR-0026](adr/0026-kernel-and-product-completeness.md)):

- Linux / POSIX / glibc compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
- Multi-tenant cloud hypervisor (unless a future ADR owns it)
- Being an AI agent framework (may *host* workers; is not a chat SDK)

---

## How to extend

1. Add a row (or split a slice) in this file first; if it is product-facing,
   say which horizon and which use case it serves.
2. Write/accept a design ADR before boundary code.
3. Land code + gates; flip status only with evidence named in the row.
4. Point GitHub tracking ([#17](https://github.com/gianlucamazza/harbor-kernel/issues/17))
   at this file — do not invent a second status table.
5. Keep [vision.md](vision.md) narrative aligned when a horizon’s “paid / still
   pay” list changes (vision may lag a day; **status** never leaves this file).
