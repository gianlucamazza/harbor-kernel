---
id: 0002
title: Kernel compiled softfloat, FP left trapping
status: proposed
date: 2026-08-04
---

# ADR-0002: Kernel compiled softfloat, FP left trapping

## Contesto

Il target era `aarch64-unknown-none`, che abilita `+neon`. `CPACR_EL1.FPEN` non
veniva programmato da nessuna parte, e il suo valore al reset è architetturalmente
UNKNOWN: l'immagine funzionava solo grazie a ciò che il firmware lasciava.

Non era teorico. Disassemblando l'immagine di allora:

```
0000000000082b40 <memset>:
   82b68: dup v0.4h, w1
```

`memset` conteneva SIMD ed è raggiungibile da `mm::alloc_zeroed`. Con FPEN a zero
quella istruzione trappa (ESR EC=0x07) e finisce nel panic handler.

Il costo non finisce lì: `vectors.s` non salva `q0`–`q31`, quindi qualunque uso di
FP nel path IRQ corromperebbe silenziosamente lo stato del codice interrotto.

## Decisione

Compilare per **`aarch64-unknown-none-softfloat`** e lasciare `CPACR_EL1.FPEN` a
zero, cioè FP che trappa.

Il compilatore non può emettere FP/SIMD, quindi il problema è chiuso *per
costruzione* e non gestito a runtime. Lasciare FPEN spento è deliberato: una
futura istruzione FP finita lì per sbaglio produce un fault diagnosticabile
invece di corrompere il trap frame.

È ciò che fanno Linux (`-mgeneral-regs-only`), seL4 e Zephyr, per la stessa ragione.

## Conseguenze

**Positive** — nessuno stato FP da salvare negli stub di eccezione; nessuna
dipendenza dal `CPACR` lasciato dal firmware; trap frame invariato a 272 byte
invece di 784.

**Negative** — nessuna aritmetica in virgola mobile nel kernel. Non è un limite
oggi: non ce n'è. `compiler_builtins` fornisce `memset`/`memcpy` senza SIMD, a un
costo su copie grandi che non abbiamo misurato e che non è sul path critico.

## Alternative considerate

| Alternativa | Perché no |
| --- | --- |
| Abilitare `CPACR_EL1.FPEN` | Rende ogni IRQ responsabile di 32 registri da 128 bit: +512 byte di trap frame e latenza su ogni tick, per un kernel che non fa FP |
| Salvare `q0`–`q31` nei vettori | Stesso costo, pagato sempre, per un beneficio mai usato |
| Tenere `+neon` e sperare | È lo stato da cui veniamo, sopravvissuto per fortuna del firmware |

## Gate che protegge questa decisione

`make no-simd` disassembla l'immagine linkata e fallisce se compare un registro
FP/SIMD. **Visto rosso** sull'immagine pre-softfloat (`dup v0.4h` in `memset`).

## Quando rivalutare

Quando gli agenti EL0 avranno bisogno di FP (M5). La forma corretta è il *lazy FP
switching*: FPEN spento di default, trap al primo uso per task, salvataggio di
`q0`–`q31` + `FPCR`/`FPSR` solo per i task che l'hanno toccata. **Il kernel resta
softfloat comunque.**

## Riferimenti

`.cargo/config.toml`, `rust-toolchain.toml`, `src/arch/aarch64/cpu.rs` (commento
sul perché FPEN resta spento), [`verification.md`](../verification.md).
