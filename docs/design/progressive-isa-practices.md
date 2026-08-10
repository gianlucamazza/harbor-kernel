# Progressive second-ISA — practices (no debt bar)

**Status:** design contract for any lab/product ISA beyond AArch64.  
**Applies to:** H3 L0 x86 ([ADR-0071](../adr/0071-h3-l0-x86-qemu-first-slice.md)) and later slices.  
**Related:** [native-multiarch-practices.md](native-multiarch-practices.md),
[arch-contract.md](../arch-contract.md), [ADR-0015](../adr/0015-multi-arch-scaffold.md).

This page is the **anti-workaround** bar: how a second ISA lands without
stub-shaped lies, loader hacks, or permanent dual trees.

---

## Normative bar

> A progressive slice is **incomplete**, never **dishonest**.  
> Incomplete = not called on this slice’s path, or returns a typed refusal.  
> Dishonest = looks done (Ok / enabled / empty success) when it is not.

| Allowed | Forbidden |
| --- | --- |
| Thin lab entry that does not run product `bootstrap` | Stubbing the whole Pi stack so it “compiles” on x86 |
| Explicit `panic!("… not on L0")` if reached | `Ok(())` / `is_enabled() == true` that hides missing work **when callers could rely on it** |
| Boot protocol the **loader actually implements** (PVH for QEMU ELF64) | Multiboot1 on ELF64, GRUB-only CI, Linux EFI stub for lab gates |
| Code model matching load address (`small` @ 1 MiB) | `code-model=kernel` (high-half) with low VMA |
| Page tables in a dedicated section, zeroed then filled | Relying on “remember to zero BSS before CR0.PG” as the only safety |
| Pure decode in `kernel-core`; arch only reads registers | Family/model arithmetic in the board path |
| Status `done (QEMU-x86)` only | Collapsing into AArch64 `done (QEMU)` or `done (HW)` |
| One facade re-export list (roles); fill modules over slices | `dyn Arch`, runtime ISA switch, copy of `linux/arch` layout |

---

## Family P — progressive surface

| ID | Practice | Concrete rule |
| -- | -------- | ------------- |
| P.1 | **Roles before renames** | Keep arch-contract module names (`el0`, `switch_ttbr0` role) until a port ADR renames; map honestly in the matrix |
| P.2 | **L0 minimum path** | Boot → console TX → CPU identity → idle. No agents, no timer IRQ, no ring3 on L0 |
| P.3 | **Thin entry is a feature** | `kernel_main` → `lab::run` (`src/lab/`, project-topology maturity axis); product modules stay `cfg` out — not stubbed in |
| P.4 | **Contract modules may exist unused** | Re-export the facade; uncalled APIs may `panic!` with a slice-tagged message if entered |
| P.5 | **No silent success** | Dynamic map/unmap/switch that do not exist return `Err` or panic; do not return `Ok` that implies a table walk |
| P.6 | **Boot owns early paging** | Identity (or high-half) tables built in asm; Rust `mmu` takes over only when it owns the tables |
| P.7 | **BSS vs boot tables** | Boot page tables live in `.boot.pt` (NOLOAD), zeroed and filled in asm; never share the post-paging BSS wipe range |
| P.8 | **Loader protocol is evidence** | Choose the protocol QEMU documents for ELF64 `-kernel` (PVH note). “Tried Multiboot first” is history, not a dual path |
| P.9 | **Packaging ≠ ABI** | `objcopy`/`cp` to a Harbor image name is packaging (like `kernel8.img`); guest path stays freestanding ELF |
| P.10 | **One panic role** | Single `mod panic` per image (`cfg`/`path`); COM1 vs PL011 is the bind, not a second panic product |

---

## Family B — boot construction (x86 lab)

| ID | Practice | Why |
| -- | -------- | --- |
| B.1 | PVH `XEN_ELFNOTE_PHYS32_ENTRY` (type 18), name `"Xen"` | QEMU’s ELF64 direct-boot path; Linux-free |
| B.2 | 32-bit protected entry → PAE → EFER.LME → PG → far jump to 64-bit CS | Standard long-mode enable; matches what PVH hands you |
| B.3 | Identity map covering load + stack + early MMIO needs for the slice | L0: low 1 GiB with 2 MiB pages is enough for COM1 + image |
| B.4 | `relocation-model=static`, image base = link VMA | No PIE/GOT before a relocator exists |
| B.5 | **Small** code model at 1 MiB load | Kernel code model is the −2 GiB window — wrong here |
| B.6 | GDT with 64-bit code + data; load before long mode | Required for the far jump |
| B.7 | Stack in a named NOLOAD section in a RW PT_LOAD | ELF MemSize matches usable RAM; not a phantom location counter bump alone |

---

## Family C — layering on a second ISA

| ID | Practice | Gate |
| -- | -------- | ---- |
| C.1 | Policy imports `crate::arch` + `crate::bsp::board` only | `make layering` |
| C.2 | Drivers take ports/bases from BSP (`COM1_PORT`), not magic numbers in policy | Review + layering |
| C.3 | Product `ARCH`/`BOARD` Makefile allowlist stays closed until a product claim | porting.md |
| C.4 | Lab targets are dedicated (`x86-elf`, `x86-boot-check`), not fake `ARCH=x86_64` product | Makefile |

---

## Anti-patterns (name them so they do not return)

1. **Fake Multiboot** headers that QEMU ignores while PVH actually boots.  
2. **High-half code model** with low-half link address.  
3. **Zeroing `.bss` after enabling paging** while page tables live in `.bss`.  
4. **Compiling the Pi bootstrap on x86** via empty stubs for GIC/SD/agents.  
5. **`mmu::activate` → Ok` that implies Rust owns maps** when only boot.s does.  
6. **Collapsing evidence labels** (`done (QEMU-x86)` ≠ `done (QEMU)`).  
7. **GRUB disk images or Linux EFI stubs** as the lab CI path.  
8. **Blanket `#![allow(dead_code)]`** as a substitute for knowing the L0 call graph.

---

## Slice ladder (what “complete” means)

| Slice | Must be true | Still incomplete (honest) |
| ----- | ------------ | ------------------------- |
| **L0** | PVH boot, COM1, CPUID line, gate green | No IDT, no APIC timer, no sched, no ring3 |
| **Lab kernel** | Timer + exception path + cooperative sched oracle | No agents / full IPC |
| **Lab model** | User session role + one agent-class oracle | Not product multi-arch |
| **Product multi-arch** | Successor to ADR-0007 + product evidence policy | — |

Each step opens a **new** ADR or amends only under ADR-0058; L0 does not pretreat L1.

---

## Checklist before claiming a slice “clean”

- [ ] Boot protocol matches the runner (no dual dead headers).  
- [ ] Link/code model match load address.  
- [ ] Early tables not co-mingled with post-paging BSS wipe.  
- [ ] Oracle script asserts the lines the code prints.  
- [ ] Product gate still green (`make boot-check` when product is aarch64).  
- [ ] Layering + xrefs + arch-contract facade list still agree.  
- [ ] No `Ok`/enabled that over-claims incomplete roles.  
- [ ] Status vocabulary uses the lab label (`done (QEMU-x86)`).
