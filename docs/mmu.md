# MMU and kernel heap (M2)

## Goals

- Identity-map kernel RAM and device MMIO, **one region at a time**
- W^X: nothing is both writable and executable
- An unmapped guard page below the stack
- Enable stage-1 MMU + I/D caches at EL1
- Provide a bump heap for kernel allocations

## Translation regime

| Item          | Value                                   |
| ------------- | --------------------------------------- |
| Granule       | 4 KiB                                   |
| VA size       | 39-bit (`TCR_EL1.T0SZ = 25`)            |
| Initial level | L1 (1 GiB blocks, then 2 MiB and 4 KiB) |
| TTBR          | `TTBR0_EL1` only (`TCR_EL1.EPD1` set)   |

`EPD1` matters: nothing is mapped in the upper half, so a stray high address
must fault rather than start a walk through an uninitialised `TTBR1_EL1`.

Levels are picked per chunk: the largest of 1 GiB / 2 MiB / 4 KiB whose size
divides _both_ the virtual and the physical address and still fits in what is
left. A region whose VA and PA alignments disagree degrades to pages rather
than mapping physical memory the caller did not ask for.

Level 3 leaves use descriptor type `0b11`, which at levels 1 and 2 means
"table" instead. Writing an L3 leaf as `0b01` leaves the page simply unmapped.

### Regions

Derived from the linker symbols by `mm::layout::kernel_regions`:

| Region                              | Type          | Permissions       |
| ----------------------------------- | ------------- | ----------------- |
| below the image (`0`–`0x80000`)     | Normal WB     | RW, no execute    |
| `.text` (+ vectors)                 | Normal WB     | **RX, read-only** |
| `.rodata`                           | Normal WB     | RO, no execute    |
| `.data` / `.bss`                    | Normal WB     | RW, no execute    |
| translation table arena             | Normal WB     | RW, no execute    |
| **guard page**                      | —             | **unmapped**      |
| stack (64 KiB)                      | Normal WB     | RW, no execute    |
| heap                                | Normal WB     | RW, no execute    |
| peripherals (`0xFE00_0000`, 16 MiB) | Device-nGnRnE | RW, no execute    |
| GIC (`0xFF84_0000`, 16 KiB)         | Device-nGnRnE | RW, no execute    |

Anything else faults. Tables come from a 64 KiB arena reserved by `link.ld`;
six are used today, and `mmu::tables_remaining()` is printed at boot so
exhaustion is visible before it becomes a mapping failure.

### Verifying the protections

A protection nobody has seen fire is an assumption. Both were checked by
temporarily adding a deliberate fault to `bootstrap::run` and booting under
QEMU:

| Probe                        | Expected          | Observed                                                                       |
| ---------------------------- | ----------------- | ------------------------------------------------------------------------------ |
| write to `.text` (`0x80000`) | permission fault  | `ESR=0x9600004F` → DFSC `0b001111` (permission, level 3), WnR=1, `FAR=0x80000` |
| write to the guard page      | translation fault | `ESR=0x96000047` → DFSC `0b000111` (translation, level 3), `FAR=0x96000`       |

The probes are not in the tree: a deliberate fault is a dead board. Re-run them
by hand after any change to `link.ld` or to the region list.

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
