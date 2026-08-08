# Glossary

The words Harbor uses in a way that will not match the guess you brought with
you. Each row names the document that **owns** the term; this page is a fast
entry point, not a second definition.

Read this once and [`architecture.md`](architecture.md),
[`roadmap.md`](roadmap.md) and [`verification.md`](verification.md) stop
needing a decoder.

## The model

| Term | Means here | Does **not** mean | Owner |
| --- | --- | --- | --- |
| **Agent** | The unit of isolation: a **pair** of an EL1 driver task and the EL0 program it drives | An LLM, a chatbot, or anything that implies a model in the loop | [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) |
| **Driver task** | The EL1 half of an agent — the thing the scheduler actually runs, including the session loop | "A device driver" (though device drivers are agents too) | [`architecture.md`](architecture.md#an-agent-is-a-pair-and-the-driver-is-the-schedulable-half) |
| **EL0 program** | The unprivileged half: its own address space, its own capability slots | A process (it is not independently schedulable) | [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) |
| **Session** | One enter → SVC → resume → end cycle of an EL0 program inside its driver task. A fault ends the *session*; the creator decides the *task* | A login or a connection | [ADR-0018](adr/0018-agent-fault-policy.md) |
| **Capability** | An unforgeable kernel-side handle (send, recv, IRQ notification) | A permission bit or a POSIX capability | [ADR-0017](adr/0017-el0-capability-abi.md) |
| **Slot** | The **index** EL0 code names when it uses authority. The raw `CapId` never leaves the kernel | A memory slot | [ADR-0017](adr/0017-el0-capability-abi.md) |
| **Task-cap** | A capability naming a **task** (band `cap::TASK_BAND`, decoded by `CapId::classify` — ADR-0059), granting install-into-its-empty-slots via peer transfer. Goes stale on target exit; not itself transferable | A kill/control handle (that is a residual), or a thread id | [ADR-0053](adr/0053-k3-peer-transfer-design.md) / [ADR-0057](adr/0057-taskcap-lifecycle.md) |
| **Grant** | An entry in a manifest that binds one of the loader's capabilities into an agent's slot table | An install-time permission prompt | [ADR-0021](adr/0021-agents-as-data-and-the-manifest.md) |
| **Manifest** | Data describing which agents exist, their image, their window and their slot table — "an agent is data" | A build script | [ADR-0021](adr/0021-agents-as-data-and-the-manifest.md) |
| **Composition** | A set of agents plus the grant graph that says who may name whom | A container image | [`vision.md`](vision.md) |
| **Agent store** | The packed blob of agent images injected into the product image, from which the loader builds the manifest | A package registry | [ADR-0027](adr/0027-h1-external-agent-store.md), [ADR-0029](adr/0029-agent-store-in-image.md) |
| **TCB** | Trusted computing base: the kernel (mm, sched, ipc/caps, irq, arch, thin bootstrap) — everything else is an agent | The task control block (that is a `Tcb` in the code) | [`SECURITY.md`](../SECURITY.md) |
| **Ambient authority** | Power you hold because of *who you are* rather than *what you were passed*. Harbor has none by design | — | [`vision.md`](vision.md) |

## The process vocabulary

| Term | Means here | Owner |
| --- | --- | --- |
| **K-track** | A **kernel** completeness track (K1…K10): a mechanism the microkernel still owes | [`roadmap.md`](roadmap.md) |
| **P-track** | A **product** completeness track (P1…P6): a service or platform path delivered as agents, not as new syscalls | [`roadmap.md`](roadmap.md) |
| **H0 / H1 / H2** | Horizons — product stories, in order: foundation, composition/appliance OS, full boundary OS. Horizons narrate; **only the K/P tables carry status** | [`roadmap.md`](roadmap.md), [`vision.md`](vision.md) |
| **M / P milestone** | The *foundation-era* numbering. **M** added capability, **P** added protection or evidence and no capability at all. Closed at M8 — history, not live planning | [`foundation-history.md`](foundation-history.md) |
| **Oracle** | A demo agent or task whose exact console lines a boot gate asserts on. Lives behind the `oracle` feature so no production image carries it | [`stack.md`](stack.md), [`verification.md`](verification.md) |
| **Gate** | A check wired into `make check` that fails the build. Documentation drift is a failed gate, not a nit | [`CONTRIBUTING.md`](../CONTRIBUTING.md) |
| **Stamp** | An observation on real Pi 4B silicon, with a dated serial transcript behind it | [`verification.md`](verification.md) |
| **Blob** | Closed platform firmware before our entry point. Documented, pinned by hash, minimised — never hidden | [`blobs.md`](blobs.md) |

## Status words

These are load-bearing; they are the reason a status table can be trusted.

| Label | Means |
| --- | --- |
| **implemented** | In the tree now |
| **done (QEMU)** | Exercised under emulation. QEMU has booted a kernel that hung on silicon, so this is not the strong claim |
| **done (HW)** | Observed on a Raspberry Pi 4B, with a transcript |
| **open** | On the completeness roadmap, or awaiting a decision — a gap, not an identity |
| **in design** | An ADR is being written; no boundary code yet |
| **proposed** | Design exists, not accepted or complete |
| **historical** | True at a past date, kept as a record |

Full vocabulary and which document owns which fact:
[`docs/README.md`](README.md#ownership-and-status-vocabulary).
