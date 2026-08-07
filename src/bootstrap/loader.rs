//! The loader: one loop over a table, instead of a grant written as code
//! (ADR-0021).
//!
//! # What is product here and what is not
//!
//! Everything in this file is compiled into every image. The **table** is the
//! scaffolding: `MANIFEST` is `cfg(feature = "oracle")`, and without the oracle
//! it is empty. So the product carries a loader with nothing to load, and
//! `make product-builds` reports that as a number rather than this file
//! pretending otherwise. The first product entry is M8's console endpoint —
//! ADR-0021's consequences say so, and say why the ADR's positive claim is only
//! half delivered until then.
//!
//! # Why an agent is one image
//!
//! An entry describes an agent, not a test. The oracle's other bodies —
//! `el0_scheduled_task`, `el0_ipc_sender`, the concurrent pair — run four and
//! five programs in sequence and check refusal counters and fault policy between
//! them. Those are drivers, they stay hand-written, and the boundary is recorded
//! in ADR-0021 rather than left to be inferred from what is in the table.

use kernel_core::cap::CapId;
use kernel_core::manifest::{AgentEntry, BindError, bind};
use kernel_core::paging::Perms;

use crate::agent::{Agent, SessionEnd};
use crate::arch::cpu;
use crate::mm::AddressSpace;
use crate::sched::{self, MAX_TASKS, TaskId};
use crate::sync::SyncCell;

#[cfg(feature = "oracle")]
mod oracle_entries {
    use kernel_core::manifest::{AgentEntry, MAX_SLOTS};
    use kernel_core::prog;

    /// `movz x0,#1; movz x1,#'H'; svc #2; movz x0,#1; movz x1,#'!'; svc #2; svc #1`
    ///
    /// A `const` in `.rodata`, built by the encoder whose bytes
    /// `kernel_core::prog`'s tests compare against `llvm-mc` (ADR-0021 §3).
    /// Nothing new has to be trusted for a manifest image that was not already
    /// trusted for a hand-spawned agent's program.
    const PUTC_HI: [u8; 32] = prog::encode_putc_hi_exit(CONSOLE_SLOT as u16);

    /// Slot the loader puts the console capability in, when it grants one.
    ///
    /// Slot 0 is left empty deliberately, as everywhere else here: an agent that
    /// miscounts finds nothing rather than something adjacent.
    const CONSOLE_SLOT: usize = 1;

    /// Index of the console capability in the **loader's** list, not the
    /// agent's. The distinction is the whole of ADR-0021 §2.
    const HELD_CONSOLE: u8 = 0;

    /// Two entries, one image, one difference: who was granted the console.
    ///
    /// This is the demonstration the manifest exists for. `echo` and `mute` run
    /// the identical bytes; `echo` prints `H!` and `mute` is refused twice. The
    /// authority is not in the program and not in the code that spawns it — it
    /// is one `Some` in a table.
    ///
    /// `mute` also declares **two** text pages against `echo`'s one, so a boot
    /// exercises a window geometry the BSP no longer fixes.
    pub(super) static MANIFEST: &[AgentEntry] = &[
        AgentEntry {
            name: "echo",
            image: &PUTC_HI,
            text_pages: 1,
            stack_pages: 3,
            slots: slots_with(Some(HELD_CONSOLE)),
            device: None,
        },
        AgentEntry {
            name: "mute",
            image: &PUTC_HI,
            text_pages: 2,
            stack_pages: 3,
            slots: slots_with(None),
            device: None,
        },
    ];

    const fn slots_with(console: Option<u8>) -> [Option<u8>; MAX_SLOTS] {
        let mut slots = [None; MAX_SLOTS];
        slots[CONSOLE_SLOT] = console;
        slots
    }
}

#[cfg(feature = "oracle")]
use oracle_entries::MANIFEST;

/// Nothing to load. See the module doc: the loader is product, the table is not.
#[cfg(not(feature = "oracle"))]
static MANIFEST: &[AgentEntry] = &[];

/// Which manifest entry each task slot is running, if any.
///
/// **A side table here rather than a field in the TCB**, and the distinction is
/// architectural, not stylistic. The scheduler sits below `agent` and
/// `bootstrap` in the layering; a manifest is a concept it has no business
/// knowing. `Tcb.agent: Option<u8>` compiled and passed `make layering` — the
/// gate reads `crate::` import edges, and an `Option<u8>` imports nothing — but
/// it put application state in the scheduler's own struct, which is exactly the
/// non-import coupling F24 left as review-only.
///
/// Indexed by task slot, so it is the same shape as the thing it replaced and
/// costs the same lookup. `sched::spawn` still takes a bare `fn()`; the loader
/// remembers which entry it just handed out instead of asking the scheduler to
/// carry it.
static ENTRY_OF_TASK: SyncCell<[Option<u8>; MAX_TASKS]> = SyncCell::new([None; MAX_TASKS]);

