# Architecture

## Purpose

`rpi_minimal_agentic` is an agent-based microkernel for the Raspberry Pi 4
Model B. Agents will be isolated units that interact only through message
passing and capabilities.

**M2 + P0 in tree:** identity MMU + bump heap, timer IRQ ticks, **idle (WFI)**
+ **UART RX IRQ** into a kernel ring. Validate idle/RX on hardware after flash.

## Layering

```
┌──────────────────────────────────────────────────────────┐
│  Agents (future EL0)                                     │
│  message passing · capability-mediated resources         │
└────────────────────────────▲─────────────────────────────┘
                             │ syscalls / IPC / cap_irq
┌────────────────────────────┴─────────────────────────────┐
│  Kernel policy                                           │
│  bootstrap · time · console · (sched M3+)                │
└───────────▲─────────────────────────────▲────────────────┘
            │ register / handle           │
┌───────────┴───────────┐     ┌───────────┴────────────────┐
│  irq                  │     │  drivers                   │
│  dispatch · IrqChip   │     │  gicv2 · pl011             │
└───────────▲───────────┘     └───────────▲────────────────┘
            │ claim/eoi                   │
┌───────────┴───────────┐     ┌───────────┴────────────────┐
│  arch/exception       │     │  arch/timer (CNTP)         │
│  VBAR · frame · entry │     │  program / rearm / pending│
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
7. No hardware atomics until the MMU establishes memory attributes (M2).
8. Main idle uses `WFI` when the RX ring is empty and no tick report is due.

## Interrupt / timer / console contract

| Role | Module | Responsibility |
|------|--------|----------------|
| Clocksource | `arch/timer` | CNTP deadline, ISTATUS, re-arm |
| Irqchip | `drivers/gicv2` | enable, claim/EOI, SPI target CPU0 |
| Dispatch | `irq` | id → handler |
| Tick policy | `time` | `on_timer_irq`, `ticks()` |
| Console RX | `console` | ring, `on_uart_rx_irq`, `pop_rx` |
| Bind | `bsp/rpi4/irq` | TIMER=30, UART=153, static GIC |

## Agent model (target)

| Concept | Role |
|---------|------|
| Agent | Schedulable entity + mailbox; later own address space |
| Message | Sole interaction channel |
| Capability | Unforgeable handle (future: IRQ notification) |

`irq::register` is the hook for later capability mediation.

## Milestones

| ID | Deliverable | Status |
|----|-------------|--------|
| M0 | Hello UART + echo | **done** |
| M1 | Exceptions + timer IRQ ticks | **done** (HW) |
| M2 | MMU + kernel heap (+ atomics after attrs) | **done** (HW) |
| P0 | Idle (WFI) + UART RX IRQ + ring | **in tree** — HW validate |
| M3 | Cooperative tasks | planned |
| M4 | IPC + capabilities | planned |
| M5 | EL0 agents | planned |
| M6 | Driver-as-agent | planned |

## Non-goals

- Linux / POSIX compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
