# Architecture facade contract

What every ISA module must expose through `crate::arch` so policy, `mm`,
`sched`, `irq`, and drivers can stay ISA-agnostic at the import level.

**Selection:** `src/arch/mod.rs` chooses the implementation with
`#[cfg(target_arch = "…")]` and re-exports these modules. Unsupported
`target_arch` → `compile_error!`.

**Enforcement:** outside `src/arch/`, never import `crate::arch::<isa>`
(`make layering`, ADR-0015).

**Product support today:** AArch64 + Raspberry Pi 4.  
**Lab support:** x86_64 + QEMU q35 L0 ([ADR-0071](adr/0071-h3-l0-x86-qemu-first-slice.md))
— progressive fill of the same roles; see
[progressive-isa-practices.md](design/progressive-isa-practices.md).

## Required modules

| Module      | Role                                                                  | Policy / callers rely on (indicative)                                                                                                                                                                                                                                              |
| ----------- | --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cpu`       | IRQ mask, idle, halt, barriers, CPU identity                          | `without_irqs`, `irq_save`/`irq_restore`, `wait_for_interrupt`, `halt`, `sync_pipeline`; ID-register readers for the ADR-0065 platform self-check (`midr_el1`, `id_aa64mmfr0_el1`, `id_aa64pfr0_el1` — an ISA port supplies its equivalents, decode stays in `kernel_core::cpuid`) |
| `timer`     | Clocksource deadline / tick IRQ                                       | arm absolute deadline, status, re-arm; used by `time` and BSP IRQ bind                                                                                                                                                                                                             |
| `mmu`       | Kernel map activate, map/unmap, root phys, TTBR switch                | `activate`, `map`, `unmap`, `kernel_root_phys`, `switch_ttbr0` (name may generalize later)                                                                                                                                                                                         |
| `cache`     | I/D cache and TLB maintenance after map changes                       | called from `mmu` / `mm`                                                                                                                                                                                                                                                           |
| `switch`    | Cooperative context switch                                            | `Context`, `context_switch` (layout is ISA ABI)                                                                                                                                                                                                                                    |
| `exception` | Vectors + trap frame + init                                           | `init`, `TrapFrame`                                                                                                                                                                                                                                                                |
| `el0`       | User-mode session enter/resume/end, and the published session pointer | `enter`/`resume`/`end_session`/`run`, `El0Outcome`, entry IRQ mask policy, `El0Session` + `publish`/`published`, `saved_gpr`/`set_saved_gpr` (the syscall reply window)                                                                                                            |
| `mmio`      | Volatile MMIO accessor                                                | `Mmio` used by drivers and BSP                                                                                                                                                                                                                                                     |
| `probe`     | Deliberate external-abort recovery for soft presence                  | RNG and similar soft-fail paths                                                                                                                                                                                                                                                    |
| `bootinfo`  | Firmware handoff (e.g. DTB pointer)                                   | early map + optional consume; `device_tree_slice` serves the discovery report (ADR-0072/0073) once the blob is mapped, `None` where the boot protocol has no tree                                                                                                                  |
| `smp`       | Secondary core unpark / alive / IPI handshake (optional on uni-core)  | unpark + alive ([ADR-0070](adr/0070-k8-smp-first-slice.md)); IPI flags + release/wait ([ADR-0074](adr/0074-k8-ipi-wake-second-slice.md)); may be empty stubs on a single-core ISA port                                                                                             |

A port owes one more thing than the table shows: the session state a lower-EL
exception needs must be reachable **by linker symbol**, because the vector path
is entered by hardware and has no caller to pass a pointer to. On AArch64 that
is `CURRENT_EL0`, an `AtomicPtr` published by the scheduler on every switch
([ADR-0019](adr/0019-no-static-mut.md)). The type is the port's choice; the
obligation is that the scheduler can publish it, that the vector path can load
it without a caller, and that entering EL0 refuses a session the assembly would
not see ([ADR-0017](adr/0017-el0-capability-abi.md) §1).

## Required per-ISA artefacts (not re-exported as Rust modules)

| Artefact          | Location (AArch64)                     | Role                                                  |
| ----------------- | -------------------------------------- | ----------------------------------------------------- |
| Boot entry        | `src/arch/aarch64/boot.s`              | `_start` → EL1, early MMU, BSS, stack → `kernel_main` |
| Linker script     | `src/arch/aarch64/link.ld`             | load address, stacks, guard, table arena              |
| Exception vectors | `src/arch/aarch64/exception/vectors.s` | VBAR table                                            |

Boot assembly is included from the ISA `mod.rs` so `main` never names the ISA.

## Board contract (separate from ISA)

Boards are **not** part of `arch`. They implement `crate::bsp::board` via a
`board-*` feature:

| Surface   | Role                                                     |
| --------- | -------------------------------------------------------- |
| `memmap`  | Bases, sizes, IRQ ids, identity RAM end, user VA window  |
| `console` | Bind PL011 (or board UART) + GPIO pinmux                 |
| `irq`     | Bind irqchip + timer/UART lines into `irq`               |
| `rng`     | Optional SoC RNG bind                                    |
| `gpio`    | Pinmux for the console bind                               |
| `pm`      | Reset cause / watchdog block (`board::pm::reset_status`) |

Policy imports `crate::bsp::board`, never `crate::bsp::rpi4`.

## Role names with an AArch64 shape (kept until a rename ADR)

Do **not** rename preemptively (progressive-isa P.1):

- Module name `el0` and TTBR0-centric API names (`switch_ttbr0`, …) — x86 lab
  maps the **role** (user AS switch / session); L0 panics if called
- IRQ save token as `u64` (`cpu::irq_save`) — RFLAGS.IF on x86
- Softfloat / no FPEN (ADR-0002) is an **AArch64 product** choice; lab x86
  documents SIMD allowed (ADR-0071)
- Early-MMU exclusives story is Cortex-A72 Device-nGnRnE specific (ADR-0003)

Lab mapping table:
[`design/host-lab-platform-matrix.md`](design/host-lab-platform-matrix.md).
Progressive fill rules:
[`design/progressive-isa-practices.md`](design/progressive-isa-practices.md).

## Out of contract

- Driver protocols (PL011, GICv2, …) — may be reused if silicon matches
- Cooperative scheduler, IPC, agent shell — policy, not arch
- `kernel-core` pure logic — host-tested, ISA-agnostic maths
