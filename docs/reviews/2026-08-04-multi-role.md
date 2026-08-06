# Multi-role analysis — 2026-08-04

Prima passata completa pre-M3, secondo [ADR-0001](../adr/0001-multi-role-analysis.md)
(`status: proposed`). Solo lettura del tree; nessun fix funzionale in questa
passata.

| | |
| --- | --- |
| **Tree** | working tree al 2026-08-04 (include `mmu::map` + DTB RO post-`activate`) |
| **Ruoli** | R1–R12 (tutti compilati) |
| **Metodo** | 4 batch read-only in parallelo + sintesi con verifica spot |
| **Baseline size** | `kernel8.img` ≈ 45 KiB; ELF ≈ 404 KiB (`debug=true`, non stripped); text 41 KiB |

## Sintesi esecutiva

Il kernel di bring-up è **solido sul percorso che dichiara**: early MMU prima di
Rust, mappa W^X, free-list, GIC + timer + UART RX ring, WFI idle, gate
build-enforced con blind spot documentati. Layering rules 1–2 e 4–9 sono
osservate nel grafo degli import.

Nessun **P0** (hang silenzioso / safety bug aperto e non mitigato) emerso dalla
review statica. Il debito materiale è:

1. **Drift documentazione** rispetto al codice (regola 7, bump heap, UART HW,
   bringup) — alto impatto sulla fiducia, effort basso.
2. **Gate e CI** non allineati al claim “`make check` ⊇ CI” (skip QEMU soft,
   Miri solo CI).
3. **Prerequisiti M3** non solo “allocator + perms”: manca un modello di
   esecuzione e la policy IRQ è seal-once con `fn()` senza cookie.
4. **Integrità heap / panic TX / exception stack** prima di affidare task e
   stack allocati all’allocator.

---

## Stato dei findings

Questo report elencava trenta findings e non ne registrava nessuno come chiuso;
`architecture.md` ne tracciava sei. Nessuna delle due parti tracciava le altre
ventiquattro — la forma esatta del rischio che ADR-0001 nomina per sé stesso.

Gli stati qui sotto sono stati assegnati durante l'audit file-per-file del
2026-08-06, verificando ciascuno contro il codice. `partly` e `open` sono
altrettanto deliberati di `closed`.

| ID | Stato | Evidenza |
| --- | --- | --- |
| F01 | **closed** | `mmu.md` describes the free-list allocator |
| F02 | **closed** | P0 evidence in `verification.md` |
| F03 | **closed** | README and docs name `--features bringup` |
| F04 | **closed** | rule 7 rewritten around `SyncCell` and `activate` |
| F05 | **closed** | `kernel-core` denies `unsafe_code` with one documented exception |
| F06 | **closed** | `make check` is a superset; Miri and boot-check both in it |
| F07 | **closed** | CI runs QEMU from a pinned Arch container (2026-08-06) |
| F08 | **closed** | `assert_blobs_pinned` re-verifies at write time |
| F09 | **closed** | `poll::until` bounds every TX spin |
| F10 | **closed** | SP_EL1 exception stack with its own guard — HW evidence |
| F11 | **closed** | allocator refuses and counts invalid frees |
| F12 | **closed** | ADR-0006 |
| F13 | **closed** | ADR-0008 shape; the cookie is still unread — see the note in `irq::register` |
| F14 | **closed** | `refuse_to_boot` halts rather than offering an unprotected console |
| F15 | **closed** | ADR-0011 risk-accept |
| F16 | **closed** | every SAFETY comment rewritten during the 2026-08-06 audit |
| F17 | **closed** | `MmuError::AlreadyMapped` refuses an exact collision |
| F18 | **closed** | absolute `CNTP_CVAL` deadlines + missed-tick counter |
| F19 | **closed** | `interrupts_bound` guards the unmask |
| F20 | **closed** | `RX_DROPPED` counted and asserted by the boot check |
| F21 | **closed** | `blr`/`br` refused, `b` followed (2026-08-06) |
| F22 | **partly** | fourteen negative assertions now; no fault injection or hostile EL0 |
| F23 | **open** | no ADR. The early map still encodes board topology in `arch` |
| F24 | **closed** | `make layering`, extended to facade isolation in ADR-0015 |
| F25 | **closed** | kept deliberately — the disassembly gates need symbol names; `objcopy` drops the sections from the image |
| F26 | **closed** | ADR-0013 |
| F27 | **closed** | `make doc-claims` |
| F28 | **closed** | `gicv2` routes classification, spurious detection and bit slots through `kernel_core::gic` |
| F29 | **closed** | shared mount guard; backup is a precondition (2026-08-06) |
| F30 | **closed** | paths corrected; `bootinfo` documents what consumes the DTB |

