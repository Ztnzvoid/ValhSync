//! What a server offers as a newer ValhSync build.
//!
//! An update channel is the sharpest edge in a launcher: whoever decides what
//! bytes land there decides what runs on the player's machine. So an offer is
//! not a URL and not a version string -- it is a document signed by the same
//! Ed25519 key the player already pinned for the mod pack, naming a build by
//! its BLAKE3 digest. A server that cannot sign cannot offer, and bytes that
//! do not hash to what the offer says are never run.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::hash::is_hex_hash;
use crate::sign::PublicKey;

/// Version of the offer document itself.
pub const UPDATE_FORMAT: u32 = 1;

/// The three paths the channel lives at. Spelt once, here, because the
/// publisher and the launcher are built from the same workspace but shipped
/// as separate binaries: a rename on one side would otherwise read as "this
/// server offers nothing" on the other, for as long as nobody noticed.
pub const OFFER_PATH: &str = "/update.json";
pub const SIGNATURE_PATH: &str = "/update.sig";
/// The build itself, under its BLAKE3: `{BUILD_PREFIX}{hash}`.
pub const BUILD_PREFIX: &str = "/update/";

/// No ValhSync build is anywhere near this large. The ceiling exists so a
/// hostile offer cannot make the launcher fill a disk before it checks a hash.
pub const MAX_BUILD_BYTES: u64 = 128 * 1024 * 1024;

/// The build a server offers, as signed by that server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateOffer {
    pub format: u32,
    /// Semantic version of the build on offer, `major.minor.patch`.
    pub version: String,
    /// Rust target triple it was built for. A launcher ignores an offer that
    /// is not for the triple it is itself running.
    pub target: String,
    /// File name to install it under, so the launcher keeps its own name.
    pub exe: String,
    pub size: u64,
    pub blake3: String,
}

impl UpdateOffer {
    /// Verify the signature, then parse, then check every field. In that
    /// order: nothing from the network is parsed before it is known to come
    /// from the pinned key.
    pub fn parse_verified(bytes: &[u8], signature_text: &str, key: &PublicKey) -> Result<Self> {
        key.verify_text(bytes, signature_text)?;
        let offer: Self = serde_json::from_slice(bytes)?;
        offer.validate()?;
        Ok(offer)
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |msg: String| Err(CoreError::InvalidManifest(msg));

        if self.format != UPDATE_FORMAT {
            return bad(format!(
                "update format {} is not supported by this launcher (expected {UPDATE_FORMAT})",
                self.format
            ));
        }
        if parse_version(&self.version).is_none() {
            return bad(format!(
                "{:?} is not a major.minor.patch version",
                self.version
            ));
        }
        if self.target.is_empty()
            || self.target.len() > 64
            || !self
                .target
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        {
            return bad(format!("{:?} is not a target triple", self.target));
        }
        // The offer names a file the launcher will write beside itself. A
        // separator or a parent segment here would let a server choose where.
        if self.exe.is_empty()
            || self.exe.len() > 64
            || self.exe.contains(['/', '\\', ':'])
            || self.exe.split('.').any(|part| part == "..")
            || self.exe.starts_with('.')
            || !self
                .exe
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        {
            return bad(format!("{:?} is not a plain file name", self.exe));
        }
        if self.size == 0 || self.size > MAX_BUILD_BYTES {
            return bad(format!("{} is not a plausible build size", self.size));
        }
        if !is_hex_hash(&self.blake3) {
            return bad("blake3 is not a digest".into());
        }
        Ok(())
    }

    /// Is this worth offering to someone already running `current`?
    ///
    /// Strictly newer only: equal is not an update, and older is a downgrade
    /// the player never asked for.
    #[must_use]
    pub fn is_newer_than(&self, current: &str) -> bool {
        match (parse_version(&self.version), parse_version(current)) {
            (Some(offered), Some(running)) => offered > running,
            _ => false,
        }
    }

