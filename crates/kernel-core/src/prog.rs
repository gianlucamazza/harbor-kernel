//! The machine code an EL0 agent runs, assembled by hand and checked by a
//! disassembler.
//!
//! Every agent in the boot oracle — on QEMU and on silicon — executes one of the
//! byte sequences below. They used to live in `src/agent`, where they were pure
//! functions with no tests: their only check was that the boot passed. But
//! `el0-task: console sends=2` proves the *kernel* counted two `SYS_SEND`, not
//! that the program said what anyone believed. An encoder wrong in a way that
//! still produces two sends passes that assertion.
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
//! skips the poll path when RX is empty, and the branch offset had to be
//! recomputed the day console output moved from `SYS_PUTC` to `SYS_SEND`. A
//! wrong offset there does not fail a unit test of the kernel — it produces
//! `rx poll unexpected sends=…` on a board.
//!
//! [`kernel_core::a64`] states the same principle about instruction words:
//! host-tested so nothing invents an encoding by hand in two places. This module
//! is the layer above, which was left behind.

use crate::a64;
use crate::syscall;

/// Message `tag` for one console byte via `SYS_SEND` (M8).
///
/// The console server drops any other tag without TX.
pub const CONSOLE_TAG_BYTE: u16 = 0;

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

/// A64: recv through a slot, then `SYS_SEND` the payload byte to the console.
///
/// ```text
/// movz x0, #recv_slot
/// svc  #4                 // RECV → x1=tag, x2=a, x3=b
/// mov  x2, x2             // a stays in x2 (Message.a is the console byte)
/// movz x1, #CONSOLE_TAG
/// movz x0, #console_slot
/// svc  #3                 // SEND
/// svc  #1
/// b .
/// ```
///
/// `SYS_RECV` leaves the payload in `x2`. `SYS_SEND` wants tag in `x1` and `a`
/// in `x2`, so only the tag and slot need loading — the byte is already where
/// SEND wants it.
pub const fn encode_recv_console_exit(recv_slot: u16, console_slot: u16) -> [u8; 28] {
    let mut out = [0u8; 28];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, recv_slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_RECV));
    // x2 already holds Message.a (the payload byte). Load tag and console slot.
    push_word(&mut out, &mut i, a64::movz_x(1, CONSOLE_TAG_BYTE));
    push_word(&mut out, &mut i, a64::movz_x(0, console_slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: one console byte via `SYS_SEND`, then exit.
///
/// Pointed at a slot that holds no send capability, this is the agent that must
/// be refused on the good path (denied-by-default console).
pub const fn encode_console_once_exit(slot: u16, byte: u8) -> [u8; 24] {
    let mut out = [0u8; 24];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, CONSOLE_TAG_BYTE));
    push_word(&mut out, &mut i, a64::movz_x(2, byte as u16));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
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

/// Write one packet-pool slot, submit it through `net-tx`, await completion,
/// receive one `net-rx` token, return it through `net-rx-return`, then exit.
/// The pool VA is board-owned; the descriptor never crosses EL0.
pub const fn encode_net_tx_rx_exit(
    pool_va: u64,
    tx_slot: u16,
    complete_slot: u16,
    rx_slot: u16,
    rx_return_slot: u16,
) -> [u8; 120] {
    let mut out = [0u8; 120];
    let mut i = 0;
    push_u64(&mut out, &mut i, 0, pool_va);
    push_u64(&mut out, &mut i, 1, 0x1122_3344_5566_7788);
    push_word(&mut out, &mut i, a64::str_x_imm(1, 0, 0));
    push_u64(&mut out, &mut i, 1, 0x99AA_BBCC_DDEE_FF00);
    push_word(&mut out, &mut i, a64::str_x_imm(1, 0, 8));
    push_word(&mut out, &mut i, a64::movz_x(0, tx_slot));
    push_word(
        &mut out,
        &mut i,
        a64::movz_x(1, crate::net::TAG_TX_SUBMIT as u16),
    );
    push_u64(
        &mut out,
        &mut i,
        2,
        crate::net::packed_token(crate::net::PacketToken {
            slot: 1,
            generation: 0,
            len: 16,
        }),
    );
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::movz_x(0, complete_slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_RECV));
    push_word(&mut out, &mut i, a64::movz_x(0, rx_slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_RECV));
    // SYS_RECV leaves the packed RX token in x2. Preserve it while loading
    // the return endpoint and wire tag.
    push_word(&mut out, &mut i, a64::movz_x(0, rx_return_slot));
    push_word(
        &mut out,
        &mut i,
        a64::movz_x(1, crate::net::TAG_RX_RETURN as u16),
    );
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// Backward-compatible TX-only encoder for host callers that do not exercise
/// the receive side of the service.
pub const fn encode_net_tx_exit(pool_va: u64, tx_slot: u16, complete_slot: u16) -> [u8; 100] {
    let mut out = [0u8; 100];
    let mut i = 0;
    push_u64(&mut out, &mut i, 0, pool_va);
    push_u64(&mut out, &mut i, 1, 0x1122_3344_5566_7788);
    push_word(&mut out, &mut i, a64::str_x_imm(1, 0, 0));
    push_u64(&mut out, &mut i, 1, 0x99AA_BBCC_DDEE_FF00);
    push_word(&mut out, &mut i, a64::str_x_imm(1, 0, 8));
    push_word(&mut out, &mut i, a64::movz_x(0, tx_slot));
    push_word(
        &mut out,
        &mut i,
        a64::movz_x(1, crate::net::TAG_TX_SUBMIT as u16),
    );
    push_u64(
        &mut out,
        &mut i,
        2,
        crate::net::packed_token(crate::net::PacketToken {
            slot: 1,
            generation: 0,
            len: 16,
        }),
    );
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::movz_x(0, complete_slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_RECV));
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

/// A64: `movz x0,#slot; svc #6; svc #1; b .` — wait on IRQ notification, exit.
///
/// ADR-0030 / K1 remainder. Slot must hold an IRQ-notification cap (timer
/// cookie in the oracle). Empty slot is the authority-refuse path.
pub const fn encode_wait_irq_exit(slot: u16) -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_WAIT_IRQ));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// Little-endian `u64` of the product name `console` (ADR-0102).
pub const CONSOLE_NAME_LE: u64 = u64::from_le_bytes(*b"console\0");

