//! Records the target triple this publisher was built for.
//!
//! The triple has to be the exact string a launcher compares against when it
//! decides whether an offered build runs on its machine. Only Cargo knows it:
//! `std::env::consts` cannot spell a triple (it cannot tell `-gnu` from
//! `-musl`, nor `-msvc` from `-gnu` on Windows), and `TARGET` is handed to
//! build scripts alone.

const DESCRIPTION: &str = "ValhSync publisher for Valheim mod packs";
const ORIGINAL_FILENAME: &str = "valhsync-server.exe";

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo::rustc-env=VALHSYNC_TARGET={target}");

    // Windows metadata. Without it the binary has no product name, no
    // publisher and no version at all -- the default for a Rust executable,
    // and one of the things a heuristic scanner weighs when it has never seen
    // a file before and nobody has signed it. It is not a substitute for
    // signing; it is the part that costs nothing.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "ValhSync")
            .set("FileDescription", DESCRIPTION)
            .set("CompanyName", "ValhSync")
            .set("LegalCopyright", "MIT OR Apache-2.0")
            .set("OriginalFilename", ORIGINAL_FILENAME);
        if let Err(e) = res.compile() {
            // A missing resource compiler must not stop anyone building from
            // source; the binary is merely nameless again.
            println!("cargo::warning=no version resource embedded: {e}");
        }
    }
}
