# Excellence review — 2026-08-17

Audit di eccellenza dell'intero progetto secondo [ADR-0001](../adr/0001-multi-role-analysis.md):
solo findings, nessuna decisione, nessuno status (owner dello status resta
[`docs/roadmap.md`](../roadmap.md)). Nessun fix in questa passata. In coda, oltre
ai findings, un **piano di completamento** (§5) richiesto esplicitamente per
questa passata: non modifica la roadmap, la legge.

|                       |                                                                                                                                                                                                                             |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Tree**              | baseline `2d5b8af` (branch `agent/p3-genet-stamp-init-phy`), ottenuta committando il diff doc pendente **prima** di iniziare, per non ripetere la «corsa col tree» del 2026-08-08. Working tree pulito per tutta la passata |
| **Metodo**            | 7 dimensioni read-only (A GENET/root-cause, B confini, C `unsafe`/panic, D copertura, E drift SSOT, F gate/CI/DX, G regressione della passata precedente) + gate host eseguiti in diretta                                   |
| **Eseguito**          | `fmt-check test doc-claims doc-symbols xrefs vocabulary-sync roadmap-evidence layering arch-board-free no-static-mut irq-scope shellcheck mutation-freshness` + `cargo clippy --workspace -- -D warnings` — **tutti verdi** |
| **Non eseguito**      | QEMU boot-check / product-boot-check / panic-check / x86-boot-check / qemu-virtio-check, `make mutants`, deploy SD, boot HW. Ogni claim che dipende da questi è marcato **non valutato**                                    |
| **Riferimento Linux** | `bcmgenet.c` / `bcmgenet.h` (copia locale `/tmp/bcmgenet.c`, 2025). I confronti in §A citano riga per riga la fonte, non la memoria                                                                                         |
| **Verdetti**          | CONFIRMED = evidenza riprodotta sulla baseline; PLAUSIBLE = non refutabile senza silicio                                                                                                                                    |

## Sintesi esecutiva

Il tree è **verde su ogni gate che esiste**, e i numeri dichiarati sono veri:
592 host test contati in diretta (559 + 2 + 3 + 2 + 16 + 10), 237 blocchi
`unsafe` contro 258 commenti `SAFETY` con `undocumented_unsafe_blocks = deny`
a reggere il conto, **zero** `TODO`/`FIXME`/`todo!`/`unimplemented!` in tutto
l'albero tracciato. La remediation del 2026-08-08 regge: il claim «`make check`
è superset della CI» è stato ritrattato in modo esplicito in tre punti
(`Makefile:132-138`, `pull_request_template.md:17`, `CONTRIBUTING.md:26`), e
`roadmap-evidence` / `vocabulary-sync` sono vivi e verdi.

Tre pattern riassumono i 16 findings.

1. **Il fronte attivo è fuori dalla rete di verifica.** `crates/kernel-core/src/genet.rs`
   è il modulo più grande e più modificato del tree — 3142 righe, 51 commit in
   nove giorni — e **non è nella lista di mutation testing** (18 moduli su 58).
   Il gate che dovrebbe accorgersene, `mutation-freshness`, conta i mutanti
   _dentro_ quello scope: è rimasto verde a 660 mentre nasceva un modello
   intero. È F-7 del 2026-08-08 (`taskcap` mai mutato) che si ripete su un
   modulo nuovo, e il trigger di sblocco che ADR-0049 si era dato — «the next
   membership miss» — è scattato senza che nessuno lo vedesse (F-2).
   Peggio: il modello host-testato e il codice che gira sul Pi **hanno
   divergito**. `RingState`, `RingCursor`, `InterruptWork`, `ResetState` hanno
   zero consumatori in `src/`; il boot path fa la propria aritmetica
   produttore/consumatore a mano (F-3). Le 28 prove su `genet.rs` sono prove
   su codice che il silicio non esegue.

2. **Il dead-end GENET non è un mistero: è una sequenza sbagliata, e nove
   giorni di slice a variabile singola non potevano trovarlo** perché ogni
   slice aggiungeva un registro _nello stesso punto sbagliato_. Il confronto
   riga-per-riga con `bcmgenet.c` produce quattro discrepanze strutturali mai
   messe in tabella: UniMAC/TBUF/RBUF programmati **dopo** l'abilitazione del
   DMA anziché prima (F-4); `UMAC_TX_FLUSH` pulsato dopo l'enable e con un
   readback al posto di `udelay(10)` (F-5); due bit di controllo (`DMA_OWN`,
   `DMA_WRAP`) che Linux non mette mai in un BD di TX (F-6); il blocco HFB mai
   azzerato, su una board il cui firmware inizializza GENET per il netboot
   (F-7). E soprattutto: `init_phy` legge BMSR **subito dopo** un reset BMCR,
   quando l'autonegoziazione gigabit non può essere finita — il `link down` del
   boot delle 14:39 non è sfortuna, è l'esito progettato (F-1).

