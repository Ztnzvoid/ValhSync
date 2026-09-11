//! The launcher build this publisher offers, if there is one.
//!
//! The publisher and the launcher ship in one archive and are unpacked side by
//! side, so the build worth offering is the file sitting next to this
//! executable and its version is this crate's version. Nothing is configurable
//! here on purpose: an admin cannot point the update channel at a file of
//! their choosing, and a publisher whose admin never unpacked the launcher
//! offers nothing at all.

use std::fs::Metadata;
use std::path::{Path, PathBuf};

use valhsync_core::update::{UPDATE_FORMAT, UpdateOffer};
use valhsync_core::{Keypair, hash, sign};

/// Target triple this binary was built for, from `build.rs`. It must be spelt
/// exactly as the launcher spells its own, which only Cargo knows.
const TARGET: &str = env!("VALHSYNC_TARGET");

/// Name the launcher has in the archive, and the name it keeps once installed.
#[cfg(windows)]
const LAUNCHER: &str = "valhsync.exe";
#[cfg(not(windows))]
const LAUNCHER: &str = "valhsync";

/// A signed offer and the bytes it describes, prepared once at startup so no
/// request ever hashes a file or reaches for the signing key.
#[derive(Debug, Clone)]
pub struct Offered {
    pub doc: Vec<u8>,
    pub sig: String,
    pub path: PathBuf,
    pub blake3: String,
    pub size: u64,
}

/// Look for the launcher beside the running publisher and sign an offer for
/// it. `None` whenever there is nothing sane to offer -- an admin who never
/// unpacked the launcher has a working publisher, not a broken one, so this is
/// never reported as an error.
pub fn find(keypair: &Keypair) -> Option<Offered> {
    let exe = std::env::current_exe().ok()?;
    find_beside(exe.parent()?, keypair)
}

fn find_beside(dir: &Path, keypair: &Keypair) -> Option<Offered> {
    let path = dir.join(LAUNCHER);
    let size = std::fs::metadata(&path)
        .ok()
        .filter(Metadata::is_file)?
        .len();
    let blake3 = hash::to_hex(&hash::hash_file(&path).ok()?);

    let offer = UpdateOffer {
        format: UPDATE_FORMAT,
        version: env!("CARGO_PKG_VERSION").to_string(),
        target: TARGET.to_string(),
        exe: LAUNCHER.to_string(),
        size,
        blake3: blake3.clone(),
    };
    // A launcher past the size ceiling, or a build script that handed us no
    // triple, is refused by the launcher anyway. Offer nothing rather than a
    // document nobody can accept.
    offer.validate().ok()?;

    let doc = serde_json::to_vec(&offer).ok()?;
    let sig = sign::encode_signature(&keypair.sign(&doc));
    Some(Offered {
        doc,
        sig,
        path,
        blake3,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use valhsync_core::CoreError;

    fn put_launcher(dir: &Path, bytes: &[u8]) {
        std::fs::write(dir.join(LAUNCHER), bytes).unwrap();
    }

    #[test]
    fn an_offer_passes_the_launchers_own_check() {
        let dir = tempfile::tempdir().unwrap();
        put_launcher(dir.path(), b"a launcher build");
        let kp = Keypair::generate();

        let offered = find_beside(dir.path(), &kp).expect("a launcher is right there");
        let offer = UpdateOffer::parse_verified(&offered.doc, &offered.sig, &kp.public())
            .expect("the launcher accepts what we signed");

        assert_eq!(offer.blake3, offered.blake3);
        assert_eq!(offer.size, offered.size);
        assert_eq!(offer.size, "a launcher build".len() as u64);
        assert_eq!(offer.exe, LAUNCHER);
        assert_eq!(offer.version, env!("CARGO_PKG_VERSION"));
        assert!(offer.runs_on(TARGET), "target is {TARGET:?}");
        assert_eq!(offered.path, dir.path().join(LAUNCHER));
    }

    #[test]
    fn an_offer_is_refused_under_another_key() {
        let dir = tempfile::tempdir().unwrap();
        put_launcher(dir.path(), b"a launcher build");
        let offered = find_beside(dir.path(), &Keypair::generate()).unwrap();
        let stranger = Keypair::generate();
        let err = UpdateOffer::parse_verified(&offered.doc, &offered.sig, &stranger.public())
            .unwrap_err();
        assert!(matches!(err, CoreError::BadSignature), "{err:?}");
    }

    #[test]
    fn a_publisher_without_a_launcher_beside_it_offers_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_beside(dir.path(), &Keypair::generate()).is_none());
        // A directory of that name is not a build either.
        std::fs::create_dir(dir.path().join(LAUNCHER)).unwrap();
        assert!(find_beside(dir.path(), &Keypair::generate()).is_none());
    }
}
