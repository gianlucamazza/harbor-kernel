# Architecture

This is the normative description of how Harbor works **today**. For the
documentation map, ownership rules and status vocabulary see
[`docs/README.md`](README.md); for evidence rather than design intent see
[`verification.md`](verification.md). Product shape and use cases:
[`vision.md`](vision.md). Completeness policy:
[ADR-0026](adr/0026-kernel-and-product-completeness.md).

New to the vocabulary (agent, slot, grant, K/P, `done (QEMU)` vs `done (HW)`)?
[`glossary.md`](glossary.md). What it is built with: [`stack.md`](stack.md).
The closed foundation record: [`foundation-history.md`](foundation-history.md).

## Purpose

Harbor aims at a **complete** agent-based microkernel and product OS on the
Raspberry Pi 4 Model B: agents, messages, capability grants, and services on
that model — not Linux/POSIX parity
([ADR-0026](adr/0026-kernel-and-product-completeness.md),
[`vision.md`](vision.md)). Public orientation: [`../README.md`](../README.md).

**Today:** foundation **done on Pi 4B** (cooperative tasks through console
endpoint, manifest loader, blocking recv, parked-wait cancel). The kernel and
product OS are **not yet complete**; open work is the
[completeness roadmap](roadmap.md). Historical milestone narrative:
[foundation history](foundation-history.md).

## How Harbor differs from a traditional kernel

A traditional OS treats a **process** (or thread) as both the unit of
isolation and the unit of scheduling: user code runs, traps into the kernel,
and resumes the same schedulable context. Harbor is shaped by a different
question — make isolation, authority, messaging and verification _visible
enough to test on silicon_ — so several familiar assumptions do not hold.

```
Traditional:                         Harbor:
  scheduler ──► process / thread       scheduler ──► driver task (EL1)
                    │                                  │ session loop
                    └─ trap / svc                      ├─ enter / resume EL0 program
                       user code                       │    (private AS, slot caps)
                                                       └─ park on recv = park the driver
```

| Concern               | Traditional kernel                                    | Harbor                                                                                                                                                                                                                                                                                            |
| --------------------- | ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Schedulable unit      | Process or thread                                     | **Driver task** only; the EL0 program is not what `sched` switches ([ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md))                                                                                                                                                         |
| Isolation unit        | Same process (AS + credentials)                       | **Agent = pair**: EL1 driver + EL0 program with its own address space                                                                                                                                                                                                                             |
| Preemption            | Timer quantum; IRQs may switch                        | Voluntary yield primary + **quantum preemption on the IRQ epilogue** (EL0 [ADR-0064](adr/0064-k4-el0-preemption-first-slice.md), EL1 [ADR-0068](adr/0068-k4-el1-preemption-second-slice.md)); device handlers still never switch ([ADR-0006](adr/0006-cooperative-execution-model.md) as amended) |
| Authority names       | Forgeable-looking handles / FDs checked in the kernel | EL0 names only a **slot index** into its own table; raw `CapId` never leaves the kernel ([ADR-0017](adr/0017-el0-capability-abi.md))                                                                                                                                                              |
| How work is created   | Dynamic spawn / load paths                            | Agents described as **manifest data**; the loader binds grants it already holds ([ADR-0021](adr/0021-agents-as-data-and-the-manifest.md))                                                                                                                                                         |
| Device drivers        | In-kernel, or servers with broad maps                 | **Driver-as-agent** with page-sized named MMIO windows ([ADR-0013](adr/0013-narrow-device-windows.md))                                                                                                                                                                                            |
| User fault            | Kernel reaps or signals the process                   | Kernel **ends the session**; the **creator** decides the driver task’s fate ([ADR-0018](adr/0018-agent-fault-policy.md))                                                                                                                                                                          |
| “Done” for a boundary | Feature lands and tests pass in CI                    | **M** milestones add capability; **P** milestones add protection/evidence; hardware claims need Pi 4B stamps ([verification.md](verification.md))                                                                                                                                                 |

Three consequences that look unrelated are the same shape fact:

1. **Cost.** Every agent spends one of a small number of task slots and a 16 KiB
   kernel stack on the driver loop, even when the EL0 text is tiny.
2. **Kill and lifetime.** You cannot kill “the agent” without ending the driver
   that was watching it; the session lives in that task’s TCB.
3. **Preemption was not a small scheduler change.** Preempting a driver
   mid-session with a live EL0 context needed the per-task session state of
   ADR-0017 first, and landed as two deliberate slices (EL0
   [ADR-0064](adr/0064-k4-el0-preemption-first-slice.md), then EL1
   [ADR-0068](adr/0068-k4-el1-preemption-second-slice.md)).

