//! Known servers and their pinned keys (`servers.json`).
//!
//! A server's identity is its public key. The URL and the name can change; the
//! key cannot without the player importing a fresh invite explicitly.

use serde::{Deserialize, Serialize};
use valsync_core::{Invite, PublicKey};

use crate::error::{Result, SyncError};
use crate::paths::{AppPaths, read_json, write_json_atomic};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownServer {
    /// Public key, base64url. Stable identity.
    pub id: String,
    pub name: String,
    pub url: String,
    pub added_at: String,
    /// Pack id seen at the last successful sync, for the status line.
    #[serde(default)]
    pub last_pack_id: Option<String>,
}

impl KnownServer {
    pub fn public_key(&self) -> Result<PublicKey> {
        Ok(PublicKey::from_b64(&self.id)?)
    }

    pub fn fingerprint(&self) -> String {
        self.public_key()
            .map_or_else(|_| "invalid-key".into(), |k| k.fingerprint())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerBook {
    pub servers: Vec<KnownServer>,
    /// Id of the server used when none is named.
    pub default: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOutcome {
    Added,
    /// Same key, same URL: nothing to do.
    AlreadyKnown,
    /// Same key, new URL or name: updated in place.
    Updated,
    /// A server with this URL had another key; replaced because the caller
    /// asked for it.
    KeyReplaced,
}

impl ServerBook {
    pub fn load(paths: &AppPaths) -> Result<Self> {
        Ok(read_json(&paths.servers_file())?.unwrap_or_default())
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        write_json_atomic(&paths.servers_file(), self)
    }

    /// Import an invite. Refuses to silently replace a pinned key: a server
    /// already known under the same URL with a different key is an error
    /// unless `replace_key` is set.
    pub fn join(&mut self, invite: &Invite, replace_key: bool) -> Result<JoinOutcome> {
        invite.validate()?;
        let id = invite.server_id();

        if let Some(existing) = self.servers.iter_mut().find(|s| s.id == id) {
            if existing.url == invite.url && existing.name == invite.name {
                return Ok(JoinOutcome::AlreadyKnown);
            }
            existing.url.clone_from(&invite.url);
            existing.name.clone_from(&invite.name);
            return Ok(JoinOutcome::Updated);
        }

        let same_url = self
            .servers
            .iter()
            .position(|s| s.url.eq_ignore_ascii_case(&invite.url));
        let mut outcome = JoinOutcome::Added;
        if let Some(i) = same_url {
            if !replace_key {
                let old = &self.servers[i];
                return Err(SyncError::KeyMismatch {
                    name: old.name.clone(),
                    pinned: old.fingerprint(),
                    received: invite
                        .public_key()
                        .map(|k| k.fingerprint())
                        .unwrap_or_default(),
                });
            }
            let old = self.servers.remove(i);
            if self.default.as_deref() == Some(&old.id) {
                self.default = None;
            }
            outcome = JoinOutcome::KeyReplaced;
        }

        self.servers.push(KnownServer {
            id: id.clone(),
            name: invite.name.clone(),
            url: invite.url.clone(),
            added_at: valsync_core::clock::now_rfc3339(),
            last_pack_id: None,
        });
        if self.default.is_none() {
            self.default = Some(id);
        }
        Ok(outcome)
    }

    pub fn remove(&mut self, selector: &str) -> Result<KnownServer> {
        let id = self.resolve(Some(selector))?.id.clone();
        let i = self
            .servers
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| SyncError::UnknownServer(selector.into()))?;
        let removed = self.servers.remove(i);
        if self.default.as_deref() == Some(&removed.id) {
            self.default = self.servers.first().map(|s| s.id.clone());
        }
        Ok(removed)
    }

    /// Find a server by name (case-insensitive), by key prefix, or fall back
    /// to the default / only server.
    pub fn resolve(&self, selector: Option<&str>) -> Result<&KnownServer> {
        if self.servers.is_empty() {
            return Err(SyncError::NoServer);
        }
        if let Some(sel) = selector {
            let sel_trim = sel.trim();
            return self
                .servers
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(sel_trim))
                .or_else(|| {
                    (sel_trim.len() >= 6)
                        .then(|| self.servers.iter().find(|s| s.id.starts_with(sel_trim)))
                        .flatten()
                })
                .ok_or_else(|| SyncError::UnknownServer(sel_trim.to_string()));
        }
        if let Some(id) = &self.default
            && let Some(s) = self.servers.iter().find(|s| &s.id == id)
        {
            return Ok(s);
        }
        if self.servers.len() == 1 {
            return Ok(&self.servers[0]);
        }
        Err(SyncError::AmbiguousServer(
            self.servers
                .iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        ))
    }

    pub fn set_default(&mut self, id: &str) {
        self.default = Some(id.to_string());
    }

    pub fn note_pack(&mut self, id: &str, pack_id: &str) {
        if let Some(s) = self.servers.iter_mut().find(|s| s.id == id) {
            s.last_pack_id = Some(pack_id.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use valsync_core::Keypair;

    fn invite(url: &str, name: &str, kp: &Keypair) -> Invite {
        Invite::new(url, &kp.public(), name)
    }

    #[test]
    fn join_resolve_remove() {
        let kp = Keypair::generate();
        let mut book = ServerBook::default();
        assert!(matches!(book.resolve(None), Err(SyncError::NoServer)));

        let inv = invite("http://a:2470", "Alpha", &kp);
        assert_eq!(book.join(&inv, false).unwrap(), JoinOutcome::Added);
        assert_eq!(book.join(&inv, false).unwrap(), JoinOutcome::AlreadyKnown);
        assert_eq!(book.resolve(None).unwrap().name, "Alpha");
        assert_eq!(book.resolve(Some("alpha")).unwrap().name, "Alpha");

        let moved = invite("http://b:2470", "Alpha", &kp);
        assert_eq!(book.join(&moved, false).unwrap(), JoinOutcome::Updated);
        assert_eq!(book.servers[0].url, "http://b:2470");

        // Same URL, different key: refused unless explicitly replaced.
        let other = Keypair::generate();
        let impostor = invite("http://b:2470", "Alpha", &other);
        assert!(matches!(
            book.join(&impostor, false),
            Err(SyncError::KeyMismatch { .. })
        ));
        assert_eq!(
            book.join(&impostor, true).unwrap(),
            JoinOutcome::KeyReplaced
        );
        assert_eq!(book.servers.len(), 1);
        assert_eq!(book.servers[0].id, other.public().to_b64());

        let third = Keypair::generate();
        book.join(&invite("http://c", "Gamma", &third), false)
            .unwrap();
        assert_eq!(book.resolve(None).unwrap().name, "Alpha", "default sticks");
        assert!(matches!(
            book.resolve(Some("nope")),
            Err(SyncError::UnknownServer(_))
        ));
        book.remove("Alpha").unwrap();
        assert_eq!(book.resolve(None).unwrap().name, "Gamma");
    }
}
