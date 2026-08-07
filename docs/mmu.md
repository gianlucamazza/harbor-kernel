# MMU and kernel heap

## Goals

- Identity-map kernel RAM and device MMIO, **one region at a time**
- W^X: nothing is both writable and executable
- An unmapped guard page below each stack, and a separate exception stack
- Enable stage-1 MMU + I/D caches at EL1
- Provide a free-list heap behind `GlobalAlloc` (the bump allocator remains for
  early boot, where nothing is ever returned)

## Two maps

Translation is enabled **before any Rust runs**, from `arch/aarch64/boot.s`, using a coarse
identity map resolved at compile time (`mm::early::EARLY_L1`): 1 GiB blocks
covering 3 GiB of RAM plus the device window. Its purpose is not the mapping —
it is that no kernel code ever executes without memory attributes, because
atomic read-modify-write does not work without them. See
[`verification.md`](verification.md) for the bug that established this.

It lives in `mm` and not in `arch` on purpose. The table encodes *which
gigabyte is what* on a BCM2711, which is board knowledge, and it used to sit in
`arch/aarch64/mmu.rs` — inside the tree rule 3 reserves for CPU and ISA. That
was **F23**, and it stayed open for two days with `make layering` one directory
away, because that gate sees imports and the other way to know a board is to
write its addresses by hand. The seam now has a name: the board says which
gigabyte is what ([`memmap::EARLY_BLOCKS`]), the architecture says what a block
descriptor is and how to turn translation on, and `mm/early.rs` is the only
thing that needs both. `make arch-board-free` refuses the regression.

[`memmap::EARLY_BLOCKS`]: ../src/bsp/rpi4/memmap.rs

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

`mmu::unmap` is the reverse path for a page-aligned range that is already
mapped: each page becomes a translation fault. It is required by
[ADR-0006](adr/0006-cooperative-execution-model.md) for task-stack **guard
pages** inside the identity-mapped heap. The heap is usually covered by 2 MiB
blocks, so unmapping a single 4 KiB page **splits** the covering block into a
next-level table (break-before-make: clear block → TLBI → install table), then
clears the leaf. Physical pages are not returned to the free list by `unmap`;
only the virtual mapping is removed.

Invalidation granularity comes from `kernel_core::paging::tlbi_plan`: per page
(`tlbi vaae1is`, operand = VA >> 12) up to 64 pages, whole-TLB (`tlbi vmalle1`)
beyond, where thousands of broadcasts cost more than the refills a global flush
forces. Shared by `map` and `unmap`. For **valid→invalid** (`unmap`), the
invalidation is architecturally mandatory; a stale TLB entry would keep a
"guard" writable. That is the first kernel operation that makes TLB
maintenance falsifiable in principle — see [`verification.md`](verification.md).

The first user of `map` is the device-tree blob: the firmware puts it wherever
it likes (`0x2eff1f00` on this board), the kernel map covers far less, so it is
mapped read-only after `activate`. Verified in both directions — without the
call, a read of the blob takes `ESR=0x96000006`, a level-2 translation fault at
exactly its address; with it, the magic reads back as `0xd00dfeed`.

## Translation regime

| Item          | Value                                   |
| ------------- | --------------------------------------- |
| Granule       | 4 KiB                                   |
| VA size       | 39-bit (`TCR_EL1.T0SZ = 25`)            |
| Initial level | L1 (1 GiB blocks, then 2 MiB and 4 KiB) |
| TTBR          | `TTBR0_EL1` only (`TCR_EL1.EPD1` set)   |

`EPD1` matters: nothing is mapped in the upper half, so a stray high address
must fault rather than start a walk through an uninitialised `TTBR1_EL1`.

**M5 v1** keeps this regime ([ADR-0014](adr/0014-ttbr-split-m5.md)): user
tasks get their own `TTBR0` root that still includes **kernel identity maps**
(EL0 denied) plus a private user VA window. A future high-half kernel on
`TTBR1` is a successor ADR, not required for M5 done-when.

### The user window is per agent, not per board

