//! The loader: one loop over a table, instead of a grant written as code
//! (ADR-0021), optionally filled from an external agent store (ADR-0027 / 0029).
//!
//! # What is product here and what is not
//!
//! Everything in this file is compiled into every image. The **beacon** entry
//! is product (M8): always-on, grants the console send end, prints `H!` via
//! `SYS_SEND`. Oracle-only **mute** runs the same image without the grant so
//! the denial path is seen on the good path.
//!
//! When a valid store is present in the image `.agent_store` section
//! ([ADR-0029](../../docs/adr/0029-agent-store-in-image.md)), that table
//! **replaces** the built-in one for the boot. The host injects the blob after
//! link (`scripts/agent/inject-store.py`); the same image boots on QEMU and Pi.

use core::mem::MaybeUninit;

use kernel_core::agentstore::{self, MAX_AGENTS, StoreAgent};
use kernel_core::cap::CapId;
use kernel_core::manifest::{AgentEntry, BindError, MAX_SLOTS, bind};
use kernel_core::paging::Perms;
use kernel_core::prog;

use crate::agent::{Agent, SessionEnd};
use crate::ipc;
use crate::mm::AddressSpace;
use crate::sched::{self, MAX_TASKS, TaskId};
use crate::sync::{Mutex, SyncCell};

/// Capacity of the image-resident agent store (ADR-0029).
///
/// Sized for several small EL0 programs; the host injects into this window.
/// Must match the size reserved in `link.ld` for `__agent_store_*`.
pub const AGENT_STORE_CAPACITY: usize = 16 * 1024;

// Linker-provided window (zeros at link, host inject after objcopy). Not a
// Rust `static` with a known initializer: LTO would fold those zeros forever
// and the product would never see an inject (seen: `ldr wzr` + always-builtin).
unsafe extern "C" {
    safe static __agent_store_start: u8;
    safe static __agent_store_end: u8;
}

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
                home_cpu: 0,
            },
            AgentEntry {
                name: "mute",
                image: &CONSOLE_HI,
                text_pages: 2,
                stack_pages: 3,
                slots: slots_with(None),
                device: None,
                home_cpu: 0,
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
            home_cpu: 0,
        }];
        &M
    }
}

/// Loader side tables: the manifest in force this boot, and which entry each
/// task came from.
///
/// One mutex for both, because `entry_for_task` needs them together and a
/// non-re-entrant lock cannot be taken twice on that path (ADR-0091).
struct SideTables {
    active: Option<&'static [AgentEntry]>,
    entry_of_task: [Option<u8>; MAX_TASKS],
}

/// Name bytes for store-backed entries (immortal for the boot).
static NAME_POOL: SyncCell<[[u8; agentstore::NAME_LEN]; MAX_AGENTS]> =
    SyncCell::new([[0u8; agentstore::NAME_LEN]; MAX_AGENTS]);

/// Store-backed entries materialised once at load.
static STORE_ENTRIES: SyncCell<[MaybeUninit<AgentEntry>; MAX_AGENTS]> =
    SyncCell::new([const { MaybeUninit::uninit() }; MAX_AGENTS]);

/// Serialises the loader side tables under dual-current (ADR-0077): product
/// agents home on CPU 1 (ADR-0088), so `recall` runs concurrently with any late
/// `remember`.
static SIDE: Mutex<SideTables> = Mutex::new(SideTables {
    active: None,
    entry_of_task: [None; MAX_TASKS],
});

fn remember(task: TaskId, index: u8) {
    SIDE.with(|side| side.entry_of_task[task.slot()] = Some(index));
}

/// Resolve the manifest entry for a task under a **single** lock hold.
///
/// The mutex is not re-entrant: separate `recall` + `active_manifest` helpers
/// would deadlock on the agent body path.
fn entry_for_task(task: TaskId) -> Option<&'static AgentEntry> {
    SIDE.with(|side| {
        let index = side.entry_of_task[task.slot()]?;
        let m = side.active.unwrap_or_else(builtin_manifest);
        m.get(index as usize)
    })
}

