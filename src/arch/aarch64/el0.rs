//! EL0 entry / SVC resume (ADR-0014, ADR-0017).
//!
//! ## Protocol
//!
//! 1. [`enter`] publishes the kernel root into the session, switches to the
//!    user `TTBR0`, programs `ELR`/`SPSR`/`SP_EL0`, `ERET` to EL0.
//! 2. Lower-EL sync: `kernel_entry` → `switch_ttbr0(kernel)` → classify. On
//!    **SVC**, user GPRs/`ELR`/`SPSR`/`SP_EL0` are saved. AArch64 already sets
//!    `ELR` to the instruction *after* the SVC (preferred return) — no software
//!    `+4`. [`resume`] continues that context.
//! 3. [`resume`] re-installs the user root and `ERET`s with the saved state.
//! 4. [`end_session`] clears the session's kernel root.
//!
//! [`run`] is one-shot: `enter` + [`end_session`]. Default entry masks IRQs in
//! EL0 (`SPSR` DAIF.I); sessions that need timer/UART while user runs call
//! [`set_entry_irqs_unmasked`] before [`enter`].
//!
//! ## Whose session
//!
//! Every function above takes a `*mut El0Session` and refuses to act unless it
//! is the one currently published. The state itself lives in the scheduler's
//! TCB (ADR-0017 §1), so it is **per task**: two agents can be live at EL0, and
//! the invariant "no second session while one is live" is structural rather
//! than a rule someone has to follow.
//!
//! What this module owns is the [`El0Session`] layout, the single published
//! pointer, and the refusal. What it does not own is *which* session belongs to
//! the running task — that is `sched::current_el0_session`, and comparing the
//! two answers is the whole point: a switch that stops publishing panics on the
//! next EL0 entry instead of handing one agent another agent's saved registers.
//!
//! ## Why one `static mut` remains
//!
//! [`crate::sync`] argues that a `static mut` in edition 2024 has no way to
//! state who may touch it. `CURRENT_EL0` is one anyway, and the reason is not
//! laziness: the assembly here and in `vectors.s` reaches it **by symbol name**
//! (`adrp`/`add`), and a `SyncCell` has no linker-visible name to load. That
//! argument is ADR-0016's and it still holds; what ADR-0017 changed is that it
//! now buys *one* symbol holding a pointer instead of nine holding state.
//!
//! The field offsets the assembly applies to that pointer are derived from the
//! struct rather than written out — see the `.equ` block below.

use crate::arch::exception::{TrapFrame, read_esr_el1, read_far_el1};
use crate::arch::mmu;

/// `SPSR_EL1` for EL0t with DAIF all masked (default session contract).
const SPSR_EL0_IRQS_MASKED: u64 = 0x3c0;
/// `SPSR_EL1` for EL0t with DAIF.I clear — IRQs may take lower-EL IRQ vectors.
const SPSR_EL0_IRQS_OPEN: u64 = 0x340;

/// Result of one EL0 stretch (enter or resume until the next lower-EL sync).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum El0Outcome {
    /// `SVC` from AArch64 EL0. Session may [`resume`].
    Svc { imm: u16 },
    /// Data abort from lower EL. Session ends.
    DataAbort { esr: u64, far: u64 },
    /// IRQ while EL0 ran with IRQs unmasked. Session may [`resume`] after handle.
    Irq,
    /// Other sync from lower EL. Session ends.
    OtherSync { esr: u64, far: u64 },
}

/// Next [`enter`] uses EL0 `SPSR` with IRQs masked (default after boot / end).
///
/// Per session, and read by `el0_run` only, at the start of one — never by
/// `el0_resume`, so this cannot alter a session already under way. Safe rather
/// than `unsafe fn` for that reason: the worst a mistimed call does is choose
/// the mask for the *next* entry of the task it names.
#[inline]
pub fn set_entry_irqs_masked(session: *mut El0Session) {
    require_published(session);
    current().entry_spsr = SPSR_EL0_IRQS_MASKED;
}

/// Next [`enter`] uses EL0 `SPSR` with DAIF.I clear (IRQ → [`El0Outcome::Irq`]).
#[inline]
pub fn set_entry_irqs_unmasked(session: *mut El0Session) {
    require_published(session);
    current().entry_spsr = SPSR_EL0_IRQS_OPEN;
}