None of this claims superiority over production kernels. **POSIX remains out
of model.** SMP is an **open completeness track** (not a permanent refusal);
fairness under a hostile busy-loop is now enforced at both ELs by the IRQ
epilogue — residuals today are honest in [`SECURITY.md`](../SECURITY.md).
The payoff of the shape is that each boundary is named, gated, and
demonstrable rather than implied by a large ABI.

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
│  console · agent · mm (frames, aspace) · taskcap · status│
│  naming · storage · durable (P5/P2 service state)        │
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
   and `scripts/check/pre-mmu-path.sh` fails the build if that changes. Because
   the window is now that small, `console::acquire` and the panic handler use
   ordinary atomics like everything else.

   **Second sub-rule, and it is about scope rather than about type: a `DAIF`
   save/restore pair must not span a call that can switch tasks.**
   `cpu::without_irqs` reads `DAIF` before its closure and writes it back after,
   so a switch in between hands the next task this task's mask and later
   restores a value captured in an epoch that has ended. The EL0 session loop
   used to hold one mask for the whole session; it holds one per enter/resume
   step now, because `SYS_RECV` parks (ADR-0022). `scripts/check/irq-scope.sh`
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

   That build also reports a number worth reading: how many items are
   unreachable without the oracle (69 as of 2026-08-09; the number is
   reprinted by every `make product-builds` run, which is the source to
   trust over this sentence). `bootstrap::loader` is product code and
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

Rules 1, 3, 4 and 10 are checked by `make layering` (`scripts/check/layering.sh`)
against every `crate::` import edge (and ISA/board path leaks). Rule 2's
substance — _what_ the BSP does, bind rather than implement — is not decidable
from an import graph and stays review's; rule 5 is conformant by a single
`irq::init` call site, enforced by nothing. Coupling that is not an import (a
shared constant, an agreed register value) is still review-only — see
[`verification.md`](verification.md).

**Agent shell imports** (policy, not a lower layer): `arch`, `mm`, `sched`,
`console`, `naming` (ADR-0039 `SYS_RESOLVE`), `irq` (lower-EL IRQ →
`handle_cpu_irq` then resume) and `ipc` (`SYS_SEND`/`SYS_RECV` by slot — the
translation lives in `ipc` because the authority counter is `ipc`'s to
maintain). The peer-transfer path reaches `taskcap` only through `sched`
(ADR-0054/0055). No `drivers` / `bsp` from `agent` — board PA/VA for demos
live in bootstrap.

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
**PL011 driver agent** (M6) are in tree. Voluntary switches primary, with
quantum preemption on the IRQ-return epilogue ([ADR-0006](adr/0006-cooperative-execution-model.md)
as amended by [ADR-0068](adr/0068-k4-el1-preemption-second-slice.md)).

| Concept    | Role                                                                                                                                                                                                                        | Status                                                                    |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Task (M3)  | Schedulable EL1 entity + private stack                                                                                                                                                                                      | **done (HW)**                                                             |
| Agent      | A **pair**: an EL1 driver task and the EL0 program it drives ([ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)). Multi-SVC, IRQ resume, console via `SYS_SEND`, PL011 RX own, and a recv it can wait on | **done (HW)**                                                             |
| Message    | Sole interaction channel (M4)                                                                                                                                                                                               | **done (HW)**                                                             |
| Capability | Unforgeable handle (send/recv; IRQ notification — [ADR-0030](adr/0030-el0-irq-capability.md); channel revoke — [ADR-0032](adr/0032-k3-channel-revoke.md))                                                                   | **done (HW)** (stamp 2026-08-08; status SSOT is [roadmap.md](roadmap.md)) |
| Manifest   | The table that says which agents exist and what each is granted ([ADR-0021](adr/0021-agents-as-data-and-the-manifest.md))                                                                                                   | **done (HW)**                                                             |

`irq::register` is the hook for later capability mediation.

### An agent is a pair, and the driver is the schedulable half

`Agent::run_user_prog_resuming` is a **synchronous loop owned by an EL1 task**:
it enters EL0, handles each SVC, resumes, and returns when the session ends. So
what `sched` admits and switches to is the _driver_, not the EL0 context —
and an agent costs one of `MAX_TASKS` slots plus a 16 KiB kernel stack on top of
its address space, however small its program.

[ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) records the
shape rather than changing it, because three things that look unrelated are the
same fact: preemption would have to preempt the driver mid-session with a live
EL0 context; ADR-0018's _"the creator decides what happens to the task"_ means
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

