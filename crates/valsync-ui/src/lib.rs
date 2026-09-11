//! The look shared by both ValSync windows: burnt wood, polished bone, worn
//! brass, northern mist. One definition, so the launcher and the server
//! console are visibly the same product. Tokens documented in
//! `docs/palette.md`.

pub mod console;
pub mod frame;
pub mod theme;
pub mod widgets;

pub use theme::{apply, callout, card, icon};
