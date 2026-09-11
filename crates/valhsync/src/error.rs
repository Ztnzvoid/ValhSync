use std::path::PathBuf;

use valhsync_core::CoreError;

/// Every failure the launcher can report to a player. Each message says what
/// happened and what to do next.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(
        "cannot reach the server at {url}: {reason}. Check your connection, or ask the admin whether valhsync-server is running"
    )]
    Unreachable { url: String, reason: String },

    #[error(
        "the server at {url} answered HTTP {status} for {what}; it may be misconfigured or out of date"
    )]
    HttpStatus {
        url: String,
        status: u16,
        what: String,
    },

    #[error(
        "the server's signing key does not match the one pinned for \"{name}\" (pinned {pinned}, received {received}). Nothing was changed. Ask the admin for a fresh invite code and import it with `valhsync join --replace-key`"
    )]
    KeyMismatch {
        name: String,
        pinned: String,
        received: String,
    },

    #[error("Valheim is running; close it, then try again")]
    GameRunning,

    #[error(
        "Valheim was not found. Tell ValhSync where it is with `valhsync game-root <folder>` (the folder containing valheim.exe)"
    )]
    GameNotFound,

    #[error("{0} does not look like a Valheim folder (no valheim.exe or valheim.x86_64 inside)")]
    NotAGameFolder(PathBuf),

    #[error("Steam was not found; start Valheim from Steam yourself, or install Steam")]
    SteamNotFound,

    #[error("no server known yet; import an invite code with `valhsync join <code>`")]
    NoServer,

    #[error("no server named {0:?}; run `valhsync servers` to list them")]
    UnknownServer(String),

    #[error("several servers are known; say which one: {0}")]
    AmbiguousServer(String),

    #[error("download of {path} failed: {reason}")]
    Download { path: String, reason: String },

    #[error(
        "the sync failed while applying changes ({reason}); the previous state was restored from backup {stamp}"
    )]
    RolledBack { reason: String, stamp: String },

    #[error(
        "the sync failed while applying changes ({reason}) AND restoring backup {stamp} failed too ({rollback_error}). Fix the cause, then run `valhsync rollback`"
    )]
    RollbackFailed {
        reason: String,
        stamp: String,
        rollback_error: String,
    },

    #[error(
        "\"{name}\" sent a manifest generated at {received}, older than the one already applied ({seen}). This can be a replay of an old pack by someone on the network path. Nothing was changed. If the admin really rolled the pack back, run again with --allow-older"
    )]
    OlderManifest {
        name: String,
        seen: String,
        received: String,
    },

    #[error("no backup to restore")]
    NoBackup,

    #[error(
        "first sync needs a confirmation; review the plan with `valhsync status` and run again with --yes"
    )]
    NeedsConfirmation,

    #[error("{0}")]
    Core(#[from] CoreError),

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{0}")]
    Other(String),
}

impl SyncError {
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// Wrap an I/O result with a message naming the path.
    pub fn at<T>(
        path: &std::path::Path,
        what: &str,
        r: std::io::Result<T>,
    ) -> std::result::Result<T, Self> {
        r.map_err(|e| Self::io(format!("{what} {}", path.display()), e))
    }
}

impl From<serde_json::Error> for SyncError {
    fn from(e: serde_json::Error) -> Self {
        Self::Other(format!("invalid JSON: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, SyncError>;
