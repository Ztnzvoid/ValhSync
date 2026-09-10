//! `valsync`: the player launcher. CLI today; the graphical window (M5) will
//! share every module of the library half.

fn main() {
    if let Err(e) = valsync::cli::run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
