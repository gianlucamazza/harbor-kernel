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
│  bootstrap · shell · time · console · mm · (sched M3+)   │
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
   `mmu::enable`.** `swap`, `fetch_add` and `compare_exchange` compile to an
   `LDXR`/`STXR` pair, and with the MMU off every access is Device-nGnRnE,
   where exclusives do not make progress on Cortex-A72 — the retry loop spins
   forever. The board goes silent with no fault to show for it, and QEMU does
   not reproduce it, because its exclusive monitor ignores memory attributes.
   Plain atomic loads and stores (`LDAR`/`STLR`) are fine anywhere. Code that
   runs before the MMU — `console::acquire`, the panic handler — uses
   `SyncCell` and the single-core invariant instead.
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
going, not what the code does. `mm::paging` and the free-list allocator are the
first two pieces the rest can be built on; the third, an execution abstraction,
does not exist yet.

| Concept    | Role                                                  |
| ---------- | ----------------------------------------------------- |
| Agent      | Schedulable entity + mailbox; later own address space |
| Message    | Sole interaction channel                              |
| Capability | Unforgeable handle (future: IRQ notification)         |

`irq::register` is the hook for later capability mediation.

## Milestones

| ID  | Deliverable                               | Status                    |
| --- | ----------------------------------------- | ------------------------- |
| M0  | Hello UART + echo                         | **done**                  |
| M1  | Exceptions + timer IRQ ticks              | **done** (HW)             |
| M2  | MMU + kernel heap (+ atomics after attrs) | **done** (HW)             |
| P0  | Idle (WFI) + UART RX IRQ + ring            | **done** (HW)             |
| P1  | W^X + guard page + free-list `GlobalAlloc`  | **done** (HW, fault-probed) |
| P2  | Early MMU, softfloat, build-enforced gates  | **done** (HW)             |
| M3  | Cooperative tasks                         | planned                   |
| M4  | IPC + capabilities                        | planned                   |
| M5  | EL0 agents                                | planned                   |
| M6  | Driver-as-agent                           | planned                   |

M3 is unblocked: it needs an allocator that frees and per-region permissions,
both of which exist. What it still lacks is an execution abstraction — there is
no task, no context switch, no scheduler.

M5 needs two things `arch::mmu` does not have: a frame allocator (the table
arena is a fixed, build-time pool, which is the right shape for mapping the
kernel once and the wrong one for address spaces that come and go) and a notion
of more than one address space at all — `activate` installs *the* map, and
`TTBR0` is switched once.

"done (HW)" means the deliverable was observed working on a Raspberry Pi 4B,
not merely in QEMU. The distinction earned its place: emulation booted a kernel
that hung on silicon, because TCG's exclusive monitor ignores memory
attributes. See [`verification.md`](verification.md).

## Non-goals

- Linux / POSIX compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
