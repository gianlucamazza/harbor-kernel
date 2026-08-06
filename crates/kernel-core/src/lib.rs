//! Pure kernel logic, testable on the host.
//!
//! Everything in this crate is a total function over integers: register
//! encodings, divisor math, allocator bookkeeping. No MMIO, no assembly, no
//! `unsafe` — with one deliberate exception, the SPSC ring, whose `UnsafeCell`
//! buffer and `Sync` assertion are what let the IRQ producer and the main-loop
//! consumer share it without aliasing `&mut`. It carries a scoped `#[allow]`
//! and is covered by Miri. The kernel crate owns the hardware; this crate owns
//! the arithmetic
//! that used to be buried inside it and therefore untestable.
//!
//! `no_std` for the kernel build, `std` under `cargo test` so the default test
//! harness links.

#![cfg_attr(not(test), no_std)]

pub mod a64;
pub mod bump;
pub mod cap;
pub mod delay;
pub mod display;
pub mod font8x8;
pub mod frame;
pub mod gic;
pub mod heap;
pub mod ipc;
pub mod layout;
pub mod paging;
pub mod poll;
pub mod ring;
pub mod rng;
pub mod runqueue;
pub mod spi;
pub mod syscall;
pub mod tasks;
pub mod textgrid;
pub mod timer;
pub mod uart;
pub mod wake;
