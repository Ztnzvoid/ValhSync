//! Where the launcher keeps its own files.
//!
//! - config dir (`%APPDATA%\valhsync`, `~/.config/valhsync`): known servers,
//!   settings, installed state
//! - backups dir (`%LOCALAPPDATA%\valhsync\backups`, `~/.local/share/valhsync/backups`)
//!
//! Nothing else on the machine is written to, apart from the game folder.
//!
//! The profile is the right home: a player replaces the executable with a
//! newer one and keeps their servers. Two ways to put it elsewhere — a
//! `valhsync-data` folder beside the executable, or `VALHSYNC_HOME` — which is
//! what a player on a shared machine or a stick wants, and what anyone
//! testing the launcher as a newcomer needs.

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
    /// `VALHSYNC_HOME`, else a `valhsync-data` folder beside the executable,
    /// else the platform defaults.
    pub fn discover() -> Result<Self> {
        if let Some(home) = portable_home() {
            return Self::at(&home);
        }
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

    /// True when this run keeps its files somewhere other than the profile.
    #[must_use]
    pub fn is_portable() -> bool {
        portable_home().is_some()
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

    pub fn news_file(&self) -> PathBuf {
        self.config_dir.join("news.json")
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

/// The folder this run should keep its files in, when it is not the profile.
///
/// `VALHSYNC_HOME` first, so a test or a script can say so without moving
/// anything. Then a `valhsync-data` folder next to the executable: it is only
/// honoured when someone created it, never made on its own, so an ordinary
/// player is never quietly switched to a portable install.
fn portable_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("VALHSYNC_HOME") {
        let home = PathBuf::from(home);
        if !home.as_os_str().is_empty() {
            return Some(home);
        }
    }
    let beside = std::env::current_exe().ok()?.parent()?.join(PORTABLE_DIR);
    beside.is_dir().then_some(beside)
}

/// Create this next to `valhsync.exe` to keep everything with it.
pub const PORTABLE_DIR: &str = "valhsync-data";

#[cfg(test)]
mod portable_tests {
    use super::*;

    #[test]
    fn a_home_of_ones_own_holds_everything_together() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::at(tmp.path()).unwrap();
        assert!(paths.config_dir.starts_with(tmp.path()));
        assert!(paths.backups_dir.starts_with(tmp.path()));
        assert!(paths.config_dir.is_dir() && paths.backups_dir.is_dir());
        assert!(paths.servers_file().starts_with(&paths.config_dir));
    }
}
