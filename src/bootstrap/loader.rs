//! The loader: one loop over a table, instead of a grant written as code
//! (ADR-0021).
//!
//! # What is product here and what is not
//!
//! Everything in this file is compiled into every image. The **beacon** entry
//! is product (M8): always-on, grants the console send end, prints `H!` via
//! `SYS_SEND`. Oracle-only **mute** runs the same image without the grant so
//! the denial path is seen on the good path.

use kernel_core::cap::CapId;
use kernel_core::manifest::{AgentEntry, BindError, MAX_SLOTS, bind};
use kernel_core::paging::Perms;
use kernel_core::prog;

use crate::agent::{Agent, SessionEnd};
use crate::arch::cpu;
use crate::ipc;
use crate::mm::AddressSpace;
use crate::sched::{self, MAX_TASKS, TaskId};
use crate::sync::SyncCell;

/// Slot the loader puts the console capability in, when it grants one.
///
/// Slot 0 is left empty deliberately, as everywhere else here: an agent that
/// miscounts finds nothing rather than something adjacent.
const CONSOLE_SLOT: usize = 1;

/// Index of the console capability in the **loader's** list, not the agent's.
const HELD_CONSOLE: u8 = 0;

/// `H!` via two `SYS_SEND`s, then exit — shared product/oracle image bytes.
const CONSOLE_HI: [u8; 40] = prog::encode_console_hi_exit(CONSOLE_SLOT as u16);

const fn slots_with(console: Option<u8>) -> [Option<u8>; MAX_SLOTS] {
    let mut slots = [None; MAX_SLOTS];
    slots[CONSOLE_SLOT] = console;
    slots
}

/// Product + oracle-overlay table.
///
/// Product always has **beacon** (granted console). With `oracle`, **mute** is
/// appended (same image, no grant) — ADR-0021 same-image / table-diff.
fn manifest() -> &'static [AgentEntry] {
    #[cfg(feature = "oracle")]
    {
        static M: [AgentEntry; 2] = [
            AgentEntry {
                name: "beacon",
                image: &CONSOLE_HI,
                text_pages: 1,
                stack_pages: 3,
                slots: slots_with(Some(HELD_CONSOLE)),
                device: None,
            },
            AgentEntry {
                name: "mute",
                image: &CONSOLE_HI,
                text_pages: 2,
                stack_pages: 3,
                slots: slots_with(None),
                device: None,
            },
        ];
        &M
    }
    #[cfg(not(feature = "oracle"))]
    {
        static M: [AgentEntry; 1] = [AgentEntry {
            name: "beacon",
            image: &CONSOLE_HI,
            text_pages: 1,
            stack_pages: 3,
            slots: slots_with(Some(HELD_CONSOLE)),
            device: None,
        }];
        &M
    }
}

/// Which manifest entry each task slot is running, if any.
///
/// **A side table here rather than a field in the TCB**, and the distinction is
/// architectural, not stylistic. The scheduler sits below `agent` and
/// `bootstrap` in the layering; a manifest is a concept it has no business
/// knowing.
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
const _: () = assert!(sched::MAX_CAPS_PER_TASK == kernel_core::manifest::MAX_SLOTS);

/// Create one task per manifest entry, binding its slots against `held`.
pub fn load_all(held: &[CapId]) {
    let table = manifest();
    if table.is_empty() {
        crate::kprintln!("loader: manifest empty, nothing to create");
        return;
    }
    for (index, entry) in table.iter().enumerate() {
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
            Err(BindError::NoSuchCapability { slot, index, held }) => crate::kprintln!(
                "loader: {} refused — slot {slot} names capability {index} of {held}",
                entry.name
            ),
            Err(e) => crate::kprintln!("loader: {} refused — {e:?}", entry.name),
        }
    }
}

/// The body every manifest agent runs. One trampoline, N descriptions.
fn agent_body() {
    let Some(index) = recall(sched::current_task_id()) else {
        crate::kprintln!("loader: a task reached the agent body with no manifest entry");
        return;
    };
    let Some(entry) = manifest().get(index as usize) else {
        crate::kprintln!("loader: task names manifest entry {index}, which is not there");
        return;
    };
    run(entry);
}

fn run(entry: &AgentEntry) {
    let name = entry.name;

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
    if let Some(grant) = entry.device
        && let Err(e) = aspace.map_device_page(grant.va, grant.pa, Perms::USER_RW)
    {
        crate::kprintln!("loader: {name} device grant FAILED {e:?}");
        aspace.destroy();
        return;
    }

    let mut agent = Agent::from_aspace(aspace);
    match agent.run_user_prog_resuming(entry.image) {
        Ok(stats) if stats.end == SessionEnd::Exit => {
            // Creator drain barrier: wait until the console server has written
            // any enqueued bytes before the report kprintln (M8 ordering).
            // Mute never holds the console send cap — skip.
            if let Some(cap) = sched::my_cap(CONSOLE_SLOT) {
                match ipc::yield_until_empty_default(cap) {
                    Ok(()) => {}
                    Err(e) => crate::kprintln!("loader: {name} drain wait FAILED {e:?}"),
                }
            }
            crate::kprintln!(
                "loader: {name} ran sends={} refusals={}",
                stats.sends,
                stats.authority_refusals
            );
        }
        Ok(stats) => crate::kprintln!("loader: {name} ended {:?}", stats.end),
        Err(e) => crate::kprintln!("loader: {name} FAILED {e:?}"),
    }
    agent.destroy();
}
