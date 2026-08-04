# MMU and kernel heap (M2)

## Goals

- Identity-map kernel RAM and device MMIO
- Enable stage-1 MMU + I/D caches at EL1
- Provide a bump heap for kernel allocations

## Translation regime

| Item          | Value                                 |
| ------------- | ------------------------------------- |
| Granule       | 4 KiB                                 |
| VA size       | 39-bit (`TCR_EL1.T0SZ = 25`)          |
| Initial level | L1 (1 GiB blocks)                     |
| TTBR          | `TTBR0_EL1` only (`TCR_EL1.EPD1` set) |

`EPD1` matters: nothing is mapped in the upper half, so a stray high address
must fault rather than start a walk through an uninitialised `TTBR1_EL1`.

### Identity map

| L1 index | VA/PA range                 | Memory type            |
| -------- | --------------------------- | ---------------------- |
| 0        | `0x0000_0000`–`0x3FFF_FFFF` | Normal WB (MAIR Attr0) |
| 1        | `0x4000_0000`–`0x7FFF_FFFF` | Normal WB              |
| 3        | `0xC000_0000`–`0xFFFF_FFFF` | Device-nGnRnE (Attr1)  |

Covers kernel at `0x80000`, heap, UART (`0xFE20_1000`), GIC (`0xFF84_x000`).

## Code

| Path                               | Role                                           |
| ---------------------------------- | ---------------------------------------------- |
| `crates/kernel-core/src/paging.rs` | Descriptor + `TCR_EL1` encodings (host-tested) |
| `arch/aarch64/mmu.rs`              | Table build + `enable_identity()`              |
| `arch/aarch64/cache.rs`            | I-cache / D-cache set-way / TLB invalidation   |
| `mm/mod.rs`                        | Bump allocator from `__heap_start`             |
| `bsp/rpi4/memmap.rs`               | Which blocks are RAM and which are device      |
| `link.ld`                          | `__heap_start` after stack                     |

`enable_identity` takes the block list from the caller and returns `Result`:
which physical ranges are RAM is board knowledge, and a bring-up failure
reports instead of killing the boot.

Caches are invalidated **before** `SCTLR_EL1.{M,C,I}` are set. The table is
written with the MMU off so it lands in memory, but the walker then reads it
through the caches — and the platform firmware left lines of its own behind.

## Bring-up order

```
exception::init
mmu::enable_identity   // before irq (still fine after UART cold init)
mm::init_heap
board::irq::init
irq_enable
heap demo + ticks console
```

## Heap

- Start: linker `__heap_start` (page-aligned after stack)
- Size: min(64 MiB, remaining to `IDENTITY_RAM_END`)
- API: `mm::alloc`, `mm::alloc_zeroed`, `mm::heap_remaining`
- No free (bump only) in M2

## Out of scope (later)

- EL0 / user maps
- Fine-grained 4K device pages
- ASID, multi-core TLB
- Full `kfree`
