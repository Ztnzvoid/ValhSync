//! Invite codes: `valhsync1:` + base64url(JSON `{url, pubkey, name}`).
//!
//! Importing one pins the server's public key on the player's side. That is
//! the trust anchor of the whole system, so the code must be obtained from
//! the admin directly, not from an untrusted channel.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::manifest::MAX_SERVER_NAME;
use crate::sign::{PublicKey, b64_decode, b64_encode};

pub const PREFIX: &str = "valhsync1:";
/// Longest accepted URL inside an invite.
pub const MAX_URL_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    /// Base URL of `valhsync-server`, e.g. `http://valheim.example.org:2470`.
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
            bad("it should start with \"valhsync1:\"; check that the whole code was copied")
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

/// Default port of `valhsync-server`.
pub const DEFAULT_PORT: u16 = 2470;

/// Turn what an admin types or dictates into a base URL: `valheim.example.org`,
/// `1.2.3.4:2470` and `https://pack.example.org/valheim` all work.
///
/// Returns `None` for something that cannot be a host at all.
pub fn address_to_url(input: &str) -> Option<String> {
    let text = input.trim();
    let (has_scheme, body) = match text
        .strip_prefix("http://")
        .or_else(|| text.strip_prefix("https://"))
    {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let body = body.trim_end_matches('/');
    if body.is_empty()
        || body.contains(char::is_whitespace)
        || body.contains(char::is_control)
        || body.contains('@')
        || body.contains("://")
    {
        return None;
    }
    let (authority, path) = body.split_once('/').map_or((body, ""), |(a, p)| (a, p));
    if authority.is_empty() {
        return None;
    }
    let scheme = if text.starts_with("https://") {
        "https"
    } else {
        "http"
    };
    // A bare host gets the default port; anything with a port or a path is
    // taken as the admin wrote it.
    if !has_scheme && path.is_empty() && !authority.contains(':') {
        return Some(format!("http://{authority}:{DEFAULT_PORT}"));
    }
    Some(format!("{scheme}://{body}"))
}

/// The same host on ValhSync's own port.
///
/// Players are given the address of the *game* server, which is the one they
/// are told about, and they type it here. Nothing but ValhSync ever answers on
/// [`DEFAULT_PORT`], so trying it once is unambiguous. Returns `None` when
/// there is nothing to try: the port is already the default, or the URL names
/// a path, which means a static export rather than a live server.
#[must_use]
pub fn on_default_port(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if rest.contains('/') {
        return None;
    }
    let host = rest.split_once(':').map_or(rest, |(h, _)| h);
    if host.is_empty() {
        return None;
    }
    let candidate = format!("{scheme}://{host}:{DEFAULT_PORT}");
    (candidate != url).then_some(candidate)
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
            "Serveur de test",
        );
        let code = inv.encode().unwrap();
        assert!(code.starts_with(PREFIX));
        let back = Invite::parse(&format!("  {code}\n")).unwrap();
        assert_eq!(back, inv);
        assert_eq!(back.url, "http://valheim.example.org:2470");
        assert_eq!(back.public_key().unwrap(), kp.public());
    }

    #[test]
    fn the_game_port_is_retried_on_ours() {
        assert_eq!(
            on_default_port("http://203.0.113.10:2456").as_deref(),
            Some("http://203.0.113.10:2470")
        );
        assert_eq!(
            on_default_port("http://valheim.example.org").as_deref(),
            Some("http://valheim.example.org:2470")
        );
        // Nothing to try: already ours, or a static export under a path.
        assert_eq!(on_default_port("http://203.0.113.10:2470"), None);
        assert_eq!(on_default_port("https://you.github.io/pack"), None);
        assert_eq!(on_default_port("not a url"), None);
    }

    #[test]
    fn addresses_become_urls() {
        assert_eq!(
            address_to_url("valheim.example.org").as_deref(),
            Some("http://valheim.example.org:2470")
        );
        assert_eq!(
            address_to_url(" 203.0.113.10:2470/ ").as_deref(),
            Some("http://203.0.113.10:2470")
        );
        assert_eq!(
            address_to_url("https://you.github.io/pack").as_deref(),
            Some("https://you.github.io/pack")
        );
        assert_eq!(
            address_to_url("http://host").as_deref(),
            Some("http://host")
        );
        for bad in [
            "",
            "   ",
            "http://",
            "a b",
            "user@host",
            "ftp://x",
            // No credentials, no second scheme, no control characters.
            "user:pass@host",
            "http://a@b",
            "host\u{0}x",
            "http://ho st",
        ] {
            assert_eq!(address_to_url(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(Invite::parse("").is_err());
        assert!(Invite::parse("valhsync2:abc").is_err());
        assert!(Invite::parse("valhsync1:!!!").is_err());
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