/// One-shot: enter EL0 until the first sync, then end the session.
///
/// # Safety
/// Same as [`enter`].
pub unsafe fn run(
    session: *mut El0Session,
    ttbr0_phys: usize,
    entry: u64,
    user_sp: u64,
) -> El0Outcome {
    // SAFETY: `enter`'s obligations are the caller's, forwarded by this
    // function's own `# Safety`. The `end_session` that follows is sound
    // because `enter` has returned: whatever the outcome, this session took its
    // one event and is not going to be resumed — that is what "one-shot" means.
    let outcome = unsafe { enter(session, ttbr0_phys, entry, user_sp) };
    // SAFETY: `enter` has returned, so this session took its one event and will
    // not be resumed — which is what makes ending it here sound.
    unsafe { end_session(session) };
    outcome
}

/// Enter EL0 until the first lower-EL sync.
///
/// # Safety
/// Prepared user root; IRQs masked; sole session.
pub unsafe fn enter(
    session: *mut El0Session,
    ttbr0_phys: usize,
    entry: u64,
    user_sp: u64,
) -> El0Outcome {
    require_published(session);
    let Some(kernel_ttbr) = mmu::kernel_root_phys() else {
        panic!("el0::enter: kernel map not activated");
    };
    // SAFETY: single core with IRQs masked (the caller's obligation), so no
    // other context can be between these writes and the `el0_run` that reads
    // them. `can_resume` is cleared *before* the entry so that a fault on the
    // very first instruction cannot be mistaken for a resumable event left over
    // from a previous session.
    unsafe {
        let live = current();
        live.user_ttbr = ttbr0_phys as u64;
        live.can_resume = 0;
        unpack(el0_run(
            ttbr0_phys as u64,
            entry,
            user_sp,
            kernel_ttbr as u64,
        ))
    }
}

/// Continue after [`El0Outcome::Svc`] or [`El0Outcome::Irq`].
///
/// After SVC, `ELR` already points past the insn. After IRQ, `ELR` is the
/// interrupted insn — architectural re-execute on resume (no software skip).
///
/// # Safety
/// Prior event was resumable; IRQs masked at EL1; session not ended.
pub unsafe fn resume(session: *mut El0Session) -> El0Outcome {
    require_published(session);
    // Both are checked rather than assumed: `el0_resume` would `eret` into a
    // context that was never saved, and `vectors.s` would take the next
    // lower-EL exception with no kernel root to reinstall. Panicking here is
    // the difference between a message and an unrecoverable fetch.
    if current().can_resume == 0 {
        panic!("el0::resume: no resumable session");
    }
    if current().kernel_ttbr0 == 0 {
        panic!("el0::resume: session kernel TTBR cleared");
    }
    // SAFETY: the two checks above are exactly `el0_resume`'s preconditions.
    unsafe { unpack(el0_resume()) }
}

/// Clear session symbols (call after the last event if not already cleared).
///
/// # Safety
/// No EL0 session may be live. This clears `el0_kernel_ttbr0`, which
/// `vectors.s` requires to be non-zero on every lower-EL exception: calling it
/// while EL0 can still be entered turns the next fault from that agent into
/// [`el0_missing_kernel_ttbr`] and a panic. It was a safe `fn`, which made
/// breaking the vector path's precondition ordinary safe Rust.
#[inline]
pub unsafe fn end_session(session: *mut El0Session) {
    require_published(session);
    // The caller has established that no session is live, so nothing is going
    // to read these again before the next `enter` writes them.
    let live = current();
    live.kernel_ttbr0 = 0;
    live.can_resume = 0;
    live.user_ttbr = 0;
    live.entry_spsr = SPSR_EL0_IRQS_MASKED;
}

fn unpack(packed: u64) -> El0Outcome {
    let kind = (packed & 0xFFFF_FFFF) as u32;
    match kind {
        1 => El0Outcome::Svc {
            imm: (packed >> 32) as u16,
        },
        2 => {
            let (esr, far) = fault_syndrome();
            El0Outcome::DataAbort { esr, far }
        }
        4 => El0Outcome::Irq,
        _ => {
            let (esr, far) = fault_syndrome();
            El0Outcome::OtherSync { esr, far }
        }
    }
}

/// The syndrome saved by the vector path for the event being decoded.
fn fault_syndrome() -> (u64, u64) {
    // `exception_sync_el0` wrote both on the way here, from the very event whose
    // packed kind is being decoded — the vector path runs to completion before
    // `el0_run_finish` returns that value, and nothing else writes them until
    // the next lower-EL exception on this task.
    let live = current();
    (live.esr, live.far)
}

