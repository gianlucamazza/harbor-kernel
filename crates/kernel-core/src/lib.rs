//! Pure kernel logic, testable on the host.
//!
//! Everything in this crate is a total function over integers: register
//! encodings, divisor math, allocator bookkeeping. No MMIO, no assembly, no
//! `unsafe` — with two deliberate exceptions, the SPSC ring and the wake
//! queue, whose `UnsafeCell` buffers and `Sync` assertions are what let the
//! IRQ producer and the main-loop consumer share them without aliasing
//! `&mut`. Each carries a module-scoped `#[allow]` and is covered by Miri.
//! The kernel crate owns the hardware; this crate owns the arithmetic
//! that used to be buried inside it and therefore untestable.
//!
//! `no_std` for the kernel build, `std` under `cargo test` so the default test
//! harness links.
//!
//! # How this crate is checked
//!
//! Three ways, in increasing order of what they can say:
//!
//! - **Unit tests**, beside each module, on cases someone chose.
//! - **`tests/public_api.rs`**, from outside, on the surface `src/` depends on:
//!   a type that stops being exported breaks here rather than in the kernel.
//! - **`tests/model_sched.rs` and `tests/model_ipc.rs`**, which choose nothing.
//!   They replay every sequence of operations up to a bound — millions — and
//!   check the scheduler's invariants and the authority table's agreement with
//!   a reference implementation. The bound is stated in
//!   `docs/verification.md`; a bounded result is not a proof and that document
//!   says which is which.
//!
//! Only the second and third can catch a regression nobody predicted, and only
//! the third can catch a *specification* that says less than it claims — which
//! is what it found first.

#![cfg_attr(not(test), no_std)]

pub mod a64;
pub mod agentstore;
pub mod asid;
pub mod blob;
pub mod budget;
pub mod bump;
pub mod cap;
pub mod capslots;
pub mod cpuid;
pub mod delay;
pub mod density;
pub mod display;
pub mod durable;
pub mod durable_media;
pub mod fault;
pub mod fdt;
pub mod font8x8;
pub mod frame;
pub mod gic;
pub mod heap;
pub mod held;
pub mod hwdesc;
pub mod ipc;
pub mod irqcap;
pub mod irqtable;
pub mod irqwait;
pub mod layout;
pub mod lifecycle;
pub mod loaderplan;
pub mod manifest;
pub mod mbr;
pub mod naming;
pub mod net;
pub mod paging;
pub mod parktime;
pub mod poll;
pub mod preempt;
pub mod prog;
pub mod reply;
pub mod reset;
pub mod ring;
pub mod rng;
pub mod runqueue;
pub mod rxline;
pub mod sdcard;
pub mod sdhci;
pub mod spi;
pub mod storage;
pub mod syscall;
pub mod taskcap;
pub mod tasks;
pub mod textgrid;
pub mod timer;
pub mod uart;
pub mod virtio;
pub mod wake;
