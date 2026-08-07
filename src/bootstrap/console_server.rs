//! EL1 console server: drain the console mailbox onto the shared UART (M8).
//!
//! Holds the **recv** end of the console channel. Parks on
//! [`crate::ipc::recv`] when empty; on a message, drains **all** currently
//! queued messages without yielding so a multi-byte burst stays contiguous
//! once this task runs (design K13).
//!
//! Never transmits from an IRQ path (architecture rule 6). Unknown message
//! tags are dropped without TX (design K16).

use crate::console;
use crate::ipc::{self, Message};
use crate::sched;
use kernel_core::prog::CONSOLE_TAG_BYTE;

/// Resident server body. Spawned with the console recv cap at slot 0.
pub fn run() {
    let Some(cap) = sched::my_cap(0) else {
        // spawn_with_caps always places the first cap at slot 0; empty is a
        // bootstrap bug, not a recoverable state.
        crate::kprintln!("console-server: no recv cap at slot 0");
        return;
    };
    loop {
        match ipc::recv(cap) {
            Ok(first) => {
                write_console_msg(first);
                while let Ok(msg) = ipc::try_recv(cap) {
                    write_console_msg(msg);
                }
            }
            Err(ipc::RecvError::BadCap | ipc::RecvError::Busy) => {
                crate::kprintln!("console-server: recv FAILED");
                return;
            }
            Err(ipc::RecvError::Empty) => {
                // Blocking recv never returns Empty for a non-idle task.
                crate::kprintln!("console-server: unexpected Empty");
                return;
            }
        }
    }
}

fn write_console_msg(msg: Message) {
    if msg.tag != u32::from(CONSOLE_TAG_BYTE) {
        return;
    }
    let byte = (msg.a & 0xFF) as u8;
    let _ = console::with_tx(|uart| uart.write_byte(byte));
}
