# Completeness roadmap

**Single source of truth** for open and done **K** (kernel) and **P** (product)
tracks. Policy: [ADR-0026](adr/0026-kernel-and-product-completeness.md). Model
and layering: [architecture.md](architecture.md). Product narrative and use
cases: [vision.md](vision.md).

Status vocabulary: `open` | `in design` | `done (QEMU)` | `done (HW)`.

Order below is a **working plan** for product coherence — not a ship calendar.
**Design ADR before any boundary move** ([ADR-0001](adr/0001-multi-role-analysis.md)).

Foundation M0–M8 is **closed on Pi 4B**. Historical milestone narrative stays in
[foundation-history.md](foundation-history.md) — note that its **M/P** milestone
letters are the foundation-era vocabulary and are unrelated to the **K/P** tracks
below.

<a id="completeness-roadmap"></a>

---

## Mission (why this roadmap exists)

Quoted from its owner, [`vision.md`](vision.md) — `make doc-claims` compares the
two copies:

<!-- mission:begin -->

> Harbor is an OS where software arrives as **agents**, authority arrives as
> **grants**, and every boundary can be **shown to hold** — and the project
> **finishes** that OS, mechanism by mechanism and service by service.

<!-- mission:end -->

What each word buys, and why this file exists to track it:

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
| **H1 — Composition / appliance OS** | Multi-agent product you can compose and load without rebuild-only demos; early device/supervisor story | **Paid (QEMU first slices):** K1–K3 (+ transfer), K9, K10 (+ cascade), K6, P1, P2, P5 (+ EL0 resolve), P6. **Still on the critical path:** P3/P4 as needed, K5. Residuals: K2 timeout, K9 IRQ-agent, P2 media/EL0 |
| **H2 — Boundary OS** | Full boundary OS: fair execution, denser agents, production isolation, multi-core, remaining platform paths | K4, K5 (remainder), K7, K8, remainder of P2–P5, hardening of first slices to **done (HW)** where claimed |

**H1 product bar (what “composition OS” means here):**

1. **Compose** — pack/inject/inspect agents + grants (P6; first slice done QEMU).
2. **Run several agents** from a product store, not a single beacon (P1; first slice done QEMU).
3. **Load without rebuild-only** — external store path (K6; done QEMU); on-target put/get first slice (P2; done QEMU); media that survives reboot residual.
4. **Wait and drive devices as agents** — IRQ wait (K1 QEMU); second map agent RNG200 (K9 QEMU); IRQ-cap device agent residual.
5. **Supervise** — cancel (HW) + K2 auto-reap + K10 reap/restart + creator-exit cascade (QEMU); K2 timeout residual.
6. **Move authority** — channel revoke + EL1 transfer done (QEMU); EL0 transfer residual.
7. **Find services** — EL1 registry + EL0 `SYS_RESOLVE` (P5 QEMU).

