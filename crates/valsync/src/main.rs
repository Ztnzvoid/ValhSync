//! `valsync`: the player launcher.
//!
//! One executable, two faces: double-clicked (no arguments) it opens the
//! window; given a subcommand it behaves as a command-line tool. On Windows
//! the binary is a windowed app, so the CLI path first re-attaches to the
//! parent console.

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let has_args = std::env::args_os().len() > 1;
    if !has_args {
        if let Err(e) = valsync::gui::run() {
            valsync::console::attach_parent();
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    valsync::console::attach_parent();
    if let Err(e) = valsync::cli::run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
