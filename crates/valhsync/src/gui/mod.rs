//! The one window players see. Built on egui: pure Rust, one executable, no
//! WebView. Every action goes through the same engine as the CLI.

mod app;
pub mod i18n;

pub use valhsync_ui::theme;

use anyhow::{Context as _, Result};

pub fn run() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: valhsync_ui::frame::viewport("ValhSync", [940.0, 660.0], [720.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "ValhSync",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::App::new(&cc.egui_ctx)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
    .context("cannot open the window; run `valhsync --help` for the command-line interface")
}
