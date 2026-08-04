# Architecture

## Purpose

**Harbor** (package `harbor-kernel`) **aims to be** an agent-based microkernel for the
Raspberry Pi 4 Model B, where agents are isolated units interacting only
through message passing and capabilities.

It is not one yet. What runs today is a single-core kernel at EL1: a protected
identity map, interrupts, a heap, and a serial console. The agent model below
is the target; the milestone table says which parts exist.

## Layering

```
┌──────────────────────────────────────────────────────────┐
│  Agents (future EL0)                                     │
│  message passing · capability-mediated resources         │
└────────────────────────────▲─────────────────────────────┘
                             │ syscalls / IPC / cap_irq
┌────────────────────────────┴─────────────────────────────┐
│  Kernel policy                                           │
│  bootstrap · console_loop · time · console · mm          │
│  (scheduler from M3)                                     │
└───────────▲─────────────────────────────▲────────────────┘
            │ register / handle           │
┌───────────┴───────────┐     ┌───────────┴────────────────┐
│  irq                  │     │  drivers                   │
│  dispatch · IrqChip   │     │  gicv2 · pl011             │
└───────────▲───────────┘     └───────────▲────────────────┘
            │ claim/eoi                   │
┌───────────┴───────────┐     ┌───────────┴────────────────┐
│  arch/exception       │     │  arch/{timer,mmu,cache}    │
│  VBAR · frame · entry │     │  CNTP · page tables · maint│
└───────────────────────┘     └────────────────────────────┘
            ▲                              ▲
            │         bsp/rpi4             │
            └──── memmap · IRQ bind ───────┘
```

### Rules

1. Drivers never know the board (bases / IRQ ids from BSP).
2. BSP never implements protocols (bind only).
3. Arch never names board peripherals (Generic Timer + CPU only).
4. `exception` does not import drivers, BSP, or time — only `irq::handle_cpu_irq`.
5. One irqchip owner via `irq::init(&'static dyn IrqChip)`.
6. IRQ handlers do not **transmit** on the console (`println` / TX). The UART
   RX handler may only drain the FIFO into the kernel ring.
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

8. Main idle uses `WFI` when the RX ring is empty and no tick report is due,
   with IRQs masked across the check so a wakeup cannot be lost.
9. Nothing is both writable and executable, and diagnostic scaffolding lives
   behind the `bringup` feature rather than in the production surface.

Rules 1–4 are checked by `make layering` (`scripts/check-layering.sh`) against
every `crate::` import edge. Coupling that is not an import (a shared constant,
an agreed register value) is still review-only — see
[`verification.md`](verification.md).

## Interrupt / timer / console contract

| Role        | Module          | Responsibility                     |
| ----------- | --------------- | ---------------------------------- |
| Clocksource | `arch/timer`    | CNTP deadline, ISTATUS, re-arm     |
| Irqchip     | `drivers/gicv2` | enable, claim/EOI, SPI target CPU0 |
| Dispatch    | `irq`           | id → handler                       |
| Tick policy | `time`          | `on_timer_irq`, `ticks()`          |
| Console RX  | `console`       | ring, `on_uart_rx_irq`, `pop_rx`   |
| Bind        | `bsp/rpi4/irq`  | TIMER=30, UART=153, static GIC     |
| Layout      | `mm/layout`     | regions and their permissions      |
| Allocation  | `mm`            | free list + `GlobalAlloc`          |

## Agent model (target — none of this is implemented)

The kernel today has no scheduler in code, no address-space separation and no
user mode. What runs is still a single control flow at EL1. The **execution
model** for the first unit of concurrency is decided in
[ADR-0006](adr/0006-cooperative-execution-model.md) (cooperative EL1 tasks,
heap stacks with unmapped guards, no preemption); implementing it is M3.
Everything below describes where the design goes after that, not what the code
does yet. `kernel_core::paging` with `arch::mmu`, and the free-list allocator,
are the first two pieces; the third is tasks under that ADR.

| Concept    | Role                                                  |
| ---------- | ----------------------------------------------------- |
| Task (M3)  | Schedulable EL1 entity + private stack; see ADR-0006  |
| Agent      | Task + mailbox; later own address space (M5)          |
| Message    | Sole interaction channel (M4)                         |
| Capability | Unforgeable handle (future: IRQ notification)         |

`irq::register` is the hook for later capability mediation.

## Milestones

