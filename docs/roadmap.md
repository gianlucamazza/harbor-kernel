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
| **H1 — Composition / appliance OS** | Multi-agent product you can compose and load without rebuild-only demos; early device/supervisor story | **Paid (HW stamp 2026-08-08):** bar 1–7 + K5 thin + P2 durable + K4 budget + lifecycle residuals (serial `.serial-log/20260808-030219.log`). **Still open / residual:** P3/P4 (deferred), P2 SD/power-cycle, K5 driver-half, peer transfer, resolve grant, IRQ preemption |
| **H2 — Boundary OS** | Full boundary OS: fair execution, denser agents, production isolation, multi-core, remaining platform paths | **K4** IRQ preemption residual (cooperative budget **done (HW)**); **K5** driver-half remainder; **K7/K8** design accepted (code deferred); P3/P4 deferred; SD power-cycle residual |

**H1 product bar (what “composition OS” means here):**

1. **Compose** — pack/inject/inspect agents + grants (P6; first slice done QEMU — host tools).
2. **Run several agents** from a product store, not a single beacon (P1; first slice done QEMU product inject; HW stamp used builtin beacon+mute).
3. **Load without rebuild-only** — external store path (K6; done QEMU inject + HW builtin fallback); on-target put/get (P2; **done (HW)**); SD/power-cycle residual.
4. **Wait and drive devices as agents** — IRQ wait (K1; **done (HW)**); RNG map + IRQ-cap wait (K9; **done (HW)**).
5. **Supervise** — cancel + K2 auto-reap + park timeout + K10 reap/cascade (**done (HW)**).
6. **Move authority** — revoke + EL1 transfer + EL0 return-to-creator (**done (HW)**); peer-by-TaskId residual.
7. **Find services** — EL1 registry + EL0 `SYS_RESOLVE` (**done (HW)**); resolve-grant residual.

