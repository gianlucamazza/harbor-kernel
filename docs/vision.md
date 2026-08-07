# Vision — Harbor as a complete capability composition OS

This document owns **product shape and use cases**. It does **not** own silicon
status, evidence, or the operational threat model. The project **goal** of
completeness (kernel + product OS) is normative in
[ADR-0026](adr/0026-kernel-and-product-completeness.md) and the
[completeness roadmap](architecture.md#completeness-roadmap).

| This document | Look elsewhere |
| --- | --- |
| Product shape and use cases | — |
| Horizons H0 / H1 / H2 mapped to K/P tracks | — |
| Completeness policy and track table | [ADR-0026](adr/0026-kernel-and-product-completeness.md), [`architecture.md`](architecture.md) |
| What “done today” means | [`architecture.md`](architecture.md), [`verification.md`](verification.md) |
| Authority claims that must hold now | [`../SECURITY.md`](../SECURITY.md) |

When a piece of this vision becomes a structural boundary, it becomes a design
ADR. Horizon narrative may change without an ADR; dropping completeness as the
goal requires a successor to ADR-0026.

---

## Thesis

Harbor is not aiming to be a small Linux. It is completing a **capability
composition OS**: the system *is* a set of isolated agents, authorized only by
explicit grants, talking only over controlled channels — and every boundary is
meant to be demonstrated, not merely asserted.

The project name already carries that idea ([ADR-0007](adr/0007-project-identity-harbor-kernel.md)):
a protected place where independently bounded components operate and
communicate.

**Slogan (goal, not a claim that the OS is finished today):**

> Harbor is an OS where software arrives as agents, authority arrives as
> grants, and every boundary can be shown to hold — and the project finishes
> that OS, mechanism by mechanism and service by service.

**“Agent” here is not an LLM runtime.** It is the isolation unit (today: an EL1
driver task paired with an EL0 program — [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)).
A future host for tool-limited autonomous software is one *use case of that
model*, not a redefinition of the word.

---

## Invariants that survive into an OS

If Harbor becomes a full operating system, these load-bearing choices should
still be true. Extensions get successor ADRs; reversals need explicit ones.

| Invariant | OS implication |
| --- | --- |
| No ambient authority | Software does not “own the machine”; it owns only the slots it was given |
| Messages as the logical boundary | Agents do not share an ambient heap API; they send and receive |
| Agent as the uniform isolation unit | Apps, drivers, and services are the same abstraction at different grants |
| Creator / supervisor decides fate | Fault and kill are policy of who created the agent, not silent kernel magic |
| Authority is enumerable | Audit and threat models read a grant table, not the whole kernel |
| Evidence ≠ compile | Boundaries remain claims with gates (the gate set may grow) |

Things on the **completeness roadmap** (open tracks; design ADR before code):

- agent pair cost → density work (**K5**; [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md));
- cooperative only → preemption or budgets (**K4**; successor to [ADR-0006](adr/0006-cooperative-execution-model.md));
- single core → SMP (**K8**);
- manifest in the image → external load (**K6**; [ADR-0021](adr/0021-agents-as-data-and-the-manifest.md));
- IRQ not a first-class wait → IRQ wait/caps (**K1**);
- product storage / network / multi-app → **P2–P5**.

---

## Shape of the system

```
┌─────────────────────────────────────────────────────────────┐
│  Compositions (manifest / grant graph)                      │
│  who exists, what each may name                             │
└───────────────────────────▲─────────────────────────────────┘
                            │ load / bind
┌───────────────────────────┴─────────────────────────────────┐
│  Agents (uniform abstraction)                               │
│  app · driver · service · supervisor · tool-limited worker  │
│  private address space · slot caps · messages               │
└───────────────────────────▲─────────────────────────────────┘
                            │ SVC / maps / (future) IRQ caps
┌───────────────────────────┴─────────────────────────────────┐
│  Small kernel TCB                                           │
│  mm · sched · ipc/caps · irq · arch · thin bootstrap        │
└─────────────────────────────────────────────────────────────┘
```

The kernel keeps address spaces, endpoints, scheduling, named maps, and
lifecycle. Filesystems, networks, UI, and high-level policy belong above it as
agents or compositions — the same pattern M8 already uses for console output
(EL1 server + send caps, not a special print ABI forever).

---

## Horizons

### H0 — Lab kernel (today)

**What it is:** single-core AArch64 on Pi 4B; cooperative tasks; EL0 agents;
slot caps; manifest loader; driver-as-agent (PL011); blocking recv; console
endpoint. Foundation through M8 is **done (HW)** — see
[`architecture.md`](architecture.md).

**Use cases already open**

| Use case | Why the primitives fit |
| --- | --- |
| Boundary laboratory | Isolation, authority, IPC, and fault policy are named and gated on silicon |
| Teaching / research on capability kernels | Small surface; grants in one table; honest residuals in `SECURITY.md` |
| Static multi-agent appliance image | Manifest describes a closed set of agents and grants at boot |
| Contained I/O code | Page-sized device maps; session end on fault; kill restores kernel drain |
| Cooperative message pipelines | Send/recv, park, EL1 servers on endpoints |
| Fault supervision demos | Creator lives after a peer faults ([ADR-0018](adr/0018-agent-fault-policy.md)) |
| Verification methodology transfer | Layering gates, product/oracle split, multi-role review ([ADR-0001](adr/0001-multi-role-analysis.md)) |

H0 is the **foundation complete, kernel/product not yet complete** state. It is
not the end of the project.

### H1 — Near OS: composition runtime / appliance OS

Maps roughly to closing early **K** tracks plus **P1** (and pieces of P2/P4).
The first *recognizable* product OS without betraying the model.

**Software model**

- Almost everything outside the TCB is an agent — including drivers.
- A composition manifest says who exists and what they may touch.
- System services expose endpoints (or thin EL1 infrastructure tasks), not a
  growing special-case syscall surface.

**Use cases H1 opens**

| Use case | Why Harbor’s shape leads here |
| --- | --- |
| Modular industrial / robot stacks | Sensor, control, logging as separate agents; a fault in logging need not map the sensor’s bus |
| Least-privilege edge gateways | Only the network agent would hold the NIC; only telemetry holds the egress path |
| Sealed composition firmware | Image = kernel + grant table; update one agent without granting every device |
| Lab / instrumentation benches | Agents own a peripheral page; kill is a clean release path |
| Third-party code sandbox on-device | Third party supplies EL0 text; you supply grants; console and buses are not ambient |

**Technical debt that H1 must pay (not slogans)**

- Agent images outside the kernel image (storage or load path) and a byte
  manifest format when input is untrusted ([ADR-0021](adr/0021-agents-as-data-and-the-manifest.md)).
- More device agents on the M6 pattern ([ADR-0013](adr/0013-narrow-device-windows.md)).
- IRQ as a first-class wait source for agents.
- Richer reclaim than supervisor `cancel_blocked` alone (timeouts remain a
  separate decision).
- Agent density: 16 KiB kernel stack per driver loop is the scaling wall
  ([ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)).

### H2 — System OS: a boundary operating system

Maps to remaining **K** and **P** tracks (preemption/density, network, naming,
tooling). A full OS in the usual sense — still not a Linux clone.

**Idea:** the kernel is small; the *system* is a grant graph of agents. Install
means binding text to authority, not inheriting a user session full of ambient
rights.

| Traditional OS | Harbor OS (vision) |
| --- | --- |
| Process + user + ambient files/sockets | Agent + slots + endpoints; nothing exists for you until it is passed |
| In-kernel drivers or half-trusted servers | Driver-agents with named maps and (later) IRQ caps |
| App install with coarse permissions | Agent install whose authority *is* its grant row / graph |
| User crash → signal; driver crash → often panic | Session end + supervisor; a driver fault need not be a kernel oops |
| Huge compatibility ABI | Small versioned surface; native Harbor software, not POSIX by default |

**Use cases H2 opens**

1. **Capability-first multi-app devices** (industrial tablet, kiosk, robot HMI):
   each app is an agent; the UI server holds display rights; apps do not.
2. **Grant-graph software distribution:** publish an agent with minimal declared
   grants; integrators compose; audit is reading the graph.
3. **Supervised long-lived systems:** supervisors restart or isolate failed
   agents without rebooting the board.
4. **Tool-limited autonomous workers:** an orchestrator or model runs *inside*
   an agent and receives only the tool caps you grant (serial, one sensor, one
   network endpoint). The OS does not trust the worker’s intent; it trusts the
   slots. This is a future use case, not today’s identity.
5. **Research platform for least-privilege systems:** small TCB, explicit caps,
   agents-as-data, and an evidence culture on real hardware — without claiming
   a machine-checked proof of the TCB unless a later line of work owns that.

**Prerequisites for H2**

- Cheaper agents (collapse or shrink the driver half) and/or preemption/budgets.
- Capability derive/delegate (an economy of rights beyond creator→child grant).
- Endpoint naming, discovery, and composition at scale.
- Persistence and load paths; optional signed manifests.
- SMP / ASID / high-half when the model needs them — each with its own ADR.
- Network, storage, and display as agents when product demand lands.

---

## What this vision refuses

Even at H2, Harbor is not trying to be:

- a Linux/POSIX compatibility layer “with better marketing”;
- a multi-tenant cloud hypervisor (IOMMU, dense SMP isolation, noisy neighbors);
- an AI agent framework — it may *host* tool-limited workers; it is not a
  chat/tool SDK;
- a microkernel “because microkernels are fashionable” — a microkernel because
  drivers and applications should share one confinement model.

---

## How this relates to completeness

The evidence question remains:

> Can a small Rust kernel make isolation, authority, message passing and
> verification visible enough that each boundary can be inspected, tested and
> demonstrated on silicon?

[ADR-0026](adr/0026-kernel-and-product-completeness.md) adds the product
question: keep answering that until the **kernel and product OS are complete**
under this model. H1 and H2 are horizons on that path; K/P tracks are the
backlog.

Current mechanics: [`architecture.md`](architecture.md).
Verified today: [`verification.md`](verification.md).
