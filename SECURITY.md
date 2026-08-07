# Security — Harbor Kernel

Threat model and reporting for **harbor-kernel** as of **M7 + the loader**
(EL0 authority by capability slot, agents described by a manifest, and an agent
that can wait — all stamped on Pi 4B silicon 2026-08-07). This document exists
because [ADR-0017](docs/adr/0017-el0-capability-abi.md) finally makes authority
*enumerable*; before that, a threat model would have been fiction about code
that had not drawn the boundary.
[ADR-0021](docs/adr/0021-agents-as-data-and-the-manifest.md) took the step that
sentence was reaching for: the grants an agent is given are now **one table**
rather than a boot function, so this document can name the artefact instead of
describing a shape.

It is **not** a claim that Harbor is production-hardened or multi-tenant ready.
It states what is in scope to defend, what is trusted by construction, what is
verified, and what is still open surface.

---

## Reporting

There is no private bug-bounty channel. For issues that affect isolation or
authority:

1. Prefer a **private** report to the repository owner if disclosure would help
   an attacker on a deployed board; otherwise open a GitHub issue.
2. Include: tree revision, image features (`debug-display` / default), serial
   transcript, and the shortest program or mutation that crosses a boundary
   this document says should hold.

Firmware blobs (`start4.elf`, EEPROM) are third-party; report those upstream
where appropriate ([`docs/blobs.md`](docs/blobs.md)).

---

## What Harbor is, for security purposes

| Role | Description |
| ---- | ----------- |
| **Product** | Single-core AArch64 lab kernel on Raspberry Pi 4 Model B |
| **Goal** | Agent-based microkernel: isolated units interact via messages and capabilities |
| **Today** | Cooperative EL1 tasks + EL0 agents with per-task AS, slot-indexed caps, IPC an agent can **wait** on, and a loader that creates agents from a manifest |

Assets worth defending, in order of load:

1. **Kernel integrity** — code, page tables, heap metadata, GIC/timer/UART driver state.
2. **Authority tables** — who may send/recv on which endpoint, who may print.
3. **Agent isolation** — one EL0 context must not read/write another’s memory or session state.
4. **Availability of the kernel** — a faulting agent must not take down EL1 (ADR-0018).

Non-assets (out of threat model until named otherwise): multi-user login, network
stack, disk encryption, remote attestation, SMP, preemption fairness.

---

## Trust boundaries

```
┌─────────────────────────────────────────────────────────────┐
/* TCB — trusted computing base (single core, lab board)     */
│  EL1 kernel: arch, mm, sched, ipc, irq, drivers, bootstrap │
│  Platform firmware (start4.elf) — Group 0 GIC pin, boot    │
│  Creator of agents: bootstrap::loader + the manifest it reads│
└──────────────────────────▲──────────────────────────────────┘
                           │ SVC / exceptions / map grants
┌──────────────────────────┴──────────────────────────────────┐
/* Untrusted relative to TCB                                  */
│  EL0 agent text + data in its user VA window                │
│  Anything that agent can express through the syscall ABI    │
└─────────────────────────────────────────────────────────────┘
```

| Party | Trust |
| ----- | ----- |
| **EL1 kernel** | Fully trusted. Bugs here are total compromise. |
| **Creator (bootstrap + loader)** | Fully trusted. Chooses AS geometry, caps and entry — now by reading a manifest compiled into the image, which is exactly as trusted as the code it replaced ([ADR-0021](docs/adr/0021-agents-as-data-and-the-manifest.md) §6). Not itself isolated. |
| **EL0 agent** | **Untrusted.** May be wrong, malicious, or hostile. Must not gain authority it was not granted, nor corrupt kernel or peer memory. |
| **Serial adversary** | Can type bytes on PL011. While the **kernel** owns RX the bytes land in a bounded ring the idle loop drains, and an overflow is counted (`RX_DROPPED`), not a memory path. While an **agent** owns RX (M6/M7 handover) the bytes are delivered to untrusted EL0 code by design — that is the feature — so the adversary's reach is exactly the agent's, and the agent is already untrusted. What neither case gives is a way to reach kernel memory or another agent. |
| **SPI TFT path** | Lab-only (`debug-display`). Not a security boundary; GPIO/SPI mistakes can hang the boot, not elevate EL0. |
| **QEMU** | Not evidence for memory-attribute or firmware-state claims ([`docs/verification.md`](docs/verification.md)). |

---

## Attacker model (in scope)

An adversary who can load or influence **one or more EL0 agents** (their
machine code and the registers they set before each `svc`), and who may share
the machine with other agents under the same kernel, tries to:

