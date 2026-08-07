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
| Completeness **policy** and K/P **table** | [ADR-0026](adr/0026-kernel-and-product-completeness.md), [roadmap](roadmap.md) |
| What is done on silicon | [architecture](architecture.md), [verification](verification.md) |
| Threat model | [`SECURITY.md`](../SECURITY.md) |

Dropping completeness as the project goal needs a successor to ADR-0026.
Horizon narrative may change without an ADR; structural boundaries become
design ADRs.

---

## Who this is for

| Written for | What they get |
| --- | --- |
| Systems contributors on bare metal | A small AArch64 kernel where every boundary has a gate and an ADR behind it |
| Capability / isolation researchers | A working slot-indexed authority model on real silicon, with the residuals named ([`SECURITY.md`](../SECURITY.md)) |
| Anyone building a composable appliance on a Pi 4 | Agents + a grant graph instead of a distro to strip down |
| Anyone evaluating the project | Status that distinguishes `done (QEMU)` from `done (HW)`, and open work called open |

**Not written for** people who want Linux/POSIX or a distro, a cloud
hypervisor, an LLM/agent chat framework, or a board other than the Raspberry
Pi 4B today. Those are refusals or open ports, not oversights — see
[what this vision refuses](#what-this-vision-refuses) and
[`porting.md`](porting.md).

If a word here does not mean what you expect — **agent** most of all — start at
[`glossary.md`](glossary.md).

---

## In one page

| | |
| --- | --- |
| **Shape** | Small kernel TCB · agents (app/driver/service) · compositions (manifest / grant graph) |
| **Invariants** | No ambient authority · messages as boundary · enumerable grants · evidence ≠ compile |
| **H0 (today)** | Foundation complete on Pi 4B; kernel/product **not yet** complete |
| **H1** | Appliance / composition OS (early K + multi-agent product) |
| **H2** | Full boundary OS (remaining K/P: preemption, network, naming, tooling, …) |
| **Roadmap** | [K and P tracks](roadmap.md) |

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

Open work that realises these at scale (design ADR before code): cap **transfer**
(K3 residual), denser agents (**K5**),
preemption/budget (**K4**), SMP (**K8**), product storage/network/naming
(**P2–P5**), K2 timeout residual. First slices paid: external load (**K6**),
IRQ wait EL1+EL0 (**K1**), last-SEND-hold auto-reap (**K2**), channel revoke
(**K3**), multi-agent product store (**P1**), compose tools (**P6**). Status
and H1 order: [roadmap](roadmap.md).

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

| Use case | Why the shape fits |
| --- | --- |
| Modular robot / industrial stacks | Sensor, control, logging as separate agents |
| Least-privilege edge gateways | Only the net agent holds the NIC (when it exists) |
| Sealed composition firmware | Kernel + grant table; update one agent without every device |
| Third-party sandbox on-device | They supply text; you supply grants |

**Paid (first slices, QEMU):** external load (**K6**,
[ADR-0027](adr/0027-h1-external-agent-store.md) +
[ADR-0029](adr/0029-agent-store-in-image.md)); wait-on-IRQ EL1+EL0 (**K1**,
[ADR-0028](adr/0028-wait-on-irq.md) + [ADR-0030](adr/0030-el0-irq-capability.md));
last-SEND-hold auto-reap (**K2**, [ADR-0031](adr/0031-k2-last-send-hold-auto-reap.md));
channel revoke (**K3**, [ADR-0032](adr/0032-k3-channel-revoke.md)); supervisor
reap/restart (**K10**, [ADR-0033](adr/0033-k10-supervisor-reap.md)); multi-agent
product store (**P1**); host compose tools (**P6**).

**Must still pay (H1 critical path):** second driver-agent (**K9**), naming
(**P5**), on-target storage (**P2**); network (**P3**) and product display
(**P4**) when needed; density (**K5**). Residuals: K2 **timeout**, K3 **transfer**,
K10 creator-exit cascade. Working order:
[roadmap § H1 working order](roadmap.md#h1-working-order-product-critical-path).

### H2 — Boundary operating system

Remaining **K** and **P** for full boundary-OS depth: preemption/budget
(**K4**), ASID/isolation (**K7**), SMP (**K8**), denser agents, HW stamps, and
remaining product-path depth. Full OS sense under this model — still not Linux.

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
