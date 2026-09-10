//! Ed25519 keys and detached signatures.
//!
//! Text encodings are base64url without padding, so a key or a signature can
//! be pasted anywhere (invite codes, config files, URLs) without escaping.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;

use crate::error::{CoreError, Result};

pub fn b64_encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Lenient decode: accepts url-safe or standard alphabet, padded or not.
pub fn b64_decode(text: &str) -> Option<Vec<u8>> {
    let trimmed = text.trim().trim_end_matches('=');
    URL_SAFE_NO_PAD
        .decode(trimmed)
        .or_else(|_| STANDARD_NO_PAD.decode(trimmed))
        .ok()
}

/// A server's signing key. The secret never appears in `Debug` output.
#[derive(Clone)]
pub struct Keypair {
    key: SigningKey,
}

impl fmt::Debug for Keypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keypair")
            .field("public", &self.public().fingerprint())
            .finish_non_exhaustive()
    }
}

impl Keypair {
    pub fn generate() -> Self {
        Self {
            key: SigningKey::generate(&mut OsRng),
        }
    }

    pub fn from_secret_b64(text: &str) -> Result<Self> {
        let bytes = b64_decode(text)
            .ok_or_else(|| CoreError::InvalidKey("secret key is not valid base64".into()))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| CoreError::InvalidKey("secret key must be 32 bytes".into()))?;
        Ok(Self {
            key: SigningKey::from_bytes(&arr),
        })
    }

    pub fn secret_b64(&self) -> String {
        b64_encode(&self.key.to_bytes())
    }

    pub fn public(&self) -> PublicKey {
        PublicKey(self.key.verifying_key())
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        self.key.sign(message)
    }
}

/// A server's public key, as pinned by the launcher when an invite is imported.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicKey(VerifyingKey);

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.fingerprint())
    }
}

impl PublicKey {
    pub fn from_b64(text: &str) -> Result<Self> {
        let bytes = b64_decode(text)
            .ok_or_else(|| CoreError::InvalidKey("public key is not valid base64".into()))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| CoreError::InvalidKey("public key must be 32 bytes".into()))?;
        let key = VerifyingKey::from_bytes(&arr)
            .map_err(|e| CoreError::InvalidKey(format!("public key is not a valid point: {e}")))?;
        Ok(Self(key))
    }

    pub fn to_b64(&self) -> String {
        b64_encode(self.0.as_bytes())
    }

    /// Short, human-comparable identifier: first 16 hex chars of BLAKE3(key),
    /// grouped in fours. Shown when a key changes so players can compare it
    /// with what the admin announces.
    pub fn fingerprint(&self) -> String {
        let hex = blake3::hash(self.0.as_bytes()).to_hex();
        hex.as_str()
            .as_bytes()
            .chunks(4)
            .take(4)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect::<Vec<_>>()
            .join("-")
    }

    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<()> {
        self.0
            .verify(message, signature)
            .map_err(|_| CoreError::BadSignature)
    }

    /// Verify a signature given in its text form.
    pub fn verify_text(&self, message: &[u8], signature_text: &str) -> Result<()> {
        let sig = decode_signature(signature_text)?;
        self.verify(message, &sig)
    }
}

pub fn encode_signature(sig: &Signature) -> String {
    b64_encode(&sig.to_bytes())
}

pub fn decode_signature(text: &str) -> Result<Signature> {
    let bytes = b64_decode(text).ok_or(CoreError::BadSignature)?;
    let arr: [u8; 64] = bytes.try_into().map_err(|_| CoreError::BadSignature)?;
    Ok(Signature::from_bytes(&arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_roundtrip() {
        let kp = Keypair::generate();
        let msg = b"{\"format\":1}";
        let sig = kp.sign(msg);
        kp.public().verify(msg, &sig).unwrap();
        let text = encode_signature(&sig);
        kp.public().verify_text(msg, &text).unwrap();
        kp.public().verify_text(msg, &format!("{text}\n")).unwrap();
    }

    #[test]
    fn tampering_is_detected() {
        let kp = Keypair::generate();
        let sig = kp.sign(b"original");
        assert!(matches!(
            kp.public().verify(b"originaL", &sig),
            Err(CoreError::BadSignature)
        ));
        let other = Keypair::generate();
        assert!(matches!(
            other.public().verify(b"original", &sig),
            Err(CoreError::BadSignature)
        ));
    }

    #[test]
    fn key_text_roundtrip() {
        let kp = Keypair::generate();
        let restored = Keypair::from_secret_b64(&kp.secret_b64()).unwrap();
        assert_eq!(restored.public(), kp.public());
        let pk = PublicKey::from_b64(&kp.public().to_b64()).unwrap();
        assert_eq!(pk, kp.public());
        assert_eq!(pk.fingerprint().len(), 19);
        assert!(PublicKey::from_b64("not base64!!").is_err());
        assert!(PublicKey::from_b64(&b64_encode(&[0u8; 31])).is_err());
        assert!(decode_signature("short").is_err());
    }
}