Uno solo resta aperto. F23 — la topologia di board codificata in `arch`
attraverso la mappa precoce — era stata assegnata a un ADR che non è mai stato
scritto; ADR-0015 ha spostato `boot.s` e `link.ld` sotto l'albero ISA senza
affrontare quel punto.

---

## Top findings (per severità)

| Sev | ID | Ruolo | Titolo | Azione | Effort |
| --- | --- | --- | --- | --- | --- |
| P1 | F01 | R11 | `mmu.md` Goals: “bump heap” vs free-list | fix docs | S |
| P1 | F02 | R11 | UART RX/WFI: “validate on HW” vs P0 done (HW) | fix docs / re-validate | S |
| P1 | F03 | R11 | `BRINGUP_SELFTEST` inesistente; serve `--features bringup` | fix docs | S |
| P1 | F04 | R11/R4 | Regola 7: `mmu::enable` + acquire/`SyncCell` obsoleti | fix docs | S |
| P1 | F05 | R12 | `kernel-core` lib: “no unsafe” vs `ring` | fix docs | S |
| P1 | F06 | R5/R8 | `make check` non è superset di CI (Miri; QEMU soft-skip) | fix docs + gate | S |
| P1 | F07 | R5 | CI `check` senza QEMU → boot-check sempre SKIPPED lì | fix CI / script | S |
| P1 | F08 | R6 | `deploy-sd` non ri-verifica hash blob | fix script | S |
| P1 | F09 | R4 | Panic/console TX spin unbounded su `FR_TXFF` | fix | S |
| P1 | F10 | R4 | Stack overflow su guard: no exception stack dedicato | ADR o fix | M |
| P1 | F11 | R2/R12 | Free-list: double-free / wild free non rilevati | fix + test | M |
| P1 | F12 | R10 | M3: manca execution model (solo prereq alloc/W^X) | ADR M3 | L |
| P1 | F13 | R10/R3/R9 | `irq::seal` + `Handler = fn()` vs cap_irq / M4 | ADR | M |
| P1 | F14 | R9 | Early map RWX resta se `activate` fallisce e si entra in shell | fix path / ADR | M |
| P1 | F15 | R6 | DTB mappato ma non parsato; truth di board hard-coded | ADR risk-accept o parse | S–L |
| P2 | F16 | R2 | Commenti SAFETY “MMU off” su path con translation on | fix | S |
| P2 | F17 | R2 | `mmu::map` sovrascrive leaf same-level senza conflitto | fix | M |
| P2 | F18 | R3 | Timer re-arm relativo (`TVAL`) → drift di fase | fix pre-M3 time | M |
| P2 | F19 | R3 | Bind IRQ fallito: si unmaska comunque e si re-arma timer | fix | S |
| P2 | F20 | R3/R9 | RX ring full: drop silenzioso (no counter) | fix | S |
| P2 | F21 | R5 | Pre-MMU check: un solo livello di `bl`, cieco a `blr` | fix script | M |
| P2 | F22 | R5 | Boot oracle QEMU incompleto (no negative assert) | fix script | S |
| P2 | F23 | R1 | Early map board topology in `arch` | ADR | M |
| P2 | F24 | R1 | Nessun gate automatico sui confini di import | fix script | S |
| P2 | F25 | R7 | Release `debug=true` / no strip → ELF 404 KiB vs img 45 KiB | fix deploy | S |
| P2 | F26 | R9 | Finestre Device 16 MiB (+ GIC) — blast radius pre-M6 | ADR later | M |
| P2 | F27 | R8 | README elenca male i gate di `make check` | fix docs | S |
| P2 | F28 | R12 | Driver GIC non consuma del tutto `kernel_core::gic` | fix | S |
| P2 | F29 | R6 | `restore-rpios-boot` safety più debole di deploy | fix | S |
| P2 | F30 | R11 | Path `mm::paging` errato; boot.s “Nothing consumes” DTB | fix | S |

