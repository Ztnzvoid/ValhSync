//! The admin window. Same look as the player launcher, because it is the same
//! product: burnt wood, bone, worn brass.
//!
//! It exists because everything this window does was, until now, a TOML file
//! to hand-edit and a console to read: finding the dedicated server, knowing
//! that a crossplay server refuses local addresses, deciding which mods are
//! server-only, and copying an invite code out of a terminal.

mod app;
mod worker;

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use eframe::egui;
use valsync_ui::theme;

/// Open the window. `config_path` and `data_dir` are the same ones the CLI
/// uses, so both faces of the program work on one configuration.
pub fn run(config_path: PathBuf, data_dir: PathBuf) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ValSync · Serveur")
            .with_inner_size([780.0, 760.0])
            .with_min_inner_size([680.0, 560.0])
            .with_icon(theme::icon()),
        ..Default::default()
    };
    eframe::run_native(
        "ValSync Server",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::App::new(&cc.egui_ctx, config_path, data_dir)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
    .context("cannot open the window; run `valsync-server --help` for the command line")
}
