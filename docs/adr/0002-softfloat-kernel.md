---
id: 0002
title: Kernel compiled softfloat, FP left trapping
status: proposed
date: 2026-08-04
---

# ADR-0002: Kernel compiled softfloat, FP left trapping

## Context

The target was `aarch64-unknown-none`, which enables `+neon`. Nothing programmed
`CPACR_EL1.FPEN`, and its reset value is architecturally UNKNOWN: the image
worked only by virtue of what the firmware happened to leave behind.

This was not theoretical. Disassembling the image of the time:

```
0000000000082b40 <memset>:
   82b68: dup v0.4h, w1
```

`memset` contained SIMD, and it is reachable from `mm::alloc_zeroed`. With FPEN
at zero that instruction traps (ESR EC=0x07) and lands in the panic handler.

The cost does not stop there: `vectors.s` does not save `q0`–`q31`, so any use of
FP on the IRQ path would silently corrupt the interrupted code's state.

## Decision

Build for **`aarch64-unknown-none-softfloat`** and leave `CPACR_EL1.FPEN` at
zero — FP traps.

The compiler cannot emit FP/SIMD, so the problem is closed _by construction_
rather than handled at runtime. Leaving FPEN off is deliberate: a future FP
instruction that ends up there by mistake produces a diagnosable fault instead of
corrupting the trap frame.

This is what Linux (`-mgeneral-regs-only`), seL4 and Zephyr do, for the same
reason.

## Consequences

**Positive** — no FP state to save in the exception stubs; no dependency on the
`CPACR` the firmware leaves; the trap frame stays 272 bytes instead of 784.

**Negative** — no floating-point arithmetic in the kernel. Not a limit today:
there is none. `compiler_builtins` supplies `memset`/`memcpy` without SIMD, at a
cost on large copies that has not been measured and is not on the critical path.

## Alternatives considered

| Alternative                | Why not                                                                                                                                |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Enable `CPACR_EL1.FPEN`    | Makes every IRQ responsible for 32 128-bit registers: +512 bytes of trap frame and latency on every tick, for a kernel that does no FP |
| Save `q0`–`q31` in vectors | The same cost, paid always, for a benefit never used                                                                                   |
| Keep `+neon` and hope      | This is the state we came from, which survived on the firmware's good manners                                                          |

## The gate that protects this decision

`make no-simd` disassembles the linked image and fails if an FP/SIMD register
appears. **Seen red** on the pre-softfloat image (`dup v0.4h` in `memset`).

## When to revisit

When EL0 agents need FP (M5). The correct shape is _lazy FP switching_: FPEN off
by default, trap on first use per task, save `q0`–`q31` plus `FPCR`/`FPSR` only
for tasks that have touched it. **The kernel stays softfloat regardless.**

## References

`.cargo/config.toml`, `rust-toolchain.toml`, `src/arch/aarch64/cpu.rs` (the
comment on why FPEN stays off), [`verification.md`](../verification.md).