fn remember(task: TaskId, index: u8) {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked and one core, so this `&mut` cannot overlap
        // another. Nothing in an IRQ handler reads this table.
        let table = unsafe { &mut *ENTRY_OF_TASK.get() };
        table[task.0 as usize] = Some(index);
    });
}

fn recall(task: TaskId) -> Option<u8> {
    cpu::without_irqs(|| {
        // SAFETY: as `remember`.
        let table = unsafe { &*ENTRY_OF_TASK.get() };
        table[task.0 as usize]
    })
}

/// `MAX_CAPS_PER_TASK` and the manifest's slot count are the same number.
///
/// Written twice — here and in `kernel_core::manifest` — because the two crates
/// cannot see each other's constants. Asserted in the layer that binds them, so
/// the scheduler does not have to name a manifest to state its own bound.
const _: () = assert!(sched::MAX_CAPS_PER_TASK == kernel_core::manifest::MAX_SLOTS);

/// Create one task per manifest entry, binding its slots against `held`.
///
/// `held` is what the loader itself holds. An entry names indices into it, so
/// the tasks this creates can only hold capabilities the loader already had —
/// arithmetic, not a check (ADR-0021 §2).
pub fn load_all(held: &[CapId]) {
    if MANIFEST.is_empty() {
        crate::kprintln!("loader: manifest empty, nothing to create");
        return;
    }
    for (index, entry) in MANIFEST.iter().enumerate() {
        match bind(entry, held) {
            Ok(slots) => match sched::spawn_with_slots(agent_body, &slots) {
                Ok(task) => {
                    remember(task, index as u8);
                    crate::kprintln!(
                        "loader: {} loaded text={} stack={}",
                        entry.name,
                        entry.text_pages,
                        entry.stack_pages
                    );
                }
                Err(e) => crate::kprintln!("loader: {} spawn FAILED {e:?}", entry.name),
            },
            // The refusal the manifest exists to make structural: an entry that
            // names authority the loader does not hold cannot be created, and
            // the report says which index it reached for.
            Err(BindError::NoSuchCapability { slot, index, held }) => crate::kprintln!(
                "loader: {} refused — slot {slot} names capability {index} of {held}",
                entry.name
            ),
            Err(e) => crate::kprintln!("loader: {} refused — {e:?}", entry.name),
        }
    }
}

/// The body every manifest agent runs. One trampoline, N descriptions.
///
/// It looks up which entry it is rather than being told, because `sched::spawn`
/// takes a bare `fn()`. The lookup is an **index into the manifest**, resolved
/// against the array's own bound — the same shape a capability slot has one
/// floor down: a task cannot reach an entry that is not in the table.
fn agent_body() {
    let Some(index) = recall(sched::current_task_id()) else {
        crate::kprintln!("loader: a task reached the agent body with no manifest entry");
        return;
    };
    let Some(entry) = MANIFEST.get(index as usize) else {
        crate::kprintln!("loader: task names manifest entry {index}, which is not there");
        return;
    };
    run(entry);
}

fn run(entry: &AgentEntry) {
    let name = entry.name;

    // Geometry first, before a frame is taken: an image that does not fit its
    // declared text is refused by the entry, not by a page fault later.
    if let Err(e) = entry.validate(kernel_core::paging::PAGE_SIZE as usize) {
        crate::kprintln!("loader: {name} refused — {e:?}");
        return;
    }

    let mut aspace = match AddressSpace::create_with(entry.text_pages, entry.stack_pages) {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("loader: {name} address space FAILED {e:?}");
            return;
        }
    };
    if let Err(e) = aspace.prepare_for_el0() {
        crate::kprintln!("loader: {name} prepare FAILED {e:?}");
        aspace.destroy();
        return;
    }
    if let Some(grant) = entry.device {
        // SAFETY-adjacent, and worth saying: the grant is data the loader was
        // compiled with, so it names a page the image itself chose to hand out.
        // It is not input, and this is exactly the line ADR-0021 §4 says a byte
        // format would cross.
        if let Err(e) = aspace.map_device_page(grant.va, grant.pa, Perms::USER_RW) {
            crate::kprintln!("loader: {name} device grant FAILED {e:?}");
            aspace.destroy();
            return;
        }
    }

    let mut agent = Agent::from_aspace(aspace);
    match agent.run_user_prog_resuming(entry.image) {
        Ok(stats) if stats.end == SessionEnd::Exit => crate::kprintln!(
            "loader: {name} ran putcs={} refusals={}",
            stats.putcs,
            stats.authority_refusals
        ),
        Ok(stats) => crate::kprintln!("loader: {name} ended {:?}", stats.end),
        Err(e) => crate::kprintln!("loader: {name} FAILED {e:?}"),
    }
    agent.destroy();
}
