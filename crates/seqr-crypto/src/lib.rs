//! Seqr cryptographic core.
//!
//! This crate exposes a small set of pure primitives that the rest of the
//! application composes into the protocol. It deliberately knows nothing about
//! conversations, networking, or storage — only keys and bytes.
//!
//! Module map:
//! - [`keys`]      — long-term identity keypairs (X25519 for agreement, Ed25519 for signing).
//! - [`agreement`] — derive a shared symmetric key from a keypair (ECDH + HKDF).
//! - [`aead`]      — seal/open message payloads with ChaCha20-Poly1305.
//! - [`sign`]      — detached signatures over arbitrary bytes.
//! - [`kdf`]       — stretch a login password into a vault key (Argon2id).
//! - [`group`]     — generate a random group key.

pub mod aead;
pub mod agreement;
pub mod group;
pub mod kdf;
pub mod keys;
pub mod sign;

/// A 32-byte symmetric key. Used both for ECDH-derived secrets and group keys.
pub type SymmetricKey = [u8; 32];

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("decryption failed: ciphertext could not be authenticated")]
    Decrypt,
    #[error("ciphertext too short")]
    Truncated,
    #[error("signature verification failed")]
    BadSignature,
    #[error("invalid key bytes")]
    BadKey,
    #[error("password key derivation failed")]
    Kdf,
}