---

## Per ruolo

### R1 — Architetto di layering

**Problemi**

#### [P2] F23 — Early map codifica topologia BCM in `arch`
- **Evidenza:** `src/arch/aarch64/mmu.rs:56-68` (`EARLY_L1` 0–3 GiB RAM + Device @ 0xC000_0000)
- **Impatto:** tensione con regola 3; la mappa fine è correttamente BSP-driven
- **Azione:** ADR — scaffold fisso Pi4 risk-accepted, oppure blocchi da BSP a `const`
- **Effort:** M

**Migliorie**

#### [P2] F24 — Nessun gate import-boundary
- **Evidenza:** gate automatici solo no-simd / pre-MMU / boot; regole 1–4 solo umane
- **Azione:** script `rg` su `exception`/`arch`/`drivers` imports in `make check`
- **Effort:** S

#### [P3] Policy legata a `Pl011` concreto (`bootstrap/shell.rs`)
- **Azione:** trait/`ConsoleTx` prima di shell multi-task — **Effort:** S

**Ottimizzazioni:** nessuna materiale (barriera `dsb` in gicv2 vs `cpu::sync_pipeline` — P3 strutturale).

**Rationale “pulito”:** regole 1–2, 4–9 rispettate (exception → solo `irq`; IRQ no TX via `Pl011Rx`; un solo `IrqChip`; WFI sotto `without_irqs`).

---

### R2 — Memoria / MMU

**Problemi**

#### [P2] F16 — SAFETY “MMU off” mentre early/kernel map è attiva
- **Evidenza:** `mmu.rs:111`, `274`, `292` vs `switch_ttbr0` e early map in `boot.s`
- **Azione:** fix commenti → “early map active, IRQs masked”
- **Effort:** S

#### [P2] F17 — `map` overwrites leaf same-level
- **Evidenza:** `mmu.rs:332-337`; `BlockAlreadyMapped` solo per blocco più grezzo
- **Impatto:** DTB o regione futura che overlap-pa una leaf già mappata cambia PA/perms in silenzio
- **Azione:** reject se leaf non-vuota, salvo API remap esplicita
- **Effort:** M

#### [P1] F11 — Free-list non difende double-free / free selvaggio
- **Evidenza:** `crates/kernel-core/src/heap.rs` dealloc si fida dell’header; `GlobalAlloc` in `mm/mod.rs`
- **Impatto:** sotto M3 corrompe free-list; danno ritardato
- **Azione:** bit “free”/canary + test host adversarial
- **Effort:** M

**Migliorie:** overlap check DTB vs `kernel_regions` (S); goals `mmu.md` (F01).

**Ottimizzazioni:** [P3] chunking L3 lead-in — risk-accepted fino a multi-AS.

**Risk-accepted:** early map 3 GiB RWX fino a `activate` (necessario per attributi).

---

### R3 — Interrupt / concorrenza / idle

**Problemi**

#### [P2] F18 — Re-arm timer con `CNTP_TVAL` relativo
- **Evidenza:** `timer.rs:70-75`
- **Impatto:** drift di fase sotto latenza handler; M3 che usa tick length lo nota
- **Azione:** deadline assoluta su `CVAL` + policy catch-up
- **Effort:** M

#### [P2] F19 — Fallimento bind IRQ: boot continua e unmaska
- **Evidenza:** `bootstrap/mod.rs:136-170`
- **Azione:** unmask/re-arm solo se bind OK
- **Effort:** S

**Migliorie**

#### [P2] F20 — Drop RX silenziosi
- **Evidenza:** `ring` push false; `pl011` drain; nessun counter (a differenza di `irq::Counters`)
- **Azione:** `AtomicU32` drops + report su tick
- **Effort:** S

