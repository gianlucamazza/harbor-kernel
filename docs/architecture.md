# Architecture

## Purpose

**Harbor** (package `harbor-kernel`) **aims to be** an agent-based microkernel for the
Raspberry Pi 4 Model B, where agents are isolated units interacting only
through message passing and capabilities.

It is not a finished agent OS yet. What runs today is a single-core kernel at
EL1 with a protected identity map, interrupts, a heap, **cooperative tasks
(M3)**, **IPC/caps (M4)**, and **M5 address spaces + one-shot EL0** — all
**done on Pi 4B hardware**. The product “agent shell” (scheduled EL0,
syscalls, driver agents) is still the target; the milestone table says which
parts exist.

## Layering

```
┌──────────────────────────────────────────────────────────┐
│  Agents (M5–M6 v1 + concurrent shell done HW)            │
│  message passing · capability-mediated resources         │
└────────────────────────────▲─────────────────────────────┘
                             │ SVC / IPC / (EL0 IRQ later)
┌────────────────────────────┴─────────────────────────────┐
│  Kernel policy                                           │
│  bootstrap · console_loop · sched · ipc · time · console │
│  mm (frames, aspace) · status (debug-display TFT)        │
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

8. Idle (console loop) uses `WFI` when the RX ring is empty, no tick report is
   due, and no task is ready; it **yields** when the runqueue is non-empty. The
   emptiness check runs with IRQs masked so a wakeup cannot be lost.
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
| Scheduler   | `sched`         | cooperative spawn / yield / exit   |
| Task stacks | `mm/task_stack` | heap stack + unmapped guard        |

## Agent model (target beyond M3)

**Tasks exist** (M3): cooperative EL1 scheduling per
[ADR-0006](adr/0006-cooperative-execution-model.md). There is still no
address-space separation and no user mode. The table below marks what is code
today versus roadmap.

| Concept    | Role                                                  | Status        |
| ---------- | ----------------------------------------------------- | ------------- |
| Task (M3)  | Schedulable EL1 entity + private stack; see ADR-0006  | **done (HW)** |
| Agent      | Task + mailbox + private AS at EL0; multi-SVC + IRQ resume; SYS_PUTC; PL011 RX poll | **done (QEMU)**; HW stamp open |
| Message    | Sole interaction channel (M4)                         | **done** (fixed `Message` + mailbox) |
| Capability | Unforgeable handle (send/recv; future: IRQ notification) | **done** (CapId + hold table; IRQ caps later) |

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
| M3  | Cooperative tasks                                        | **done** (HW, fault-probed) |
| M4  | IPC + capabilities                                       | **done (HW)**               |
| M5  | EL0 agents                                               | **done (HW)**               |
| M6  | Driver-as-agent                                          | **done (HW)** (PL011 page agent v1) |

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
| M4  | [ADR-0008](adr/0008-irq-handler-policy.md) (**accepted**): cookie handlers + wake queue; mailbox ABI  | A message crosses between two tasks that share no memory; a send on a capability the sender does not hold is refused and counted, and the refusal is visible on the console; IRQ wakes use the ADR-0008 queue only      |
| M5  | [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md) + [ADR-0014](adr/0014-ttbr-split-m5.md) (TTBR0 v1); multi-role prep | A task runs at EL0 in its own `TTBR0`; an EL0 write to a kernel address takes a permission fault with the ESR recorded here, the way W^X was; `SVC` returns to EL1 and back                                             |
| M6  | M5 done; [ADR-0013](adr/0013-narrow-device-windows.md) (**accepted**); F26                              | EL0 agent maps **only** the PL011 page, touches the device, is destroyed (kill); kernel console/ticks continue                                                                                                            |

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

### Closed slices (post-M5)

| Slice | Status | Evidence |
| ----- | ------ | -------- |
| **M5-P1** scheduled EL0 task | **done (HW)** | `el0-task: svc ping` / `ok` |
| **M5-P2** minimal `SVC` dispatch | **done (HW)** | `kernel_core::syscall::decode`; refuse `0x99` |
| **M5-P3** dual AS create/destroy | **done (HW)** | `aspace: dual create/destroy ok` |
| **M6-D0** [ADR-0013](adr/0013-narrow-device-windows.md) | **accepted** | 2026-08-05 |
| **M6 v1** PL011 page agent + kill | **done (HW)** | `pl011-agent: FR read + svc ok` / `killed ok` |

### Closed (multi-agent shell v1)

| Slice | Status | Evidence |
| ----- | ------ | -------- |
| **Agent shell** (`src/agent`) | **done (HW)** | `Agent` owns AS; one-shot EL0; no fake resume |
| **Concurrent dual agent** | **done (HW)** | `agents: concurrent ok` — two TCBs, both AS live, each EL0 once |

Pi 4B stamp: [verification.md §M5-P / M6](verification.md#m5-p--m6-v1-qemu) (2026-08-05).

### Closed (EL0 multi-SVC resume)

| Slice | Status | Evidence |
| ----- | ------ | -------- |
| **SVC resume** | **done (HW)** | `enter`/`resume`/`end_session`; `SYS_EXIT`; `el0-task: resume pings=2` |
| Preferred ELR for SVC | documented | AArch64 ELR is already past SVC — no software +4 |

### Closed (EL0 IRQ / putc / RX poll — issue #1 v1)

| Slice | Status | Evidence |
| ----- | ------ | -------- |
| **EL0 IRQ save/resume** | **done (QEMU)** | lower-EL IRQ → `El0Outcome::Irq`; `handle_cpu_irq` + **architectural re-execute** resume; `el0-task: irq resume irqs=N` |
| **`SYS_PUTC`** | **done (QEMU)** | imm 2; saved `x0` → kernel TX; `el0-task: putc bytes=2` |
| **PL011 RX poll path** | **done (QEMU)** | `suspend_rx` + user FR/DR poll; `pl011-agent: rx poll empty` (no invented RX data) |
| Kernel panic/TX console | preserved | TX stays kernel-owned; RX drain suspended only for the poll session |

**Non-goals in v1 (not deferred hacks):** software ELR skip after IRQ, treating empty
FIFO as “received”, long-lived agent-owned console RX.

### Next (ordered)

| # | Work | Done when |
| - | ---- | --------- |
| 1 | **HW stamp** for IRQ / putc / RX poll | Same oracles on Pi 4B serial |
| 2 | **Full RX-owned agent** | Agent owns RX long-term with real bytes; idle still ticks; kill restores kernel drain |
| 3 | **Optional P-pass** | Tighten kernel EL1 Device blankets (not required for M6 v1) |

**Explicit non-goals** until their own ADR: preemption, TTBR1 high-half, ASID production, SMP, USB host, full framebuffer.

### Open findings, against the milestone they block

From [the multi-role review](reviews/2026-08-04-multi-role.md). Findings not
listed here block nothing and are tracked in that report alone.

| Finding | Blocks              | Why                                                                                                                                                      |
| ------- | ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| F12     | — (resolved)        | Closed by [ADR-0006](adr/0006-cooperative-execution-model.md); the ADR was the deliverable                                                               |
| F18     | — (resolved)        | Absolute `CNTP_CVAL` deadlines + missed-tick counter; pure cooperative yield never depended on it                                                        |
| F13     | — (resolved)        | Shape accepted: `Handler = fn(IrqCookie)` + IRQ→voluntary wake queue — [ADR-0008](adr/0008-irq-handler-policy.md); code lands with first M4 PR           |
| F26     | — (resolved M6 v1)  | [ADR-0013](adr/0013-narrow-device-windows.md) **accepted**; agent maps are page-sized named windows only; kernel coarse Device may remain until a P-pass  |
| F15     | — (resolved)        | Risk-accepted: board truth is BSP constants; DTB mapped RO for a future parser — [ADR-0011](adr/0011-dtb-mapped-board-constants-risk-accept.md)           |
| F24     | — (resolved)        | Layering rules 1–4 are enforced by `make layering`; non-import coupling remains review-only (gate blind spots in verification)                           |

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
gate. Must not block or redefine M4–M6; M6 may later *reuse* those drivers as
agents.

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
| [ADR-0007](adr/0007-project-identity-harbor-kernel.md) | Project identity Harbor / `harbor-kernel` (**accepted**) |
| [ADR-0008](adr/0008-irq-handler-policy.md)      | IRQ handler shape for M4 wakes / caps; closes F13 (**accepted**) |
| [ADR-0009](adr/0009-optional-spi-tft-debug-console.md) | Optional SPI TFT status surface; lab side-track (**accepted**) |
| [ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md) | SPI sessions + DBI stream; regwidth-16 SKU note (**accepted**) |
| [ADR-0011](adr/0011-dtb-mapped-board-constants-risk-accept.md) | DTB mapped; board truth compiled-in; closes F15 (**accepted**) |
| [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md) | Frame allocator for user AS; M5 needs-first (**accepted**) |
| [ADR-0013](adr/0013-narrow-device-windows.md) | Narrow device MMIO for agents; F26/M6 v1 (**accepted**) |
| [ADR-0014](adr/0014-ttbr-split-m5.md) | TTBR regime M5 v1 (TTBR0 + kernel maps in user AS) (**accepted**) |
| [`docs/reviews/`](reviews/)                     | Pass outcomes (findings), not decisions                                     |

## Non-goals

- Linux / POSIX compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