3. **L'onestà documentale ha superato la leggibilità.** La riga di evidenza
   ADR-0105+0106 di `verification.md` è **una singola cella di tabella markdown
   da ~4500 parole** con 25 esperimenti datati dentro (F-10): è la SSOT del solo
   fronte aperto del progetto ed è illeggibile, indiffabile, irrevisionabile.
   Accanto, `SECURITY.md` continua a dichiarare «No network stack» mentre P3 è
   `done (QEMU)` con un servizio di rete EL1 composto (F-9), e la tabella
   §The layers di `verification.md` — la mappa di cosa ogni gate _non_ vede —
   è, per ammissione di `CONTRIBUTING.md:85`, «checked by nobody» (F-13).

Nessun P0: nessun bug di memory-safety aperto, nessun `unsafe` non documentato,
nessun gate rosso. Il debito è concentrato su un solo sottosistema e su tre
gate mancanti.

---

## §1 Metodo

Sette dimensioni read-only sulla baseline `2d5b8af`. La differenza rispetto al
2026-08-08 è che la dimensione A (GENET) è stata condotta **contro il sorgente
Linux reale**, non contro la ricostruzione a memoria del comportamento atteso:
ogni claim di §A cita sia `harbor-kernel` sia `bcmgenet.c` con riga. Questa è
la ragione per cui quattro discrepanze strutturali emergono ora e non nelle 57
slice precedenti: le slice confrontavano _cosa_ Linux scrive, la passata
confronta _quando_ lo scrive.

I gate host sono stati eseguiti in serie (cap CPU locale a 1 core) e sono tutti
verdi; i numeri di questo report vengono da quell'esecuzione, non dai
documenti. I gate build/boot-dependent non sono stati eseguiti e nessun claim
qui sopra li presuppone verdi.

## §2 Matrice di copertura verifica

| Perimetro                                                                                                            | LoC     | Host test         | Mutation               | Miri   | Oracle QEMU     | Stamp HW                  |
| -------------------------------------------------------------------------------------------------------------------- | ------- | ----------------- | ---------------------- | ------ | --------------- | ------------------------- |
| `kernel-core`, 18 moduli in scope                                                                                    | —       | sì                | **sì** (660/22, 08-13) | no¹    | transitivo      | transitivo                |
| `kernel-core`, 40 moduli fuori scope                                                                                 | —       | sì                | **mai**                | no¹    | transitivo      | transitivo                |
| ↳ di cui `genet.rs`                                                                                                  | 3 142   | 28 unit + 2 emul. | **mai**                | no     | nessuna riga²   | negativo (25 esperimenti) |
| ↳ di cui `genet_fdt.rs`, `net.rs`, `virtio.rs`, `paging.rs`, `heap.rs`, `naming.rs`, `sdcard.rs`, `durable_media.rs` | —       | sì                | **mai**                | no     | parziale        | parziale                  |
| `kernel-core::ring`, `::wake`                                                                                        | —       | sì                | fuori scope            | **sì** | —               | —                         |
| `src/` (bare metal)                                                                                                  | ~18 400 | **0**             | n/a                    | n/a    | ~149 asserzioni | sì, per sottosistema      |
| ↳ di cui `src/drivers/genet.rs`                                                                                      | 817     | 0                 | n/a                    | n/a    | **0**²          | negativo                  |

¹ Miri gira solo su `ring::tests` e `wake::tests`, per non sforare il timeout CI — limite auto-dichiarato in `verification.md`.
² QEMU non ha un probe GENET valido: nessuna delle righe `genet:` del boot path è asserita da un oracolo emulato. L'unica evidenza è il silicio, ed è negativa.

## §3 Matrice punti ciechi dei gate

| Gate                                                                        | Punto cieco strutturale                                                                                       | Istanza viva                                         |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| `mutation-freshness.sh`                                                     | conta i mutanti **dentro** la FILES list di `run-mutants.sh`: un modulo fuori lista può crescere all'infinito | **sì**: `genet.rs` +3142 LoC, gate verde a 660 (F-2) |
| `run-mutants.sh`                                                            | membership della FILES list è a mano (18/58 moduli); ADR-0049 lo sapeva e ha differito                        | **sì**: 40 moduli mai mutati (F-2)                   |
| `roadmap-evidence.sh`                                                       | esige _una riga_ di evidenza per ogni `done`, non che la riga sia leggibile o scomponibile                    | **sì**: cella da 4500 parole (F-10)                  |
| `doc-claims.sh`                                                             | set e conteggi, mai semantica — punto cieco già auto-documentato                                              | **sì**: `SECURITY.md` «No network stack» (F-9)       |
| §The layers di `verification.md`                                            | è la mappa dei punti ciechi e **non ha un gate proprio** (`CONTRIBUTING.md:85` lo ammette)                    | non misurata (F-13)                                  |
| `hw-transcript-check.sh`                                                    | assert su un file che non è nel repo (`.serial-log/` è in `.gitignore`)                                       | **sì**: 53 transcript locali, 0 tracciati (F-12)     |
| `layering.sh`                                                               | edge di import, non decisioni duplicate tra modello puro e driver                                             | **sì**: modello e driver divergono senza edge (F-3)  |
| `no-static-mut`, `irq-scope`, `arch-board-free`, `xrefs`, `vocabulary-sync` | punti ciechi 2026-08-08 invariati                                                                             | nessuna nuova istanza rilevata                       |

