//! The manifest: what a server wants installed on a player's machine.
//!
//! The server serializes it once, signs those exact bytes, and serves both.
//! The client verifies the signature on the raw bytes **before** parsing, so a
//! forged manifest never reaches serde. After parsing, [`Manifest::validate`]
//! applies every structural rule; a manifest with one bad entry is rejected
//! as a whole.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::MANIFEST_FORMAT;
use crate::error::{CoreError, Result};
use crate::hash;
use crate::limits::{Limits, human_bytes};
use crate::path::{self, AllowedRoots};
use crate::sign::PublicKey;

/// How the launcher treats a file that already exists on the player's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    /// Always replaced when it differs from the manifest.
    Enforce,
    /// Installed only if absent, then left alone (player preferences).
    Seed,
}

impl Policy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enforce => "enforce",
            Self::Seed => "seed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative to the game root, `/`-separated. See [`crate::path`].
    pub path: String,
    pub size: u64,
    /// Lowercase hex BLAKE3 digest; also the download key.
    pub blake3: String,
    pub policy: Policy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub server_name: String,
    /// `host:port` handed to the game (`+connect`).
    pub game_address: String,
    pub generated_at: String,
    /// `b3:` + BLAKE3 over the sorted entries. Unchanged means nothing to do.
    pub pack_id: String,
    /// Directories ValhSync owns: anything found there that is not in the
    /// manifest is an unmanaged file and gets quarantined.
    pub managed_roots: Vec<String>,
    /// Valheim's network version on the server, when it could be read.
    ///
    /// Matching mods are not enough: the game refuses a client whose network
    /// version differs, and the two Steam applications update separately, so a
    /// server left behind rejects everyone. Absent on manifests from before
    /// this field, and on servers whose log has not said yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_version: Option<u32>,
    pub files: Vec<FileEntry>,
}

/// Longest accepted `server_name`.
pub const MAX_SERVER_NAME: usize = 100;
/// Longest accepted `game_address`.
pub const MAX_GAME_ADDRESS: usize = 255;
/// Most `managed_roots` accepted.
pub const MAX_MANAGED_ROOTS: usize = 32;

/// `host:port` as handed to Valheim: hostname, IPv4 or bracketed IPv6, digits,
/// dots, dashes, underscores and colons. Anything else is refused so the value
/// can never smuggle shell or URL syntax into the launch command.
pub fn is_valid_game_address(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_GAME_ADDRESS
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'))
}

/// Is the host part of `host:port` a private, loopback or link-local address?
///
/// It matters because a Valheim server started with `-crossplay` relays every
/// connection through PlayFab and, in Iron Gate's own words, "it's not
/// possible to connect using a local IP address or a loopback IP address".
/// Such a server is only reachable through its **public** address or its join
/// code, even from the same LAN.
pub fn is_private_host(address: &str) -> bool {
    let host = address.rsplit_once(':').map_or(address, |(h, _)| h);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let Ok(ip) = host.parse::<std::net::IpAddr>() else {
        return false; // a host name: we cannot tell, assume it is fine
    };
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 unique local, fe80::/10 link local
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Display strings (server names) must not carry control characters: they end
/// up in terminals and logs, where escape sequences could forge output.
pub fn is_clean_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(is_deceptive)
}

/// Characters that make a string read as something other than what it is.
///
/// `char::is_control` is category Cc only, which covers the escape sequences
/// that could forge terminal output. It does not cover Cf -- the bidirectional
/// overrides and the zero-width joiners -- and those are what turn a name
/// ending in `txt.<RLO>lld` into something a player reads as a text file and
/// confirms. The plan screen is exactly where someone looks before agreeing to
/// a sync, so what is written there has to mean what it looks like.
#[must_use]
pub fn is_deceptive(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{200b}'..='\u{200f}'   // zero width, LRM, RLM
            | '\u{202a}'..='\u{202e}' // embeddings and overrides
            | '\u{2060}'..='\u{2064}' // word joiner, invisible operators
            | '\u{2066}'..='\u{2069}' // directional isolates
            | '\u{feff}'              // byte order mark
        )
}

fn sort_key(path: &str) -> (String, String) {
    (path.to_lowercase(), path.to_string())
}

