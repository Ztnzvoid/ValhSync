//! Backups and rollback.
//!
//! Before a sync touches the game folder it opens a backup: a folder under the
//! backups dir with a `backup.json` journal. Every change is recorded there
//! *as it is done*, with the replaced or removed bytes stashed under `files/`.
//! Rolling back replays the journal in reverse, so a sync interrupted at any
//! point can be undone exactly.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use valsync_core::path::{ensure_within_root, to_os_path};
use valsync_core::{InstalledState, clock};

use crate::error::{Result, SyncError};
use crate::paths::{AppPaths, copy_atomic, move_file, read_json, write_json_atomic};
use crate::servers::KnownServer;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Done {
    /// New file written; rollback deletes it.
    Added { path: String },
    /// Existing file stashed then overwritten; rollback puts the stash back.
    Replaced { path: String },
    /// Existing file moved into the stash; rollback puts it back.
    Removed { path: String },
    /// Moved inside the game folder to the quarantine; rollback moves it back.
    Quarantined { path: String, to: String },
}

impl Done {
    pub fn path(&self) -> &str {
        match self {
            Self::Added { path }
            | Self::Replaced { path }
            | Self::Removed { path }
            | Self::Quarantined { path, .. } => path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRecord {
    pub stamp: String,
    pub created_at: String,
    pub server_id: String,
    pub server_name: String,
    pub pack_id: String,
    pub game_root: PathBuf,
    /// `installed.json` as it was before the sync (`None` = first sync).
    pub previous_state: Option<InstalledState>,
    pub done: Vec<Done>,
    /// Set once the sync finished; a backup without it belongs to an
    /// interrupted sync.
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub restored_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Backup {
    pub dir: PathBuf,
    pub record: BackupRecord,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RollbackReport {
    pub restored: usize,
    pub deleted: usize,
    pub unquarantined: usize,
}

impl Backup {
    pub fn create(
        paths: &AppPaths,
        server: &KnownServer,
        pack_id: &str,
        game_root: &Path,
        previous_state: Option<InstalledState>,
    ) -> Result<Self> {
        let mut stamp = clock::dir_stamp();
        let mut dir = paths.backups_dir.join(&stamp);
        // Two syncs within the same second: make the name unique.
        let mut n = 1;
        while dir.exists() {
            stamp = format!("{}-{n}", clock::dir_stamp());
            dir = paths.backups_dir.join(&stamp);
            n += 1;
        }
        SyncError::at(
            &dir,
            "cannot create",
            std::fs::create_dir_all(dir.join("files")),
        )?;
        let backup = Self {
            dir,
            record: BackupRecord {
                stamp,
                created_at: clock::now_rfc3339(),
                server_id: server.id.clone(),
                server_name: server.name.clone(),
                pack_id: pack_id.to_string(),
                game_root: game_root.to_path_buf(),
                previous_state,
                done: Vec::new(),
                completed_at: None,
                restored_at: None,
            },
        };
        backup.save()?;
        Ok(backup)
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let record: BackupRecord = read_json(&dir.join("backup.json"))?
            .ok_or_else(|| SyncError::Other(format!("{} has no backup.json", dir.display())))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            record,
        })
    }

    /// All backups, newest first.
    pub fn list(paths: &AppPaths) -> Result<Vec<Self>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&paths.backups_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(SyncError::io("cannot list backups", e)),
        };
        for entry in entries.flatten() {
            if entry.path().is_dir()
                && let Ok(b) = Self::load(&entry.path())
            {
                out.push(b);
            }
        }
        out.sort_by(|a, b| b.record.stamp.cmp(&a.record.stamp));
        Ok(out)
    }

    /// The newest backup that has not been restored yet.
    pub fn latest_restorable(paths: &AppPaths) -> Result<Option<Self>> {
        Ok(Self::list(paths)?
            .into_iter()
            .find(|b| b.record.restored_at.is_none()))
    }

    pub fn save(&self) -> Result<()> {
        write_json_atomic(&self.dir.join("backup.json"), &self.record)
    }

    /// Where the previous bytes of `rel` are stashed.
    pub fn stash_path(&self, rel: &str) -> PathBuf {
        to_os_path(&self.dir.join("files"), rel)
    }

    /// Journal one change. Written to disk immediately.
    pub fn record_done(&mut self, done: Done) -> Result<()> {
        self.record.done.push(done);
        self.save()
    }

    pub fn mark_completed(&mut self) -> Result<()> {
        self.record.completed_at = Some(clock::now_rfc3339());
        self.save()
    }

    /// Undo every journaled change, newest first, then restore the previous
    /// installed state. Idempotent: a second call finds nothing to do.
    pub fn rollback(&mut self, installed_file: &Path) -> Result<RollbackReport> {
        let root = self.record.game_root.clone();
        let mut report = RollbackReport::default();
        if self.record.restored_at.is_some() {
            return Ok(report);
        }
        let done = std::mem::take(&mut self.record.done);
        let mut remaining = done.clone();

        for entry in done.iter().rev() {
            match entry {
                Done::Added { path } => {
                    let target = ensure_within_root(&root, path)?;
                    match std::fs::remove_file(&target) {
                        Ok(()) => report.deleted += 1,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => {
                            self.record.done = remaining;
                            self.save()?;
                            return Err(SyncError::io(
                                format!("cannot delete {}", target.display()),
                                e,
                            ));
                        }
                    }
                }
                Done::Replaced { path } | Done::Removed { path } => {
                    let stash = self.stash_path(path);
                    if stash.is_file() {
                        let target = ensure_within_root(&root, path)?;
                        if let Err(e) = copy_atomic(&stash, &target) {
                            self.record.done = remaining;
                            self.save()?;
                            return Err(e);
                        }
                        report.restored += 1;
                    }
                }
                Done::Quarantined { path, to } => {
                    let from = ensure_within_root(&root, to)?;
                    if from.is_file() {
                        let target = ensure_within_root(&root, path)?;
                        if let Err(e) = move_file(&from, &target) {
                            self.record.done = remaining;
                            self.save()?;
                            return Err(e);
                        }
                        report.unquarantined += 1;
                    }
                }
            }
            remaining.pop();
        }

        match &self.record.previous_state {
            Some(state) => write_json_atomic(installed_file, state)?,
            None => {
                if let Err(e) = std::fs::remove_file(installed_file)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(SyncError::io("cannot reset installed state", e));
                }
            }
        }
        // Keep the journal for the record, but flag it as replayed.
        self.record.done = done;
        self.record.restored_at = Some(clock::now_rfc3339());
        self.save()?;
        Ok(report)
    }