`make check` **non** è più dichiarato superset della CI senza qualificazione: la
ritrattazione di F-9/2026-08-08 regge in tutti e tre i punti. `miri` e
`shellcheck` continuano a fallire duri in CI e a richiedere un opt-in esplicito
in locale.

---

## §4 Findings

Formato ADR-0001: `[F-n] SEVERITÀ VERDETTO — claim — evidenza — azione proposta — effort`.
La classe di remediation è una proposta; l'accettazione è umana.

### P1 — debito che blocca il gate ADR-0105 o regredisce un gate

**[F-1] P1 CONFIRMED — `init_phy` pretende link-up subito dopo un reset BMCR: il boot path garantisce strutturalmente `link=down`.**
`src/drivers/genet.rs:712-725` (`reset_phy`) fa `mdio_write(BMCR, BMCR_RESET)` e poi attende **solo** che il bit RESET si auto-cancelli. `init_phy` (`:730-735`) legge BMSR immediatamente dopo e chiama `require_up()`. Un reset BMCR riporta il PHY ai default e fa ripartire l'autonegoziazione, che su 1000BASE-T impiega secondi; l'attesa è sul bit sbagliato (`BMCR_RESET`, non `BMSR_ANEG_DONE`/`BMSR_LINK`) e comunque di ordini di grandezza troppo corta. Linux non fa nulla di simile: consegna il PHY a `bcmgenet_mii_probe` (`bcmgenet.c:3403`) e poi a `phy_start` (`:3348`), e il link-up arriva più tardi come evento.
**Impatto**: la regressione stampata oggi (`tx cons len=124` → `tx unavailable (link down)`, boot 14:39, `src=3f2d01b8`) non è un caso: è l'esito progettato. Finché `init_phy` sta dove sta, **ogni** boot rifiuta TX e RX prima del doorbell e il gate di evidenza ADR-0105 è irraggiungibile.
**Azione**: ADR — «acquisizione del link» è una decisione di confine (chi aspetta, con quale bound, cosa significa attendere un link in un boot path che non deve appendersi). **Effort M.**

**[F-2] P1 CONFIRMED — il modulo su cui vive tutto lo sforzo GENET non è mai stato mutato, e il gate che dovrebbe accorgersene è cieco per costruzione.**
`scripts/host/run-mutants.sh:84-87`: `FILES = (ipc tasks layout irqtable rxline reset cap syscall prog manifest taskcap irqcap reply runqueue irqwait capslots lifecycle loaderplan)` — **18 moduli su 58**. Fuori: `genet.rs` (3142 LoC), `genet_fdt.rs`, `net.rs`, `virtio.rs`, `paging.rs`, `heap.rs`, `naming.rs`, `sdcard.rs`, `durable_media.rs`. `scripts/check/mutation-freshness.sh` confronta il conteggio di `cargo mutants --list` **ristretto a quella lista** con lo stamp: eseguito oggi, `clean (660 mutants, run 2026-08-13 at 1e4a235)`, mentre fra quello stamp e la baseline sono entrati 51 commit su `genet.rs`.
È F-7 del 2026-08-08 che si ripete. E [ADR-0049:28](../adr/0049-deferred-residuals.md) aveva differito la lista derivata con condizione di sblocco esplicita — _«Marker-derived list, or the next membership miss»_: **il trigger è scattato e non c'è nulla che lo osservi**.
**Azione**: gate (lista derivata da marker, o quantomeno `genet*` in FILES + un run) e aggiornamento di ADR-0049. **Effort S** per la membership, **M** per la lista derivata.

**[F-3] P1 CONFIRMED — il modello puro host-testato e il codice che gira sul silicio hanno divergito.**
`RingState` (`crates/kernel-core/src/genet.rs:909`), `RingCursor` (`:984`), `InterruptWork` (`:1012`), `ResetState` (`:1036`) hanno **zero riferimenti in `src/`**. Il boot path fa la propria aritmetica: `submit_one_tx` (`src/drivers/genet.rs:415-456`) scrive `PROD_INDEX = 1` come letterale e classifica cons/idle con helper `const fn` separati (`TxReport::cons_is_idle`/`cons_has_posted`), senza passare dallo stato del modello. Stessa storia per `queue_supported`, `umac_cmd_with_speed`, `tbuf_raw_frame`, `umac_tx_pkts_packed`, `umac_tx_pok`, `DMA_ARBITER_RR/SP`: zero consumatori sia in `src/` sia nei test di integrazione.
**Impatto**: `verification.md:125` elenca quel modello come l'evidenza host di ADR-0105/0106. È evidenza su codice non eseguito. Il crate puro non governa più il driver: lo accompagna.
**Azione**: ADR + estrazione (il driver deve consumare il modello, o il modello va potato a ciò che il driver usa). **Effort M–L.**

