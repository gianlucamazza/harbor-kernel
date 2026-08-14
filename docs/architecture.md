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
| Device drivers        | In-kernel, or servers with broad maps                 | **Driver-as-agent** with page-sized MMIO windows named by **vocabulary index**, never by a physical address on the wire ([ADR-0013](adr/0013-narrow-device-windows.md), [ADR-0100](adr/0100-device-windows.md); first composed instance [ADR-0101](adr/0101-composed-driver-agent.md))               |
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
of model.** SMP first depth is **done (HW)** through steal (unpark
[ADR-0070](adr/0070-k8-smp-first-slice.md), IPI [ADR-0074](adr/0074-k8-ipi-wake-second-slice.md),
dual-current queues [ADR-0076](adr/0076-k8-per-core-queues-first-slice.md)/[0077](adr/0077-smp-shared-state-discipline.md),
per-core timer + EL1/EL0 on CPU 1 [ADR-0079](adr/0079-k8-per-core-timer-preemption-first-slice.md)/[0081](adr/0081-k8-el0-on-cpu1-first-slice.md),
steal [ADR-0083](adr/0083-k8-work-stealing-first-slice.md); stamps 2026-08-10).
**Product SMP policy** ([ADR-0088](adr/0088-product-home-cpu.md)): each store /
manifest entry may name sticky **`home_cpu`** (default **0**); the default
product pack pins chirp on CPU 1 for dual-current evidence. Steal remains
opt-in for EL1 workers without a live agent AS; agents are not auto-balanced
(agent+TLB residual). Shared kernel tables are **`Mutex<T>`** — IRQ mask + spin,
with the datum inside the lock ([ADR-0077](adr/0077-smp-shared-state-discipline.md),
[ADR-0091](adr/0091-data-in-lock.md); including the loader). Fairness under a hostile busy-loop is enforced at both
ELs by the IRQ epilogue on each scheduled core. Details:
[`SECURITY.md`](../SECURITY.md). Multi-role inventory:
[post-K8 review](reviews/2026-08-10-post-k8-multi-role.md).
The payoff of the shape is that each boundary is named, gated, and
demonstrable rather than implied by a large ABI.

## Layering

Scale axes (where new code lands):
[`design/project-topology.md`](design/project-topology.md). Lab maturity path is
`src/lab/` — not a stubbed product tree.

```
┌──────────────────────────────────────────────────────────┐
│  Agents (M5–M7 + loader + waiting recv — all done HW)    │
│  message passing · capability-mediated resources         │
└────────────────────────────▲─────────────────────────────┘
                             │ SVC / IPC / EL0 IRQ (session)
┌────────────────────────────┴─────────────────────────────┐
│  Kernel policy (product path)                            │
│  bootstrap (+ discover) · loader · console_loop · sched  │
│  ipc · time · console · agent · mm (frames, aspace)      │
│  taskcap · naming · storage · durable · panic            │
│  lab/  (lab path only — thin bring-up, not Pi stubs)     │
└───────────▲─────────────────────────────▲────────────────┘
            │ register / handle           │
┌───────────┴───────────┐     ┌───────────┴────────────────┐
│  irq                  │     │  drivers                   │
│  dispatch · IrqChip   │     │  gicv2 · pl011 · rng200    │
│  fn(IrqCookie)        │     │  sdhci · pm · uart16550    │
│                       │     │  delay                     │
└───────────▲───────────┘     └───────────▲────────────────┘
            │ claim/eoi                   │
┌───────────┴───────────┐     ┌───────────┴────────────────┐
│  arch/exception       │     │  arch/{cpu,timer,mmu,cache,│
│  VBAR · frame · el0   │     │   switch,mmio,probe,smp,   │
│                       │     │   bootinfo}                │
└───────────────────────┘     └────────────────────────────┘
            ▲                              ▲
            │         bsp/rpi4             │
            └─ memmap · console · irq · gpio ─┘
               pm · rng · sdhci

┌──────────────────────────────────────────────────────────┐
│  crates/kernel-core — pure, host-tested, no MMIO         │
│  encodings · arithmetic · decode · bounded models        │
│  called by every layer above; calls none of them         │
└──────────────────────────────────────────────────────────┘
```

