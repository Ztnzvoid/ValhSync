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

    /// Parse and validate.
    ///
    /// The text is what the player pasted, so it may be a whole message with
    /// the code somewhere inside it, and the code itself may have been wrapped
    /// across lines on the way. [`extract`] puts it back together.
    pub fn parse(text: &str) -> Result<Self> {
        let bad = |m: &str| CoreError::InvalidInvite(m.to_string());
        let body = extract(text).ok_or_else(|| {
            bad("it should start with \"valhsync1:\"; check that the whole code was copied")
        })?;
        let json =
            b64_decode(&body).ok_or_else(|| bad("the code is corrupted (not valid base64)"))?;
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

/// The body of the invite code inside `text`, whatever surrounds it.
///
/// Chat apps, mail clients and text editors wrap a two-hundred character code
/// onto several lines, and a player pastes the message rather than the code.
/// So: find the line holding the prefix, then keep joining the lines that
/// follow for as long as they are nothing but more of the code. Prose after
/// the code stops the run, and the whitespace inside it disappears.
#[must_use]
pub fn extract(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim);
    let first = lines.find_map(|l| l.split_once(PREFIX).map(|(_, rest)| rest))?;

    let mut body = String::from(first);
    for line in lines {
        if line.is_empty() || !line.chars().all(is_code_char) {
            break;
        }
        body.push_str(line);
    }
    body.retain(|c| !c.is_whitespace());
    Some(body)
}

/// The base64url alphabet the code is written in, padding included.
fn is_code_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '=')
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// The port ValhSync serves on by default: the game's own.
///
/// Valheim uses it in UDP only, so nothing collides, and a router rule that
/// covers TCP+UDP -- which is what most of them write -- already carries it.
/// Asking an admin to open a second port for a mod list is not something this
/// project does.
pub const DEFAULT_PORT: u16 = 2456;

/// The port earlier versions used, and which a hosted setup may still be on.
pub const LEGACY_PORT: u16 = 2470;

/// Is this Valheim's own join code rather than an address?
///
/// A crossplay server shows a six-digit code, and it is the thing a player is
/// most likely to be handed, so it is the thing they will paste here. It
/// cannot work: that code reaches the *game* through PlayFab's relay, which
/// carries no file transfer and no HTTP. Recognising it is the difference
/// between a useless DNS error and an answer.
#[must_use]
pub fn looks_like_join_code(text: &str) -> bool {
    let t = text.trim();
    (4..=8).contains(&t.len()) && t.chars().all(|c| c.is_ascii_digit())
}

/// Turn what an admin types or dictates into a base URL: `valheim.example.org`,
/// `1.2.3.4:2456` and `https://pack.example.org/valheim` all work.
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

/// The same host on the other port worth trying.
///
/// Players type the address of the *game* server, because that is the one they
/// were given, and ValhSync now answers on that very port. When it does not,
/// the server is probably still on [`LEGACY_PORT`], so try that once before
/// giving up -- and the other way round, for a player who was handed a
/// `:2470` address for a server that has since moved.
///
/// Returns `None` when there is nothing to try: the URL names a path, which
/// means a static export rather than a live server.
#[must_use]
pub fn other_port(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if rest.contains('/') {
        return None;
    }
    let (host, port) = rest
        .split_once(':')
        .map_or((rest, None), |(h, p)| (h, p.parse().ok()));
    if host.is_empty() {
        return None;
    }
    let other = match port {
        Some(LEGACY_PORT) => DEFAULT_PORT,
        _ => LEGACY_PORT,
    };
    let candidate = format!("{scheme}://{host}:{other}");
    (candidate != url).then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        let key = crate::Keypair::generate();
        Invite::new("http://pack.example.org:2470", &key.public(), "Midgard")
            .encode()
            .unwrap()
    }

    #[test]
    fn a_code_wrapped_on_the_way_still_reads() {
        let code = sample();
        let (head, tail) = code.split_at(70);

        for text in [
            // As sent.
            code.clone(),
            // Wrapped by a chat app or a mail client.
            format!("{head}\n{tail}"),
            format!("{head}\r\n{tail}\r\n"),
            // Wrapped and pasted with the message around it.
            format!("Salut, voici le code :\n\n{head}\n{tail}\n\nA plus !"),
            // Indented by a quote marker's worth of spaces.
            format!("   {head}\n   {tail}   "),
        ] {
            let invite = Invite::parse(&text).unwrap_or_else(|e| panic!("{text:?}: {e}"));
            assert_eq!(invite.name, "Midgard");
            assert_eq!(invite.url, "http://pack.example.org:2470");
        }
    }

    #[test]
    fn prose_after_the_code_is_not_swallowed() {
        let code = sample();
        let text = format!("{code}\nvoila, colle ca dans le launcher");
        assert_eq!(Invite::parse(&text).unwrap().name, "Midgard");
    }

    #[test]
    fn a_truncated_code_is_still_refused() {
        let code = sample();
        let err = Invite::parse(&code[..code.len() - 12])
            .unwrap_err()
            .to_string();
        assert!(err.contains("corrupted"), "{err}");

        let err = Invite::parse("no code here at all")
            .unwrap_err()
            .to_string();
        assert!(err.contains("valhsync1:"), "{err}");
    }
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
    fn the_old_port_is_retried_and_the_new_one_too() {
        // The address a player types is the game's, which is where ValhSync
        // now listens; a server still on the old port answers there.
        assert_eq!(
            other_port("http://203.0.113.10:2456").as_deref(),
            Some("http://203.0.113.10:2470")
        );
        // And the other way round, for an address handed out long ago.
        assert_eq!(
            other_port("http://203.0.113.10:2470").as_deref(),
            Some("http://203.0.113.10:2456")
        );
        assert_eq!(
            other_port("http://valheim.example.org").as_deref(),
            Some("http://valheim.example.org:2470")
        );
        // Nothing to try: a static export lives under a path.
        assert_eq!(other_port("https://you.github.io/pack"), None);
        assert_eq!(other_port("not a url"), None);
    }

    #[test]
    fn a_valheim_join_code_is_not_an_address() {
        for code in ["236486", "024150", "1234", "12345678"] {
            assert!(looks_like_join_code(code), "{code}");
        }
        for other in [
            "203.0.113.10",
            "203.0.113.10:2456",
            "valheim.example.org",
            "123456789",
            "",
        ] {
            assert!(!looks_like_join_code(other), "{other}");
        }
    }

    #[test]
    fn addresses_become_urls() {
        assert_eq!(
            address_to_url("valheim.example.org").as_deref(),
            Some("http://valheim.example.org:2456")
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