**[F-4] P1 CONFIRMED — UniMAC, TBUF e RBUF sono programmati _dopo_ l'abilitazione di entrambi i motori DMA; Linux fa tutto prima.**
`src/drivers/genet.rs:355-383` (`boot_after_program`): `enable_queue0()` scrive TDMA/RDMA `DMA_CTRL` alla riga 363, e **solo dopo** arrivano `program_rgmii_oob` (:378), `program_umac_init` (:379), `program_tbuf_tsb` (:380), `program_rbuf_tbuf_size` (:381), `program_rbuf_64b` (:382), `program_rbuf_chk` (:383).
Linux: `bcmgenet_open` → `init_umac()` (`bcmgenet.c:2602-2660`: MIB reset, `UMAC_MAX_FRAME_LEN`, `TBUF_64B_EN`, `RBUF_ALIGN_2B|RBUF_64B_EN`, `RBUF_CHK_CTRL`, `RBUF_TBUF_SIZE_CTRL`) → `bcmgenet_set_hw_addr` → `bcmgenet_hfb_init` → `bcmgenet_init_dma()`, che programma gli anelli e abilita `DMA_EN` **per ultimo** (`:3172-3180`).
**Impatto**: cambiare la modalità TSB di TBUF e i bit RBUF mentre TDMA è già abilitato è esattamente la classe di manovra che incaglia silenziosamente il datapath. Nessuna delle 25 slice ha testato questa variabile, perché ogni slice aggiungeva un registro **nello stesso punto sbagliato**.
**Azione**: fix (riordino), un boot. **Effort S.**

**[F-5] P1 CONFIRMED — `UMAC_TX_FLUSH` è pulsato nel posto sbagliato e con un readback al posto di un ritardo; stessa classe per il latch RBUF.**
`src/drivers/genet.rs:547-552` (`pulse_tx_flush`) usa `let _ = read32(...)` come settle fra l'assert e il deassert, ed è chiamato da `program_umac_init` (`:644`), quindi **dopo** l'abilitazione DMA. Linux pulsa `UMAC_TX_FLUSH` dentro `bcmgenet_init_dma`, **prima** di programmare qualunque anello e prima di `DMA_EN`, con `udelay(10)` fra le due scritture (`bcmgenet.c:3113-3115`); `init_umac` non tocca mai `TX_FLUSH`.
Stessa classe, due volte ancora: `reset()` scrive `RBUF_CTRL = 0` senza settle (`src/drivers/genet.rs:133`) dove `reset_umac` ha `udelay(10)` (`bcmgenet.c:2562-2563`); e `bcmgenet_umac_reset` (`:3299-3311`) pulsa `RBUF_CTRL` BIT(1) con `udelay(10)` su **entrambi** i fronti — passo per cui Harbor non ha alcun analogo, benché Linux lo chiami «take MAC out of reset» prima di `init_umac`.
**Azione**: fix + una riga in ADR-0106: _una rilettura di un registro Device non è un settle_. **Effort S.**

**[F-6] P1 PLAUSIBLE — il descrittore TX di Harbor porta due bit di controllo che Linux non mette mai in un BD di trasmissione.**
`src/drivers/genet.rs:795-807` chiama `Descriptor::words(Ownership::Device, start=true, end=true, wrap=true)`, che produce `DMA_OWN | DMA_SOP | DMA_EOP | DMA_WRAP` (`crates/kernel-core/src/genet.rs:791-812`), poi vi applica `tx_desc_status` (APPEND_CRC + qtag). Linux `bcmgenet_xmit` costruisce `len_stat = (size << DMA_BUFLENGTH_SHIFT) | (qtag_mask << DMA_TX_QTAG_SHIFT) | DMA_TX_APPEND_CRC | DMA_SOP | DMA_EOP` — **senza `DMA_OWN` e senza `DMA_WRAP`** (`bcmgenet.c:2184-2200`).
Il tree sa già che l'OWN non viene riscritto dal silicio in TX (`crates/kernel-core/src/genet.rs:1874-1875`, esperimento `src=fa00d083`) e continua comunque a impostarlo. `DMA_WRAP` non è mai stato nominato in nessuna slice.
**PLAUSIBLE** e non CONFIRMED perché il datasheet non dice se bit indefiniti in un BD TX siano ignorati o fatali; è però una variabile singola, mai provata.
**Azione**: fix, un boot. **Effort S.**

**[F-7] P1 CONFIRMED — il blocco HFB non viene mai azzerato, su una board il cui firmware inizializza GENET per il netboot.**
`grep -r 'HFB\|hfb' src crates` → **0 occorrenze**. Linux chiama `bcmgenet_hfb_init(priv)` in `bcmgenet_open` prima di `init_dma` (`bcmgenet.c:3380`, definizione `:743`, che azzera `HFB_CTRL` a `:724`), e in `bcmgenet_netif_stop` riscrive `HFB_CTRL = 0` sotto il commento _«Disable MAC receive»_ (`:3438`) — cioè il filtro hardware è, per Linux, l'interruttore della ricezione.
**Impatto**: candidato vivo e indipendente dal TX per spiegare perché nessuna RX arriva mai. Harbor eredita lo stato che il firmware ha lasciato e non lo osserva né lo azzera.
**Azione**: fix / emendamento ADR-0106 (il contratto di reset deve nominare HFB). **Effort S.**

