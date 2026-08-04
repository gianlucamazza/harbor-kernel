# Architecture

## Purpose

`rpi_minimal_agentic` **aims to be** an agent-based microkernel for the
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

The kernel today has no unit of execution, no scheduler, no address-space
separation and no user mode. Everything below describes where the design is
going, not what the code does. `kernel_core::paging` with `arch::mmu`, and the free-list allocator, are the
first two pieces the rest can be built on; the third, an execution abstraction,
does not exist yet.

| Concept    | Role                                                  |
| ---------- | ----------------------------------------------------- |
| Agent      | Schedulable entity + mailbox; later own address space |
| Message    | Sole interaction channel                              |
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
| P4  | Exception stack, refused frees, fatal map failure        | QEMU only — **open**        |
| M3  | Cooperative tasks                                        | planned, blocked            |
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
[`verification.md`](verification.md). P4 is `open` for exactly this reason — it
works under emulation, and it changes the boot sequence and the vector group the
hardware enters through, which is the category emulation has already been wrong
about here.

### What each planned milestone needs, and how it is judged done

The done column above was earned against a stated observable. The same standard
applies forwards, or it is not the same standard.

| ID  | Needs first                                                                                           | Done when                                                                                                                                                                                                               |
| --- | ----------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M3  | An execution-model ADR (F12); a per-task stack with a guard, from the heap rather than `link.ld`      | Two tasks yield to each other on hardware and the console shows their output interleaved; each task stack is validated by `mm::layout`; a probe shows one task's overflow faulting rather than reaching another's stack |
| M4  | An IRQ/handler-policy ADR (F13) — `Handler = fn()` has nowhere to carry a capability                  | A message crosses between two tasks that share no memory; a send on a capability the sender does not hold is refused and counted, and the refusal is visible on the console                                             |
| M5  | A frame allocator (ADR-0005 is the wrong shape for this); more than one address space; EL0 entry/exit | A task runs at EL0 in its own `TTBR0`; an EL0 write to a kernel address takes a permission fault with the ESR recorded here, the way W^X was; `SVC` returns to EL1 and back                                             |
| M6  | M4 and M5; narrower device windows (F26) — a driver agent must not receive 16 MiB of MMIO             | The PL011 RX path runs as an EL0 agent and the console still echoes; killing that agent leaves the kernel ticking                                                                                                       |

M3 is **blocked, not merely unplanned**. Its dependencies exist — an allocator
that frees, per-region permissions, per-stack guards — but [ADR-0001](adr/0001-multi-role-analysis.md)
requires that a finding which moves a boundary becomes an ADR _before_ the code
implementing it, and F12 is precisely that. Writing tasks first would make the
execution model an artefact of the first implementation that compiled.

### Open findings, against the milestone they block

From [the multi-role review](reviews/2026-08-04-multi-role.md). Findings not
listed here block nothing and are tracked in that report alone.

| Finding | Blocks | Why                                                                                                                             |
| ------- | ------ | ------------------------------------------------------------------------------------------------------------------------------- |
| F12     | M3     | No execution model is recorded; the ADR is the deliverable, not the code                                                        |
| F18     | M3     | The timer re-arms relative to `TVAL`, so ticks drift in phase — a scheduler inherits that                                       |
| F13     | M4     | `Handler = fn()` cannot carry a capability, and M4 is where handlers become mediated                                            |
| F26     | M6     | Device windows are 16 MiB blankets; an agent-owned driver would receive all of it                                               |
| F15     | none   | The DTB is mapped and never parsed, so board truth stays hard-coded. Parse it or risk-accept it in an ADR — today it is neither |
| F24     | none   | The layering rules above are enforced by review only. This project has twice watched an ungated rule be forgotten               |

## Decisions and reviews

The choices that constrain the code have an ADR, each naming the alternative
that was rejected and the gate that would catch its reversal.

| Artefact                                        | Role                                                                        |
| ----------------------------------------------- | --------------------------------------------------------------------------- |
| [`docs/adr/`](adr/README.md)                    | Architecture Decision Records (lifecycle: proposed → accepted → superseded) |
| [ADR-0001](adr/0001-multi-role-analysis.md)     | Multi-role analysis as pre-milestone gate                                   |
| [ADR-0002](adr/0002-softfloat-kernel.md)        | Kernel compiled softfloat, FP left trapping                                 |
| [ADR-0003](adr/0003-early-mmu.md)               | MMU enabled before any Rust runs                                            |
| [ADR-0004](adr/0004-gic-group0-firmware-pin.md) | GIC Group 0 with IAR/EOIR, and the firmware pin                             |
| [ADR-0005](adr/0005-static-page-table-arena.md) | Static page-table arena instead of a frame allocator                        |
| [`docs/reviews/`](reviews/)                     | Pass outcomes (findings), not decisions                                     |

## Non-goals

- Linux / POSIX compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
