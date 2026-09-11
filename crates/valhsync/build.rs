//! Records the target triple this launcher was built for.
//!
//! It has to be the exact string the publisher writes into an offer, because
//! the launcher compares the two to decide whether an offered build runs on
//! this machine at all. Only Cargo knows it: `std::env::consts` cannot spell a
//! triple (it cannot tell `-gnu` from `-musl`, nor `-msvc` from `-gnu` on
//! Windows), and `TARGET` is handed to build scripts alone.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo::rustc-env=VALHSYNC_TARGET={target}");
}
