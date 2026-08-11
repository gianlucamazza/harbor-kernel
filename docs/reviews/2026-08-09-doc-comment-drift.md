# Doc/comment drift report

> **Historical record (2026-08-09).** Every drift below was repaired the same
> day, and the snapshot that follows describes the tree **as it was then** —
> K8 had one slice, `MAX_TASKS` was smaller, and several statuses have since
> moved. Read it for what was found and how, not for current truth, which is
> [`../roadmap.md`](../roadmap.md). `docs/README.md` already excludes reviews
> from current truth; this line says so where a reader lands.

## Authority snapshot
AUTHORITATIVE STATUS (as of commits through ADR-0070, do not invent older states):
- K4 budget + EL0 preemption + EL1 preemption: done (HW). Transcripts 20260809-122251 (EL0) and 20260809-151021 (EL1).
- K8 first slice (unpark core1 idle): done (QEMU) via ADR-0070; HW stamp residual; NO per-core runqueue yet.
- ADR-0067: lab QEMU x86 intent only, no x86 code. ADR-0069: host-class north star; name remains Harbor (not HarbOS).
- Product board today remains Raspberry Pi 4B (ADR-0007). Multi-arch ready structure, not multi-product.
- ADR-0006 cooperative model partially superseded by K4 preemption on IRQ epilogue (EL0+EL1); device IRQ handlers still never switch.
- Secondary cores: QEMU uses spin-table PA 0xe0; real Pi uses secondary_entry table. smp: core1 alive is the oracle.
- Automated gates xrefs/doc-claims/doc-symbols/roadmap-evidence were clean before this run; still hunt prose/comment lag.
OUT OF SCOPE: rewriting code logic, inventing new features, renaming Harbor, claiming K8 HW done.

## Counts
- Raw scan findings: 48
- Verified window: 16
- Confirmed: 16
- Fixed entries reported: 15

## Confirmed findings
- `/home/gianluca/Workspace/experiments/harbor-kernel/README.md`: Where we are snapshot (dated 2026-08-08) is fully lagging: Next is still “P2 SD/power-cycle on Pi or K4 preemption code”; Not yet lists “IRQ preemption, SMP”; Execution is “Cooperative only — preemption/SMP open”. Authoritative status: K4 budget+EL0+EL1 preemption done (HW); P2 SD power-cycle done (HW); K8 first slice done (QEMU) via ADR-0070.
- `/home/gianluca/Workspace/experiments/harbor-kernel/SECURITY.md`: Threat residual table SMP/ASID/TTBR1 still says “K8 SMP still design-only; cooperative single-core” and K7 “done (QEMU)” only. Preemption row is current, so this is an internal contradiction. K7 first slice is done (HW); K8 first slice is done (QEMU) ADR-0070 with HW stamp residual.
- `/home/gianluca/Workspace/experiments/harbor-kernel/docs/architecture.md`: Completeness roadmap snapshot still lists H1 next “SD power-cycle · IRQ preemption”, H2 depth “K4 IRQ preemption residual … K8 code after design ADR”, and open (kernel) “K4 IRQ preemption _code_ … K8 code”. All three ignore ADR-0064/0068 HW and ADR-0070 QEMU.
- `/home/gianluca/Workspace/experiments/harbor-kernel/docs/roadmap.md`: H2 horizon cell says “K8 design only” while the K8 track row correctly says first slice done (QEMU) ADR-0070. Same page self-contradicts.
- `/home/gianluca/Workspace/experiments/harbor-kernel/docs/vision.md`: Open-work paragraph still lists “K4 IRQ-side preemption residual (cooperative budget done HW)” and “SMP code (K8, design accepted)”, plus P2 SD residual. H2 section still says “IRQ preemption code on K4 (budget done HW; design ADR-0051)” and “K8 SMP code” as remaining work.
- `/home/gianluca/Workspace/experiments/harbor-kernel/docs/stack.md`: “Not in the stack” table still groups “Preemption, SMP, ASID residuals” as open tracks “K4 IRQ preemption / K8 / K7 …”. Cores used says only “One. SMP is an open track” with no ADR-0070 QEMU nuance.
- `/home/gianluca/Workspace/experiments/harbor-kernel/docs/README.md`: Status steering block (2026-08-09): H1 next still “SD power-cycle”; H2 still “K8 design; SMP open” despite ADR-0070 and P2 SD done (HW).
- `/home/gianluca/Workspace/experiments/harbor-kernel/docs/verification.md`: Honest-limit prose after park/session evidence still ends “Same-EL preemption stays open under K4”, but ADR-0068 is done (HW) with transcript 20260809-151021.
- `/home/gianluca/Workspace/experiments/harbor-kernel/docs/verification.md`: ADR-0051 evidence index row still says the re-audit list “re-opens with the same-EL slice”, implying EL1 is pending. ADR-0068 row correctly records HW done.
- `/home/gianluca/Workspace/experiments/harbor-kernel/docs/adr/0048-k8-smp-first-slice.md`: ADR-0048 body points at ADR-0070 but still presents “First implementation slice (future)”, non-goal “Implementing SMP now”, and Deferral “K8 waits for dual-core gate after K4/K7”. Frontmatter title remains “design only” without noting first code landed.
- `/home/gianluca/Workspace/experiments/harbor-kernel/docs/adr/0051-k4-irq-preemption-design.md`: YAML title still “design only”; Acceptance still says it does not implement switches and “coding still needs a dedicated implementation ADR”, while a later amendment already records full implementation via 0064/0068. Top of ADR reads as “same-EL still in design”.
- `/home/gianluca/Workspace/experiments/harbor-kernel/docs/adr/0067-host-lab-second-isa-intent.md`: Non-goals still frame H2 Pi completeness as open “(K4 same-EL, K8)”. K4 same-EL is done (HW); K8 has a QEMU first slice (0070), not pure design.
- `/home/gianluca/Workspace/experiments/harbor-kernel/src/status.rs`: Glass status line hardcodes `EL1  W^X  cooperative` (around show_boot_after_display), presenting a cooperative-only product model after K4 EL0+EL1 quantum preemption is done (ADR-0064/0068, HW stamps).
- `/home/gianluca/Workspace/experiments/harbor-kernel/src/agent/mod.rs`: Module crate docs open with `Cooperative agent shell` and only cite ADR-0006 (cooperative), while the session loop already calls `resume_step_preemptible` / ADR-0064 on every resumable path.
- `/home/gianluca/Workspace/experiments/harbor-kernel/src/arch/aarch64/switch.rs`: Module and `Context` docs say `Voluntary EL1 context switch` / `cooperative switch` only, but `context_switch` is also the stack-swap for `Switch::Preempt` (EL0 safe-point and EL1 IRQ-epilogue).
- `/home/gianluca/Workspace/experiments/harbor-kernel/crates/kernel-core/src/tasks.rs`: Module header is still `decision half of a cooperative switch (ADR-0006)` only, while `Switch::Preempt` (ADR-0064) is a first-class kind with idle-guard semantics.

