//! The admin window. Same look as the player launcher, because it is the same
//! product: burnt wood, bone, worn brass.
//!
//! It exists because everything this window does was, until now, a TOML file
//! to hand-edit and a console to read: finding the dedicated server, knowing
//! that a crossplay server refuses local addresses, deciding which mods are
//! server-only, and copying an invite code out of a terminal.

mod app;
pub mod i18n;
mod worker;

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use valhsync_ui::theme;

/// Open the window. `config_path` and `data_dir` are the same ones the CLI
/// uses, so both faces of the program work on one configuration.
pub fn run(config_path: PathBuf, data_dir: PathBuf) -> Result<()> {
    let options = eframe::NativeOptions {
        // Tall enough that the server tab opens with the whole of it on
        // screen -- the card, the log and the prompt -- without a scroll to
        // discover. Clamped to the display in `run`, since not every screen
        // is this tall.
        viewport: valhsync_ui::frame::viewport(
            "ValhSync · Serveur",
            [820.0, 940.0],
            [680.0, 520.0],
        ),
        ..Default::default()
    };
    eframe::run_native(
        "ValhSync Server",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::App::new(&cc.egui_ctx, config_path, data_dir)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
    .context("cannot open the window; run `valhsync-server --help` for the command line")
}
