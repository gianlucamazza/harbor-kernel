# Vision — Harbor as a capability composition OS

## Mission (canonical)

This document **owns** the mission sentence. Everywhere else — the root README,
[`roadmap.md`](roadmap.md) — quotes it verbatim between the same markers, and
`make doc-claims` fails the build if a copy drifts. Reword it here, or not at
all.

<!-- mission:begin -->

> Harbor is an OS where software arrives as **agents**, authority arrives as
> **grants**, and every boundary can be **shown to hold** — and the project
> **finishes** that OS, mechanism by mechanism and service by service.

<!-- mission:end -->

That is a **goal**, not a claim that the product is finished today.

Harbor is not a small Linux. The system _is_ isolated agents, talking only over
controlled channels, authorized only by explicit grants. The name means a
protected place for bounded components
([ADR-0007](adr/0007-project-identity-harbor-kernel.md)).

**“Agent” is not an LLM runtime.** It is the isolation unit (today: an EL1
driver task plus an EL0 program —
[ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)). Hosting
tool-limited software inside an agent is a future _use_ of that unit.

| This document                             | Elsewhere                                                                      |
| ----------------------------------------- | ------------------------------------------------------------------------------ |
| Product shape, horizons, use cases        | —                                                                              |
| Completeness **policy** and K/P **table** | [ADR-0026](adr/0026-kernel-and-product-completeness.md), [roadmap](roadmap.md) |
| What is done on silicon                   | [architecture](architecture.md), [verification](verification.md)               |
| Threat model                              | [`SECURITY.md`](../SECURITY.md)                                                |

Dropping completeness as the project goal needs a successor to ADR-0026.
Horizon narrative may change without an ADR; structural boundaries become
design ADRs.

---

## Who this is for

| Written for                                      | What they get                                                                                                      |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------ |
| Systems contributors on bare metal               | A small AArch64 kernel where every boundary has a gate and an ADR behind it                                        |
| Capability / isolation researchers               | A working slot-indexed authority model on real silicon, with the residuals named ([`SECURITY.md`](../SECURITY.md)) |
| Anyone building a composable appliance on a Pi 4 | Agents + a grant graph instead of a distro to strip down                                                           |
| Anyone evaluating the project                    | Status that distinguishes `done (QEMU)` from `done (HW)`, and open work called open                                |

