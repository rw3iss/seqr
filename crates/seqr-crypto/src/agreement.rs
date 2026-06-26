//! Key agreement: derive a shared symmetric key from one's own X25519 secret and a
//! peer's X25519 public key.
//!
//! Raw X25519 output is run through HKDF-SHA256 with a domain-separation label so the
//! resulting key is uniformly random and bound to this application's purpose. Both
//! sides compute the identical key without transmitting any secret.

use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::SymmetricKey;

const HKDF_INFO: &[u8] = b"seqr/v1/pairwise-key";

/// Derive the pairwise symmetric key for a 1:1 conversation.
///
/// `my_secret` is this device's X25519 secret; `their_public` is the peer's X25519
/// public key (obtained from their profile blob). The operation is symmetric:
/// `derive(a_sec, b_pub) == derive(b_sec, a_pub)`.
pub fn derive_pairwise(my_secret: &StaticSecret, their_public: &PublicKey) -> SymmetricKey {
    let shared = my_secret.diffie_hellman(their_public);
    let hk = Hkdf::<Sha256>::new(None, shared.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut key)
        .expect("32 is a valid HKDF-SHA256 output length");
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Identity;

    #[test]
    fn both_sides_agree() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let a = derive_pairwise(&alice.agreement_secret, &bob.public().agreement_public());
        let b = derive_pairwise(&bob.agreement_secret, &alice.public().agreement_public());
        assert_eq!(a, b);
    }

    #[test]
    fn different_pairs_differ() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let carol = Identity::generate();
        let ab = derive_pairwise(&alice.agreement_secret, &bob.public().agreement_public());
        let ac = derive_pairwise(&alice.agreement_secret, &carol.public().agreement_public());
        assert_ne!(ab, ac);
    }
}
