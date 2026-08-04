---
id: 0004
title: GIC Group 0 with IAR/EOIR, and the firmware pin it depends on
status: proposed
date: 2026-08-04
---

# ADR-0004: GIC Group 0 con IAR/EOIR, e il pin del firmware da cui dipende

## Contesto

`drivers/gicv2.rs` programma le PPI in **Group 0** e claim/EOI via **`IAR`/`EOIR`**,
non tramite i registri aliasati di Group 1.

Questa scelta è **empirica**, non derivata dal manuale: durante il bring-up di M1,
`HPPIR` riportava la PPI 30 come pending ma la claim via Group 1 non faceva
avanzare i tick. Nella vista Non-Secure di GICv2 il bit 0 di `GICD_CTLR` è
`EnableGrp1`, quindi la sequenza che funziona dipende dallo stato in cui
`start4.elf` lascia il distributore.

È l'unica dipendenza dal firmware chiuso che il kernel ha nel path caldo, ed è
**passiva**: stato ereditato, non un protocollo. Le altre due della stessa natura
sono `CNTFRQ_EL0` (letto, non impostato) e il clock del PL011 (48 MHz assunti, con
`enable_uart=1` e `core_freq_min=500`).

## Decisione

Mantenere Group 0 + `IAR`/`EOIR`, e **legare esplicitamente questa scelta al pin del
firmware**: `firmware_tag=1.20250430`, con gli hash in `EXPECTED.sha256` verificati
prima dell'installazione.

Un bump del tag è una modifica deliberata che richiede di **rieseguire i gate di
bring-up su hardware**, perché una regressione qui non produce un errore: produce
un boot che arriva alla console e non stampa mai `ticks=`.

## Conseguenze

**Positive** — il path è verificato sul silicio con questo firmware (`HPPIR=30`,
`IAR=0x1e id=30`, `ticks 0 -> 2`, `selftest: OK`), non solo assunto.

**Negative** — il kernel non è portabile a un firmware arbitrario senza rivalidare.
Il vincolo è documentato in [`blobs.md`](../blobs.md) invece che implicito nel driver.

## Alternative considerate

| Alternativa | Perché no |
| --- | --- |
| Group 1 + `AIAR`/`AEOIR` | Provata durante M1: `HPPIR` vedeva la PPI, la claim non avanzava i tick |
| Riprogrammare il distributore da zero | Richiede di conoscere lo stato del lato secure, che non possediamo |
| Non pinnare il firmware | Renderebbe questa dipendenza invisibile finché non si rompe |

## Gate che protegge questa decisione

I gate `--features bringup` (`make bringup-builds` ne garantisce la compilazione;
l'esecuzione richiede hardware). Verificati su Pi 4B Rev 1.5 il 2026-08-04, dopo il
passaggio alla MMU precoce che ha cambiato il regime di memoria sotto di loro.

`scripts/fetch-blobs.sh` rifiuta blob i cui hash non corrispondono a quelli
committati — **visto rosso** corrompendo un hash atteso.

## Quando rivalutare

A ogni bump di `firmware_tag`, e su qualunque board il cui EEPROM lasci il GIC in
uno stato diverso. Il segnale di regressione è l'assenza di `ticks=`, non un errore.

## Riferimenti

`src/drivers/gicv2.rs`, `src/bootstrap/selftest.rs`, [`blobs.md`](../blobs.md),
[`interrupts.md`](../interrupts.md).