Use cases that pull H1: modular robot/industrial stacks, least-privilege edge
gateways, sealed composition firmware, on-device third-party sandbox
([vision § H1](vision.md#h1--composition--appliance-os)).

---

## K — microkernel mechanisms

| ID | Track | Status | Done when (sketch) | Needs first |
| --- | --- | --- | --- | --- |
| K1 | Wait-on-IRQ (first-class) | **done (QEMU)** ([ADR-0028](adr/0028-wait-on-irq.md) EL1 + [ADR-0030](adr/0030-el0-irq-capability.md) EL0) | EL1 `wait_for_irq`; EL0 `SYS_WAIT_IRQ` via IRQ notification cap; oracle `irq-wait: woke` + `el0-irq: woke` | ADR-0008 → 0028 → 0030 |
| K2 | Park reclaim (timeout and/or auto-reap on last send drop) | **done (QEMU)** first slice ([ADR-0031](adr/0031-k2-last-send-hold-auto-reap.md) last-SEND-hold auto-reap); timeout still open | Ephemeral channels: last SEND hold drop cancels waiter; console default stays stable | Successor to ADR-0025 → 0031; timeout later |
| K3 | Cap transfer / revoke / endpoint release | **done (QEMU)** ([ADR-0032](adr/0032-k3-channel-revoke.md) revoke + [ADR-0037](adr/0037-k3-cap-transfer.md) EL1 transfer); EL0 transfer later | Channel can die; transfer moves CapId between tasks; stale CapId refused | ADR-0017 → 0032 → 0037 |
| K4 | Preemption or CPU budget | **open** | Hostile busy-loop is not permanent DoS residual | Successor to ADR-0006; name agent-pair impact (0023) |
| K5 | Agent density (shrink/collapse driver half) | **open** | Many small agents without 16 KiB kernel stack each by default | Successor to ADR-0023 |
| K6 | External agent load + byte manifest | **done (QEMU)** ([ADR-0027](adr/0027-h1-external-agent-store.md) format, [ADR-0029](adr/0029-agent-store-in-image.md) placement) | Image store inject; product prefers store, oracle empty → builtin | ADR-0021 → 0027 → 0029 |
| K7 | ASID (+ TTBR1 if required) | **open** | Production isolation without cloned-kernel-only story as the end state | Design ADR |
| K8 | SMP | **open** | Multi-core runqueue/IRQ model on silicon | Design ADR |
| K9 | Driver-as-agent beyond PL011 (+ IRQ caps) | **done (QEMU)** first slice ([ADR-0034](adr/0034-k9-rng-driver-agent.md) RNG200 page); IRQ-cap device agent later | Second named Device page map + kill; QEMU may fault load | ADR-0013 → 0034; IRQ agent later |
| K10 | Supervisor lifecycle (restart, creator exit) | **done (QEMU)** ([ADR-0033](adr/0033-k10-supervisor-reap.md) reap + [ADR-0038](adr/0038-k10-creator-exit-cascade.md) cascade); force-kill Running later | `supervisor_reap_blocked`; exit cascades cancel of blocked children | 0018/0025 → 0033 → 0038 |

---

## P — product operating system

Product tracks deliver **services and platform paths** as agents/compositions,
not as a growing special-case syscall surface ([vision](vision.md) shape).

| ID | Track | Status | Done when (sketch) | Typical deps | Horizon |
| --- | --- | --- | --- | --- | --- |
| P1 | Multi-agent product image beyond beacon | **done (QEMU)** first slice (beacon + chirp in store) | Product store n≥2; both run via console endpoint | ADR-0027/0029 | H1 |
| P2 | Storage path (block + load/persist) | **done (QEMU)** first slice ([ADR-0036](adr/0036-p2-keyed-blob-store.md) keyed blobs); SD/media + EL0 residual | On-target put/get without host inject of that payload | ADR-0036 (after K6) | H1 (appliance) → H2 depth |
| P3 | Network agent + caps | **open** | Network I/O only via granted caps; no ambient net | K1/K9 helpful | H1 edge gateway → H2 |
| P4 | Display/input product path | **open** | Product-grade path (may graduate `debug-display` discipline) | Device agents; optional after K9 | H1 lab UI → H2 |
| P5 | Naming / discovery / system services | **done (QEMU)** ([ADR-0035](adr/0035-p5-name-registry.md) EL1 + [ADR-0039](adr/0039-p5-el0-resolve.md) `SYS_RESOLVE`) | Bind/resolve; EL0 installs into empty slot | ADR-0035 → 0039 | H1 composition → H2 |
| P6 | Compose/audit tooling | **done (QEMU)** first slice (pack / inject / inspect) | Host tools for store composition and audit | P1 | H1 |

---

## H1 working order (product-critical path)

Priority is **composition OS usefulness**, not ID order. Each step still needs
its design ADR before boundary code.

| Step | Track(s) | Why now (mission fit) |
| --- | --- | --- |
| 1 | **P3** / **P4** as needed | Edge net / product display when a concrete composition needs them |
| 2 | **K5** density | Scale agent count when MAX_TASKS/stacks press |
| — | **P2 media / EL0 storage** (later) | SD/eMMC + storage caps |
| — | **K9 IRQ-cap device agent** (later) | Second agent that waits on device IRQ via cap |
| — | **K2 timeout** / EL0 transfer / resolve grant | Later slices |

**H2 (after or interleaved when design is ready):** **K4** preemption/budget,
**K7** ASID/TTBR1, **K8** SMP — production fairness and isolation depth, not
the first composition demo.

First slices already paid for H1 entry: **K1**, **K2**, **K3**, **K9** (RNG map), **K10**, **K6**, **P1**, **P2** (blobs), **P5** (names), **P6**.

```text
Mission: agents · grants · evidence · finish the OS
                │
    H0 foundation ████████ done (HW)
                │
    H1 composition ████████ K1–K3 K9 K10 K6 P1 P2 P5 P6 done (QEMU)
                │          next: (P3|P4) · K5
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