| ID  | Deliverable                                              | Status                      |
| --- | -------------------------------------------------------- | --------------------------- |
| M0  | Hello UART + echo                                        | **done**                    |
| M1  | Exceptions + timer IRQ ticks                             | **done** (HW)               |
| M2  | MMU + kernel heap (+ atomics after attrs)                | **done** (HW)               |
| P0  | Idle (WFI) + UART RX IRQ + ring                          | **done** (HW)               |
| P1  | W^X + guard page + free-list `GlobalAlloc`               | **done** (HW, fault-probed) |
| P2  | Early MMU, softfloat, build-enforced gates               | **done** (HW)               |
| P3  | Layout validation, runtime `map` + TLB maintenance, ADRs | **done** (HW)               |
| P4  | Exception stack, refused frees, fatal map failure         | **done** (HW, fault-probed) |
| M3  | Cooperative tasks                                        | **QEMU** (HW overflow probe pending) |
| M4  | IPC + capabilities                                       | planned                     |
| M5  | EL0 agents                                               | planned                     |
| M6  | Driver-as-agent                                          | planned                     |

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

| ID  | Needs first                                                                                           | Done when                                                                                                                                                                                                               |
| --- | ----------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M3  | [ADR-0006](adr/0006-cooperative-execution-model.md) (F12 done); per-task heap stack + unmapped guard  | Two tasks yield to each other on hardware and the console shows their output interleaved; each task stack is validated by `mm::layout`; a probe shows one task's overflow faulting rather than reaching another's stack |
| M4  | An IRQ/handler-policy ADR (F13) — `Handler = fn()` has nowhere to carry a capability                  | A message crosses between two tasks that share no memory; a send on a capability the sender does not hold is refused and counted, and the refusal is visible on the console                                             |
| M5  | A frame allocator (ADR-0005 is the wrong shape for this); more than one address space; EL0 entry/exit | A task runs at EL0 in its own `TTBR0`; an EL0 write to a kernel address takes a permission fault with the ESR recorded here, the way W^X was; `SVC` returns to EL1 and back                                             |
| M6  | M4 and M5; narrower device windows (F26) — a driver agent must not receive 16 MiB of MMIO             | The PL011 RX path runs as an EL0 agent and the console still echoes; killing that agent leaves the kernel ticking                                                                                                       |

M3 is **in progress**. [ADR-0006](adr/0006-cooperative-execution-model.md) is
**accepted**. Implemented: runqueue in `kernel-core`, `mmu::unmap` with block
split, heap task stacks with unmapped guards, voluntary context switch, idle =
console loop, demo tasks with interleaved console output under QEMU (gated by
`boot-check`). Still required for `done (HW)`: overflow probe on silicon
(translation fault on a task guard) and a short multi-role pass. Inventing
preemption or `link.ld` task stacks is a reversal of the ADR.

### Open findings, against the milestone they block

From [the multi-role review](reviews/2026-08-04-multi-role.md). Findings not
listed here block nothing and are tracked in that report alone.

| Finding | Blocks              | Why                                                                                                                                                      |
| ------- | ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| F12     | — (resolved)        | Closed by [ADR-0006](adr/0006-cooperative-execution-model.md); the ADR was the deliverable                                                               |
| F18     | time-based sched.   | Relative `TVAL` re-arm drifts phase; blocks sleep-N-ticks or a preemptive quantum, not pure cooperative yield (ADR-0006)                                 |
| F13     | M4                  | `Handler = fn()` cannot carry a capability, and M4 is where handlers become mediated                                                                     |
| F26     | M6                  | Device windows are 16 MiB blankets; an agent-owned driver would receive all of it                                                                        |
| F15     | none                | The DTB is mapped and never parsed, so board truth stays hard-coded. Parse it or risk-accept it in an ADR — today it is neither                          |
| F24     | — (resolved)        | Layering rules 1–4 are enforced by `make layering`; non-import coupling remains review-only (gate blind spots in verification)                           |

## Decisions and reviews

The choices that constrain the code have an ADR, each naming the alternative
that was rejected and the gate that would catch its reversal.

| Artefact                                        | Role                                                                        |
| ----------------------------------------------- | --------------------------------------------------------------------------- |
| [`docs/adr/`](adr/README.md)                    | Architecture Decision Records (lifecycle: proposed → accepted → superseded) |
| [ADR-0001](adr/0001-multi-role-analysis.md)     | Multi-role analysis as pre-milestone gate (**accepted**)                    |
| [ADR-0002](adr/0002-softfloat-kernel.md)        | Kernel compiled softfloat, FP left trapping (**accepted**)                  |
| [ADR-0003](adr/0003-early-mmu.md)               | MMU enabled before any Rust runs (**accepted**)                             |
| [ADR-0004](adr/0004-gic-group0-firmware-pin.md) | GIC Group 0 with IAR/EOIR, and the firmware pin (**accepted**)              |
| [ADR-0005](adr/0005-static-page-table-arena.md) | Static page-table arena instead of a frame allocator (**accepted**)         |
| [ADR-0006](adr/0006-cooperative-execution-model.md) | Cooperative execution model (M3 tasks); closes F12 (**accepted**) |
| [`docs/reviews/`](reviews/)                     | Pass outcomes (findings), not decisions                                     |

## Non-goals

- Linux / POSIX compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
