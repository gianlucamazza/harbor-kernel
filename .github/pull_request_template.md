<!--
Harbor keeps one owner per fact. These boxes are about *where* a change is
written down, not about ceremony. See CONTRIBUTING.md.
-->

## What this changes

<!-- One or two sentences. What is different after this merges? -->

## Evidence

<!--
Host test, QEMU oracle, or Pi 4B transcript — and say which. "It compiles" is
not evidence; `done (QEMU)` and `done (HW)` are different claims.
-->

- [ ] `make check` is green locally (it is a superset of CI)

## Documentation ownership

- [ ] Moves a **K/P track**? → status updated in `docs/roadmap.md` **only** (no
      second table in README, vision or architecture)
- [ ] Changes a **stack assumption** (toolchain, target, feature, host tool,
      dependency)? → `docs/stack.md` updated
- [ ] Introduces a **term a reader would guess wrong**? → row in
      `docs/glossary.md`
- [ ] Moves a **boundary**? → design ADR accepted first (`docs/adr/`);
      accepted ADRs are immutable, change them with a successor
- [ ] Adds a **gate**? → wired into `make check` and the README `make check`
      line (`make doc-claims` compares them)

<!-- Historical milestone narrative belongs in docs/foundation-history.md, not
     in architecture.md. Long transcripts belong in docs/verification.md, not
     in the README. -->
