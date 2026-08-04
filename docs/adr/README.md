# Architecture Decision Records

Decisioni strutturali del progetto. Lifecycle (disciplina ADR):

1. Nasce `proposed` con Contesto / Decisione / Conseguenze / Alternative.
2. L'umano accetta (eventualmente con refinement) → `accepted`.
3. Un ADR `accepted` è **immutabile**. Per cambiarlo si crea un successore e si
   marca il vecchio `superseded` (link in `related`).

Numerazione monotona; non rinumerare. Commit preferibilmente separati (`docs:`)
dal codice che ne consegue.

| ID | Titolo | Stato |
| --- | --- | --- |
| [0001](0001-multi-role-analysis.md) | Multi-role analysis as project gate before M3 | proposed |
| [0002](0002-softfloat-kernel.md) | Kernel compiled softfloat, FP left trapping | proposed |
| [0003](0003-early-mmu.md) | MMU enabled before any Rust runs | proposed |
| [0004](0004-gic-group0-firmware-pin.md) | GIC Group 0 with IAR/EOIR, and the firmware pin | proposed |
| [0005](0005-static-page-table-arena.md) | Static page-table arena instead of a frame allocator | proposed |

Review operative (findings, non decisioni): [`../reviews/`](../reviews/).

## Perché 0002–0005 esistono

0001 ha istituzionalizzato *come* si fa review prima che fosse registrato *cosa*
era stato deciso: chi arrivava e chiedeva "perché softfloat?" non aveva risposta.
0002–0005 coprono le quattro scelte che vincolano davvero il codice.

Ognuna nomina **il gate che intercetterebbe la propria inversione**, e per tre di
esse quel gate è stato visto rosso (vedi la tabella delle mutazioni in
[`../verification.md`](../verification.md)). 0005 dichiara di non averne uno: il
suo limite è osservabile solo da chi legge il boot, ed è scritto nell'ADR invece
di essere sottinteso.
