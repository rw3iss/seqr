//! Authenticated encryption of message payloads with ChaCha20-Poly1305.
//!
//! Each `seal` generates a fresh random 96-bit nonce, prepended to the ciphertext so
//! `open` is self-contained. Associated data (AAD) — e.g. the conversation id and
//! epoch — is authenticated but not encrypted, binding a ciphertext to its context so
//! it cannot be replayed into a different conversation or epoch.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand_core::{OsRng, RngCore};

use crate::{CryptoError, SymmetricKey};

const NONCE_LEN: usize = 12;

/// Seal `plaintext` under `key`, authenticating `aad`. Output is `nonce || ciphertext`.
pub fn seal(key: &SymmetricKey, plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad })
        .expect("ChaCha20-Poly1305 encryption is infallible for valid keys");
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    out
}

/// Open a `nonce || ciphertext` blob produced by [`seal`], checking `aad`.
pub fn open(key: &SymmetricKey, sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < NONCE_LEN {
        return Err(CryptoError::Truncated);
    }
    let (nonce_bytes, ct) = sealed.split_at(NONCE_LEN);
    let cipher = ChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), Payload { msg: ct, aad })
        .map_err(|_| CryptoError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = [7u8; 32];
        let sealed = seal(&key, b"hello sir", b"conv-1|epoch-3");
        let opened = open(&key, &sealed, b"conv-1|epoch-3").unwrap();
        assert_eq!(opened, b"hello sir");
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = seal(&[1u8; 32], b"secret", b"");
        assert!(matches!(open(&[2u8; 32], &sealed, b""), Err(CryptoError::Decrypt)));
    }

    #[test]
    fn tampered_aad_fails() {
        let key = [9u8; 32];
        let sealed = seal(&key, b"secret", b"conv-1|epoch-3");
        // Replaying into a different epoch must fail.
        assert!(matches!(open(&key, &sealed, b"conv-1|epoch-4"), Err(CryptoError::Decrypt)));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = [3u8; 32];
        let mut sealed = seal(&key, b"secret", b"");
        let last = sealed.len() - 1;
        sealed[last] ^= 0xff;
        assert!(open(&key, &sealed, b"").is_err());
    }

    #[test]
    fn nonces_are_unique() {
        let key = [4u8; 32];
        let a = seal(&key, b"x", b"");
        let b = seal(&key, b"x", b"");
        assert_ne!(a, b, "fresh nonce must make ciphertexts differ");
    }

    #[test]
    fn truncated_input() {
        assert!(matches!(open(&[0u8; 32], &[0u8; 4], b""), Err(CryptoError::Truncated)));
    }
}
