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

Review operative (findings, non decisioni): [`../reviews/`](../reviews/).
