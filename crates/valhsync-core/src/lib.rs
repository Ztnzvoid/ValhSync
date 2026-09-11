//! Core library of ValhSync.
//!
//! Everything here is pure logic over bytes and the local filesystem: the
//! manifest model, path validation, BLAKE3 hashing, Ed25519 signatures, pack
//! scanning and sync planning. There is deliberately **no network I/O** in this
//! crate, so all of it can be unit tested against temporary directories.
//!
//! The crate must never panic on hostile input: a manifest is data received
//! from the network, and every function that touches it returns an error
//! instead of unwrapping.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo)
)]

pub mod clock;
pub mod crash;
pub mod error;
pub mod gamelog;
pub mod hash;
pub mod invite;
pub mod limits;
pub mod manifest;
pub mod path;
pub mod plan;
pub mod scan;
pub mod sign;
pub mod state;
pub mod steam;

pub use error::{CoreError, Result};
pub use invite::Invite;
pub use limits::Limits;
pub use manifest::{FileEntry, Manifest, Policy};
pub use path::AllowedRoots;
pub use plan::{Action, SyncPlan};
pub use sign::{Keypair, PublicKey};
pub use state::InstalledState;

/// Manifest format version understood by this build.
pub const MANIFEST_FORMAT: u32 = 1;

/// Relative directory (under the game root) where unmanaged files are moved.
/// Lives under `BepInEx/` but outside `plugins/`, so BepInEx never loads it.
pub const QUARANTINE_ROOT: &str = "BepInEx/_valhsync_quarantine";
