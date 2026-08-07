//! The machine code an EL0 agent runs, assembled by hand and checked by a
//! disassembler.
//!
//! Every agent in the boot oracle — on QEMU and on silicon — executes one of the
//! byte sequences below. They used to live in `src/agent`, where they were pure
//! functions with no tests: their only check was that the boot passed. But
//! `el0-task: putc bytes=2` proves the *kernel* counted two `SYS_PUTC`, not that
//! the program said what anyone believed. An encoder wrong in a way that still
//! produces two putcs passes that assertion.
//!
//! # Why the tests disassemble instead of comparing bytes
//!
//! A test carrying the expected bytes as hex would be the same transcription
//! risk one floor up: two hand-encodings agreeing with each other proves they
//! were written by the same person on the same day.
//!
//! So the specification is the **assembly in plain text**, sitting beside each
//! test, and the oracle is `llvm-mc --disassemble`. Nobody transcribes hex in
//! either direction. When the tool is missing the tests **fail** rather than
//! skip — the lesson of `make no-simd`, which once reported `clean` having
//! disassembled nothing.
//!
//! The one that earns its keep is [`encode_pl011_rx_poll_exit`]: its `tbnz`
//! skips three instructions, and the branch offset had to be recomputed by hand
//! the day `SYS_PUTC` grew a slot argument. A wrong offset there does not fail a
//! test — it produces `rx poll unexpected putcs=…` on a board, and sends whoever
//! reads it into the kernel instead of into the program.
//!
//! [`kernel_core::a64`] states the same principle about instruction words:
//! host-tested so nothing invents an encoding by hand in two places. This module
//! is the layer above, which was left behind.

use crate::a64;
use crate::syscall;

#[inline]
fn push_word(out: &mut [u8], at: &mut usize, word: u32) {
    let b = a64::le_bytes(word);
    out[*at..*at + 4].copy_from_slice(&b);
    *at += 4;
}

/// A64: `movz x0,#slot; movz x1,#tag; movz x2,#a; svc #3; svc #1; b .`
///
/// Send one message through a slot, then exit. `b` is left zero — three
/// immediates are enough to see a payload cross, and `movz` carries 16 bits.
pub fn encode_send_exit(slot: u16, tag: u16, a: u16) -> [u8; 24] {
    let mut out = [0u8; 24];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, tag));
    push_word(&mut out, &mut i, a64::movz_x(2, a));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `movz x0,#slot; svc #4; mov x0,x2; svc #2; svc #1; b .`
///
/// Receive through a slot, then `SYS_PUTC` the payload the kernel delivered.
///
/// The `mov` is the evidence. `SYS_RECV` returns the status in `x0` and the
/// message in `x1..x3`, so an agent that printed `x0` would print a zero and
/// prove only that it resumed. Moving `x2` — the message's `a` field — into the
/// `putc` argument makes the byte on the console *the payload*, carried from
/// another agent's registers through the kernel into this one's.
pub fn encode_recv_putc_exit(recv_slot: u16, console_slot: u16) -> [u8; 28] {
    let mut out = [0u8; 28];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, recv_slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_RECV));
    // The payload arrives in x2 and `SYS_PUTC` wants it in x1, under the
    // console slot in x0. Two moves rather than one, because the two calls
    // disagree about where a byte lives — which is what an ABI table is for.
    push_word(&mut out, &mut i, a64::mov_x_reg(1, 2));
    push_word(&mut out, &mut i, a64::movz_x(0, console_slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_PUTC));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `movz x0,#slot; movz x1,#byte; svc #2; svc #1; b .`
///
/// One `SYS_PUTC` through a named slot, then exit. Pointed at a slot that holds
/// no console capability, this is the agent that must be refused on the good
/// path (ADR-0017 §3).
pub fn encode_putc_once_exit(slot: u16, byte: u8) -> [u8; 20] {
    let mut out = [0u8; 20];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, u16::from(byte)));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_PUTC));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `movz x0,#slot; svc #3; svc #1; b .` — send through a slot, then exit.
