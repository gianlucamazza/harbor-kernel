//! The loader: one loop over a table, instead of a grant written as code
//! (ADR-0021), optionally filled from an external agent store (ADR-0027 / 0029).
//!
//! # What is product here and what is not
//!
//! Everything in this file is compiled into every image. The **beacon** entry
//! is product (M8): always-on, grants the console send end, prints `H!` via
//! `SYS_SEND`. Oracle-only **mute** runs the same image without the grant so
//! the denial path is seen on the good path, and oracle-only **nowindow** does
//! the same for the device vocabulary (ADR-0100): the same bytes again, asking
//! for a window this product does not declare, refused on every boot.
//!
//! When a valid store is present in the image `.agent_store` section
//! ([ADR-0029](../../docs/adr/0029-agent-store-in-image.md)), that table
//! **replaces** the built-in one for the boot. The host injects the blob after
//! link (`scripts/agent/inject-store.py`); the same image boots on QEMU and Pi.

use core::mem::MaybeUninit;

use kernel_core::agentstore::{self, MAX_AGENTS, StoreAgent};
use kernel_core::loaderplan;
use kernel_core::manifest::{AgentEntry, BindError, MAX_SLOTS, ResolvedWindow};
use kernel_core::prog;

// The console's position in the vocabulary, taken from where the vocabulary is
// declared (ADR-0099). The built-in manifest below grants by index exactly as a
// store entry does, so restating the integer here would be a third copy — one
// `make vocabulary-sync` does not compare, on the path the boot falls back to
// when no store is present.
use super::authority::HELD_CONSOLE;
#[cfg(feature = "board-qemu-virt")]
use super::network_runtime;
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

/// `H!` via two `SYS_SEND`s, then exit — shared product/oracle image bytes.
const CONSOLE_HI: [u8; 40] = prog::encode_console_hi_exit(CONSOLE_SLOT as u16);
const LOOKUP_CONSOLE: [u8; 52] = prog::encode_resolve_send_exit(0, b'N');
#[cfg(feature = "board-qemu-virt")]
const NET_IMAGE: [u8; 100] =
    prog::encode_net_tx_exit(crate::bsp::board::memmap::USER_PACKET_POOL_VA, 0, 1);

const fn slots_with(console: Option<u8>) -> [Option<u8>; MAX_SLOTS] {
    let mut slots = [None; MAX_SLOTS];
    slots[CONSOLE_SLOT] = console;
    slots
}

/// Built-in table when no external store is present.
fn builtin_manifest() -> &'static [AgentEntry] {
    #[cfg(feature = "oracle")]
    {
        static M: &[AgentEntry] = &[
            AgentEntry {
                name: "beacon",
                image: &CONSOLE_HI,
                text_pages: 1,
                stack_pages: 3,
                slots: slots_with(Some(HELD_CONSOLE)),
                may_resolve: false,
                device: None,
                packet_pool: false,
                home_cpu: 0,
            },
            AgentEntry {
                name: "mute",
                image: &CONSOLE_HI,
                text_pages: 2,
                stack_pages: 3,
                slots: slots_with(None),
                may_resolve: false,
                device: None,
                packet_pool: false,
                home_cpu: 0,
            },
            // ADR-0100: the device half of what `mute` is for. It names a
            // window **past the end** of the vocabulary, so every oracle boot
            // shows the arithmetic refusal on the good path — a composition
            // cannot reach a page the board never declared.
            //
            // The index is deliberately not 0: since ADR-0101 the product
            // declares `rng` there, and naming it would exercise the *vacancy*
            // path instead (which `entropy` already covers on a board with no
            // RNG200). Two different refusals, one agent each.
            AgentEntry {
                name: "lookup",
                image: &LOOKUP_CONSOLE,
                text_pages: 1,
                stack_pages: 3,
                slots: slots_with(None),
                may_resolve: true,
                device: None,
                packet_pool: false,
                home_cpu: 0,
            },
            AgentEntry {
                name: "noresolve",
                image: &LOOKUP_CONSOLE,
                text_pages: 1,
                stack_pages: 3,
                slots: slots_with(None),
                may_resolve: false,
                device: None,
                packet_pool: false,
                home_cpu: 0,
            },
            // ADR-0100: the device half of what `mute` is for. It names a
            // window **past the end** of the vocabulary, so every oracle boot
            // shows the arithmetic refusal on the good path.
            AgentEntry {
                name: "nowindow",
                image: &CONSOLE_HI,
                text_pages: 1,
                stack_pages: 3,
                slots: slots_with(Some(HELD_CONSOLE)),
                may_resolve: false,
                device: Some(kernel_core::manifest::DeviceGrant {
                    va: 0x9000,
                    window: 3,
                }),
                packet_pool: false,
                home_cpu: 0,
            },
            #[cfg(feature = "board-qemu-virt")]
            AgentEntry {
                name: "edge-gateway",
                image: &NET_IMAGE,
                text_pages: 1,
                stack_pages: 3,
                slots: [
                    Some(super::authority::HELD_NET_TX),
                    Some(super::authority::HELD_NET_TX_COMPLETE),
                    Some(super::authority::HELD_NET_RX),
                    Some(super::authority::HELD_NET_RX_RETURN),
                ],
                may_resolve: false,
                device: None,
                packet_pool: true,
                home_cpu: 0,
            },
        ];
        M
    }
    #[cfg(not(feature = "oracle"))]
    {
        static M: &[AgentEntry] = &[AgentEntry {
            name: "beacon",
            image: &CONSOLE_HI,
            text_pages: 1,
            stack_pages: 3,
            slots: slots_with(Some(HELD_CONSOLE)),
            may_resolve: false,
            device: None,
            packet_pool: false,
            home_cpu: 0,
        }];
        M
    }
}

