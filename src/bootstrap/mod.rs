//! Kernel bring-up: the ordered sequence that takes the machine from `_start`
//! to a running console, and nothing else.
//!
//! What the machine then *does* lives in [`console_loop`]; the hardware gates
//! used to debug the M1 interrupt path live in [`selftest`], behind the
//! `bringup` feature. Keeping the three apart matters because only this file
//! has to be read in order: every line here depends on the ones above it.

mod authority;
mod blob_server;
mod console_loop;
mod console_server;
#[cfg(feature = "oracle")]
mod demos;
mod discover;
mod loader;
#[cfg(feature = "board-qemu-virt")]
mod network_runtime;
#[cfg(feature = "board-qemu-virt")]
mod network_server;
#[cfg(feature = "panic-probe")]
mod panic_probe;
#[cfg(feature = "bringup")]
mod selftest;

use crate::arch::{bootinfo, cpu, exception, mmu, smp, timer};
use kernel_core::asid::ASID_BITS;
use kernel_core::cpuid;
use kernel_core::layout::Region;
use kernel_core::paging::{MemKind, Perms};

use crate::bsp::board;
use crate::console;
use crate::drivers::pl011::Pl011;
use crate::irq;
use crate::mm;
use crate::println;

/// Timer rate (IRQ ticks per second).
const TIMER_HZ: u32 = 10;

/// Kernel heap size, clamped to the identity-mapped RAM window.
const HEAP_SIZE: usize = 64 * 1024 * 1024;

/// Keep firmware-owned DTB pages outside the kernel's identity-mapped heap and
/// frame-pool carve-outs. Firmware is allowed to place the blob anywhere in
/// RAM; reserving a fixed address would merely move the collision to another
/// machine model.
fn heap_end_avoiding_device_tree(desired: usize) -> (usize, bool) {
    let Some((dtb_base, dtb_len)) = bootinfo::device_tree_pages() else {
        return (desired, false);
    };
    let dtb_base = dtb_base as usize;
    let Some(dtb_end) = (dtb_base as u64).checked_add(dtb_len) else {
        return (desired, false);
    };
    let dtb_end = dtb_end as usize;
    let heap_start = mm::heap_start();
    let Some(frame_end) = desired.checked_add(board::memmap::FRAME_POOL_BYTES) else {
        return (desired, false);
    };
    let overlaps = heap_start < dtb_end && dtb_base < frame_end;
    if !overlaps {
        return (desired, false);
    }

    // The frame pool is contiguous and must remain whole. Move the complete
    // heap+pool window below the DTB, preserving page alignment; if that is
    // impossible, the normal frame/layout validation refuses the boot.
    let before_dtb = dtb_base.saturating_sub(board::memmap::FRAME_POOL_BYTES);
    let aligned = before_dtb & !(board::memmap::FRAME_SIZE - 1);
    (aligned, true)
}

/// Page tables that must remain free once the kernel map is built.
///
/// Every spawned task unmaps a guard page, and a guard page that lands in a
/// 2 MiB block not yet split costs one arena table — which the arena never
/// gets back (ADR-0005). So the worst case is one per spawnable task, plus the
/// device tree's, plus margin.
///
/// Derived from [`sched::MAX_TASKS`] rather than written down: the constant
/// used to say "`MAX_TASKS` is 4" while the scheduler said 12, so the reserve
/// under-counted by 3× and late spawns silently lost their guard page to
/// `OutOfTables`.
/// `MAX_TASKS` counts idle, which runs on the `link.ld` bootstrap stack and
/// never spawns, so the spawnable worst case is one less. The arena size that
/// has to cover this lives in `link.ld`; the boot-time refusal below is what
/// ties the two together, and it names `PAGE_TABLE_ARENA_SIZE` when it fires.
const MIN_SPARE_TABLES: usize = (crate::sched::MAX_TASKS - 1) + 1 + 2;

/// Stop the boot, having said why, when the kernel map could not be established.
///
/// The early map from `boot.s` is RWX across three gigabytes by construction —
/// it exists only to give the first Rust code memory attributes. Everything the
/// kernel claims about itself, W^X and the guard page both, arrives with
/// `mmu::activate`. Continuing without it would hand an interactive console to a
/// machine with no memory protection at all, having said so once in a line that
/// scrolls past in a second.
///
/// A boot that cannot establish its invariants has not degraded, it has failed.
/// Halting is also the honest signal: silence after this message is easier to
/// notice, and harder to ignore, than a prompt that works.
/// The PA-range field on the `cpu:` line: `pa44`, or `pa?` when the encoding
/// is one the architecture reserves. Diagnostic only, so a reserved value must
/// not refuse the boot — but it must not print as a plausible number either.
struct PaBits(Option<u32>);

impl core::fmt::Display for PaBits {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(bits) => write!(f, "{bits}"),
            None => f.write_str("?"),
        }
    }
}

fn refuse_to_boot(uart: &mut Pl011, reason: core::fmt::Arguments<'_>) -> ! {
    println!(uart, "BOOT REFUSED: {reason}");
    println!(
        uart,
        "the early map is RWX — no W^X, no guard page. Halting rather than \
         offering a console on an unprotected machine."
    );
    cpu::halt()
}