`kernel_core::layout::UserWindow` describes it — base, total pages, and how many
of those are **text** — and `AddressSpace::create_with(text_pages, stack_pages)`
builds one. The lowest `text_pages` are mapped `USER_RX` and the rest `USER_RW`,
so W^X holds inside a user window exactly as it does in the kernel map, with
`UserWindow::is_text_page` the single place that decides which is which.

Until [ADR-0021](adr/0021-agents-as-data-and-the-manifest.md) the geometry was
one BSP constant: one page of text, three of stack, the same for every agent.
That was the ABI, because with no loader there was nothing to negotiate with. It
is now whatever a manifest entry declares, up to `mm::MAX_TEXT_PAGES` (16, so
64 KiB) — and `USER_STACK_PAGES` in the BSP survives only as the *default* an
`AddressSpace::create` gets when nobody asks.

**The pages of a window are not contiguous in physical memory.** Each comes from
its own `frames::alloc`, and they are adjacent only by accident of the pool's
LIFO free order — an accident that holds on a fresh boot and stops holding after
the first create/destroy cycle. `AddressSpace` therefore records the physical
address of every text page and `poke_user` walks them one at a time, publishing
each range for instruction fetch. An earlier version wrote from page 0's
physical address alone, which is why the write bound was a single frame for as
long as the text was.

Levels are picked per chunk: the largest of 1 GiB / 2 MiB / 4 KiB whose size
divides _both_ the virtual and the physical address and still fits in what is
left. A region whose VA and PA alignments disagree degrades to pages rather
than mapping physical memory the caller did not ask for.

Level 3 leaves use descriptor type `0b11`, which at levels 1 and 2 means
"table" instead. Writing an L3 leaf as `0b01` leaves the page simply unmapped.

### Task stacks (M3)

Spawned tasks use [`mm::task_stack`](../src/mm/task_stack.rs): heap allocation
of usable pages plus one guard, then `mmu::unmap` on the guard. Geometry is
checked with `kernel_core::layout::validate_guarded_stack`. The bootstrap and
exception stacks in `link.ld` remain separate.

Every production boot also runs a **block-split smoke** in `heap_check`: unmap a
page known to sit inside a 2 MiB leaf so break-before-make is not alignment-dead
at the heap head. QEMU asserts `split … split 1, remapped`.

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
| exception stack, `SP_EL1` (16 KiB)  | Normal WB     | RW, no execute    |
| **guard page**                      | —             | **unmapped**      |
| kernel stack, `SP_EL0` (64 KiB)     | Normal WB     | RW, no execute    |
| heap                                | Normal WB     | RW, no execute    |
| peripherals (`0xFE00_0000`, 16 MiB) | Device-nGnRnE | RW, no execute    |
| GIC (`0xFF84_0000`, 16 KiB)         | Device-nGnRnE | RW, no execute    |

Anything else faults. Tables come from a 128 KiB arena reserved by `link.ld`.
How many are used moves with `.text`, so the number is not written down here:
`mmu::tables_remaining()` is printed at boot, and `bootstrap` refuses to boot if
what is left is under the reserve it derives from `sched::MAX_TASKS` — that
reserve used to be a hand-written six against a `MAX_TASKS` of twelve.

### Verifying the protections