    /// Delete the oldest backups beyond `keep`. Returns how many were removed.
    pub fn prune(paths: &AppPaths, keep: usize) -> Result<usize> {
        let all = Self::list(paths)?;
        let mut removed = 0;
        for old in all.iter().skip(keep.max(1)) {
            if std::fs::remove_dir_all(&old.dir).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> KnownServer {
        KnownServer {
            id: "k".into(),
            name: "S".into(),
            url: "http://x".into(),
            added_at: String::new(),
            last_pack_id: None,
        }
    }

    #[test]
    fn journal_and_rollback() {
        let home = tempfile::tempdir().unwrap();
        let paths = AppPaths::at(home.path()).unwrap();
        let game = tempfile::tempdir().unwrap();
        let g = game.path();
        std::fs::create_dir_all(g.join("BepInEx/plugins/Old")).unwrap();
        std::fs::write(g.join("winhttp.dll"), b"old doorstop").unwrap();
        std::fs::write(g.join("BepInEx/plugins/Old/Old.dll"), b"old").unwrap();
        std::fs::write(g.join("BepInEx/plugins/Stray.dll"), b"stray").unwrap();
        let installed = paths.installed_file();

        let mut b = Backup::create(&paths, &server(), "b3:x", g, None).unwrap();

        // replace winhttp.dll
        std::fs::copy(g.join("winhttp.dll"), b.stash_path("winhttp.dll")).unwrap();
        std::fs::write(g.join("winhttp.dll"), b"new doorstop").unwrap();
        b.record_done(Done::Replaced {
            path: "winhttp.dll".into(),
        })
        .unwrap();
        // remove Old.dll
        move_file(
            &g.join("BepInEx/plugins/Old/Old.dll"),
            &b.stash_path("BepInEx/plugins/Old/Old.dll"),
        )
        .unwrap();
        b.record_done(Done::Removed {
            path: "BepInEx/plugins/Old/Old.dll".into(),
        })
        .unwrap();
        // add New.dll
        std::fs::create_dir_all(g.join("BepInEx/plugins/New")).unwrap();
        std::fs::write(g.join("BepInEx/plugins/New/New.dll"), b"new").unwrap();
        b.record_done(Done::Added {
            path: "BepInEx/plugins/New/New.dll".into(),
        })
        .unwrap();
        // quarantine Stray.dll
        let q = "BepInEx/_valsync_quarantine/x/BepInEx/plugins/Stray.dll";
        move_file(&g.join("BepInEx/plugins/Stray.dll"), &to_os_path(g, q)).unwrap();
        b.record_done(Done::Quarantined {
            path: "BepInEx/plugins/Stray.dll".into(),
            to: q.into(),
        })
        .unwrap();
        std::fs::write(&installed, b"{}").unwrap();

        // reload from disk, as `valsync rollback` would
        let mut loaded = Backup::latest_restorable(&paths).unwrap().unwrap();
        assert_eq!(loaded.record.done.len(), 4);
        let report = loaded.rollback(&installed).unwrap();
        assert_eq!(
            report,
            RollbackReport {
                restored: 2,
                deleted: 1,
                unquarantined: 1
            }
        );
        assert_eq!(
            std::fs::read(g.join("winhttp.dll")).unwrap(),
            b"old doorstop"
        );
        assert_eq!(
            std::fs::read(g.join("BepInEx/plugins/Old/Old.dll")).unwrap(),
            b"old"
        );
        assert!(!g.join("BepInEx/plugins/New/New.dll").exists());
        assert_eq!(
            std::fs::read(g.join("BepInEx/plugins/Stray.dll")).unwrap(),
            b"stray"
        );
        assert!(!installed.exists(), "first sync: state file removed");
        assert!(Backup::latest_restorable(&paths).unwrap().is_none());

        // idempotent
        let report = loaded.rollback(&installed).unwrap();
        assert_eq!(report, RollbackReport::default());
    }

    #[test]
    fn prune_keeps_newest() {
        let home = tempfile::tempdir().unwrap();
        let paths = AppPaths::at(home.path()).unwrap();
        let game = tempfile::tempdir().unwrap();
        for _ in 0..4 {
            Backup::create(&paths, &server(), "p", game.path(), None).unwrap();
        }
        assert_eq!(Backup::list(&paths).unwrap().len(), 4);
        assert_eq!(Backup::prune(&paths, 2).unwrap(), 2);
        assert_eq!(Backup::list(&paths).unwrap().len(), 2);
    }
}
