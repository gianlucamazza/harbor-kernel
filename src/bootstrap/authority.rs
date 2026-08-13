//! The vocabulary this product's compositions may name (ADR-0099).
//!
//! A store entry grants authority by writing an **index** into its slots, and
//! this file is what those integers mean. It is deliberately the only place
//! they are written: before ADR-0099 the list was three lines in the middle of
//! [`super::run`], so adding a service meant editing the boot sequence, and
//! nothing on the packer's side said what index 1 was.
//!
//! # Declare first, mint second
//!
//! [`kernel_core::held::Set`] separates reserving a position from filling it,
//! and [`assemble`] uses that separation for its whole shape: every position is
//! declared up front, then each service is started and provides its capability
//! if it came up. A service that fails leaves a **hole** at its own index and
//! moves nothing else, so a composition that names index 1 either gets index 1
//! or is refused — never index 0's authority by accident.
//!
//! The console line proves it either way: one `authority:` line per position,
//! `ok` or `VACANT`, before the loader runs.

use kernel_core::held::{DeclareError, Held, Window, Windows};
use kernel_core::paging::Perms;

use super::console_server;

/// The console send end (M8).
///
/// The index is written down rather than only returned by [`Set::declare`],
/// because it is an **ABI**: `scripts/agent/pack-store.py` writes this integer
/// into a store entry's slots, and `make vocabulary-sync` compares the two
/// files. `assemble` asserts that the declaration actually landed here, so the
/// constant cannot drift from the order below.
pub const HELD_CONSOLE: u8 = 0;
pub const NAME_CONSOLE: &str = "console";

/// The SoC RNG200 page (ADR-0101).
///
/// Granted **read-only**: an agent composed to read entropy has no business
/// writing the control register, and since ADR-0100 the rights travel with the
/// window rather than being welded into the mapping site.
pub const WINDOW_RNG: u8 = 0;
pub const WINDOW_NAME_RNG: &str = "rng";

/// Both vocabularies a composition may name, assembled for this boot.
///
/// Two alphabets, one mechanism ([`kernel_core::held::Set`]): capabilities are
/// handed to an agent's slot table, windows are mapped into its address space,
/// and both are named by an index the composition chooses at compose time.
pub struct Authority {
    /// What a store entry's `slots` index (ADR-0099).
    pub held: Held,
    /// What a store entry's device grant indexes (ADR-0100).
    ///
    /// **Empty in this product.** ADR-0100 built the mechanism and deliberately
    /// declared no window with it: granting a device is a composition decision,
    /// and the first one arrives with the first composed driver-agent. Until
    /// then every entry naming a window is refused by arithmetic — `index >= 0`
    /// — which is the property being shipped.
    pub windows: Windows,
}

/// Assemble the product's vocabulary: declare every position, mint what can be
/// minted, and say on the wire which positions are filled.
///
/// Nothing here refuses a boot. A service that cannot start leaves its position
/// empty and the agents that named it are refused by the loader — the kernel
/// coming up short is a different fact from a composition asking for too much,
/// and both are printed.
pub fn assemble(rng_present: bool) -> Authority {
    let mut set = Held::new();

    let console = declare_or_report(&mut set, NAME_CONSOLE, HELD_CONSOLE);

    // Console: the channel's send end is the authority; the recv end stays with
    // the resident server. Minting and declaring are the same event, which is
    // why the mint moved here from `bootstrap::run` (ADR-0099 §3).
    if let Some(index) = console {
        match start_console_service() {
            Some(cap) => provide_or_report(&mut set, index, cap, NAME_CONSOLE),
            None => crate::kprintln!("authority: {index} {NAME_CONSOLE} VACANT"),
        }
    }

    // ADR-0100/0101: the device-window vocabulary. One position, the RNG200
    // page, provided only on a board that has the block — `rng_present` is the
    // boot's own probe answering, not a second one.
    let mut windows = Windows::new();
    if let Some(index) = declare_window(&mut windows, WINDOW_NAME_RNG, WINDOW_RNG) {
        provide_window(&mut windows, index, WINDOW_NAME_RNG, rng_present);
    }
    crate::kprintln!("authority: windows {} declared", windows.len());

    Authority { held: set, windows }
}

