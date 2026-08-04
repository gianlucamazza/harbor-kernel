---
id: 0001
title: Multi-role analysis as project gate before M3
status: proposed
date: 2026-08-04
---

# ADR-0001: Multi-role analysis as project gate before M3

## Contesto

`rpi_minimal_agentic` ha superato i milestone di bring-up fino a P2 (early MMU,
softfloat, gate build-enforced, heap free-list, W^X, idle WFI) ed è pronto, sul
piano delle dipendenze dichiarate, per M3 (cooperative tasks).

La superficie di fallimento del kernel bare-metal è asimmetrica:

- QEMU/TCG non riproduce il comportamento degli esclusivi su Device-nGnRnE; un
  green `make boot-check` non è prova su memory attributes, cache o stato
  firmware (vedi [`verification.md`](../verification.md)).
- Le protezioni (W^X, guard page) valgono solo se qualcuno le ha viste sparare
  su hardware; un map che “si attiva” non dimostra enforcement.
- Le regole di layering in [`architecture.md`](../architecture.md) sono
  esplicite ma non enforce-ate da tooling: restano disciplina + review umana.
- Prima di introdurre un’astrazione di esecuzione (task / yield / scheduler),
  scelte non riesaminate rischiano di solidificarsi sotto M3.

I gate automatici restano necessari e insufficienti. Serve un inventario
multi-prospettiva, ripetibile, che produca azioni o *accepted risk* espliciti —
non una code review monolitica one-off.

## Decisione

Adottare una **review multi-ruolo a ruoli fissi** come disciplina di progetto.

### Cadenza

1. **Baseline completa** prima di M3 (prima esecuzione: report in
   [`docs/reviews/`](../reviews/)).
2. **Re-run incrementale** sui diff che toccano memoria, IRQ/`unsafe`, boot
   chain o confini di layering, prima di marcare un milestone `done (HW)`.
3. Findings di tipo *architectural boundary* (confini, modello di sicurezza,
   ABI) → **ADR dedicato** prima del codice che li implementa.

### Ruoli fissi

| ID | Ruolo | Focus |
| --- | --- | --- |
| R1 | Architetto di layering | Regole arch/`bsp`/`drivers`/`irq`/`exception` |
| R2 | Memoria / MMU | Early map, W^X, layout, heap, tabelle |
| R3 | Interrupt / concorrenza / idle | GIC, timer, ring, atomics, WFI |
| R4 | Audit `unsafe` e panic | Inventario, invarianti, halt path |
| R5 | Verifica e blind spot | Gate, CI, cosa un green non prova |
| R6 | Boot chain e firmware | EL2→EL1, blobs, DTB, deploy |
| R7 | Prestazioni e footprint | Size, latenza, idle, alloc (misurate) |
| R8 | Tooling / CI / DX | Makefile, script, toolchain, onboarding |
| R9 | Sicurezza pre-agent | Superficie EL1, MMIO, prerequisiti cap |
| R10 | Roadmap agent (M3–M6) | Gap readiness, non design fantasy |
| R11 | Documentazione | Drift docs↔code, honest claims |
| R12 | API `kernel-core` | Pure logic, testabilità, confini |

### Tassonomia findings

Per ogni ruolo: **problemi**, **migliorie**, **ottimizzazioni** (queste ultime
solo con metrica o ipotesi falsificabile).

Severità:

| Tag | Significato |
| --- | --- |
| `P0` | Correttezza, hang HW, safety |
| `P1` | Debito che blocca M3+ o regredisce un gate |
| `P2` | Qualità / DX |
| `P3` | Nice-to-have |
| `Risk-accepted` | Visto, deliberatamente non fixato, motivato |

### Artefatti

| Cosa | Dove |
| --- | --- |
| Questa decisione | `docs/adr/0001-…` (immutabile una volta `accepted`) |
| Indice ADR | [`README.md`](README.md) |
| Esito di una passata | `docs/reviews/YYYY-MM-DD-multi-role.md` |
| Decisioni strutturali derivate | ADR `0002+` |

L’ADR di processo **non** elenca i bug. I findings vivono nel report; se un
finding cambia un confine o un modello, diventa un ADR successivo.

### Formato finding (nel report)

```markdown
### [P1] R2 — titolo corto
- **Aspetto:** problema | miglioria | ottimizzazione
- **Evidenza:** path:line o citazione doc / comportamento osservato
- **Impatto:** …
- **Azione proposta:** fix | ADR | test | risk-accepted
- **Effort:** S | M | L
```

## Conseguenze

**Positive**

- Tracciabilità: ogni finding ha ruolo, evidenza, severità, next step.
- Allinea la review umana ai blind spot già documentati dei gate automatici.
- Istituzionalizza un checkpoint pre-milestone (in particolare pre-M3).
- Separa processo (questo ADR) da esito (report) e da decisioni puntuali (ADR
  successori).

**Negative / costi**

- Costo tempo (ordine di 1–2 sessioni per passata completa); mitigato da
  checklist per ruolo e da re-run solo sui diff rilevanti.
- Rischio *review theater*: tanti finding senza backlog. Mitigazione: ogni
  `P0`/`P1` richiede azione o `Risk-accepted` esplicito; un ruolo può chiudere
  con “nessun finding materiale” + rationale.

## Alternative considerate

| Alternativa | Perché non scelta |
| --- | --- |
| Solo CI / gate automatici | Ciechi su attributes, cache, firmware, layering, roadmap |
| Code review monolitica single-role | Perde prospettive (sicurezza vs size vs latenza IRQ) |
| Audit esterno una tantum | Non istituzionalizza la disciplina pre-milestone |
| Formal methods / model checking ora | ROI basso pre-M3; host tests + Miri su `kernel-core` bastano come base |

## Riferimenti

- [`docs/architecture.md`](../architecture.md) — layering e milestone
- [`docs/verification.md`](../verification.md) — gate e blind spot
- Prima passata: [`docs/reviews/2026-08-04-multi-role.md`](../reviews/2026-08-04-multi-role.md)