/// Bytes of the image-resident store (ADR-0029). Immortal for the boot.
fn store_bytes() -> &'static [u8] {
    let start = core::ptr::addr_of!(__agent_store_start);
    let end = core::ptr::addr_of!(__agent_store_end);
    // SAFETY: symbols bound by `link.ld` to a page-aligned RO window inside
    // the loaded image; inject finishes before entry.
    let len = unsafe { end.offset_from(start) as usize };
    debug_assert!(len == AGENT_STORE_CAPACITY);
    // SAFETY: same as above — `start`/`len` describe the RO agent-store window.
    unsafe { core::slice::from_raw_parts(start, len) }
}

/// Try to build a `'static` manifest from the image `.agent_store` section.
///
/// Invalid magic / empty zeros → `None` (builtin fallback). A valid store is
/// trusted boot input, same class as the rest of `kernel8.img` (ADR-0027/0029).
fn try_store_manifest() -> Option<&'static [AgentEntry]> {
    let raw = store_bytes();

    let mut parsed = [StoreAgent {
        name: b"",
        text_pages: 0,
        stack_pages: 0,
        slots: [agentstore::SLOT_NONE; MAX_SLOTS],
        home_cpu: 0,
        image: b"",
    }; MAX_AGENTS];
    let agents = agentstore::parse(raw, &mut parsed).ok()?;

    // SAFETY: single-threaded boot; no agent has run yet, and the window is
    // mechanically preemption-free too — sched::STARTED is still 0, which
    // gates both switch_with and the ADR-0068 EL1 IRQ-epilogue preemption.
    let names = unsafe { &mut *NAME_POOL.get() };
    // SAFETY: same boot window — exclusive `&mut` of static pool storage.
    let entries = unsafe { &mut *STORE_ENTRIES.get() };

    for (i, a) in agents.iter().enumerate() {
        let nlen = a.name.len().min(agentstore::NAME_LEN);
        names[i] = [0u8; agentstore::NAME_LEN];
        names[i][..nlen].copy_from_slice(&a.name[..nlen]);
        // SAFETY: image bytes live in the immortal store section.
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
    let slice: &'static [AgentEntry] =
        unsafe { core::slice::from_raw_parts(entries.as_ptr() as *const AgentEntry, agents.len()) };
    Some(slice)
}

const _: () = assert!(sched::MAX_CAPS_PER_TASK == kernel_core::manifest::MAX_SLOTS);

/// Create one task per active manifest entry, binding slots against `held`.
pub fn load_all(held: &[CapId]) {
    let table = match try_store_manifest() {
        Some(t) => {
            crate::kprintln!("loader: store n={} image", t.len());
            SIDE.with(|side| side.active = Some(t));
            t
        }
        None => {
            crate::kprintln!("loader: builtin");
            let t = builtin_manifest();
            SIDE.with(|side| side.active = Some(t));
            t
        }
    };

    if table.is_empty() {
        crate::kprintln!("loader: manifest empty, nothing to create");
        return;
    }
    for (index, entry) in table.iter().enumerate() {
        if let Err(e) = entry.validate(kernel_core::paging::PAGE_SIZE as usize) {
            crate::kprintln!("loader: {} refused — {e:?}", entry.name);
            continue;
        }
        match bind(entry, held) {
            Ok(slots) => {
                // ADR-0088: sticky home from the entry (store or builtin).
                match sched::spawn_with_slots_on(entry.home_cpu, agent_body, &slots) {
                    Ok(task) => {
                        remember(task, index as u8);
                        crate::kprintln!(
                            "loader: {} loaded text={} stack={} home={}",
                            entry.name,
                            entry.text_pages,
                            entry.stack_pages,
                            entry.home_cpu
                        );
                    }
                    Err(e) => crate::kprintln!("loader: {} spawn FAILED {e:?}", entry.name),
                }
            }
            Err(BindError::NoSuchCapability { slot, index, held }) => crate::kprintln!(
                "loader: {} refused — slot {slot} names capability {index} of {held}",
                entry.name
            ),
            Err(e) => crate::kprintln!("loader: {} refused — {e:?}", entry.name),
        }
    }
}

fn agent_body() {
    let Some(entry) = entry_for_task(sched::current_task_id()) else {
        crate::kprintln!("loader: a task reached the agent body with no manifest entry");
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
