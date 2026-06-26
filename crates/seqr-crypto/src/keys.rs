//! Long-term identity keys.
//!
//! Each Seqr identity holds two keypairs:
//! - an **X25519** keypair, used to derive pairwise shared secrets (see [`crate::agreement`]);
//! - an **Ed25519** keypair, used to sign every message so peers (especially group
//!   members sharing one symmetric key) cannot forge one another (see [`crate::sign`]).
//!
//! Secret keys implement zeroize-on-drop via the underlying dalek types. These
//! structs are the in-memory representation; persistence is the vault's concern.

use ed25519_dalek::{SigningKey, VerifyingKey};
use rand_core::OsRng;
use x25519_dalek::{PublicKey as XPublic, StaticSecret as XSecret};

use crate::CryptoError;

/// A freshly generated, or loaded, Seqr identity. Holds secret material — never
/// serialize this directly; persist the raw bytes inside the encrypted vault.
pub struct Identity {
    pub agreement_secret: XSecret,
    pub signing_key: SigningKey,
}

/// The public half of an identity — safe to put in a profile blob and transmit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicIdentity {
    pub agreement_public: [u8; 32],
    pub signing_public: [u8; 32],
}

impl Identity {
    /// Generate a brand-new identity using the OS CSPRNG.
    pub fn generate() -> Self {
        let agreement_secret = XSecret::random_from_rng(OsRng);
        let signing_key = SigningKey::generate(&mut OsRng);
        Self { agreement_secret, signing_key }
    }

    /// Reconstruct an identity from its raw secret bytes (as stored in the vault).
    pub fn from_secret_bytes(
        agreement: &[u8; 32],
        signing: &[u8; 32],
    ) -> Result<Self, CryptoError> {
        let agreement_secret = XSecret::from(*agreement);
        let signing_key = SigningKey::from_bytes(signing);
        Ok(Self { agreement_secret, signing_key })
    }

    /// Raw secret bytes for persistence. Handle with care; store only encrypted.
    pub fn secret_bytes(&self) -> ([u8; 32], [u8; 32]) {
        (self.agreement_secret.to_bytes(), self.signing_key.to_bytes())
    }

    /// The shareable public identity.
    pub fn public(&self) -> PublicIdentity {
        PublicIdentity {
            agreement_public: XPublic::from(&self.agreement_secret).to_bytes(),
            signing_public: self.signing_key.verifying_key().to_bytes(),
        }
    }
}

impl PublicIdentity {
    pub fn verifying_key(&self) -> Result<VerifyingKey, CryptoError> {
        VerifyingKey::from_bytes(&self.signing_public).map_err(|_| CryptoError::BadKey)
    }

    pub fn agreement_public(&self) -> XPublic {
        XPublic::from(self.agreement_public)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_secret_bytes() {
        let id = Identity::generate();
        let (a, s) = id.secret_bytes();
        let restored = Identity::from_secret_bytes(&a, &s).unwrap();
        assert_eq!(id.public(), restored.public());
    }

    #[test]
    fn distinct_identities_differ() {
        assert_ne!(Identity::generate().public(), Identity::generate().public());
    }
}
