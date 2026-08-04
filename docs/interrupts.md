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
| UART0 SPI 153 + RX ring + WFI idle | P0 (validate on HW) |

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

## Layering

```
exception_irq_el1
    → irq::handle_cpu_irq()
         chip.claim() → handlers[id]() → chip.end()

time::on_timer_irq       ← TIMER_IRQ (PPI 30)
console::on_uart_rx_irq  ← UART_IRQ  (SPI 153) → RX ring only
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

## UART0 (P0)

| Item          | Value                                   |
| ------------- | --------------------------------------- |
| GIC id        | **SPI 153** (VC IRQ 57 + SPI base 96)   |
| PL011 sources | `RXIM` + `RTIM` (single-char with FIFO) |
| Handler       | `console::on_uart_rx_irq` → `ByteRing`  |
| Consumer      | main / idle loop (`pop_rx`, TX poll)    |

## Production bring-up

```
exception::init
bsp::irq::init(hz)       // timer + UART handlers, enable both lines
console::enable_rx_irq   // PL011 IMSC
timer::on_interrupt()    // clean deadline
irq_enable
idle console (drain ring, ticks, WFI)
```

Optional full gates: set `BRINGUP_SELFTEST = true` in `bootstrap/mod.rs`.

## Console rule

| Context               | UART                                           |
| --------------------- | ---------------------------------------------- |
| bootstrap / main loop | exclusive `Pl011` **TX**; drain RX **ring**    |
| UART RX IRQ           | drain FIFO → ring only (**no** TX / `println`) |
| other IRQ handlers    | **no** UART                                    |
| panic                 | mask IRQ, re-acquire console                   |