/// Declare a position, or say why the vocabulary refused it.
///
/// A refusal here is a bug in this file — a duplicate name or more positions
/// than [`kernel_core::held::MAX_HELD`] — so it prints and continues rather than
/// halting: the boot is still worth having with one service missing, and the
/// line names which.
fn declare_or_report(set: &mut Held, name: &'static str, expected: u8) -> Option<u8> {
    match set.declare(name) {
        Ok(index) if index == expected => Some(index),
        Ok(index) => {
            // The declarations above this one changed and the constant did not.
            // Refusing the position is the safe half: an agent that named the
            // constant would otherwise reach whatever now sits at that index.
            crate::kprintln!(
                "authority: {name} declared at {index}, ABI says {expected} — REFUSED"
            );
            None
        }
        Err(DeclareError::Duplicate { name, index }) => {
            crate::kprintln!("authority: {name} already declared at {index}");
            None
        }
        Err(DeclareError::Full { max }) => {
            crate::kprintln!("authority: {name} not declared — vocabulary full at {max}");
            None
        }
    }
}

fn provide_or_report(set: &mut Held, index: u8, cap: kernel_core::cap::CapId, name: &str) {
    match set.provide(index, cap) {
        Ok(()) => crate::kprintln!("authority: {index} {name} ok"),
        Err(e) => crate::kprintln!("authority: {index} {name} VACANT {e:?}"),
    }
}

/// Mint the console channel and start the resident EL1 server (M8).
///
/// The send end is what agents receive through the vocabulary; the recv end
/// stays with the server. Authority is ordinary `CapRights::SEND`: there is no
/// special console capability type since `SYS_PUTC` was removed.
///
/// `None` if the channel could not be created; the boot goes on without a
/// console endpoint rather than refusing, because the UART still works.
fn start_console_service() -> Option<kernel_core::cap::CapId> {
    match crate::ipc::create_channel() {
        Ok(ch) => {
            match crate::sched::spawn_with_caps(console_server::run, &[ch.recv]) {
                Ok(_) => crate::kprintln!("console-server: up"),
                Err(e) => crate::kprintln!("console-server: spawn FAILED {e:?}"),
            }
            crate::kprintln!("console: capability minted");
            Some(ch.send)
        }
        Err(e) => {
            crate::kprintln!("console: capability FAILED {e:?}");
            None
        }
    }
}

/// Declare a window position, or say why the vocabulary refused it.
///
/// Same shape as [`declare_or_report`], and separate from it because the two
/// vocabularies have separate ceilings and separate ABI constants — sharing one
/// helper would mean one of them silently checking the other's.
fn declare_window(set: &mut Windows, name: &'static str, expected: u8) -> Option<u8> {
    match set.declare(name) {
        Ok(index) if index == expected => Some(index),
        Ok(index) => {
            crate::kprintln!(
                "authority: window {name} declared at {index}, ABI says {expected} — REFUSED"
            );
            None
        }
        Err(DeclareError::Duplicate { name, index }) => {
            crate::kprintln!("authority: window {name} already declared at {index}");
            None
        }
        Err(DeclareError::Full { max }) => {
            crate::kprintln!("authority: window {name} not declared — vocabulary full at {max}");
            None
        }
    }
}

/// Provide the RNG window if the board has the device, and say which happened.
///
/// **`absent` is not `FAILED`** (ADR-0101 §2). A vacancy in the capability
/// vocabulary means a service that should have started did not; a vacancy here
/// can simply mean this board has no such block — which is what QEMU's raspi4b
/// is, and a correct boot of a correct kernel. The two get different words so a
/// gate can accept one and refuse the other, and so a reader is not told
/// something broke when nothing did.
fn provide_window(set: &mut Windows, index: u8, name: &str, present: bool) {
    if !present {
        crate::kprintln!("authority: {index} {name} absent");
        return;
    }
    let window = Window {
        pa: crate::bsp::board::memmap::RNG200_BASE as u64,
        perms: Perms::USER_RO,
    };
    match set.provide(index, window) {
        Ok(()) => crate::kprintln!("authority: {index} {name} ok"),
        Err(e) => crate::kprintln!("authority: {index} {name} FAILED {e:?}"),
    }
}