The eleven `arch/` modules are the port surface, and the list is not
decorative: [`arch-contract.md`](arch-contract.md) is what a port is checked
against, and `make doc-claims` compares that table against the facade's
re-export list so the two cannot drift apart. `kernel-core` sits outside the
stack rather than under it — it is the only layer with no hardware at all,
which is what makes it the place where a decision can be tested exhaustively
on the host.

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
   — never `static mut`. State that is _not_ producer/consumer lives **inside**
   a [`sync::Mutex<T>`](../src/sync.rs): IRQ mask, spin, mutate, release,
   restore — with the datum owned by the lock rather than declared beside it
   ([ADR-0091](adr/0091-data-in-lock.md)). A lock next to a cell cannot be
   written, because there is no lock type to write it with.

   `SyncCell` survives as a **closed residual of three statics**, each with the
   reason a guard cannot serve it: `irq::STATE` (read from the dispatch path,
   where a handler must never take a lock — ADR-0008) and the loader's
   `NAME_POOL` / `STORE_ENTRIES` (they mint `&'static` out of their interior,
   which no guard can lend). A fourth user needs an ADR.

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

   **The scheduler switch is the one exception, and it is fenced.** It must
   release the lock _before_ `context_switch` while `DAIF` stays masked across
   the stack swap, which a closure-scoped section cannot express. That is
   `Mutex::lock_masked`, returning a `MaskedGuard` that releases on drop and
   never touches `DAIF` — so it adds no masked region for the walker to miss.
   The same gate refuses `lock_masked` outside `src/sched/mod.rs`, which is what
   keeps the exception one file wide rather than a capability every caller has
   ([ADR-0091](adr/0091-data-in-lock.md) §3).

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
   unreachable without the oracle (83 as of 2026-08-10; the number is
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

11. **Read thin, decode pure.** A hardware read is one instruction and no
    logic; the interpretation of what it read is a total function in
    `kernel-core`, where the host can test it exhaustively. `cpu::midr_el1` →
    `kernel_core::cpuid`, `PM_RSTS` → `kernel_core::reset`, `ESR_EL1` →
    `kernel_core::fault`, sector 0 → `kernel_core::mbr`, the device tree →
    `kernel_core::fdt` + `hwdesc` (ADR-0065, ADR-0072). The payoff is that the
    part which is easy to get subtly wrong — a bit field, a fault-status
    table, a cells-encoded `reg` — is the part that never needed hardware to
    check, and the part that needs hardware stays too small to hide a bug.

    No gate decides this: "is this logic pure?" is not derivable from an
    import graph, so rule 11 is review's job. Naming it here rather than
    implying it is the same choice ADR-0016 made when it wrote `Nothing.` in a
    reversal row.

Rules 1, 3, 4 and 10 are checked by `make layering` (`scripts/check/layering.sh`)
against every `crate::` import edge (and ISA/board path leaks). Rule 7 has three
automated clauses, two of them in `make irq-scope`: no switching call inside a
masked region, and no `lock_masked` outside the scheduler; `make no-static-mut`
is the third. Rule 2's
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

### Boot phases

`bootstrap::run` is a list of named calls, one per phase, in the order they must
happen ([ADR-0095](adr/0095-boot-phases.md)). The names and the order are the
contract; the function's length is not, and is deliberately not quoted here.

**The seam is `console::install_tx`.** Above it a phase takes `&mut Pl011` and
prints with `println!(uart, …)`, because the handle is still exclusively owned;
below it the handle has _moved_ into kernel storage, so a phase takes nothing
and prints with `kprintln!`. Both endpoints — `console::acquire` and
`install_tx` — stay inline in `run` rather than inside a phase: they are the
reason the signatures change halfway down, and hiding either makes the second
half unreadable.

Before the seam, each taking the console handle:

| Phase                  | Does                                                                                                                                                                                           |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `print_banner`         | Says what this image is, on the wire, before anything can go wrong                                                                                                                             |
| `survey_firmware`      | Reads what firmware handed us while every physical address is still readable                                                                                                                   |
| `establish_kernel_map` | Heap and frame bounds → region table → `mmu::activate` → read `SCTLR_EL1.M` back. One function, because `kernel_regions` borrows a buffer the caller of `activate` must own. Returns `MemPlan` |
| `report_reset`         | Why the board came up, latched by silicon, before anything obscures it                                                                                                                         |
| `verify_cpu`           | Which core this is, against the core the kernel was built for (ADR-0065)                                                                                                                       |
| `unpark_secondary`     | ADR-0070: core 1 into an idle loop                                                                                                                                                             |
| `map_dtb_and_discover` | Map the blob back in — the kernel map covers less than the early one — and report what it says (ADR-0072/0073)                                                                                 |
| `assert_table_reserve` | Refuse to boot if the table arena is nearly spent, counted _after_ the DTB map                                                                                                                 |
| `init_memory_pools`    | Heap, frame pool, and `sched::init` — something must be the current task before anyone enters EL0 (ADR-0017 §1)                                                                                |
| `probe_rng`            | RNG200: one line, never fatal                                                                                                                                                                  |
| `bind_interrupts`      | GIC + timer PPI. A failed bind is not fatal; the console stays output-only                                                                                                                     |
| `seal_dispatch`        | Freeze the IRQ table so the dispatch path reads state nothing can mutate                                                                                                                       |
| `bring_up_cpu1`        | ADR-0074/0076: banked GICC, SGI wake, pinned marker. After the seal, because the secondary needs a handler present                                                                             |
| `enable_interrupts`    | Arm PL011 RX and unmask — both halves guarded on the bind having succeeded                                                                                                                     |

After the seam, printing through the shared TX:

| Phase                   | Does                                                                                                                                 |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `start_console_service` | Mint the console channel, spawn the resident EL1 server (M8)                                                                         |
| `loader::load_all`      | Agents that are data (ADR-0021), bound against the caps the loader holds                                                             |
| `demos::run_all`        | The oracle's spawns — behind `feature = "oracle"`, and in `demos.rs` so `product-builds` derives its forbidden symbols from one file |
| `report_boot`           | What the phases above cost, on the board's own clock                                                                                 |

`refuse_to_boot`, `exception::init`, `console_loop::heap_check` and
`console_loop::run` stay inline: they already delegate, and wrapping them would
add a name while removing ordering information.

## Interrupt / timer / console contract

| Role         | Module                                             | Responsibility                                                                     |
| ------------ | -------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Clocksource  | `arch/timer`                                       | CNTP deadline, ISTATUS, re-arm                                                     |
| Irqchip      | `drivers/gicv2`                                    | enable, claim/EOI, SPI target CPU0                                                 |
| Dispatch     | `irq` + `kernel_core::irqtable`                    | id → handler; seal freezes the table                                               |
| Tick policy  | `time`                                             | `on_timer_irq`, `ticks()`                                                          |
| Console RX   | `console`                                          | ring / `suspend_rx`·`resume_rx`; agent poll when owned                             |
| Bind         | `bsp/rpi4/irq`                                     | TIMER=30, UART=153, static GIC                                                     |
| Layout       | `mm/layout`                                        | regions and their permissions                                                      |
| Allocation   | `mm`                                               | free list + `GlobalAlloc`                                                          |
| Scheduler    | `sched`                                            | spawn / yield / exit + IRQ-epilogue quantum preemption                             |
| Task stacks  | `mm/task_stack`                                    | heap stack + unmapped guard                                                        |
| Discovery    | `bootstrap/discover` + `kernel_core::{fdt,hwdesc}` | firmware tree → reconcile with compiled claims → `discover:` lines (ADR-0072/0073) |
| Fault report | `arch/exception` + `kernel_core::fault` + `panic`  | syndrome decoded in words; the address named against the live map                  |

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
and an agent costs one of `MAX_TASKS` slots plus a kernel stack (default
**Full** 16 KiB; **Thin** 4 KiB + guard via [ADR-0044](adr/0044-k5-agent-density.md);
**Mini** 4 KiB **no guard** via [ADR-0086](adr/0086-k5-mini-stack-first-slice.md)
(**K5-S**, policy [ADR-0085](adr/0085-k5-density-residual-design.md))) on top of its address space,
however small its program. Collapsing or multiplexing the driver half is
**K5-H/B** (deferred; same ADR) — not “raise `MAX_TASKS`.”

[ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) records the
shape rather than changing it, because three things that look unrelated are the
same fact: preemption has to preempt the driver mid-session with a live
EL0 context; ADR-0018's _"the creator decides what happens to the task"_ means
the driver task, so an agent cannot be killed without killing its watcher; and
`MAX_TASKS` is scarce because every agent spends a slot on the loop that drives
it. Oracle demos have raised the ceiling over time (census tax); density work is
not “raise `MAX_TASKS` again.”

Where the distinction matters, this document says **driver task** or **EL0
program** rather than "agent".

### Capacity model (today)

Bounds are intentional under-/over-caps relative to each other — not one number.
Code is authoritative; this table is the map.

| Bound                        |                              Value (today) | Owns                      | Note                                                                                                                                                                                                       |
| ---------------------------- | -----------------------------------------: | ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sched::MAX_TASKS`           |                                     **57** | `src/sched/mod.rs`        | Includes dual idle + oracle census; three lifecycle oracle tasks can overlap the ADR-0090 force-kill/preemption window on fast runners (ADR-0031/0033); product peaks at **8** slots on QEMU (`entropy` refused — no RNG200) and **9** on a Pi 4B that runs the five-agent store (`slots=4/9` after agents exit; stamp 2026-08-14, `20260814-113438.log`), measured from the shipped image's `slots=` field (ADR-0098 / ADR-0102 / ADR-0103); **do not raise for density** (ADR-0085) |
| Stack classes                | Full 20 KiB · Thin 8 KiB · Mini 4 KiB heap | `kernel_core::density`    | Mini = one page, no unmapped guard (ADR-0086)                                                                                                                                                              |
| Caps per task                |                                      **4** | `sched` / manifest        | Slot ABI width                                                                                                                                                                                             |
| Task-caps (system)           |                                     **32** | `kernel_core::taskcap`    | Deliberately &lt; `MAX_TASKS`                                                                                                                                                                              |
| Agent store entries          |                                      **8** | `kernel_core::agentstore` | Composition scale ≠ scheduler scale                                                                                                                                                                        |
| Manifest grant slots         |                                      **4** | `kernel_core::manifest`   | Per-entry grant table                                                                                                                                                                                      |
| Name registry                |                                      **8** | `kernel_core::naming`     | P5                                                                                                                                                                                                         |
| IRQ waiters / task-id bitmap |                                 8 / **64** | `kernel_core::irqwait`    | Bitmap covers `MAX_TASKS` (≤64)                                                                                                                                                                            |
| IRQ wake SPSC queue          |                                     **32** | `src/irq/wait.rs`         | Deliberately &lt; oracle `MAX_TASKS`; full = drop + count, not lost task                                                                                                                                   |
| Park timeouts armed          |                                     **16** | `kernel_core::parktime`   | Under full task census                                                                                                                                                                                     |
| Durable / storage blobs      |                                      **4** | storage/durable pure      | P2 first depth                                                                                                                                                                                             |

**Product SMP policy ([ADR-0088](adr/0088-product-home-cpu.md)):** each manifest /
store entry may name sticky **`home_cpu`** (`0 .. N_CPUS`). Absent / zero →
product default **CPU 0**. Default pack pins **chirp** on CPU 1 for dual-current
composition evidence; lab oracles may still `spawn_on` directly. Work stealing
pulls only **stealeable** Ready EL1 tasks (opt-in; agents with an AS mark
non-stealeable). Global tick advance for timeouts remains **CPU 0** (per-core
quantum is local).

### An agent is data

Since ADR-0021 an agent can be a **manifest entry** rather than a compiled-in
Rust `fn`: an image, a window geometry, and a slot table. `bootstrap::loader` is
one loop over `kernel_core::manifest`, and every entry gets the same
trampoline — one body, N descriptions. Which entry a task is running is the
loader's own side table, not a field in the TCB: the scheduler sits below
`agent` and `bootstrap`, and a manifest is a concept it has no business
knowing ([ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)).

The security argument is arithmetic, not a check. An entry's slot carries an
**index into a declared vocabulary** (`held` — [ADR-0099](adr/0099-composition-vocabulary.md)),
never a `CapId`, so there is nothing outside that list for a manifest to name.
A device page is the same shape: the store names a **window index**, and
`authority` is what that integer means ([ADR-0100](adr/0100-device-windows.md)).
`manifest::bind` is where the index becomes a capability or a mapping, and an
index past the end is a refusal that says which one it reached for. A
declared position nobody filled is a different refusal (`HeldVacant` /
`WindowVacant`) — a failed mint leaves a hole and does not shift later
indices.

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
[`roadmap.md`](roadmap.md). The SPI TFT status surface
([ADR-0009](adr/0009-optional-spi-tft-debug-console.md),
[ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md)) is recorded there too, as
the side-track it was — and it is now **retired**
([ADR-0094](adr/0094-retire-debug-display.md)): it compiled without ever being
executed, and the pure half survives in `kernel_core`.

<a id="completeness-roadmap"></a>

## Completeness roadmap

**Tables live in [`roadmap.md`](roadmap.md)** (single source of truth for K/P
status). Policy: [ADR-0026](adr/0026-kernel-and-product-completeness.md).

| Snapshot                     | Tracks                                                                                                                                                            |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **done (HW)** H1 depth stamp | 2026-08-08 serial — K5 thin, P2 durable, K4 budget, lifecycle residuals ([verification](verification.md#hardware-evidence-h1-depth-stamps-on-silicon-2026-08-08)) |
| **H1 next**                  | Pi4 GENET v5 backend implementation and hardware evidence for the completed QEMU P3 edge-gateway target ([ADR-0104](adr/0104-p3-edge-network-composition.md), [ADR-0105](adr/0105-pi4-nic-backend-boundary.md), proposed design [ADR-0106](adr/0106-pi4-genet-v5-backend-design.md)); the product prints a `genet:` FDT report, maps a compiled 64 KiB GENET window, probes `rev=6.0` on silicon, and after a successful probe prints PHY-identify and BMSR link lines, then programs and enables queue 0 and may print one bounded TX report (a down BMSR refuses before the doorbell); it keeps the network vocabulary vacant; a general Pi4 oracle boot baseline is stamped in verification; P2 durable endpoint **done (HW)** in [ADR-0103](adr/0103-p2-el0-durable-endpoint.md) |
| **H2 depth**                 | K4+K7-ASID+K8+F-R1-P1+K5-S done (HW); K7 residual ADR-0084; K5-H / K5-B **code** if trigger (0085/0089)                                                           |
| **open (kernel)**            | K5-H if trigger (0085); K5-B **design paid** (0089), code only if trigger; K7-M optional; K7-T if trigger (0084); optional agent steal+TLB                        |
| **open (product)**           | Pi4 GENET v5 backend implementation and NIC evidence for the QEMU-complete P3 target follows [ADR-0105](adr/0105-pi4-nic-backend-boundary.md) (the `genet:` FDT report is not that gate); P4 remains deferred (ADR-0049); denser composition (K5-H if slots bind); console server remains compiled-in EL1 infrastructure |
| **paid (HW) product SMP**    | `home_cpu` (0088); K10 force-exit (0090)                                                                                                                          |
| **paid (HW) composition**    | Declared `held` (0099); device windows (0100); first composed driver-agent `entropy` (0101); product name bind (0102); P2 EL0 durable endpoint (0103)                                                                       |
| **paid (hygiene)**           | SMP shared tables including loader (0077 amended 2026-08-11); slot meter measured (0098); multi-role F-R5-2 / F-R7-1                                             |

When a track changes status, edit **`roadmap.md` only** — do not re-list full
K/P tables here. Horizon mapping and working order also live in `roadmap.md`.

## Decisions and reviews

The choices that constrain the code have an ADR, each naming the alternative
that was rejected and the gate that would catch its reversal.

| Artefact                                                             | Role                                                                                                                                                                                         |
| -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`../SECURITY.md`](../SECURITY.md)                                   | Threat model and reporting (M7 authority surface; residuals named)                                                                                                                           |
| [`docs/adr/`](adr/README.md)                                         | Architecture Decision Records (lifecycle: proposed → accepted → superseded)                                                                                                                  |
| [ADR-0001](adr/0001-multi-role-analysis.md)                          | Multi-role analysis as pre-milestone gate (**accepted**)                                                                                                                                     |
| [ADR-0002](adr/0002-softfloat-kernel.md)                             | Kernel compiled softfloat, FP left trapping (**accepted**)                                                                                                                                   |
| [ADR-0003](adr/0003-early-mmu.md)                                    | MMU enabled before any Rust runs (**accepted**)                                                                                                                                              |
| [ADR-0004](adr/0004-gic-group0-firmware-pin.md)                      | GIC Group 0 with IAR/EOIR, and the firmware pin (**accepted**)                                                                                                                               |
| [ADR-0005](adr/0005-static-page-table-arena.md)                      | Static page-table arena instead of a frame allocator (**accepted**)                                                                                                                          |
| [ADR-0006](adr/0006-cooperative-execution-model.md)                  | Cooperative execution model (M3 tasks); closes F12 (**accepted**)                                                                                                                            |
| [ADR-0007](adr/0007-project-identity-harbor-kernel.md)               | Project identity Harbor / `harbor-kernel` (**accepted**)                                                                                                                                     |
| [ADR-0008](adr/0008-irq-handler-policy.md)                           | IRQ handler shape for M4 wakes / caps; closes F13 (**accepted**)                                                                                                                             |
| [ADR-0009](adr/0009-optional-spi-tft-debug-console.md)               | Optional SPI TFT status surface; lab side-track (**superseded**) — by 0094                                                                                                                   |
| [ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md)                | SPI sessions + DBI stream; regwidth-16 SKU note (**superseded**) — by 0094                                                                                                                   |
| [ADR-0011](adr/0011-dtb-mapped-board-constants-risk-accept.md)       | DTB mapped; board truth compiled-in; closes F15 (**accepted**)                                                                                                                               |
| [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md)           | Frame allocator for user AS; M5 needs-first (**accepted**)                                                                                                                                   |
| [ADR-0013](adr/0013-narrow-device-windows.md)                        | Narrow device MMIO for agents; F26/M6 v1 (**accepted**)                                                                                                                                      |
| [ADR-0014](adr/0014-ttbr-split-m5.md)                                | TTBR regime M5 v1 (TTBR0 + kernel maps in user AS) (**accepted**)                                                                                                                            |
| [ADR-0015](adr/0015-multi-arch-scaffold.md)                          | Multi-arch scaffold: cfg facade + board features (**accepted**)                                                                                                                              |
| [ADR-0016](adr/0016-el0-session-protocol.md)                         | EL0 session protocol: one slot, prose contract, named successor (**superseded**) — by 0017 and 0018                                                                                          |
| [ADR-0017](adr/0017-el0-capability-abi.md)                           | EL0 capability ABI: slot-indexed authority, session state in the TCB (**accepted**)                                                                                                          |
| [ADR-0018](adr/0018-agent-fault-policy.md)                           | Agent fault policy: the kernel ends the session, the creator decides the task (**accepted**)                                                                                                 |
| [ADR-0019](adr/0019-no-static-mut.md)                                | No `static mut`: the last one becomes an atomic, rule 7 without an exception (**accepted**)                                                                                                  |
| [ADR-0020](adr/0020-spidevice-contract-without-a-caller.md)          | `SpiDevice`: a contract with no caller (**superseded**) — by 0094, the trait went with the panel                                                                                             |
| [ADR-0021](adr/0021-agents-as-data-and-the-manifest.md)              | Agents as data described by a manifest; the grant becomes a binding, not code (**accepted**)                                                                                                 |
| [ADR-0022](adr/0022-blocking-recv-and-the-mask-that-travels.md)      | Blocking `SYS_RECV`: the agent parks; `without_irqs` stops spanning a switch (**accepted**)                                                                                                  |
| [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) | An agent is a **pair**: an EL1 driver task and the EL0 program it drives; the driver is what the scheduler runs (**accepted**)                                                               |
| [ADR-0024](adr/0024-parked-task-visibility.md)                       | Parked tasks are counted (`blocked_count` / `block_events`); reclaim/timeout deferred (**accepted**)                                                                                         |
| [ADR-0025](adr/0025-cancel-blocked-wait.md)                          | Supervisor `cancel_blocked` aborts a parked wait (`Cancelled`); no timeout queue (**accepted**)                                                                                              |
| [ADR-0026](adr/0026-kernel-and-product-completeness.md)              | Completeness of kernel (K) and product OS (P) is the project goal (**accepted**)                                                                                                             |
| [ADR-0027](adr/0027-h1-external-agent-store.md)                      | H1 entry: external agent store at fixed PA (**accepted**)                                                                                                                                    |
| [ADR-0028](adr/0028-wait-on-irq.md)                                  | K1 entry: EL1 wait on IRQ cookie (**accepted**)                                                                                                                                              |
| [ADR-0029](adr/0029-agent-store-in-image.md)                         | Agent store placement: image section inject (**accepted**)                                                                                                                                   |
| [ADR-0030](adr/0030-el0-irq-capability.md)                           | K1 remainder: EL0 `SYS_WAIT_IRQ` + IRQ notification caps (**accepted**)                                                                                                                      |
| [ADR-0031](adr/0031-k2-last-send-hold-auto-reap.md)                  | K2 entry: last SEND-hold auto-cancel on ephemeral channels (**accepted**)                                                                                                                    |
| [ADR-0032](adr/0032-k3-channel-revoke.md)                            | K3 entry: channel revoke + generation recycle (**accepted**)                                                                                                                                 |
| [ADR-0033](adr/0033-k10-supervisor-reap.md)                          | K10 entry: supervisor reaps blocked task; restart = re-spawn (**accepted**)                                                                                                                  |
| [ADR-0034](adr/0034-k9-rng-driver-agent.md)                          | K9 entry: RNG200 second driver-as-agent page map (**accepted**)                                                                                                                              |
| [ADR-0035](adr/0035-p5-name-registry.md)                             | P5 entry: EL1 name registry (**accepted**)                                                                                                                                                   |
| [ADR-0036](adr/0036-p2-keyed-blob-store.md)                          | P2 entry: EL1 keyed blob store (on-target put/get) (**accepted**)                                                                                                                            |
| [ADR-0037](adr/0037-k3-cap-transfer.md)                              | K3 residual: EL1 cap transfer (**accepted**)                                                                                                                                                 |
| [ADR-0038](adr/0038-k10-creator-exit-cascade.md)                     | K10 residual: creator-exit cascade cancel (**accepted**)                                                                                                                                     |
| [ADR-0039](adr/0039-p5-el0-resolve.md)                               | P5 residual: EL0 SYS_RESOLVE (**accepted**)                                                                                                                                                  |
| [ADR-0040](adr/0040-k2-park-timeout.md)                              | K2 residual: park timeout on ticks (**accepted**)                                                                                                                                            |
| [ADR-0041](adr/0041-el0-cap-transfer.md)                             | K3 residual: EL0 SYS_TRANSFER (**accepted**)                                                                                                                                                 |
| [ADR-0042](adr/0042-el0-recv-timeout.md)                             | K2 residual: EL0 SYS_RECV_TIMEOUT (**accepted**)                                                                                                                                             |
| [ADR-0043](adr/0043-k9-irq-device-agent.md)                          | K9 residual: IRQ-cap device agent (**accepted**)                                                                                                                                             |
| [ADR-0044](adr/0044-k5-agent-density.md)                             | K5: thin stacks (**accepted**)                                                                                                                                                               |
| [ADR-0045](adr/0045-p2-durable-store.md)                             | P2 durable region (**accepted**)                                                                                                                                                             |
| [ADR-0046](adr/0046-k4-cooperative-cpu-budget.md)                    | K4 cooperative budget (**accepted**)                                                                                                                                                         |
| [ADR-0047](adr/0047-k7-asid-isolation-design.md)                     | K7 ASID design (**accepted**)                                                                                                                                                                |
| [ADR-0050](adr/0050-k7-asid-first-slice.md)                          | K7 first slice — ASID pool + CONTEXTIDR (**accepted**); done (HW)                                                                                                                            |
| [ADR-0084](adr/0084-k7-residual-policy.md)                           | K7 residual policy — measure / TTBR1 triggers / rollover (**accepted**)                                                                                                                      |
| [ADR-0085](adr/0085-k5-density-residual-design.md)                   | K5 density residual — K5-S/H/B split; Mini first code (**accepted**)                                                                                                                         |
| [ADR-0086](adr/0086-k5-mini-stack-first-slice.md)                    | K5-S Mini stacks — one page, no unmapped guard (**accepted**)                                                                                                                                |
| [ADR-0087](adr/0087-oracle-waits-and-the-hosts-verdict.md)           | Oracle cross-core waits in guest time; a starved host earns no verdict (**accepted**)                                                                                                        |
| [ADR-0088](adr/0088-product-home-cpu.md)                             | Product multi-core — manifest `home_cpu` + loader pin (**accepted**); done (HW)                                                                                                              |
| [ADR-0089](adr/0089-k5-b-pair-collapse-design.md)                    | K5-B pair collapse design — session as schedulable; **no code** until trigger (**accepted**)                                                                                                 |
| [ADR-0090](adr/0090-k10-force-exit-running.md)                       | K10 force-exit Running at safe point (**accepted**); done (HW)                                                                                                                               |
| [ADR-0091](adr/0091-data-in-lock.md)                                 | Data in the lock — `Mutex<T>` owns its datum; `SyncCell` a closed residual (**accepted**); done (HW)                                                                                         |
| [ADR-0092](adr/0092-lifecycle-verdicts.md)                           | Supervisor lifecycle verdicts pure in `kernel-core` (**accepted**); done (HW)                                                                                                                |
| [ADR-0093](adr/0093-panic-path-positive-evidence.md)                 | Panic path positive evidence — deliberate guard-page fault (**accepted**); done (HW)                                                                                                         |
| [ADR-0094](adr/0094-retire-debug-display.md)                         | Retire `debug-display`; the panel returns with a composition (**accepted**)                                                                                                                  |
| [ADR-0095](adr/0095-boot-phases.md)                                  | Boot phases as named functions; console handover as the seam (**accepted**); done (HW)                                                                                                       |
| [ADR-0096](adr/0096-gates-that-do-not-depend-on-remembering.md)      | Mutation freshness, `hw-check`, and no CI skip (**accepted**); gates seen red                                                                                                                |
| [ADR-0097](adr/0097-loader-plan.md)                                  | Loader plan pure in `kernel-core`; last of ADR-0049's R1 extractions (**accepted**); done (HW)                                                                                               |
| [ADR-0098](adr/0098-slot-meter-measured.md)                          | Slot occupancy counted in `kernel_core::tasks`, printed by the product, read by `oracle-census` (**accepted**); done (HW)                                                                    |
| [ADR-0099](adr/0099-composition-vocabulary.md)                       | Declared `held` vocabulary in `kernel_core::held`; `bind` over `Option`; `HeldVacant`; `bootstrap::authority` (**accepted**); done (HW)                                                      |
| [ADR-0100](adr/0100-device-windows.md)                               | Device windows named by index, not by physical address; `held::Set<T>` generic; store carries `window` + `va` (**accepted**); done (HW)                                                    |
| [ADR-0101](adr/0101-composed-driver-agent.md)                        | First composed driver-agent: `entropy` arrives in the store holding the `rng` window and reads the device before it speaks; `absent` is not `FAILED` (**accepted**); done (HW) |
| [ADR-0102](adr/0102-product-binds-a-name.md)                         | Product binds `console` in the name registry; store `lookup` finds it by grant, not by slot (**accepted**); done (HW) |
| [ADR-0103](adr/0103-p2-el0-durable-endpoint.md)                      | P2 durable storage request/reply endpoint for EL0 agents (**accepted**); done (HW) |
| [ADR-0104](adr/0104-p3-edge-network-composition.md)                 | P3 edge-gateway composition over virtio-net (**accepted**) |
| [ADR-0105](adr/0105-pi4-nic-backend-boundary.md)                    | Pi 4 NIC backend boundary and evidence gate (**proposed**); a `genet:` FDT report is not this gate |
| [ADR-0106](adr/0106-pi4-genet-v5-backend-design.md)                 | Pi 4 BCM2711 GENET v5 backend design (**proposed**); product prints a `genet:` FDT report, not a NIC |
| [ADR-0048](adr/0048-k8-smp-design.md)                                | K8 SMP design (**accepted**); first code slice [ADR-0070](adr/0070-k8-smp-first-slice.md)                                                                                                    |
| [ADR-0049](adr/0049-deferred-residuals.md)                           | Deferred residuals policy (**accepted**)                                                                                                                                                     |
| [ADR-0051](adr/0051-k4-irq-preemption-design.md)                     | K4 IRQ preemption design (**accepted**); code [ADR-0064](adr/0064-k4-el0-preemption-first-slice.md)/[0068](adr/0068-k4-el1-preemption-second-slice.md)                                       |
| [ADR-0052](adr/0052-p5-resolve-grant.md)                             | P5 resolve grant (**accepted**)                                                                                                                                                              |
| [ADR-0053](adr/0053-k3-peer-transfer-design.md)                      | K3 peer transfer design (**accepted**)                                                                                                                                                       |
| [ADR-0054](adr/0054-k3-peer-transfer-first-slice.md)                 | K3 peer transfer first slice (**accepted**)                                                                                                                                                  |
| [ADR-0055](adr/0055-transferable-cap-bands.md)                       | Transferable capability bands (**accepted**)                                                                                                                                                 |
| [ADR-0056](adr/0056-ipc-abi-capacities.md)                           | IPC ABI capacities (**accepted**)                                                                                                                                                            |
| [ADR-0057](adr/0057-taskcap-lifecycle.md)                            | Task-cap lifecycle invariants (**accepted**)                                                                                                                                                 |
| [ADR-0058](adr/0058-adr-amendments-and-mutation-freshness.md)        | ADR amendments + mutation freshness (**accepted**)                                                                                                                                           |
| [ADR-0059](adr/0059-typed-cap-classification.md)                     | Typed cap classification (**accepted**)                                                                                                                                                      |
| [ADR-0060](adr/0060-syscall-reply-layer.md)                          | Syscall reply layer (**accepted**)                                                                                                                                                           |
| [ADR-0061](adr/0061-refusal-detail-taxonomy.md)                      | Refusal detail in x1 (**accepted**)                                                                                                                                                          |
| [ADR-0062](adr/0062-taskid-epoch.md)                                 | Epoch in the task identity (**accepted**)                                                                                                                                                    |
| [ADR-0063](adr/0063-capslots-extraction.md)                          | Capability slots as a pure table (**accepted**)                                                                                                                                              |
| [ADR-0064](adr/0064-k4-el0-preemption-first-slice.md)                | K4 first code slice — EL0 IRQ preemption (**accepted**)                                                                                                                                      |
| [ADR-0065](adr/0065-platform-self-check.md)                          | Platform self-check — CPU identity decoded, printed, asserted at boot (**accepted**)                                                                                                         |
| [ADR-0066](adr/0066-sd-media-durable-store.md)                       | P2 — SD media persistence for the durable store (**accepted**)                                                                                                                               |
| [ADR-0067](adr/0067-host-lab-second-isa-intent.md)                   | Host/lab second ISA — QEMU x86 intent (**accepted**); code [ADR-0071](adr/0071-h3-l0-x86-qemu-first-slice.md)                                                                                |
| [ADR-0068](adr/0068-k4-el1-preemption-second-slice.md)               | K4 second code slice — same-EL (EL1) IRQ preemption (**accepted**)                                                                                                                           |
| [ADR-0069](adr/0069-harbor-host-class-north-star.md)                 | Host-class north star — native primary OS intent (**accepted**)                                                                                                                              |
| [ADR-0070](adr/0070-k8-smp-first-slice.md)                           | K8 first slice — unpark core 1, idle only (**accepted**); done (HW), stamp 2026-08-09                                                                                                        |
| [ADR-0074](adr/0074-k8-ipi-wake-second-slice.md)                     | K8 second slice — SGI IPI wake core 1 (**accepted**); done (HW), stamp 2026-08-10                                                                                                            |
| [ADR-0075](adr/0075-k8-per-core-queues-design.md)                    | K8 design — per-core queues / multi-current (**accepted**); code [0076](adr/0076-k8-per-core-queues-first-slice.md)                                                                          |
| [ADR-0076](adr/0076-k8-per-core-queues-first-slice.md)               | K8 third slice — dual current + pinned CPU1 worker (**accepted**); done (HW), stamp 2026-08-10                                                                                               |
| [ADR-0077](adr/0077-smp-shared-state-discipline.md)                  | SMP shared-state discipline — mask+spin sections, per-CPU mirrors (**accepted**); HW with 0076 stamp; mechanism restated by [ADR-0091](adr/0091-data-in-lock.md)                             |
| [ADR-0078](adr/0078-k8-per-core-timer-preemption-design.md)          | K8 design — per-core timer + preemption on CPU 1 (**accepted**); code [0079](adr/0079-k8-per-core-timer-preemption-first-slice.md)                                                           |
| [ADR-0079](adr/0079-k8-per-core-timer-preemption-first-slice.md)     | K8 fourth slice — per-core timer + EL1 preempt on CPU 1 (**accepted**); done (HW), stamp 2026-08-10                                                                                          |
| [ADR-0080](adr/0080-k8-el0-on-cpu1-design.md)                        | K8 design — EL0 sessions/agents home on CPU 1 (**accepted**); code [0081](adr/0081-k8-el0-on-cpu1-first-slice.md)                                                                            |
| [ADR-0081](adr/0081-k8-el0-on-cpu1-first-slice.md)                   | K8 fifth slice — EL0 on CPU 1 (**accepted**); done (HW), stamp 2026-08-10                                                                                                                    |
| [ADR-0082](adr/0082-k8-work-stealing-design.md)                      | K8 design — work stealing (**accepted**); code [0083](adr/0083-k8-work-stealing-first-slice.md)                                                                                              |
| [ADR-0083](adr/0083-k8-work-stealing-first-slice.md)                 | K8 sixth slice — work stealing first code (**accepted**); done (HW), stamp 2026-08-10                                                                                                        |
| [ADR-0071](adr/0071-h3-l0-x86-qemu-first-slice.md)                   | H3 L0 — x86_64 QEMU first slice (**accepted**); done (QEMU-x86)                                                                                                                              |
| [ADR-0072](adr/0072-hardware-self-discovery-design.md)               | Hardware self-discovery as boot evidence — verify, don't select (**accepted**); first code [ADR-0073](adr/0073-discovery-first-slice-fdt-report.md)                                          |
| [ADR-0073](adr/0073-discovery-first-slice-fdt-report.md)             | Discovery first slice — FDT reader + `discover:` report (**accepted**); done (HW), stamp 2026-08-10                                                                                          |
| [`docs/reviews/`](reviews/)                                          | Pass outcomes (findings), not decisions                                                                                                                                                      |

## Non-goals

Permanent **out of model** only (see [Completeness roadmap](#completeness-roadmap)
for K/P tracks):

- Linux / POSIX compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
- Multi-tenant cloud hypervisor (unless a dedicated ADR owns it)
