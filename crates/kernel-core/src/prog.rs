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
//! test, and the oracle is `llvm-mc`: the intended assembly goes *through the
//! assembler* and the comparison is on bytes. Nobody transcribes hex in either
//! direction. When the tool is missing the tests **fail** rather than skip — the
//! lesson of `make no-simd`, which once reported `clean` having disassembled
//! nothing.
//!
//! The first version went the other way — disassemble the encoder's bytes,
//! compare the printed mnemonics — and CI killed it on the first push: the
//! runner's `llvm-mc` prints a `.text` directive that the development machine's
//! does not. Disassembly output is a *rendering*, and a rendering is not an
//! interface. Assembly input is.
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

/// Append one instruction word, little-endian.
///
/// Byte-by-byte rather than `copy_from_slice`, so the whole encoder family can
/// be `const fn`: a manifest entry's image is a `const [u8; N]` in `.rodata`
/// (ADR-0021 §3), and that only works if the function that builds it runs at
/// compile time. Slice methods are not const; four index assignments are.
#[inline]
const fn push_word(out: &mut [u8], at: &mut usize, word: u32) {
    let b = a64::le_bytes(word);
    out[*at] = b[0];
    out[*at + 1] = b[1];
    out[*at + 2] = b[2];
    out[*at + 3] = b[3];
    *at += 4;
}

/// A64: `movz x0,#slot; movz x1,#tag; movz x2,#a; svc #3; svc #1; b .`
///
/// Send one message through a slot, then exit. `b` is left zero — three
/// immediates are enough to see a payload cross, and `movz` carries 16 bits.
pub const fn encode_send_exit(slot: u16, tag: u16, a: u16) -> [u8; 24] {
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
pub const fn encode_recv_putc_exit(recv_slot: u16, console_slot: u16) -> [u8; 28] {
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
pub const fn encode_putc_once_exit(slot: u16, byte: u8) -> [u8; 20] {
    let mut out = [0u8; 20];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, byte as u16));
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
pub const fn encode_send_bare_exit(slot: u16) -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `movz x0,#slot; svc #5; svc #1; b .` — try to receive, then exit.
///
/// The non-blocking half of the recv pair (ADR-0022 §4). Pointed at an empty
/// mailbox it is the only program in the tree that produces
/// [`syscall::Status::Empty`] — which is why it exists as a demo rather than as
/// a host test: once `SYS_RECV` waits, nothing else reaches that status, and a
/// status no program can produce is a status that stops being maintained.
pub const fn encode_try_recv_exit(slot: u16) -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_TRY_RECV));
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
pub const fn encode_fault_exit() -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x_lsl16(0, 0x8));
    push_word(&mut out, &mut i, a64::str_xzr(0));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `svc #imm` ; `b .`
pub const fn encode_svc_imm(imm: u16) -> [u8; 8] {
    let mut out = [0u8; 8];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::svc(imm));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `svc #0; svc #0; svc #1; b .` — two pings then exit (resume path).
pub const fn encode_ping_ping_exit() -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::svc(0));
    push_word(&mut out, &mut i, a64::svc(0));
    push_word(&mut out, &mut i, a64::svc(1));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `movz x0, #'H'; svc #2; movz x0, #'!'; svc #2; svc #1; b .`
