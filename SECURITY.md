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
| **Product** | Single-core AArch64 agent-based kernel on Raspberry Pi 4 Model B (foundation done; K/P completeness in progress) |
| **Goal** | Complete agent-based microkernel **and** product OS ([ADR-0026](docs/adr/0026-kernel-and-product-completeness.md)) |
| **Today** | Foundation on Pi 4B; H1 first slices on QEMU (store, wait-on-IRQ EL1+EL0, last-SEND-hold auto-reap, channel revoke, multi-agent product, compose tools). See [roadmap](docs/roadmap.md) |

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
| Busy-loop at EL0 and starve the system | **Mitigated (QEMU first slice)** — cooperative CPU budget / tick quantum ([ADR-0046](docs/adr/0046-k4-cooperative-cpu-budget.md)); model remains cooperative ([ADR-0006](docs/adr/0006-cooperative-execution-model.md)). Residual: **IRQ-side preemption** designed ([ADR-0051](docs/adr/0051-k4-irq-preemption-design.md)); code not landed |
| Park forever on a mailbox nobody sends to | **Mitigated** — supervisor `cancel_blocked` ([ADR-0025](docs/adr/0025-cancel-blocked-wait.md)); last-SEND-hold auto-reap on ephemeral channels ([ADR-0031](docs/adr/0031-k2-last-send-hold-auto-reap.md), QEMU); tick park timeout ([ADR-0040](docs/adr/0040-k2-park-timeout.md), QEMU). Residual: EL0 `SYS_RECV` timeout. |
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
| 6 | `SYS_WAIT_IRQ` | IRQ notification in slot (`CapRights::IRQ` object, high-half `CapId` — [ADR-0030](docs/adr/0030-el0-irq-capability.md)); **waits** for cookie signal |
| 7 | `SYS_RESOLVE` | Empty slot + short name; requires per-task resolve grant ([ADR-0052](docs/adr/0052-p5-resolve-grant.md)); install resolved `CapId` ([ADR-0039](docs/adr/0039-p5-el0-resolve.md)); missing/bad/no-grant → `Authority` |
| 8 | `SYS_TRANSFER` | Move held cap: `x0` from, `x1` to empty slot, `x2` = 0 self / 1 creator / 2 peer; `x3` = task-cap slot when peer ([ADR-0041](docs/adr/0041-el0-cap-transfer.md) / [ADR-0054](docs/adr/0054-k3-peer-transfer-first-slice.md)) |
| 9 | `SYS_RECV_TIMEOUT` | Blocking recv with tick timeout in `x1` ([ADR-0042](docs/adr/0042-el0-recv-timeout.md)); timeout → `Cancelled` |

Reply statuses include `Cancelled` (5) when a parked `SYS_RECV` is aborted by
supervisor reaping ([ADR-0025](docs/adr/0025-cancel-blocked-wait.md)).

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
including idle **18** (`sched::MAX_TASKS` — it bounds how many agents can be
parked at once, see the residual risk below); executable pages an agent may
declare **16** (`mm::MAX_TEXT_PAGES`, 64 KiB).

---

## Isolation mechanisms (and what they do not claim)

| Mechanism | Holds today | Limit |
| --------- | ----------- | ----- |
| W^X kernel map + guard pages | Yes (fault-probed HW) | Protects kernel **from itself** more than from a mapped peer |
| Per-agent `TTBR0` + user VA window | Yes (M5 HW) | Kernel maps **cloned** into user root with EL0-denied AP ([ADR-0014](docs/adr/0014-ttbr-split-m5.md) option C) — not TTBR1 high-half |
| Page-sized device maps for agents | Yes (M6, PL011) | Kernel EL1 keeps coarse `DEVICE_REGIONS` (16 MiB peripherals + GIC) — **risk-accepted** 2026-08-07; agents never receive those blankets ([ADR-0013](docs/adr/0013-narrow-device-windows.md)) |
| Slot-indexed caps | Yes (M7 HW) | Creator channel revoke (ADR-0032, QEMU); **transfer** between agents still open |
| Fault → end session, creator decides | Yes (ADR-0018, M7 HW) | Creator exit leaves agents unsupervised; no restart policy |
| Published `CURRENT_EL0` (`AtomicPtr`, ADR-0019) | Yes (HW 2026-08-07) | Stale publish panics on entry; residual: assembly assumes symbol is a pointer |
| Grants bounded by the loader's own table ([ADR-0021](docs/adr/0021-agents-as-data-and-the-manifest.md)) | Yes (HW 2026-08-07) | Manifest is **in the image** — as trusted as the code it replaced. Channel **revoke** exists for creator/EL1 ([ADR-0032](docs/adr/0032-k3-channel-revoke.md)); **no** EL0 transfer/delegation yet |
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
| **Preemption** | **Cooperative budget done (HW)** ([ADR-0046](docs/adr/0046-k4-cooperative-cpu-budget.md)). IRQ-side preemption **in design** ([ADR-0051](docs/adr/0051-k4-irq-preemption-design.md)); code deferred. |
| **Wait-on-IRQ** | **Done (QEMU):** EL1 `wait_for_irq` ([ADR-0028](docs/adr/0028-wait-on-irq.md)); EL0 `SYS_WAIT_IRQ` via IRQ notification cap ([ADR-0030](docs/adr/0030-el0-irq-capability.md)). Residual: no multi-waiter, no dynamic register, no cancel of IRQ parks. |
| **A parked task may wait until cancelled** | **Reaping (ADR-0025, done HW):** supervisor `ipc::cancel_blocked`. **Last-SEND-hold auto-reap (ADR-0031, done QEMU):** ephemeral channels. **Park timeout (ADR-0040, done QEMU):** `recv_with_timeout` cancels on tick deadline. **Visibility (ADR-0024).** Residual: no EL0 recv timeout yet; frames free only when the task exits and destroys its AS. |
| **Console TX depends on the server task** | After M8, agent console output is drained by an EL1 server. If that task exits or never runs, agents get `Full` / silent loss; kernel `kprintln` and panic steal still work. |
| **Capability transfer / revocation** | **Revoke (ADR-0032, done QEMU):** `creator_revoke` / `revoke_held` kill both channel ends; stale CapId refused on product path (`ipc: release stale refused`). **Transfer** between TCB slots still open (K3 residual). |
| **Endpoint release / generation recycle** | **Done (QEMU first slice):** real `Table::revoke_channel` frees endpoints for reuse; host tests + boot oracle. Model still offers synthetic stale handles at every step. |
| **IRQ notification capabilities** | **Done (QEMU first slice):** `kernel_core::irqcap` + bootstrap mint of timer cookie; EL0 `SYS_WAIT_IRQ` (ADR-0030). Residual: no transfer/revoke of IRQ caps; no manifest grant of IRQ caps yet. |
| **Creator lifecycle** | **Reap/restart first slice (ADR-0033, QEMU):** `supervisor_reap_blocked` + re-spawn after Empty. Residual: creator-exit cascade; force-kill Running EL0 without cooperation; remote AS destroy. |
| **Heap wild free** | Double-free refused; adversarial pointer that looks like a header can still corrupt. |
| **DTB** | Mapped RO; board truth is compiled-in (ADR-0011). |
| **Firmware / GIC Group 0** | Inherited from pinned `start4.elf` (ADR-0004). |
| **SMP / ASID / TTBR1** | **K7** first slice done (QEMU): ASID pool + CONTEXTIDR + nG user leaves (ADR-0050). Residual: TTBR1, HW TLB stamp. **K8** SMP still design-only; cooperative single-core. |
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