/// Length of [`CONSOLE_NAME_LE`] without the padding NUL.
pub const CONSOLE_NAME_LEN: u16 = 7;

pub const BLOB_KEY_CFG: u64 = crate::blob::Field::pack(b"cfg");
pub const BLOB_PAYLOAD_PERSIST: u64 = crate::blob::Field::pack(b"persist");

const fn push_u64(out: &mut [u8], i: &mut usize, reg: u8, value: u64) {
    push_word(out, i, a64::movz_x(reg, (value & 0xffff) as u16));
    push_word(
        out,
        i,
        a64::movk_x_lsl16(reg, ((value >> 16) & 0xffff) as u16),
    );
    push_word(
        out,
        i,
        a64::movk_x_lsl32(reg, ((value >> 32) & 0xffff) as u16),
    );
    push_word(
        out,
        i,
        a64::movk_x_lsl48(reg, ((value >> 48) & 0xffff) as u16),
    );
}

/// Put `cfg=persist`, get it back, notify the console, then exit (P2).
///
/// The request and reply capabilities are separate slots because IPC endpoint
/// rights are directional. The program intentionally does not inspect the
/// reply payload: the service oracle checks the durable round-trip, and
/// reaching the later console send proves the blocking reply receive returned.
pub const fn encode_blob_round_trip_exit(
    request_slot: u16,
    reply_slot: u16,
    console_slot: u16,
) -> [u8; 112] {
    let mut out = [0u8; 112];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, request_slot));
    push_word(
        &mut out,
        &mut i,
        a64::movz_x(1, crate::blob::TAG_PUT as u16),
    );
    push_u64(&mut out, &mut i, 2, BLOB_KEY_CFG);
    push_u64(&mut out, &mut i, 3, BLOB_PAYLOAD_PERSIST);
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::movz_x(0, request_slot));
    push_word(
        &mut out,
        &mut i,
        a64::movz_x(1, crate::blob::TAG_GET as u16),
    );
    push_u64(&mut out, &mut i, 2, BLOB_KEY_CFG);
    push_word(&mut out, &mut i, a64::movz_x(3, 0));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::movz_x(0, reply_slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_RECV));
    push_word(&mut out, &mut i, a64::movz_x(0, console_slot));
    push_word(&mut out, &mut i, a64::movz_x(1, CONSOLE_TAG_BYTE));
    push_word(&mut out, &mut i, a64::movz_x(2, b'S' as u16));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: resolve `console` into `slot`, send one byte through it, then exit
