//! Library half of the ValhSync launcher.
//!
//! Everything the CLI and the graphical window need lives here: known servers,
//! settings, HTTP client, game location and launch, the sync engine with its
//! backups and rollback. `main.rs` only dispatches.

pub mod backup;
pub mod cli;
pub mod engine;
pub mod error;
pub mod game;
pub mod gui;
pub mod http;
pub mod invite_file;
pub mod paths;
pub mod selfupdate;
pub mod servers;
pub mod settings;
pub mod vanilla;

pub use error::SyncError;

/// Name of the sidecar file an admin can ship next to the executable so the
/// launcher joins the server on first start.
pub const INVITE_FILE_NAME: &str = "valhsync-invite.txt";

/// Working directory created inside the game root during a sync, so the final
/// renames stay on the same volume. Removed when the sync ends.
pub const TMP_DIR_NAME: &str = "_valhsync_tmp";