/// Full kernel bring-up; never returns.
/// The optional features this image was compiled with, as one word per feature.
///
/// `cfg!` rather than `#[cfg]` so every arm is type-checked in every build: a
/// feature renamed in `Cargo.toml` fails here instead of silently dropping out
/// of the banner, which is the failure mode a `#[cfg]` chain has.
const fn build_features() -> &'static str {
    match (cfg!(feature = "panic-probe"), cfg!(feature = "bringup")) {
        (true, true) => "panic-probe bringup",
        (true, false) => "panic-probe",
        (false, true) => "bringup",
        (false, false) => "headless (no bring-up gates)",
    }
}

/// How long the phases above took, on the board's own clock.
///
/// The serial transcript carries host timestamps, but only when someone
/// captured it with `serial-capture`; a log read on its own could not answer
/// "is this slow?" at all. `CNTPCT` runs from reset, so these are milliseconds
/// since the counter started, not since power — which is what a comparison
/// between two boots of the same image needs.
fn report_boot(mmu_at: u64, discover_at: u64) {
    let hz = timer::frequency_hz();
    crate::kprintln!(
        "boot: mmu={} ms discover={} ms ready={} ms",
        kernel_core::delay::counts_to_ms(hz, mmu_at),
        kernel_core::delay::counts_to_ms(hz, discover_at),
        kernel_core::delay::counts_to_ms(hz, timer::physical_count())
    );
}

/// Arm the PL011 RX IRQ and unmask, if the lines were actually bound.
///
/// Both halves are guarded on `interrupts_bound` for different reasons. Arming
/// RX into a dispatch table that never bound the line leaves the FIFO to fill
/// with nobody draining it; unmasking after a failed bind arms a timer whose
/// handler was never registered, so the IRQ fires, the dispatcher acknowledges
/// it with nothing to run, and the console looks alive while the tick counter
/// never moves — a failure that reads as a subtler bug than the one it is.
fn enable_interrupts(uart: &mut Pl011, interrupts_bound: bool) {
    if interrupts_bound {
        // SAFETY: `uart` is the caller's exclusively-owned console handle,
        // borrowed uniquely for this call — `&mut` is what makes that a fact
        // rather than a claim about where the handle came from. EL1 IRQs stay
        // masked until the unmask below, so the handler cannot run against a
        // base that is only half published.
        unsafe {
            console::enable_rx_irq(uart);
        }
    }

    if interrupts_bound {
        timer::on_interrupt();
        cpu::sync_pipeline();
        cpu::irq_enable();
        cpu::sync_pipeline();
        #[cfg(feature = "board-qemu-virt")]
        println!(uart, "IRQs enabled (timer + UART RX + virtio-mmio slots)");
        #[cfg(not(feature = "board-qemu-virt"))]
        println!(uart, "IRQs enabled (timer + UART RX)");
        println!(uart, "idle: WFI when no RX/tick work");
    } else {
        println!(uart, "interrupts stay masked — console is output only");
    }
}

/// Bring CPU 1 into the schedule: banked GICC + SGI wake, then a pinned marker.
///
/// ADR-0074 / K8 second slice. Needs the shared table sealed (a handler is
/// present) and the distributor open, so it runs after `seal_dispatch` — and
/// before the primary unmasks, because the *target* unmasks inside secondary
/// idle. ADR-0076 adds the multi-current half: reserve CPU 1's idle, pin a
/// marker worker, and wait for it to run. The primary prints the result;
/// there is no console TX from core 1.
fn bring_up_cpu1(uart: &mut Pl011, core1: bool, interrupts_bound: bool) {
    if core1 && interrupts_bound {
        if board::irq::probe_core1_ipi() {
            println!(uart, "smp: core1 ipi");
        } else {
            println!(uart, "smp: core1 ipi timeout");
        }
        // ADR-0076: multi-current — reserve CPU1 idle, pin a marker worker,
        // wait for it to run (primary prints — no console TX from core 1).
        match crate::sched::start_cpu1() {
            Ok(_) => match crate::sched::spawn_core1_marker() {
                Ok(_) => {
                    if crate::sched::wait_core1_ran(200_000_000) {
                        println!(uart, "smp: core1 ran");
                    } else {
                        println!(uart, "smp: core1 ran timeout");
                    }
                }
                Err(e) => println!(uart, "smp: core1 spawn FAILED {e:?}"),
            },
            Err(e) => println!(uart, "smp: core1 start FAILED {e:?}"),
        }
    } else if core1 {
        println!(uart, "smp: core1 ipi skipped (irq unbound)");
    }
}

/// Freeze the IRQ dispatch table: no further handlers may be registered.
///
/// After this the IRQ path reads state nothing can mutate under it. The count
/// is printed because a boot that registered nothing looks exactly like a
/// healthy one until the first interrupt nobody answers — and by then the
/// evidence is a counter rather than a line at the point of failure.
fn seal_dispatch(uart: &mut Pl011) {
    irq::seal();
    println!(
        uart,
        "irq: sealed with {} handlers registered",
        irq::registered()
    );
}