/// (ADR-0102).
///
/// The name is seven bytes, so it takes a `movz` and three `movk` — the
/// existing [`encode_resolve_exit`] only carries a 16-bit immediate, which
/// is why the oracle demo bound `ab`.
pub const fn encode_resolve_send_exit(slot: u16, byte: u8) -> [u8; 52] {
    let mut out = [0u8; 52];
    let mut i = 0;
    let n = CONSOLE_NAME_LE;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, CONSOLE_NAME_LEN));
    push_word(&mut out, &mut i, a64::movz_x(2, (n & 0xffff) as u16));
    push_word(
        &mut out,
        &mut i,
        a64::movk_x_lsl16(2, ((n >> 16) & 0xffff) as u16),
    );
    push_word(
        &mut out,
        &mut i,
        a64::movk_x_lsl32(2, ((n >> 32) & 0xffff) as u16),
    );
    push_word(
        &mut out,
        &mut i,
        a64::movk_x_lsl48(2, ((n >> 48) & 0xffff) as u16),
    );
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_RESOLVE));
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, CONSOLE_TAG_BYTE));
    push_word(&mut out, &mut i, a64::movz_x(2, byte as u16));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: resolve short name into empty slot, then exit (ADR-0039).
///
/// `name_le` is up to two ASCII bytes little-endian in a 16-bit imm (e.g.
/// `b"ab"` → `0x6261`). Longer EL0 names need a later packing path.
pub const fn encode_resolve_exit(slot: u16, name_len: u16, name_le: u16) -> [u8; 24] {
    let mut out = [0u8; 24];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, name_len));
    push_word(&mut out, &mut i, a64::movz_x(2, name_le));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_RESOLVE));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: transfer from_slot → to_slot (dest: 0 self / 1 creator), then exit (ADR-0041).
pub const fn encode_transfer_exit(from: u16, to_slot: u16, dest: u16) -> [u8; 24] {
    let mut out = [0u8; 24];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, from));
    push_word(&mut out, &mut i, a64::movz_x(1, to_slot));
    push_word(&mut out, &mut i, a64::movz_x(2, dest));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_TRANSFER));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: peer transfer via task-cap slot, then exit (ADR-0054).
///
/// `dest` is forced to 2; `peer_cap_slot` is the local slot holding the task-cap.
pub const fn encode_transfer_peer_exit(from: u16, to_slot: u16, peer_cap_slot: u16) -> [u8; 28] {
    let mut out = [0u8; 28];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, from));
    push_word(&mut out, &mut i, a64::movz_x(1, to_slot));
    push_word(&mut out, &mut i, a64::movz_x(2, 2)); // peer
    push_word(&mut out, &mut i, a64::movz_x(3, peer_cap_slot));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_TRANSFER));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: recv with timeout ticks, then exit (ADR-0042).
pub const fn encode_recv_timeout_exit(slot: u16, timeout_ticks: u16) -> [u8; 20] {
    let mut out = [0u8; 20];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, timeout_ticks));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_RECV_TIMEOUT));
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

/// A64: two console bytes `H!` via `SYS_SEND`, then exit.
///
/// ```text
/// movz x0, #slot; movz x1, #0; movz x2, #'H'; svc #3
/// movz x0, #slot; movz x1, #0; movz x2, #'!'; svc #3
/// svc #1; b .
/// ```
pub const fn encode_console_hi_exit(slot: u16) -> [u8; 40] {
    let mut out = [0u8; 40];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, CONSOLE_TAG_BYTE));
    push_word(&mut out, &mut i, a64::movz_x(2, b'H' as u16));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::movz_x(0, slot));
    push_word(&mut out, &mut i, a64::movz_x(1, CONSOLE_TAG_BYTE));
    push_word(&mut out, &mut i, a64::movz_x(2, b'!' as u16));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
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

