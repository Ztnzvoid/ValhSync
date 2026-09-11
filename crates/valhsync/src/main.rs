//! `valhsync`: the player launcher.
//!
//! One executable, two faces: double-clicked (no arguments) it opens the
//! window; given a subcommand it behaves as a command-line tool. On Windows
//! the binary is a windowed app, so the CLI path first re-attaches to the
//! parent console.

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    // Before anything else: a panic must leave a file behind. A windowed
    // program that dies otherwise vanishes with nothing to report.
    if let Ok(paths) = valhsync::paths::AppPaths::discover() {
        valhsync_core::crash::install_hook(&paths.config_dir);
    }
    let has_args = std::env::args_os().len() > 1;
    if !has_args {
        if let Err(e) = valhsync::gui::run() {
            valhsync_ui::console::attach_parent();
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    valhsync_ui::console::attach_parent();
    if let Err(e) = valhsync::cli::run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