    /// A build for another platform is not an update, it is a brick.
    #[must_use]
    pub fn runs_on(&self, target: &str) -> bool {
        self.target == target
    }
}

/// `major.minor.patch`, each a plain number. Anything else is refused rather
/// than guessed at: an update decision is not the place for leniency.
#[must_use]
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let (a, b, c) = (next()?, next()?, next()?);
    if parts.next().is_some() {
        return None;
    }
    Some((a, b, c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign::Keypair;

    fn offer() -> UpdateOffer {
        UpdateOffer {
            format: UPDATE_FORMAT,
            version: "0.2.0".into(),
            target: "x86_64-pc-windows-msvc".into(),
            exe: "valhsync.exe".into(),
            size: 4_608_045,
            blake3: "a".repeat(64),
        }
    }

    #[test]
    fn versions_are_compared_as_numbers_not_text() {
        let mut o = offer();
        o.version = "0.10.0".into();
        assert!(o.is_newer_than("0.9.0"), "0.10.0 sorts after 0.9.0");
        o.version = "0.1.0".into();
        assert!(!o.is_newer_than("0.1.0"), "the same build is not an update");
        assert!(!o.is_newer_than("0.2.0"), "never offer a downgrade");
    }

    #[test]
    fn an_unparseable_version_is_never_newer() {
        let mut o = offer();
        o.version = "latest".into();
        assert!(!o.is_newer_than("0.1.0"));
        assert!(o.validate().is_err());
    }

    /// The offer names the file the launcher writes beside itself. A server
    /// must not be able to choose anywhere else.
    #[test]
    fn the_exe_name_cannot_escape_its_folder() {
        for name in [
            "../valhsync.exe",
            "..\\valhsync.exe",
            "sub/valhsync.exe",
            "C:valhsync.exe",
            "..",
            ".hidden",
            "",
        ] {
            let mut o = offer();
            o.exe = name.into();
            assert!(o.validate().is_err(), "{name:?} should be refused");
        }
        let mut o = offer();
        o.exe = "valhsync".into();
        assert!(o.validate().is_ok(), "a bare name is fine");
    }

    #[test]
    fn a_build_for_another_platform_is_not_an_update() {
        assert!(offer().runs_on("x86_64-pc-windows-msvc"));
        assert!(!offer().runs_on("x86_64-unknown-linux-gnu"));
    }

    #[test]
    fn an_offer_signed_by_another_key_is_refused() {
        let ours = Keypair::generate();
        let theirs = Keypair::generate();
        let bytes = serde_json::to_vec(&offer()).unwrap();
        let sig = crate::sign::encode_signature(&theirs.sign(&bytes));
        let err = UpdateOffer::parse_verified(&bytes, &sig, &ours.public()).unwrap_err();
        assert!(matches!(err, CoreError::BadSignature), "{err:?}");

        let sig = crate::sign::encode_signature(&ours.sign(&bytes));
        assert_eq!(
            UpdateOffer::parse_verified(&bytes, &sig, &ours.public()).unwrap(),
            offer()
        );
    }

    /// Tampering after signing must not survive: the digest is the only thing
    /// standing between the offer and what actually runs.
    #[test]
    fn a_swapped_digest_breaks_the_signature() {
        let kp = Keypair::generate();
        let bytes = serde_json::to_vec(&offer()).unwrap();
        let sig = crate::sign::encode_signature(&kp.sign(&bytes));
        let mut tampered = offer();
        tampered.blake3 = "b".repeat(64);
        let tampered = serde_json::to_vec(&tampered).unwrap();
        assert!(UpdateOffer::parse_verified(&tampered, &sig, &kp.public()).is_err());
    }

    #[test]
    fn absurd_sizes_are_refused() {
        let mut o = offer();
        o.size = 0;
        assert!(o.validate().is_err());
        o.size = MAX_BUILD_BYTES + 1;
        assert!(o.validate().is_err());
    }
}
