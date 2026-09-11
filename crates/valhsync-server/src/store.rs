//! Content-addressed store: `store/<blake3>` holds an immutable copy of every
//! published file. HTTP never touches the admin's folders directly, so a URL
//! can only ever name a hash, never a path.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use valhsync_core::hash;
use valhsync_core::scan::ScannedFile;

#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Self> {
        let dir = data_dir.join("store");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn path_for(&self, hex: &str) -> Option<PathBuf> {
        hash::is_hex_hash(hex).then(|| self.dir.join(hex))
    }

    /// Copy every scanned file into the store. Files already present are
    /// skipped. Returns how many were added.
    pub fn ingest(&self, files: &[ScannedFile]) -> Result<usize> {
        let mut added = 0;
        for f in files {
            let dest = self.dir.join(&f.entry.blake3);
            if let Ok(md) = std::fs::metadata(&dest)
                && md.is_file()
                && md.len() == f.entry.size
            {
                continue;
            }
            let tmp = self
                .dir
                .join(format!("{}.tmp-{}", f.entry.blake3, std::process::id()));
            std::fs::copy(&f.source, &tmp)
                .with_context(|| format!("cannot copy {} into the store", f.source.display()))?;
            // The file may have changed between hashing and copying.
            if let Err(e) = hash::verify_file(&tmp, &f.entry.blake3, &f.entry.path) {
                let _ = std::fs::remove_file(&tmp);
                bail!(
                    "{} changed while it was being packaged ({e}); scan again",
                    f.entry.path
                );
            }
            std::fs::rename(&tmp, &dest)
                .with_context(|| format!("cannot finalize {} in the store", f.entry.path))?;
            added += 1;
        }
        Ok(added)
    }

    /// Delete blobs that no live manifest references, plus stale temp files.
    pub fn gc(&self, keep: &HashSet<String>) -> Result<usize> {
        let mut removed = 0;
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let stale_tmp = name.contains(".tmp-");
            let unreferenced = hash::is_hex_hash(name) && !keep.contains(name);
            if (stale_tmp || unreferenced) && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use valhsync_core::{FileEntry, Policy};

    fn scanned(dir: &Path, name: &str, content: &[u8]) -> ScannedFile {
        let source = dir.join(name);
        std::fs::write(&source, content).unwrap();
        ScannedFile {
            entry: FileEntry {
                path: format!("BepInEx/plugins/{name}"),
                size: content.len() as u64,
                blake3: hash::to_hex(&hash::hash_bytes(content)),
                policy: Policy::Enforce,
            },
            source,
        }
    }

    #[test]
    fn ingest_is_idempotent_and_gc_keeps_live_blobs() {
        let data = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();
        let a = scanned(src.path(), "a.dll", b"aaa");
        let b = scanned(src.path(), "b.dll", b"bbb");
        assert_eq!(store.ingest(&[a.clone(), b.clone()]).unwrap(), 2);
        assert_eq!(store.ingest(&[a.clone(), b.clone()]).unwrap(), 0);
        assert!(store.path_for(&a.entry.blake3).unwrap().is_file());
        assert!(store.path_for("../etc/passwd").is_none());

        let keep: HashSet<String> = [a.entry.blake3.clone()].into_iter().collect();
        assert_eq!(store.gc(&keep).unwrap(), 1);
        assert!(store.path_for(&a.entry.blake3).unwrap().is_file());
        assert!(!store.path_for(&b.entry.blake3).unwrap().exists());
    }

    #[test]
    fn source_changed_during_copy_is_detected() {
        let data = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();
        let mut a = scanned(src.path(), "a.dll", b"aaa");
        a.entry.blake3 = hash::to_hex(&hash::hash_bytes(b"different"));
        let err = store.ingest(&[a]).unwrap_err();
        assert!(err.to_string().contains("changed"), "{err}");
    }
}
