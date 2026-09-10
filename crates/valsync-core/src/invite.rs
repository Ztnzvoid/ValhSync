//! Invite codes: `valsync1:` + base64url(JSON `{url, pubkey, name}`).
//!
//! Importing one pins the server's public key on the player's side. That is
//! the trust anchor of the whole system, so the code must be obtained from
//! the admin directly, not from an untrusted channel.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::manifest::MAX_SERVER_NAME;
use crate::sign::{PublicKey, b64_decode, b64_encode};

pub const PREFIX: &str = "valsync1:";
/// Longest accepted URL inside an invite.
pub const MAX_URL_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    /// Base URL of `valsync-server`, e.g. `http://valheim.example.org:2470`.
    pub url: String,
    /// Server public key, base64url.
    pub pubkey: String,
    /// Display name; the manifest's `server_name` takes over once fetched.
    pub name: String,
}

impl Invite {
    pub fn new(url: impl Into<String>, key: &PublicKey, name: impl Into<String>) -> Self {
        Self {
            url: normalize_url(&url.into()),
            pubkey: key.to_b64(),
            name: name.into(),
        }
    }

    pub fn encode(&self) -> Result<String> {
        let json = serde_json::to_vec(self)?;
        Ok(format!("{PREFIX}{}", b64_encode(&json)))
    }

    /// Parse and validate. Whitespace around the code is tolerated because
    /// players paste it from chat apps.
    pub fn parse(text: &str) -> Result<Self> {
        let bad = |m: &str| CoreError::InvalidInvite(m.to_string());
        let trimmed = text.trim();
        let body = trimmed.strip_prefix(PREFIX).ok_or_else(|| {
            bad("it should start with \"valsync1:\"; check that the whole code was copied")
        })?;
        let json =
            b64_decode(body).ok_or_else(|| bad("the code is corrupted (not valid base64)"))?;
        let mut invite: Self = serde_json::from_slice(&json)
            .map_err(|_| bad("the code is corrupted (bad payload)"))?;
        invite.url = normalize_url(&invite.url);
        invite.validate()?;
        Ok(invite)
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| CoreError::InvalidInvite(m.to_string());
        if self.url.len() > MAX_URL_LEN {
            return Err(bad("server URL is too long"));
        }
        let rest = self
            .url
            .strip_prefix("http://")
            .or_else(|| self.url.strip_prefix("https://"))
            .ok_or_else(|| bad("server URL must start with http:// or https://"))?;
        let host = rest.split('/').next().unwrap_or("");
        if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c == '@') {
            return Err(bad("server URL has no valid host"));
        }
        if !crate::manifest::is_clean_text(&self.name, MAX_SERVER_NAME) {
            return Err(bad(
                "server name is empty, too long or contains control characters",
            ));
        }
        self.public_key()?;
        Ok(())
    }

    pub fn public_key(&self) -> Result<PublicKey> {
        PublicKey::from_b64(&self.pubkey)
            .map_err(|e| CoreError::InvalidInvite(format!("bad public key: {e}")))
    }

    /// Stable identifier of a server across renames and URL changes: its key.
    pub fn server_id(&self) -> String {
        self.pubkey.clone()
    }
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign::Keypair;

    #[test]
    fn roundtrip() {
        let kp = Keypair::generate();
        let inv = Invite::new(
            "http://valheim.example.org:2470/",
            &kp.public(),
            "Serveur de Ztnzvoid",
        );
        let code = inv.encode().unwrap();
        assert!(code.starts_with(PREFIX));
        let back = Invite::parse(&format!("  {code}\n")).unwrap();
        assert_eq!(back, inv);
        assert_eq!(back.url, "http://valheim.example.org:2470");
        assert_eq!(back.public_key().unwrap(), kp.public());
    }

    #[test]
    fn rejects_garbage() {
        assert!(Invite::parse("").is_err());
        assert!(Invite::parse("valsync2:abc").is_err());
        assert!(Invite::parse("valsync1:!!!").is_err());
        assert!(Invite::parse(&format!("{PREFIX}{}", b64_encode(b"{}"))).is_err());

        let kp = Keypair::generate();
        for url in [
            "ftp://x",
            "valheim.example.org:2470",
            "http://",
            "http://user@host",
        ] {
            let inv = Invite::new(url, &kp.public(), "n");
            assert!(inv.validate().is_err(), "{url}");
        }
        assert!(
            Invite::new("http://h", &kp.public(), "x\u{1b}[0m")
                .validate()
                .is_err()
        );
        let mut inv = Invite::new("http://h", &kp.public(), "n");
        inv.pubkey = "AAAA".into();
        assert!(Invite::parse(&inv.encode().unwrap()).is_err());
    }
}