impl Manifest {
    /// Build a manifest from scanned entries, sorting them and computing the pack id.
    pub fn new(
        server_name: impl Into<String>,
        game_address: impl Into<String>,
        managed_roots: Vec<String>,
        mut files: Vec<FileEntry>,
    ) -> Self {
        files.sort_by_cached_key(|f| sort_key(&f.path));
        let pack_id = Self::compute_pack_id(&files);
        Self {
            format: MANIFEST_FORMAT,
            server_name: server_name.into(),
            game_address: game_address.into(),
            generated_at: crate::clock::now_rfc3339(),
            pack_id,
            managed_roots,
            // Filled in by the publisher, which is the side that can read it.
            network_version: None,
            files,
        }
    }

    /// Note which Valheim this pack is meant for.
    #[must_use]
    pub fn with_network_version(mut self, version: Option<u32>) -> Self {
        self.network_version = version;
        self
    }

    /// Order-independent identity of the pack contents.
    pub fn compute_pack_id(files: &[FileEntry]) -> String {
        let mut sorted: Vec<&FileEntry> = files.iter().collect();
        sorted.sort_by_cached_key(|f| sort_key(&f.path));
        let mut hasher = blake3::Hasher::new();
        for f in sorted {
            hasher.update(f.path.as_bytes());
            hasher.update(b"\t");
            hasher.update(f.blake3.as_bytes());
            hasher.update(b"\t");
            hasher.update(f.size.to_string().as_bytes());
            hasher.update(b"\t");
            hasher.update(f.policy.as_str().as_bytes());
            hasher.update(b"\n");
        }
        format!("b3:{}", hasher.finalize().to_hex())
    }

    /// The bytes that get signed and served. Pretty-printed so an admin can
    /// read the published file; the signature covers this exact output.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>> {
        let mut copy = self.clone();
        copy.files.sort_by_cached_key(|f| sort_key(&f.path));
        let mut bytes = serde_json::to_vec_pretty(&copy)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Verify the signature over `bytes`, then parse, then validate.
    /// This is the only entry point a client should use.
    pub fn parse_verified(
        bytes: &[u8],
        signature_text: &str,
        key: &PublicKey,
        roots: &AllowedRoots,
        limits: &Limits,
    ) -> Result<Self> {
        key.verify_text(bytes, signature_text)?;
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate(roots, limits)?;
        Ok(manifest)
    }

    /// Apply every structural rule. Errors name the offending entry.
    pub fn validate(&self, roots: &AllowedRoots, limits: &Limits) -> Result<()> {
        let bad = |msg: String| Err(CoreError::InvalidManifest(msg));

        if self.format != MANIFEST_FORMAT {
            return bad(format!(
                "format {} is not supported by this launcher (expected {MANIFEST_FORMAT}); update ValhSync",
                self.format
            ));
        }
        if !is_clean_text(&self.server_name, MAX_SERVER_NAME) {
            return bad("server_name is empty, too long or contains control characters".into());
        }
        if !is_valid_game_address(&self.game_address) {
            return bad(
                "game_address must be host:port (letters, digits, '.', '-', '_', ':', '[', ']')"
                    .into(),
            );
        }
        if !crate::clock::is_rfc3339(&self.generated_at) {
            return bad(format!(
                "generated_at {:?} is not RFC 3339",
                self.generated_at
            ));
        }
        if self.managed_roots.len() > MAX_MANAGED_ROOTS {
            return bad("too many managed_roots".into());
        }
        for root in &self.managed_roots {
            let probe = format!("{root}/_");
            if let Err(reason) = path::validate(&probe, roots) {
                return bad(format!("managed root {root:?}: {reason}"));
            }
        }
        if self.files.len() > limits.max_files {
            return Err(CoreError::LimitExceeded(format!(
                "manifest lists {} files, maximum is {}",
                self.files.len(),
                limits.max_files
            )));
        }

        let mut seen: HashMap<String, &str> = HashMap::with_capacity(self.files.len());
        let mut total: u64 = 0;
        for f in &self.files {
            path::validate_or_err(&f.path, roots)?;
            if !hash::is_hex_hash(&f.blake3) {
                return bad(format!("{}: invalid blake3 digest", f.path));
            }
            if f.size > limits.max_file_bytes {
                return Err(CoreError::LimitExceeded(format!(
                    "{} is {}, maximum per file is {}",
                    f.path,
                    human_bytes(f.size),
                    human_bytes(limits.max_file_bytes)
                )));
            }
            total = total.saturating_add(f.size);
            if total > limits.max_pack_bytes {
                return Err(CoreError::LimitExceeded(format!(
                    "pack exceeds {}",
                    human_bytes(limits.max_pack_bytes)
                )));
            }
            // Windows and macOS filesystems are case-insensitive: two entries
            // differing only by case would fight over one file.
            if let Some(prev) = seen.insert(f.path.to_lowercase(), &f.path) {
                return bad(format!("duplicate path {:?} and {:?}", prev, f.path));
            }
        }

        let expected = Self::compute_pack_id(&self.files);
        if self.pack_id != expected {
            return bad("pack_id does not match the file list".into());
        }
        Ok(())
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Entries keyed by lowercase path, for case-insensitive lookups.
    pub fn by_lower_path(&self) -> HashMap<String, &FileEntry> {
        self.files
            .iter()
            .map(|f| (f.path.to_lowercase(), f))
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::sign::{Keypair, encode_signature};

    pub(crate) fn entry(path: &str, content: &[u8], policy: Policy) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            size: content.len() as u64,
            blake3: hash::to_hex(&hash::hash_bytes(content)),
            policy,
        }
    }

