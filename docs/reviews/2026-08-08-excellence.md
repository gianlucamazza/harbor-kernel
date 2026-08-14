# Excellence review — 2026-08-08

Audit di eccellenza dell'intero progetto, secondo [ADR-0001](../adr/0001-multi-role-analysis.md):
solo findings, nessuna decisione, nessuno status (owner dello status resta
`docs/roadmap.md`). Solo lettura del tree; nessun fix in questa passata.

|                      |                                                                                                                                                                                                                                                                   |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Tree**             | audit avviato su `440d77b` + working tree (slice ADR-0054 non committata); la slice è stata committata **durante l'audit** come `0cee6e4`. Ogni finding è stato ri-verificato contro `0cee6e4` (tree pulito)                                                      |
| **Metodo**           | 7 dimensioni read-only in parallelo (A conformità architetturale, B unsafe/SAFETY, C copertura verifica, D drift SSOT, E claim sicurezza, F salute gate, G review slice 0054) + passata avversariale: ogni claim ≥ major riaperto da un verificatore indipendente |
| **Verdetti**         | CONFIRMED = evidenza riprodotta sul tree finale; PLAUSIBLE = non refutabile read-only. Un solo claim refutato in toto (layering rosso — superato dal commit, vedi «Corsa col tree»)                                                                               |
| **Vietato in audit** | `make`, `cargo`, QEMU — ogni evidenza build-dependent è marcata come tale                                                                                                                                                                                         |

## Corsa col tree

L'ipotesi iniziale «`make layering` è rosso: `taskcap` assente dalla allowlist»
era vera sullo snapshot d'avvio e **falsa sul tree finale**: `layering.sh`
faceva parte della slice ed è stato committato con gli edge `sched→taskcap`,
`bootstrap→taskcap`, `taskcap→arch`. Il residuo che sopravvive è F-20 (ADR-0054
non contiene la clausola di layering che ADR-0031 §5 aveva reso convenzione).
Tutti i findings sotto sono verificati contro `0cee6e4`.

## Sintesi esecutiva

Il progetto è **eccellente sul percorso che i gate guardano** — e questo audit
trova quasi tutto il debito esattamente un passo fuori da quel percorso. I tre
pattern che riassumono i 34 findings:

1. **Editing gate-shaped.** La slice 0054 ha aggiornato _esattamente_ le righe
   che un gate legge (riga 8 della tabella syscall, conteggio test del README,
   code-layout block) e ha lasciato stale tutto ciò che nessun gate legge nello
   stesso file: `SECURITY.md` ora si contraddice da solo su transfer e timeout
   (F-1), `verification.md` non ha una riga per tre flip `done (QEMU)`
   consecutivi (F-6), i commenti con numeri (`Cargo.toml`, `ci.yml`) sono stale
   del 30–100 % (F-15).
2. **Autorità cresciuta più in fretta del suo threat model.** Peer transfer
   funziona ed è pulito nel codice, ma: le catene di delega sono implementate
   benché dichiarate non-goal (F-3), il bump ABI IPC è avvenuto due volte senza
   ADR successore (F-4), un IRQ-cap è trasferibile mentre SECURITY.md dice il
   contrario (F-11), e la semantica push/untyped della riga 8 non è scritta da
   nessuna parte (F-12).
3. **Evidenza che invecchia senza forcing function.** Mutation testing:
   l'artefatto su disco certifica 1 modulo su 40, il gate non sa distinguere un
   run parziale da uno completo, e `taskcap` — il nuovo modulo di autorità —
   non è mai stato mutato (F-7). L'oracle negativo della slice non discrimina
   la proprietà che nomina (F-8).

Nessun P0 (memory-safety bug aperto). Il corpus `unsafe` è il punto più forte
del tree (§5). Il debito è quasi tutto doc-fix + gate-fix a effort basso, più
tre decisioni che richiedono un ADR.

---

## §1 Metodo

Sette dimensioni parallele, ciascuna con perimetro e comandi propri, seguite da
merge/dedup (i ~60 findings grezzi convergono in 34) e da una passata
avversariale: due verificatori indipendenti, senza accesso al ragionamento
originale, hanno riaperto ogni `file:line` citato dai claim ≥ major sul tree
`0cee6e4`. Esito: 21/22 CONFIRMED, 5 ADJUSTED (correzioni recepite sotto),
1 REFUTED (layering, vedi sopra). I findings sotto major portano il verdetto
della dimensione d'origine, cross-confermato quando due dimensioni sono
arrivate allo stesso fatto per vie indipendenti.

## §2 Matrice di copertura verifica (estratto portante)

Colonne: host test / miri / mutants (ultimo run completo 2026-08-07, 10 file) /
oracle QEMU / stamp HW.