/// Registers the kernel may write when answering a syscall.
///
/// `x0..x3` are the reply window of the EL0 ABI
/// ([`kernel_core::syscall`]). Every other register is the agent's own
/// context: writing one would not be answering the agent, it would be
/// corrupting it — and the agent has no way to tell the difference.
pub const ABI_REPLY_REGS: usize = 4;

/// Low 64 bits of user `x0` at the last SVC/IRQ (for `SYS_PUTC`, etc.).
#[inline]
pub fn saved_x0(session: *mut El0Session) -> u64 {
    saved_gpr(session, 0)
}

/// User `x{n}` as saved at the last SVC/IRQ.
///
/// Safe rather than `unsafe fn` because a mistimed call returns a stale value
/// rather than breaking an invariant — the caller is expected to have just
/// received `Svc` or `Irq`, and nothing else consults this.
///
/// # Panics
/// If `n` is not a general-purpose register index (0..=30). An argument index
/// is a constant at every call site, so this is a build-time mistake that
/// deserves to be loud rather than a silent read of the wrong word.
#[inline]
pub fn saved_gpr(session: *mut El0Session, n: usize) -> u64 {
    require_published(session);
    let live = current();
    match live.saved.gpr.get(n) {
        Some(&value) => value,
        None => panic!("el0::saved_gpr: x{n} is not a general-purpose register"),
    }
}

/// Write user `x{n}`, which the agent will see when the session resumes.
///
/// Restricted to [`ABI_REPLY_REGS`] deliberately. This is the one place the
/// kernel reaches into an agent's register file, and the bound is what keeps
/// "answering a syscall" from being able to mean anything else.
///
/// # Panics
/// If `n` is outside the reply window.
#[inline]
pub fn set_saved_gpr(session: *mut El0Session, n: usize, value: u64) {
    require_published(session);
    if n >= ABI_REPLY_REGS {
        panic!("el0::set_saved_gpr: x{n} is outside the syscall reply window");
    }
    current().saved.gpr[n] = value;
}

/// Saved user context for SVC resume (`TrapFrame` field order without pad).
#[repr(C)]
pub struct SavedUser {
    gpr: [u64; 31],
    elr: u64,
    spsr: u64,
}

/// One task's EL0 session state — the nine globals ADR-0016 recorded, in one
/// object the scheduler can put in a TCB (ADR-0017 §1).
///
/// `#[repr(C)]` because the assembly in this module reaches every field by
/// offset. Those offsets are *derived* from this declaration and assembled into
/// `.equ` symbols below, so reordering a field moves the assembly with it; the
/// `offset_of` assertions next to them are a tripwire on an unintended reorder,
/// not the mechanism that keeps the two in agreement.
#[repr(C)]
pub struct El0Session {
    saved: SavedUser,
    /// User `SP_EL0` at the SVC (kernel finish overwrites `SP_EL0` with its frame).
    saved_sp_el0: u64,
    user_ttbr: u64,
    /// Kernel root to reinstall on a lower-EL exception. Non-zero exactly while
    /// a session is live — `vectors.s` requires it and panics without it.
    kernel_ttbr0: u64,
    run_sp: u64,
    /// `SPSR_EL1` installed on the next [`enter`] (resume restores saved SPSR).
    entry_spsr: u64,
    esr: u64,
    far: u64,
    kind: u64,
    can_resume: u64,
}

impl El0Session {
    /// A task that has never entered EL0. All-zero but for the entry mask,
    /// which is also "no live session": `kernel_ttbr0` and `can_resume` are 0.
    pub const fn new() -> Self {
        Self {
            saved: SavedUser {
                gpr: [0; 31],
                elr: 0,
                spsr: 0,
            },
            saved_sp_el0: 0,
            user_ttbr: 0,
            kernel_ttbr0: 0,
            run_sp: 0,
            entry_spsr: SPSR_EL0_IRQS_MASKED,
            esr: 0,
            far: 0,
            kind: 0,
            can_resume: 0,
        }
    }
}

/// Byte offset of `kernel_ttbr0`, for the vector path's own `.equ`.
///
/// `vectors.s` is assembled from `arch::exception`, which emits its symbols
/// there. Exporting the number rather than the `.equ` keeps one definition
/// without depending on the order two `global_asm!` blocks are concatenated in.
pub const KERNEL_TTBR0_OFFSET: usize = core::mem::offset_of!(El0Session, kernel_ttbr0);