| Goal | In scope? |
| ---- | --------- |
| Read/write kernel memory from EL0 | Yes — must fail (permission / translation) |
| Forge a capability (name another task’s authority) | Yes — must fail (slot index, not raw `CapId`) |
| Use a slot not granted (empty / OOB / wrong rights) | Yes — refuse + authority counter |
| Print without a console capability | Yes — refuse; byte must not appear on UART |
| Crash the kernel by faulting at EL0 | Yes — session ends; creator handles; kernel lives |
| Steal another agent’s saved GPRs / session | Yes — `CURRENT_EL0` publish + assert on entry |
| Exhaust frames / tables to DoS later agents | Partially — pool is finite; destroy should return frames |
| Busy-loop at EL0 and starve the system | **Out of scope** — cooperative model (ADR-0006); no preemption |
| Park forever on a mailbox nobody sends to | **Partially** — the park is voluntary and cannot be forced on a peer, but nothing reclaims the slot. See the residual risk below; new with [ADR-0022](docs/adr/0022-blocking-recv-and-the-mask-that-travels.md) |
| Take a second waiter's place on an endpoint | Yes — `Table::park` refuses (`Status::Busy`) and counts a state refusal. One endpoint, one waiter |
| Feed an RX-owning agent hostile input from the wire | Yes, and it is *supposed* to arrive — the agent is untrusted either way. What must hold is that the handover cannot leave the line armed with nothing to drain (`kernel_core::rxline`, host-tested) |
| Attack via firmware / JTAG / SD swap | Out of physical-lab model (operator is trusted) |
| Remote network exploit | No network stack |

---

## Authority surface (what an agent can name)

Defined by [ADR-0017](docs/adr/0017-el0-capability-abi.md) and
`kernel_core::syscall`:

| Imm | Call | Authority required |
| --- | ---- | ------------------ |
| 0 | `SYS_PING` | none |
| 1 | `SYS_EXIT` | none |
| 3 | `SYS_SEND` | `CapRights::SEND` on endpoint in slot (console output uses this with tag 0 and the byte in `a` — M8) |
| 4 | `SYS_RECV` | `CapRights::RECV` on endpoint in slot — **waits** if the mailbox is empty ([ADR-0022](docs/adr/0022-blocking-recv-and-the-mask-that-travels.md)) |
| 5 | `SYS_TRY_RECV` | `CapRights::RECV` on endpoint in slot, **never waits** — answers `Empty` |

Imm 2 is unassigned (formerly transitional `SYS_PUTC`); `decode(2)` is `Unknown`.

**Structural property:** EL0 passes a **slot index** into its own
`Tcb.caps[MAX_CAPS_PER_TASK]` (`MAX_CAPS_PER_TASK = 4`). It cannot form a
`CapId` that names another task’s grant. EL1 still uses `CapId` +
`current_holds` for its own demos.

**Refusals** are split so a full mailbox is not counted as forgery:

- `authority` — unheld / empty / OOB slot
- `full` — flow control
- `state` — dead endpoint (unreachable until release exists)

Constants load-bearing for the model: mailboxes **8**, endpoints **16**, mailbox
depth **4** (ADR-0017); capability slots per task **4**; concurrent tasks
including idle **16** (`sched::MAX_TASKS` — it bounds how many agents can be
parked at once, see the residual risk below); executable pages an agent may
declare **16** (`mm::MAX_TEXT_PAGES`, 64 KiB).

---

## Isolation mechanisms (and what they do not claim)

| Mechanism | Holds today | Limit |
| --------- | ----------- | ----- |
| W^X kernel map + guard pages | Yes (fault-probed HW) | Protects kernel **from itself** more than from a mapped peer |
| Per-agent `TTBR0` + user VA window | Yes (M5 HW) | Kernel maps **cloned** into user root with EL0-denied AP ([ADR-0014](docs/adr/0014-ttbr-split-m5.md) option C) — not TTBR1 high-half |
| Page-sized device maps for agents | Yes (M6, PL011) | Kernel still has coarse Device windows until a P-pass |
| Slot-indexed caps | Yes (M7 HW) | No transfer; grants only at creation |
| Fault → end session, creator decides | Yes (ADR-0018, M7 HW) | Creator exit leaves agents unsupervised; no restart policy |
| Published `CURRENT_EL0` (`AtomicPtr`, ADR-0019) | Yes (HW 2026-08-07) | Stale publish panics on entry; residual: assembly assumes symbol is a pointer |
| Grants bounded by the loader's own table ([ADR-0021](docs/adr/0021-agents-as-data-and-the-manifest.md)) | Yes (HW 2026-08-07) | The manifest is **in the image** — as trusted as the code it replaced. It makes authority legible, not dynamic: no revocation, no delegation |
| Per-agent window geometry, W^X inside it | Yes (HW 2026-08-07) | `text_pages` executable, the rest writable, never both. A larger window costs frames from a 512-frame pool and is refused as an error, not a panic |
| One mask per session step, never across a switch ([ADR-0022](docs/adr/0022-blocking-recv-and-the-mask-that-travels.md)) | Yes | `make irq-scope` is **lexical**: a call that switches three frames down passes it. The indirect form is review's |

---

## Guarantees we claim (and how they are checked)

Only claims with a gate or silicon stamp belong here. A prose rule without a
check is an assumption — see [`docs/verification.md`](docs/verification.md).