#### [P1] F13 — Seal-once vs handler dinamici
- **Evidenza:** `irq::seal` + `Handler = fn()`; sealed in bootstrap
- **Azione:** ADR prima di M3/M4 (softirq ring vs register mediatо)
- **Effort:** M

**Ottimizzazioni:** [P3] trap frame completo su ogni IRQ — misurare prima di spezzare stub.

**Risk-accepted:** path GIC Group 0; WFI idle corretto.

---

### R4 — `unsafe` / panic

**Problemi**

#### [P1] F09 — TX panic può spinare per sempre
- **Evidenza:** `pl011.rs` wait su `TXFF` unbounded; `panic.rs` → `steal` → `writeln!`
- **Impatto:** UART wedge → panic muto dopo `irq_disable`
- **Azione:** limite spin (almeno su path panic)
- **Effort:** S

#### [P1] F10 — Overflow stack → guard senza stack eccezione
- **Evidenza:** un solo SP_EL1; vector fa `sub sp`; panic vuole stack+UART
- **Impatto:** overflow diagnosticabile male (board quiet)
- **Azione:** ADR exception stack EL1, o documentare limite in verification
- **Effort:** M

#### [P2] OOM via `GlobalAlloc` null → panic generico
- **Azione:** API fallibile per M3; tenere `GlobalAlloc` per ergonomia
- **Effort:** M

**Migliorie:** F04 allineare regola 7; SyncCell come tripwire SMP (ADR al primo multi-core).

**Risk-accepted:** anti-ricorsione panic; split RX IRQ type-safe.

---

### R5 — Verifica / blind spot

**Problemi**

#### [P1] F06 / F07 — Claim “superset” e soft-skip QEMU
- **Evidenza:** `verification.md:18-19`; `qemu-boot-check.sh:22-25` exit 0; CI `check` senza QEMU
- **Impatto:** green locale/CI check senza boot coverage; Miri solo job dedicato
- **Azione:** matrice gate esplicita; fail se QEMU manca salvo `ALLOW_BOOT_SKIP=1`; CI assert `boot-check: clean`
- **Effort:** S

#### [P2] F21 — Pre-MMU path shallow
- **Evidenza:** `check-pre-mmu-path.sh` un hop da `_start` / gate; no `blr`
- **Azione:** recurse call graph o fail su `blr`
- **Effort:** M

#### [P2] F22 — Oracle QEMU stringhe positive incomplete
- **Azione:** assert negativi: `LEAKED`, `MMU FAILED`, `LAYOUT INVALID`; opz. `DTB mapped`
- **Effort:** S

**Migliorie:** [P3] build CI `--features bringup`; fault probes W^X non in-tree (documentati).

**Risk-accepted:** QEMU cieco su attributes (processo: HW dopo mem/IRQ/firmware).

---

### R6 — Boot / firmware

**Problemi**

#### [P1] F08 — Deploy non re-hash blob
- **Evidenza:** `fetch-blobs.sh` verifica; `deploy-sd.sh` solo presence
- **Azione:** `sha256sum --check EXPECTED.sha256` prima di install
- **Effort:** S

#### [P1] F15 — DTB survey+map, no parse
- **Evidenza:** `bootinfo`, `bootstrap` map RO; `memmap` hard-coded
- **Azione:** ADR risk-accept Pi4-only **oppure** parse minimo (memory/clocks)
- **Effort:** S (ADR) / L (parse)

**Migliorie:** [P2] F29 restore safety; secondary cores parked pre-MMU (ADR SMP); QEMU bypass firmware (risk-accepted + checklist blob bump).

**Risk-accepted:** pin hash + tag git; early BSS/stack pre-attributi (gate disasm).

---

### R7 — Prestazioni / footprint / idle

**Baseline:** `TIMER_HZ=10`, heap 64 MiB, stack 64 KiB, arena PT 64 KiB, img ~45 KiB, ELF ~404 KiB.

**Ottimizzazioni (falsificabili)**

#### [P2] F25 — debug info in release
- **Ipotesi:** DWARF domina ELF; runtime invariato
- **Azione:** strip per deploy, ELF unstripped per gdb; confrontare size
- **Effort:** S