**`crates/kernel-core` (10 645 LoC, 344 `#[test]`, tutti i 39 moduli con test):**
miri sì (salvo model check, documentato). Mutation: **solo 10 file su 40**
(`ipc, tasks, layout, irqtable, rxline, reset, cap, syscall, prog, manifest` =
43 % del crate). Mai mutati, tra gli altri: `taskcap` (nuovo, autorità),
`runqueue`, `asid`, `irqcap`, `naming`, `paging` (799 LoC), `heap` (608 LoC).

**`src/` (13 484 LoC, 0 test, 0 `cfg(test)`):** evidenza esclusivamente
transitiva via oracle QEMU — **nessun modulo di sottosistema emette una stringa
asserita**; tutte le ~100 assertion risalgono a 6 file di scaffolding
(`bootstrap/*`, `agent`). Zone morte (zero evidenza positiva a ogni livello):

| Zona                                                                                            | LoC    | Evidenza                                                                                                                                                              |
| ----------------------------------------------------------------------------------------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/panic.rs`                                                                                  | 48     | solo negativa (`grep PANIC && fail`) — un boot verde prova che non è mai girato                                                                                       |
| isola `debug-display` (`status.rs`, `bsp/rpi4/display.rs`, `drivers/{ili9486,spi/*,pin,delay}`) | ~1 240 | build-only; nessuna riga nella layer table di verification.md                                                                                                         |
| `mm/asid.rs` (K7)                                                                               | 42     | QEMU-only; zero occorrenze in verification.md; è la classe che verification.md §«What emulation cannot catch» dichiara più debole in TCG                              |
| `src/taskcap/` + `el0-xfer-peer`                                                                | 38     | QEMU-only, 2 assertion su ~12 righe emesse                                                                                                                            |
| `sync.rs`, `mm/early.rs`, `irq/chip.rs`, `bsp/rpi4/memmap.rs`, sei file `arch/`                 | ~900   | boots-only                                                                                                                                                            |
| immagine **prodotto**                                                                           | —      | ~9 assertion (`qemu-product-boot-check.sh`) contro ~100 della default: la configurazione spedita è verificata un ordine di grandezza più debolmente di quella di test |

## §3 Matrice punti ciechi dei gate

| Gate                 | Punto cieco strutturale                                                                                                                             | Istanza viva                                                                                                                    |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `layering.sh`        | multi-line `use crate::{…}`; `use super::`; laundering via `pub use`; solo commenti `//`                                                            | nessuna (0 forme a rischio nel tree)                                                                                            |
| `irq-scope.sh`       | **coppie manuali `irq_save`/`irq_restore` non sono un opener**; switch indiretti (auto-documentato); lista SWITCHERS fissa                          | **sì**: `src/sched/mod.rs:869–972` è una regione mascherata mai camminata, e `taskcap::revoke_task` (:908) ci sta dentro (F-13) |
| `doc-claims.sh`      | set e conteggi, mai semantica (auto-documentato); **arità degli argomenti** di una syscall esistente; commenti in `Cargo.toml`/`.github/` mai letti | sì: F-1, F-15                                                                                                                   |
| `doc-symbols.sh`     | lista DOCS scritta a mano (12 file); 66 `.md` fuori                                                                                                 | sì: `docs/design/m8-console-endpoint.md` nomina 2 simboli dichiarati da nessuna parte (F-19)                                    |
| `xrefs.sh`           | esistenza, non contenuto; ancore `#…` non verificate; **i corpi degli ADR accettati non sono mai diffati**                                          | sì: F-10                                                                                                                        |
| `no-static-mut.sh`   | solo dichiarazioni letterali; le funzioni `-> &'static mut` restituiscono l'ergonomia di `static mut` senza il letterale                            | sì: `irq::state()`, `durable::region` (F-26)                                                                                    |
| `pre-mmu-path.sh`    | build-dependent (non valutato qui); set auditato con terminale a mano                                                                               | —                                                                                                                               |
| `arch-board-free.sh` | solo letterali hex; base scritta in decimale o `3*1024*1024*1024` invisibile                                                                        | nessuna                                                                                                                         |
| `run-mutants.sh`     | **non verifica che il run coprisse la lista richiesta**: un run scoped/interrotto passa le baseline banalmente                                      | sì: `mutants.out/` attuale = solo `manifest.rs` (F-7)                                                                           |
| `product-image.sh`   | marker derivati da **solo `demos.rs`**; filtro `{`/`\` scarta 131 stringhe su 208                                                                   | sì: 4 stringhe oracle della slice vivono in `bootstrap/mod.rs:645-650`, invisibili (F-14)                                       |

`make check` vs CI: superset **falso** per un passo — la CI esegue `make blobs`
e `check` no; il claim è ripetuto in `Makefile:125`, `ci.yml` e
`pull_request_template.md:17` (F-9). `shellcheck` e `miri` possono auto-saltare
a verde localmente senza opt-in esplicito, a differenza di `boot-check` (F-27).

---

## §4 Findings (per severità)

Formato: `[F-n] SEVERITÀ VERDETTO — claim — evidenza — classe di remediation`
(la classe è una proposta; l'accettazione è umana). Origine tra parentesi.

### Major

**[F-1] MAJOR CONFIRMED — `SECURITY.md` si contraddice da solo: la prosa residuale è stale di 3+ slice mentre la tabella è aggiornata.**
`SECURITY.md:156` «transfer between agents still open», `:159` «no EL0
transfer/delegation yet», `:196` «Transfer between TCB slots still open» — vs
la riga 8 della **stessa** tabella (`:122`, aggiornata dalla slice) e
`sched::transfer_held` (`src/sched/mod.rs:411`). `:100`+`:194` «no EL0 recv
timeout» vs riga 9 (`:123`) e `SYS_RECV_TIMEOUT=9`. `:193` «Done (QEMU)» per
wait-on-IRQ vs `roadmap.md:82` done (HW). `:199` «Residual: creator-exit
cascade» vs `roadmap.md:91` done (HW). È il pattern «editing gate-shaped»: la
slice ha toccato l'unica riga che `doc-claims.sh` confronta. (D-2/E-1/E-6/G-5)
— **doc-fix**, poi **gate-fix** (vedi Remediation R2).

**[F-2] MAJOR CONFIRMED (adjusted) — «Busy-loop at EL0 | Mitigated» è più forte del codice.**
`SECURITY.md:99` dice «Mitigated (QEMU first slice) — cooperative CPU budget»
con residuo «IRQ-side preemption code not landed». Ma il budget non copre EL0
_affatto_: `sched::budget_expired` (`src/sched/mod.rs:524`) ha due soli caller,
entrambi worker EL1 (`demos.rs:1219,1236`); il loop di sessione EL0 su
`El0Outcome::Irq` fa `handle_cpu_irq()` + `resume_step` senza alcun check
(`src/agent/mod.rs:539-547`). L'evidenza `budget: rotated` è rotazione di
worker EL1. La riga andrebbe riscritta: mitigato per worker EL1 cooperativi,
**aperto** per EL0. (E-5) — **doc-fix**.

**[F-3] MAJOR CONFIRMED — le catene di delega sono implementate benché dichiarate non-goal, decise in un commento.**
`src/sched/mod.rs:468` «The moved object may be any CapId (endpoint, IRQ, even
another task-cap)», `:479` «moving the task-cap itself is allowed» — mentre
ADR-0053 le elenca sotto Non-goals (`:46`) e ADR-0054 sotto Residuals (`:47`).
A che detiene il task-cap di B può darlo a C: autorità di install transitiva,
senza attenuazione, non menzionata in `SECURITY.md`. Il fix stretto è un check
di banda a due righe in `transfer_held_to_peer` (rifiuta cap nella banda
`0x4000`), coerente con il residuo dichiarato. (E-2/G-1) — **needs-ADR** (o
code-fix che ripristina il non-goal).

**[F-4] MAJOR CONFIRMED — costanti ABI EL0 cambiate due volte senza ADR successore.**
ADR-0017 §4 (`0017:158`): «mailbox count (8) and endpoint count (16) become
part of this ABI». Tree: `MAX_MAILBOXES=16`, `MAX_ENDPOINTS=32`
(`src/ipc/mod.rs:78-79`) — 8/16 → 12/24 in `98fb538` (silente), → 16/32 in
`0cee6e4` («needed two more channels» per gli oracle, +4/+8 contro un solo
`create_channel` aggiunto). Due owner dello stesso fatto ora in disaccordo;
ADR-0054 non menziona il bump; pressione di test che muove ABI di prodotto.
(A-2/G-3) — **needs-ADR** (successore di 0017 §4) + **gate-fix** (riga
doc-claims: costanti vs documento).

**[F-5] MAJOR CONFIRMED (adjusted) — il binding task-cap→task poggia su un invariante non asserito, con `TaskId` riciclato senza generazione.**
`TaskId(pub u32)` è l'indice di slot TCB (`runqueue.rs:19`), riusato dopo exit
(`tasks.rs:145-163`); la generazione del cap protegge l'_handle_, non il
_binding_. La correttezza sta tutta in `revoke_task(from)` sull'unico funnel
d'uscita (`sched/mod.rs:908`, valore di ritorno scartato) — chiamata prevista
da ADR-0054 §2, ma la _dipendenza_ (perché è obbligatoria: riciclo di slot) non
è documentata né asserita; `Decision::Stay` e l'early-return `STARTED==0` sono
bypass strutturali che nulla vieta; sotto K8 SMP la finestra
`switch(Exit)`→`revoke_task` diventa una corsa reale. L'ordine attuale è
verificato corretto (revoke prima della cascata ADR-0038, nessuna seconda
porta d'uscita via ADR-0033). (G-2) — **needs-ADR** (asserire l'invariante;
fix durevole: epoch di spawn nell'Entry) + **gate-fix** (F-8 lo renderebbe
osservabile).

**[F-6] MAJOR CONFIRMED — l'indice delle evidenze non ha una riga per tre flip `done (QEMU)` consecutivi, e li contraddice.**
`docs/verification.md` (1 633 righe): zero occorrenze di `asid`,
`el0-xfer-peer`, `taskcap`, `0054`; l'unica occorrenza di `resolve-grant` è
`:589` «peer transfer / resolve-grant still residual». Roadmap: K7 ASID
(`:88`), resolve-grant (`:70`), peer (`:69,:84`) tutti done (QEMU). Gli oracle
esistono e girano — manca solo l'indice, e nessun gate legge il contenuto di
verification.md. Aggravante K7: ASID/nG/TLB è esattamente la classe che
verification.md dichiara più debole sotto TCG, e non c'è stamp HW. (D-1/C-4/
F-8/G-12) — **doc-fix** + **gate-fix** (R2).

**[F-7] MAJOR CONFIRMED — mutation testing: artefatto parziale indistinguibile da uno completo, nessuna forcing function, il nuovo modulo di autorità mai mutato.**
(a) `mutants.out/` = 16 mutanti, tutti `manifest.rs`; il run completo (318
mutanti, 10 file) è finito clobberato in `.old` e **fallirebbe la baseline**
(2 timeout vs `BASELINE_TIMEOUT=1`; verification.md `:1385` dice 1, la
riconciliazione narrata a `:1401-1408` non ha artefatto che la dimostri).
(b) `run-mutants.sh` valuta `wc -l missed.txt` senza asserire che il run abbia
coperto la lista richiesta: un run scoped passa banalmente. (c) `mutants` non è
in `make check` né in CI; output gitignorato. (d) `taskcap.rs` assente dalla
lista `--file` — la lista è scritta a mano, la stessa classe di fallimento che
`product-image.sh:66-76` documenta per sé. (C-1/2/3/12, F-1/2/3) —
**gate-fix** (validare lo scope da `mutants.json`; derivare la lista) +
**needs-ADR** (cadenza/forcing function).

**[F-8] MAJOR CONFIRMED — l'oracle negativo della slice non discrimina la proprietà che nomina, ed è un downgrade silenzioso della promessa di ADR-0053.**
`el0_peer_xfer_refuse_task` è spawnato **senza cap** (`bootstrap/mod.rs:648`,
`spawn` liscio) e invoca `transfer(0,0,peer=1)` con entrambi gli slot vuoti
(`demos.rs:1155,1163`): rimuovendo il check del task-cap, `BadFromSlot`
produrrebbe la stessa riga `el0-xfer-peer: refused` e il gate resterebbe
verde. ADR-0053 §evidence prometteva «refuse **stale** task-cap»; ADR-0054 §4
l'ha riscritta in «without a valid task-cap» e il path stale non è esercitato
da nessun livello sopra lo unit test. Inoltre delle ~12 righe emesse dai 4
demo il gate ne asserisce 2: `donor emptied` — l'invariante move-not-copy — è
calcolata, stampata e **non asserita**. (E-8/G-4/F-9/G-8) — **gate-fix**.

**[F-9] MAJOR CONFIRMED — «make check è un superset della CI» è falso, in tre posti.**
La CI esegue `make blobs` (fetch+hash firmware pinnato, `ci.yml:134-135`);
`check:` (`Makefile:129`) non lo include. Il claim sta in `Makefile:125` («the
one property this target claims», `:132`), e in
`.github/pull_request_template.md:17`. Un verde locale non predice la CI su
`EXPECTED.sha256` corrotto o tag upstream morto. (F-4) — **gate-fix** (aggiungere
`blobs` a check, con la dipendenza di rete da decidere) o **doc-fix** (ritrattare
in tre posti).

**[F-10] MAJOR CONFIRMED — l'immutabilità degli ADR accettati è violata per convenzione consolidata.**
Regola scritta due volte (`CONTRIBUTING.md:12-13`, `docs/README.md:121-122`).
`0cee6e4` riscrive il **contenuto** di ADR-0053 accettato: item 2 del design
cambia meccanismo (auto-mint on spawn → mint EL1 esplicito, demoted a
residuo), un non-goal rimosso, rationale del deferral sostituito. Precedenti:
ADR-0047 riscritto in `560ab4e`-era (`560ba4e`), ADR-0039 in `440d77b`,
ADR-0045 in `fd7a4d0`. La pratica è coerente («riconciliazione post-slice») ma
la regola scritta dice altro, e `xrefs.sh` non diffa mai i corpi. (D-5) —
**needs-ADR**: o emendare la regola (campo `amended:` in frontmatter,
gate-checkabile) o passare ai successori.

**[F-11] MAJOR CONFIRMED — un IRQ-cap è trasferibile mentre SECURITY.md dichiara il contrario.**
`SECURITY.md:198` «Residual: no transfer/revoke of IRQ caps». `transfer_held`
non ha filtro di banda (`sched/mod.rs:411-449`): un cap `0x8000` si muove verso
self/creator/peer. Il modello single-armer di ADR-0030 (`WaitIrqError::Busy`)
era progettato per un holder fisso; il passaggio a runtime è non testato e non
documentato. (E-7) — **doc-fix** (se intenzionale) o **code-fix** (filtro di
banda, coerente con F-3).

### Medium

**[F-12] MEDIUM CONFIRMED — la riga 8 documenta registri, non autorità.**
`SECURITY.md:122` non nomina: la banda `0x4000`, la semantica **push** (il
ricevente non acconsente, non è notificato, non conosce il donor né i diritti
dell'oggetto), l'assenza di filtro sul tipo di cap mosso. Con
`MAX_CAPS_PER_TASK=4` e slot posizionali, un donor ostile può piazzare un cap
del tipo sbagliato dove il peer si aspetta una SEND, o esaurirne gli slot.
«Push-only e untyped» è una decisione, non un'omissione. (E-3/G-6) —
**doc-fix** + **needs-ADR** per la proprietà receiver-side.

**[F-13] MEDIUM CONFIRMED — `irq-scope` è cieco alle coppie manuali `irq_save`/`irq_restore`, e la slice ha messo codice nuovo proprio lì.**
`OPENER="without_irqs"` (`irq-scope.sh:58`); `src/sched/mod.rs:869/881/972` è
l'unica regione a coppia manuale del tree e `taskcap::revoke_task` (`:908`) ci
sta dentro (benigno: nested mask, verificato — ma nessun gate l'ha stabilito).
Fix a una riga: `OPENER = r"without_irqs|irq_save"`, o vietare la coppia cruda
fuori da `cpu.rs`. (F-5) — **gate-fix**.

**[F-14] MEDIUM CONFIRMED — 4 stringhe oracle della slice sono invisibili a `product-image.sh`.**
I marker sono derivati **solo** da `demos.rs` (`product-image.sh:82-83`); le
stringhe `el0-xfer-peer: parent spawned/…FAILED/refuse spawned/…FAILED` vivono
in `bootstrap/mod.rs:645-650` (blocco `#[cfg(feature="oracle")]`). Buco
preesistente (`:603-604`, `:635-640`), allargato dalla slice. In più il filtro
`{`/`\` scarta 131 letterali su 208. (F-6/F-6b) — **gate-fix** (estendere
l'estrazione a ogni regione `oracle`).

**[F-15] MEDIUM CONFIRMED — i commenti con numeri sono una zona franca dei gate, e sono stale.**
`Cargo.toml:59` «181 SAFETY / 174 unsafe» vs **233/213** reali; `ci.yml:121`
«172 host tests» vs 344 `#[test]`; `ci.yml:36` «some sixty assertions» vs 105
`fail`. Lo stesso commento di Cargo.toml ammonisce sui «fact in two places».
Nessun gate legge `Cargo.toml` o `.github/`. (D-4/B-1/F-10) — **doc-fix**
(o togliere i numeri: la sostanza — ratio ≥ 1 — è vera e gate-checkata da
clippy).

**[F-16] MEDIUM CONFIRMED — `AddressSpace::drop` diverge da `destroy`: leak latente di ASID + niente TLBI.**
`destroy()` (`aspace.rs:332-343`): frame, `invalidate_asid`, `asid::free`,
`forget`. `Drop` (`:471-478`): solo frame. Oggi irraggiungibile — tutti i 19
siti passano per `destroy()` (verificato) — ma `Agent` non ha `Drop` proprio,
quindi ogni futura early-return riacquista il leak in silenzio. (B-3 adjusted)
— **code-fix** (allineare `Drop`, o `debug_assert!` nel Drop).

**[F-17] MEDIUM CONFIRMED — `kernel-core` non ha `undocumented_unsafe_blocks` e gli allow sono a scope di modulo.**
`crates/kernel-core/Cargo.toml` ha solo `unsafe_code="deny"`; `ring.rs:19` e
`wake.rs:12` portano `#![allow(unsafe_code)]` per l'intero file; `lib.rs:5`
dice «one deliberate exception» ma le eccezioni sono due. (B-2) — **gate-fix**.

**[F-18] MEDIUM CONFIRMED — `revoke_task` on exit ha zero evidenza end-to-end; quattro cause di rifiuto collassano in un solo `Status::Authority`.**
L'intera storia di lifecycle del nuovo potere ha come sola evidenza lo unit
test della tabella pura; nessuna assertion `stale refused` (ADR-0032 la sua
l'aveva). `transfer_reply` mappa ogni `Err(_)` su `Authority`
(`agent/mod.rs:240-245`): self-target, banda errata, stale e slot vuoto sono
indistinguibili per EL0 e per l'oracle; tre varianti su quattro non testate
sopra lo unit test. (E-9/C-7/G-7) — **gate-fix** + **code-fix** (errori
distinti).

**[F-19] MEDIUM CONFIRMED — `docs/design/m8-console-endpoint.md` nomina due simboli inesistenti ed è fuori dalla lista DOCS.**
`console::grant_console_cap` / `console::is_console_cap`: zero dichiarazioni
in `src/`+`crates/` (pianificata la loro rimozione nel documento stesso, PR5
avvenuta). Il file legge come descrittivo, non come record datato. (F-7) —
**doc-fix** (header «dated plan» o ingresso in DOCS + fix path).

**[F-20] MEDIUM CONFIRMED — ADR-0054 non ha la clausola di layering per i nuovi edge.**
ADR-0031 §5 autorizzava esplicitamente `sched→ipc` («enforced allow-list
update»); la slice aggiunge tre edge (`sched→taskcap`, `bootstrap→taskcap`,
`taskcap→arch`) solo nel gate, non nell'ADR. Il diagramma di
`architecture.md:82-84` non elenca `taskcap` tra i moduli kernel-policy, e la
lista import dell'agent (`:178-182`) omette `console`/`naming` (pre-esistente).
(A-1 residuo, A-5) — **doc-fix**.

**[F-21] MEDIUM CONFIRMED — la layer table di verification.md è il registro dei blind spot del progetto, ed è incompleta.**
Manca una riga per 7 dei 20 gate di `make check` (`arch-board-free, xrefs,
shellcheck, board-guard, debug-builds, debug-display-builds,
product-boot-check`) e per l'isola `debug-display` (~1 240 LoC
compiled-never-executed). La cella «Blind to» di irq-scope non menziona F-13.
Nulla confronta questa tabella col Makefile. (F-8/C-8) — **doc-fix** +
**gate-fix** (R2).

**[F-22] MEDIUM CONFIRMED — README status snapshot stale di 3 slice.**
`README.md:108-110`: «transfer/timeout/creator-exit cascade open» (tutti done
(HW) per roadmap `:83,:84,:91`); tier «H1 first slices (QEMU)» stale per 5
elementi su 9 ormai done (HW). La slice ha toccato la riga 111 (gated) della
stessa tabella. (D-3/E-12/G-13) — **doc-fix**.

**[F-23] MEDIUM CONFIRMED — `bsp/rpi4/memmap.rs::DEVICE_REGIONS` è const data non validata il cui failure mode è il panic path non verificato.**
Nessun host test asserisce non-overlap/allineamento/ordinamento della tabella;
la validazione runtime (`kernel_core::layout`) fallisce sul boot path, cioè in
F-24. (C-9) — **code-fix** (const-assert o test kernel-core, un pomeriggio).

**[F-24] MEDIUM CONFIRMED — `src/panic.rs` ha solo evidenza negativa.**
`PANICKING` guard, `console::steal` (unico caller), `show_panic` (unico
caller): nessuna esecuzione positiva in alcun livello. È il codice che gira
solo quando tutto il resto è rotto. (C-5) — **gate-fix** (variante boot a
panic deliberato che asserisce banner e park di rientranza).

### Minor / Info

**[F-25] MINOR CONFIRMED** — wrap della generazione u16 dopo 65 535 mint/slot
rivalida un handle stale; il test copre solo `gen+1`; il commento
(`taskcap.rs:103`) è vero per un ciclo e silenziosamente falso al wrap.
EL1-only, irraggiungibile nel boot attuale. (G-9) — **doc-fix** + un test.

**[F-26] MINOR CONFIRMED** — le funzioni `-> &'static mut`/`&'static T`
(`irq::state()` `irq/mod.rs:107`, `durable::region` `durable/mod.rs:14`,
`el0::current`) reintroducono l'ergonomia di `static mut` dietro un gate che
cerca solo il letterale. Corrette oggi (verificato ogni caller), fragili per
costruzione. (B-5) — **code-fix** (pattern closure come `with_heap`) o
**gate-fix** (regex companion).

**[F-27] MINOR CONFIRMED** — `shellcheck` e `miri` auto-saltano a verde senza
opt-in (`Makefile:142-147, 267-272`), a differenza di `boot-check`
(`ALLOW_BOOT_SKIP`); la CI non installa shellcheck (affidamento sull'immagine
runner, non verificabile da qui). (F-11) — **gate-fix**.

**[F-28] MINOR CONFIRMED** — `MAX_TASK_CAPS=32 < MAX_TASKS=40`; `mint` senza
dedup; unico release è `revoke_task(target)` (niente free per-cap): un server
longevo può esaurire la tabella e `mint FAILED` non è asserito da alcun gate.
Il ledger di crescita (`sched/mod.rs:44-54`) non è stato aggiornato per i 4
task nuovi (42 siti `sched::spawn*`, alcuni in loop — il conteggio siti non è
il conteggio task). (G-10/G-15 adjusted) — **doc-fix** + riga di ledger.

**[F-29] MINOR CONFIRMED** — `taskcap::mint` accetta qualsiasi u32 senza check
di liveness/range (mintabile un cap per idle o per `TaskId(9999)`); il
contratto del modulo puro è più debole della prosa dell'ADR. EL1-only. (G-11)
— **code-fix** o **doc-fix**.

**[F-30] MINOR CONFIRMED** — oracle peer-transfer timing-dependent: 64
`yield_now()` non giustificati contro ~40 task di boot e budget fisso 15 s
(`Makefile:64`); un round lento si presenta come assertion mancante, non come
timeout. (G-14/F oracle-health) — **gate-fix** (sentinella «boot completed» o
commento sul 64).

**[F-31] MINOR CONFIRMED** — ADR-0046 §2 nomina `sched::yield_if_budget_expired()`
che non esiste (solo `budget_expired`, `sched/mod.rs:524`); ADR-0053 §lifecycle
dice «generation bump» on exit ma `revoke_task` deliberatamente non bumpa
(lazy al re-mint, equivalente ma meccanismo diverso). (A-3/A-4) — **doc-fix**.

**[F-32] MINOR CONFIRMED** — `SECURITY.md:178-179` cita `tests/model_ipc.rs` /
`tests/model_sched.rs`: il path reale è `crates/kernel-core/tests/`;
`run-mutants.sh` ripete lo stesso path errato. `:177` «no static mut in src/»
sotto-dichiara il gate (scandisce anche `crates/`). (E-10/E-11) — **doc-fix**.

**[F-33] INFO CONFIRMED** — architecture.md `:173` dichiara le Rules 1–4+10
gate-checkate, ma la Rule 2 (BSP bind-only) non è meccanicamente decidibile da
un import-graph; Rule 5 (un solo owner irqchip) non ha gate (conforme oggi:
un solo `irq::init`). Il commento corrente `ipc: refuse count=7` legge come
ledger totale ma conta solo i refusal IPC-send, non quelli per-sessione dei reply
mapper. verification.md ha un claim HW senza data/log (`:1104-1122`) e un
«168 total suite» stale (`:260`). `docs/README.md:111` data «2026-08-07» con
contenuto del 2026-08-08. `0x8000` duplicato come letterale in
`taskcap::lookup` invece di riferire la costante irqcap (un
`const _: assert!` chiuderebbe la classe). (A-6/A-7/C-10/C-14/C-15/D-6/C-13)
— **doc-fix**/**accept-as-is**.

**[F-34] INFO PLAUSIBLE** — nessuna evidenza che le 2 assertion nuove siano
state viste rosse prima di verde (non decidibile read-only; il progetto
altrove registra il red-first). Sotto ADR-0051 (preemption IRQ) andranno
ri-auditate: le 4 regioni mascherate separate di `transfer_held_to_peer`
(lookup e move non atomici tra loro) e la finestra exit/revoke di F-5 — da
aggiungere alla lista «what must be re-audited» di ADR-0051. (G-16/F-13) —
**doc-fix** in ADR-0051.

---

## §5 Findings positivi (con evidenza)

- **Il corpus unsafe è eccezionale.** Parità SAFETY/blocchi **esatta** in ogni
  file caldo (sched 37/37, mmu 18/18, el0 6/6, aspace 9/9, loader 12/12…);
  verdetto «adequate» su tutti i 18 blocchi di `mmu.rs`, tutti i 6 di
  `el0.rs`, 35/37 di `sched` (2 thin, 0 wrong); zero `transmute`, zero
  `static mut`, zero TODO/FIXME nel tree. I commenti SAFETY citano
  precondizioni verificabili e in più casi nominano _l'altro accessor_ che
  falsificherebbe il claim (`mmu.rs:385,:685`) o il bug che li ha motivati.
  L'header di `aspace.rs:39-76` («three things correct today for reasons
  written somewhere else») e il commento `symbol_addr!`
  (`mm/layout.rs:53-69`) sono i migliori artefatti letti in questo audit.