**[F-8] P1 CONFIRMED — l'anello TX 0 è programmato con un solo descrittore mentre gli anelli 1–4 sono posizionati come se ne avesse 128.**
`src/drivers/genet.rs:219-238`: `RingProgram::new(TDMA, queue, first=0, count=1, buffer_bytes=tx.length)`. `program_priority_tx_rings` (`:273-287`) colloca gli anelli 1–4 a BD 128/160/192/224 via `v5_priority_tx_first`, cioè **sull'assunzione che l'anello 0 possieda i BD 0..127** (`crates/kernel-core/src/genet.rs:26`, `V5_Q0_TX_BD_CNT = 128`). Linux programma l'anello 0 con `size = GENET_Q0_TX_BD_CNT` e scrive `DMA_RING_BUF_SIZE = (size << DMA_RING_SIZE_SHIFT) | RX_BUF_LENGTH` — la lunghezza costante 2048, non quella del pacchetto (`bcmgenet.c:2730-2733`, `:2930-2947`).
**Azione**: fix. **Effort S.**

### P2 — qualità, DX, onestà dei claim

**[F-9] P2 CONFIRMED — `SECURITY.md` dichiara ancora «No network stack».**
`SECURITY.md:113` (tabella minacce, riga _Remote network exploit_) e `:55` (non-assets: «network stack»). Nel frattempo P3 è `done (QEMU)` con transport, servizio pacchetti EL1 e cap direzionali (`docs/roadmap.md:104`, ADR-0104), e l'albero contiene `src/bootstrap/network_server.rs` e `network_runtime.rs`. La tabella §Today dello stesso file (`:46`) è aggiornata e cita ADR-0104 — quindi il file si contraddice da solo, esattamente la forma di F-1 del 2026-08-08, nello stesso file.
**Azione**: doc-fix, oppure una frase esplicita «fuori dal threat model _di proposito_, e perché». **Effort S.**

**[F-10] P2 CONFIRMED — la riga di evidenza ADR-0105+0106 è una singola cella markdown da ~4500 parole.**
`docs/verification.md:125`. Contiene 25 esperimenti su silicio datati, ciascuno con stamp, `src=`, riga seriale e conclusione negativa. È la SSOT del solo fronte aperto del progetto, ed è illeggibile in un editor, indiffabile in una PR e irrevisionabile. Tutti i 57 commit del bring-up l'hanno toccata. `roadmap-evidence` è verde perché esige _una riga_, non una riga leggibile.
**Azione**: ristrutturare in una sezione datata come le altre sezioni «Hardware evidence», lasciando nella cella un puntatore. **Effort M.**

**[F-11] P2 CONFIRMED — l'affermazione di ADR-0049 che ciò che resta fuori dalle reti di test in `src/` è «mechanism — MMIO, assembly, lock discipline — not decisions» è oggi falsa.**
[ADR-0049:29](../adr/0049-deferred-residuals.md). `src/drivers/genet.rs` sono 817 righe di **decisioni**: l'ordine di init (F-4), quando asserire `RGMII_LINK`, quando armare `RX_EN`, cosa `boot_after_program` sequenzia, quale settle è accettabile (F-5). Nessuna di queste è meccanismo, e nessuna è testata.
**Azione**: ADR successore o emendamento a 0049. **Effort S.**

**[F-12] P2 CONFIRMED — `.serial-log/` è in `.gitignore`: i transcript che sostengono ogni claim `done (HW)` non sono nel repo.**
`.gitignore:25`; `git ls-files .serial-log` → 0; 53 log presenti solo su disco. `make hw-check TRANSCRIPT=…` opera su un file che nessun clone possiede, e le prove citate in `verification.md` sopravvivono solo come estratti incollati a mano.
**Azione**: decisione — tracciare i transcript citati, oppure scrivere esplicitamente che la copia markdown _è_ il record e il file è effimero. **Effort S.**

**[F-13] P2 CONFIRMED — la tabella §The layers di `verification.md`, che è la mappa dei punti ciechi, non ha un gate.**
`CONTRIBUTING.md:85` lo dichiara per iscritto: _«…which claims to list every layer and is checked by nobody»_. Ci sono 13 script in `scripts/check/` più gli oracoli di boot; nulla verifica che ognuno compaia nella tabella con il proprio «Blind to».
**Azione**: gate (ogni `scripts/check/*.sh` e ogni target di `check` deve avere una riga). **Effort S.**

**[F-14] P2 CONFIRMED — quattro `expect` sull'allocatore di frame nel codice di rete di produzione.**
`src/bootstrap/network_runtime.rs:508, 517, 531, 540` (`"packet page allocated"`, `"DMA packet page allocated"`). L'esaurimento del pool di frame diventa un panic del kernel invece di un rifiuto bounded, sul path P3 che è `done (QEMU)`. Il contrasto è nello stesso albero: `program_held_queue0` (`src/bootstrap/mod.rs:526-532`) tratta lo stesso `None` come `Queue0Report::NoFrames`.
**Azione**: fix. **Effort S.**

