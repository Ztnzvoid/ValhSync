//! The one window players see. Built on egui: pure Rust, one executable, no
//! WebView. Every action goes through the same engine as the CLI.

mod app;
pub mod i18n;
pub mod theme;

use anyhow::{Context as _, Result};
use eframe::egui;

pub fn run() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ValSync")
            .with_inner_size([640.0, 500.0])
            .with_min_inner_size([540.0, 420.0])
            .with_icon(theme::icon()),
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