///
/// The hostile half of F22: pointed at a slot the task does not hold, this is
/// an agent reaching for authority it was not granted. It is a demo program
/// rather than a test because the refusal has to be *seen on the good path* —
/// a protection nobody watches fire is an assumption.
pub fn encode_send_bare_exit(slot: u16) -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `movz x0,#8,lsl#16; str xzr,[x0]; svc #1; b .`
///
/// Writes to a kernel address the agent does not have, which takes a data abort
/// at EL0. The `svc #1` after it is never reached — it is there so the program
/// says what it *meant* to do, rather than trailing off into a branch-to-self
/// that would look like the fault was the intent of the encoder rather than of
/// the test.
pub fn encode_fault_exit() -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x_lsl16(0, 0x8));
    push_word(&mut out, &mut i, a64::str_xzr(0));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `svc #imm` ; `b .`
pub fn encode_svc_imm(imm: u16) -> [u8; 8] {
    let mut out = [0u8; 8];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::svc(imm));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `svc #0; svc #0; svc #1; b .` — two pings then exit (resume path).
pub fn encode_ping_ping_exit() -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::svc(0));
    push_word(&mut out, &mut i, a64::svc(0));
    push_word(&mut out, &mut i, a64::svc(1));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `movz x0, #'H'; svc #2; movz x0, #'!'; svc #2; svc #1; b .`
pub fn encode_putc_hi_exit(slot: u16) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, u16::from(b'H')));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_PUTC));
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, u16::from(b'!')));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_PUTC));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// Finite spin then `SYS_EXIT` — GPRs survive IRQ save/restore, so this makes
/// forward progress under plain (architectural) IRQ resume.
///
/// ```text
/// movz x0, #iters          // low 16 bits; high half zero
/// 1: sub  x0, x0, #1
///    cbnz x0, 1b            // offset −1 word from the cbnz (gas-checked)
/// svc #1
/// b .
/// ```
///
/// Pair with [`el0::set_entry_irqs_unmasked`] and
/// [`crate::arch::timer::accelerate_next_tick`] so a timer IRQ arrives while
/// the counter is still non-zero.
pub fn encode_spin_exit(iters: u16) -> [u8; 20] {
    let mut out = [0u8; 20];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, iters));
    push_word(&mut out, &mut i, a64::sub_x_imm(0, 0, 1));
    push_word(&mut out, &mut i, a64::cbnz_x(0, -1));
    push_word(&mut out, &mut i, a64::svc(1));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// PL011 RX poll once at `USER_PL011_VA` (`0x5000_0000`):
