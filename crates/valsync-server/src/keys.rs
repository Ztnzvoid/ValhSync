//! The server's Ed25519 signing key, stored under the data directory.
//!
//! `keys/server.key` holds the secret (base64url). Losing it means every
//! player has to import a new invite code; leaking it means anyone can sign a
//! manifest for this server. It is created with owner-only permissions where
//! the OS supports that.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use valsync_core::Keypair;

pub fn key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("keys").join("server.key")
}

pub fn pub_path(data_dir: &Path) -> PathBuf {
    data_dir.join("keys").join("server.pub")
}

pub fn load(data_dir: &Path) -> Result<Keypair> {
    let path = key_path(data_dir);
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "no signing key at {}; run `valsync-server init` first",
            path.display()
        )
    })?;
    Keypair::from_secret_b64(&text).with_context(|| format!("{} is corrupted", path.display()))
}

/// Returns the keypair and whether it was just created.
pub fn load_or_create(data_dir: &Path) -> Result<(Keypair, bool)> {
    if key_path(data_dir).is_file() {
        return Ok((load(data_dir)?, false));
    }
    let kp = Keypair::generate();
    write(data_dir, &kp)?;
    Ok((kp, true))
}

/// Replace the key. The old one is kept next to it, renamed, in case the
/// rotation was a mistake.
pub fn rotate(data_dir: &Path) -> Result<Keypair> {
    let path = key_path(data_dir);
    if !path.is_file() {
        bail!("no key to rotate; run `valsync-server init` first");
    }
    let backup = path.with_extension(format!("key.old-{}", valsync_core::clock::dir_stamp()));
    std::fs::rename(&path, &backup)
        .with_context(|| format!("cannot move old key to {}", backup.display()))?;
    let kp = Keypair::generate();
    write(data_dir, &kp)?;
    Ok(kp)
}

fn write(data_dir: &Path, kp: &Keypair) -> Result<()> {
    let path = key_path(data_dir);
    let dir = path.parent().context("key path has no parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    write_private(&path, format!("{}\n", kp.secret_b64()).as_bytes())?;
    std::fs::write(pub_path(data_dir), format!("{}\n", kp.public().to_b64()))
        .with_context(|| format!("cannot write {}", pub_path(data_dir).display()))?;
    Ok(())
}

fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    let tmp = path.with_extension("key.tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(&tmp)
            .with_context(|| format!("cannot create {}", tmp.display()))?;
        std::io::Write::write_all(&mut f, contents)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_load_rotate() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load(tmp.path()).is_err());
        let (kp, created) = load_or_create(tmp.path()).unwrap();
        assert!(created);
        let (again, created) = load_or_create(tmp.path()).unwrap();
        assert!(!created);
        assert_eq!(kp.public(), again.public());
        assert_eq!(
            std::fs::read_to_string(pub_path(tmp.path()))
                .unwrap()
                .trim(),
            kp.public().to_b64()
        );
        let rotated = rotate(tmp.path()).unwrap();
        assert_ne!(rotated.public(), kp.public());
        assert_eq!(load(tmp.path()).unwrap().public(), rotated.public());
        let olds: Vec<_> = std::fs::read_dir(tmp.path().join("keys"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains("old-"))
            .collect();
        assert_eq!(olds.len(), 1);
    }
}
