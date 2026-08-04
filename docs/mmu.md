# MMU and kernel heap (M2)

## Goals

- Identity-map kernel RAM and device MMIO
- Enable stage-1 MMU + I/D caches at EL1
- Provide a bump heap for kernel allocations

## Translation regime

| Item | Value |
|------|--------|
| Granule | 4 KiB |
| VA size | 39-bit (`TCR_EL1.T0SZ = 25`) |
| Initial level | L1 (1 GiB blocks) |
| TTBR | `TTBR0_EL1` only |

### Identity map

| L1 index | VA/PA range | Memory type |
|----------|-------------|-------------|
| 0 | `0x0000_0000`–`0x3FFF_FFFF` | Normal WB (MAIR Attr0) |
| 1 | `0x4000_0000`–`0x7FFF_FFFF` | Normal WB |
| 3 | `0xC000_0000`–`0xFFFF_FFFF` | Device-nGnRnE (Attr1) |

Covers kernel at `0x80000`, heap, UART (`0xFE20_1000`), GIC (`0xFF84_x000`).

## Code

| Path | Role |
|------|------|
| `arch/aarch64/mmu.rs` | Page table + `enable_identity()` |
| `mm/mod.rs` | Bump allocator from `__heap_start` |
| `link.ld` | `__heap_start` after stack |

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
