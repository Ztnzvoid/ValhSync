use std::path::PathBuf;

use crate::path::PathError;

/// Every failure the core can report. Messages are written for the person
/// reading them: they say what was wrong and, where possible, what to do.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid path {path:?}: {reason}")]
    InvalidPath {
        path: String,
        #[source]
        reason: PathError,
    },

    #[error("manifest rejected: {0}")]
    InvalidManifest(String),

    #[error("signature verification failed: the manifest was not signed by the pinned server key")]
    BadSignature,

    #[error("invalid key material: {0}")]
    InvalidKey(String),

    #[error("invalid invite code: {0}")]
    InvalidInvite(String),

    #[error("limit exceeded: {0}")]
    LimitExceeded(String),

    #[error("hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("refusing to follow a symbolic link or junction at {0}")]
    SymlinkRefused(PathBuf),

    #[error("{0}: a directory is in the way of a file to install")]
    Obstructed(PathBuf),

    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid glob pattern {pattern:?}: {source}")]
    Glob {
        pattern: String,
        #[source]
        source: globset::Error,
    },
}

impl CoreError {
    /// Attach the path that an I/O error concerns; bare `std::io::Error`
    /// messages are useless to a player.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