/// The session the assembly reaches, published by the scheduler on every switch.
///
/// This is the one `static mut` that remains of the nine, and it remains for the
/// reason ADR-0016 gave: `adrp`/`add` needs a linker-visible name and a
/// `SyncCell` has none. What changed is that it holds a *pointer*, so the state
/// it names is per-task and lives in the TCB.
///
/// Null means no task's session is published — every path that dereferences it
/// says so rather than reading address zero.
#[unsafe(no_mangle)]
static mut CURRENT_EL0: *mut El0Session = core::ptr::null_mut();

/// Publish `session` as the one the assembly will use.
///
/// # Safety
/// `session` must be null or point to an `El0Session` that outlives every EL0
/// entry made while it is published, and the caller must be the scheduler
/// running with IRQs masked: publishing while a session is live hands the
/// assembly a different task's saved registers.
#[inline]
pub unsafe fn publish(session: *mut El0Session) {
    // SAFETY: the caller's obligation, forwarded.
    unsafe { CURRENT_EL0 = session };
}

/// The currently published session pointer (null if none).
#[inline]
pub fn published() -> *mut El0Session {
    // SAFETY: a plain pointer read on one core.
    unsafe { CURRENT_EL0 }
}

/// The published session, or panic naming the reason.
///
/// The panic is the Rust-side twin of [`el0_missing_kernel_ttbr`]: same class of
/// error — a lower-EL event with no session behind it — and it deserves the same
/// message rather than a fault at address zero.
#[inline]
fn current() -> &'static mut El0Session {
    // SAFETY: single core with IRQs masked on every path that reaches here (the
    // vector handlers and the enter/resume calls, all inside `without_irqs`), so
    // this `&mut` cannot overlap another. Null is checked rather than assumed.
    unsafe {
        if CURRENT_EL0.is_null() {
            panic!("el0: no published session (scheduler did not publish on switch)");
        }
        &mut *CURRENT_EL0
    }
}

/// Refuse to act on a session the assembly would not see.
///
/// This is the check ADR-0017 required in the same commit as the pointer: the
/// scheduler says which session belongs to the running task, the assembly uses
/// whatever was last published, and nothing else compares them. A switch that
/// forgets to publish is silent until an agent reads another agent's registers.
#[inline]
fn require_published(session: *mut El0Session) {
    if published() != session {
        panic!("el0: published session is not the current task's (stale after switch)");
    }
}

unsafe extern "C" {
    fn el0_run(user_ttbr: u64, entry: u64, user_sp: u64, kernel_ttbr: u64) -> u64;
    fn el0_resume() -> u64;
}

#[unsafe(no_mangle)]
pub extern "C" fn exception_sync_el0(frame: &mut TrapFrame) {
    let esr = read_esr_el1();
    let far = read_far_el1();
    let ec = (esr >> 26) & 0x3F;
    let kind: u64 = match ec {
        0x15 => 1,
        0x24 => 2,
        _ => 3,
    };
    // Called from `vectors.s` with the CPU already through `kernel_entry` and
    // the kernel root reinstalled, so this is the only context running. It
    // writes the session state that `el0_run_finish` and `unpack` read
    // immediately afterwards; `frame` is the trap frame the vector just built
    // on SP_EL1, valid for this call. The session is the published one — the
    // same pointer the vector path just dereferenced to find that kernel root.
    let live = current();
    live.esr = esr;
    live.far = far;
    live.kind = kind;
    if kind == 1 {
        // AArch64 SVC: ELR is already the insn *after* the SVC — do not +4.
        live.saved.gpr = frame.gpr;
        live.saved.elr = frame.elr;
        live.saved.spsr = frame.spsr;
        // finish repurposes SP_EL0 as the kernel frame; keep the user SP.
        live.saved_sp_el0 = read_sp_el0();
        live.can_resume = 1;
    } else {
        live.can_resume = 0;
    }
}

/// IRQ from EL0: save user context for [`resume`] (ELR is the interrupted insn).
///
/// Vectors restore kernel `TTBR0` first. Caller should run the IRQ subsystem
/// then [`resume`]. Sessions that keep IRQs masked never reach here.
#[unsafe(no_mangle)]
pub extern "C" fn exception_irq_el0(frame: &mut TrapFrame) {
    // As `exception_sync_el0` — sole context, the published session, and a trap
    // frame the vector path just built. `ELR` is deliberately left at the
    // interrupted instruction: the architecture re-executes it on resume, so a
    // software skip here would silently drop one user instruction per IRQ.
    let live = current();
    live.kind = 4;
    live.esr = 0;
    live.far = 0;
    live.saved.gpr = frame.gpr;
    live.saved.elr = frame.elr;
    live.saved.spsr = frame.spsr;
    live.saved_sp_el0 = read_sp_el0();
    live.can_resume = 1;
}

