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
use seqr_crypto::SymmetricKey;

use super::vault::Friend;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_id_is_order_independent() {
        let a = "aa".repeat(32);
        let b = "bb".repeat(32);
        assert_eq!(direct_conversation_id(&a, &b), direct_conversation_id(&b, &a));
    }
}
