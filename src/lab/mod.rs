//! Lab maturity path — thin bring-up without the product policy tree.
//!
//! Scale axis: **maturity** ([project-topology](../../docs/design/project-topology.md)).
//! Product modules stay `cfg`’d out on lab targets (progressive-isa P.3).

#[cfg(target_arch = "x86_64")]
mod panic;

#[cfg(target_arch = "x86_64")]
mod x86;

#[cfg(target_arch = "x86_64")]
pub use x86::run;