#[unsafe(no_mangle)]
pub extern "C" fn el0_missing_kernel_ttbr() -> ! {
    panic!("lower-EL exception without published kernel TTBR0 (no live el0 session)");
}

// Field offsets the assembly below uses, derived from the struct rather than
// written twice. `.equ` symbols are absolute constants the assembler resolves,
// so a reordered field moves every load with it.
//
// The assertions beside them are a tripwire, not the mechanism: they say what
// the layout is *expected* to be, so an unintended reorder is a compile error
// naming the field instead of a silent change of meaning. `Context` carries the
// same pattern for the same reason (see the M3 entry in `verification.md`).
const _: () = assert!(core::mem::offset_of!(El0Session, saved) == 0x000);
const _: () = assert!(core::mem::offset_of!(El0Session, saved.elr) == 0x0F8);
const _: () = assert!(core::mem::offset_of!(El0Session, saved.spsr) == 0x100);
const _: () = assert!(core::mem::offset_of!(El0Session, saved_sp_el0) == 0x108);
const _: () = assert!(core::mem::offset_of!(El0Session, user_ttbr) == 0x110);
const _: () = assert!(core::mem::offset_of!(El0Session, kernel_ttbr0) == 0x118);
const _: () = assert!(core::mem::offset_of!(El0Session, run_sp) == 0x120);
const _: () = assert!(core::mem::offset_of!(El0Session, entry_spsr) == 0x128);
const _: () = assert!(core::mem::offset_of!(El0Session, esr) == 0x130);
const _: () = assert!(core::mem::offset_of!(El0Session, kind) == 0x140);
const _: () = assert!(core::mem::offset_of!(El0Session, can_resume) == 0x148);
const _: () = assert!(core::mem::size_of::<El0Session>() == 0x150);

core::arch::global_asm!(
    ".equ EL0S_GPR, {gpr}",
    ".equ EL0S_ELR, {elr}",
    ".equ EL0S_SPSR, {spsr}",
    ".equ EL0S_SP_EL0, {sp_el0}",
    ".equ EL0S_USER_TTBR, {user_ttbr}",
    ".equ EL0S_KERNEL_TTBR, {kernel_ttbr}",
    ".equ EL0S_RUN_SP, {run_sp}",
    ".equ EL0S_ENTRY_SPSR, {entry_spsr}",
    ".equ EL0S_ESR, {esr}",
    ".equ EL0S_KIND, {kind}",
    ".equ EL0S_CAN_RESUME, {can_resume}",
    gpr = const core::mem::offset_of!(El0Session, saved.gpr),
    elr = const core::mem::offset_of!(El0Session, saved.elr),
    spsr = const core::mem::offset_of!(El0Session, saved.spsr),
    sp_el0 = const core::mem::offset_of!(El0Session, saved_sp_el0),
    user_ttbr = const core::mem::offset_of!(El0Session, user_ttbr),
    kernel_ttbr = const core::mem::offset_of!(El0Session, kernel_ttbr0),
    run_sp = const core::mem::offset_of!(El0Session, run_sp),
    entry_spsr = const core::mem::offset_of!(El0Session, entry_spsr),
    esr = const core::mem::offset_of!(El0Session, esr),
    kind = const core::mem::offset_of!(El0Session, kind),
    can_resume = const core::mem::offset_of!(El0Session, can_resume),
);