/// Spin on a stop word until non-zero, then `SYS_EXIT` (ADR-0064 oracle).
///
/// Unlike [`encode_spin_exit`] this makes **no** forward progress of its own
/// and no syscalls while spinning — the only way it loses the CPU is the
/// IRQ-side preemption it exists to prove. The kernel ends it by writing a
/// non-zero word at `base_va_hi16 << 16 | stop_off` (a text-page address the
/// kernel pokes through its identity alias).
///
/// ```text
/// movz x0, #base_va_hi16, lsl #16
/// 1: ldr  w1, [x0, #stop_off]
///    cbnz x1, 2f
///    b    1b                  // offset −2 words from the b (gas-checked)
/// 2: svc  #1
///    b .
/// ```
///
/// `stop_off` must be a multiple of 4 within the text page, past the program.
pub const fn encode_spin_flag_exit(base_va_hi16: u16, stop_off: u16) -> [u8; 24] {
    let mut out = [0u8; 24];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x_lsl16(0, base_va_hi16));
    push_word(&mut out, &mut i, a64::ldr_w_imm(1, 0, stop_off));
    push_word(&mut out, &mut i, a64::cbnz_x(1, 2));
    push_word(&mut out, &mut i, a64::b_rel(-2));
    push_word(&mut out, &mut i, a64::svc(1));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// PL011 RX poll once at `USER_PL011_VA` (`0x5000_0000`):
/// if RX not empty, `SYS_SEND` the byte to the console; always `SYS_EXIT`.
///
/// Empty FIFO → zero sends (honest “no data” path). A pending character → one
/// send. Does not invent receive data.
///
/// ```text
/// movz x0, #0x5000, lsl #16
/// ldr  w1, [x0, #0x18]
/// tbnz w1, #4, +5          // empty → skip to SYS_EXIT
/// ldrb w2, [x0]            // byte into w2 = Message.a
/// movz x0, #console_slot
/// movz x1, #CONSOLE_TAG
/// svc  #3
/// svc  #1
/// b .
/// ```
pub const fn encode_pl011_rx_poll_exit(console_slot: u16) -> [u8; 36] {
    let mut out = [0u8; 36];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x_lsl16(0, 0x5000));
    push_word(&mut out, &mut i, a64::ldr_w_imm(1, 0, 0x18));
    // RXFE set → empty → skip ldrb + movz×2 + send (4 insns) → land on exit.
    // Offset is word count from the tbnz itself: +5 lands on `svc #1`.
    push_word(&mut out, &mut i, a64::tbnz_w(1, 4, 5));
    push_word(&mut out, &mut i, a64::ldrb_w(2, 0));
    push_word(&mut out, &mut i, a64::movz_x(0, console_slot));
    push_word(&mut out, &mut i, a64::movz_x(1, CONSOLE_TAG_BYTE));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_EXIT));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// Read a device register from a granted window and report one bit of it
