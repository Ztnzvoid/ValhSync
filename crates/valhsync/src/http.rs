//! Talking to `valhsync-server`. Blocking client: the CLI is sequential and the
//! window runs syncs on a worker thread.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use valhsync_core::{AllowedRoots, FileEntry, Limits, Manifest, PublicKey, hash};

use crate::error::{Result, SyncError};

/// A manifest bigger than this is refused before parsing.
pub const MANIFEST_MAX_BYTES: u64 = 8 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct Client {
    inner: reqwest::blocking::Client,
}

fn unreachable(url: &str, e: reqwest::Error) -> SyncError {
    SyncError::Unreachable {
        url: url.to_string(),
        reason: e.without_url().to_string(),
    }
}

impl Client {
    pub fn new() -> Result<Self> {
        let inner = reqwest::blocking::Client::builder()
            .user_agent(concat!("valhsync/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            // A redirect can only lead to another http(s) URL, and not far.
            .redirect(reqwest::redirect::Policy::limited(3))
            // Whole-request ceiling: generous enough for a 200 MiB file on a
            // slow link, finite so a stalled connection cannot hang the launcher.
            .timeout(Duration::from_secs(900))
            .build()
            .map_err(|e| SyncError::Other(format!("cannot create HTTP client: {e}")))?;
        Ok(Self { inner })
    }

    fn get(&self, url: &str, what: &str) -> Result<reqwest::blocking::Response> {
        let resp = self
            .inner
            .get(url)
            .send()
            .map_err(|e| unreachable(url, e))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(SyncError::HttpStatus {
                url: url.to_string(),
                status: status.as_u16(),
                what: what.to_string(),
            });
        }
        Ok(resp)
    }

    /// Fetch at most `max` bytes; more is an error, not a truncation.
    fn get_limited(&self, url: &str, what: &str, max: u64) -> Result<Vec<u8>> {
        let mut resp = self.get(url, what)?;
        if resp.content_length().is_some_and(|len| len > max) {
            return Err(SyncError::Other(format!("{what} is unexpectedly large")));
        }
        let mut out = Vec::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = resp
                .read(&mut buf)
                .map_err(|e| SyncError::Other(format!("reading {what}: {e}")))?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            if out.len() as u64 > max {
                return Err(SyncError::Other(format!("{what} is unexpectedly large")));
            }
        }
        Ok(out)
    }

    /// The public key a server publishes at `/key`. Fetching it is not proof
    /// of identity: the player still has to compare the fingerprint with what
    /// the admin told them. It only lets us verify the manifest afterwards.
    pub fn fetch_key(&self, base_url: &str) -> Result<PublicKey> {
        let base = base_url.trim_end_matches('/');
        let bytes = self.get_limited(&format!("{base}/key"), "the server key", 4096)?;
        let text = String::from_utf8_lossy(&bytes);
        PublicKey::from_b64(text.trim()).map_err(|_| {
            SyncError::Other(format!(
                "{base}/key did not answer with a public key; is this a ValhSync server?"
            ))
        })
    }

    /// What the publisher says about the game server, if it is in a position
    /// to know. Unsigned and purely informational: it decides what a label
    /// says, never what gets installed.
    pub fn fetch_game_server_state(&self, base_url: &str) -> Option<bool> {
        let base = base_url.trim_end_matches('/');
        let bytes = self
            .get_limited(&format!("{base}/health"), "the server status", 8192)
            .ok()?;
        let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        match json.get("game_server")?.as_str()? {
            "running" => Some(true),
            "stopped" => Some(false),
            _ => None,
        }
    }

    /// Manifest and signature, verified against the pinned key before parsing.
    pub fn fetch_manifest(
        &self,
        base_url: &str,
        key: &PublicKey,
        roots: &AllowedRoots,
        limits: &Limits,
    ) -> Result<(Manifest, Vec<u8>)> {
        let base = base_url.trim_end_matches('/');
        let bytes = self.get_limited(
            &format!("{base}/manifest.json"),
            "the manifest",
            MANIFEST_MAX_BYTES,
        )?;
        let sig = self.get_limited(
            &format!("{base}/manifest.sig"),
            "the manifest signature",
            4096,
        )?;
        let sig = String::from_utf8_lossy(&sig).trim().to_string();
        let manifest = Manifest::parse_verified(&bytes, &sig, key, roots, limits)?;
        Ok((manifest, bytes))
    }

    /// Download one file to `dest`, refusing to read past the announced size,
    /// then verify its digest. `on_progress` receives bytes received so far.
    pub fn download(
        &self,
        base_url: &str,
        entry: &FileEntry,
        dest: &Path,
        on_progress: &mut dyn FnMut(u64),
    ) -> Result<()> {
        let base = base_url.trim_end_matches('/');
        let url = format!("{base}/files/{}", entry.blake3);
        let fail = |reason: String| SyncError::Download {
            path: entry.path.clone(),
            reason,
        };

        let mut resp = self.get(&url, &entry.path)?;
        if resp.content_length().is_some_and(|len| len != entry.size) {
            return Err(fail(format!(
                "server announces {} bytes, manifest says {}",
                resp.content_length().unwrap_or(0),
                entry.size
            )));
        }

        if let Some(parent) = dest.parent() {
            SyncError::at(parent, "cannot create", std::fs::create_dir_all(parent))?;
        }
        let mut file = SyncError::at(dest, "cannot create", std::fs::File::create(dest))?;
        let mut buf = vec![0u8; CHUNK];
        let mut received: u64 = 0;
        loop {
            let n = resp
                .read(&mut buf)
                .map_err(|e| fail(format!("connection lost: {e}")))?;
            if n == 0 {
                break;
            }
            received += n as u64;
            if received > entry.size {
                drop(file);
                let _ = std::fs::remove_file(dest);
                return Err(fail(
                    "server sent more bytes than the manifest announced".into(),
                ));
            }
            file.write_all(&buf[..n])
                .map_err(|e| SyncError::io(format!("writing {}", dest.display()), e))?;
            on_progress(received);
        }
        file.sync_all()
            .map_err(|e| SyncError::io(format!("flushing {}", dest.display()), e))?;
        drop(file);

        if received != entry.size {
            let _ = std::fs::remove_file(dest);
            return Err(fail(format!(
                "incomplete: {received} of {} bytes",
                entry.size
            )));
        }
        if let Err(e) = hash::verify_file(dest, &entry.blake3, &entry.path) {
            let _ = std::fs::remove_file(dest);
            return Err(fail(format!(
                "content does not match its digest ({e}); the file was corrupted in transit or on the server"
            )));
        }
        Ok(())
    }
}
