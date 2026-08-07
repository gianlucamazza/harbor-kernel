# Architecture facade contract

What every ISA module must expose through `crate::arch` so policy, `mm`,
`sched`, `irq`, and drivers can stay ISA-agnostic at the import level.

**Selection:** `src/arch/mod.rs` chooses the implementation with
`#[cfg(target_arch = "…")]` and re-exports these modules. Unsupported
`target_arch` → `compile_error!`.

**Enforcement:** outside `src/arch/`, never import `crate::arch::<isa>`
(`make layering`, ADR-0015).

**Product support today:** AArch64 only. Names below match the current surface;
a non-AArch64 port may keep the *roles* while renaming modules if needed (see
[Known AArch64 shape](#known-aarch64-shape-debt-until-a-real-port)).

## Required modules

| Module | Role | Policy / callers rely on (indicative) |
| ------ | ---- | ------------------------------------- |
| `cpu` | IRQ mask, idle, halt, barriers | `without_irqs`, `irq_save`/`irq_restore`, `wait_for_interrupt`, `halt`, `sync_pipeline` |
| `timer` | Clocksource deadline / tick IRQ | arm absolute deadline, status, re-arm; used by `time` and BSP IRQ bind |
| `mmu` | Kernel map activate, map/unmap, root phys, TTBR switch | `activate`, `map`, `unmap`, `kernel_root_phys`, `switch_ttbr0` (name may generalize later) |
| `cache` | I/D cache and TLB maintenance after map changes | called from `mmu` / `mm` |
| `switch` | Cooperative context switch | `Context`, `context_switch` (layout is ISA ABI) |
| `exception` | Vectors + trap frame + init | `init`, `TrapFrame` |
| `el0` | User-mode session enter/resume/end, and the published session pointer | `enter`/`resume`/`end_session`/`run`, `El0Outcome`, entry IRQ mask policy, `El0Session` + `publish`/`published`, `saved_gpr`/`set_saved_gpr` (the syscall reply window) |
| `mmio` | Volatile MMIO accessor | `Mmio` used by drivers and BSP |
| `probe` | Deliberate external-abort recovery for soft presence | RNG and similar soft-fail paths |
| `bootinfo` | Firmware handoff (e.g. DTB pointer) | early map + optional consume |

A port owes one more thing than the table shows: the session state a lower-EL
exception needs must be reachable **by linker symbol**, because the vector path
is entered by hardware and has no caller to pass a pointer to. On AArch64 that
is `CURRENT_EL0`, an `AtomicPtr` published by the scheduler on every switch
([ADR-0019](adr/0019-no-static-mut.md)). The type is the port's choice; the
obligation is that the scheduler can publish it, that the vector path can load
it without a caller, and that entering EL0 refuses a session the assembly would
not see ([ADR-0017](adr/0017-el0-capability-abi.md) §1).

## Required per-ISA artefacts (not re-exported as Rust modules)

| Artefact | Location (AArch64) | Role |
| -------- | ------------------- | ---- |
| Boot entry | `src/arch/aarch64/boot.s` | `_start` → EL1, early MMU, BSS, stack → `kernel_main` |
| Linker script | `src/arch/aarch64/link.ld` | load address, stacks, guard, table arena |
| Exception vectors | `src/arch/aarch64/exception/vectors.s` | VBAR table |

Boot assembly is included from the ISA `mod.rs` so `main` never names the ISA.

## Board contract (separate from ISA)

Boards are **not** part of `arch`. They implement `crate::bsp::board` via a
`board-*` feature:

| Surface | Role |
| ------- | ---- |
| `memmap` | Bases, sizes, IRQ ids, identity RAM end, user VA window |
| `console` | Bind PL011 (or board UART) + GPIO pinmux |
| `irq` | Bind irqchip + timer/UART lines into `irq` |
| `rng` | Optional SoC RNG bind |
| `display` | Optional (`debug-display`) |

Policy imports `crate::bsp::board`, never `crate::bsp::rpi4`.

## Known AArch64 shape (debt until a real port)

These are intentional until a second ISA exists; do **not** rename preemptively:

- Module name `el0` and TTBR0-centric APIs (`switch_ttbr0`, `ttbr0_phys` fields)
- DAIF-shaped IRQ save token (`u64` from `cpu::irq_save`)
- Softfloat / no FPEN (ADR-0002) is an AArch64 product choice
- Early-MMU exclusives story is Cortex-A72 Device-nGnRnE specific (ADR-0003)

A port documents its mapping of these roles in a successor note or ADR.

## Out of contract

- Driver protocols (PL011, GICv2, …) — may be reused if silicon matches
- Cooperative scheduler, IPC, agent shell — policy, not arch
- `kernel-core` pure logic — host-tested, ISA-agnostic maths
