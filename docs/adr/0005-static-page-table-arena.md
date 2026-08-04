---
id: 0005
title: Static page-table arena instead of a frame allocator
status: proposed
date: 2026-08-04
---

# ADR-0005: Arena statica per le tabelle invece di un frame allocator

## Contesto

`arch::mmu` alloca le tabelle di traduzione da un'arena di 64 KiB riservata in
`link.ld` (`PAGE_TABLE_ARENA_SIZE`). Sei tabelle sono usate dalla mappa del kernel;
ne restano dieci, e `tables_remaining()` è stampato a ogni boot.

La tentazione, dovendo mappare a runtime (`mmu::map`), è costruire subito un frame
allocator. Sarebbe infrastruttura speculativa: il kernel mappa se stesso una volta,
più regioni singole che il firmware assegna, e non libera mai una tabella.

## Decisione

Arena a dimensione fissa, dimensionata a build time, con lo spazio residuo riportato
al boot e `MmuError::OutOfTables` come fallimento esplicito.

È ciò che fa anche Linux con `init_pg_dir`: un pool riservato staticamente per
mappare il kernel, distinto dall'allocatore di frame che serve agli address space.

## Conseguenze

**Positive** — nessun allocatore da avere pronto prima della prima mappatura, cioè
nessuna dipendenza circolare fra heap e tabelle; esaurimento visibile prima che
diventi un fallimento (`40960 B of table arena left` a ogni boot).

**Negative** — non regge address space dinamici. Un `MAX_REGIONS` o un numero di
chiamate a `mmu::map` che cresce va accompagnato dalla verifica del residuo.

## Alternative considerate

| Alternativa | Perché no |
| --- | --- |
| Frame allocator adesso | Serve quando gli address space vanno e vengono, cioè da M5. Costruirlo prima significa progettarlo senza il suo caso d'uso |
| Tabelle dallo heap | Circolare: lo heap è mappato dalle tabelle che si vorrebbe allocare da lui |
| Arena più grande e non pensarci | Sposta il limite senza renderlo visibile; il residuo stampato è ciò che rende il limite osservabile |

## Gate che protegge questa decisione

Nessun gate automatico: il segnale è `tables_remaining()` sulla console e
`MmuError::OutOfTables`, che è un `Result` e non un panic. **Questo è un punto
debole dichiarato** — l'esaurimento verrebbe notato da chi legge il boot, non da
un controllo. Un'asserzione nel boot-check sul residuo minimo sarebbe il modo di
chiuderlo, e non è ancora fatta.

## Quando rivalutare

A M5, o prima se `mmu::map` acquisisce molti chiamanti. Il trigger concreto è il
primo address space che nasce e muore.

## Riferimenti

`link.ld` (`PAGE_TABLE_ARENA_SIZE`), `src/arch/aarch64/mmu.rs`,
[`mmu.md`](../mmu.md).
