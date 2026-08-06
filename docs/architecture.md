# Architecture

## Purpose

**Harbor** (package `harbor-kernel`) **aims to be** an agent-based microkernel for the
Raspberry Pi 4 Model B, where agents are isolated units interacting only
through message passing and capabilities.

It is not a finished agent OS yet. What runs today is a single-core kernel at
EL1 with a protected identity map, interrupts, a heap, **cooperative tasks
(M3)**, **IPC/caps (M4)**, **EL0 address spaces (M5)**, and a **PL011 driver
agent (M6)** — through multi-SVC resume and concurrent agents **done on Pi 4B**.
Post-M6 slices (EL0 IRQ resume, `SYS_PUTC`, RX ownership with real bytes) are
**done on QEMU**; silicon stamp for those is open. See [Roadmap](#roadmap).

## Layering

```
┌──────────────────────────────────────────────────────────┐
│  Agents (M5–M6 + shell done HW; IRQ/PUTC/RX-own QEMU)    │
│  message passing · capability-mediated resources         │
└────────────────────────────▲─────────────────────────────┘
                             │ SVC / IPC / EL0 IRQ (session)
┌────────────────────────────┴─────────────────────────────┐
│  Kernel policy                                           │
│  bootstrap · console_loop · sched · ipc · time · console │
│  agent · mm (frames, aspace) · status (debug-display)  │
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

8. Idle (console loop) uses `WFI` when the RX ring is empty, no tick report is
   due, and no task is ready; it **yields** when the runqueue is non-empty. The
   emptiness check runs with IRQs masked so a wakeup cannot be lost.
9. Nothing is both writable and executable, and diagnostic scaffolding lives
   behind the `bringup` feature rather than in the production surface.
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
`console` (`SYS_PUTC` TX), `irq` (lower-EL IRQ → `handle_cpu_irq` then resume)
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
| Agent      | Task + AS at EL0; multi-SVC (**HW**); IRQ resume + `SYS_PUTC` + PL011 RX own (**QEMU**) | **hybrid** — [Roadmap](#roadmap) |
| Message    | Sole interaction channel (M4)                                                           | **done (HW)**                    |
| Capability | Unforgeable handle (send/recv; future: IRQ notification)                                | **done (HW)** (IRQ caps later)   |

`irq::register` is the hook for later capability mediation.

## Milestones

| ID  | Deliverable                                              | Status                                                 |
| --- | -------------------------------------------------------- | ------------------------------------------------------ |
| M0  | Hello UART + echo                                        | **done**                                               |
| M1  | Exceptions + timer IRQ ticks                             | **done** (HW)                                          |
| M2  | MMU + kernel heap (+ atomics after attrs)                | **done** (HW)                                          |
| P0  | Idle (WFI) + UART RX IRQ + ring                          | **done** (HW)                                          |
| P1  | W^X + guard page + free-list `GlobalAlloc`               | **done** (HW, fault-probed)                            |
| P2  | Early MMU, softfloat, build-enforced gates               | **done** (HW)                                          |
| P3  | Layout validation, runtime `map` + TLB maintenance, ADRs | **done** (HW)                                          |
| P4  | Exception stack, refused frees, fatal map failure        | **done** (HW, fault-probed)                            |
| M3  | Cooperative tasks                                        | **done** (HW, fault-probed)                            |
| M4  | IPC + capabilities                                       | **done (HW)**                                          |
| M5  | EL0 agents                                               | **done (HW)**                                          |
| M6  | Driver-as-agent                                          | **done (HW)** page map+FR+kill; **RX own done (QEMU)** |

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

| ID  | Needs first                                                                                                                                                                                                                                                | Done when                                                                                                                                                                                                                                                                                                                                                                                |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M3  | [ADR-0006](adr/0006-cooperative-execution-model.md) (F12 done); per-task heap stack + unmapped guard                                                                                                                                                       | Two tasks yield to each other on hardware and the console shows their output interleaved; each task stack is validated by `mm::layout`; a probe shows one task's overflow faulting rather than reaching another's stack                                                                                                                                                                  |
| M4  | [ADR-0008](adr/0008-irq-handler-policy.md) (**accepted**): cookie handlers + wake queue; mailbox ABI                                                                                                                                                       | A message crosses between two tasks that share no memory; a send on a capability the sender does not hold is refused and counted, and the refusal is visible on the console; IRQ wakes use the ADR-0008 queue only                                                                                                                                                                       |
| M5  | [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md) + [ADR-0014](adr/0014-ttbr-split-m5.md) (TTBR0 v1); multi-role prep                                                                                                                             | A task runs at EL0 in its own `TTBR0`; an EL0 write to a kernel address takes a permission fault with the ESR recorded here, the way W^X was; `SVC` returns to EL1 and back                                                                                                                                                                                                              |
| M6  | M5 done; [ADR-0013](adr/0013-narrow-device-windows.md) (**accepted**); F26                                                                                                                                                                                 | EL0 agent maps **only** the PL011 page, touches the device, is destroyed (kill); kernel console/ticks continue. RX ownership (poll + real bytes) is a post-v1 product slice gated on QEMU, HW stamp open.                                                                                                                                                                                |
| M7  | [ADR-0017](adr/0017-el0-capability-abi.md) (EL0 capability ABI) and [ADR-0018](adr/0018-agent-fault-policy.md) (agent fault policy), both **accepted** 2026-08-06 — which is what unblocks the milestone under [ADR-0001](adr/0001-multi-role-analysis.md) | Two EL0 agents exchange a message neither can forge; one of them faults; its creator handles the fault and the other keeps running; the kernel stays alive — **on silicon**, with a serial transcript. Slice 1 (EL0 session state in the `Tcb`) is **done (HW)**: the nine machine-wide `static mut` are one published pointer and sessions are per task, so the sentence is now sayable |

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

### Open (QEMU) — M7 slice 2

| Slice                                   | Status          | Evidence                                                                                                                            |
| --------------------------------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| **`SYS_SEND` / `SYS_RECV` by slot**     | **done (QEMU)** | `el0-ipc: sent slot=0 tag=7 a=42`; the receiving agent moves the payload into `SYS_PUTC` itself, so the `*` on the console is the message |
| **Authority refused on the good path**  | **done (QEMU)** | `el0-ipc: refused slot=1 authority=1` — an agent naming a slot its table does not have                                              |
| **Full ≠ unauthorised**                 | **done (QEMU)** | five sends into a four-deep mailbox: `full=1`, authority unchanged                                                                   |
| Silicon                                 | **open**        | the M7 done-when is an EL0→EL0 exchange on hardware, with a serial transcript                                                       |
| Blocking `SYS_RECV`                     | **not done**    | needs a yield out of a live session; deliberately out of this slice (ADR-0017 consequences)                                         |

### Closed (HW) — M7 slice 1, stamped on silicon 2026-08-06 21:25

| Slice                                | Status        | Evidence                                                                                                                                                                                      |
| ------------------------------------ | ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **EL0 session state in the `Tcb`**   | **done (HW)** | nine `static mut` → one `CURRENT_EL0`; every EL0 oracle unchanged and driven from per-task sessions — `resume pings=2`, `putc bytes=2`, `irq resume irqs=1`, `rx own bytes=2`, `pool=496`/`512` |
| **Publication checked, not assumed** | **done (HW)** | no panic across five agents in four tasks; deleting the publish from the switch panics on the first spawned-task entry — see `verification.md`, checks seen to fail                            |
| A switch _inside_ a live session     | **not done**  | per-task state makes it harmless; nothing performs it yet. M7 slice 2's evidence, not this one's                                                                                              |

Transcript: [verification.md §M7 slice 1](verification.md#hardware-evidence-m7-slice-1-per-task-el0-sessions-closed).

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

### Next (ordered)

| #   | Work                                                                                                                         | Done when                                                                                                                                                                                                                                                                    |
| --- | ---------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | **M7 slices 2–4**: cap table + `SYS_SEND`/`SYS_RECV`; `SYS_PUTC` behind a console capability denied by default; fault policy | The M7 done-when above, on silicon                                                                                                                                                                                                                                           |
| 2   | **F22**: two hostile EL0 programs in the boot oracle                                                                         | One faults deliberately, one names a slot it does not hold; both refusals asserted by `make boot-check`. Writable only after slice 2                                                                                                                                         |
| 3   | **Threat model + `SECURITY.md`**                                                                                             | After [ADR-0017](adr/0017-el0-capability-abi.md), which is where authority is finally defined                                                                                                                                                                                |
| 4   | **Optional: IRQ-wake RX**                                                                                                    | UART SPI → EL0 `Irq` without kernel draining `DR`                                                                                                                                                                                                                            |
| 5   | **Optional P-pass**                                                                                                          | Tighten kernel EL1 Device blankets (not required for M6 v1)                                                                                                                                                                                                                  |

**Explicit non-goals** until their own ADR: preemption, TTBR1 high-half, ASID production, SMP, USB host, full framebuffer; long-running interactive echo agent replacing the idle body.

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

| Artefact                                                       | Role                                                                                                |
| -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| [`docs/adr/`](adr/README.md)                                   | Architecture Decision Records (lifecycle: proposed → accepted → superseded)                         |
| [ADR-0001](adr/0001-multi-role-analysis.md)                    | Multi-role analysis as pre-milestone gate (**accepted**)                                            |
| [ADR-0002](adr/0002-softfloat-kernel.md)                       | Kernel compiled softfloat, FP left trapping (**accepted**)                                          |
| [ADR-0003](adr/0003-early-mmu.md)                              | MMU enabled before any Rust runs (**accepted**)                                                     |
| [ADR-0004](adr/0004-gic-group0-firmware-pin.md)                | GIC Group 0 with IAR/EOIR, and the firmware pin (**accepted**)                                      |
| [ADR-0005](adr/0005-static-page-table-arena.md)                | Static page-table arena instead of a frame allocator (**accepted**)                                 |
| [ADR-0006](adr/0006-cooperative-execution-model.md)            | Cooperative execution model (M3 tasks); closes F12 (**accepted**)                                   |
| [ADR-0007](adr/0007-project-identity-harbor-kernel.md)         | Project identity Harbor / `harbor-kernel` (**accepted**)                                            |
| [ADR-0008](adr/0008-irq-handler-policy.md)                     | IRQ handler shape for M4 wakes / caps; closes F13 (**accepted**)                                    |
| [ADR-0009](adr/0009-optional-spi-tft-debug-console.md)         | Optional SPI TFT status surface; lab side-track (**accepted**)                                      |
| [ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md)          | SPI sessions + DBI stream; regwidth-16 SKU note (**accepted**)                                      |
| [ADR-0011](adr/0011-dtb-mapped-board-constants-risk-accept.md) | DTB mapped; board truth compiled-in; closes F15 (**accepted**)                                      |
| [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md)     | Frame allocator for user AS; M5 needs-first (**accepted**)                                          |
| [ADR-0013](adr/0013-narrow-device-windows.md)                  | Narrow device MMIO for agents; F26/M6 v1 (**accepted**)                                             |
| [ADR-0014](adr/0014-ttbr-split-m5.md)                          | TTBR regime M5 v1 (TTBR0 + kernel maps in user AS) (**accepted**)                                   |
| [ADR-0015](adr/0015-multi-arch-scaffold.md)                    | Multi-arch scaffold: cfg facade + board features (**accepted**)                                     |
| [ADR-0016](adr/0016-el0-session-protocol.md)                   | EL0 session protocol: one slot, prose contract, named successor (**superseded**) — by 0017 and 0018 |
| [ADR-0017](adr/0017-el0-capability-abi.md)                     | EL0 capability ABI: slot-indexed authority, session state in the TCB (**accepted**)                 |
| [ADR-0018](adr/0018-agent-fault-policy.md)                     | Agent fault policy: the kernel ends the session, the creator decides the task (**accepted**)        |
| [ADR-0019](adr/0019-no-static-mut.md) | No `static mut`: the last one becomes an atomic, rule 7 without an exception (**proposed**) |
| [`docs/reviews/`](reviews/)                                    | Pass outcomes (findings), not decisions                                                             |

## Non-goals

- Linux / POSIX compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
