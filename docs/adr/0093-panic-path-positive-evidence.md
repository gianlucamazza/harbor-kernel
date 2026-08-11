---
id: 0093
title: Positive evidence for the panic path — one deliberate fault, one boot
status: accepted
date: 2026-08-11
accepted: 2026-08-11
related: [0005, 0006, 0049, 0058, 0087]
---

# ADR-0093: Positive evidence for the panic path

## Acceptance status

**Accepted** (2026-08-11), on delegated authority (structural improvement plan
approved by the owner on 2026-08-11; owner delegated acceptance for the slices
that plan names).

## Problem

`src/panic.rs` has **negative evidence only**, and has had since the excellence
review named it F-24: every gate asserts that no boot printed `PANIC`. Nothing
asserts that the path _works_ when it is needed. The pieces it depends on —
`exception::record_fault` publishing the syndrome, `last_fault` still holding
_this_ fault when policy reads it, `console::steal` reprogramming a UART from an
arbitrary context, the `PANICKING` re-entry guard, `cpu::halt` actually stopping
— are exercised by nothing.

That is the worst shape for a diagnostic: the code that runs when everything
else has failed is the code with no evidence. [ADR-0049](0049-deferred-residuals.md)
deferred it for want of a boot flavour, which is a reason to build one, not a
reason to keep deferring.

## Decision

A `panic-probe` image: **one feature, one boot, one deliberate fault, no
knobs.**

### 1. The fault: a task-stack guard page

The probe allocates a task stack with `TaskStack::allocate` — the same call
every spawn makes — announces its guard-page address, and writes to it. Chosen over the alternatives because it proves two things at once:

- it reaches the branch with the most policy in it — `AddressNote::UnmappedInside("heap")`
  and the "task-stack guard page, i.e. stack overflow" wording (`src/panic.rs`);
- it is the first **positive** evidence that the ADR-0005 guard page faults at
  all. Every other gate observes only that nothing has fallen into it.

**One case, deliberately.** The six branches of `report_faulting_address` are
selected by `kernel_core::layout::describe_address`, which is pure, host-tested
and already inside the mutation scope. Branch coverage is not what is missing —
the **wiring** is. A second case would add boot flavours without adding
evidence.

### 2. The probe announces before it faults

The probe prints `panic-probe: stack guard at 0x<va>, writing`
_before_ the store. Without that line, "the kernel did not panic" and "the probe
never ran" produce the same log, and a gate that cannot tell those apart is a
gate that passes when the probe silently stops being built.

### 3. A separate runner, because the oracle refuses panics

`scripts/lib/boot-oracle.sh` fails any log containing `PANIC` — correctly, and
that must not be relaxed. `scripts/boot/qemu-panic-boot-check.sh` is its own
runner with its own assertions, modelled on `qemu-product-boot-check.sh`
(layered assertions, ceiling-not-duration per [ADR-0087](0087-oracle-waits-and-the-hosts-verdict.md),
done when `*** halt ***` appears).

Assertions, each naming the layer it proves:

| #     | Assertion                                                                        | Proves                                                                                       |
| ----- | -------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| 1–2   | `Harbor: hello`, `MMU on`                                                        | the variant is still a real boot, and the region map exists so `describe_address` can answer |
| 3     | `panic-probe: stack guard at 0x…, writing`                              | the probe ran, and said **where**                                                            |
| 4     | `*** KERNEL PANIC ***`                                                           | the handler was reached — the positive evidence                                              |
| 5     | `sync exception EL1: …`                                                          | the panic came from a trap, not from any `panic!`                                            |
| 6     | `ESR=…` / `FAR=…`                                                                | the syndrome was captured and formatted                                                      |
| **7** | **the `FAR` in the panic == the address announced at 3**                         | `record_fault` → `last_fault` is reporting _this_ fault, not a stale one                     |
| 8     | `fault: 0x… unmapped inside "heap" — task-stack guard page, i.e. stack overflow` | the whole chain, wired                                                                       |
| 9     | `*** halt ***`                                                                   | the diagnostic completed rather than truncating                                              |
| 10    | `KERNEL PANIC` appears exactly once                                              | the `PANICKING` guard held                                                                   |
| 11    | `Harbor: hello` appears exactly once                                             | `cpu::halt` stops; no reset loop                                                             |
| 12    | nothing after `*** halt ***`                                                     | the core is parked                                                                           |

Assertion 7 is what separates this gate from `grep PANIC`: it checks that the
number the panic path _prints_ is the number the probe _wrote_.

### 4. It cannot leak into a product image

`scripts/boot/product-image.sh` already refuses demo symbols; it gains
`panic_probe`. `qemu-product-boot-check.sh`'s `oracle_leaks` list gains
`panic-probe:`. The feature is off by default and outside `oracle`.

## Alternatives rejected

- **Calling `panic!()` directly from a probe.** Cheaper, and it would prove the
  handler prints — but not `record_fault`, not `last_fault`, not the address
  naming, which is the half with the policy. The trap is the point.
- **A knob (`PANIC_CASE=guard|wild|…`).** Every case needs its own boot anyway;
  a knob buys configurability nobody asked for and a gate whose meaning depends
  on an environment variable.
- **Leaving it deferred.** The deferral was for want of an image flavour, and
  building one costs a feature and a script.

## Gates

| Check                                  | Evidence                                                     |
| -------------------------------------- | ------------------------------------------------------------ |
| The panic path has positive evidence   | `make panic-check`, twelve assertions above, in `make check` |
| The probe cannot silently stop running | assertion 3 fails if the announce line is missing            |
| The syndrome is this fault's           | assertion 7 compares announced VA with printed `FAR`         |
| It never reaches a product image       | `product-image.sh` symbol refusal + `oracle_leaks`           |

Closes the **Panic-path oracle** row of ADR-0049.