/// Bind the GIC and the timer PPI. `true` if the lines are live.
///
/// A failed bind is not fatal: the boot continues with interrupts masked and
/// a console that is output only, which `enable_interrupts` then honours.
fn bind_interrupts(uart: &mut Pl011) -> bool {
    // SAFETY: single-core; exclusive GIC ownership.
    let bound = match unsafe { board::irq::init(TIMER_HZ) } {
        Ok(()) => true,
        Err(error) => {
            println!(uart, "IRQ bind FAILED: {error:?}");
            false
        }
    };
    cpu::sync_pipeline();

    println!(
        uart,
        "CNTFRQ={} Hz  timer={} Hz  PPI={}",
        timer::frequency_hz(),
        TIMER_HZ,
        board::irq::TIMER_IRQ
    );
    bound
}

/// Probe the SoC RNG200, report one line, and say whether the block is there.
///
/// After the MMU (Device attributes), before IRQs. On-die block, same class as
/// the PL011 — always brought up. Logical failure (timeout, health) is soft:
/// one line and the boot continues. Never refuse a boot for entropy.
///
/// The return value is what [`authority`](authority) needs to decide whether to
/// provide the `rng` window (ADR-0101): present means an agent composed to
/// drive it can be given the page, absent means the board does not have the
/// device and the position stays a hole. Probing twice to answer the same
/// question is how two answers start to disagree, so the boot's own probe is
/// the one that answers.
fn probe_rng(uart: &mut Pl011) -> bool {
    // SAFETY: single core; RNG200 window not otherwise claimed.
    match unsafe { board::rng::init() } {
        Ok(rng) => match rng.try_word() {
            Ok(Some(word)) => println!(uart, "rng200: ok word={word:#010x}"),
            Ok(None) => {
                let mut words = [0u32; 1];
                match rng.read_words(&mut words) {
                    Ok(1) => println!(uart, "rng200: ok word={:#010x}", words[0]),
                    Ok(_) => println!(uart, "rng200: ok (FIFO empty after warm-up)"),
                    Err(error) => println!(uart, "rng200: warm-up ok, read FAILED: {error:?}"),
                }
            }
            Err(error) => println!(uart, "rng200: read FAILED: {error:?}"),
        },
        Err(error) => {
            println!(uart, "rng200: unavailable ({error:?})");
            return false;
        }
    }
    true
}

/// Refuse to boot if the table arena is nearly spent.
///
/// Every later spawn may cost a table — a guard unmap splits a 2 MiB heap block
/// and the arena never frees. Checked *after* the DTB map so the reserve is
/// what spawn actually sees; counting before that under-states the cost.
fn assert_table_reserve(uart: &mut Pl011) {
    if mmu::tables_free() < MIN_SPARE_TABLES {
        refuse_to_boot(
            uart,
            format_args!(
                "table arena nearly exhausted: {} tables left, need {MIN_SPARE_TABLES} \
                 (raise PAGE_TABLE_ARENA_SIZE in link.ld)",
                mmu::tables_free()
            ),
        )
    }
}

/// Bring up the heap and the frame pool, then claim idle for the scheduler.
///
/// `sched::init` belongs here rather than later: since ADR-0017 §1 the EL0
/// session lives in the TCB, so *something* has to be the current task before
/// anyone can enter EL0. It claims idle — the task this bootstrap is already
/// running on — and publishes its session. It used to run after the demos,
/// which was only tenable while the session was a machine-wide global that
/// existed from link time.
fn init_memory_pools(uart: &mut Pl011, heap_end: usize, frame_base: usize, frame_end: usize) {
    // SAFETY: the heap region was just mapped as Normal memory.
    let heap_ok = unsafe { mm::init_heap(heap_end) };
    if heap_ok {
        println!(uart, "heap remaining = {} bytes", mm::heap_remaining());
    } else {
        println!(uart, "heap UNAVAILABLE (empty region)");
    }

    // ADR-0012 S1: named phys frame pool (identity-mapped with the kernel map).
    // SAFETY: `frame pool` region was mapped RW Normal; exclusive of the heap.
    let frames_ok = unsafe { mm::frames::init(frame_base, frame_end) };
    if frames_ok {
        println!(
            uart,
            "frames: {} free / {}  base={frame_base:#x}  ({} KiB pool)",
            mm::frames::free_count(),
            mm::frames::capacity(),
            board::memmap::FRAME_POOL_BYTES / 1024
        );
        crate::sched::init();
        #[cfg(feature = "board-rpi4")]
        report_genet_queue0(uart);

        // M5 S2/S3: AS create → prepare (kernel clone + user stack) → EL0 probes.
        #[cfg(feature = "oracle")]
        demos::m5_aspace_and_el0_smoke(uart);
    } else {
        println!(uart, "frames: UNAVAILABLE");
    }
}