pub const fn encode_putc_hi_exit(slot: u16) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, b'H' as u16));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_PUTC));
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, b'!' as u16));
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
pub const fn encode_spin_exit(iters: u16) -> [u8; 20] {
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
pub const fn encode_pl011_rx_poll_exit(console_slot: u16) -> [u8; 32] {
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

    /// Assemble `asm` for AArch64 and return the bytes of `.text`.
    ///
    /// The direction matters. The first version of these tests **disassembled**
    /// the encoder's bytes and compared the printed mnemonics, and CI killed it
    /// on the first push: the runner's `llvm-mc` emits a `.text` directive line
    /// that the development machine's does not. Disassembly output is a
    /// *rendering* — directives, `#2` versus `#0x2`, trailing `// =0x…`
    /// annotations — and none of that is a stable interface.
    ///
    /// Assembly *input* is. So the intended program is written as assembly, the
    /// assembler turns it into bytes, and the comparison is on bytes. Nothing a
    /// future LLVM chooses to print can move it.
    ///
    /// # Panics
    /// If the LLVM tools are missing, or either step fails. Deliberately: a
    /// check that cannot run must not report clean — `make no-simd` once printed
    /// `clean` having disassembled nothing.
    fn assemble(asm: &str) -> Vec<u8> {
        let dir = std::env::temp_dir().join(format!(
            "harbor-prog-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let obj = dir.join("prog.o");
        let bin = dir.join("prog.bin");

        let mut mc = Command::new("llvm-mc")
            .args(["--assemble", "--triple=aarch64", "-filetype=obj", "-o"])
            .arg(&obj)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect(
                "llvm-mc is required to check the EL0 programs and is not on the PATH. \
                 It ships with the LLVM tools the Makefile already needs for llvm-objcopy \
                 and llvm-objdump. Refusing to pass without assembling.",
            );
        mc.stdin
            .take()
            .expect("stdin")
            .write_all(asm.as_bytes())
            .expect("write asm to llvm-mc");
        let out = mc.wait_with_output().expect("llvm-mc output");
        assert!(
            out.status.success(),
            "llvm-mc failed on:\n{asm}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );

        let out = Command::new("llvm-objcopy")
            .args(["-O", "binary", "--only-section=.text"])
            .arg(&obj)
            .arg(&bin)
            .output()
            .expect("llvm-objcopy is required and is not on the PATH");
        assert!(
            out.status.success(),
            "llvm-objcopy failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let bytes = std::fs::read(&bin).expect("read .text");
        let _ = std::fs::remove_dir_all(&dir);
        bytes
    }

    /// Compare an encoder's output against the assembly it is meant to be.
    fn assert_program(name: &str, bytes: &[u8], asm: &str) {
        let expected = assemble(asm);
        assert_eq!(
            bytes,
            expected.as_slice(),
            "\n{name} does not match the assembly it documents:\n{asm}\n\
             encoder:   {bytes:02x?}\n assembler: {expected:02x?}\n"
        );
    }

    #[cfg_attr(
        miri,
        ignore = "shells out to llvm-mc, which Miri cannot run; these encoders are pure and a64's unit tests cover the instruction words"
    )]
    #[test]
    fn every_el0_program_assembles_to_the_instructions_it_documents() {
        assert_program("encode_svc_imm(0)", &encode_svc_imm(0), "svc #0\nb .\n");

        assert_program(
            "encode_ping_ping_exit",
            &encode_ping_ping_exit(),
            "svc #0\nsvc #0\nsvc #1\nb .\n",
        );

        assert_program(
            "encode_send_exit(0, 7, 42)",
            &encode_send_exit(0, 7, 42),
            "movz x0, #0\nmovz x1, #7\nmovz x2, #42\nsvc #3\nsvc #1\nb .\n",
        );

        assert_program(
            "encode_send_bare_exit(1)",
            &encode_send_bare_exit(1),
            "movz x0, #1\nsvc #3\nsvc #1\nb .\n",
        );

        assert_program(
            "encode_putc_once_exit(1, b'X')",
            &encode_putc_once_exit(1, b'X'),
            "movz x0, #1\nmovz x1, #88\nsvc #2\nsvc #1\nb .\n",
        );

        assert_program(
            "encode_putc_hi_exit(1)",
            &encode_putc_hi_exit(1),
            "movz x0, #1\nmovz x1, #72\nsvc #2\n\
             movz x0, #1\nmovz x1, #33\nsvc #2\nsvc #1\nb .\n",
        );

        // The payload arrives in x2 and SYS_PUTC wants it in x1, under the
        // console slot in x0 — two moves, because the two calls disagree about
        // where a byte lives.
        assert_program(
            "encode_recv_putc_exit(0, 1)",
            &encode_recv_putc_exit(0, 1),
            "movz x0, #0\nsvc #4\nmov x1, x2\nmovz x0, #1\nsvc #2\nsvc #1\nb .\n",
        );

        // `svc #5`, not `#4`: the two recvs are different calls, and an agent
        // that must not wait says so at the call site.
        assert_program(
            "encode_try_recv_exit(0)",
            &encode_try_recv_exit(0),
            "movz x0, #0\nsvc #5\nsvc #1\nb .\n",
        );

        // Writes to a kernel address the agent does not have: a data abort.
        assert_program(
            "encode_fault_exit",
            &encode_fault_exit(),
            "movz x0, #8, lsl #16\nstr xzr, [x0]\nsvc #1\nb .\n",
        );

        assert_program(
            "encode_spin_exit(0x800)",
            &encode_spin_exit(0x800),
            "movz x0, #2048\nsub x0, x0, #1\ncbnz x0, #-4\nsvc #1\nb .\n",
        );
    }

    /// The one that earns the suite.
    ///
    /// `tbnz` skips the load, the slot and the `putc` when the RX FIFO is empty
    /// — three instructions, so the target is four words past the branch. That
    /// offset was recomputed by hand the day `SYS_PUTC` grew a slot argument,
    /// and a wrong value fails nothing: it prints `rx poll unexpected putcs=…`
    /// on a board, which reads like a kernel bug.
    #[cfg_attr(
        miri,
        ignore = "shells out to llvm-mc, which Miri cannot run; these encoders are pure and a64's unit tests cover the instruction words"
    )]
    #[test]
    fn the_rx_poll_branch_skips_exactly_the_putc_path() {
        assert_program(
            "encode_pl011_rx_poll_exit(1)",
            &encode_pl011_rx_poll_exit(1),
            "movz x0, #0x5000, lsl #16\n\
             ldr w1, [x0, #24]\n\
             tbnz w1, #4, #16\n\
             ldrb w1, [x0]\n\
             movz x0, #1\n\
             svc #2\n\
             svc #1\n\
             b .\n",
        );
    }
}