### P3 — nice-to-have / risk-accepted

**[F-15] P3 CONFIRMED — 47 branch remoti e 15 locali `agent/*`, tutti già merge-ati.** Rumore su ogni `git branch -a`. **Azione**: potatura. **Effort S.**

**[F-16] P3 Risk-accepted — `src/arch/x86_64/el0.rs:23,29,37` sono tre `panic!("… not implemented")`.** Path lab H3 L0, coerente con «L1+ non iniziato». Va bene finché resta _dichiarato_: oggi non compare in nessuna tabella di residui. **Azione**: una riga in ADR-0049 o risk-accepted esplicito. **Effort S.**

### Verificato e sano (dimensione G)

- La ritrattazione del claim «superset della CI» regge in tutti e tre i punti (`Makefile:132-138`, `pull_request_template.md:17`, `CONTRIBUTING.md:26`), con il motivo scritto accanto.
- `undocumented_unsafe_blocks = deny` regge: 237 blocchi `unsafe`, 258 `SAFETY`, zero `allow` di quel lint.
- Zero `TODO`/`FIXME`/`XXX`/`HACK`/`todo!`/`unimplemented!` in tutto l'albero tracciato.
- I 19 `panic!` in `src/` sono tutti invarianti difensivi o stub dichiarati (8 in `arch/aarch64/el0.rs`, 3 negli stub x86, 2 nei fault non gestibili, 6 sparsi in mmu/console/early).
- Solo 7 `expect`/`unwrap` fuori dai test in tutto `src/`; 4 sono F-14, 2 in `genet.rs:275,283` sono provabilmente infallibili (dominio `1..=4` costante), 1 in `bsp/qemu_virt/network.rs:37` è garantito dal loop che lo precede.
- I numeri dichiarati sono veri: 592 host test contati in diretta, 660 mutanti confermati dal gate.
- `roadmap-evidence`, `vocabulary-sync`, `irq-scope` esteso e i marker `product-image` — i gate nati dalla remediation 2026-08-08 — sono vivi e verdi.

---

## §5 Piano di completamento

Cosa manca perché Harbor si possa dire finito. Per ogni residuo: stato, cosa
manca, **criterio di chiusura falsificabile**, dipendenze, effort. Questa
sezione non modifica lo status: lo legge da `docs/roadmap.md`.

### 5.1 P3 — backend GENET su Pi 4 (unico fronte attivo)

**Stato**: ADR-0105/0106 `proposed`. Probe, PHY id, programmazione anelli,
WRR, RGMII OOB, UniMAC, TBUF/RBUF e `RBUF_CHK_CTRL` sono scritti; nessun frame
è mai comparso sul filo; da PR #77 il link cade e TX/RX si rifiutano prima del
doorbell.

**Cosa manca** (gate di evidenza ADR-0105, testuale): una cattura seriale su Pi
4 reale con _probe, link state, una TX bounded, una RX bounded,
reset/recovery e rifiuto di device assente_. Oggi mancano: **la TX sul filo**
(il CONS DMA non è una trasmissione: TSV `0x49c`/`0x4a8`/`0x4ec` a zero, pcap
senza `0x88b5`), **la RX**, e **l'absent-device refusal su Pi**.

**Criterio di chiusura**: un pcap sull'host che contiene un frame con
EtherType `0x88b5` e SA `02:00:00:00:00:01`, **e** un TSV UniMAC diverso da
zero nello stesso boot. Fino ad allora `raspi4b` non ha un backend NIC.

**Raccomandazione di metodo — la decisione strategica di questa passata.**
Le 25 slice a variabile singola hanno esaurito il loro potere esplicativo:
tutte aggiungevano un registro nello stesso punto della sequenza, e §A mostra
che il punto è sbagliato. L'ordine consigliato non è «la prossima variabile»
ma:

| #   | Passo                                                                                                                                            | Perché prima                                                                                    | Effort        |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------- | ------------- |
| 1   | **F-1** — togliere `require_up()` immediato dopo il reset BMCR e dare al link un'attesa bounded esplicita (o non resettare il PHY sul boot path) | senza questo ogni boot successivo rifiuta prima del doorbell e non misura nulla                 | M (serve ADR) |
| 2   | **F-4** — riordinare: UniMAC/TBUF/RBUF/HFB **prima** di `DMA_EN`                                                                                 | una sola riorganizzazione copre 6 registri già scritti; è la discrepanza strutturale più grande | S             |
| 3   | **F-5** — `TX_FLUSH` in `init_dma` con ritardo reale, latch RBUF con ritardo reale                                                               | stesso boot del passo 2                                                                         | S             |
| 4   | **F-7** — azzerare HFB nel reset                                                                                                                 | unico candidato mai provato che spiega la RX                                                    | S             |
| 5   | **F-6** — togliere `DMA_OWN`/`DMA_WRAP` dal BD di TX                                                                                             | variabile singola, costo nullo                                                                  | S             |
| 6   | **F-8** — anello 0 con 128 BD e `RING_BUF_SIZE` alla Linux                                                                                       | rimuove l'incoerenza interna con gli anelli 1–4                                                 | S             |
| 7   | **F-2** — `genet*` nello scope di mutation + un run                                                                                              | prima di dichiarare qualunque cosa `done` su questo modulo                                      | S             |
| 8   | **F-3** — far consumare al driver il modello puro, o potare il modello                                                                           | chiude la divergenza che rende l'evidenza host non pertinente                                   | M–L           |