/// Loader side tables: the manifest in force this boot, which entry each task
/// came from, and the device page each was planned to get.
///
/// One mutex for all three, because `entry_for_task` needs them together and a
/// non-re-entrant lock cannot be taken twice on that path (ADR-0091).
struct SideTables {
    active: Option<&'static [AgentEntry]>,
    entry_of_task: [Option<u8>; MAX_TASKS],
    /// The device page each task was planned to get (ADR-0100), resolved
    /// against the window vocabulary before the spawn.
    ///
    /// Carried per task rather than re-resolved in the agent body, because the
    /// plan already decided it: `agent_body` executes a decision, it does not
    /// make one (ADR-0097). It also means the body has no reason to reach for
    /// the vocabulary, and so no way to reach a window its entry never named.
    window_of_task: [Option<ResolvedWindow>; MAX_TASKS],
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
    window_of_task: [None; MAX_TASKS],
});

/// Resolve a task's manifest entry, and the device page its plan resolved, under
/// a **single** lock hold.
///
/// The mutex is not re-entrant: separate `recall` + `active_manifest` helpers
/// would deadlock on the agent body path. The window rides along for the same
/// reason — a second hold to fetch it would be the same deadlock, one field
/// later.
fn entry_for_task(task: TaskId) -> Option<(&'static AgentEntry, Option<ResolvedWindow>)> {
    SIDE.with(|side| {
        let index = side.entry_of_task[task.slot()]?;
        let m = side.active.unwrap_or_else(builtin_manifest);
        let entry = m.get(index as usize)?;
        Some((entry, side.window_of_task[task.slot()]))
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
        may_resolve: false,
        packet_pool: false,
        window: agentstore::WINDOW_NONE,
        device_va: 0,
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

/// Create one task per active manifest entry, binding slots against the
/// composition's vocabulary (ADR-0099).
///
/// The judgement — which manifest is in force, what an empty one means, and
/// per entry the order `validate` → `bind` → act — is
/// [`kernel_core::loaderplan`] (ADR-0097). What is left here is what a pure
/// function cannot do: parse the store, spawn, remember, and print.
///
/// Takes the whole [`super::authority::Authority`] rather than its slices
/// because a vacancy is worth naming: `bind` can only say *position 1 is
/// empty*, and the reader of a boot log wants to know that the position was
/// `blob` — or, for a device window, `rng`.
pub fn load_all(auth: &super::authority::Authority) {
    let (source, table) = match try_store_manifest() {
        Some(t) => (loaderplan::Source::Store { agents: t.len() }, t),
        None => (loaderplan::Source::Builtin, builtin_manifest()),
    };
    match source {
        loaderplan::Source::Store { agents } => crate::kprintln!("loader: store n={agents} image"),
        loaderplan::Source::Builtin => crate::kprintln!("loader: builtin"),
    }
    SIDE.with(|side| side.active = Some(table));

    let mut plans =
        [loaderplan::EntryPlan::Refuse(loaderplan::Refusal::Invalid(BindError::BadGeometry {
            text_pages: 0,
            stack_pages: 0,
        })); MAX_AGENTS];
    let planned = match loaderplan::plan(
        table,
        auth.held.as_slice(),
        auth.windows.as_slice(),
        kernel_core::paging::PAGE_SIZE as usize,
        &mut plans,
    ) {
        Ok(n) => n,
        Err(loaderplan::PlanError::Empty) => {
            crate::kprintln!("loader: manifest empty, nothing to create");
            return;
        }
        Err(loaderplan::PlanError::OutTooSmall { entries, capacity }) => {
            crate::kprintln!("loader: manifest has {entries} entries, room for {capacity}");
            return;
        }
    };

    for (entry, plan) in table.iter().zip(plans.iter().take(planned)) {
        match *plan {
            loaderplan::EntryPlan::Spawn {
                index,
                home_cpu,
                slots,
                device,
            } => {
                // ADR-0088: sticky home, decided by the plan from the entry.
                //
                // Spawn and remember under **one** hold of the side tables. The
                // spawn admits the task to `home_cpu`'s ready queue, and for
                // `home_cpu = 1` that queue belongs to a CPU that is already
                // running: it can dispatch the task before this core reaches
                // `remember`, and `agent_body` then finds no entry for itself
                // and returns without ever entering EL0. Seen on 2026-08-11 in
                // a `make check` whose host was busy — `loader: a task reached
                // the agent body with no manifest entry`, between beacon's line
                // and chirp's.
                //
                // Holding across the spawn closes it rather than narrowing it:
                // `entry_for_task` takes the same lock, so a task that beats us
                // to a CPU waits for the mapping instead of missing it. Lock
                // order SIDE → SCHED, documented in `crate::sync`.
                let spawned: Result<TaskId, sched::SpawnError> = SIDE.with(|side| {
                    let task = sched::spawn_with_slots_on(home_cpu, agent_body, &slots)?;
                    if entry.may_resolve && !sched::grant_resolve(task) {
                        crate::kprintln!("loader: {} resolve grant FAILED", entry.name);
                    }
                    side.entry_of_task[task.slot()] = Some(index);
                    side.window_of_task[task.slot()] = device;
                    Ok(task)
                });
                match spawned {
                    Ok(_) => crate::kprintln!(
                        "loader: {} loaded text={} stack={} home={}",
                        entry.name,
                        entry.text_pages,
                        entry.stack_pages,
                        home_cpu
                    ),
                    Err(e) => crate::kprintln!("loader: {} spawn FAILED {e:?}", entry.name),
                }
            }
            loaderplan::EntryPlan::Refuse(loaderplan::Refusal::Unheld(
                BindError::NoSuchCapability {
                    slot,
                    index,
                    held: declared,
                },
            )) => crate::kprintln!(
                "loader: {} refused — slot {slot} names capability {index} of {declared}",
                entry.name
            ),
            loaderplan::EntryPlan::Refuse(loaderplan::Refusal::Unheld(
                BindError::PacketPoolRequired { slot, index },
            )) => crate::kprintln!(
                "loader: {} refused — slot {slot} capability {index} requires packet pool",
                entry.name
            ),
            // ADR-0099: the position exists and nothing was minted into it. A
            // different sentence from the one above on purpose — this is the
            // kernel having come up short, not the composition over-reaching —
            // and the name is what a reader needs to know which service it was.
            loaderplan::EntryPlan::Refuse(loaderplan::Refusal::Unheld(BindError::HeldVacant {
                slot,
                index,
            })) => crate::kprintln!(
                "loader: {} refused — slot {slot} names {} which is VACANT",
                entry.name,
                auth.held.name_of(index).unwrap_or("?")
            ),
            // ADR-0100: the device half of the two refusals above. Named the
            // same way and for the same reason — one says the composition asked
            // for a window the board never declared, the other that the device
            // behind a declared one is not on this board.
            loaderplan::EntryPlan::Refuse(loaderplan::Refusal::Unheld(
                BindError::NoSuchWindow { index, windows },
            )) => crate::kprintln!(
                "loader: {} refused — names window {index} of {windows}",
                entry.name
            ),
            loaderplan::EntryPlan::Refuse(loaderplan::Refusal::Unheld(
                BindError::WindowVacant { index },
            )) => crate::kprintln!(
                "loader: {} refused — window {} is VACANT",
                entry.name,
                auth.windows.name_of(index).unwrap_or("?")
            ),
            loaderplan::EntryPlan::Refuse(
                loaderplan::Refusal::Invalid(e) | loaderplan::Refusal::Unheld(e),
            ) => crate::kprintln!("loader: {} refused — {e:?}", entry.name),
        }
    }
}

fn agent_body() {
    let Some((entry, window)) = entry_for_task(sched::current_task_id()) else {
        crate::kprintln!("loader: a task reached the agent body with no manifest entry");
        return;
    };
    run(entry, window);
}

fn run(entry: &AgentEntry, window: Option<ResolvedWindow>) {
    let name = entry.name;

    if let Err(e) = entry.validate(kernel_core::paging::PAGE_SIZE as usize) {
        crate::kprintln!("loader: {name} refused — {e:?}");
        return;
    }

    let mut aspace = match AddressSpace::create_with_packet_pool(
        entry.text_pages,
        entry.stack_pages,
        entry.packet_pool,
    ) {
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
    // ADR-0100: the page and its rights come from the vocabulary, resolved by
    // the plan. `Perms::USER_RW` used to be welded in here, which meant a
    // read-only device could not be expressed even in principle.
    if let Some(w) = window
        && let Err(e) = aspace.map_device_page(w.va, w.pa, w.perms)
    {
        crate::kprintln!("loader: {name} device grant FAILED {e:?}");
        aspace.destroy();
        return;
    }
    if entry.packet_pool {
        #[cfg(feature = "board-qemu-virt")]
        let mapped =
            network_runtime::packet_pool_pages().map(|pages| aspace.map_packet_pool(&pages));
        #[cfg(not(feature = "board-qemu-virt"))]
        let mapped: Option<Result<(), crate::mm::AsError>> = None;
        match mapped {
            Some(Ok(())) => {}
            Some(Err(error)) => {
                crate::kprintln!("loader: {name} packet pool FAILED {error:?}");
                aspace.destroy();
                return;
            }
            None => {
                crate::kprintln!("loader: {name} packet pool unavailable");
                aspace.destroy();
                return;
            }
        }
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
