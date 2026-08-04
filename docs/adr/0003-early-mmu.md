---
id: 0003
title: MMU enabled before any Rust runs
status: proposed
date: 2026-08-04
---

# ADR-0003: MMU enabled before any Rust runs

## Contesto

La board caricava il kernel e restava muta: LED ACT acceso durante la lettura
della SD e poi spento — la firma di un caricamento *riuscito* — e nessun output.

La causa era un `AtomicBool::swap` in `console::acquire`, la prima istruzione di
`bootstrap::run`. Una read-modify-write atomica compila in una coppia
`LDXR`/`STXR`, e con la traduzione spenta ogni accesso è Device-nGnRnE, dove gli
esclusivi non progrediscono su Cortex-A72: il retry loop gira per sempre. Nessun
fault, nessun output.

QEMU bootava l'immagine senza problemi, perché il monitor di esclusività di TCG
ignora gli attributi di memoria. **L'emulazione non può intercettare questa classe.**

Il progetto aveva già questa lezione scritta come gotcha di M1. È stata ritirata
per un ragionamento incompleto — "vale solo prima di M2" — e reintrodotta lo
stesso giorno, con la nota davanti agli occhi.

## Decisione

`boot.s` abilita la traduzione **prima di chiamare `kernel_main`**, con una mappa
identità grossolana valutata a compile time (`arch::mmu::EARLY_L1`): tre blocchi da
1 GiB di RAM più la finestra device.

Lo scopo non è la mappa — è che **nessun codice del kernel giri senza attributi di
memoria**. La mappa fine W^X diventa uno switch di `TTBR0` (`mmu::activate`).

È la sequenza di Linux arm64 (`__create_page_tables` + `__enable_mmu` in `head.S`
prima di `start_kernel`), di ARM Trusted Firmware, di Zephyr e di seL4.

## Conseguenze

**Positive** — la finestra in cui valgono regole diverse non esiste più, quindi non
può essere dimenticata; atomici leciti ovunque; cache attive da subito; lo switch a
mappa viva è più semplice di un'accensione a freddo (le scritture delle tabelle e le
letture del walker passano per le stesse cache, quindi serve una barriera invece di
invalidare tutto).

**Negative** — la mappa iniziale è RWX su 3 GiB finché `activate` non la sostituisce.
È necessario: senza attributi non si arriva nemmeno alla console. Se `activate`
fallisce si resta lì, ed è un rischio dichiarato (finding F14).

## Alternative considerate

| Alternativa | Perché no |
| --- | --- |
| Tenere la finestra e vietare le RMW | È la regola che è già stata dimenticata una volta, da chi l'aveva scritta |
| `SyncCell` al posto degli atomici lì | Correttezza per assunzione invece che per costruzione: rompe al primo secondo core |
| Costruire la mappa fine subito in `boot.s` | Serve il layout dal linker e un percorso d'errore, cioè una console, che non esiste ancora |

## Gate che protegge questa decisione

`scripts/check-pre-mmu-path.sh` deriva dall'immagine il percorso `_start` → gate,
fallisce se compare un esclusivo o se il percorso si allunga. **Visto rosso**
piantando un `fetch_add` chiamato da `_start` prima del gate.

## Quando rivalutare

Al primo core secondario: ognuno dovrà eseguire il proprio `early_mmu_enable`
prima di toccare qualunque stato condiviso.

## Riferimenti

`src/boot.s`, `src/arch/aarch64/mmu.rs`, [`mmu.md`](../mmu.md),
[`verification.md`](../verification.md).
