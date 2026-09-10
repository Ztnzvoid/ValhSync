//! What ValSync itself installed on this machine. This is how the launcher
//! knows which files it may remove later (only its own) and what to restore.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, Policy};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledFile {
    pub path: String,
    pub blake3: String,
    pub policy: Policy,
}

/// Snapshot of the last pack applied to the game folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledState {
    /// Public key (base64url) of the server this pack came from.
    pub server_id: String,
    pub server_name: String,
    pub pack_id: String,
    pub applied_at: String,
    pub files: Vec<InstalledFile>,
}

impl InstalledState {
    /// The state that describes `manifest` once it is fully applied.
    pub fn from_manifest(manifest: &Manifest, server_id: &str) -> Self {
        Self {
            server_id: server_id.to_string(),
            server_name: manifest.server_name.clone(),
            pack_id: manifest.pack_id.clone(),
            applied_at: crate::clock::now_rfc3339(),
            files: manifest
                .files
                .iter()
                .map(|f| InstalledFile {
                    path: f.path.clone(),
                    blake3: f.blake3.clone(),
                    policy: f.policy,
                })
                .collect(),
        }
    }

    pub fn by_lower_path(&self) -> HashMap<String, &InstalledFile> {
        self.files
            .iter()
            .map(|f| (f.path.to_lowercase(), f))
            .collect()
    }
}