## Fixer summary
Aligned 15 paths of lagging status prose/comments with post-ADR-0070 truth: K4 EL0+EL1 preemption done (HW), P2 SD power-cycle done (HW), K8 first unpark/idle slice done (QEMU) with HW stamp and per-core queues residual. Gates xrefs/doc-claims/doc-symbols/roadmap-evidence all clean. No runtime logic changes beyond the TFT status banner string.

### Fixed
- /home/gianluca/Workspace/experiments/harbor-kernel/README.md
- /home/gianluca/Workspace/experiments/harbor-kernel/SECURITY.md
- /home/gianluca/Workspace/experiments/harbor-kernel/docs/architecture.md
- /home/gianluca/Workspace/experiments/harbor-kernel/docs/roadmap.md
- /home/gianluca/Workspace/experiments/harbor-kernel/docs/vision.md
- /home/gianluca/Workspace/experiments/harbor-kernel/docs/stack.md
- /home/gianluca/Workspace/experiments/harbor-kernel/docs/README.md
- /home/gianluca/Workspace/experiments/harbor-kernel/docs/verification.md
- /home/gianluca/Workspace/experiments/harbor-kernel/docs/adr/0048-k8-smp-design.md
- /home/gianluca/Workspace/experiments/harbor-kernel/docs/adr/0051-k4-irq-preemption-design.md
- /home/gianluca/Workspace/experiments/harbor-kernel/docs/adr/0067-host-lab-second-isa-intent.md
- /home/gianluca/Workspace/experiments/harbor-kernel/src/status.rs
- /home/gianluca/Workspace/experiments/harbor-kernel/src/agent/mod.rs
- /home/gianluca/Workspace/experiments/harbor-kernel/src/arch/aarch64/switch.rs
- /home/gianluca/Workspace/experiments/harbor-kernel/crates/kernel-core/src/tasks.rs

### Skipped
- docs/adr/0048-k8-smp-first-slice.md — path does not exist; edits applied to docs/adr/0048-k8-smp-design.md (design ADR; first-slice code is 0070)