#### [P2] Heap 64 MiB a idle
- **Ipotesi:** working set ≪ 64 MiB; riduce blast radius e (forse) tabelle
- **Azione:** const configurabile + misura `tables_remaining` / heap_check
- **Effort:** S

**Risk-accepted:** WFI @ 10 Hz (gate QEMU dipende da `ticks=20` in 15 s).

**Non ottimizzare senza misura:** full trap frame IRQ; hold `without_irqs` su heap (post-M3).

---

### R8 — Tooling / CI / DX

**Problemi:** F06/F07/F27 (README gate list incompleta: omette pre-MMU e boot-check).

**Migliorie:** CI usa `make miri`; `deploy` dipende da blobs; `flock` su serial; default `SD_MOUNT`; `make restore-rpios`; rust-cache CI.

---

### R9 — Sicurezza pre-agent

**Modello onesto:** single EL1 trusted, identity map, core 0. Non multi-tenant.

**Problemi**

#### [P1] F14 — Failure path `activate` → early RWX + shell
- **Evidenza:** `bootstrap/mod.rs:99-102` continua; early map resta RWX
- **Azione:** degraded mode esplicito / halt / no shell “normale”
- **Effort:** M

#### [P1] F13 — `register` libero fino a seal (ok oggi; forma M4)
- **Azione:** documentare invariante + ADR cap_irq

#### [P2] F26 — Device window 16 MiB
- **Azione:** narrowing page-level prima di M6; risk-accepted pre-agent

**Migliorie:** F20 RX drops; bound DTB `totalsize` al parse; low RAM 0–0x80000 RW (risk-accepted fino a SMP).

**Risk-accepted:** no EL0/PAN/ASID; W^X protegge il kernel **da sé**, non da agent.

---

### R10 — Roadmap agent (M3–M6)

| Prereq | Stato |
| --- | --- |
| Free-list + GlobalAlloc | have |
| W^X per-regione | have |
| IRQ/timer/RX/WFI | have |
| Task / yield / runqueue | **missing** |
| Context switch (SP in frame) | **missing** (coop può usare yield software) |
| Multi-AS / frame alloc / EL0 | **missing** (M5) |
| Caps / IPC / driver-as-agent | **missing** (M4–M6) |

#### [P1] F12 — M3 “unblocked” ≠ “ready”
- **Evidenza:** `architecture.md:111-113`; nessun modulo task
- **Azione:** ADR M3 (TCB, stack da heap, yield, idle task, no preemption)
- **Effort:** L

#### [P1] F13 — seal + `fn()` bloccano cap_irq shape
- **Azione:** ADR con M3 mailbox notify

**Migliorie:** documentare M3 = cooperative only; per-task stack canary (guard page richiede frame alloc).

**Risk-accepted:** docs già oneste sull’agent model non implementato.

---

### R11 — Documentazione

Tutti i **P1 F01–F04** e **P2 F30** sono drift verificati con `rg` sul tree.

| Drift | Doc | Codice |
| --- | --- | --- |
| bump heap | `mmu.md:9` | free-list `GlobalAlloc` |
| UART validate on HW | `interrupts.md:18` | architecture/README P0 done (HW) |
| BRINGUP_SELFTEST | `interrupts.md:83` | `--features bringup` only |
| regola 7 / SyncCell acquire | `architecture.md:53-61` | early map + `AtomicBool` acquire |
| `mm::paging` | `architecture.md:84` | `kernel_core::paging` + `arch::mmu` |
| DTB “nothing consumes” | `boot.s:27-30` | `bootinfo` + map |
| panic re-acquire | `interrupts.md:92` | `console::steal` |

**Azione:** fix docs (batch S) — massima leva fiducia/effort.

---

### R12 — `kernel-core` API / testabilità

**Forze:** `unsafe_code = "deny"` + allow su ring; host tests + Miri; FreeList offset-based; paging/layout testati.

**Problemi:** F05 claim “no unsafe”; F11 double-free; F28 GIC pure math non consumata; `Ack` vs `ack_id` duplicati.