    pub(crate) fn sample() -> Manifest {
        Manifest::new(
            "Test server",
            "valheim.example.org:2456",
            vec!["BepInEx/plugins".into(), "BepInEx/patchers".into()],
            vec![
                entry("winhttp.dll", b"doorstop", Policy::Enforce),
                entry("BepInEx/plugins/Mod/Mod.dll", b"mod", Policy::Enforce),
                entry("BepInEx/config/Mod.cfg", b"cfg", Policy::Seed),
            ],
        )
    }

    fn roots() -> AllowedRoots {
        AllowedRoots::bepinex()
    }

    #[test]
    fn valid_manifest_passes() {
        sample().validate(&roots(), &Limits::default()).unwrap();
    }

    #[test]
    fn pack_id_is_order_independent() {
        let a = sample();
        let mut files = a.files.clone();
        files.reverse();
        assert_eq!(Manifest::compute_pack_id(&files), a.pack_id);
        let mut changed = a.files.clone();
        changed[0].size += 1;
        assert_ne!(Manifest::compute_pack_id(&changed), a.pack_id);
    }

    #[test]
    fn canonical_bytes_are_stable_and_sorted() {
        let m = sample();
        let mut shuffled = m.clone();
        shuffled.files.reverse();
        assert_eq!(
            m.to_canonical_bytes().unwrap(),
            shuffled.to_canonical_bytes().unwrap()
        );
        let parsed: Manifest = serde_json::from_slice(&m.to_canonical_bytes().unwrap()).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn signed_roundtrip_and_rejections() {
        let kp = Keypair::generate();
        let m = sample();
        let bytes = m.to_canonical_bytes().unwrap();
        let sig = encode_signature(&kp.sign(&bytes));
        let back =
            Manifest::parse_verified(&bytes, &sig, &kp.public(), &roots(), &Limits::default())
                .unwrap();
        assert_eq!(back, m);

        // one flipped byte in the body
        let mut tampered = bytes.clone();
        let i = tampered.len() / 2;
        tampered[i] ^= 0x01;
        assert!(matches!(
            Manifest::parse_verified(&tampered, &sig, &kp.public(), &roots(), &Limits::default()),
            Err(CoreError::BadSignature)
        ));

        // another server's key
        let other = Keypair::generate();
        assert!(matches!(
            Manifest::parse_verified(&bytes, &sig, &other.public(), &roots(), &Limits::default()),
            Err(CoreError::BadSignature)
        ));
    }

    #[test]
    fn signed_but_malicious_manifest_is_rejected_whole() {
        let kp = Keypair::generate();
        for evil in [
            "../../evil.dll",
            "C:/Windows/x.dll",
            "valheim.exe",
            "BepInEx/plugins/CON",
        ] {
            let mut m = sample();
            m.files.push(entry(evil, b"x", Policy::Enforce));
            m.pack_id = Manifest::compute_pack_id(&m.files);
            let bytes = m.to_canonical_bytes().unwrap();
            let sig = encode_signature(&kp.sign(&bytes));
            let err =
                Manifest::parse_verified(&bytes, &sig, &kp.public(), &roots(), &Limits::default())
                    .unwrap_err();
            assert!(
                matches!(err, CoreError::InvalidPath { .. }),
                "{evil}: {err}"
            );
        }
    }

    #[test]
    fn structural_rules() {
        let lim = Limits::default();

        let mut m = sample();
        m.format = 2;
        assert!(m.validate(&roots(), &lim).is_err());

        let mut m = sample();
        m.files.push(entry(
            "BepInEx/plugins/mod/MOD.DLL",
            b"dup",
            Policy::Enforce,
        ));
        m.pack_id = Manifest::compute_pack_id(&m.files);
        assert!(
            m.validate(&roots(), &lim)
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );

        let mut m = sample();
        m.files[0].blake3 = "ABC".into();
        m.pack_id = Manifest::compute_pack_id(&m.files);
        assert!(m.validate(&roots(), &lim).is_err());

        let mut m = sample();
        m.pack_id = "b3:0000".into();
        assert!(
            m.validate(&roots(), &lim)
                .unwrap_err()
                .to_string()
                .contains("pack_id")
        );

        let mut m = sample();
        m.managed_roots.push("valheim_Data".into());
        assert!(m.validate(&roots(), &lim).is_err());

        let mut m = sample();
        m.game_address = "host with space:2456".into();
        assert!(m.validate(&roots(), &lim).is_err());

        let mut m = sample();
        m.generated_at = "not a date".into();
        assert!(m.validate(&roots(), &lim).is_err());
    }

    #[test]
    fn game_address_and_name_are_strict() {
        for bad in [
            "host:2456&calc",
            "host:2456|x",
            "host:2456;x",
            "host 2456",
            "$(x):1",
            "h\u{1b}[31m:1",
            "",
        ] {
            assert!(!is_valid_game_address(bad), "{bad:?}");
        }
        for ok in [
            "valheim.example.org:2456",
            "192.168.1.50:2456",
            "[2001:db8::1]:2456",
            "my-server_1.net:2456",
        ] {
            assert!(is_valid_game_address(ok), "{ok}");
        }
        assert!(!is_clean_text("Evil\u{1b}[2J", 100));
        assert!(!is_clean_text("line\nbreak", 100));
        assert!(!is_clean_text("   ", 100));
        assert!(is_clean_text("Serveur de test", 100));
        let mut m = sample();
        m.server_name = "Spoof\u{1b}[1A".into();
        assert!(m.validate(&roots(), &Limits::default()).is_err());
    }

    #[test]
    fn private_hosts_are_recognised() {
        for p in [
            "192.168.1.50:2456",
            "10.0.0.5:2456",
            "172.16.4.1:2456",
            "127.0.0.1:2456",
            "localhost:2456",
            "[::1]:2456",
            "[fe80::1]:2456",
        ] {
            assert!(is_private_host(p), "{p}");
        }
        for p in [
            "203.0.113.10:2456",
            "valheim.example.org:2456",
            "[2001:db8::1]:2456",
            "8.8.8.8:2456",
        ] {
            assert!(!is_private_host(p), "{p}");
        }
    }

    #[test]
    fn limits_are_enforced() {
        let mut m = sample();
        m.files[0].size = 201 * 1024 * 1024;
        m.pack_id = Manifest::compute_pack_id(&m.files);
        assert!(matches!(
            m.validate(&roots(), &Limits::default()),
            Err(CoreError::LimitExceeded(_))
        ));

        let m = sample();
        let tiny = Limits::from_mib(200, 2048, 2);
        assert!(matches!(
            m.validate(&roots(), &tiny),
            Err(CoreError::LimitExceeded(_))
        ));

        let mut m = sample();
        for f in &mut m.files {
            f.size = 100 * 1024 * 1024;
        }
        m.pack_id = Manifest::compute_pack_id(&m.files);
        let small_pack = Limits::from_mib(200, 250, 5000);
        assert!(matches!(
            m.validate(&roots(), &small_pack),
            Err(CoreError::LimitExceeded(_))
        ));
    }
}

#[cfg(test)]
mod deception_tests {
    use super::*;

    /// A name that renders as one thing and is another is exactly how the
    /// plan screen gets someone to confirm something they did not read.
    #[test]
    fn bidi_and_zero_width_are_not_clean_text() {
        assert!(is_clean_text("Northwatch", 64));
        for bad in [
            "txt\u{202e}lld.exe",
            "North\u{200b}watch",
            "\u{2066}Northwatch\u{2069}",
            "North\u{feff}watch",
            "North\u{0007}watch",
        ] {
            assert!(!is_clean_text(bad, 64), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_path_cannot_render_as_another_name() {
        let roots = crate::path::AllowedRoots::bepinex();
        let ok = crate::path::validate("BepInEx/plugins/Mod/thing.dll", &roots);
        assert!(ok.is_ok(), "{ok:?}");
        let bad = crate::path::validate("BepInEx/plugins/Mod/txt\u{202e}lld.exe", &roots);
        assert!(bad.is_err(), "a right-to-left override should be refused");
    }
}
