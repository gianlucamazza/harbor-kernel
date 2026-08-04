//! Tell cargo about the inputs it cannot see.
//!
//! The linker script is passed through `rustflags` in `.cargo/config.toml`, so
//! cargo has no idea it exists: editing `link.ld` leaves the previous binary in
//! place and the next build silently links the old layout. On a kernel that is
//! not a stale artefact, it is a stale *memory map* — regions, permissions,
//! stack and guard page all come from that file, and the mismatch surfaces as
//! a fault somewhere unrelated.
//!
//! Found by editing `link.ld`, restoring it, and watching the boot check still
//! run the mutated image.

fn main() {
    println!("cargo:rerun-if-changed=link.ld");
    // The entry stub and the exception vectors are pulled in with
    // `global_asm!(include_str!(...))`, which cargo does not track either.
    println!("cargo:rerun-if-changed=src/boot.s");
    println!("cargo:rerun-if-changed=src/arch/aarch64/exception/vectors.s");
    println!("cargo:rerun-if-changed=build.rs");
}
