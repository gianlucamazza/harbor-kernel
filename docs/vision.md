# Vision — Harbor as a capability composition OS

> Harbor is an OS where software arrives as **agents**, authority arrives as
> **grants**, and every boundary can be **shown to hold** — and the project
> **finishes** that OS, mechanism by mechanism and service by service.

That is a **goal**, not a claim that the product is finished today.

Harbor is not a small Linux. The system *is* isolated agents, talking only over
controlled channels, authorized only by explicit grants. The name means a
protected place for bounded components
([ADR-0007](adr/0007-project-identity-harbor-kernel.md)).

**“Agent” is not an LLM runtime.** It is the isolation unit (today: an EL1
driver task plus an EL0 program —
[ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)). Hosting
tool-limited software inside an agent is a future *use* of that unit.

| This document | Elsewhere |
| --- | --- |
| Product shape, horizons, use cases | — |
| Completeness **policy** and K/P **table** | [ADR-0026](adr/0026-kernel-and-product-completeness.md), [architecture § completeness](architecture.md#completeness-roadmap) |
| What is done on silicon | [architecture](architecture.md), [verification](verification.md) |
| Threat model | [`SECURITY.md`](../SECURITY.md) |

Dropping completeness as the project goal needs a successor to ADR-0026.
Horizon narrative may change without an ADR; structural boundaries become
design ADRs.

---

## In one page

| | |
| --- | --- |
| **Shape** | Small kernel TCB · agents (app/driver/service) · compositions (manifest / grant graph) |
| **Invariants** | No ambient authority · messages as boundary · enumerable grants · evidence ≠ compile |
| **H0 (today)** | Foundation complete on Pi 4B; kernel/product **not yet** complete |
| **H1** | Appliance / composition OS (early K + multi-agent product) |
| **H2** | Full boundary OS (remaining K/P: preemption, network, naming, tooling, …) |
| **Roadmap** | [K and P tracks](architecture.md#completeness-roadmap) |

---

## Invariants that survive into an OS

| Invariant | Implication |
| --- | --- |
| No ambient authority | Software owns only the slots it was given |
| Messages as the logical boundary | No ambient shared-heap API between agents |
| Agent as uniform isolation unit | Apps, drivers, and services differ by **grants**, not by kind |
| Creator / supervisor decides fate | Fault and kill are policy, not silent kernel magic |
| Authority is enumerable | Audit reads a grant table |
| Evidence ≠ compile | Boundaries stay claims with gates |

Open work that realises these at scale (design ADR before code): denser agents
(**K5**), preemption/budget (**K4**), SMP (**K8**), product storage/network
(**P2+**). First slices paid: external load (**K6**), IRQ wait (**K1**),
multi-agent product store (**P1**).

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

Close early **K** tracks and **P1** (real multi-agent product), with pieces of
storage/display as needed.

| Use case | Why the shape fits |
| --- | --- |
| Modular robot / industrial stacks | Sensor, control, logging as separate agents |
| Least-privilege edge gateways | Only the net agent holds the NIC (when it exists) |
| Sealed composition firmware | Kernel + grant table; update one agent without every device |
| Third-party sandbox on-device | They supply text; you supply grants |

**Paid (first slices):** external load (**K6**, [ADR-0027](adr/0027-h1-external-agent-store.md) +
[ADR-0029](adr/0029-agent-store-in-image.md) image inject); IRQ wait EL1
(**K1**, [ADR-0028](adr/0028-wait-on-irq.md)); multi-agent product store
(**P1**, beacon+chirp). EL0 IRQ caps still open.

**Must still pay:** more device agents (**K9**), reclaim beyond cancel (**K2**),
agent density (**K5**), EL0 IRQ capability, on-target FS/storage (**P2**),
network (**P3**).

### H2 — Boundary operating system

Remaining **K** and **P**: preemption/density, cap economy, network, naming,
tooling. Full OS sense — still not Linux.

| Traditional OS | Harbor (vision) |
| --- | --- |
| Process + ambient files/sockets | Agent + slots; nothing exists until passed |
| In-kernel or half-trusted drivers | Driver-agents with named maps |
| Coarse install permissions | Authority *is* the grant row / graph |
| Huge compatibility ABI | Small versioned surface |

**Use cases:** multi-app capability-first devices; grant-graph distribution;
supervised long-lived systems; tool-limited autonomous workers; least-privilege
research platform.

---

## What this vision refuses

- Linux/POSIX parity as a goal  
- Multi-tenant cloud hypervisor (unless a future ADR owns it)  
- Being an AI agent framework (may *host* workers; is not a chat SDK)  
- Microkernel fashion without the confinement story  

---

## Completeness

Evidence question: can each boundary be inspected, tested, and shown on silicon?

Product question ([ADR-0026](adr/0026-kernel-and-product-completeness.md)): keep
going until the **kernel and product OS are complete** under this model.

Mechanics: [architecture](architecture.md).  
Evidence: [verification](verification.md).