/// if RX not empty, `SYS_PUTC` the byte; always `SYS_EXIT`.
///
/// Empty FIFO → zero putcs (honest “no data” path). A pending character → one
/// putc. Does not invent receive data.
pub fn encode_pl011_rx_poll_exit(console_slot: u16) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x_lsl16(0, 0x5000));
    push_word(&mut out, &mut i, a64::ldr_w_imm(1, 0, 0x18));
    // RXFE (bit 4) set → empty → skip the load, the slot and the putc
    push_word(&mut out, &mut i, a64::tbnz_w(1, 4, 4));
    push_word(&mut out, &mut i, a64::ldrb_w(1, 0));
    push_word(&mut out, &mut i, a64::movz_x(0, console_slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_PUTC));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// Disassemble `bytes` with `llvm-mc`, one mnemonic per line.
    ///
    /// # Panics
    /// If `llvm-mc` is not on the PATH. Deliberately: a check that cannot run
    /// must not report clean. `make no-simd` learned this the hard way — it
    /// once printed `clean` having disassembled nothing, because an empty
    /// pipeline made its `grep` fail and `!` inverted that into success.
    fn disassemble(bytes: &[u8]) -> Vec<String> {
        let hex: Vec<String> = bytes.iter().map(|b| format!("0x{b:02x}")).collect();
        let mut child = Command::new("llvm-mc")
            .args(["--disassemble", "--triple=aarch64"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect(
                "llvm-mc is required to check the EL0 programs and is not on the PATH. \
                 It ships with the same LLVM tools the Makefile already needs for \
                 llvm-objcopy and llvm-objdump. Refusing to pass without disassembling.",
            );
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(hex.join(" ").as_bytes())
            .expect("write hex to llvm-mc");
        let out = child.wait_with_output().expect("llvm-mc output");
        assert!(
            out.status.success(),
            "llvm-mc failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .lines()
            // The trailing `// =0x…` llvm-mc prints beside an immediate is an
            // annotation, not part of the instruction. Whitespace is collapsed
            // for the same reason: the tool aligns with tabs.
            .map(|l| l.split("//").next().unwrap_or(l))
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Compare a program against the assembly it is meant to be.
    fn assert_program(bytes: &[u8], expected: &[&str]) {
        let actual = disassemble(bytes);
        assert_eq!(
            actual, expected,
            "\n  intended: {expected:#?}\n  assembled: {actual:#?}\n"
        );
    }

    #[cfg_attr(
        miri,
        ignore = "shells out to llvm-mc, which Miri cannot run; these encoders are pure and the unit tests of a64 cover the words"
    )]
    #[test]
    fn every_el0_program_assembles_to_the_instructions_it_documents() {
        assert_program(&encode_svc_imm(0), &["svc #0", "b #0"]);

        assert_program(
            &encode_ping_ping_exit(),
            &["svc #0", "svc #0", "svc #0x1", "b #0"],
        );

        assert_program(
            &encode_send_exit(0, 7, 42),
            &[
                "mov x0, #0",
                "mov x1, #7",
                "mov x2, #42",
                "svc #0x3",
                "svc #0x1",
                "b #0",
            ],
        );

        assert_program(
            &encode_send_bare_exit(1),
            &["mov x0, #1", "svc #0x3", "svc #0x1", "b #0"],
        );

        assert_program(
            &encode_putc_once_exit(1, b'X'),
            &["mov x0, #1", "mov x1, #88", "svc #0x2", "svc #0x1", "b #0"],
        );

        assert_program(
            &encode_putc_hi_exit(1),
            &[
                "mov x0, #1",
                "mov x1, #72",
                "svc #0x2",
                "mov x0, #1",
                "mov x1, #33",
                "svc #0x2",
                "svc #0x1",
                "b #0",
            ],
        );

        // The payload arrives in x2 and SYS_PUTC wants it in x1, under the
        // console slot in x0 — two moves because the two calls disagree about
        // where a byte lives.
        assert_program(
            &encode_recv_putc_exit(0, 1),
            &[
                "mov x0, #0",
                "svc #0x4",
                "mov x1, x2",
                "mov x0, #1",
                "svc #0x2",
                "svc #0x1",
                "b #0",
            ],
        );

        // Writes to a kernel address the agent does not have: a data abort.
        assert_program(
            &encode_fault_exit(),
            &["mov x0, #524288", "str xzr, [x0]", "svc #0x1", "b #0"],
        );

        assert_program(
            &encode_spin_exit(0x800),
            &[
                "mov x0, #2048",
                "sub x0, x0, #1",
                "cbnz x0, #-4",
                "svc #0x1",
                "b #0",
            ],
        );
    }

    /// The one that earns the suite.
    ///
    /// `tbnz` skips the load, the slot and the `putc` when the RX FIFO is empty
    /// — three instructions, so the target is four words ahead of the branch.
    /// That offset was recomputed by hand the day `SYS_PUTC` grew a slot
    /// argument, and a wrong value does not fail anything: it produces
    /// `rx poll unexpected putcs=…` on a board, which reads like a kernel bug.
    #[cfg_attr(
        miri,
        ignore = "shells out to llvm-mc, which Miri cannot run; these encoders are pure and the unit tests of a64 cover the words"
    )]
    #[test]
    fn the_rx_poll_branch_skips_exactly_the_putc_path() {
        assert_program(
            &encode_pl011_rx_poll_exit(1),
            &[
                "mov x0, #1342177280", // PL011 window at 0x5000_0000
                "ldr w1, [x0, #24]",   // FR
                "tbnz w1, #4, #16",    // RXFE set → skip ldrb, movz, svc: 4 words
                "ldrb w1, [x0]",       // DR
                "mov x0, #1",          // console slot
                "svc #0x2",
                "svc #0x1",
                "b #0",
            ],
        );
    }
}
