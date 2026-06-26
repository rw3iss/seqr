//! Group key sealing/opening and roster helpers.
//!
//! A group key (Kg) is delivered to each member sealed under the pairwise X25519 key
//! between the originator and that member (see [`super::conversation::pairwise_key`]).
//! Because that pairwise key is symmetric, the recipient opens it with the same
//! derivation against the originator. Chat messages then use Kg directly via the
//! ordinary [`super::message`] framing.

use seqr_crypto::keys::Identity;
use seqr_crypto::{aead, SymmetricKey};

use super::vault::Friend;
use super::CoreError;

/// AAD binding a sealed group key to its group and epoch.
fn key_aad(group_id: &str, epoch: u64) -> Vec<u8> {
    format!("seqr-groupkey|{group_id}|{epoch}").into_bytes()
}

/// Seal the group key for one recipient, under the pairwise key between us and them.
pub fn seal_group_key(
    me: &Identity,
    recipient: &Friend,
    group_id: &str,
    epoch: u64,
    key: &SymmetricKey,
) -> Result<String, CoreError> {
    let pair = super::conversation::pairwise_key(me, recipient)?;
    let sealed = aead::seal(&pair, key, &key_aad(group_id, epoch));
    Ok(hex::encode(sealed))
}

/// Open a sealed group key sent to us by `originator`.
pub fn open_group_key(
    me: &Identity,
    originator: &Friend,
    group_id: &str,
    epoch: u64,
    sealed_hex: &str,
) -> Result<SymmetricKey, CoreError> {
    let pair = super::conversation::pairwise_key(me, originator)?;
    let sealed = hex::decode(sealed_hex).map_err(|_| CoreError::Crypto("bad sealed key".into()))?;
    let plain = aead::open(&pair, &sealed, &key_aad(group_id, epoch))?;
    plain.try_into().map_err(|_| CoreError::Crypto("group key wrong length".into()))
}

/// A short random group id (hex).
pub fn new_group_id() -> String {
    hex::encode(&seqr_crypto::group::generate_group_key()[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use seqr_crypto::group::generate_group_key;

    fn member_of(id: &Identity, name: &str) -> Friend {
        let p = id.public();
        Friend {
            display_name: name.into(),
            agreement_public: hex::encode(p.agreement_public),
            signing_public: hex::encode(p.signing_public),
            node_addr: String::new(),
        }
    }

    #[test]
    fn group_key_seals_and_opens_for_member() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let kg = generate_group_key();
        let gid = new_group_id();

        // Alice seals Kg for Bob; Bob opens it using Alice as originator.
        let sealed = seal_group_key(&alice, &member_of(&bob, "Bob"), &gid, 0, &kg).unwrap();
        let opened = open_group_key(&bob, &member_of(&alice, "Alice"), &gid, 0, &sealed).unwrap();
        assert_eq!(opened, kg);
    }

    #[test]
    fn outsider_cannot_open() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let mallory = Identity::generate();
        let kg = generate_group_key();
        let gid = new_group_id();
        let sealed = seal_group_key(&alice, &member_of(&bob, "Bob"), &gid, 0, &kg).unwrap();
        // Mallory uses Alice as originator but isn't the intended recipient.
        assert!(open_group_key(&mallory, &member_of(&alice, "Alice"), &gid, 0, &sealed).is_err());
    }
}