Se dopo i passi 1–6 il TSV resta a zero, la classe di ipotesi rimanente non è
più «un registro mancante» ma clock/alimentazione del blocco RGMII o stato
lasciato dal firmware — e va aperta come ipotesi esplicita in ADR-0106, non
cercata a tentativi.

### 5.2 Residui kernel (H2)

| Residuo                 | Stato                                         | Cosa manca                                                           | Criterio di chiusura                                                                                                   | Effort     |
| ----------------------- | --------------------------------------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | ---------- |
| **K5-H** (no slot wall) | `held` con trigger numerico (ADR-0085 §3)     | nulla: il picco misurato è 8/57 QEMU, 9/57 Pi                        | il trigger scatta quando `oracle-census` misura un picco vicino al tetto. **Verificato oggi: ancora lontano**          | —          |
| **K5-B code**           | design `accepted` (ADR-0089), codice deferred | implementazione pair-collapse                                        | stesso trigger di K5-H                                                                                                 | L          |
| **K7-M / K7-T / K7-R**  | gated da ADR-0084, issue #21                  | lab di switch-cost (M), TTBR1 (T), rollover ASID sotto pressione (R) | K7-M: una misura pubblicata. K7-T: il trigger numerico dell'ADR. K7-R: un boot che esaurisce lo spazio ASID e recupera | M ciascuno |
| **agent+TLB steal**     | residuo K8                                    | work stealing che sposta un agente con il suo TLB                    | uno stamp HW dual-core che mostra un agente migrato e nessuna staleness                                                | M          |
| **cores 2–3**           | mai attivati                                  | unpark + scheduling su 4 core                                        | `smp_seen=4` su silicio con quantum su tutti                                                                           | M          |

### 5.3 Residui prodotto

| Residuo                      | Stato                                                     | Cosa manca                                                                                       | Criterio di chiusura                                                                                                                                                          | Effort           |
| ---------------------------- | --------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------- |
| **P4 display/input**         | `open`, deferred (ADR-0049), pannello ritirato (ADR-0094) | **una decisione, non del codice**: P4 è aperto dal 2026-08 senza composition target e senza data | o un ADR di composizione UI, o un ADR che lo dichiara **non-goal permanente**. Una riga di roadmap che resta aperta per sempre è la forma che questo progetto rifiuta altrove | S (la decisione) |
| **P6 compose/audit tooling** | `done (QEMU)`                                             | mai esercitato su HW                                                                             | un `pack`/`inject`/`inspect-store` su un'immagine che poi bootta sul Pi, con stamp                                                                                            | S–M              |
| **H3 L1+**                   | L0 x86 `done (QEMU)`; L1+ non iniziato                    | ADR separati; oggi 3 `panic!("not implemented")` in `arch/x86_64/el0.rs`                         | fuori scope di questo ciclo; va però nominato in ADR-0049 (F-16)                                                                                                              | L                |

### 5.4 Debito di processo

| Voce                              | Criterio di chiusura                                                                              | Effort |
| --------------------------------- | ------------------------------------------------------------------------------------------------- | ------ |
| F-2 scope mutation                | `genet*` mutato almeno una volta, e un meccanismo che rende impossibile una nuova membership miss | S–M    |
| F-13 gate sulla tabella dei layer | uno script che fallisce se un `scripts/check/*.sh` non ha riga                                    | S      |
| F-12 transcript                   | decisione tracciato/effimero, scritta                                                             | S      |
| F-10 cella da 4500 parole         | la riga ADR-0105 rientra in una cella leggibile                                                   | M      |
| F-9 `SECURITY.md`                 | la tabella minacce e i non-assets nominano P3                                                     | S      |
| F-15 branch                       | `agent/*` merge-ati potati                                                                        | S      |
| issue #28                         | CPU garantita per l'oracolo QEMU in CI                                                            | S      |

### 5.5 Ordine consigliato

1. **F-1 → F-8** nell'ordine di §5.1: sbloccano il solo fronte attivo e sono in gran parte un pomeriggio più i boot.
2. **F-2 + F-13**: i due gate mancanti. Vanno _prima_ di dichiarare `done` qualunque cosa di GENET, altrimenti il prossimo `done (HW)` nasce senza rete come è nato questo modulo.
3. **Passata doc unica**: F-9, F-10, F-11, F-12, F-16. Un pomeriggio, azzera la coda P2.
4. **F-3**: la divergenza modello/driver, dopo che la sequenza funziona — riscrivere l'aritmetica mentre si cerca un bug di sequenza aggiungerebbe una variabile.
5. **P4**: la decisione, non il codice.
6. Residui K, secondo i loro trigger, che oggi non sono scattati.