<a id="roadmap"></a>

## Foundation history (M0–M8)

The milestone narrative — what each **M**/**P** milestone had to show, every
closed slice, the hardware stamps that closed it, and the foundation review
findings — lives in [`foundation-history.md`](foundation-history.md).

It is a **record**, not planning: the foundation is closed on Pi 4B (M0–M8 plus
the parked-wait policy of [ADR-0024](adr/0024-parked-task-visibility.md) /
[ADR-0025](adr/0025-cancel-blocked-wait.md)), and live work is numbered K/P in
[`roadmap.md`](roadmap.md). The optional SPI TFT status surface
([ADR-0009](adr/0009-optional-spi-tft-debug-console.md),
[ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md)) is recorded there too, as
the side-track it was.

<a id="completeness-roadmap"></a>

## Completeness roadmap

**Tables live in [`roadmap.md`](roadmap.md)** (single source of truth for K/P
status). Policy: [ADR-0026](adr/0026-kernel-and-product-completeness.md).

| Snapshot                     | Tracks                                                                                                                                                            |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **done (HW)** H1 depth stamp | 2026-08-08 serial — K5 thin, P2 durable, K4 budget, lifecycle residuals ([verification](verification.md#hardware-evidence-h1-depth-stamps-on-silicon-2026-08-08)) |
| **H1 next**                  | P3\|P4 only with composition (deferred) · SD power-cycle · IRQ preemption                                                                                         |
| **H2 depth**                 | K4 IRQ preemption residual; K7 first slice done (HW); K8 code after design ADR                                                                                    |
| **open (kernel)**            | K4 IRQ preemption _code_, K7 residuals / K8 code                                                                                                                  |
| **open (product)**           | P2 SD residual, P3/P4 deferred (ADR-0049)                                                                                                                         |

When a track changes status, edit **`roadmap.md` only** — do not re-list full
K/P tables here. Horizon mapping and working order also live in `roadmap.md`.

## Decisions and reviews

The choices that constrain the code have an ADR, each naming the alternative
that was rejected and the gate that would catch its reversal.

| Artefact                                                             | Role                                                                                                                           |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| [`../SECURITY.md`](../SECURITY.md)                                   | Threat model and reporting (M7 authority surface; residuals named)                                                             |
| [`docs/adr/`](adr/README.md)                                         | Architecture Decision Records (lifecycle: proposed → accepted → superseded)                                                    |
| [ADR-0001](adr/0001-multi-role-analysis.md)                          | Multi-role analysis as pre-milestone gate (**accepted**)                                                                       |
| [ADR-0002](adr/0002-softfloat-kernel.md)                             | Kernel compiled softfloat, FP left trapping (**accepted**)                                                                     |
| [ADR-0003](adr/0003-early-mmu.md)                                    | MMU enabled before any Rust runs (**accepted**)                                                                                |
| [ADR-0004](adr/0004-gic-group0-firmware-pin.md)                      | GIC Group 0 with IAR/EOIR, and the firmware pin (**accepted**)                                                                 |
| [ADR-0005](adr/0005-static-page-table-arena.md)                      | Static page-table arena instead of a frame allocator (**accepted**)                                                            |
| [ADR-0006](adr/0006-cooperative-execution-model.md)                  | Cooperative execution model (M3 tasks); closes F12 (**accepted**)                                                              |
| [ADR-0007](adr/0007-project-identity-harbor-kernel.md)               | Project identity Harbor / `harbor-kernel` (**accepted**)                                                                       |
| [ADR-0008](adr/0008-irq-handler-policy.md)                           | IRQ handler shape for M4 wakes / caps; closes F13 (**accepted**)                                                               |
| [ADR-0009](adr/0009-optional-spi-tft-debug-console.md)               | Optional SPI TFT status surface; lab side-track (**accepted**)                                                                 |
| [ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md)                | SPI sessions + DBI stream; regwidth-16 SKU note (**accepted**)                                                                 |
| [ADR-0011](adr/0011-dtb-mapped-board-constants-risk-accept.md)       | DTB mapped; board truth compiled-in; closes F15 (**accepted**)                                                                 |
| [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md)           | Frame allocator for user AS; M5 needs-first (**accepted**)                                                                     |
| [ADR-0013](adr/0013-narrow-device-windows.md)                        | Narrow device MMIO for agents; F26/M6 v1 (**accepted**)                                                                        |
| [ADR-0014](adr/0014-ttbr-split-m5.md)                                | TTBR regime M5 v1 (TTBR0 + kernel maps in user AS) (**accepted**)                                                              |
| [ADR-0015](adr/0015-multi-arch-scaffold.md)                          | Multi-arch scaffold: cfg facade + board features (**accepted**)                                                                |
| [ADR-0016](adr/0016-el0-session-protocol.md)                         | EL0 session protocol: one slot, prose contract, named successor (**superseded**) — by 0017 and 0018                            |
| [ADR-0017](adr/0017-el0-capability-abi.md)                           | EL0 capability ABI: slot-indexed authority, session state in the TCB (**accepted**)                                            |
| [ADR-0018](adr/0018-agent-fault-policy.md)                           | Agent fault policy: the kernel ends the session, the creator decides the task (**accepted**)                                   |
| [ADR-0019](adr/0019-no-static-mut.md)                                | No `static mut`: the last one becomes an atomic, rule 7 without an exception (**accepted**)                                    |
| [ADR-0020](adr/0020-spidevice-contract-without-a-caller.md)          | `SpiDevice`: contract kept, ADR-0010's descriptive sentence retracted (**accepted**)                                           |
| [ADR-0021](adr/0021-agents-as-data-and-the-manifest.md)              | Agents as data described by a manifest; the grant becomes a binding, not code (**accepted**)                                   |
| [ADR-0022](adr/0022-blocking-recv-and-the-mask-that-travels.md)      | Blocking `SYS_RECV`: the agent parks; `without_irqs` stops spanning a switch (**accepted**)                                    |
| [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) | An agent is a **pair**: an EL1 driver task and the EL0 program it drives; the driver is what the scheduler runs (**accepted**) |
| [ADR-0024](adr/0024-parked-task-visibility.md)                       | Parked tasks are counted (`blocked_count` / `block_events`); reclaim/timeout deferred (**accepted**)                           |
| [ADR-0025](adr/0025-cancel-blocked-wait.md)                          | Supervisor `cancel_blocked` aborts a parked wait (`Cancelled`); no timeout queue (**accepted**)                                |
| [ADR-0026](adr/0026-kernel-and-product-completeness.md)              | Completeness of kernel (K) and product OS (P) is the project goal (**accepted**)                                               |
| [ADR-0027](adr/0027-h1-external-agent-store.md)                      | H1 entry: external agent store at fixed PA (**accepted**)                                                                      |
| [ADR-0028](adr/0028-wait-on-irq.md)                                  | K1 entry: EL1 wait on IRQ cookie (**accepted**)                                                                                |
| [ADR-0029](adr/0029-agent-store-in-image.md)                         | Agent store placement: image section inject (**accepted**)                                                                     |
| [ADR-0030](adr/0030-el0-irq-capability.md)                           | K1 remainder: EL0 `SYS_WAIT_IRQ` + IRQ notification caps (**accepted**)                                                        |
| [ADR-0031](adr/0031-k2-last-send-hold-auto-reap.md)                  | K2 entry: last SEND-hold auto-cancel on ephemeral channels (**accepted**)                                                      |
| [ADR-0032](adr/0032-k3-channel-revoke.md)                            | K3 entry: channel revoke + generation recycle (**accepted**)                                                                   |
| [ADR-0033](adr/0033-k10-supervisor-reap.md)                          | K10 entry: supervisor reaps blocked task; restart = re-spawn (**accepted**)                                                    |
| [ADR-0034](adr/0034-k9-rng-driver-agent.md)                          | K9 entry: RNG200 second driver-as-agent page map (**accepted**)                                                                |
| [ADR-0035](adr/0035-p5-name-registry.md)                             | P5 entry: EL1 name registry (**accepted**)                                                                                     |
| [ADR-0036](adr/0036-p2-keyed-blob-store.md)                          | P2 entry: EL1 keyed blob store (on-target put/get) (**accepted**)                                                              |
| [ADR-0037](adr/0037-k3-cap-transfer.md)                              | K3 residual: EL1 cap transfer (**accepted**)                                                                                   |
| [ADR-0038](adr/0038-k10-creator-exit-cascade.md)                     | K10 residual: creator-exit cascade cancel (**accepted**)                                                                       |
| [ADR-0039](adr/0039-p5-el0-resolve.md)                               | P5 residual: EL0 SYS_RESOLVE (**accepted**)                                                                                    |
| [ADR-0040](adr/0040-k2-park-timeout.md)                              | K2 residual: park timeout on ticks (**accepted**)                                                                              |
| [ADR-0041](adr/0041-el0-cap-transfer.md)                             | K3 residual: EL0 SYS_TRANSFER (**accepted**)                                                                                   |
| [ADR-0042](adr/0042-el0-recv-timeout.md)                             | K2 residual: EL0 SYS_RECV_TIMEOUT (**accepted**)                                                                               |
| [ADR-0043](adr/0043-k9-irq-device-agent.md)                          | K9 residual: IRQ-cap device agent (**accepted**)                                                                               |
| [ADR-0044](adr/0044-k5-agent-density.md)                             | K5: thin stacks (**accepted**)                                                                                                 |
| [ADR-0045](adr/0045-p2-durable-store.md)                             | P2 durable region (**accepted**)                                                                                               |
| [ADR-0046](adr/0046-k4-cooperative-cpu-budget.md)                    | K4 cooperative budget (**accepted**)                                                                                           |
| [ADR-0047](adr/0047-k7-asid-isolation-design.md)                     | K7 ASID design (**accepted**)                                                                                                  |
| [ADR-0050](adr/0050-k7-asid-first-slice.md)                          | K7 first slice — ASID pool + CONTEXTIDR (**accepted**)                                                                         |
| [ADR-0048](adr/0048-k8-smp-design.md)                                | K8 SMP design (**accepted**) — code deferred                                                                                   |
| [ADR-0049](adr/0049-deferred-residuals.md)                           | Deferred residuals policy (**accepted**)                                                                                       |
| [ADR-0051](adr/0051-k4-irq-preemption-design.md)                     | K4 IRQ preemption design (**accepted**) — code deferred                                                                        |
| [ADR-0052](adr/0052-p5-resolve-grant.md)                             | P5 resolve grant (**accepted**)                                                                                                |
| [ADR-0053](adr/0053-k3-peer-transfer-design.md)                      | K3 peer transfer design (**accepted**)                                                                                         |
| [ADR-0054](adr/0054-k3-peer-transfer-first-slice.md)                 | K3 peer transfer first slice (**accepted**)                                                                                    |
| [ADR-0055](adr/0055-transferable-cap-bands.md)                       | Transferable capability bands (**accepted**)                                                                                   |
| [ADR-0056](adr/0056-ipc-abi-capacities.md)                           | IPC ABI capacities (**accepted**)                                                                                              |
| [ADR-0057](adr/0057-taskcap-lifecycle.md)                            | Task-cap lifecycle invariants (**accepted**)                                                                                   |
| [ADR-0058](adr/0058-adr-amendments-and-mutation-freshness.md)        | ADR amendments + mutation freshness (**accepted**)                                                                             |
| [ADR-0059](adr/0059-typed-cap-classification.md)                     | Typed cap classification (**accepted**)                                                                                        |
| [ADR-0060](adr/0060-syscall-reply-layer.md)                          | Syscall reply layer (**accepted**)                                                                                             |
| [ADR-0061](adr/0061-refusal-detail-taxonomy.md)                      | Refusal detail in x1 (**accepted**)                                                                                            |
| [ADR-0062](adr/0062-taskid-epoch.md)                                 | Epoch in the task identity (**accepted**)                                                                                      |
| [ADR-0063](adr/0063-capslots-extraction.md)                          | Capability slots as a pure table (**accepted**)                                                                                |
| [ADR-0064](adr/0064-k4-el0-preemption-first-slice.md)                | K4 first code slice — EL0 IRQ preemption (**accepted**)                                                                        |
| [ADR-0065](adr/0065-platform-self-check.md)                          | Platform self-check — CPU identity decoded, printed, asserted at boot (**accepted**)                                           |
| [ADR-0066](adr/0066-sd-media-durable-store.md)                       | P2 — SD media persistence for the durable store (**accepted**)                                                                 |
| [ADR-0067](adr/0067-host-lab-second-isa-intent.md)                   | Host/lab second ISA — QEMU x86 intent; code deferred (**accepted**)                                                            |
| [ADR-0068](adr/0068-k4-el1-preemption-second-slice.md)               | K4 second code slice — same-EL (EL1) IRQ preemption (**accepted**)                                                             |
| [ADR-0069](adr/0069-harbor-host-class-north-star.md)                 | Host-class north star — native primary OS intent (**accepted**)                                                                |
| [ADR-0070](adr/0070-k8-smp-first-slice.md)                           | K8 first slice — unpark core 1, idle only (**accepted**)                                                                       |
| [`docs/reviews/`](reviews/)                                          | Pass outcomes (findings), not decisions                                                                                        |

## Non-goals

Permanent **out of model** only (see [Completeness roadmap](#completeness-roadmap)
for K/P tracks):

- Linux / POSIX compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
- Multi-tenant cloud hypervisor (unless a dedicated ADR owns it)
