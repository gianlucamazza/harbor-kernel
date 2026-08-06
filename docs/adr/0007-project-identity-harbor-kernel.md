---
id: 0007
title: Project identity — Harbor and harbor-kernel
status: accepted
date: 2026-08-04
accepted: 2026-08-04
---

# ADR-0007: Project identity — Harbor and `harbor-kernel`

## Acceptance status

**Accepted.** This ADR fixes the naming decision. The migration it describes
has since been implemented in `5d3bd3d`, which renamed the Cargo package, the
lockfile, the ELF path, the documentation, the serial banner, and the QEMU
assertion. The gate below has been re-run: the only remaining occurrences of
the old name are the deliberate historical citations in ADR-0001 and in this
document.

## Context

The project began as `rpi_minimal_agentic`. That name describes the original
experiment, but it is not a durable description of the system's mission:

- `minimal` describes size rather than the architectural goal;
- `agentic` is broad and suggests capabilities that are still on the roadmap;
- `rpi` identifies today's supported hardware, but not the kernel's design;
- the project is currently a single-core EL1 kernel, while agents, IPC,
  capabilities, and EL0 address spaces are planned milestones.

The running system already establishes the foundation for that roadmap: early
MMU, W^X, guarded stacks, interrupts, heap allocation, explicit layering, and
verification on both QEMU and Raspberry Pi 4 hardware. Its mission is to grow
that foundation into a system where isolated tasks and agents operate inside
explicit boundaries and communicate through controlled channels.

The name must therefore be useful both for the current verified kernel and for
the future task, IPC, capability, and driver-as-agent milestones. It must not
claim that those future capabilities already exist.

## Decision

Adopt **Harbor** as the public project name and **`harbor-kernel`** as the
canonical technical identifier for the repository and Rust package.

The standard presentation is:

```text
Harbor
A verified Rust kernel for Raspberry Pi 4
```

The name expresses a protected place in which independently bounded components
can operate and communicate through controlled channels. It is a project
identity, not a claim about a particular scheduler, IPC ABI, capability
format, or user-mode implementation.

The following identifiers remain intentionally platform-specific or firmware-
specific:

- Raspberry Pi 4 Model B remains the only officially supported board;
- `bsp/rpi4` remains the board-support namespace;
- `kernel8.img` remains the boot image name required by the Raspberry Pi
  firmware and `boot/config.txt`.

## Naming rules

| Surface | Canonical form |
| --- | --- |
| Project/display name | `Harbor` |
| Repository/package identifier | `harbor-kernel` |
| Documentation prose | `Harbor` or `Harbor kernel` |
| Official platform | Raspberry Pi 4 Model B |
| Board-support namespace | `rpi4` |
| Firmware image | `kernel8.img` |

The name `agentic` should not be reintroduced as the primary identity. Future
agent functionality may be described in architecture and milestone documents,
but remains separate from the project name.

## Alternatives considered

| Alternative | Reason not selected |
| --- | --- |
| `rpi_minimal_agentic` | Describes the experiment's origin, but is provisional and overstates the current agent model. |
| `rpi-harbor` | Preserves the platform in the primary name, but makes the product identity less concise and less reusable. |
| `Harbor` as the package identifier | Natural as a display name, but less explicit and less suitable as a repository/package identifier. |
| `Stratum`, `Axiom`, `Lattice` | Each captures one aspect of layering, invariants, or isolation, but none covers protection plus controlled communication as directly as Harbor. |

## Consequences

### Positive

- The public identity is short, memorable, and consistent with the project's
  protection and communication goals.
- The name remains honest while the implementation advances from kernel
  bring-up to tasks and agents.
- Raspberry Pi support remains explicit in the subtitle and hardware docs
  without making the brand depend on one board revision.
- The technical identifier is unambiguous in Cargo and repository contexts.

### Negative

- `Harbor` does not identify Rust, AArch64, or Raspberry Pi without its
  subtitle.
- The name is metaphorical and may require the subtitle in technical indexes
  and release artefacts.
- Existing users and scripts will need a coordinated rename from the old
  package and binary name.

## Migration boundary

The rename is a follow-up implementation change, not part of this ADR. When
implemented, it must update Cargo metadata, the generated lockfile, the ELF
path, documentation, serial banner, QEMU assertions, and project directory as
appropriate.

The migration must not rename `kernel8.img`, `bsp/rpi4`, or any firmware-facing
identifier. It must also preserve unrelated worktree changes and make no
functional kernel changes.

## The gate that protects this decision

This is a documentation and identity decision, so its first gate is review of
the accepted ADR and a repository-wide search showing that the later migration
has no stale active references to the old project name.

The implementation follow-up must additionally pass the existing formatting,
documentation, build, and QEMU gates. A rename is complete only when the new
package builds, the boot image is still `kernel8.img`, and the boot check
recognises the Harbor banner.

## References

- [`architecture.md`](../architecture.md) — current layers, roadmap, and
  agent milestones
- [`verification.md`](../verification.md) — evidence model and declared blind
  spots
- [ADR-0001](0001-multi-role-analysis.md) — review discipline before
  architectural milestones
- [ADR-0006](0006-cooperative-execution-model.md) — first task model before
  scheduler implementation
