//! Conversation helpers for 1:1 chats.
//!
//! A 1:1 conversation needs an identifier both parties compute identically (so the
//! frame's AAD and history grouping line up regardless of who sent the message). We
//! derive it deterministically from the two signing public keys, order-independent.
//!
//! The pairwise symmetric key is the X25519 ECDH result (see [`seqr_crypto::agreement`]);
//! epoch 0 until key rotation arrives in a later milestone.

use seqr_crypto::agreement::derive_pairwise_bytes;
use seqr_crypto::keys::Identity;
use seqr_crypto::{aead, SymmetricKey};

use super::vault::{Friend, VaultData};
use super::CoreError;

/// Current epoch for direct conversations until rotation (M5) is implemented.
pub const DIRECT_EPOCH: u64 = 0;

/// Order-independent conversation id for the two parties' signing public keys.
pub fn direct_conversation_id(a_signing_hex: &str, b_signing_hex: &str) -> String {
    let (lo, hi) = if a_signing_hex <= b_signing_hex {
        (a_signing_hex, b_signing_hex)
    } else {
        (b_signing_hex, a_signing_hex)
    };
    format!("{lo}-{hi}")
}

/// Derive the pairwise symmetric key for a conversation with `friend`.
pub fn pairwise_key(me: &Identity, friend: &Friend) -> Result<SymmetricKey, CoreError> {
    let their_agreement = hex::decode(&friend.agreement_public)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
        .ok_or(CoreError::BadProfile("bad agreement key".into()))?;
    Ok(derive_pairwise_bytes(&me.agreement_secret, &their_agreement))
}

fn direct_key_aad(conversation_id: &str, epoch: u64) -> Vec<u8> {
    format!("seqr-directkey|{conversation_id}|{epoch}").into_bytes()
}

/// Seal a rotated 1:1 key for `friend`, under the long-term identity pairwise key.
pub fn seal_direct_key(
    me: &Identity,
    friend: &Friend,
    conversation_id: &str,
    epoch: u64,
    key: &SymmetricKey,
) -> Result<String, CoreError> {
    let base = pairwise_key(me, friend)?;
    Ok(hex::encode(aead::seal(&base, key, &direct_key_aad(conversation_id, epoch))))
}

/// Open a rotated 1:1 key sent by `originator`.
pub fn open_direct_key(
    me: &Identity,
    originator: &Friend,
    conversation_id: &str,
    epoch: u64,
    sealed_hex: &str,
) -> Result<SymmetricKey, CoreError> {
    let base = pairwise_key(me, originator)?;
    let sealed = hex::decode(sealed_hex).map_err(|_| CoreError::Crypto("bad sealed key".into()))?;
    let plain = aead::open(&base, &sealed, &direct_key_aad(conversation_id, epoch))?;
    plain.try_into().map_err(|_| CoreError::Crypto("key wrong length".into()))
}

/// The current key + epoch for a 1:1 with `friend`: a rotated key if present, else the
/// identity-derived pairwise key at epoch 0.
pub fn current_direct_key(
    data: &VaultData,
    me: &Identity,
    friend: &Friend,
) -> Result<(u64, SymmetricKey), CoreError> {
    let my_signing = hex::encode(me.public().signing_public);
    let conv_id = direct_conversation_id(&my_signing, &friend.signing_public);
    match data.direct_key(&conv_id) {
        Some(dk) => {
            let key = hex::decode(&dk.key)
                .ok()
                .and_then(|v| <[u8; 32]>::try_from(v).ok())
                .ok_or(CoreError::Crypto("bad stored key".into()))?;
            Ok((dk.epoch, key))
        }
        None => Ok((DIRECT_EPOCH, pairwise_key(me, friend)?)),
    }
}

/// The key for a specific 1:1 `epoch` (to open a received frame). Epoch 0 is always the
/// identity pairwise key; any other epoch must match the stored rotated key.
pub fn direct_key_for_epoch(
    data: &VaultData,
    me: &Identity,
    friend: &Friend,
    epoch: u64,
) -> Result<SymmetricKey, CoreError> {
    if epoch == DIRECT_EPOCH {
        return pairwise_key(me, friend);
    }
    let my_signing = hex::encode(me.public().signing_public);
    let conv_id = direct_conversation_id(&my_signing, &friend.signing_public);
    match data.direct_key(&conv_id) {
        Some(dk) if dk.epoch == epoch => hex::decode(&dk.key)
            .ok()
            .and_then(|v| <[u8; 32]>::try_from(v).ok())
            .ok_or(CoreError::Crypto("bad stored key".into())),
        _ => Err(CoreError::Crypto("unknown key epoch".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_id_is_order_independent() {
        let a = "aa".repeat(32);
        let b = "bb".repeat(32);
        assert_eq!(direct_conversation_id(&a, &b), direct_conversation_id(&b, &a));
    }

    #[test]
    fn rotated_direct_key_seals_and_opens() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let bob_friend = Friend {
            display_name: "Bob".into(),
            agreement_public: hex::encode(bob.public().agreement_public),
            signing_public: hex::encode(bob.public().signing_public),
            node_addr: String::new(),
        };
        let alice_friend = Friend {
            display_name: "Alice".into(),
            agreement_public: hex::encode(alice.public().agreement_public),
            signing_public: hex::encode(alice.public().signing_public),
            node_addr: String::new(),
        };
        let new_key = seqr_crypto::group::generate_group_key();
        let conv = "c";
        let sealed = seal_direct_key(&alice, &bob_friend, conv, 1, &new_key).unwrap();
        let opened = open_direct_key(&bob, &alice_friend, conv, 1, &sealed).unwrap();
        assert_eq!(opened, new_key);
    }
}
