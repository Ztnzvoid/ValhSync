//! BLAKE3 hashing. The hex digest doubles as the download key on the server
//! (`GET /files/<hex>`), so its format is checked strictly.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub use blake3::Hash;

use crate::error::{CoreError, Result};

/// Length of a hex-encoded BLAKE3 digest.
pub const HEX_LEN: usize = 64;

/// Hash a file by streaming it; never loads it whole in memory.
pub fn hash_file(path: &Path) -> Result<Hash> {
    let file = File::open(path).map_err(|e| CoreError::io(path, e))?;
    let mut hasher = blake3::Hasher::new();
    hasher
        .update_reader(BufReader::new(file))
        .map_err(|e| CoreError::io(path, e))?;
    Ok(hasher.finalize())
}

pub fn hash_bytes(bytes: &[u8]) -> Hash {
    blake3::hash(bytes)
}

pub fn to_hex(hash: &Hash) -> String {
    hash.to_hex().to_string()
}

/// Exactly 64 lowercase hex characters. Uppercase is refused so a digest has a
/// single spelling everywhere (URLs, state files, manifests).
pub fn is_hex_hash(s: &str) -> bool {
    s.len() == HEX_LEN && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Verify that a file on disk has the expected digest.
pub fn verify_file(path: &Path, expected_hex: &str, label: &str) -> Result<()> {
    let actual = to_hex(&hash_file(path)?);
    if actual == expected_hex {
        Ok(())
    } else {
        Err(CoreError::HashMismatch {
            path: label.to_string(),
            expected: expected_hex.to_string(),
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_format_is_strict() {
        let h = to_hex(&hash_bytes(b"hello"));
        assert!(is_hex_hash(&h));
        assert!(!is_hex_hash(&h.to_uppercase()));
        assert!(!is_hex_hash(&h[..63]));
        assert!(!is_hex_hash(&format!("{h}0")));
        assert!(!is_hex_hash("../../etc/passwd"));
    }

    #[test]
    fn file_hash_matches_bytes_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("f.bin");
        std::fs::write(&p, b"some bytes").unwrap();
        assert_eq!(hash_file(&p).unwrap(), hash_bytes(b"some bytes"));
        verify_file(&p, &to_hex(&hash_bytes(b"some bytes")), "f.bin").unwrap();
        let err = verify_file(&p, &to_hex(&hash_bytes(b"other")), "f.bin").unwrap_err();
        assert!(matches!(err, CoreError::HashMismatch { .. }));
    }
}
