//! The loader: one loop over a table, instead of a grant written as code
//! (ADR-0021), optionally filled from an external agent store (ADR-0027).
//!
//! # What is product here and what is not
//!
//! Everything in this file is compiled into every image. The **beacon** entry
//! is product (M8): always-on, grants the console send end, prints `H!` via
//! `SYS_SEND`. Oracle-only **mute** runs the same image without the grant so
//! the denial path is seen on the good path.
//!
//! When a valid store is present at [`AGENT_STORE_PA`] (ADR-0027), that table
//! **replaces** the built-in one for the boot.

use core::mem::MaybeUninit;

use kernel_core::agentstore::{self, MAX_AGENTS, StoreAgent};
use kernel_core::cap::CapId;
use kernel_core::manifest::{AgentEntry, BindError, MAX_SLOTS, bind};
use kernel_core::paging::Perms;
use kernel_core::prog;

use crate::agent::{Agent, SessionEnd};
use crate::arch::{cpu, mmu};
use crate::ipc;
use crate::mm::AddressSpace;
use crate::sched::{self, MAX_TASKS, TaskId};
use crate::sync::SyncCell;
use kernel_core::layout::Region;
use kernel_core::paging::{MemKind, Perms as MapPerms};

/// Physical address where a boot loader may place an agent store (ADR-0027).
pub const AGENT_STORE_PA: usize = 0x1000_0000;

/// Bytes scanned from [`AGENT_STORE_PA`].
pub const AGENT_STORE_MAX: usize = 256 * 1024;

/// Slot the loader puts the console capability in, when it grants one.
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

/// Built-in table when no external store is present.
fn builtin_manifest() -> &'static [AgentEntry] {
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

/// Active manifest for this boot (store or builtin).
static ACTIVE: SyncCell<Option<&'static [AgentEntry]>> = SyncCell::new(None);

/// Name bytes for store-backed entries (immortal for the boot).
static NAME_POOL: SyncCell<[[u8; agentstore::NAME_LEN]; MAX_AGENTS]> =
    SyncCell::new([[0u8; agentstore::NAME_LEN]; MAX_AGENTS]);

/// Store-backed entries materialised once at load.
static STORE_ENTRIES: SyncCell<[MaybeUninit<AgentEntry>; MAX_AGENTS]> =
    SyncCell::new([const { MaybeUninit::uninit() }; MAX_AGENTS]);

static ENTRY_OF_TASK: SyncCell<[Option<u8>; MAX_TASKS]> = SyncCell::new([None; MAX_TASKS]);

fn remember(task: TaskId, index: u8) {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked and one core.
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

fn active_manifest() -> &'static [AgentEntry] {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; ACTIVE set once before any agent runs.
        let a = unsafe { &*ACTIVE.get() };
        a.unwrap_or_else(builtin_manifest)
    })
}

/// Try to build a `'static` manifest from the external store at [`AGENT_STORE_PA`].
///
/// # Safety
///
/// The physical range must be identity-mapped Normal RAM for the life of the
/// boot. Invalid magic simply fails the parse; garbage that looks valid is
/// trusted boot input (ADR-0027).
fn try_store_manifest() -> Option<&'static [AgentEntry]> {
    // Kernel fine map does not cover arbitrary RAM (same as the DTB). Map the
    // store window RO before reading (ADR-0027).
    let region = Region {
        base: AGENT_STORE_PA as u64,
        len: AGENT_STORE_MAX as u64,
        kind: MemKind::NormalWb,
        perms: MapPerms::RO,
        name: "agent store",
    };
    // SAFETY: kernel map active; range is lab-convention RAM outside the image.
    if unsafe { mmu::map(&region) }.is_err() {
        return None;
    }

    // SAFETY: range just mapped RO Normal.
    let raw = unsafe {
        core::slice::from_raw_parts(AGENT_STORE_PA as *const u8, AGENT_STORE_MAX)
    };

    let mut parsed = [StoreAgent {
        name: b"",
        text_pages: 0,
        stack_pages: 0,
        slots: [agentstore::SLOT_NONE; MAX_SLOTS],
        image: b"",
    }; MAX_AGENTS];
    let agents = agentstore::parse(raw, &mut parsed).ok()?;

    // SAFETY: single-threaded boot; no agent has run yet.
    let names = unsafe { &mut *NAME_POOL.get() };
    let entries = unsafe { &mut *STORE_ENTRIES.get() };

    for (i, a) in agents.iter().enumerate() {
        let nlen = a.name.len().min(agentstore::NAME_LEN);
        names[i] = [0u8; agentstore::NAME_LEN];
        names[i][..nlen].copy_from_slice(&a.name[..nlen]);
        // SAFETY: image bytes live in the immortal store range.
        let image: &'static [u8] =
            unsafe { core::slice::from_raw_parts(a.image.as_ptr(), a.image.len()) };
        // SAFETY: names[i] is static pool storage; UTF-8 validated by parse;
        // pointer remains valid for the boot.
        let name: &'static str = unsafe {
            let p = names.as_ptr().add(i) as *const u8;
            let s = core::slice::from_raw_parts(p, nlen);
            core::str::from_utf8_unchecked(s)
        };
        entries[i].write(agentstore::to_entry(a, name, image));
    }

    // SAFETY: first `agents.len()` entries were written above.
    let slice: &'static [AgentEntry] = unsafe {
        core::slice::from_raw_parts(entries.as_ptr() as *const AgentEntry, agents.len())
    };
    Some(slice)
}

const _: () = assert!(sched::MAX_CAPS_PER_TASK == kernel_core::manifest::MAX_SLOTS);

/// Create one task per active manifest entry, binding slots against `held`.
pub fn load_all(held: &[CapId]) {
    let table = match try_store_manifest() {
        Some(t) => {
            crate::kprintln!("loader: store n={} pa={AGENT_STORE_PA:#x}", t.len());
            cpu::without_irqs(|| {
                // SAFETY: boot-only.
                unsafe {
                    *ACTIVE.get() = Some(t);
                }
            });
            t
        }
        None => {
            crate::kprintln!("loader: builtin");
            let t = builtin_manifest();
            cpu::without_irqs(|| {
                // SAFETY: boot-only.
                unsafe {
                    *ACTIVE.get() = Some(t);
                }
            });
            t
        }
    };

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

fn agent_body() {
    let Some(index) = recall(sched::current_task_id()) else {
        crate::kprintln!("loader: a task reached the agent body with no manifest entry");
        return;
    };
    let Some(entry) = active_manifest().get(index as usize) else {
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