/// Map the device tree back in, report what it says, and mark the phase.
///
/// The kernel map deliberately covers far less than the early one, so the blob
/// is now outside it. Mapping it back is the first region whose address only
/// the firmware knows, which is exactly what `mmu::map` exists for.
///
/// The report itself (ADR-0072/0073) observes the tree, reconciles it with the
/// compiled claims and prints one line per fact. **Fail-open:** a missing or
/// unmappable blob prints its `unknown` form and the boot continues.
///
/// Returns the phase mark for the `boot:` timing line.
fn map_dtb_and_discover(uart: &mut Pl011, core1: bool) -> u64 {
    let mut dtb_mapped = false;
    if let Some((base, len)) = bootinfo::device_tree_pages() {
        let region = Region {
            base,
            len,
            kind: MemKind::NormalWb,
            // Read-only: nothing in this kernel writes the blob, and a device
            // tree that the kernel can modify is a device tree nobody can trust.
            perms: Perms::RO,
            name: "device tree",
        };
        // SAFETY: kernel map active, IRQs masked; the range is firmware-owned
        // RAM outside every other region.
        match unsafe { mmu::map(&region) } {
            Ok(()) => {
                println!(uart, "DTB mapped: {len} bytes at {base:#x}");
                dtb_mapped = true;
            }
            Err(error) => println!(uart, "DTB map FAILED: {error:?}"),
        }
    }

    // The discovery report (ADR-0072/0073): observe the tree, reconcile with
    // the compiled claims, print one line per fact. Fail-open — a missing or
    // unmappable blob prints its `unknown` form and the boot continues.
    // smp-seen: primary always + core1 if unpark returned alive (not the
    // secondary_wait counter — that stays 0 under QEMU -kernel).
    let smp_seen = 1u64 + u64::from(core1);
    discover::report(uart, dtb_mapped, smp_seen);
    // GENET FDT binding, not a `discover:` fact (ADR-0072 closed inventory)
    // and not a device probe (ADR-0105/0106 stay proposed).
    let dtb = if dtb_mapped {
        // SAFETY: same contract as `discover::report` — the blob is mapped RO.
        unsafe { bootinfo::device_tree_slice() }
    } else {
        None
    };
    let genet = kernel_core::genet_fdt::boot_report(dtb_mapped, dtb);
    println!(uart, "{genet}");
    #[cfg(feature = "board-rpi4")]
    report_genet_mmio(uart, genet);
    timer::physical_count()
}

/// Bring up GENET through the existing driver when the FDT binding matches
/// the compiled window. Always prints one probe line (ADR-0072: fail-open).
/// After a successful revision probe, prints one PHY-identify line.
/// After a successful identify, prints one BMSR link line.
///
/// Uses `Genet::probe` (mask, stop DMA, UniMAC reset), then
/// `Genet::identify_phy` (PHYIDR only), then `Genet::classify_link`
/// (BMSR only). Queue-0 program/enable run later, after frames exist.
/// Does not reset the PHY, require link-up, submit TX/RX, or bind
/// the network vocabulary.
#[cfg(feature = "board-rpi4")]
fn report_genet_mmio(uart: &mut Pl011, report: kernel_core::genet_fdt::Report) {
    use crate::bsp::board::memmap;
    use crate::drivers::genet::{Error, Genet};
    use kernel_core::genet::{
        LinkReport, MdioError, MmioProbe, PhyError, PhyIdentify, RevisionError, mmio_probe_intent,
    };

    let probed = match report {
        kernel_core::genet_fdt::Report::Unavailable(_) => Err(MmioProbe::NoBinding),
        kernel_core::genet_fdt::Report::Binding(binding) => {
            match mmio_probe_intent(
                Some((binding.mmio_base, binding.mmio_len)),
                memmap::GENET_BASE as u64,
                memmap::GENET_REG_BYTES as u64,
            ) {
                Err(outcome) => Err(outcome),
                Ok(()) => {
                    // SAFETY: the compiled window is in DEVICE_REGIONS and
                    // mapped Device; extract already validated this Binding.
                    match unsafe { Genet::probe(binding) } {
                        Ok(controller) => Ok(controller),
                        Err(Error::NotPresent) => Err(MmioProbe::NotPresent),
                        Err(Error::Revision(RevisionError::Unsupported(major))) => {
                            Err(MmioProbe::Unsupported(major))
                        }
                        Err(Error::Timeout) => Err(MmioProbe::Timeout),
                        Err(Error::InvalidBinding) | Err(_) => Err(MmioProbe::InvalidBinding),
                    }
                }
            }
        }
    };
    match probed {
        Err(line) => println!(uart, "{line}"),
        Ok(controller) => {
            println!(uart, "{}", MmioProbe::Revision(controller.revision()));
            let phy = match controller.identify_phy() {
                Ok(link) => PhyIdentify::Identity(link),
                Err(Error::Timeout) => PhyIdentify::Timeout,
                Err(Error::Phy(error)) => PhyIdentify::Unavailable(error),
                Err(Error::Mdio(error)) => PhyIdentify::Unavailable(PhyError::Id(error)),
                Err(_) => PhyIdentify::Unavailable(PhyError::Id(MdioError::ReadFail)),
            };
            println!(uart, "{phy}");
            if matches!(phy, PhyIdentify::Identity(_)) {
                let link = match controller.classify_link() {
                    Ok(state) => LinkReport::Classified(state),
                    Err(Error::Timeout) => LinkReport::Timeout,
                    Err(Error::Mdio(error)) => LinkReport::Unavailable(error),
                    Err(_) => LinkReport::Unavailable(MdioError::ReadFail),
                };
                println!(uart, "{link}");
            }
            HELD_GENET.with(|held| *held = Some(controller));
        }
    }
}

#[cfg(feature = "board-rpi4")]
static HELD_GENET: crate::sync::Mutex<Option<crate::drivers::genet::Genet>> =
    crate::sync::Mutex::new(None);