- **L'oracle di boot è sopra lo standard di produzione**: terzo esito
  INDETERMINATE con misura CPU da `/proc/self/stat`, `grep -a` ovunque con
  l'incidente che l'ha motivato, assertion di _ordinamento_ (`empty<sent<got`)
  e non di presenza, nove condizioni negative asserite assenti.
- **La slice 0054 è pulita dove conta**: split puro/impuro corretto (tabella
  in kernel-core, owner IRQ-masked di 38 righe in `src/`), zero unsafe nuovo,
  ordine di exit verificato window-free su single core (revoke prima della
  cascata; nessuna seconda porta d'uscita), disgiunzione delle bande cap
  verificata in tre direzioni con forgery cross-band controllate a mano.
- **doc-claims simulato: tutte e 7 le verifiche verdi**, incluso il conteggio
  353 = 332+12+9 esatto e il code-layout aggiornato per `taskcap` in entrambe
  le regioni.
- **La disciplina «one owner per fact» funziona dove un gate la guarda** — il
  debito di questo audit è quasi interamente nel residuo _fuori_ dallo sguardo
  dei gate, il che è la conferma più forte possibile della strategia dei gate.

## §6 Non-findings (verificato e pulito)

Purezza kernel-core (zero `asm!`/`cfg(target`/volatile, incluso taskcap);
facciata Rule 10 simulata su tutti i 62 file: zero violazioni; Rule 6 (IRQ mai
TX) pulita su entrambi gli handler; Rule 8 WFI con emptiness-check dentro la
maschera; W^X host-tested; ADR-0017 §1/§2, 0019, 0022, 0031, 0032, 0037,
0041, 0046 §1/§3, 0050: conformi clausola per clausola; nessun indice EL0 non
validato raggiunge un array (bound sull'array, non su costante); hold
accounting conservato attraverso transfer; nessun path `Agent` attuale salta
`destroy()`; il ledger corrente `ipc: refuse count=7` non è disturbato dalla
slice (il contatore è machine-wide per gli IPC send, non per-sessione,
verificato); SPSC ring/wake con pairing Acquire/Release corretto
e Miri 2-thread; `use crate::{`: 0 forme nel tree; `/* */`: 0 nel tree (il
buco block-comment dei gate è irraggiungibile); xrefs a tre vie coerenti per
0053/0054.

---

## Remediation aggregata (proposte, in ordine di leva)

**R1 — Estrazioni verso kernel-core** (chiudono strutturalmente F-18/F-5 e la
zona `src/` senza test): (1) tabella cap-slot di sched (~100 LoC pure:
transfer/install/my_cap) → mutabile e model-checkabile; (2) reply mapper di
`agent` (~90 % puri, il blind spot già conceduto da verification.md:1418);
(3) piano manifest→spawn del loader; (4) test kernel-core su `DEVICE_REGIONS`
esportata (F-23); (5) composizione park/timeout/cancel di sched.

**R2 — Gate nuovi a leva alta**: (a) ogni cella `done (QEMU)|done (HW)` della
roadmap deve nominare una stringa oracle presente sia in `qemu-boot-check.sh`
sia in `verification.md` — avrebbe preso F-6 tre commit fa; (b) scope-check di
`run-mutants.sh` da `mutants.json` (F-7); (c) `OPENER` esteso a `irq_save`
(F-13); (d) marker `product-image` da ogni regione `oracle` (F-14); (e) campo
`amended:` negli ADR + diff dei corpi in `xrefs.sh` (F-10); (f) `blobs` in
`check` o ritrattazione tripla (F-9).

**R3 — Tre decisioni che servono un ADR**: delega/attenuazione dei task-cap
(F-3, e con essa F-11/F-12), costanti ABI IPC (F-4), invariante
revoke-on-exit + epoch di spawn (F-5).

**R4 — Passata doc-fix unica** su SECURITY.md (prosa residuale F-1/F-2/F-12),
README (F-22), verification.md (F-6/F-21), commenti numerici (F-15), ADR
minori (F-31/F-32). Un pomeriggio, azzera il grosso della lista.


---

## Postscript — remediation (stesso giorno)

La remediation è stata eseguita il 2026-08-08 (vedi git history del giorno):
ADR-0055/0056/0057/0058 accettati per delega; filtro di banda sul transfer;
oracle `band refused` / `stale refused` / `donor emptied` + assenze
(`STALE-TASKCAP`, `mint FAILED`, `STALE MOVED`); nuovo gate `roadmap-evidence`
(visto rosso su 22 ADR); scope-check di `run-mutants.sh` (visto rosso
sull'artefatto manifest-only); `irq-scope` esteso alle coppie `irq_save` crude
(visto rosso su probe); marker `product-image` estesi alle regioni oracle di
`bootstrap/mod.rs`; `Drop` di `AddressSpace` allineato a `destroy`; lint
`undocumented_unsafe_blocks` su kernel-core; validazione const di
`DEVICE_REGIONS`; passata doc unica su SECURITY.md / README / verification.md
(indice evidenze per ADR) / commenti numerici; convenzione `amended:` con
backfill su 7 ADR. Deferral espliciti registrati in ADR-0049: variante boot a
panic, spawn-epoch pre-SMP, lista mutation derivata, estrazioni R1.
Questo postscript è l'unico aggiornamento del documento; i findings sopra
restano il record datato dell'audit.