### 5.6 Cosa richiede un ADR prima del codice

- **F-1** — l'acquisizione del link è un confine: chi attende, con quale bound, e cosa il prodotto stampa mentre attende.
- **F-3** — se il driver debba consumare il modello puro o il modello vada potato è una decisione di architettura, non un refactor.
- **F-11** — ADR-0049 successore: la frase «fuori dalle reti c'è solo meccanismo» va ritirata o riqualificata.
- **P4** — composizione UI o non-goal permanente.

---

## §6 Remediation aggregata, in ordine di leva

**R1 — Sequenza GENET (F-4, F-5, F-7, F-6, F-8).** Cinque fix piccoli e
indipendenti che si possono raggruppare in due boot invece che in cinque
slice, perché nessuno dei cinque è «una variabile in più nello stesso punto»:
sono un riordino, due settle reali, un blocco mai toccato e due bit di
troppo. È la leva più alta del tree.

**R2 — Gate mancanti (F-2, F-13).** (a) `genet*` nello scope di
`run-mutants.sh` più un run, e la lista derivata da marker che ADR-0049 aveva
già differito con trigger — trigger che è scattato; (b) un gate che esige una
riga in §The layers per ogni checker. Entrambi avrebbero preso il debito di
questa passata mesi prima.

**R3 — Quattro decisioni che servono un ADR**: acquisizione del link (F-1),
proprietà modello/driver (F-3), la frase di ADR-0049 sul «solo meccanismo»
(F-11), e P4 come composizione o non-goal.

**R4 — Passata doc unica (F-9, F-10, F-12, F-16, F-15).** `SECURITY.md`,
la cella da 4500 parole, la decisione sui transcript, gli stub x86 in
ADR-0049, la potatura dei branch. Un pomeriggio, azzera la coda P2/P3.

---

## Postscript — remediation (stesso giorno)

Eseguita il 2026-08-17, in ordine di leva.

**Decisioni.** [ADR-0107](../adr/0107-genet-sequence-first-bring-up.md) (metodo:
l'unità di esperimento diventa una claim di sequenza) e
[ADR-0108](../adr/0108-boot-path-link-acquisition.md) (il boot path non resetta
il PHY; un settle è bounded in millisecondi contro `CNTFRQ_EL0`), entrambe
`proposed`. [ADR-0049](../adr/0049-deferred-residuals.md) emendata: la lista di
mutation derivata è chiusa dal proprio trigger, e la frase «fuori dalle reti
c'è solo meccanismo» è ritirata (F-11).

**Gate (F-2, F-13).** `make mutation-scope` — lo scope vive in
`docs/mutation-scope.toml` con tre stati (`in_scope` / `queued` / `exempt`),
ogni modulo di `kernel-core` deve averne uno, e `run-mutants.sh` e
`mutation-freshness.sh` derivano la lista da lì invece di tenerne due copie.
Visto rosso su `genet`/`genet_fdt`. `genet` e `genet_fdt` entrano in scope:
660 mutanti diventano 1515. `make layers-table` — ogni prerequisito di `check`
deve avere una riga in §The layers con un «Blind to» non vuoto; visto rosso su
`vocabulary-sync`, che mancava da sempre.

**Codice (F-1, F-4, F-5, F-6, F-7, F-8).** Il reset del PHY esce dal boot path.
La sequenza di init passa **prima** di `DMA_EN`, i settle diventano attese reali
su `CNTFRQ_EL0`, `bcmgenet_hfb_clear` viene implementato (non era mai stato
toccato), il BD di TX perde `DMA_OWN`/`DMA_WRAP`, il BD di RX torna
address-only, l'anello 0 prende 128 BD in TX e 256 in RX, e `XON_XOFF_THRESH`
smette di essere zero. Sei nuovi host test sull'aritmetica HFB. 598 host test.
Silicio non pagato: ADR-0105/0106 restano `proposed`.

**Doc (F-9, F-14).** `SECURITY.md` non dice più «No network stack»: la riga
minacce e i non-assets distinguono l'assenza di uno stack IP dalla presenza di
un servizio pacchetti EL1 host-testato.

**Correzione a F-14.** Il finding sopravvaluta il difetto: i quattro `expect`
di `network_runtime.rs` erano **irraggiungibili**, perché la guardia
`if page.is_none() { return None }` li precede. Non erano un panic in attesa.
Sono stati comunque rimossi — la via d'uscita per esaurimento è ora un rifiuto
bounded nella forma oltre che nel fatto — ma la severità corretta era P3, non
P2. Il finding resta sopra come record datato dell'audit; questa riga è la
rettifica.

**Non fatto in questa passata**: F-3 (divergenza modello/driver — va dopo che
la sequenza funziona, non durante), F-10 (la cella da 4500 parole), F-12
(decisione sui transcript), F-15 (potatura branch), F-16 (stub x86 in
ADR-0049), e il piano di completamento oltre P3.