/// Program, then enable, Linux v5 default TX ring 0 after the frame pool exists.
///
/// Probe runs at discover time, before frames; the programmed descriptors
/// need two identity-mapped frames inside the FDT DMA windows. Enable
/// writes RING_CFG+CTRL only after Programmed. RGMII OOB (ext-gphy,
/// no MAC delay) and UniMAC max-frame/station address are programmed
/// after Enabled. TBUF is in 64-byte TSB mode; the probe carries that prefix.
/// One bounded TX and one
/// bounded RX follow; a down BMSR refuses before the doorbell
/// or RX arm. After CONS the product prints a UniMAC TSV window
/// (packed 0x49c, Linux 0x4a8, pok 0x4ec). A bounded recover then
/// returns the controller to Idle.
#[cfg(feature = "board-rpi4")]
fn report_genet_queue0(uart: &mut Pl011) {
    use crate::drivers::genet::Error;
    use kernel_core::genet::Queue0Report;

    let lines = HELD_GENET.with(|held| {
        let controller = held.as_mut()?;
        let programmed = program_held_queue0(controller);
        let enabled = if matches!(programmed, Queue0Report::Programmed) {
            Some(match controller.enable_queue0() {
                Ok(()) => Queue0Report::Enabled,
                Err(Error::Enable(error)) => Queue0Report::Enable(error),
                Err(_) => Queue0Report::Enable(kernel_core::genet::QueueEnableError::NotProgrammed),
            })
        } else {
            None
        };
        let rgmii = if matches!(enabled, Some(Queue0Report::Enabled)) {
            Some(controller.program_rgmii_oob())
        } else {
            None
        };
        let umac = if matches!(enabled, Some(Queue0Report::Enabled)) {
            Some(controller.program_umac_init())
        } else {
            None
        };
        let tbuf = if matches!(enabled, Some(Queue0Report::Enabled)) {
            Some(controller.program_tbuf_tsb())
        } else {
            None
        };
        let tbuf_size = if matches!(enabled, Some(Queue0Report::Enabled)) {
            Some(controller.program_rbuf_tbuf_size())
        } else {
            None
        };
        let tx = if matches!(enabled, Some(Queue0Report::Enabled)) {
            Some(match controller.submit_one_tx() {
                Ok(report) => report,
                Err(Error::Timeout) => kernel_core::genet::TxReport::Timeout,
                Err(Error::Phy(kernel_core::genet::PhyError::LinkDown)) => {
                    kernel_core::genet::TxReport::LinkDown
                }
                Err(_) => kernel_core::genet::TxReport::NotEnabled,
            })
        } else {
            None
        };
        let mib = if tx.is_some() {
            Some(controller.read_umac_tsv())
        } else {
            None
        };
        let rx = if matches!(enabled, Some(Queue0Report::Enabled)) {
            Some(match controller.submit_one_rx() {
                Ok(report) => report,
                Err(Error::Timeout) => kernel_core::genet::RxReport::Timeout,
                Err(Error::Phy(kernel_core::genet::PhyError::LinkDown)) => {
                    kernel_core::genet::RxReport::LinkDown
                }
                Err(_) => kernel_core::genet::RxReport::NotEnabled,
            })
        } else {
            None
        };
        let recovered = if matches!(enabled, Some(Queue0Report::Enabled)) {
            Some(match controller.recover() {
                Ok(report) => report,
                Err(Error::Timeout) => kernel_core::genet::ResetReport::Timeout,
                Err(_) => kernel_core::genet::ResetReport::NotEnabled,
            })
        } else {
            None
        };
        Some((
            programmed, enabled, rgmii, umac, tbuf, tbuf_size, tx, mib, rx, recovered,
        ))
    });
    if let Some((programmed, enabled, rgmii, umac, tbuf, tbuf_size, tx, mib, rx, recovered)) = lines
    {
        println!(uart, "{programmed}");
        if let Some(enabled) = enabled {
            println!(uart, "{enabled}");
        }
        if let Some(rgmii) = rgmii {
            println!(uart, "{rgmii}");
        }
        if let Some(umac) = umac {
            println!(uart, "{umac}");
        }
        if let Some(tbuf) = tbuf {
            println!(uart, "{tbuf}");
        }
        if let Some(tbuf_size) = tbuf_size {
            println!(uart, "{tbuf_size}");
        }
        if let Some(tx) = tx {
            println!(uart, "{tx}");
        }
        if let Some(mib) = mib {
            println!(uart, "{mib}");
        }
        if let Some(rx) = rx {
            println!(uart, "{rx}");
        }
        if let Some(recovered) = recovered {
            println!(uart, "{recovered}");
        }
    }
}

