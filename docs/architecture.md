# Architecture

This is the normative description of Harbor’s current architecture and
roadmap. For the documentation map, ownership rules and status vocabulary see
[`docs/README.md`](README.md); for evidence rather than design intent see
[`verification.md`](verification.md). Product shape and use cases:
[`vision.md`](vision.md). Completeness policy:
[ADR-0026](adr/0026-kernel-and-product-completeness.md).

## Purpose

Harbor aims at a **complete** agent-based microkernel and product OS on the
Raspberry Pi 4 Model B: agents, messages, capability grants, and services on
that model — not Linux/POSIX parity
([ADR-0026](adr/0026-kernel-and-product-completeness.md),
[`vision.md`](vision.md)). Public orientation: [`../README.md`](../README.md).

**Today:** foundation **done on Pi 4B** (cooperative tasks through console
endpoint, manifest loader, blocking recv, parked-wait cancel). The kernel and
product OS are **not yet complete**; open work is the
[completeness roadmap](#completeness-roadmap). Historical milestone narrative:
[Roadmap](#roadmap).

## How Harbor differs from a traditional kernel

A traditional OS treats a **process** (or thread) as both the unit of
isolation and the unit of scheduling: user code runs, traps into the kernel,
and resumes the same schedulable context. Harbor is shaped by a different
question — make isolation, authority, messaging and verification *visible
enough to test on silicon* — so several familiar assumptions do not hold.

```
Traditional:                         Harbor:
  scheduler ──► process / thread       scheduler ──► driver task (EL1)
                    │                                  │ session loop
                    └─ trap / svc                      ├─ enter / resume EL0 program
                       user code                       │    (private AS, slot caps)
                                                       └─ park on recv = park the driver
```

| Concern | Traditional kernel | Harbor |
| --- | --- | --- |
| Schedulable unit | Process or thread | **Driver task** only; the EL0 program is not what `sched` switches ([ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)) |
| Isolation unit | Same process (AS + credentials) | **Agent = pair**: EL1 driver + EL0 program with its own address space |
| Preemption | Timer quantum; IRQs may switch | **Cooperative** only; IRQ handlers never context-switch ([ADR-0006](adr/0006-cooperative-execution-model.md)) |
| Authority names | Forgeable-looking handles / FDs checked in the kernel | EL0 names only a **slot index** into its own table; raw `CapId` never leaves the kernel ([ADR-0017](adr/0017-el0-capability-abi.md)) |
| How work is created | Dynamic spawn / load paths | Agents described as **manifest data**; the loader binds grants it already holds ([ADR-0021](adr/0021-agents-as-data-and-the-manifest.md)) |
| Device drivers | In-kernel, or servers with broad maps | **Driver-as-agent** with page-sized named MMIO windows ([ADR-0013](adr/0013-narrow-device-windows.md)) |
| User fault | Kernel reaps or signals the process | Kernel **ends the session**; the **creator** decides the driver task’s fate ([ADR-0018](adr/0018-agent-fault-policy.md)) |
| “Done” for a boundary | Feature lands and tests pass in CI | **M** milestones add capability; **P** milestones add protection/evidence; hardware claims need Pi 4B stamps ([verification.md](verification.md)) |

Three consequences that look unrelated are the same shape fact:

1. **Cost.** Every agent spends one of a small number of task slots and a 16 KiB
   kernel stack on the driver loop, even when the EL0 text is tiny.
2. **Kill and lifetime.** You cannot kill “the agent” without ending the driver
   that was watching it; the session lives in that task’s TCB.
3. **Preemption is not a small scheduler change.** It would mean preempting a
   driver mid-session with a live EL0 context — a different problem from
   preempting a plain kernel task.

None of this claims superiority over production kernels. **POSIX remains out of
model.** Preemption, SMP, and fairness under a hostile busy-loop are **open
completeness tracks** (not permanent refusals) — residuals today are honest in
[`SECURITY.md`](../SECURITY.md). The payoff of the shape is that each boundary
is named, gated, and demonstrable rather than implied by a large ABI.

## Layering

```
┌──────────────────────────────────────────────────────────┐
│  Agents (M5–M7 + loader + waiting recv — all done HW)    │
│  message passing · capability-mediated resources         │
└────────────────────────────▲─────────────────────────────┘
                             │ SVC / IPC / EL0 IRQ (session)
┌────────────────────────────┴─────────────────────────────┐
│  Kernel policy                                           │
│  bootstrap · loader · console_loop · sched · ipc · time  │
│  console · agent · mm (frames, aspace) · status          │
└───────────▲─────────────────────────────▲────────────────┘
            │ register / handle           │
┌───────────┴───────────┐     ┌───────────┴────────────────┐
│  irq                  │     │  drivers                   │
│  dispatch · IrqChip   │     │  gicv2 · pl011 · rng200    │
│  fn(IrqCookie)        │     │  spi · ili9486 (feature)   │
└───────────▲───────────┘     └───────────▲────────────────┘
            │ claim/eoi                   │
┌───────────┴───────────┐     ┌───────────┴────────────────┐
│  arch/exception       │     │  arch/{timer,mmu,switch,   │
│  VBAR · frame · el0   │     │         probe, el0}        │
└───────────────────────┘     └────────────────────────────┘
            ▲                              ▲
            │         bsp/rpi4             │
            └─ memmap · IRQ · gpio · display (feature) ─┘
```

### Rules

1. Drivers never know the board (bases / IRQ ids from BSP).
2. BSP never implements protocols (bind only).
3. Arch never names board peripherals (Generic Timer + CPU only).
4. `exception` does not import drivers, BSP, or time — only `irq::handle_cpu_irq`.
5. One irqchip owner via `irq::init(&'static dyn IrqChip)`.
6. IRQ handlers do not **transmit** on the console (`println` / TX). When the
   kernel owns RX, the UART RX handler may only drain the FIFO into the kernel
   ring. When an agent owns RX, PL011 RX IRQs are **masked** and the agent
   polls `DR` (no IRQ-side drain race).
7. State shared between the IRQ path and the main loop uses `core::sync::atomic`
   — never `static mut`. State that is _not_ producer/consumer uses `SyncCell`
   and is mutated inside `cpu::without_irqs`.

   **Exception, and it is a hard one: no atomic read-modify-write before
   the MMU is enabled.** `swap`, `fetch_add` and `compare_exchange` compile to an
   `LDXR`/`STXR` pair, and with the MMU off every access is Device-nGnRnE,
   where exclusives do not make progress on Cortex-A72 — the retry loop spins
   forever. The board goes silent with no fault to show for it, and QEMU does
   not reproduce it, because its exclusive monitor ignores memory attributes.
   Plain atomic loads and stores (`LDAR`/`STLR`) are fine anywhere. Code that
   runs before the MMU is on is confined to `_start` and `early_mmu_enable`,
   and `scripts/check-pre-mmu-path.sh` fails the build if that changes. Because
   the window is now that small, `console::acquire` and the panic handler use
   ordinary atomics like everything else.

   **Second sub-rule, and it is about scope rather than about type: a `DAIF`
   save/restore pair must not span a call that can switch tasks.**
   `cpu::without_irqs` reads `DAIF` before its closure and writes it back after,
   so a switch in between hands the next task this task's mask and later
   restores a value captured in an epoch that has ended. The EL0 session loop
   used to hold one mask for the whole session; it holds one per enter/resume
   step now, because `SYS_RECV` parks (ADR-0022). `scripts/check-irq-scope.sh`
   walks each region by brace depth and fails on a switching call inside it.

8. Idle (console loop) uses `WFI` when the RX ring is empty, no tick report is
   due, and no task is ready; it **yields** when the runqueue is non-empty. The
   emptiness check runs with IRQs masked so a wakeup cannot be lost.
9. Nothing is both writable and executable, and diagnostic scaffolding lives
   behind a feature rather than in the production surface: `bringup` for the
   masked-IRQ gates, **`oracle`** for the demo tasks and agents every boot-check
   assertion reads.

   `oracle` is **on by default**, unlike the other two, because `make
boot-check` _is_ the oracle and a gate that needs a flag is a gate someone
   forgets. What the feature buys is that an image without it exists, is
   compiled, and is checked: `make product-builds` refuses one that still
   carries the demo strings.

   That build also reports a number worth reading: **36 items are unreachable
   without the oracle**, down from 95. `bootstrap::loader` is product code and
   calls `sched::spawn_with_slots`, `AddressSpace`, `Agent` and the EL0 session, so
   they finally have a product-path caller (ADR-0021).

   M8 gives the product a **beacon** agent and an always-on EL1 console server.
   The product image creates tasks; `make product-builds` reports size and
   unreachable counts. Oracle adds **mute** only (same image, no grant).

   `make product-builds` prints the current unreachable count, and this
   paragraph is the only place that used to hard-code it: no gate compares the
   two, so a stale figure here is drift a reader has to catch.

10. **Facade isolation (ADR-0015).** Outside `src/arch/`, import only
    `crate::arch::{…}` — never `crate::arch::<isa>`. Outside a board package,
    import only `crate::bsp::board` — never `crate::bsp::<board>`. ISA
    selection is `target_arch`; board selection is a `board-*` Cargo feature.
    Boot entry and the linker script live under the active ISA tree
    (`src/arch/aarch64/`). The product supports AArch64 + Pi 4 only; the
    structure is multi-arch _ready_, not multi-arch product. Contract:
    [`arch-contract.md`](arch-contract.md); port checklist: [`porting.md`](porting.md).

Rules 1–4 and 10 are checked by `make layering` (`scripts/check-layering.sh`)
against every `crate::` import edge (and ISA/board path leaks). Coupling that
is not an import (a shared constant, an agreed register value) is still
review-only — see [`verification.md`](verification.md).

**Agent shell imports** (policy, not a lower layer): `arch`, `mm`, `sched`, plus
`ipc` (slot send/recv; console is a send cap), `irq` (lower-EL IRQ → `handle_cpu_irq` then resume)
and `ipc` (`SYS_SEND`/`SYS_RECV` by slot — the translation lives in `ipc`
because the authority counter is `ipc`'s to maintain). No `drivers` / `bsp`
from `agent` — board PA/VA for demos live in bootstrap.

## Interrupt / timer / console contract

| Role        | Module                          | Responsibility                                         |
| ----------- | ------------------------------- | ------------------------------------------------------ |
| Clocksource | `arch/timer`                    | CNTP deadline, ISTATUS, re-arm                         |
| Irqchip     | `drivers/gicv2`                 | enable, claim/EOI, SPI target CPU0                     |
| Dispatch    | `irq` + `kernel_core::irqtable` | id → handler; seal freezes the table                   |
| Tick policy | `time`                          | `on_timer_irq`, `ticks()`                              |
| Console RX  | `console`                       | ring / `suspend_rx`·`resume_rx`; agent poll when owned |
| Bind        | `bsp/rpi4/irq`                  | TIMER=30, UART=153, static GIC                         |
| Layout      | `mm/layout`                     | regions and their permissions                          |
| Allocation  | `mm`                            | free list + `GlobalAlloc`                              |
| Scheduler   | `sched`                         | cooperative spawn / yield / exit                       |
| Task stacks | `mm/task_stack`                 | heap stack + unmapped guard                            |

## Agent model

**Tasks** (M3), **messages/caps** (M4), **private AS + EL0** (M5), and a
**PL011 driver agent** (M6) are in tree. Cooperative only ([ADR-0006](adr/0006-cooperative-execution-model.md)).

| Concept    | Role                                                                                    | Status                           |
| ---------- | --------------------------------------------------------------------------------------- | -------------------------------- |
| Task (M3)  | Schedulable EL1 entity + private stack                                                  | **done (HW)**                    |
| Agent      | A **pair**: an EL1 driver task and the EL0 program it drives ([ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)). Multi-SVC, IRQ resume, console via `SYS_SEND`, PL011 RX own, and a recv it can wait on | **done (HW)** |
| Message    | Sole interaction channel (M4)                                                           | **done (HW)**                    |
| Capability | Unforgeable handle (send/recv; future: IRQ notification)                                | **done (HW)** (IRQ caps later)   |
| Manifest   | The table that says which agents exist and what each is granted ([ADR-0021](adr/0021-agents-as-data-and-the-manifest.md)) | **done (HW)** |

`irq::register` is the hook for later capability mediation.

### An agent is a pair, and the driver is the schedulable half

`Agent::run_user_prog_resuming` is a **synchronous loop owned by an EL1 task**:
it enters EL0, handles each SVC, resumes, and returns when the session ends. So
what `sched` admits and switches to is the *driver*, not the EL0 context —
and an agent costs one of `MAX_TASKS` slots plus a 16 KiB kernel stack on top of
its address space, however small its program.

[ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) records the
shape rather than changing it, because three things that look unrelated are the
same fact: preemption would have to preempt the driver mid-session with a live
EL0 context; ADR-0018's *"the creator decides what happens to the task"* means
the driver task, so an agent cannot be killed without killing its watcher; and
`MAX_TASKS` is scarce because every agent spends a slot on the loop that drives
it.

Where the distinction matters, this document says **driver task** or **EL0
program** rather than "agent".

### An agent is data

Since ADR-0021 an agent can be a **manifest entry** rather than a compiled-in
Rust `fn`: an image, a window geometry, and a slot table. `bootstrap::loader` is
one loop over `kernel_core::manifest`, and every entry gets the same
trampoline — one body, N descriptions. Which entry a task is running is the
loader's own side table, not a field in the TCB: the scheduler sits below
`agent` and `bootstrap`, and a manifest is a concept it has no business
knowing ([ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)).

The security argument is arithmetic, not a check. An entry's slot carries an
**index into the loader's own capability list**, never a `CapId`, so there is
nothing outside that list for a manifest to name. `manifest::bind` is where the
index becomes a capability, and an index past the end is a refusal that says
which one it reached for.

What is **not** a manifest entry: a body that runs several programs in sequence
and checks counters between them. That is an oracle, and `el0_scheduled_task`
and `el0_ipc_sender` stay hand-written for that reason.

## Milestones

| ID  | Deliverable                                              | Status                                                              |
| --- | -------------------------------------------------------- | ------------------------------------------------------------------- |
| M0  | Hello UART + echo                                        | **done**                                                            |
| M1  | Exceptions + timer IRQ ticks                             | **done** (HW)                                                       |
| M2  | MMU + kernel heap (+ atomics after attrs)                | **done** (HW)                                                       |
| P0  | Idle (WFI) + UART RX IRQ + ring                          | **done** (HW)                                                       |
| P1  | W^X + guard page + free-list `GlobalAlloc`               | **done** (HW, fault-probed)                                         |
| P2  | Early MMU, softfloat, build-enforced gates               | **done** (HW)                                                       |
| P3  | Layout validation, runtime `map` + TLB maintenance, ADRs | **done** (HW)                                                       |
| P4  | Exception stack, refused frees, fatal map failure        | **done** (HW, fault-probed)                                         |
| M3  | Cooperative tasks                                        | **done** (HW, fault-probed)                                         |
| M4  | IPC + capabilities                                       | **done (HW)**                                                       |
| M5  | EL0 agents                                               | **done (HW)**                                                       |
| M6  | Driver-as-agent                                          | **done (HW)** page map + FR + kill; **RX own done (HW)** 2026-08-06 |

**M** milestones add capability. **P** milestones add protection or evidence and
add no capability at all: they are numbered separately because "the kernel can
now do X" and "the kernel can now be trusted about X" are different claims, and
mixing them lets the second silently stand in for the first. A P milestone is
work that would be invisible in a demo.

"done (HW)" means the deliverable was observed working on a Raspberry Pi 4B, not
merely in QEMU. The distinction earned its place: emulation booted a kernel that
hung on silicon, because TCG's exclusive monitor ignores memory attributes. See
[`verification.md`](verification.md). P4 met it in three parts: the board boots
with the split stacks and takes timer IRQs — which can only arrive through the
EL1t vector entries — and both fault probes were re-run at their new addresses.

### What each planned milestone needs, and how it is judged done

The done column above was earned against a stated observable. The same standard
applies forwards, or it is not the same standard.

| ID  | Needs first                                                                                                                                                                                                                                                | Done when                                                                                                                                                                                                                                                                            |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| M3  | [ADR-0006](adr/0006-cooperative-execution-model.md) (F12 done); per-task heap stack + unmapped guard                                                                                                                                                       | Two tasks yield to each other on hardware and the console shows their output interleaved; each task stack is validated by `mm::layout`; a probe shows one task's overflow faulting rather than reaching another's stack                                                              |
| M4  | [ADR-0008](adr/0008-irq-handler-policy.md) (**accepted**): cookie handlers + wake queue; mailbox ABI                                                                                                                                                       | A message crosses between two tasks that share no memory; a send on a capability the sender does not hold is refused and counted, and the refusal is visible on the console; IRQ wakes use the ADR-0008 queue only                                                                   |
| M5  | [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md) + [ADR-0014](adr/0014-ttbr-split-m5.md) (TTBR0 v1); multi-role prep                                                                                                                             | A task runs at EL0 in its own `TTBR0`; an EL0 write to a kernel address takes a permission fault with the ESR recorded here, the way W^X was; `SVC` returns to EL1 and back                                                                                                          |
| M6  | M5 done; [ADR-0013](adr/0013-narrow-device-windows.md) (**accepted**); F26                                                                                                                                                                                 | EL0 agent maps **only** the PL011 page, touches the device, is destroyed (kill); kernel console/ticks continue. RX ownership (poll + real bytes) was a post-v1 product slice, closed on silicon 2026-08-06 with [issue #1](https://github.com/gianlucamazza/harbor-kernel/issues/1). |
| M7  | [ADR-0017](adr/0017-el0-capability-abi.md) (EL0 capability ABI) and [ADR-0018](adr/0018-agent-fault-policy.md) (agent fault policy), both **accepted** 2026-08-06 — which is what unblocks the milestone under [ADR-0001](adr/0001-multi-role-analysis.md) | Two EL0 agents exchange a message neither can forge; one of them faults; its creator handles the fault and the other keeps running; the kernel stays alive — **on silicon**, with a serial transcript. **Done (HW) 2026-08-07** in one boot across all four slices                   |

M3 is **done (HW)**. [ADR-0006](adr/0006-cooperative-execution-model.md) is
**accepted**. Observed on **Pi 4B silicon**: interleaved `task-a`/`task-b`,
unmap smoke, and a scheduled task-stack overflow that took a **translation
fault** in its own guard with peers live
([verification.md](verification.md#m3-cooperative-tasks-hardware)). QEMU remains
gated by `boot-check`. Desk multi-role pass:
[reviews/2026-08-04-m3-incremental.md](reviews/2026-08-04-m3-incremental.md).
Inventing preemption or `link.ld` task stacks is a reversal of the ADR.

M4 is **done (HW)**. [ADR-0008](adr/0008-irq-handler-policy.md) is **accepted**.
QEMU `boot-check` and Pi 4B boot (2026-08-05) show message cross + refuse
count ([verification.md](verification.md#m4-ipc--capabilities)).

M5 is **done (HW)**. [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md)
and [ADR-0014](adr/0014-ttbr-split-m5.md) are **accepted**. S0–S4: named frame
pool, `AddressSpace` prepare (kernel clone + user window), one-shot
`arch::el0::run` (`switch_ttbr0` sole path), SVC + EL0 store-to-kernel fault
probes, destroy without pool leak. QEMU `boot-check` and Pi 4B PL011 (2026-08-05)
show the same oracles
([verification.md](verification.md#m5-el0--address-spaces)).

<a id="roadmap"></a>

## Roadmap

### Closed (HW) — through multi-SVC / M6 v1 map

| Slice                                                   | Status        | Evidence                                                   |
| ------------------------------------------------------- | ------------- | ---------------------------------------------------------- |
| **M5-P1…P3**                                            | **done (HW)** | scheduled EL0, SVC refuse, dual AS                         |
| **M6-D0** [ADR-0013](adr/0013-narrow-device-windows.md) | **accepted**  | 2026-08-05                                                 |
| **M6 v1** PL011 page + FR + kill                        | **done (HW)** | `pl011-agent: FR read + svc ok` / `killed ok`              |
| **Agent shell** + concurrent dual agent                 | **done (HW)** | `agents: concurrent ok`                                    |
| **SVC resume**                                          | **done (HW)** | `enter`/`resume`/`end_session`; `el0-task: resume pings=2` |
| Preferred ELR for SVC                                   | documented    | AArch64 ELR already past SVC — no software `+4`            |

Pi 4B stamp detail: [verification.md §M5-P / M6](verification.md#m5-p--m6-post).

### Closed (HW) — M7, stamped on silicon 2026-08-07 00:05

One boot carrying all four slices. Transcript and what the ordering proves:
[verification.md §M7 closed on silicon](verification.md#hardware-evidence-m7-closed-on-silicon-2026-08-07).

| Slice                                      | Status        | Evidence                                                                                                                                                                                                                                           |
| ------------------------------------------ | ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **1 — EL0 session state in the `Tcb`**     | **done (HW)** | nine `static mut` → one published `CURRENT_EL0`; no panic across five agents in four tasks, and deleting the publish from the switch panics on the first spawned-task entry                                                                        |
| **2 — `SYS_SEND` / `SYS_RECV` by slot**    | **done (HW)** | `el0-ipc: sent slot=0 tag=7 a=42` → `*el0-ipc: got payload via EL0 recvs=1`; the receiving agent moves the payload into `SYS_PUTC` itself, so the `*` is the message and not a status code                                                         |
| **2 — authority refused on the good path** | **done (HW)** | `el0-ipc: refused slot=1 authority=2`; a full mailbox counts as `full`, never as authority                                                                                                                                                         |
| **3 — `SYS_PUTC` behind a capability**     | **done (HW)** | `console: capability minted`, then `el0-ipc: console denied, printed nothing` — and the byte that agent tried to print is asserted **absent** from the log                                                                                         |
| **4 — fault policy** (ADR-0018)            | **done (HW)** | `agent faulted esr=0x9200004f far=0x80000 faults=1` then `creator alive after fault`, with the peer completing 22 ms later; `SessionEnd` is `#[must_use]` and has been seen to fail a build                                                        |
| **The done-when, end to end**              | **done (HW)** | two EL0 agents with different capability tables exchange a message neither can forge, one faults, its creator handles it, the other completes, the kernel keeps ticking                                                                            |
| Blocking `SYS_RECV`                        | **done (HW)**   | The agent parks and a peer send wakes it; the oracle spawns the receiver **first** and it still gets the payload, so ordering by construction is gone. `SYS_TRY_RECV` keeps the non-blocking path and is the only producer of `Status::Empty` left ([ADR-0022](adr/0022-blocking-recv-and-the-mask-that-travels.md)) |

Cost: `pool=496` at the concurrent peak and `pool=512` after the kill, identical
to the pre-M7 sessions. Four slices, no frames.

### Closed (HW) — issue #1, stamped on silicon 2026-08-06

Every row below was QEMU-only until the hardware session of 2026-08-06.
Transcript and the four register-level claims:
[verification.md §the four changes of 2026-08-05](verification.md#hardware-evidence-the-four-changes-of-2026-08-05-closed).

| Slice                          | Status        | Evidence                                                                                                                                                |
| ------------------------------ | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **EL0 IRQ save/resume**        | **done (HW)** | architectural re-execute; `el0-task: irq resume irqs=1`                                                                                                 |
| **`SYS_PUTC`**                 | **done (HW)** | imm 2; `el0-task: putc bytes=2`                                                                                                                         |
| **RX poll empty**              | **done (HW)** | `pl011-agent: rx poll empty`                                                                                                                            |
| **RX-owned agent (poll)**      | **done (HW)** | drain off + IMSC mask; `rx own begin/end`, and an injected byte reached the _agent_ while the kernel drain was suspended (`rx poll unexpected putcs=1`) |
| **Real RX bytes**              | **done (HW)** | PL011 **LBE** inject; `rx own bytes=2`, intact underneath ~3500 injected bytes                                                                          |
| **Kill restores kernel drain** | **done (HW)** | `resume_rx` + `killed ok`; idle ticks ran to 270 with no storm                                                                                          |
| Kernel TX / panic              | preserved     | TX never handed to agent                                                                                                                                |

QEMU gate: `make boot-check` / `scripts/qemu-boot-check.sh` (all of the above
oracles). It has three outcomes, not two: `timer: MISSED` is corroborated
against the host CPU the emulator received, and reports **INDETERMINATE**
(exit 3) rather than a red it cannot attribute.

### Closed — ADR-0019 (rule 7 absolute)

| Slice                           | Status        | Evidence                                                                                                                                                                                                                                                                                                                                                       |
| ------------------------------- | ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`CURRENT_EL0` → `AtomicPtr`** | **done (HW)** | same `adrp`/`add`/`ldr` symbol; `Release` publish / `Acquire` load. Stamped on silicon 2026-08-07 10:24 — the plain `ldr` in `vectors.s` sees the published pointer on every EL0 exception, with zero panics and zero `no published session` ([verification](verification.md#hardware-evidence-main-after-adr-0019--the-atomic-on-the-vector-path-2026-08-07)) |
| **`make no-static-mut`**        | **done**      | greps `src/` for declarations; prerequisite of `make check`                                                                                                                                                                                                                                                                                                    |
| Rule 7 exception                | **gone**      | no `static mut` remains; ADR-0016/0017 keep their false premise text (immutable) and point here                                                                                                                                                                                                                                                                |

### Closed — threat model ([`SECURITY.md`](../SECURITY.md))

| Slice                        | Status   | Evidence                                                                                                           |
| ---------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------ |
| **Threat model + reporting** | **done** | Root [`SECURITY.md`](../SECURITY.md): TCB, attacker, authority surface, claims with gates, residual non-guarantees |
| Bound to M7 authority        | **yes**  | Slot ABI, console denied-by-default, fault policy, refusal counters — as of silicon 2026-08-07                     |

### Closed — M8 console endpoint (HW) 2026-08-07

| Slice | Status | Evidence |
| --- | --- | --- |
| EL1 console server drains the endpoint | **done (HW)** | [`verification.md` §M8](verification.md#hardware-evidence-m8-console-endpoint-closed-on-silicon-2026-08-07); design: [`design-m8-console-endpoint.md`](design-m8-console-endpoint.md) |
| Product manifest carries the beacon | **done (HW)** | same transcript + product QEMU gate |
| `SYS_PUTC` removed; denied-by-default preserved | **done (HW)** | mute refusals=2; refuse count=5; syscall gate |

### Closed — parked-task policy (ADR-0024 / 0025)

| Slice | Status | Evidence |
| --- | --- | --- |
| Visibility (`blocked_count` / `block_events`) | **done (HW)** | [ADR-0024](adr/0024-parked-task-visibility.md); [verification §](verification.md#parked-task-visibility-and-cancel-closed-on-silicon-adr-0024--0025-2026-08-07) |
| Supervisor `cancel_blocked` → `Cancelled` | **done (HW)** | [ADR-0025](adr/0025-cancel-blocked-wait.md); `ipc: reaped cancelled` on silicon |
| Timeout / auto-reap on last send drop | **open (K2)** | Completeness track; not done by ADR-0025 |

Issue [#13](https://github.com/gianlucamazza/harbor-kernel/issues/13) is **closed**
for visibility + cancel. [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)
(agent = driver + EL0 program) is accepted so preemption/reaping discussions
name **which half** they mean.

### Closed — F26 EL1 Device residual (risk-accept, 2026-08-07)

[ADR-0013](adr/0013-narrow-device-windows.md) already closed F26 for **agent**
maps (page-sized only). Kernel EL1 may keep coarse Device windows until a
P-pass. That P-pass ([#2](https://github.com/gianlucamazza/harbor-kernel/issues/2),
**closed not planned** 2026-08-07) is risk-accepted rather than implemented:

| Layer | Window | Status |
| --- | --- | --- |
| Agent AS | Named page(s) only (`map_device_page`) | **done (HW)** M6 |
| Kernel EL1 | `DEVICE_REGIONS`: 16 MiB peripherals + 16 KiB GIC (`memmap`) | **risk-accepted** — EL1-only TCB; no agent sees the blanket |

Re-open #2 only if a new agent needs a peripheral still covered only by a
blanket, or if an audit shows EL1 stray stores into Device as a live bug class.

<a id="completeness-roadmap"></a>

## Completeness roadmap

Policy: [ADR-0026](adr/0026-kernel-and-product-completeness.md). Foundation
M0–M8 is closed; the project goal is **kernel completeness (K)** and **product
OS completeness (P)**. Order is a working plan — design ADR before any boundary
move ([ADR-0001](adr/0001-multi-role-analysis.md)). Status vocabulary:
`open` | `in design` | `done (QEMU)` | `done (HW)`.

### K — microkernel mechanisms

| ID | Track | Status | Done when (sketch) | Needs first |
| --- | --- | --- | --- | --- |
| K1 | Wait-on-IRQ (first-class) | **done (QEMU)** first slice ([ADR-0028](adr/0028-wait-on-irq.md)); EL0 IRQ cap open | EL1 `wait_for_irq(cookie)`; timer/UART `signal`; oracle `irq-wait: woke` | ADR-0008 → 0028; EL0 syscall successor |
| K2 | Park reclaim (timeout and/or auto-reap on last send drop) | **open** | Orphan parks do not hold `MAX_TASKS` forever without policy | Successor to ADR-0025 |
| K3 | Cap transfer / revoke / endpoint release | **open** | Authority can move and die without reboot; stale generation exercised by real release | ADR-0017 successor |
| K4 | Preemption or CPU budget | **open** | Hostile busy-loop is not permanent DoS residual | Successor to ADR-0006; name agent-pair impact (0023) |
| K5 | Agent density (shrink/collapse driver half) | **open** | Many small agents without 16 KiB kernel stack each by default | Successor to ADR-0023 |
| K6 | External agent load + byte manifest | **done (QEMU)** first slice ([ADR-0027](adr/0027-h1-external-agent-store.md)); Pi place still open | Fixed-PA store at `0x10000000`; product prefers store, oracle builtin fallback | ADR-0021 → 0027; P2 for on-target pack/place |
| K7 | ASID (+ TTBR1 if required) | **open** | Production isolation without cloned-kernel-only story as the end state | Design ADR |
| K8 | SMP | **open** | Multi-core runqueue/IRQ model on silicon | Design ADR |
| K9 | Driver-as-agent beyond PL011 (+ IRQ caps) | **open** | Second peripheral on the M6 pattern; IRQ-cap path | K1 useful; ADR-0013 pattern |
| K10 | Supervisor lifecycle (restart, creator exit) | **open** | Product supervisor can restart/reap without ad-hoc demos | Builds on 0018/0025 |

### P — product operating system

| ID | Track | Status | Done when (sketch) | Typical deps |
| --- | --- | --- | --- | --- |
| P1 | Multi-agent product image beyond beacon | **done (QEMU)** first slice (beacon + chirp in store) | Product `agents.bin` n≥2; both run via console endpoint | ADR-0027 store; richer agents later |
| P2 | Storage path (block + load/persist) | **open** | Persist or load agent/data without rebuild-only workflow | Often after K6 |
| P3 | Network agent + caps | **open** | Network I/O only via granted caps; no ambient net | K1/K9 helpful |
| P4 | Display/input product path | **open** | Product-grade path (may graduate `debug-display` discipline) | Device agents |
| P5 | Naming / discovery / system services | **open** | Endpoints findable without hard-coded oracle wiring | K3 useful |
| P6 | Compose/audit tooling | **open** | Host and/or on-target tools for grant graph and manifests | P1 |

### Standing watches (not completeness tracks)

| Work | Done when | Issue |
| --- | --- | --- |
| **ADR-0020 expiry watch** | XPT2046 lands and `SpiDevice` gets a caller, or the trait goes and ADR-0020 is superseded | [#14](https://github.com/gianlucamazza/harbor-kernel/issues/14) |

### Out of model (permanent non-goals)

These are **not** completeness tracks ([ADR-0026](adr/0026-kernel-and-product-completeness.md)):

- Linux / POSIX / glibc compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
- Multi-tenant cloud hypervisor (unless a future ADR owns it)

### Open findings, against the milestone they block

From [the multi-role review](reviews/2026-08-04-multi-role.md). Findings not
listed here block nothing and are tracked in that report alone.

| Finding | Blocks | Why |
| ------- | ------ | --- |

Status for all thirty lives in the review itself, assigned by the 2026-08-06
audit and verified against the code — this page used to track six of them while
the report tracked none. **None is still open.** The last was F23, board
topology encoded in `arch` through the early map: closed on 2026-08-06 by moving
the map to `src/mm/early.rs`, where the seam between board and CPU has a name
instead of a hiding place, and by `make arch-board-free`, which sees the way of
knowing a board that `make layering` cannot — writing its addresses out by
hand.

| F12 | — (resolved) | Closed by [ADR-0006](adr/0006-cooperative-execution-model.md); the ADR was the deliverable |
| F18 | — (resolved) | Absolute `CNTP_CVAL` deadlines + missed-tick counter; pure cooperative yield never depended on it |
| F13 | — (resolved) | Shape accepted: `Handler = fn(IrqCookie)` + IRQ→voluntary wake queue — [ADR-0008](adr/0008-irq-handler-policy.md); code lands with first M4 PR |
| F26 | — (resolved M6 v1) | [ADR-0013](adr/0013-narrow-device-windows.md) **accepted**; agent maps are page-sized named windows only; kernel coarse Device may remain until a P-pass |
| F15 | — (resolved) | Risk-accepted: board truth is BSP constants; DTB mapped RO for a future parser — [ADR-0011](adr/0011-dtb-mapped-board-constants-risk-accept.md) |
| F24 | — (resolved) | Layering rules 1–4 are enforced by `make layering`; non-import coupling remains review-only (gate blind spots in verification) |
| F23 | — (resolved) | Early map in `mm::early`; board says which gigabyte is what via `memmap::EARLY_BLOCKS`; `make arch-board-free` refuses a physical range base under `src/arch/` |

### Side-track (not an M/P milestone)

Optional lab **SPI TFT status surface** (Waveshare-class 3.5″ / ILI9486) is
specified in [ADR-0009](adr/0009-optional-spi-tft-debug-console.md),
[ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md), and
[`hardware.md`](hardware.md). It is observability, not agent capability: UART
stays primary; the panel is a structured status sink behind a default-off
feature (`debug-display`). **SPI0, regwidth-16 ILI bring-up, and the status
surface are silicon-closed**
([verification](verification.md#rng200-and-spi0-hardware)). Missing
peripherals soft-fail via `arch::probe` (QEMU RNG hole) rather than a feature
gate. Must not block or redefine M4–M6; M6 may later _reuse_ those drivers as
agents.

## Decisions and reviews

The choices that constrain the code have an ADR, each naming the alternative
that was rejected and the gate that would catch its reversal.

| Artefact                                                        | Role                                                                                                |
| --------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| [`../SECURITY.md`](../SECURITY.md)                              | Threat model and reporting (M7 authority surface; residuals named)                                  |
| [`docs/adr/`](adr/README.md)                                    | Architecture Decision Records (lifecycle: proposed → accepted → superseded)                         |
| [ADR-0001](adr/0001-multi-role-analysis.md)                     | Multi-role analysis as pre-milestone gate (**accepted**)                                            |
| [ADR-0002](adr/0002-softfloat-kernel.md)                        | Kernel compiled softfloat, FP left trapping (**accepted**)                                          |
| [ADR-0003](adr/0003-early-mmu.md)                               | MMU enabled before any Rust runs (**accepted**)                                                     |
| [ADR-0004](adr/0004-gic-group0-firmware-pin.md)                 | GIC Group 0 with IAR/EOIR, and the firmware pin (**accepted**)                                      |
| [ADR-0005](adr/0005-static-page-table-arena.md)                 | Static page-table arena instead of a frame allocator (**accepted**)                                 |
| [ADR-0006](adr/0006-cooperative-execution-model.md)             | Cooperative execution model (M3 tasks); closes F12 (**accepted**)                                   |
| [ADR-0007](adr/0007-project-identity-harbor-kernel.md)          | Project identity Harbor / `harbor-kernel` (**accepted**)                                            |
| [ADR-0008](adr/0008-irq-handler-policy.md)                      | IRQ handler shape for M4 wakes / caps; closes F13 (**accepted**)                                    |
| [ADR-0009](adr/0009-optional-spi-tft-debug-console.md)          | Optional SPI TFT status surface; lab side-track (**accepted**)                                      |
| [ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md)           | SPI sessions + DBI stream; regwidth-16 SKU note (**accepted**)                                      |
| [ADR-0011](adr/0011-dtb-mapped-board-constants-risk-accept.md)  | DTB mapped; board truth compiled-in; closes F15 (**accepted**)                                      |
| [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md)      | Frame allocator for user AS; M5 needs-first (**accepted**)                                          |
| [ADR-0013](adr/0013-narrow-device-windows.md)                   | Narrow device MMIO for agents; F26/M6 v1 (**accepted**)                                             |
| [ADR-0014](adr/0014-ttbr-split-m5.md)                           | TTBR regime M5 v1 (TTBR0 + kernel maps in user AS) (**accepted**)                                   |
| [ADR-0015](adr/0015-multi-arch-scaffold.md)                     | Multi-arch scaffold: cfg facade + board features (**accepted**)                                     |
| [ADR-0016](adr/0016-el0-session-protocol.md)                    | EL0 session protocol: one slot, prose contract, named successor (**superseded**) — by 0017 and 0018 |
| [ADR-0017](adr/0017-el0-capability-abi.md)                      | EL0 capability ABI: slot-indexed authority, session state in the TCB (**accepted**)                 |
| [ADR-0018](adr/0018-agent-fault-policy.md)                      | Agent fault policy: the kernel ends the session, the creator decides the task (**accepted**)        |
| [ADR-0019](adr/0019-no-static-mut.md)                           | No `static mut`: the last one becomes an atomic, rule 7 without an exception (**accepted**)         |
| [ADR-0020](adr/0020-spidevice-contract-without-a-caller.md)     | `SpiDevice`: contract kept, ADR-0010's descriptive sentence retracted (**accepted**)                |
| [ADR-0021](adr/0021-agents-as-data-and-the-manifest.md)         | Agents as data described by a manifest; the grant becomes a binding, not code (**accepted**)        |
| [ADR-0022](adr/0022-blocking-recv-and-the-mask-that-travels.md) | Blocking `SYS_RECV`: the agent parks; `without_irqs` stops spanning a switch (**accepted**)         |
| [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) | An agent is a **pair**: an EL1 driver task and the EL0 program it drives; the driver is what the scheduler runs (**accepted**) |
| [ADR-0024](adr/0024-parked-task-visibility.md) | Parked tasks are counted (`blocked_count` / `block_events`); reclaim/timeout deferred (**accepted**) |
| [ADR-0025](adr/0025-cancel-blocked-wait.md) | Supervisor `cancel_blocked` aborts a parked wait (`Cancelled`); no timeout queue (**accepted**) |
| [ADR-0026](adr/0026-kernel-and-product-completeness.md) | Completeness of kernel (K) and product OS (P) is the project goal (**accepted**) |
| [ADR-0027](adr/0027-h1-external-agent-store.md) | H1 entry: external agent store at fixed PA (**accepted**) |
| [ADR-0028](adr/0028-wait-on-irq.md) | K1 entry: EL1 wait on IRQ cookie (**accepted**) |
| [`docs/reviews/`](reviews/)                                     | Pass outcomes (findings), not decisions                                                             |

## Non-goals

Permanent **out of model** only (see [Completeness roadmap](#completeness-roadmap)
for K/P tracks):

- Linux / POSIX compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
- Multi-tenant cloud hypervisor (unless a dedicated ADR owns it)