A protection nobody has seen fire is an assumption. W^X and the guard page were
both checked on hardware by planting a deliberate fault; the ESR values, and
what each one has to be, live in
[`verification.md`](verification.md#protections-are-only-verified-when-you-have-seen-them-fire).

They are recorded there and not here on purpose. This table used to exist in
both files, and both copies went stale together when the stack split moved the
guard page — which is exactly what a duplicated fact does.

One point belongs with the design rather than the evidence: the guard page must
give a _translation_ fault, not a permission one. A mapped page with restrictive
permissions would still let a read through, and a stack that overflowed by
reading would go unnoticed.

## Two stacks, so an overflow can be reported

The kernel runs on `SP_EL0` (`boot.s` sets `SPSel = 0`); `SP_EL1` holds a
separate 16 KiB exception stack with a guard page of its own. Exception entry
forces `PSTATE.SP` to 1, so the hardware switches stacks on the way in — no
juggling in the vector — and the live handlers are therefore the **EL1t**
vector entries, not EL1h. The EL1h entries are reached only from inside a
handler, and route to `exc_unexpected`: a fault inside a fault is not something
this kernel tries to survive.

The addresses themselves live in [`verification.md`](verification.md), dated to
the layout they were observed on. They are deliberately not repeated here: this
table and that one drifted apart once already, the moment the stack split moved
the guard.

What this buys is visible in where the report stops. Overflow the kernel stack
with a small-frame recursion and compare:

| Arrangement              | Where the reported `FAR` lands |
| ------------------------ | ------------------------------ |
| One stack (before this)  | the guard's **bottom**         |
| Separate exception stack | the guard's **top**            |

With one stack the handler saved its trap frame below the faulting `SP`, inside
the guard page, and faulted again — about fifteen times, walking 272 bytes lower
each entry, until it landed _below_ the guard in the translation table arena,
which is mapped RW. That entry could finally save its frame and print. The
diagnosis arrived, having overwritten page tables to get there. With `SP_EL1`
elsewhere, the first fault reports and nothing else is touched.

The earlier expectation was that one stack would simply hang; it did not, and
the probe said so. The defect was quieter than that.

The probes are not in the tree: a deliberate fault is a dead board. Re-run them
by hand after any change to `link.ld` or to the region list, following
[`verification.md`](verification.md).

## Code

| Path                               | Role                                                             |
| ---------------------------------- | ---------------------------------------------------------------- |
| `crates/kernel-core/src/paging.rs` | Descriptor + `TCR_EL1` encodings, region splitting (host-tested) |
| `crates/kernel-core/src/heap.rs`   | Free-list allocator arithmetic (host-tested)                     |
| `mm/early.rs`                      | `EARLY_L1`, `early_mmu_enable` — the one place board and CPU meet before anything exists (F23) |
| `arch/aarch64/mmu.rs`              | `enable_identity`, `program_regime`, `activate` — CPU only       |
| `arch/aarch64/cache.rs`            | I-cache / D-cache set-way / TLB invalidation                     |
| `mm/layout.rs`                     | Kernel regions and their permissions, from the linker            |
| `crates/kernel-core/src/layout.rs` | `UserWindow` (per-agent geometry, W^X inside it, write bounds) and guarded-stack validation — host-tested. Same file name as `mm/layout.rs`, different subject: the kernel's regions there, the agent's window here |
| `mm/aspace.rs`                     | `AddressSpace`: root, kernel clone, the user window, `poke_user` |
| `mm/mod.rs`                        | Kernel heap + `GlobalAlloc`                                      |
| `bsp/rpi4/memmap.rs`               | Device windows                                                   |
| `src/arch/aarch64/link.ld`         | Page-aligned region boundaries, table arena, guard page          |

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
  console_loop::run
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

- **Kernel table arena free / grow.** The ADR-0005 arena remains a fixed bump
  for the **kernel** map only. User AS tables and data pages come from the
  **separate** frame pool ([ADR-0012](adr/0012-frame-allocator-for-address-spaces.md))
  owned by `mm::frames` / `mm::aspace` — not from this arena.
- **Product multi-AS scheduling / ASID / SMP TLB.** In tree: user `TTBR0` roots,
  dual-AS teardown, scheduled multi-SVC / IRQ-resume EL0 sessions (sole
  `switch_ttbr0`), concurrent dual agents. Still out: ASID namespace, multi-core
  TLB maintenance, two TCBs **simultaneously** live at EL0.
- **High-half kernel / `TTBR1`.** Deliberately deferred
  ([ADR-0014](adr/0014-ttbr-split-m5.md) successor).
- **Agent Device maps** are **in tree** (M6): one PL011 page via
  `AddressSpace::map_device_page` ([ADR-0013](adr/0013-narrow-device-windows.md)),
  including RX-own poll + LBE self-test. Still out of scope: shrinking the
  **kernel** EL1 16 MiB peripheral blanket, multi-page device caps, IOMMU.