#[cfg(feature = "board-rpi4")]
fn program_held_queue0(
    controller: &mut crate::drivers::genet::Genet,
) -> kernel_core::genet::Queue0Report {
    use crate::drivers::genet::Error;
    use kernel_core::genet::{Descriptor, MAX_FRAME_BYTES, Queue0Report};

    let Some((tx_id, tx_cpu)) = crate::mm::frames::alloc() else {
        return Queue0Report::NoFrames;
    };
    let Some((rx_id, rx_cpu)) = crate::mm::frames::alloc() else {
        let _ = crate::mm::frames::free(tx_id);
        return Queue0Report::NoFrames;
    };
    let release = || {
        let _ = crate::mm::frames::free(tx_id);
        let _ = crate::mm::frames::free(rx_id);
    };
    let len = MAX_FRAME_BYTES;
    let dma = controller.binding().dma;
    let Ok(tx_dma) = dma.map_cpu(tx_cpu as u64, u64::from(len)) else {
        release();
        return Queue0Report::OutsideDma;
    };
    let Ok(rx_dma) = dma.map_cpu(rx_cpu as u64, u64::from(len)) else {
        release();
        return Queue0Report::OutsideDma;
    };
    let tx = Descriptor {
        address: tx_dma,
        length: len,
        status: 0,
    };
    let rx = Descriptor {
        address: rx_dma,
        length: len,
        status: 0,
    };
    match controller.configure_queue0(tx, rx, tx_cpu, rx_cpu) {
        Ok(()) => Queue0Report::Programmed,
        Err(error) => {
            release();
            match error {
                Error::Descriptor(error) => Queue0Report::Descriptor(error),
                Error::Ring(error) => Queue0Report::Ring(error),
                Error::Enable(error) => Queue0Report::Enable(error),
                _ => {
                    Queue0Report::Descriptor(kernel_core::genet::DescriptorError::AddressOutsideDma)
                }
            }
        }
    }
}

/// What the kernel map was built from, and when it went live.
///
/// Set once, whole, by [`establish_kernel_map`]. Deliberately **not** a mutable
/// boot context (ADR-0095 §2): the phases below read fields a single earlier
/// phase produced, and a struct that could be half-written would turn a compile
/// error into a zero at boot time.
struct MemPlan {
    heap_end: usize,
    frame_base: usize,
    frame_end: usize,
    /// Phase mark for the `boot:` timing line.
    mmu_at: u64,
}

/// Decide the kernel's memory bounds, build the map, and switch to it.
///
/// One function and not three, because `kernel_regions` borrows a local buffer
/// that whoever calls `mmu::activate` has to own (ADR-0095 §3).
///
/// Heap and frame-pool bounds come first: both are mapped regions (ADR-0012 —
/// a named pool, not "the rest of RAM"). On any failure the early map stays
/// active, so the refusal still reaches a working console, and then the boot
/// stops, because that map protects nothing.
///
/// `activate` returning `Ok` is not the same as the MMU being on, so the claim
/// printed at the end is read back from `SCTLR_EL1` rather than inferred from
/// the path that just ran.
fn establish_kernel_map(uart: &mut Pl011) -> MemPlan {
    let desired_heap_end = (mm::heap_start() + HEAP_SIZE).min(board::memmap::IDENTITY_RAM_END);
    let (heap_end, dtb_reserved) = heap_end_avoiding_device_tree(desired_heap_end);
    if dtb_reserved {
        println!(
            uart,
            "heap: reserved DTB window, end moved to {heap_end:#x}"
        );
    }
    let (frame_base, frame_end) = match mm::frames::range_after_heap(heap_end) {
        Some(range) => range,
        None => refuse_to_boot(
            uart,
            format_args!(
                "frame pool does not fit after heap at {heap_end:#x} \
                 (need {} B under IDENTITY_RAM_END)",
                board::memmap::FRAME_POOL_BYTES
            ),
        ),
    };
    let mut region_buffer = [mm::layout::empty_region(); mm::layout::MAX_REGIONS];
    let regions =
        match mm::layout::kernel_regions(heap_end as u64, frame_end as u64, &mut region_buffer) {
            Ok(regions) => regions,
            Err(error) => {
                // The layout itself is inconsistent — overlapping regions, a
                // mapped guard page, a W+X region. Mapping it would produce
                // something that boots and protects nothing.
                refuse_to_boot(uart, format_args!("layout invalid: {error:?}"))
            }
        };

    // Swap the coarse early map for the real one. On failure the early map
    // stays active, so the report still reaches the console — and then the boot
    // stops, because that map protects nothing.
    // SAFETY: single core, IRQs masked, early map active.
    if let Err((error, region)) = unsafe { mmu::activate(regions) } {
        refuse_to_boot(uart, format_args!("could not map {region}: {error:?}"))
    }
    // The map is live: remember the two bounds it was built from, so a later
    // fault can be told which region its address belongs to.
    mm::layout::record_bounds(heap_end as u64, frame_end as u64);

    // `activate` returning `Ok` is not the same as the MMU being on. The claim
    // printed below is about the hardware, so it is read back from `SCTLR_EL1`
    // rather than inferred from the path that just ran.
    if !mmu::is_enabled() {
        refuse_to_boot(
            uart,
            format_args!("activate reported success but SCTLR_EL1.M is clear"),
        )
    }

    println!(
        uart,
        "MMU on  (W^X, guard page at {:#x}, {} B of table arena left)",
        mm::layout::guard_page(),
        mmu::tables_remaining()
    );
    // Phase marks for the `boot:` timing line at the end of bring-up. Sampled
    // here rather than derived from the console timestamps, which belong to
    // the host and only exist when someone was capturing.
    MemPlan {
        heap_end,
        frame_base,
        frame_end,
        mmu_at: timer::physical_count(),
    }
}