**Not written for** people who want Linux/POSIX or a distro, a cloud
hypervisor, or an LLM/agent chat framework. A board other than the Raspberry
Pi 4B is **not product today** (open port / host-class path — [ADR-0069](adr/0069-harbor-host-class-north-star.md)), not an oversight. See
[what this vision refuses](#what-this-vision-refuses) and
[`porting.md`](porting.md).

If a word here does not mean what you expect — **agent** most of all — start at
[`glossary.md`](glossary.md).

---

## In one page

|                     |                                                                                                                                                                               |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Shape**           | Small kernel TCB · agents (app/driver/service) · compositions (manifest / grant graph)                                                                                        |
| **Invariants**      | No ambient authority · messages as boundary · enumerable grants · evidence ≠ compile                                                                                          |
| **H0 (today)**      | Foundation complete on Pi 4B; kernel/product **not yet** complete                                                                                                             |
| **H1**              | Appliance / composition OS (early K + multi-agent product)                                                                                                                    |
| **H2**              | Full boundary OS (remaining K/P: preemption, network, naming, tooling, …)                                                                                                     |
| **H3 (north star)** | Host-class **native** Harbor — usable in place of the previous OS for a named workload ([ADR-0069](adr/0069-harbor-host-class-north-star.md)); not a claim of readiness today |
| **Roadmap**         | [K and P tracks](roadmap.md)                                                                                                                                                  |

---

## Invariants that survive into an OS

| Invariant                         | Implication                                                   |
| --------------------------------- | ------------------------------------------------------------- |
| No ambient authority              | Software owns only the slots it was given                     |
| Messages as the logical boundary  | No ambient shared-heap API between agents                     |
| Agent as uniform isolation unit   | Apps, drivers, and services differ by **grants**, not by kind |
| Creator / supervisor decides fate | Fault and kill are policy, not silent kernel magic            |
| Authority is enumerable           | Audit reads a grant table                                     |
| Evidence ≠ compile                | Boundaries stay claims with gates                             |

Open work that realises these at scale (design ADR before code where needed):
**peer** cap transfer depth residuals, **K5** driver-half collapse residual
(thin stacks done HW), **K7** residuals (TTBR1 / switch-cost), **K8** residual
steal / per-core preempt / EL0-on-CPU1 (unpark+IPI+queues first **done (HW)**,
[ADR-0070](adr/0070-k8-smp-first-slice.md)/[0074](adr/0074-k8-ipi-wake-second-slice.md)/[0076](adr/0076-k8-per-core-queues-first-slice.md)/[0077](adr/0077-smp-shared-state-discipline.md)),
product network/display only with a composition target (**P3**, **P4** deferred).
Closed on HW for fairness: **K4** budget + EL0 + EL1 IRQ-epilogue preemption
([ADR-0064](adr/0064-k4-el0-preemption-first-slice.md)/[0068](adr/0068-k4-el1-preemption-second-slice.md)).
First slices paid: external load (**K6**), IRQ wait EL1+EL0 (**K1**), last-SEND-hold
auto-reap + park timeout (**K2**), channel revoke + EL1/EL0 transfer (**K3**),
multi-agent product store (**P1**), on-target + durable blobs + SD power-cycle (**P2**),
name registry + EL0 resolve (**P5**), compose tools (**P6**), thin stacks (**K5**),
cooperative budget + IRQ preemption (**K4**). Status and H1 order: [roadmap](roadmap.md).

---

## Shape of the system

```
┌─────────────────────────────────────────────────────────────┐
│  Compositions (manifest / grant graph)                      │
│  who exists, what each may name                             │
└───────────────────────────▲─────────────────────────────────┘
                            │ load / bind
┌───────────────────────────┴─────────────────────────────────┐
│  Agents                                                     │
│  app · driver · service · supervisor · tool-limited worker  │
│  private AS · slot caps · messages                          │
└───────────────────────────▲─────────────────────────────────┘
                            │ SVC / maps / (future) IRQ caps
┌───────────────────────────┴─────────────────────────────────┐
│  Small kernel TCB                                           │
│  mm · sched · ipc/caps · irq · arch · thin bootstrap        │
└─────────────────────────────────────────────────────────────┘
```

Filesystem, network, UI, and high-level policy belong **above** the kernel as
agents or compositions — the same idea as the console endpoint (server + send
caps), not an ever-growing special-case syscall surface.

---

## Horizons

### H0 — Foundation (today)

Single-core Pi 4B; cooperative tasks; EL0 agents; slot caps; manifest loader;
PL011 driver-agent; blocking recv; console endpoint + beacon. Foundation
through M8 is **done (HW)** — see [architecture](architecture.md).

**Already useful for:** boundary lab, teaching capability systems, static
appliance images, contained I/O agents, cooperative pipelines, fault
supervision demos, verification methodology.

H0 is **not** the end of the project: foundation complete, OS not yet complete.

### H1 — Composition / appliance OS

Close the **composition bar**: multi-agent product images, loadable stores,
early device/supervisor story, and enough authority lifecycle that compositions
are not reboot-scoped demos. Storage/net/display land as **agents** when a
concrete composition needs them — not as ambient kernel features.

| Use case                          | Why the shape fits                                          |
| --------------------------------- | ----------------------------------------------------------- |
| Modular robot / industrial stacks | Sensor, control, logging as separate agents                 |
| Least-privilege edge gateways     | Only the net agent holds the NIC (when it exists)           |
| Sealed composition firmware       | Kernel + grant table; update one agent without every device |
| Third-party sandbox on-device     | They supply text; you supply grants                         |

**Paid (first slices, QEMU):** external load (**K6**,
[ADR-0027](adr/0027-h1-external-agent-store.md) +
[ADR-0029](adr/0029-agent-store-in-image.md)); wait-on-IRQ EL1+EL0 (**K1**,
[ADR-0028](adr/0028-wait-on-irq.md) + [ADR-0030](adr/0030-el0-irq-capability.md));
last-SEND-hold auto-reap (**K2**, [ADR-0031](adr/0031-k2-last-send-hold-auto-reap.md));
channel revoke (**K3**, [ADR-0032](adr/0032-k3-channel-revoke.md)); supervisor
reap/restart (**K10**, [ADR-0033](adr/0033-k10-supervisor-reap.md)); RNG map agent
(**K9**, [ADR-0034](adr/0034-k9-rng-driver-agent.md)); multi-agent
product store (**P1**); on-target keyed blobs (**P2**,
[ADR-0036](adr/0036-p2-keyed-blob-store.md)); name registry (**P5**,
[ADR-0035](adr/0035-p5-name-registry.md)); host compose tools (**P6**).

**H1 entry + first-slice depth are paid (HW)** (composition bar, lifecycle
residuals, **K5** thin stacks, **P2** durable + SD power-cycle, **K4** budget +
EL0/EL1 preemption). Still open: network (**P3**) and product display (**P4**)
**only when a composition needs them** (deferred without a target); **K5**
driver-half collapse residual. Working order:
[roadmap § H1 working order](roadmap.md#next-working-order-post-h1-hw-stamp).

### H2 — Boundary operating system

Remaining **K** and **P** for full boundary-OS depth: **K7** residuals
(TTBR1 / switch-cost), **K5** driver-half, **K8** steal / per-core preempt /
EL0-on-CPU1 (queues first **done (HW)** [ADR-0076](adr/0076-k8-per-core-queues-first-slice.md)/[0077](adr/0077-smp-shared-state-discipline.md);
design [ADR-0048](adr/0048-k8-smp-design.md)/[0075](adr/0075-k8-per-core-queues-design.md)), deferred **P3**/**P4**, peer
transfer depth residuals ([ADR-0053](adr/0053-k3-peer-transfer-design.md)), and
remaining product-path depth. **K4** IRQ preemption is closed on HW
([ADR-0064](adr/0064-k4-el0-preemption-first-slice.md)/[0068](adr/0068-k4-el1-preemption-second-slice.md);
design [ADR-0051](adr/0051-k4-irq-preemption-design.md)). Resolve-grant is done
(HW) ([ADR-0052](adr/0052-p5-resolve-grant.md); Pi stamp 2026-08-09). Full OS sense under this model
— still not Linux.

| Traditional OS                    | Harbor (vision)                            |
| --------------------------------- | ------------------------------------------ |
| Process + ambient files/sockets   | Agent + slots; nothing exists until passed |
| In-kernel or half-trusted drivers | Driver-agents with named maps              |
| Coarse install permissions        | Authority _is_ the grant row / graph       |
| Huge compatibility ABI            | Small versioned surface                    |

**Use cases:** multi-app capability-first devices; grant-graph distribution;
supervised long-lived systems; tool-limited autonomous workers; least-privilege
research platform.

### H3 — Host-class native Harbor (north star)

**Long-term destination** ([ADR-0069](adr/0069-harbor-host-class-north-star.md)):
Harbor runs **bare-metal** on host-class hardware (the machine that is today’s
dev host), Linux-free, under the same agent/grant model — usable **in place of**
the previous OS for an owner-named workload. The project name stays **Harbor**
(kernel package: `harbor-kernel`).

This is **not** a ship date and **not** today’s product board. Pi 4B remains
the official platform ([ADR-0007](adr/0007-project-identity-harbor-kernel.md))
until a successor expands it. H0–H2 still prove and complete the model.

Path (maturity levels in ADR-0069):

| Level  | Sketch                                                                        |
| ------ | ----------------------------------------------------------------------------- |
| **L0** | Lab x86 guest under QEMU — **done (QEMU-x86)** ([ADR-0071](adr/0071-h3-l0-x86-qemu-first-slice.md); intent [ADR-0067](adr/0067-host-lab-second-isa-intent.md)) |
| **L1** | Bare-metal laptop bring-up (console-class)                                    |
| **L2** | Self-host tools on Harbor                                                     |
| **L3** | Daily slice — named workload without rebooting to Linux for that work         |
| **L4** | Primary OS for the declared life-slice; Linux optional recovery only          |

Lab QEMU and multi-arch practices are **steps**, not the ceiling. Replacement
means Harbor underneath, not Linux ABI compatibility.

---

## What this vision refuses

- Linux/POSIX parity as a goal (including “run unmodified Linux apps” as the path to daily use)
- Multi-tenant cloud hypervisor (unless a future ADR owns it)
- Being an AI agent framework (may _host_ workers; is not a chat SDK)
- Microkernel fashion without the confinement story
- Claiming host-class primary OS readiness before L3 evidence ([ADR-0069](adr/0069-harbor-host-class-north-star.md))

---

## Completeness

Evidence question: can each boundary be inspected, tested, and shown on silicon?

Product question ([ADR-0026](adr/0026-kernel-and-product-completeness.md)): keep
going until the **kernel and product OS are complete** under this model.

Mechanics: [architecture](architecture.md).  
Evidence: [verification](verification.md).
