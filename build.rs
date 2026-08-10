//! Tell cargo about the inputs it cannot see.
//!
//! The linker script is passed through `rustflags` in `.cargo/config.toml`, so
//! cargo has no idea it exists: editing it leaves the previous binary in place
//! and the next build silently links the old layout. On a kernel that is not a
//! stale artefact, it is a stale *memory map* — regions, permissions, stack and
//! guard page all come from that file, and the mismatch surfaces as a fault
//! somewhere unrelated.
//!
//! Found by editing `link.ld`, restoring it, and watching the boot check still
//! run the mutated image.
//!
//! Boot entry and the linker script live under the active ISA tree
//! (`src/arch/aarch64/`); a port adds a sibling path and updates this file.

fn main() {
    emit_source_id();
    println!("cargo:rerun-if-changed=src/arch/aarch64/link.ld");
    // The entry stub and the exception vectors are pulled in with
    // `global_asm!(include_str!(...))`, which cargo does not track either.
    println!("cargo:rerun-if-changed=src/arch/aarch64/boot.s");
    println!("cargo:rerun-if-changed=src/arch/aarch64/exception/vectors.s");
    println!("cargo:rerun-if-changed=src/arch/x86_64/link.ld");
    println!("cargo:rerun-if-changed=src/arch/x86_64/boot.s");
    println!("cargo:rerun-if-changed=build.rs");
}

/// Stamp the image with the source it was built from.
///
/// A flashed card is otherwise indistinguishable from another one, and the
/// `build:` line only said *which features* — not which tree. That gap cost a
/// session: a headless image on a board with the panel attached looks exactly
/// like broken hardware. Transcripts are the evidence ADRs cite, so a
/// transcript that cannot name its own commit is evidence about nothing in
/// particular.
///
/// `--dirty` matters more than the hash: a stamp that hides uncommitted edits
/// would let a transcript claim a commit that never contained the code that
/// produced it. No git, no repo, no problem — the value becomes `nogit`, which
/// is a printed outcome rather than a silent omission.
fn emit_source_id() {
    // Cargo cannot see a commit happening, so without these the stamp would
    // survive across commits and lie by staleness. `.git/HEAD` moves on
    // checkout, `.git/index` on stage — together they cover the transitions
    // that change `git describe`'s answer.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    let described = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty", "--abbrev=8"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "nogit".to_string());

    println!("cargo:rustc-env=HARBOR_SOURCE_ID={described}");
}