| Claim | Evidence |
| ----- | -------- |
| EL0 store to kernel text → data abort (permission) | boot-check + HW `el0: FAULT ok ESR=…` |
| Unknown SVC imm ends / refuses the session path | `el0-task: svc refuse` |
| Send/recv without hold → authority refusal | M4 forger + M7 `refused slot=1` |
| Console denied by default; denied byte absent | `console denied` + boot-check asserts no `X` |
| Creator survives agent fault; peer continues | `creator alive after fault` + ordering on serial |
| No `static mut` in `src/` | `make no-static-mut` |
| The authority table matches its specification | `tests/model_ipc.rs` — 5 229 043 sequences over `Table<2,4,2>`, every observable compared against a reference implementation |
| The scheduler's invariants hold in every reachable state | `tests/model_sched.rs` — 2 396 745 sequences over `Tasks<3>`, five invariants after every step |
| Softfloat / no FP in image | `make no-simd` |
| Layering (no driver→board, arch≠drivers) | `make layering` / `arch-board-free` |
| A manifest entry cannot name authority the loader lacks | Host tests in `kernel_core::manifest`; on silicon, `loader: mute ran sends=0 refusals=2` beside `loader: beacon ran sends=2 refusals=0` — **the same image**, differing only in the table |
| A `DAIF` save/restore pair never spans a task switch | `make irq-scope`, seen red on a planted `yield_now` |
| The syscall ABI here matches the kernel's | `make doc-claims` compares this table's immediates with `kernel_core::syscall` — the set only; whether a row's *description* is true is still review's job |

---

## Known non-guarantees (honest residual risk)

| Topic | Status |
| ----- | ------ |
| **Preemption** | None. Hostile infinite loop at EL0 or EL1 is DoS. |
| **Wait-on-IRQ** | Not implemented; an agent that wants an interrupt polls or yields cooperatively. Blocking recv *is* implemented (ADR-0022). |
| **A parked agent is parked forever** | `SYS_RECV` has **no timeout**, and nothing reclaims a task that waits on an endpoint whose send capability nobody holds. It keeps its task slot, its address space and its frames until reset, and no counter reports it — `sched::MAX_TASKS` is 16, so sixteen such agents and the machine creates no more tasks. This is availability surface **introduced by ADR-0022**, named here rather than discovered: the ADR rejected a timeout as a decision needing its own deadline queue and its own second reason to leave `Blocked`. Not reachable by a hostile agent *alone* — parking requires a `RECV` capability the creator granted — but reachable by a buggy one. |
| **Console TX depends on the server task** | After M8, agent console output is drained by an EL1 server. If that task exits or never runs, agents get `Full` / silent loss; kernel `kprintln` and panic steal still work. |
| **Capability transfer / revocation** | Not implemented. |
| **Endpoint release / generation recycle** | No kernel path releases an endpoint, so no kernel path mints a stale handle. The *check* is no longer unexercised: `tests/model_ipc.rs` offers a stale `CapId` — same index, previous generation — at every step of every sequence, and removing the generation comparison from `lookup` is caught in two operations. What stays untested is release itself, which does not exist. |
| **IRQ notification capabilities** | Cookie shape exists; cookie unread; no cap_irq. |

| **Creator lifecycle** | Bootstrap outlives agents; reaping undefined. |
| **Heap wild free** | Double-free refused; adversarial pointer that looks like a header can still corrupt. |
| **DTB** | Mapped RO; board truth is compiled-in (ADR-0011). |
| **Firmware / GIC Group 0** | Inherited from pinned `start4.elf` (ADR-0004). |
| **SMP / ASID / TTBR1** | Non-goals until their ADRs. |
| **Threat coverage of `debug-display`** | Lab path; not part of the agent TCB story. |

---

## Secure development expectations

- **ADRs** before moving an authority or isolation boundary ([ADR-0001](docs/adr/0001-multi-role-analysis.md)).
- **Reversal gates** named in each ADR; several have been seen red
  ([`docs/verification.md`](docs/verification.md) “Checks that have been seen to fail”).
- **Mutation testing** on authority modules (`make mutants`) before milestones
  that move the boundary.
- **Bounded exhaustive model checking** of `kernel_core::{tasks, ipc}` against a
  reference implementation, inside `make check`. Bounded is not proved: the
  bound is stated in [`docs/verification.md`](docs/verification.md), and
  extending the result to the kernel's own table sizes is an argument written
  down there, not a theorem.
- **Silicon** for memory attributes, GIC firmware state, RX handover — QEMU is
  blind there.

---

## Versioning this document

Update when:

- the syscall or cap ABI changes (especially M8 console endpoint),
- a new isolation regime lands (TTBR1, ASID, preemption),
- or a residual above is closed with evidence.

The architecture roadmap marks this deliverable done when this file exists and
names the M7 authority boundary; keeping it true is ongoing.

Related: [ADR-0017](docs/adr/0017-el0-capability-abi.md),
[ADR-0018](docs/adr/0018-agent-fault-policy.md),
[ADR-0014](docs/adr/0014-ttbr-split-m5.md),
[`docs/architecture.md`](docs/architecture.md),
[`docs/verification.md`](docs/verification.md).
