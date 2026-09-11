//! The one window players see. Built on egui: pure Rust, one executable, no
//! WebView. Every action goes through the same engine as the CLI.

mod app;
pub mod i18n;

pub use valsync_ui::theme;

use anyhow::{Context as _, Result};

pub fn run() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: valsync_ui::frame::viewport("ValSync", [660.0, 540.0], [560.0, 440.0]),
        ..Default::default()
    };
    eframe::run_native(
        "ValSync",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::App::new(&cc.egui_ctx)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
    .context("cannot open the window; run `valsync --help` for the command-line interface")
}