/// Say what this image is, on the wire, before anything else can go wrong.
fn print_banner(uart: &mut Pl011) {
    println!(uart, "Harbor: hello");
    println!(
        uart,
        "EL1 · W^X map · heap · timer + UART RX IRQ · WFI idle"
    );
    // The image says what it was built as, on the wire, before anything else
    // can go wrong. A flashed card is otherwise indistinguishable from another
    // one: which features a `kernel8.img` carries depends on a `make`
    // invocation nobody can read afterwards, and a `bringup` or `panic-probe`
    // image that reached a card by accident behaves nothing like the product.
    // Cost: one line. What it replaces: guessing.
    // `src=` is `git describe --always --dirty` at compile time (build.rs).
    // The feature word says what the image *does*; the source id says what it
    // was built *from*, which is what a transcript cited by an ADR needs in
    // order to be evidence about a specific tree rather than about "a build".
    println!(
        uart,
        "build: {} src={} {}",
        build_features(),
        env!("HARBOR_SOURCE_ID"),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
}

/// Inspect what the firmware handed us while every physical address is
/// still readable, and cache the answer. Everything the BSP hard-codes —
/// RAM size, UART clock, peripheral base — is in that blob; parsing it is
/// future work, but the pointer is unrecoverable once lost.
fn survey_firmware(uart: &mut Pl011) {
    // SAFETY: the coarse early map is active and this runs once — the call
    // site in `run` is the only one, before any phase that could survey again.
    unsafe {
        bootinfo::survey();
    }
    match bootinfo::device_tree() {
        Some(dtb) => println!(uart, "DTB at {dtb:#x}"),
        None => println!(
            uart,
            "no DTB (x0 was {:#x}); board constants are compiled in",
            bootinfo::dtb_address()
        ),
    }
}

/// Why the board came up, read before anything else can obscure it.
///
/// `halt()` is `loop { wfe }` with IRQs masked and cannot exit, so a board
/// that boots again after `*** halt ***` was reset from outside this kernel.
/// That was observed twice in one hardware session with no way to tell a
/// firmware watchdog from a brownout. The silicon latches the answer; this
/// line puts it in every transcript, so the question is a lookup rather
/// than an investigation the next time it happens.
fn report_reset(uart: &mut Pl011) {
    // SAFETY: single core, PM window inside the mapped peripheral region, and
    // this is the only code in the tree that touches the block.
    let reset = unsafe { board::pm::reset_status() };
    println!(
        uart,
        "reset: {:?} partition={} (PM_RSTS={:#010x})", reset.cause, reset.partition, reset.raw
    );
}

/// Which core this is, checked against the core the kernel was built for
/// (ADR-0065). The A72 knowledge in this tree — pre-MMU exclusives
/// confinement, explicit I/D-cache maintenance, ADR-0050's ASID arithmetic
/// — was comment-only until this line put the observed identity in every
/// transcript. Load-bearing mismatches refuse the boot; an unknown part is
/// a distinct printed outcome and the boot continues, because the
/// A72-specific handling is conservative on other cores and what the check
/// owes there is visibility, not a verdict.
fn verify_cpu(uart: &mut Pl011) {
    let midr = cpu::midr_el1();
    let mmfr0 = cpu::id_aa64mmfr0_el1();
    let pfr0 = cpu::id_aa64pfr0_el1();
    if !cpuid::tgran4_supported(mmfr0) || !cpuid::el0_aarch64(pfr0) || !cpuid::el1_aarch64(pfr0) {
        // The whole paging model is written against the 4 KiB granule, and the
        // session model against AArch64 EL0/EL1. There is nothing to degrade to.
        refuse_to_boot(
            uart,
            format_args!(
                "core lacks the 4 KiB granule or AArch64 EL0/EL1 \
                 (ID_AA64MMFR0={mmfr0:#x}, ID_AA64PFR0={pfr0:#x})"
            ),
        )
    }
    let asid = match cpuid::asid_bits(mmfr0) {
        Some(bits) if bits >= ASID_BITS => bits,
        // Fewer hardware bits than the pool hands out would alias two address
        // spaces in the TLB — ADR-0050's isolation, gone silently. A reserved
        // encoding refuses too: a width nobody can name cannot back the pool.
        answer => refuse_to_boot(
            uart,
            format_args!(
                "hardware ASID width {answer:?} cannot back the {ASID_BITS}-bit \
                 pool of ADR-0050 (ID_AA64MMFR0={mmfr0:#x})"
            ),
        ),
    };
    let pa = PaBits(cpuid::pa_bits(mmfr0));
    match cpuid::part(midr) {
        cpuid::Part::CortexA72 => println!(
            uart,
            "cpu: Cortex-A72 r{}p{} asid{asid} pa{pa} (MIDR={:#010x})",
            cpuid::variant(midr),
            cpuid::revision(midr),
            midr & 0xFFFF_FFFF
        ),
        cpuid::Part::Unknown { implementer, part } => println!(
            uart,
            "cpu: unknown implementer={implementer:#04x} part={part:#05x} r{}p{} \
             asid{asid} pa{pa} (MIDR={:#010x})",
            cpuid::variant(midr),
            cpuid::revision(midr),
            midr & 0xFFFF_FFFF
        ),
    }
}

/// ADR-0070 / K8 first slice: unpark core 1 into an idle loop. Kernel map
/// and VBAR are live; IRQs still masked. Timeout prints an honest line the
/// boot oracle fails on — silence would look like a single-core boot.
///
/// Returns whether core 1 answered.
fn unpark_secondary(uart: &mut Pl011) -> bool {
    let seen = smp::secondary_seen_count();
    let core1 = smp::unpark_core1();
    if core1 {
        println!(uart, "smp: core1 alive");
    } else {
        println!(uart, "smp: core1 timeout seen={seen}");
    }
    core1
}

pub fn run() -> ! {
    // SAFETY: core 0; DAIF still masked from `boot.s`; nothing else has run.
    let Some(mut uart) = (unsafe { console::acquire() }) else {
        // Unreachable in practice — this is the first claim — but there is no
        // console to report it on, so park rather than pretend.
        cpu::halt()
    };

    print_banner(&mut uart);

    exception::init();

    survey_firmware(&mut uart);

    // No `CPACR_EL1.FPEN` here on purpose: the kernel is built for
    // `aarch64-unknown-none-softfloat`, so it contains no FP/SIMD at all
    // (enforced by `make no-simd`). Leaving FPEN clear means any future stray
    // FP instruction traps loudly instead of silently corrupting the IRQ path,
    // whose trap frame saves no q registers.

    let plan = establish_kernel_map(&mut uart);

    report_reset(&mut uart);

    verify_cpu(&mut uart);

    let core1 = unpark_secondary(&mut uart);

    let discover_at = map_dtb_and_discover(&mut uart, core1);

    assert_table_reserve(&mut uart);

    init_memory_pools(&mut uart, plan.heap_end, plan.frame_base, plan.frame_end);

    // Carried to `authority::assemble` below: the window vocabulary provides the
    // RNG page only on a board that has the block (ADR-0101).
    let rng_present = probe_rng(&mut uart);

    // Deliberate fault (ADR-0093), before IRQs are bound: the panic path is
    // then reporting one fault on one core with nothing else in flight, which
    // is what makes `FAR` comparable with the address the probe announced.
    // Diverges — nothing below this runs in a panic-probe image.
    #[cfg(feature = "panic-probe")]
    panic_probe::fault_on_a_stack_guard(&mut uart);

    let interrupts_bound = bind_interrupts(&mut uart);

    #[cfg(feature = "board-qemu-virt")]
    match network_runtime::start() {
        Ok(result) => println!(
            uart,
            "virtio-net: modern probe ok base={:#x} vendor={:#x} features={:#x} queues={} size={} ready tx-descriptor=submitted",
            result.base,
            result.vendor,
            result.features,
            result.queues,
            result.queue_size
        ),
        Err(error) => println!(uart, "virtio-net: unavailable ({error:?})"),
    }

    // Hardware gates, only when built with `--features bringup`.
    #[cfg(feature = "bringup")]
    if !selftest::run(&mut uart) {
        println!(uart, "SELFTEST FAIL — soft console (IRQs masked)");
        selftest::soft_console(&mut uart);
    }

    seal_dispatch(&mut uart);

    bring_up_cpu1(&mut uart, core1, interrupts_bound);

    enable_interrupts(&mut uart, interrupts_bound);

    console_loop::heap_check(&mut uart);

    // Shared TX for idle + worker tasks (serialized in with_tx; not a claim
    // that the whole kernel is cooperative-only — see K4 preemption).
    console::install_tx(uart);

    // The vocabulary a composition may name (ADR-0099). Declared positions,
    // then whatever could be minted into them — so a service that fails to
    // start leaves a hole at its own index instead of shifting every later one
    // down. What each integer means lives in `authority.rs` and nowhere else.
    //
    // Agents that are data (ADR-0021): an entry naming anything outside this
    // vocabulary is refused by arithmetic rather than by a check.
    //
    // Product path carries the beacon agent; oracle adds mute. See loader.
    let authority = authority::assemble(rng_present);
    loader::load_all(&authority);

    // Everything the boot oracle needs, and nothing the product does — and
    // it lives in `demos`, which is the file `product-builds` derives its
    // forbidden-symbol list from (rule 9 of `architecture.md`).
    #[cfg(feature = "oracle")]
    demos::run_all(authority.held.get(authority::HELD_CONSOLE));

    // Deliberate fault, last so the demo tasks are alive when it runs: the
    // probe must overflow its own guard while a peer stack exists, or it cannot
    // show that the fault landed there *instead of* in the peer (M3 done-when).
    #[cfg(feature = "bringup")]
    match crate::sched::spawn(selftest::guard_probe_task) {
        Ok(_) => crate::kprintln!("sched: spawned guard probe"),
        Err(e) => crate::kprintln!("sched: spawn guard probe FAILED {e:?}"),
    }

    // What the spawns cost the arena. Printed here rather than tracked in the
    // idle loop because spawning is a boot-time act today; when tasks come and
    // go at runtime this number is the one that has to be watched, and the
    // counter already exists for that.
    crate::kprintln!(
        "arena: {} splits, {} tables free",
        mmu::splits(),
        mmu::tables_free()
    );

    report_boot(plan.mmu_at, discover_at);

    // Idle body — never returns (ADR-0006).
    console_loop::run()
}
