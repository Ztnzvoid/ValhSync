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

/// Long enough for a slow handshake, short enough that a server which has gone
/// away is reported as away rather than waited on.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Ceiling for the small control requests. A reboot leaves NAT rules that
/// accept the connection and then answer nothing: `connect_timeout` does not
/// cover that, only this does. The launcher re-checks every 25s, so anything
/// longer than that is a check the player watches instead of a status.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(15);

/// Ceiling for one file download.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(900);

#[derive(Debug, Clone)]
pub struct Client {
    /// Downloads: one file can be large and slow, so the ceiling is high.
    inner: reqwest::blocking::Client,
    /// Key, health, manifest and signature. These are a few kilobytes and
    /// answer immediately or not at all, so they get a short leash.
    control: reqwest::blocking::Client,
}

fn unreachable(url: &str, e: reqwest::Error) -> SyncError {
    SyncError::Unreachable {
        url: url.to_string(),
        reason: e.without_url().to_string(),
    }
}

impl Client {
    pub fn new() -> Result<Self> {
        Self::with_timeouts(CONTROL_TIMEOUT, DOWNLOAD_TIMEOUT)
    }

    fn with_timeouts(control: Duration, download: Duration) -> Result<Self> {
        let base = || {
            reqwest::blocking::Client::builder()
                .user_agent(concat!("valhsync/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(CONNECT_TIMEOUT)
                // A redirect can only lead to another http(s) URL, and not far.
                .redirect(reqwest::redirect::Policy::limited(3))
        };
        let make = |b: reqwest::blocking::ClientBuilder| {
            b.build()
                .map_err(|e| SyncError::Other(format!("cannot create HTTP client: {e}")))
        };
        Ok(Self {
            // Whole-request ceiling: generous enough for a 200 MiB file on a
            // slow link, finite so a stalled connection cannot hang the launcher.
            inner: make(base().timeout(download))?,
            control: make(base().timeout(control))?,
        })
    }

    fn get(
        client: &reqwest::blocking::Client,
        url: &str,
        what: &str,
    ) -> Result<reqwest::blocking::Response> {
        let resp = client.get(url).send().map_err(|e| unreachable(url, e))?;
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
        let mut resp = Self::get(&self.control, url, what)?;
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

        let mut resp = Self::get(&self.inner, &url, &entry.path)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Instant;

    /// A port forward that outlived the machine behind it accepts the
    /// connection and then answers nothing. `connect_timeout` does not cover
    /// that: the handshake succeeded. The launcher used to wait out the whole
    /// download ceiling on it, with the automatic re-check blocked behind the
    /// job the whole time -- fifteen minutes of "contacting the server" for a
    /// server that was already back up.
    #[test]
    fn a_server_that_accepts_and_then_says_nothing_is_given_up_on() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            // Hold every accepted socket open, and answer none of them.
            let mut held = Vec::new();
            while let Ok((sock, _)) = listener.accept() {
                held.push(sock);
            }
        });

        let client = Client::with_timeouts(Duration::from_millis(300), DOWNLOAD_TIMEOUT).unwrap();
        let start = Instant::now();
        let err = client.fetch_key(&format!("http://{addr}")).unwrap_err();
        let waited = start.elapsed();

        assert!(
            waited < Duration::from_secs(30),
            "the control request used the download ceiling: waited {waited:?}"
        );
        assert!(
            engine_sees_it_as_offline(&err),
            "a stalled server should read as unreachable, got {err:?}"
        );
    }

    fn engine_sees_it_as_offline(e: &SyncError) -> bool {
        matches!(e, SyncError::Unreachable { .. })
    }

    /// The two ceilings exist for different things; keeping them apart is the
    /// whole point of the fix.
    #[test]
    fn control_requests_are_not_given_the_download_ceiling() {
        assert!(CONTROL_TIMEOUT < DOWNLOAD_TIMEOUT);
        assert!(CONTROL_TIMEOUT > CONNECT_TIMEOUT);
    }
}
