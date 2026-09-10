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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            game_root: None,
            keep_backups: DEFAULT_KEEP_BACKUPS,
            extra_allowed_dirs: Vec::new(),
            language: None,
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
    pub fn allowed_roots(&self) -> valsync_core::AllowedRoots {
        let mut roots = valsync_core::AllowedRoots::bepinex();
        for d in &self.extra_allowed_dirs {
            roots.allow_dir(d);
        }
        roots
    }
}
