//! Detached Ed25519 signatures.
//!
//! Every message carries a signature by its sender's Ed25519 key. In a 1:1 the
//! pairwise key already authenticates the counterpart, but in a group — where all
//! members share one symmetric key — the signature is what proves *which* member
//! authored a message and prevents members from forging one another.
//!
//! The same machinery also authenticates mailbox `pull` requests (proving ownership
//! of an identity to fetch its parked messages).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::CryptoError;

/// Sign `msg`, returning the 64-byte detached signature.
pub fn sign(key: &SigningKey, msg: &[u8]) -> [u8; 64] {
    key.sign(msg).to_bytes()
}

/// Verify a detached signature. Returns `Ok(())` only if valid.
pub fn verify(public: &VerifyingKey, msg: &[u8], sig: &[u8; 64]) -> Result<(), CryptoError> {
    let signature = Signature::from_bytes(sig);
    public.verify(msg, &signature).map_err(|_| CryptoError::BadSignature)
}

/// Verify against raw 32-byte public key bytes (convenience for wire types).
pub fn verify_raw(public_bytes: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> Result<(), CryptoError> {
    let public = VerifyingKey::from_bytes(public_bytes).map_err(|_| CryptoError::BadKey)?;
    verify(&public, msg, sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Identity;

    #[test]
    fn sign_verify_roundtrip() {
        let id = Identity::generate();
        let sig = sign(&id.signing_key, b"the medium is the message");
        assert!(verify_raw(&id.public().signing_public, b"the medium is the message", &sig).is_ok());
    }

    #[test]
    fn wrong_message_rejected() {
        let id = Identity::generate();
        let sig = sign(&id.signing_key, b"original");
        assert!(verify_raw(&id.public().signing_public, b"tampered", &sig).is_err());
    }

    #[test]
    fn wrong_signer_rejected() {
        let alice = Identity::generate();
        let mallory = Identity::generate();
        let sig = sign(&alice.signing_key, b"msg");
        assert!(verify_raw(&mallory.public().signing_public, b"msg", &sig).is_err());
    }
}