Use cases that pull H1: modular robot/industrial stacks, least-privilege edge
gateways, sealed composition firmware, on-device third-party sandbox
([vision § H1](vision.md#h1--composition--appliance-os)).

---

## K — microkernel mechanisms

| ID | Track | Status | Done when (sketch) | Needs first |
| --- | --- | --- | --- | --- |
| K1 | Wait-on-IRQ (first-class) | **done (HW)** ([ADR-0028](adr/0028-wait-on-irq.md) + [ADR-0030](adr/0030-el0-irq-capability.md); Pi stamp 2026-08-08) | EL1 `wait_for_irq`; EL0 `SYS_WAIT_IRQ`; `irq-wait: woke` + `el0-irq: woke` | ADR-0008 → 0028 → 0030 |
| K2 | Park reclaim (timeout and/or auto-reap on last send drop) | **done (HW)** ([ADR-0031](adr/0031-k2-last-send-hold-auto-reap.md) + [ADR-0040](adr/0040-k2-park-timeout.md) + [ADR-0042](adr/0042-el0-recv-timeout.md); Pi stamp 2026-08-08) | Last SEND hold drop; EL1/EL0 tick timeout → Cancelled | 0025 → 0031 → 0040 → 0042 |
| K3 | Cap transfer / revoke / endpoint release | **done (HW)** ([ADR-0032](adr/0032-k3-channel-revoke.md) + [ADR-0037](adr/0037-k3-cap-transfer.md) + [ADR-0041](adr/0041-el0-cap-transfer.md); Pi stamp 2026-08-08); peer TaskId residual | Revoke; EL1 transfer; EL0 self/creator move | 0017 → 0032 → 0037 → 0041 |
| K4 | Preemption or CPU budget | **done (HW)** first slice ([ADR-0046](adr/0046-k4-cooperative-cpu-budget.md); Pi stamp 2026-08-08); IRQ preemption residual | Tick quantum + voluntary yield; no IRQ switch | 0006 → 0046 |
| K5 | Agent density (shrink/collapse driver half) | **done (HW)** first slice ([ADR-0044](adr/0044-k5-agent-density.md); Pi stamp 2026-08-08); driver-half collapse residual | `spawn_thin` 4 KiB stacks; pure density arithmetic | 0023 → 0044 |
| K6 | External agent load + byte manifest | **done (QEMU)** ([ADR-0027](adr/0027-h1-external-agent-store.md) format, [ADR-0029](adr/0029-agent-store-in-image.md) placement) | Image store inject; product prefers store, oracle empty → builtin | ADR-0021 → 0027 → 0029 |
| K7 | ASID (+ TTBR1 if required) | **in design** ([ADR-0047](adr/0047-k7-asid-isolation-design.md) accepted; code deferred) | ASID pool + CONTEXTIDR on switch | 0014 → 0047 |
| K8 | SMP | **in design** ([ADR-0048](adr/0048-k8-smp-design.md) accepted; code deferred) | Unpark secondary + per-core queues | 0006 → 0048 |
| K9 | Driver-as-agent beyond PL011 (+ IRQ caps) | **done (HW)** ([ADR-0034](adr/0034-k9-rng-driver-agent.md) + [ADR-0043](adr/0043-k9-irq-device-agent.md); Pi stamp 2026-08-08) | Map agent + IRQ-cap-only wait agent | 0013 → 0034 → 0043 |
| K10 | Supervisor lifecycle (restart, creator exit) | **done (HW)** ([ADR-0033](adr/0033-k10-supervisor-reap.md) + [ADR-0038](adr/0038-k10-creator-exit-cascade.md); Pi stamp 2026-08-08); force-kill Running later | `supervisor_reap_blocked`; exit cascades cancel of blocked children | 0018/0025 → 0033 → 0038 |

---

## P — product operating system

Product tracks deliver **services and platform paths** as agents/compositions,
not as a growing special-case syscall surface ([vision](vision.md) shape).

| ID | Track | Status | Done when (sketch) | Typical deps | Horizon |
| --- | --- | --- | --- | --- | --- |
| P1 | Multi-agent product image beyond beacon | **done (QEMU)** first slice (beacon + chirp in store) | Product store n≥2; both run via console endpoint | ADR-0027/0029 | H1 |
| P2 | Storage path (block + load/persist) | **done (HW)** first slices ([ADR-0036](adr/0036-p2-keyed-blob-store.md) + [ADR-0045](adr/0045-p2-durable-store.md); Pi stamp 2026-08-08); SD/power-cycle + EL0 residual | Put/get + durable section | 0036 → 0045 | H1 → H2 |
| P3 | Network agent + caps | **open** — deferred ([ADR-0049](adr/0049-deferred-residuals.md): no composition target) | Network I/O only via granted caps | K1/K9 helpful | H1 edge → H2 |
| P4 | Display/input product path | **open** — deferred ([ADR-0049](adr/0049-deferred-residuals.md): no composition target) | Product path beyond `debug-display` | Device agents | H1 UI → H2 |
| P5 | Naming / discovery / system services | **done (HW)** ([ADR-0035](adr/0035-p5-name-registry.md) + [ADR-0039](adr/0039-p5-el0-resolve.md); Pi stamp 2026-08-08); resolve-grant residual | Bind/resolve; EL0 installs into empty slot | 0035 → 0039 | H1 → H2 |
| P6 | Compose/audit tooling | **done (QEMU)** first slice (pack / inject / inspect) | Host tools for store composition and audit | P1 | H1 |

---

## Next working order (post H1 HW stamp)

Priority is **mission fit**, not ID order. ADR before boundary code.

| Step | Track(s) | Why now |
| --- | --- | --- |
| 1 | **K7** ASID first code *or* **K4** IRQ preemption design ADR | H2 entry — isolation vs fairness |
| 2 | **P2 SD/power-cycle** re-read on Pi | Closes true-media residual while lab is hot |
| 3 | **Resolve-grant** / peer transfer design | Non-ambient authority depth |
| — | **K8** SMP unpark | After K7 or when dual-core gate is ready |
| — | **P3** / **P4** | Only with a named composition (ADR-0049) |

**H1 entry + depth first slices are paid (HW stamp 2026-08-08).**  
P3/P4 deferred. H2: budget done (HW); preemption/ASID/SMP code next.

```text
Mission: agents · grants · evidence · finish the OS
                │
    H0 foundation ████████ done (HW)
                │
    H1 composition ████████ done (HW) stamp 2026-08-08
                │          residuals: SD power-cycle · peer xfer · resolve-grant
                │
    H2 boundary    ░░██████ K4 budget HW; next: preemption ADR / K7 code / K8
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