core::arch::global_asm!(
    r#"
    .global el0_run
    .global el0_resume
    .global el0_run_finish
    .text

    // Load the published session into \reg, or panic. Every path here runs
    // because a lower-EL event happened or is about to: no session published
    // means no task owns the event, which is the same error the vector path
    // names when the kernel root is clear.
    .macro load_session reg
        adrp    \reg, CURRENT_EL0
        add     \reg, \reg, :lo12:CURRENT_EL0
        ldr     \reg, [\reg]
        cbz     \reg, el0_missing_kernel_ttbr
    .endm

    // x0=user_ttbr, x1=entry, x2=user_sp, x3=kernel_ttbr
    el0_run:
        stp x29, x30, [sp, #-96]!
        mov x29, sp
        stp x19, x20, [sp, #16]
        stp x21, x22, [sp, #32]
        stp x23, x24, [sp, #48]
        stp x25, x26, [sp, #64]
        stp x27, x28, [sp, #80]

        load_session x9
        str x3, [x9, #EL0S_KERNEL_TTBR]

        mov x10, sp
        str x10, [x9, #EL0S_RUN_SP]

        msr spsel, #1
        mov x19, x1
        mov x20, x2
        // The session survives the call in a callee-saved register: this frame
        // is unwound by el0_run_finish, which restores x21 with the rest.
        mov x21, x9
        bl switch_ttbr0
        msr sp_el0, x20
        msr elr_el1, x19
        ldr x4, [x21, #EL0S_ENTRY_SPSR]
        msr spsr_el1, x4

        mov x0, xzr
        mov x1, xzr
        mov x2, xzr
        mov x3, xzr
        mov x4, xzr
        eret

    // Resume after SVC/IRQ: user TTBR + the session's saved context → ERET.
    el0_resume:
        stp x29, x30, [sp, #-96]!
        mov x29, sp
        stp x19, x20, [sp, #16]
        stp x21, x22, [sp, #32]
        stp x23, x24, [sp, #48]
        stp x25, x26, [sp, #64]
        stp x27, x28, [sp, #80]

        load_session x19
        mov x9, sp
        str x9, [x19, #EL0S_RUN_SP]

        msr spsel, #1
        ldr x0, [x19, #EL0S_USER_TTBR]
        bl switch_ttbr0

        // Restore user SP_EL0 before ERET (finish clobbered it with kernel SP).
        ldr x0, [x19, #EL0S_SP_EL0]
        msr sp_el0, x0

        ldr x10, [x19, #EL0S_ELR]
        ldr x11, [x19, #EL0S_SPSR]
        msr elr_el1, x10
        msr spsr_el1, x11

        // Base for the saved GPRs in x29 (restored last). x19 is not read again
        // past the ldp that overwrites it.
        add x29, x19, #EL0S_GPR
        ldp x0,  x1,  [x29, #0x00]
        ldp x2,  x3,  [x29, #0x10]
        ldp x4,  x5,  [x29, #0x20]
        ldp x6,  x7,  [x29, #0x30]
        ldp x8,  x9,  [x29, #0x40]
        ldp x10, x11, [x29, #0x50]
        ldp x12, x13, [x29, #0x60]
        ldp x14, x15, [x29, #0x70]
        ldp x16, x17, [x29, #0x80]
        ldp x18, x19, [x29, #0x90]
        ldp x20, x21, [x29, #0xA0]
        ldp x22, x23, [x29, #0xB0]
        ldp x24, x25, [x29, #0xC0]
        ldp x26, x27, [x29, #0xD0]
        ldr x28,      [x29, #0xE0]
        ldr x30,      [x29, #0xF0]
        ldr x29,      [x29, #0xE8]
        eret

    // Vectors: pack outcome. Clear kernel TTBR only when not resumable (fault).
    el0_run_finish:
        load_session x9
        ldr x10, [x9, #EL0S_CAN_RESUME]
        cbnz x10, 1f
        str xzr, [x9, #EL0S_KERNEL_TTBR]
1:
        ldr x11, [x9, #EL0S_RUN_SP]
        msr spsel, #0
        mov sp, x11

        ldr x0, [x9, #EL0S_KIND]
        ldr x1, [x9, #EL0S_ESR]
        and x2, x1, #0xFFFF
        orr x0, x0, x2, lsl #32

        ldp x19, x20, [sp, #16]
        ldp x21, x22, [sp, #32]
        ldp x23, x24, [sp, #48]
        ldp x25, x26, [sp, #64]
        ldp x27, x28, [sp, #80]
        ldp x29, x30, [sp], #96
        ret
    "#
);

#[inline]
fn read_sp_el0() -> u64 {
    let v: u64;
    // SAFETY: `SP_EL0` is readable at EL1 as an ordinary system register. The
    // kernel runs *on* SP_EL0 (boot.s clears SPSel), so between an EL0 entry and
    // the vector's `msr spsel, #1` this reads the user stack pointer, which is
    // the only window the callers below use.
    unsafe {
        core::arch::asm!("mrs {}, sp_el0", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    v
}