**Migliorie:** runqueue pure in kernel-core quando nasce M3; EL0 AP encodings a M5; habit Miri su PR che toccano ring/heap.

**Risk-accepted:** ring come unica primitiva concurrency nella crate pure-logic.

---

## Accepted risks (espliciti)

| Risk | Perché accettato ora | Rivalutare quando |
| --- | --- | --- |
| QEMU cieco su memory attributes / esclusive | Mitigato da early MMU + pre-MMU disasm gate; HW obbligatorio su mem/IRQ | M3 concurrency ampia |
| Early map 3 GiB RWX fino ad `activate` | Necessario per attributi prima di Rust | Se failure path resta in shell (F14) |
| Device window 16 MiB | Solo kernel trusted | Prima di M6 / EL0 |
| GIC Group 0 path | Verificato su HW con firmware pin | Blob bump |
| DTB non parsato | Pi4-only; costanti BSP stabili | Multi-board o clock drift |
| `irq::seal` statico | Corretto per timer+UART fissi | M3/M4 handler dinamici |
| No EL0 / multi-AS | Roadmap | M5 |
| Miri fuori da `make check` | Nightly vs pin stable | Opzionale `check-all` |
| TIMER 10 Hz + WFI | Idle sensato; gate QEMU dipende | Goal power/latency |
| SyncCell single-core | Contratto documentato | Primo core secondario |

---

## Backlog consigliato (ordine)

### Subito (docs + gate, effort S) — nessuna decisione architetturale

1. Allineare docs: F01–F05, F27, F30, regola 7, bringup, UART status, boot.s DTB comment.
2. Matrice gate in `verification.md` (F06); fail QEMU soft-skip salvo opt-out (F07).
3. Re-hash blob in deploy (F08).
4. Assert negativi in `qemu-boot-check.sh` (F22).
5. Link ADR + questo report da `architecture.md` / README.

### Prima di scrivere codice M3

6. **ADR M3** execution model (F12).
7. **ADR IRQ policy** seal / cookie / notify (F13).
8. Free-list integrity + test (F11).
9. Panic TX bound (F09); exception-stack story (F10).
10. Timer absolute deadline se i tick guidano scheduling (F18).
11. Path activate-failure non “shell normale” su early RWX (F14).

### Quando tocca il pezzo

12. Harden `mmu::map` leaf conflict (F17); pre-MMU call graph (F21).
13. Import-boundary gate (F24); strip deploy image (F25).
14. DTB parse o ADR risk-accept formale (F15).
15. Narrow MMIO windows (F26) prima di M6.

---

## Copertura ruoli

| Ruolo | Finding materiali | “Nessun finding” |
| --- | --- | --- |
| R1 | F23, F24, P3 Pl011 | Rules 4–9 clean |
| R2 | F11, F16, F17 | No P0 map/descriptor bug in review |
| R3 | F13, F18–F20 | WFI/idle OK |
| R4 | F09, F10, OOM | Panic recursion OK |
| R5 | F06, F07, F21, F22 | — |
| R6 | F08, F15, F29 | Blob pin model OK |
| R7 | F25, heap size | Idle WFI risk-accepted |
| R8 | F06, F07, F27 | Toolchain pin strong |
| R9 | F13, F14, F26 | Pre-agent model honest |
| R10 | F12, F13 | Docs honest on agents |
| R11 | F01–F04, F30 | — |
| R12 | F05, F11, F28 | Host-test posture strong |

---

## Prossimi artefatti possibili

| Artefatto | Trigger |
| --- | --- |
| Accettazione [ADR-0001](../adr/0001-multi-role-analysis.md) | Review umana di questo report |
| ADR-0002 M3 cooperative tasks | Prima riga di scheduler |
| ADR-0003 IRQ seal / cap_irq shape | Prima di handler non-statici |
| ADR-0004 DTB policy (parse vs hard-code) | Se si tocca board truth |
| Batch fix `docs:` | F01–F05, F27, F30 |

---

*Fine report. Re-run incrementale: diff su mem/IRQ/unsafe/boot/layering, o prima di marcare un milestone `done (HW)`.*
