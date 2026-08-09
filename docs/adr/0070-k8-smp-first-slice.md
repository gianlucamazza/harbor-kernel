---
id: 0070
title: K8 first slice — unpark core 1, idle only, smp: core1 alive
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0006, 0048, 0068]
---

# ADR-0070: K8 first slice (secondary alive)

## Acceptance status

**Accepted** (2026-08-09). Implements the first code slice of
[ADR-0048](0048-k8-smp-design.md): unpark **core 1** into an idle loop; no
per-core runqueue, no IPI scheduler, no work stealing.

## Context

ADR-0048 deferred SMP until after K4/K7 design use. K4 is **done (HW)**
(ADR-0064/0068); K7 first slice is **done (HW)**. Secondary cores still enter
`_start` and sit in `WFE` forever (`boot.s` `.L_park`). This slice wakes
**only** affinity 1, proves it runs kernel code with the shared page tables,
and idles — enough for a dual-core gate without rewriting `sched`.

QEMU `raspi4b` requires **min 4 CPUs** (`-smp 4`); cores 2–3 stay parked.

## Decision

### 1. Wake mechanism — three release paths

| Path | When | How |
| --- | --- | --- |
| **A. In-kernel table** | Real Pi (`start4.elf`): all cores enter `_start` | aff ≠ 0 loops on `secondary_entry[aff]`; primary stores `secondary_el2_entry` in slot 1 + `DSB` + `SEV` |
| **B. QEMU spin-table** | QEMU `raspi4b -kernel -smp 4` | Secondaries run QEMU's `write_smpboot64` stub, which polls PA **`0xd8 + 8×aff`**. Core 1 → write entry to **`0xe0`**, `DSB`, `SEV` |
| **C. ARM local mailbox** | Firmware mailbox poke | Write low 32 bits of entry to core1 mbox3 set (`0xFF80_009C`) |

Core 0 alone clears BSS, enables the early MMU, and runs `kernel_main`.  
QEMU does **not** run secondaries through our `_start` (`secondary_seen` stays 0); path B is required for the emulator gate. Real silicon uses path A (and C as belt-and-braces).

**Not used:** PSCI HVC/SMC — hangs or no-ops on this product path without EL3.

### 2. Secondary path (core 1 only in this slice)

1. `secondary_el2_entry` (asm): same EL2→EL1h drop as primary; install **core1**
   SP_EL1 / SP_EL0 from BSS stacks; branch to `secondary_main`.
2. `secondary_main` (Rust): `mmu::enable_existing(kernel_root)`; `exception::init`
   (same VBAR); set `CORE1_ALIVE`; `loop { wfe }` with IRQs **masked**.
3. No GIC init on core 1; no timer; no tasks; no console TX from core 1.

### 3. Stacks

Two BSS arrays (16 KiB exception + 16 KiB kernel) for core 1, covered by the
existing RW **data** map (`__data_start`…`__data_end` includes `.bss`). No
linker-script region change; no extra `mmu::map`.

### 4. When primary unparks

After kernel map **activate**, `exception::init`, and the `cpu:` line (so a
timeout still has a console). Before IRQs are enabled is preferred so core 1
never races an IRQ with an incomplete VBAR — unpark with DAIF still set on
primary is fine; core 1 keeps DAIF set forever in this slice.

### 5. Evidence

| Line | Meaning |
| --- | --- |
| `smp: core1 alive` | Core 1 set the alive flag within the spin budget |
| `smp: core1 timeout` | Fail the boot oracle (and refuse progress claims) |

Runners: QEMU `-smp 4` (required by `raspi4b`); HW Pi 4B (four cores; 2–3 parked).

### 6. Explicit non-goals (residuals)

- Per-core runqueue / `current` / work stealing  
- IPI wake and remote resched  
- Per-core timer / GIC secondary bring-up  
- Unparking cores 2–3  
- Cross-core preemption / K4 multi-core  
- Cache-coherent driver model beyond shared identity map  

## Consequences

### Positive

- Dual-core gate without scheduler rewrite  
- Honours ADR-0048 sequencing after K4  
- Thin oracle; HW stamp path clear  

### Negative / residual

- Full SMP still open (queues, IPI)  
- Alive-only core 1 does not prove IRQ affinity or TLB maintenance under load  

### Gates

| Reversal | Catch |
| --- | --- |
| No alive line | `boot-check` / `hw-transcript-check` |
| Unpark before MMU root published | Core 1 faults; timeout |
| Core 1 runs tasks | Out of scope — code review / no sched call |

## Related

- [0048](0048-k8-smp-design.md) — full SMP design  
- [0006](0006-cooperative-execution-model.md) — still single **schedulable** core for tasks  
- [0068](0068-k4-el1-preemption-second-slice.md) — preemption remains core-0 only here  
