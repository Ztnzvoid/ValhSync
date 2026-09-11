//! Records the target triple this publisher was built for.
//!
//! The triple has to be the exact string a launcher compares against when it
//! decides whether an offered build runs on its machine. Only Cargo knows it:
//! `std::env::consts` cannot spell a triple (it cannot tell `-gnu` from
//! `-musl`, nor `-msvc` from `-gnu` on Windows), and `TARGET` is handed to
//! build scripts alone.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo::rustc-env=VALHSYNC_TARGET={target}");
}
