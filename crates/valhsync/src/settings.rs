//! Player-side settings (`settings.json`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::paths::{AppPaths, read_json, write_json_atomic};

pub const DEFAULT_KEEP_BACKUPS: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Manual override of the game folder. Auto-detected when unset.
    pub game_root: Option<PathBuf>,
    /// How many backups to keep.
    pub keep_backups: usize,
    /// Extra root-level directories a manifest may write to, on top of the
    /// BepInEx defaults. Empty unless a server needs something unusual.
    pub extra_allowed_dirs: Vec<String>,
    /// UI language (`fr`, `en`); system locale when unset.
    pub language: Option<String>,
    /// BLAKE3 of the last build installed from a server's update channel.
    /// A server whose offer names a version its file does not actually carry
    /// would otherwise be accepted, restart no newer, and be accepted again
    /// for as long as the player kept the window open.
    pub installed_build: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            game_root: None,
            keep_backups: DEFAULT_KEEP_BACKUPS,
            extra_allowed_dirs: Vec::new(),
            language: None,
            installed_build: None,
        }
    }
}

impl Settings {
    pub fn load(paths: &AppPaths) -> Result<Self> {
        Ok(read_json(&paths.settings_file())?.unwrap_or_default())
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        write_json_atomic(&paths.settings_file(), self)
    }

    /// The allow-list a manifest is checked against on this machine.
    pub fn allowed_roots(&self) -> valhsync_core::AllowedRoots {
        let mut roots = valhsync_core::AllowedRoots::bepinex();
        for d in &self.extra_allowed_dirs {
            roots.allow_dir(d);
        }
        roots
    }
}
