//! Where the launcher keeps its own files.
//!
//! - config dir (`%APPDATA%\valhsync`, `~/.config/valhsync`): known servers,
//!   settings, installed state
//! - backups dir (`%LOCALAPPDATA%\valhsync\backups`, `~/.local/share/valhsync/backups`)
//!
//! Nothing else on the machine is written to, apart from the game folder.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Result, SyncError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub backups_dir: PathBuf,
}

impl AppPaths {
    /// Platform defaults.
    pub fn discover() -> Result<Self> {
        let base = directories::BaseDirs::new()
            .ok_or_else(|| SyncError::Other("cannot determine the user's home directory".into()))?;
        let paths = Self {
            config_dir: base.config_dir().join("valhsync"),
            backups_dir: base.data_local_dir().join("valhsync").join("backups"),
        };
        paths.ensure()?;
        Ok(paths)
    }

    /// Everything under one folder. Used by tests and portable setups.
    pub fn at(home: &Path) -> Result<Self> {
        let paths = Self {
            config_dir: home.join("config"),
            backups_dir: home.join("backups"),
        };
        paths.ensure()?;
        Ok(paths)
    }

    fn ensure(&self) -> Result<()> {
        for d in [&self.config_dir, &self.backups_dir] {
            SyncError::at(d, "cannot create", std::fs::create_dir_all(d))?;
        }
        Ok(())
    }

    pub fn servers_file(&self) -> PathBuf {
        self.config_dir.join("servers.json")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    pub fn installed_file(&self) -> PathBuf {
        self.config_dir.join("installed.json")
    }
}

/// Write JSON through a temp file and a rename, so a crash never leaves a
/// half-written state file.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_vec_pretty(value)?;
    SyncError::at(&tmp, "cannot write", std::fs::write(&tmp, text))?;
    SyncError::at(path, "cannot replace", std::fs::rename(&tmp, path))?;
    Ok(())
}

/// `Ok(None)` when the file does not exist.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).map_err(|e| {
            SyncError::Other(format!(
                "{} is corrupted ({e}); delete it to start over",
                path.display()
            ))
        })?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(SyncError::io(format!("cannot read {}", path.display()), e)),
    }
}

/// Move a file, falling back to copy + delete across volumes.
pub fn move_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        SyncError::at(parent, "cannot create", std::fs::create_dir_all(parent))?;
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    SyncError::at(dst, "cannot copy to", std::fs::copy(src, dst).map(|_| ()))?;
    SyncError::at(src, "cannot remove", std::fs::remove_file(src))?;
    Ok(())
}

/// After deleting or moving a file out, drop the now-empty folders above it,
/// stopping at `root`. Keeps `BepInEx/plugins` free of husks.
pub fn remove_empty_parents(root: &Path, file: &Path) {
    let mut cur = file.parent();
    while let Some(dir) = cur {
        if dir == root || !dir.starts_with(root) {
            break;
        }
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        cur = dir.parent();
    }
}

/// Copy a file into place atomically (temp file beside the target, then rename).
pub fn copy_atomic(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        SyncError::at(parent, "cannot create", std::fs::create_dir_all(parent))?;
    }
    let tmp = dst.with_extension("valhsync-tmp");
    SyncError::at(&tmp, "cannot copy to", std::fs::copy(src, &tmp).map(|_| ()))?;
    SyncError::at(dst, "cannot replace", std::fs::rename(&tmp, dst))?;
    Ok(())
}