/// (ADR-0101).
///
/// The program a **composed** driver-agent runs: it is handed a device page by
/// the loader — index into the window vocabulary, resolved by the board — reads
/// a register from it, and sends `set_byte` or `clear_byte` to the console
/// endpoint depending on one bit.
///
/// The bit is the point. A program that is given a page and exits proves only
/// that the mapping did not fault; an encoder that dropped the load would pass
/// that test unchanged. Branching on a bit of what was read means the byte on
/// the wire is an answer only a real load can give.
///
/// ```text
/// movz x0, #va_hi16, lsl #16
/// ldr  w1, [x0, #reg_off]
/// tbnz w1, #bit, +3        // set → skip the clear-byte movz
/// movz x2, #clear_byte
/// b    +2
/// movz x2, #set_byte
/// movz x0, #console_slot
/// movz x1, #CONSOLE_TAG
/// svc  #3                  // SYS_SEND
/// svc  #1                  // SYS_EXIT
/// b .
/// ```
pub const fn encode_read_device_bit_console_exit(
    va_hi16: u16,
    reg_off: u16,
    bit: u8,
    console_slot: u16,
    set_byte: u8,
    clear_byte: u8,
) -> [u8; 44] {
    let mut out = [0u8; 44];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x_lsl16(0, va_hi16));
    push_word(&mut out, &mut i, a64::ldr_w_imm(1, 0, reg_off));
    // Bit set → jump over the clear-byte movz and its branch (2 insns).
    push_word(&mut out, &mut i, a64::tbnz_w(1, bit, 3));
    push_word(&mut out, &mut i, a64::movz_x(2, clear_byte as u16));
    push_word(&mut out, &mut i, a64::b_rel(2));
    push_word(&mut out, &mut i, a64::movz_x(2, set_byte as u16));
    push_word(&mut out, &mut i, a64::movz_x(0, console_slot));
    push_word(&mut out, &mut i, a64::movz_x(1, CONSOLE_TAG_BYTE));
    push_word(&mut out, &mut i, a64::svc(syscall::SYS_SEND));
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
            "encode_console_once_exit(1, b'X')",
            &encode_console_once_exit(1, b'X'),
            "movz x0, #1\nmovz x1, #0\nmovz x2, #88\nsvc #3\nsvc #1\nb .\n",
        );

        assert_program(
            "encode_console_hi_exit(1)",
            &encode_console_hi_exit(1),
            "movz x0, #1\nmovz x1, #0\nmovz x2, #72\nsvc #3\n\
             movz x0, #1\nmovz x1, #0\nmovz x2, #33\nsvc #3\nsvc #1\nb .\n",
        );

        assert_program(
            "encode_net_tx_rx_exit(0x5300_0000, 0, 1, 2, 3)",
            &encode_net_tx_rx_exit(0x5300_0000, 0, 1, 2, 3),
            "movz x0, #0\nmovk x0, #21248, lsl #16\n\
             movk x0, #0, lsl #32\nmovk x0, #0, lsl #48\n\
             movz x1, #30600\nmovk x1, #21862, lsl #16\n\
             movk x1, #13124, lsl #32\nmovk x1, #4386, lsl #48\n\
             str x1, [x0]\n\
             movz x1, #65280\nmovk x1, #56814, lsl #16\n\
             movk x1, #48076, lsl #32\nmovk x1, #39338, lsl #48\n\
             str x1, [x0, #8]\n\
             movz x0, #0\nmovz x1, #4353\nmovz x2, #1\n\
             movk x2, #0, lsl #16\nmovk x2, #4096, lsl #32\nmovk x2, #0, lsl #48\nsvc #3\n\
             movz x0, #1\nsvc #4\nmovz x0, #2\nsvc #4\n\
             movz x0, #3\nmovz x1, #4354\nsvc #3\nsvc #1\nb .\n",
        );

        // RECV leaves a in x2; SEND wants tag in x1 and a in x2.
        assert_program(
            "encode_recv_console_exit(0, 1)",
            &encode_recv_console_exit(0, 1),
            "movz x0, #0\nsvc #4\nmovz x1, #0\nmovz x0, #1\nsvc #3\nsvc #1\nb .\n",
        );

        // `svc #5`, not `#4`: the two recvs are different calls, and an agent
        // that must not wait says so at the call site.
        assert_program(
            "encode_try_recv_exit(0)",
            &encode_try_recv_exit(0),
            "movz x0, #0\nsvc #5\nsvc #1\nb .\n",
        );

        assert_program(
            "encode_wait_irq_exit(0)",
            &encode_wait_irq_exit(0),
            "movz x0, #0\nsvc #6\nsvc #1\nb .\n",
        );

        // 0x6261 = b"ab" little-endian, the shape the doc comment promises.
        assert_program(
            "encode_resolve_exit(2, 2, 0x6261)",
            &encode_resolve_exit(2, 2, 0x6261),
            "movz x0, #2\nmovz x1, #2\nmovz x2, #25185\nsvc #7\nsvc #1\nb .\n",
        );

        // ADR-0102: seven-byte `console` plus a send. The immediates are the
        // little-endian halves, written as llvm-mc prints them (decimal).
        assert_program(
            "encode_resolve_send_exit(0, b'N')",
            &encode_resolve_send_exit(0, b'N'),
            "movz x0, #0\nmovz x1, #7\nmovz x2, #28515\n\
             movk x2, #29550, lsl #16\nmovk x2, #27759, lsl #32\nmovk x2, #101, lsl #48\n\
             svc #7\nmovz x0, #0\nmovz x1, #0\nmovz x2, #78\nsvc #3\nsvc #1\nb .\n",
        );

        assert_program(
            "encode_transfer_exit(0, 1, 1)",
            &encode_transfer_exit(0, 1, 1),
            "movz x0, #0\nmovz x1, #1\nmovz x2, #1\nsvc #8\nsvc #1\nb .\n",
        );

        // `movz x2, #2` is the peer dest hard-wired by the encoder (ADR-0054);
        // x3 carries the task-cap slot. These two were the only encoders that
        // shipped without a row here, and cargo-mutants proved it: both
        // survived replacement by a constant array until this was written.
        assert_program(
            "encode_transfer_peer_exit(0, 0, 1)",
            &encode_transfer_peer_exit(0, 0, 1),
            "movz x0, #0\nmovz x1, #0\nmovz x2, #2\nmovz x3, #1\nsvc #8\nsvc #1\nb .\n",
        );

        assert_program(
            "encode_recv_timeout_exit(0, 3)",
            &encode_recv_timeout_exit(0, 3),
            "movz x0, #0\nmovz x1, #3\nsvc #9\nsvc #1\nb .\n",
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

        // ADR-0064: spin on the stop word until non-zero, then exit.
        assert_program(
            "encode_spin_flag_exit(0x4000, 0x800)",
            &encode_spin_flag_exit(0x4000, 0x800),
            "movz x0, #0x4000, lsl #16\n\
             ldr w1, [x0, #0x800]\n\
             cbnz x1, #8\n\
             b #-8\n\
             svc #1\n\
             b .\n",
        );
    }

    #[test]
    fn network_and_blob_program_encoders_preserve_their_parameters() {
        let network = encode_net_tx_exit(0x5300_0000, 4, 5);
        assert_eq!(
            network[..4],
            a64::movz_x(0, 0).to_le_bytes(),
            "the network encoder must start by loading the pool address"
        );
        assert!(network.iter().any(|&byte| byte > 1));

        let blob = encode_blob_round_trip_exit(6, 7, 8);
        assert_eq!(
            blob[..4],
            a64::movz_x(0, 6).to_le_bytes(),
            "the blob encoder must load the request capability slot"
        );
        assert!(blob.iter().any(|&byte| byte > 1));
    }

    /// The one that earns the suite.
    ///
    /// `tbnz` skips the load, slot, tag and `SYS_SEND` when the RX FIFO is
    /// empty — four instructions after the branch, so the target is five words
    /// past the branch (`#20` bytes). A wrong offset prints
    /// `rx poll unexpected sends=…` on a board, which reads like a kernel bug.
    #[cfg_attr(
        miri,
        ignore = "shells out to llvm-mc, which Miri cannot run; these encoders are pure and a64's unit tests cover the instruction words"
    )]
    #[test]
    fn the_rx_poll_branch_skips_exactly_the_console_send_path() {
        assert_program(
            "encode_pl011_rx_poll_exit(1)",
            &encode_pl011_rx_poll_exit(1),
            "movz x0, #0x5000, lsl #16\n\
             ldr w1, [x0, #24]\n\
             tbnz w1, #4, #20\n\
             ldrb w2, [x0]\n\
             movz x0, #1\n\
             movz x1, #0\n\
             svc #3\n\
             svc #1\n\
             b .\n",
        );
    }

    #[cfg_attr(
        miri,
        ignore = "shells out to llvm-mc, which Miri cannot run; these encoders are pure and a64's unit tests cover the instruction words"
    )]
    #[test]
    fn the_device_read_sends_a_byte_that_depends_on_what_it_read() {
        // ADR-0101. The branch is the whole assertion: an encoder that dropped
        // the load, or that always sent the same byte, would still map, still
        // send, and still exit — and would be indistinguishable from this one
        // on any test that only checked the agent ran.
        assert_program(
            "encode_read_device_bit_console_exit(0x5100, 0, 0, 1, b'R', b'r')",
            &encode_read_device_bit_console_exit(0x5100, 0, 0, 1, b'R', b'r'),
            "movz x0, #0x5100, lsl #16\n\
             ldr w1, [x0, #0]\n\
             tbnz w1, #0, #12\n\
             movz x2, #114\n\
             b #8\n\
             movz x2, #82\n\
             movz x0, #1\n\
             movz x1, #0\n\
             svc #3\n\
             svc #1\n\
             b .\n",
        );
    }
}
