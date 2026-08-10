# Design contracts

Contracts and matrices — **not** completion evidence (that is
[`../verification.md`](../verification.md)) and **not** immutable decisions
(that is [`../adr/`](../adr/README.md)).

| Document | Owns |
| -------- | ---- |
| [project-topology.md](project-topology.md) | **Scale axes** — where code grows (ISA / board / lab / pure) |
| [native-multiarch-practices.md](native-multiarch-practices.md) | Multi-arch + Linux-free support bar |
| [progressive-isa-practices.md](progressive-isa-practices.md) | Progressive second-ISA honesty (no silent stubs) |
| [host-lab-platform-matrix.md](host-lab-platform-matrix.md) | Role map Pi 4 vs QEMU x86 lab |

Start a port from [`../porting.md`](../porting.md); start a structural change
from an ADR.
