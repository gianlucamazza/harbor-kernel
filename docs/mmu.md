# MMU and kernel heap

## Goals

- Identity-map kernel RAM and device MMIO, **one region at a time**
- W^X: nothing is both writable and executable
- An unmapped guard page below the stack
- Enable stage-1 MMU + I/D caches at EL1
- Provide a free-list heap behind `GlobalAlloc` (the bump allocator remains for
  early boot, where nothing is ever returned)

## Two maps

Translation is enabled **before any Rust runs**, from `boot.s`, using a coarse
identity map resolved at compile time (`arch::mmu::EARLY_L1`): 1 GiB blocks
covering 3 GiB of RAM plus the device window. Its purpose is not the mapping —
it is that no kernel code ever executes without memory attributes, because
atomic read-modify-write does not work without them. See
[`verification.md`](verification.md) for the bug that established this.

The real per-region map below is built at runtime and installed by switching
`TTBR0_EL1` (`arch::mmu::activate`). Because translation is already on, the
table writes and the walker's reads go through the same caches, so the switch
needs a barrier rather than the invalidate-everything sequence a cold enable
requires. If it fails, nothing is switched and the early map stays active —
which is what lets the failure be reported over a working console.

Reported, and then the boot stops. The early map is RWX across three gigabytes
by construction; every protection this kernel claims about itself arrives with
`activate`. Continuing would offer an interactive console on a machine with no
memory protection, having said so once in a line that scrolls past. The boot
also refuses if `activate` returns `Ok` while `SCTLR_EL1.M` reads back clear:
the claim printed is about the hardware, so it is read from the hardware.

## Mapping after boot

`activate` installs _the_ map, once, before any address the firmware assigns at
runtime is known. `mmu::map` adds a region to the live tables afterwards: it
takes the root from `ROOT`, walks down with the same `map_chunk` the initial
build uses, then publishes with `dsb ishst` and invalidates.

Invalidation granularity comes from `kernel_core::paging::tlbi_plan`: per page
(`tlbi vaae1is`, operand = VA >> 12) up to 64 pages, whole-TLB (`tlbi vmalle1`)
beyond, where thousands of broadcasts cost more than the refills a global flush
forces. Going from invalid to valid would not strictly require invalidation —
the architecture does not permit caching invalid entries — but doing it anyway
makes the same function correct for *re*mapping, where it is mandatory.

The first user is the device-tree blob: the firmware puts it wherever it likes
(`0x2eff1f00` on this board), the kernel map covers far less, so it is mapped
read-only after `activate`. Verified in both directions — without the call, a
read of the blob takes `ESR=0x96000006`, a level-2 translation fault at exactly
its address; with it, the magic reads back as `0xd00dfeed`.

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
temporarily adding a deliberate fault to `bootstrap::run` and booting **on a
Raspberry Pi 4B** — the ESR values below are from silicon, and match what QEMU
produced bit for bit:

| Probe                        | Expected          | Observed                                                                       |
| ---------------------------- | ----------------- | ------------------------------------------------------------------------------ |
| write to `.text` (`0x80000`) | permission fault  | `ESR=0x9600004F` → DFSC `0b001111` (permission, level 3), WnR=1, `FAR=0x80000` |
| write to the guard page      | translation fault | `ESR=0x96000047` → DFSC `0b000111` (translation, level 3), `FAR=0x9a000`       |

The guard page must give a _translation_ fault, not a permission one: a mapped
page with restrictive permissions would still let a read through, and a stack
that overflowed by reading would go unnoticed.

The probes are not in the tree: a deliberate fault is a dead board. Re-run them
by hand after any change to `link.ld` or to the region list, following
[`verification.md`](verification.md).

## Code

| Path                               | Role                                                             |
| ---------------------------------- | ---------------------------------------------------------------- |
| `crates/kernel-core/src/paging.rs` | Descriptor + `TCR_EL1` encodings, region splitting (host-tested) |
| `crates/kernel-core/src/heap.rs`   | Free-list allocator arithmetic (host-tested)                     |
| `arch/aarch64/mmu.rs`              | `EARLY_L1`, `early_mmu_enable`, `activate`                       |
| `arch/aarch64/cache.rs`            | I-cache / D-cache set-way / TLB invalidation                     |
| `mm/layout.rs`                     | Regions and their permissions, from the linker                   |
| `mm/mod.rs`                        | Kernel heap + `GlobalAlloc`                                      |
| `bsp/rpi4/memmap.rs`               | Device windows                                                   |
| `link.ld`                          | Page-aligned region boundaries, table arena, guard page          |

`activate` takes the region list from the caller and returns `Result`: which
physical ranges are RAM is board knowledge, and a failure reports over the
still-working early map instead of killing the boot.

## Bring-up order

```
_start          → early_mmu_enable      // translation on, before any Rust
kernel_main     → bootstrap::run
  console::acquire                      // atomics work: memory has attributes
  exception::init
  bootinfo::survey                      // validate the DTB while 3 GiB is mapped
  mmu::activate                         // switch TTBR0 to the W^X map
  mm::init_heap
  board::irq::init
  irq::seal
  irq_enable
  shell::run
```

`survey` must precede `activate`: the firmware places the blob wherever it
likes (`0x2eff1f00` on this board), which the kernel map does not cover. It
caches its answer, so `device_tree()` is correct whenever it is called
afterwards — the ordering constraint lives in one place instead of in every
caller's head.

## Heap

- Start: linker `__heap_start` (page-aligned after the stack)
- Size: min(64 MiB, remaining to `IDENTITY_RAM_END`)
- First-fit free list with splitting and address-ordered coalescing, wired to
  `GlobalAlloc` — `Box` and `Vec` work
- Every operation takes the interrupt-masked critical section: an allocator
  interrupted mid-splice corrupts its own free list, and the damage surfaces
  arbitrarily later
- The bump allocator remains for early boot, where nothing is ever returned
- A free the allocator cannot justify — a double free, or a pointer it never
  handed out — is **refused**, not performed: the list is left untouched, so the
  memory leaks instead of the heap corrupting. Blocks carry an allocated mark in
  the low bit of their size word (sizes are `GRAIN`-aligned, so those bits are
  free). Refusals are counted and printed, and the boot check fails on them.
  Sound in the direction that matters — a legitimate free is never refused — but
  not the converse: metadata lives in the arena it manages, so a wild pointer
  into memory that resembles a live header can still be accepted. Catching every
  bad free needs out-of-band metadata, which is not worth its cost here

## Out of scope (later)

- **A frame allocator.** The table arena is a fixed pool sized in `link.ld` —
  the right shape for mapping the kernel once, the wrong one for address spaces
  that come and go. Needed for M5, not before.
- **More than one address space.** `activate` installs _the_ map; there is no
  per-task `TTBR0`, no ASID, no multi-core TLB maintenance.
- **EL0 / user maps**, fine-grained device pages, `kfree` of page tables.
