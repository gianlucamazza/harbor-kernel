# Interrupts and exceptions (M1 + P0 idle/UART RX)

## Hardware evidence (Pi 4B Rev 1.5)

Re-verified after the move to an early MMU, which changed the memory regime the
polled gates run under — from no attributes to Normal WB with caches on:
`soft_ticks=3`, `gate: HPPIR=30 ok`, `inject: IAR=0x1e id=30`, `ticks 0 -> 2`,
`selftest: OK`. Build them with `--features bringup`; see
[`verification.md`](verification.md).

| Check                              | Result              |
| ---------------------------------- | ------------------- |
| PL011 console                      | OK                  |
| CNTP `ISTATUS` (IRQs masked)       | OK                  |
| GIC `HPPIR` = PPI 30 when pending  | OK                  |
| `GICC_IAR` claim + `EOIR`          | OK                  |
| Vector IRQ → `ticks=`              | **OK**              |
| UART0 SPI 153 + RX ring + WFI idle | **OK** (HW, 2026-08-04) |

### Gotchas fixed during bring-up

1. **No `AtomicU64` before MMU** — exclusive load/store can livelock without
   proper memory attributes. _Historical, M1 only:_ this held while the MMU was
   off. Since M2 the RAM is Normal WB Inner-Shareable and atomics are the
   required way to share state with the IRQ path (architecture rule 7). Do not
   re-derive plain counters from this note.
2. **Do not reprogram CNTP while the timer line is live on the GIC** without
   masking the PPI first (observed hang).
3. **Group 0 + `IAR`/`EOIR`** works for this firmware/GIC path; relying only on
   Group 1 aliased registers was unreliable during early bring-up.
4. Prefer **`HPPIR`** over a zero **`AHPPIR`** reading when both are present.
5. **SPI must set `ITARGETSR` to CPU0** — PPIs are banked; UART SPI 153 will
   not reach core 0 without a target bit.

## Dispatch table

The table lives in [`kernel_core::irqtable`](../crates/kernel-core/src/irqtable.rs),
host-tested; `src/irq` owns the chip, the interrupt mask, the counters an
exception context can reach, and the call itself.

**Sealing is the load-bearing part.** The table is mutable during bring-up and
frozen by `irq::seal()` afterwards, and that is the whole reason the IRQ path
may hold a shared `&'static` borrow while an interrupt arrives: after the seal
there is no writer left to race. It was a rule nothing checked — `seal()` set a
flag, `register()` read it, and nothing had ever registered a handler after
sealing to watch it refuse.

`register` returns `Result`, not `bool`, because the two refusals need
different fixes: an id past the table is a constant to correct, a sealed table
is a bring-up ordering bug. `bsp::rpi4::irq::BindError` carries which, and that
error is what a refusal to boot prints.

A claimed interrupt gets one of three answers, and the last two are counted
apart on purpose:

| Answer | Meaning | Counter |
| ------ | ------- | ------- |
| `Handle { handler, cookie }` | call it | — |
| `Unhandled` | in range, nobody registered | `irq: unhandled` |
| `OutOfRange` | beyond the table | out-of-range |

An in-range miss is a line someone enabled and forgot to claim; an id past the
table is a chip reporting something this kernel does not believe in. Collapsing
them would hide the second inside the first.

Bring-up prints `irq: sealed with N handlers registered` and `boot-check`
asserts it: a boot that registered nothing looks exactly like a healthy one
until the first interrupt nobody answers.

## Layering

```
exception_irq_el1
    → irq::handle_cpu_irq()
         chip.claim() → table.lookup(id) → handler(cookie) → chip.end()

exception_irq_el0        ← lower-EL IRQ during an unmasked EL0 session
    → save user context → El0Outcome::Irq
    → agent: irq::handle_cpu_irq() then el0::resume (re-execute)

time::on_timer_irq       ← TIMER_IRQ (PPI 30)
console::on_uart_rx_irq  ← UART_IRQ  (SPI 153) → RX ring only (when kernel owns drain)
arch/timer               ← CNTP only (no GIC)
drivers/gicv2            ← IrqChip (+ SPI target/level)
bsp/rpi4                 ← static GIC + bind
```

## GIC-400 (BCM2711)

| Block | Base          |
| ----- | ------------- |
| GICD  | `0xFF84_1000` |
| GICC  | `0xFF84_2000` |

Requires `enable_gic=1` in `config.txt`.

## Timer

ARM Generic Timer physical: `CNTP_*`, IRQ **PPI 30**, frequency from
`CNTFRQ_EL0` (~54 MHz on the tested board).

## UART0 (P0 + agent RX own)

| Item          | Value                                   |
| ------------- | --------------------------------------- |
| GIC id        | **SPI 153** (VC IRQ 57 + SPI base 96)   |
| PL011 sources | `RXIM` + `RTIM` (single-char with FIFO) when kernel owns RX |
| Handler       | `console::on_uart_rx_irq` → `ByteRing` (kernel drain) |
| Consumer      | idle (`pop_rx`) when drain live; EL0 poll of `DR` when agent owns RX |
| Agent own     | `suspend_rx` clears IMSC + base; LBE inject for self-test; `resume_rx` re-arms |

## Production bring-up

```
exception::init
bsp::irq::init(hz)       // timer + UART handlers, enable both lines
console::enable_rx_irq   // PL011 IMSC
timer::on_interrupt()    // clean deadline
irq_enable
idle console (drain ring, ticks, WFI)
```

Optional full gates: build with `--features bringup`. They are compiled out of a
production image entirely, along with the raw GIC accessors they need — there is
no longer a constant to flip.

## Console rule

| Context               | UART                                           |
| --------------------- | ---------------------------------------------- |
| bootstrap / main loop | exclusive `Pl011` **TX**; drain RX **ring**    |
| UART RX IRQ           | drain FIFO → ring only (**no** TX / `println`) |
| other IRQ handlers    | **no** UART                                    |
| panic                 | mask IRQ, `console::steal`, bounded TX         |
