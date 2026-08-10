# Scripts

Host tools and gates. Invoked almost always through **`make`** targets so paths
stay in one place ([`Makefile`](../Makefile)).

| Directory | Owns | Examples |
| --- | --- | --- |
| [`check/`](check/) | Invariants and doc/code agreement | layering, irq-scope, doc-claims, xrefs, oracle-census |
| [`boot/`](boot/) | Images and QEMU oracles | product-image, qemu-boot-check, qemu-x86-boot-check, qemu-product-boot-check |
| [`agent/`](agent/) | Composition store tooling (ADR-0029) | pack-store, inject-store, inspect-store |
| [`host/`](host/) | Board and lab host ops | deploy-sd, serial, fetch-blobs, mutants |
| [`lib/`](lib/) | Shared shell libraries | `sd-target.sh` |

## Rules

1. **Repo root** is `$(dirname "$0")/../..` from any script in a subdir.
2. New gate → put it under `check/` (or `boot/` if it runs the guest) and wire
   it into `make check` only if every green local run must predict CI.
3. Prefer `make foo` in docs over raw `./scripts/...` paths so renames stay
   local to the Makefile and this map.
4. `shellcheck` covers `check/`, `boot/`, `host/`, and `lib/` (`make shellcheck`).

## Common make targets

| Target | Script area |
| --- | --- |
| `make check` | `check/*` + `boot/*` (+ clippy) |
| `make boot-check` / `product-boot-check` | `boot/` (product) |
| `make x86-boot-check` / `x86-elf` | `boot/qemu-x86-boot-check.sh` (lab) |
| `make agents` | `agent/pack-store.py` |
| `make deploy` / `serial` / `blobs` | `host/` |
